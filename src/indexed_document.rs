//! Experimental document object schema. Streaming visitor; no editor save path.
use crate::{
    binary_project,
    document::{Document, LayerId},
    indexed_tile,
    pixels::Pixels,
};
use std::{collections::BTreeSet, io::Read};
const TILE: u64 = 1 << 62;
const MASK: u64 = 2 << 62;
const MAX_OBJECTS: usize = 8192;
const MAX_METADATA: usize = 1024 * 1024;
#[derive(Clone)]
pub(crate) struct Keys {
    next: u32,
    layers: Vec<(LayerId, u32)>,
}
impl Default for Keys {
    fn default() -> Self {
        Self {
            next: 1,
            layers: Vec::new(),
        }
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    next_layer_key: u32,
    layer_keys: Vec<u32>,
    selection_mask: bool,
    #[serde(default)]
    thumbnail: bool,
}
fn cancelled(check: &impl Fn() -> bool) -> Result<(), String> {
    if check() {
        Err("Indexed document canceled".into())
    } else {
        Ok(())
    }
}
fn rect(width: usize, height: usize, index: usize) -> [usize; 4] {
    let columns = width.div_ceil(256);
    let left = index % columns * 256;
    let top = index / columns * 256;
    [left, top, (left + 256).min(width), (top + 256).min(height)]
}
fn size(rect: [usize; 4]) -> [u16; 2] {
    [(rect[2] - rect[0]) as u16, (rect[3] - rect[1]) as u16]
}
fn crc(data: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    crc.sum()
}

/// The caller must publish the returned key state only after its transaction
/// commits. Emitted objects may be written incrementally and must remain private
/// on any failure. This visitor never retains a document's encoded tile set.
pub(crate) fn visit(
    doc: &Document,
    previous: &Keys,
    cancel: &impl Fn() -> bool,
    emit: &mut impl FnMut(u64, Vec<u8>) -> Result<(), String>,
) -> Result<Keys, String> {
    visit_objects(doc, previous, cancel, &mut |key, data| match data {
        Object::Encoded(bytes) => emit(key, bytes),
        Object::Source {
            pixels,
            canvas,
            region,
        } => {
            let bytes = pixels.region(canvas[0], region);
            if let Some(encoded) = indexed_tile::encode(size(region), &bytes)? {
                emit(key, encoded)?;
            }
            Ok(())
        }
        Object::Rgba { size, bytes } => {
            if let Some(encoded) = indexed_tile::encode(size, &bytes)? {
                emit(key, encoded)?;
            }
            Ok(())
        }
    })
}
pub(crate) enum Object {
    Encoded(Vec<u8>),
    Rgba {
        size: [u16; 2],
        bytes: Vec<u8>,
    },
    Source {
        pixels: Pixels,
        canvas: [usize; 2],
        region: [usize; 4],
    },
}
/// Raw raster input lets a sink validate committed pixels before recompressing.
/// Durable keys, inventory and source validation are identical to `visit`.
pub(crate) fn visit_objects(
    doc: &Document,
    previous: &Keys,
    cancel: &impl Fn() -> bool,
    emit: &mut impl FnMut(u64, Object) -> Result<(), String>,
) -> Result<Keys, String> {
    cancelled(cancel)?;
    doc.validate()?;
    if doc
        .layers
        .iter()
        .any(|layer| !layer.folder && !layer.pixels.has_geometry(doc.width, doc.height))
    {
        return Err("Indexed source pixel geometry differs from document".into());
    }
    if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
        return Err(
            "Indexed projects do not yet store layer masks; use a regular Dudel project".into(),
        );
    }
    let metadata = binary_project::metadata(doc)?;
    let mut keys = Keys {
        next: previous.next,
        layers: Vec::new(),
    };
    if keys.next == 0 {
        return Err("Invalid indexed layer key counter".into());
    }
    for layer in &doc.layers {
        let key = if let Some((_, key)) = previous.layers.iter().find(|(id, _)| *id == layer.id) {
            *key
        } else {
            let key = keys.next;
            keys.next = keys
                .next
                .checked_add(1)
                .ok_or("Indexed layer keys exhausted")?;
            key
        };
        keys.layers.push((layer.id, key));
    }
    let manifest = Manifest {
        version: 3,
        next_layer_key: keys.next,
        layer_keys: keys.layers.iter().map(|(_, key)| *key).collect(),
        selection_mask: doc.selection_mask.is_some(),
        thumbnail: true,
    };
    if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
        return Err("Unsupported indexed layer-mask document".into());
    }
    validate_manifest(&manifest, doc.layers.len())?;
    emit(
        0,
        Object::Encoded(serde_json::to_vec(&manifest).map_err(|e| e.to_string())?),
    )?;
    emit(1, Object::Encoded(metadata))?;
    emit(
        2,
        Object::Encoded(crate::indexed_thumbnail::encode(doc, cancel)?),
    )?;
    let columns = doc.width.div_ceil(256);
    for (layer, (_, key)) in doc.layers.iter().zip(&keys.layers) {
        cancelled(cancel)?;
        if layer.folder {
            continue;
        }
        for region in layer.pixels.nonzero_tiles(doc.width, doc.height) {
            cancelled(cancel)?;
            let index = region[1] / 256 * columns + region[0] / 256;
            emit(
                TILE | ((*key as u64) << 16) | index as u64,
                Object::Source {
                    pixels: layer.pixels.clone(),
                    canvas: [doc.width, doc.height],
                    region,
                },
            )?;
        }
    }
    if let Some(mask) = &doc.selection_mask {
        for index in 0..columns * doc.height.div_ceil(256) {
            cancelled(cancel)?;
            let region = rect(doc.width, doc.height, index);
            let shape = size(region);
            let mut raw = Vec::with_capacity(shape[0] as usize * shape[1] as usize);
            for y in region[1]..region[3] {
                raw.extend_from_slice(
                    &mask.bytes()[y * doc.width + region[0]..y * doc.width + region[2]],
                );
            }
            if let Some(page) = crate::indexed_mask::encode(shape, &raw)? {
                emit(MASK | index as u64, Object::Encoded(page))?;
            }
        }
    }
    cancelled(cancel)?;
    Ok(keys)
}
fn validate_manifest(manifest: &Manifest, count: usize) -> Result<(), String> {
    let unique: BTreeSet<_> = manifest.layer_keys.iter().copied().collect();
    if !matches!(
        (manifest.version, manifest.thumbnail),
        (1, false) | (2 | 3, true)
    ) || manifest.layer_keys.len() != count
        || unique.len() != count
        || unique.contains(&0)
        || manifest.next_layer_key == 0
        || unique
            .last()
            .is_some_and(|key| *key >= manifest.next_layer_key)
    {
        return Err("Invalid indexed document version or layer keys".into());
    }
    Ok(())
}
fn decode_legacy_mask(page: &[u8], shape: [u16; 2]) -> Result<Vec<u8>, String> {
    let expected = shape[0] as usize * shape[1] as usize;
    if page.len() != 16 + expected
        || &page[..8] != b"DIMASK01"
        || page[8..10] != shape[0].to_le_bytes()
        || page[10..12] != shape[1].to_le_bytes()
        || u32::from_le_bytes(page[12..16].try_into().unwrap()) != crc(&page[16..])
        || page[16..].iter().all(|byte| *byte == 0)
    {
        return Err("Invalid legacy indexed selection page".into());
    }
    Ok(page[16..].to_vec())
}
fn object(
    get: &mut impl FnMut(u64) -> Result<Vec<u8>, String>,
    key: u64,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let bytes = get(key)?;
    if bytes.len() > limit {
        return Err("Indexed object exceeds schema limit".into());
    }
    Ok(bytes)
}

