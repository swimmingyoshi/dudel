use crate::{blend::BlendMode, document::Document, selection::Selection};
use flate2::{Compression, bufread::GzDecoder, write::GzEncoder};
use serde::Serialize;
use std::io::{Read, Write};

pub const MAGIC: &[u8] = b"DUDL\x03\r\n";
/// Room for every entry's metadata with its longest (4096-byte) name.
const METADATA_LIMIT: usize = 16 * 1024 * 1024;
#[derive(Serialize)]
struct Metadata<'a> {
    version: u32,
    width: usize,
    height: usize,
    active: usize,
    selection: Option<Selection>,
    layers: Vec<LayerMetadata<'a>>,
    /// Optional, so files without guides keep their earlier bytes and older
    /// readers ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    guides: Option<&'a crate::guides::Guides>,
    /// Reference layer settings; their images follow in the project trailer.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    references: &'a [crate::reference_layers::ReferenceLayer],
}
#[derive(Serialize)]
struct LayerMetadata<'a> {
    name: &'a str,
    visible: bool,
    opacity: f32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    csp_byte_opacity: bool,
    blend: BlendMode,
    clipped: bool,
    locked: bool,
    alpha_locked: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    folder: bool,
    #[serde(skip_serializing_if = "is_zero")]
    depth: u8,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mask: Option<MaskMetadata>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    effects: &'a [crate::layer_effects::LayerEffect],
    #[serde(skip_serializing_if = "Option::is_none")]
    adjustment: Option<&'a crate::adjustment::Adjustment>,
    pixels: [u8; 0],
}
#[derive(Serialize)]
struct MaskMetadata {
    enabled: bool,
    linked: bool,
}
fn is_zero(value: &u8) -> bool {
    *value == 0
}
pub(crate) fn metadata(doc: &Document) -> Result<Vec<u8>, String> {
    doc.validate()?;
    let metadata = Metadata {
        version: doc.version,
        width: doc.width,
        height: doc.height,
        active: doc.active,
        selection: doc.selection,
        layers: doc
            .layers
            .iter()
            .map(|layer| LayerMetadata {
                name: &layer.name,
                visible: layer.visible,
                opacity: layer.opacity,
                csp_byte_opacity: layer.csp_byte_opacity,
                blend: layer.blend,
                clipped: layer.clipped,
                locked: layer.locked,
                alpha_locked: layer.alpha_locked,
                folder: layer.folder,
                depth: layer.depth,
                collapsed: layer.collapsed,
                mask: layer.mask.as_ref().map(|m| MaskMetadata {
                    enabled: m.enabled,
                    linked: m.linked,
                }),
                effects: &layer.effects,
                adjustment: layer.adjustment.as_ref(),
                pixels: [],
            })
            .collect(),
        guides: doc.guides.as_deref(),
        references: &doc.references,
    };
    let data = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
    if data.len() > METADATA_LIMIT {
        return Err("Project metadata exceeds 16 MiB".into());
    }
    Ok(data)
}
pub fn encode(doc: &Document, writer: impl Write) -> Result<(), String> {
    if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
        return Err("Legacy v3 cannot store layer masks; use current project format".into());
    }
    if doc.selection_mask.is_some() {
        return Err("Legacy v3 cannot store selection masks; use current project format".into());
    }
    if !doc.references.is_empty() {
        return Err("Legacy v3 cannot store reference layers; use current project format".into());
    }
    let data = metadata(doc)?;
    let mut writer = writer;
    writer.write_all(MAGIC).map_err(|e| e.to_string())?;
    let mut gzip = GzEncoder::new(writer, Compression::fast());
    gzip.write_all(&(data.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    gzip.write_all(&data).map_err(|e| e.to_string())?;
    for layer in &doc.layers {
        layer
            .pixels
            .write_rgba(&mut gzip)
            .map_err(|e| e.to_string())?;
    }
    gzip.finish().map_err(|e| e.to_string())?;
    Ok(())
}
pub fn decode(reader: impl Read) -> Result<Document, String> {
    let mut gzip = GzDecoder::new(std::io::BufReader::new(reader));
    let mut doc = read_metadata(&mut gzip)?;
    if !doc.references.is_empty() {
        return Err("Legacy v3 projects cannot contain reference layers".into());
    }
    if doc.version != 1 || doc.layers.iter().any(|l| l.mask.is_some()) {
        return Err("Unsupported legacy layer-mask document".into());
    }
    // Legacy layers are stored full size; bound them before allocating.
    let rasters = doc.layers.iter().filter(|layer| !layer.folder).count();
    if rasters.saturating_mul(doc.width * doc.height * 4) > crate::document::MAX_PAYLOAD {
        return Err("Legacy project payload exceeds the document budget".into());
    }
    for layer in doc.layers.iter_mut().filter(|layer| !layer.folder) {
        layer.pixels = crate::pixels::Pixels::read_rgba(&mut gzip, doc.width, doc.height)
            .map_err(|e| format!("Damaged layer pixels: {e}"))?;
    }
    finish_decode(gzip, doc)
}
pub(crate) fn read_metadata(reader: &mut impl Read) -> Result<Document, String> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .map_err(|e| format!("Damaged project metadata: {e}"))?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > METADATA_LIMIT {
        return Err("Invalid project metadata length".into());
    }
    let mut metadata = vec![0; length];
    reader
        .read_exact(&mut metadata)
        .map_err(|e| e.to_string())?;
    let doc: Document = serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
    if doc.selection_mask.is_some() {
        return Err("Selection coverage belongs in the bounded project trailer".into());
    }
    doc.validate_metadata()?;
    if doc
        .layers
        .iter()
        .filter_map(|l| l.mask.as_ref())
        .any(|m| m.coverage.dimensions() != [0, 0])
    {
        return Err("Layer mask coverage belongs in the versioned payload".into());
    }
    if doc.layers.iter().any(|layer| !layer.pixels.is_empty()) {
        return Err("Binary metadata must not contain pixels".into());
    }
    crate::reference_layers::validate(&doc.references, false)?;
    Ok(doc)
}
pub(crate) fn finish_decode<R: Read>(
    gzip: GzDecoder<std::io::BufReader<R>>,
    doc: Document,
) -> Result<Document, String> {
    let mut gzip = gzip;
    // Reading to EOF validates the gzip checksum/footer and rejects extra payload.
    if gzip
        .read(&mut [0; 1])
        .map_err(|e| format!("Damaged project checksum: {e}"))?
        != 0
    {
        return Err("Unexpected trailing project payload".into());
    }
    let mut remainder = gzip.into_inner();
    if remainder.read(&mut [0; 1]).map_err(|e| e.to_string())? != 0 {
        return Err("Unexpected trailing compressed data".into());
    }
    doc.validate()?;
    Ok(doc)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_stream_preserves_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("painting.dudel");
        std::fs::write(&path, b"previous project").unwrap();
        let result = crate::storage::atomic_stream(&path, |file| {
            file.write_all(b"partial replacement").unwrap();
            Err("Injected write failure".into())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"previous project");
    }
    #[test]
    #[ignore = "512 MiB artwork and approximately 1 GiB round-trip working set"]
    fn maximum_supported_binary_project_roundtrip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("maximum.dudel");
        let mut doc = Document::new(4096, 1024);
        for _ in 0..31 {
            doc.add_layer();
        }
        for (index, layer) in doc.layers.iter_mut().enumerate() {
            layer.pixels.fill((index * 7) as u8);
            layer.name = "\0".repeat(4096);
        }
        crate::storage::save_project(&path, &doc).unwrap();
        let loaded = crate::storage::load_project(&path).unwrap();
        assert_eq!(
            (
                loaded.width,
                loaded.height,
                loaded.layers.len(),
                loaded.active
            ),
            (4096, 1024, 32, 31)
        );
        for (expected, actual) in doc.layers.iter().zip(&loaded.layers) {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.pixels, expected.pixels);
        }
        drop(loaded);
        let recovery = crate::storage::Recovery {
            document: doc,
            original_path: Some(path),
        };
        let recovery_path = directory.path().join("maximum.gz");
        crate::storage::save_recovery(&recovery_path, &recovery).unwrap();
        let restored = crate::storage::read_recovery(&recovery_path).unwrap();
        assert_eq!(restored.original_path, recovery.original_path);
        for (expected, actual) in recovery
            .document
            .layers
            .iter()
            .zip(&restored.document.layers)
        {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.pixels, expected.pixels);
        }
    }
    #[test]
    fn binary_roundtrip_preserves_all_layers_and_rejects_damage() {
        let mut doc = Document::new(8, 4);
        for i in 0..31 {
            doc.add_layer();
            doc.layers[i + 1].pixels.fill(i as u8);
        }
        doc.layers[5].blend = BlendMode::SoftLight;
        doc.layers[6].clipped = true;
        let original = serde_json::to_vec(&doc).unwrap();
        let mut data = Vec::new();
        encode(&doc, &mut data).unwrap();
        let decoded = decode(&data[MAGIC.len()..]).unwrap();
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), original);
        let mut extra = data.clone();
        extra.extend_from_slice(b"trailing bytes");
        assert!(decode(&extra[MAGIC.len()..]).is_err());
        data.truncate(data.len() - 5);
        assert!(decode(&data[MAGIC.len()..]).is_err());
    }
    #[test]
    fn oversized_metadata_rejected_before_allocation() {
        let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
        gzip.write_all(&u32::MAX.to_le_bytes()).unwrap();
        assert!(decode(gzip.finish().unwrap().as_slice()).is_err());
    }
}