/// Cached preview only; it does not validate raster payloads or full file health.
/// The record provider must bound allocations before returning bytes.
pub(crate) fn read_thumbnail(
    objects: &[u64],
    get: &mut impl FnMut(u64) -> Result<Vec<u8>, String>,
    cancel: &impl Fn() -> bool,
) -> Result<Option<crate::indexed_thumbnail::Thumbnail>, String> {
    cancelled(cancel)?;
    if objects.len() > MAX_OBJECTS
        || objects.windows(2).any(|pair| pair[0] >= pair[1])
        || objects.get(..2) != Some(&[0, 1])
    {
        return Err("Invalid indexed preview inventory".into());
    }
    let bytes = object(get, 0, 4096)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let metadata = object(get, 1, MAX_METADATA)?;
    let length = (metadata.len() as u32).to_le_bytes();
    let doc = binary_project::read_metadata(&mut length.as_slice().chain(metadata.as_slice()))?;
    if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
        return Err("Unsupported indexed layer-mask document".into());
    }
    validate_manifest(&manifest, doc.layers.len())?;
    if objects.binary_search(&2).is_ok() != manifest.thumbnail {
        return Err("Indexed thumbnail presence differs from manifest".into());
    }
    cancelled(cancel)?;
    if !manifest.thumbnail {
        return Ok(None);
    }
    let encoded = object(get, 2, crate::indexed_thumbnail::MAX_BYTES)?;
    let thumbnail = crate::indexed_thumbnail::decode(&encoded, [doc.width, doc.height])?;
    cancelled(cancel)?;
    Ok(Some(thumbnail))
}

/// Validate all key ownership before allocating artwork. Every required object,
/// including a v2 thumbnail, must pass before returning a complete document.
pub(crate) fn decode(
    objects: &[u64],
    get: &mut impl FnMut(u64) -> Result<Vec<u8>, String>,
    cancel: &impl Fn() -> bool,
) -> Result<(Document, Keys), String> {
    decode_inner(objects, get, cancel, true)
}
/// Validate every codec/owner/page/bound without building raster layers or a
/// canvas-sized mask. Used before replacing a file during compaction.
pub(crate) fn validate_records(
    objects: &[u64],
    get: &mut impl FnMut(u64) -> Result<Vec<u8>, String>,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    decode_inner(objects, get, cancel, false).map(|_| ())
}
/// Reopening without the original runtime-ID binding allocates fresh durable
/// layer keys above the committed counter, after full validate-only decoding.
pub(crate) fn replacement_keys(
    objects: &[u64],
    get: &mut impl FnMut(u64) -> Result<Vec<u8>, String>,
    cancel: &impl Fn() -> bool,
) -> Result<Keys, String> {
    validate_records(objects, get, cancel)?;
    let bytes = object(get, 0, 4096)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Keys {
        next: manifest.next_layer_key,
        layers: Vec::new(),
    })
}
impl Keys {
    pub(crate) fn descriptor_bytes(&self) -> usize {
        self.layers.capacity() * std::mem::size_of::<(LayerId, u32)>()
    }
}
fn decode_inner(
    objects: &[u64],
    get: &mut impl FnMut(u64) -> Result<Vec<u8>, String>,
    cancel: &impl Fn() -> bool,
    materialize: bool,
) -> Result<(Document, Keys), String> {
    cancelled(cancel)?;
    if objects.len() > MAX_OBJECTS
        || objects.windows(2).any(|pair| pair[0] >= pair[1])
        || objects.get(..2) != Some(&[0, 1])
    {
        return Err("Invalid indexed document object inventory".into());
    }
    let bytes = object(get, 0, 4096)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let metadata = object(get, 1, MAX_METADATA)?;
    let length = (metadata.len() as u32).to_le_bytes();
    let mut doc = binary_project::read_metadata(&mut length.as_slice().chain(metadata.as_slice()))?;
    drop(metadata);
    if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
        return Err("Unsupported indexed layer-mask document".into());
    }
    validate_manifest(&manifest, doc.layers.len())?;
    if objects.binary_search(&2).is_ok() != manifest.thumbnail {
        return Err("Indexed thumbnail presence differs from manifest".into());
    }
    if manifest.thumbnail {
        let encoded = object(get, 2, crate::indexed_thumbnail::MAX_BYTES)?;
        crate::indexed_thumbnail::decode(&encoded, [doc.width, doc.height])?;
    }
    let columns = doc.width.div_ceil(256);
    let count = columns * doc.height.div_ceil(256);
    for key in &objects[2..] {
        if *key == 2 {
            continue;
        }
        let index = (key & 0xffff) as usize;
        if key >> 62 == 1 {
            let owner = (key & !(3u64 << 62)) >> 16;
            if owner > u32::MAX as u64 || index >= count {
                return Err("Invalid indexed tile owner or index".into());
            }
            let position = manifest
                .layer_keys
                .iter()
                .position(|id| *id as u64 == owner)
                .ok_or("Unresolved indexed layer key")?;
            if doc.layers[position].folder {
                return Err("Indexed folder must not own raster tiles".into());
            }
        } else if key >> 62 == 2 {
            if !manifest.selection_mask || *key != (MASK | index as u64) || index >= count {
                return Err("Invalid indexed selection page key".into());
            }
        } else {
            return Err("Unknown indexed document object kind".into());
        }
    }
    for layer in &mut doc.layers {
        if materialize && !layer.folder {
            layer.pixels = Pixels::sparse(doc.width, doc.height);
        }
    }
    let mut coverage = if materialize && manifest.selection_mask {
        Some(vec![0; doc.width * doc.height])
    } else {
        None
    };
    let mut mask_bounds: Option<crate::selection::Selection> = None;
    for key in &objects[2..] {
        if *key == 2 {
            continue;
        }
        cancelled(cancel)?;
        let index = (key & 0xffff) as usize;
        let region = rect(doc.width, doc.height, index);
        let shape = size(region);
        if key >> 62 == 1 {
            let owner = ((key & !(3u64 << 62)) >> 16) as u32;
            let position = manifest
                .layer_keys
                .iter()
                .position(|id| *id == owner)
                .unwrap();
            let encoded = object(get, *key, 32 + 256 * 256 * 4)?;
            let raw = indexed_tile::decode(&encoded, shape)?;
            if materialize {
                doc.layers[position]
                    .pixels
                    .writer()
                    .write_region(doc.width, region, &raw);
            }
        } else {
            let page = object(get, *key, crate::indexed_mask::MAX_BYTES)?;
            let raw = if manifest.version >= 3 {
                crate::indexed_mask::decode(&page, shape)?
            } else {
                decode_legacy_mask(&page, shape)?
            };
            let stride = shape[0] as usize;
            for (index, value) in raw.iter().enumerate() {
                if *value != 0 {
                    let x = region[0] + index % stride;
                    let y = region[1] + index / stride;
                    let bounds = mask_bounds.get_or_insert(crate::selection::Selection {
                        left: x,
                        top: y,
                        right: x + 1,
                        bottom: y + 1,
                    });
                    bounds.left = bounds.left.min(x);
                    bounds.top = bounds.top.min(y);
                    bounds.right = bounds.right.max(x + 1);
                    bounds.bottom = bounds.bottom.max(y + 1);
                }
            }
            if let Some(coverage) = coverage.as_mut() {
                for (row, y) in (region[1]..region[3]).enumerate() {
                    coverage[y * doc.width + region[0]..y * doc.width + region[2]]
                        .copy_from_slice(&raw[row * stride..(row + 1) * stride]);
                }
            }
        }
    }
    if manifest.selection_mask && mask_bounds != doc.selection {
        return Err("Indexed mask bounds differ from metadata".into());
    }
    if let Some(coverage) = coverage {
        doc.selection_mask = Some(crate::selection_mask::Mask::from_coverage(
            doc.width, doc.height, coverage,
        )?);
    }
    cancelled(cancel)?;
    if materialize {
        doc.validate()?;
    } else {
        doc.validate_metadata()?;
    }
    let keys = Keys {
        next: manifest.next_layer_key,
        layers: doc
            .layers
            .iter()
            .zip(manifest.layer_keys)
            .map(|(layer, key)| (layer.id, key))
            .collect(),
    };
    Ok((doc, keys))
}

pub(crate) fn equivalent(a: &Document, b: &Document) -> Result<bool, String> {
    if a.version != 1 || b.version != 1 {
        return Err("Indexed equivalence does not support layer-mask documents".into());
    }
    Ok(binary_project::metadata(a)? == binary_project::metadata(b)?
        && a.selection_mask == b.selection_mask
        && a.layers
            .iter()
            .zip(&b.layers)
            .all(|(a, b)| a.pixels == b.pixels))
}
pub(crate) fn fixture() -> Document {
    let mut doc = Document::new(257, 259);
    doc.layers[0].name = "Generated \u{00e9} leaf".into();
    doc.layers[0].depth = 1;
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[11, 22, 33, 128]);
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(257 * 259 - 1)
        .copy_from_slice(&[19, 20, 21, 0]);
    doc.add_layer();
    let layer = &mut doc.layers[1];
    layer.depth = 1;
    layer.opacity = 156. / 256.;
    layer.csp_byte_opacity = true;
    layer.clipped = true;
    layer.visible = false;
    layer.alpha_locked = true;
    layer.blend = crate::blend::BlendMode::Multiply;
    layer
        .pixels
        .writer()
        .rgba_mut(256)
        .copy_from_slice(&[200, 80, 20, 177]);
    let mut folder = crate::document::Layer::new_folder("Generated group".into());
    folder.opacity = 0.75;
    folder.collapsed = true;
    folder.locked = true;
    doc.layers.push(folder);
    doc.active = 2;
    let mut coverage = vec![0; doc.width * doc.height];
    coverage[0] = 1;
    coverage[256] = 128;
    *coverage.last_mut().unwrap() = 255;
    let mask = crate::selection_mask::Mask::from_coverage(doc.width, doc.height, coverage).unwrap();
    doc.selection = mask.bounds();
    doc.selection_mask = Some(mask);
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn thumbnail_only_reads_metadata_and_preview_with_v1_compatibility_and_v2_validation() {
        let doc = fixture();
        let (objects, _) = encode(&doc, &Keys::default());
        validate_records(
            &objects.keys().copied().collect::<Vec<_>>(),
            &mut |key| Ok(objects[&key].clone()),
            &|| false,
        )
        .unwrap();
        let inventory = objects.keys().copied().collect::<Vec<_>>();
        let mut reads = Vec::new();
        let thumbnail = read_thumbnail(
            &inventory,
            &mut |key| {
                reads.push(key);
                assert!(key <= 2, "thumbnail read raster/coverage payload");
                Ok(objects[&key].clone())
            },
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reads, [0, 1, 2]);
        assert_eq!(thumbnail.size, [254, 256]);
        let expected = crate::indexed_thumbnail::decode(
            &crate::indexed_thumbnail::encode(&doc, &|| false).unwrap(),
            [doc.width, doc.height],
        )
        .unwrap();
        assert_eq!(thumbnail.rgba, expected.rgba);
        let mut damaged_art = objects.clone();
        let raster = *damaged_art.keys().find(|key| **key >> 62 == 1).unwrap();
        *damaged_art.get_mut(&raster).unwrap().last_mut().unwrap() ^= 1;
        assert!(read(&damaged_art).is_err());
        assert!(
            read_thumbnail(
                &damaged_art.keys().copied().collect::<Vec<_>>(),
                &mut |key| {
                    assert!(key <= 2);
                    Ok(damaged_art[&key].clone())
                },
                &|| false
            )
            .unwrap()
            .is_some()
        );
        let mut v1 = objects.clone();
        let mut manifest: Manifest = serde_json::from_slice(&v1[&0]).unwrap();
        manifest.version = 1;
        manifest.thumbnail = false;
        let mut json = serde_json::to_value(manifest).unwrap();
        json.as_object_mut().unwrap().remove("thumbnail");
        v1.insert(0, serde_json::to_vec(&json).unwrap());
        v1.remove(&2);
        for (key, bytes) in v1.iter_mut().filter(|(key, _)| **key >> 62 == 2) {
            let region = rect(doc.width, doc.height, (*key & 0xffff) as usize);
            let shape = size(region);
            let raw = crate::indexed_mask::decode(bytes, shape).unwrap();
            let mut legacy = b"DIMASK01".to_vec();
            legacy.extend(shape[0].to_le_bytes());
            legacy.extend(shape[1].to_le_bytes());
            legacy.extend(crc(&raw).to_le_bytes());
            legacy.extend(raw);
            *bytes = legacy;
        }
        let inventory = v1.keys().copied().collect::<Vec<_>>();
        assert!(
            read_thumbnail(&inventory, &mut |key| Ok(v1[&key].clone()), &|| false)
                .unwrap()
                .is_none()
        );
        assert!(equivalent(&doc, &read(&v1).unwrap().0).unwrap());
        let mut v2 = v1.clone();
        let mut manifest: Manifest = serde_json::from_slice(&v2[&0]).unwrap();
        manifest.version = 2;
        manifest.thumbnail = true;
        v2.insert(0, serde_json::to_vec(&manifest).unwrap());
        v2.insert(2, objects[&2].clone());
        assert!(equivalent(&doc, &read(&v2).unwrap().0).unwrap());
        validate_records(
            &v2.keys().copied().collect::<Vec<_>>(),
            &mut |key| Ok(v2[&key].clone()),
            &|| false,
        )
        .unwrap();
        // A new mask record must never be accepted under an older schema.
        let mask_key = *objects.keys().find(|key| **key >> 62 == 2).unwrap();
        v2.insert(mask_key, objects[&mask_key].clone());
        assert!(read(&v2).is_err());
        let mut mixed = objects.clone();
        mixed.insert(mask_key, v1[&mask_key].clone());
        assert!(read(&mixed).is_err());

        let mut missing = objects.clone();
        missing.remove(&2);
        assert!(read(&missing).is_err());
        let mut corrupt = objects;
        *corrupt.get_mut(&2).unwrap().last_mut().unwrap() ^= 1;
        assert!(read(&corrupt).is_err());
        assert!(
            read_thumbnail(
                &corrupt.keys().copied().collect::<Vec<_>>(),
                &mut |key| Ok(corrupt[&key].clone()),
                &|| false
            )
            .is_err()
        );
    }
    fn encode(doc: &Document, keys: &Keys) -> (BTreeMap<u64, Vec<u8>>, Keys) {
        let mut objects = BTreeMap::new();
        let next = visit(doc, keys, &|| false, &mut |key, data| {
            assert!(objects.insert(key, data).is_none());
            Ok(())
        })
        .unwrap();
        (objects, next)
    }
    fn read(objects: &BTreeMap<u64, Vec<u8>>) -> Result<(Document, Keys), String> {
        decode(
            &objects.keys().copied().collect::<Vec<_>>(),
            &mut |key| objects.get(&key).cloned().ok_or("Missing object".into()),
            &|| false,
        )
    }
    #[test]
    fn all_document_properties_mask_edges_hidden_rgb_and_fresh_runtime_ids_round_trip() {
        let doc = fixture();
        let (objects, _) = encode(&doc, &Keys::default());
        let (loaded, keys) = read(&objects).unwrap();
        assert!(equivalent(&doc, &loaded).unwrap());
        assert_ne!(doc.layers[0].id, loaded.layers[0].id);
        assert_eq!(loaded.composite_transparent(), doc.composite_transparent());
        let (again, _) = encode(&loaded, &keys);
        assert_eq!(again, objects);
        for empty in [false, true] {
            let mut doc = Document::new(1, 1);
            if empty {
                doc.selection_mask =
                    Some(crate::selection_mask::Mask::from_coverage(1, 1, vec![0]).unwrap());
            }
            let (objects, _) = encode(&doc, &Keys::default());
            let (loaded, _) = read(&objects).unwrap();
            assert!(equivalent(&doc, &loaded).unwrap());
            assert_eq!(loaded.selection_mask.is_some(), empty);
        }
    }
    #[test]
    fn layer_reorder_preserves_tile_keys_and_new_layers_never_reuse_deleted_keys() {
        let mut doc = fixture();
        let (old, keys) = encode(&doc, &Keys::default());
        doc.layers.swap(0, 1);
        let (new, keys) = encode(&doc, &keys);
        assert_eq!(
            old.iter()
                .filter(|(key, _)| **key >= TILE)
                .collect::<Vec<_>>(),
            new.iter()
                .filter(|(key, _)| **key >= TILE)
                .collect::<Vec<_>>()
        );
        let retired = keys.layers[0].1;
        doc.layers.remove(0);
        doc.active = 1;
        let (_, keys) = encode(&doc, &keys);
        doc.add_layer();
        let (_, next) = encode(&doc, &keys);
        assert!(next.layers.last().unwrap().1 > retired);
    }
    #[test]
    fn corrupt_inventory_geometry_mask_and_late_decode_never_return_a_partial_document() {
        let doc = fixture();
        let (objects, _) = encode(&doc, &Keys::default());
        for kind in 0..11 {
            let mut damaged = objects.clone();
            match kind {
                0 => {
                    damaged.insert(3, vec![0]);
                }
                1 => {
                    let key = *damaged.keys().find(|key| **key >> 62 == 1).unwrap();
                    damaged.insert(key | 0xffff, vec![0]);
                }
                2 => {
                    let key = *damaged.keys().find(|key| **key >> 62 == 2).unwrap();
                    *damaged.get_mut(&key).unwrap().last_mut().unwrap() ^= 1;
                }
                3 => {
                    damaged.remove(&1);
                }
                4 => {
                    let data = damaged.get_mut(&0).unwrap();
                    let mut manifest: Manifest = serde_json::from_slice(data).unwrap();
                    manifest.layer_keys[1] = manifest.layer_keys[0];
                    *data = serde_json::to_vec(&manifest).unwrap();
                }
                5 => {
                    let key = *damaged.keys().find(|key| **key >> 62 == 1).unwrap();
                    *damaged.get_mut(&key).unwrap().last_mut().unwrap() ^= 1;
                }
                6..=8 => {
                    let mut manifest: Manifest = serde_json::from_slice(&damaged[&0]).unwrap();
                    match kind {
                        6 => manifest.version = 99,
                        7 => manifest.next_layer_key = 1,
                        _ => manifest.selection_mask = false,
                    }
                    damaged.insert(0, serde_json::to_vec(&manifest).unwrap());
                }
                9 => {
                    let last = *damaged.keys().rfind(|key| **key >> 62 == 2).unwrap();
                    damaged.remove(&last);
                }
                _ => {
                    let manifest: Manifest = serde_json::from_slice(&damaged[&0]).unwrap();
                    damaged.insert(TILE | ((manifest.layer_keys[2] as u64) << 16), vec![0]);
                }
            }
            assert!(read(&damaged).is_err(), "accepted mutation {kind}");
            assert!(
                validate_records(
                    &damaged.keys().copied().collect::<Vec<_>>(),
                    &mut |key| Ok(damaged[&key].clone()),
                    &|| false
                )
                .is_err()
            );
        }
        assert!(
            decode(
                &objects.keys().copied().collect::<Vec<_>>(),
                &mut |_| panic!("canceled decoder read"),
                &|| true
            )
            .is_err()
        );
        let mut calls = 0;
        assert!(
            visit(&doc, &Keys::default(), &|| true, &mut |_, _| {
                calls += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        let checks = std::cell::Cell::new(0);
        assert!(
            decode(
                &objects.keys().copied().collect::<Vec<_>>(),
                &mut |key| Ok(objects[&key].clone()),
                &|| {
                    let n = checks.get() + 1;
                    checks.set(n);
                    n >= 3
                }
            )
            .is_err()
        );
        let mut wrong = doc.clone();
        wrong.layers[0].pixels = Pixels::sparse(doc.height, doc.width);
        let mut emitted = 0;
        assert!(
            visit(&wrong, &Keys::default(), &|| false, &mut |_, _| {
                emitted += 1;
                Ok(())
            })
            .err()
            .unwrap()
            .contains("geometry")
        );
        assert_eq!(emitted, 0);
        assert!(
            visit(
                &doc,
                &Keys::default(),
                &|| false,
                &mut |key, _| if key >= TILE {
                    Err("late output failure".into())
                } else {
                    Ok(())
                }
            )
            .is_err()
        );
    }
}
