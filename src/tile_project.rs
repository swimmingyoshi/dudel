//! Containers v4/v5: bounded metadata and sparse raster/mask records inside gzip.
use crate::{
    binary_project,
    document::Document,
    pixels::{Pixels, SIDE},
};
#[cfg(test)]
use flate2::write::GzEncoder;
use flate2::{Compression, bufread::GzDecoder};
use std::io::{Read, Write};
pub const MAGIC: &[u8] = b"DUDL\x04\r\n";
pub const MASK_MAGIC: &[u8] = b"DUDL\x05\r\n";
/// Projects with layer effects or adjustment layers. Builds that predate them
/// reject this header instead of opening the file and dropping them on resave.
pub const STYLE_MAGIC: &[u8] = b"DUDL\x06\r\n";
/// Which header a project was read from.
#[derive(Clone, Copy, PartialEq)]
enum Container {
    Plain,
    Masked,
    Styled,
}
/// The header `doc` is saved with.
pub(crate) fn magic(doc: &Document) -> &'static [u8] {
    if doc
        .layers
        .iter()
        .any(|l| !l.effects.is_empty() || l.adjustment.is_some() || l.fill != 1.0)
    {
        STYLE_MAGIC
    } else if doc.version == 2 {
        MASK_MAGIC
    } else {
        MAGIC
    }
}
const MASK_TRAILER: &[u8; 5] = b"MSEL\x01";

/// Recovery snapshots skip the file-browser preview; nobody browses them.
pub fn encode(doc: &Document, writer: impl Write) -> Result<(), String> {
    encode_with(doc, writer, None, false)
}

pub(crate) fn encode_cancelable(
    doc: &Document,
    writer: impl Write,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    encode_with(doc, writer, cancel, true)
}

fn encode_with(
    doc: &Document,
    mut writer: impl Write,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    preview: bool,
) -> Result<(), String> {
    let check = || crate::storage::check_save_cancel(cancel);
    check()?;
    let metadata = binary_project::metadata(doc)?;
    let mut header = flate2::GzBuilder::new();
    if preview && let Some(extra) = crate::file_preview::gzip_extra(doc) {
        header = header.extra(extra);
    }
    check()?;
    writer.write_all(magic(doc)).map_err(|e| e.to_string())?;
    let mut gzip = header.write(writer, Compression::fast());
    gzip.write_all(&(metadata.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    gzip.write_all(&metadata).map_err(|e| e.to_string())?;
    let columns = doc.width.div_ceil(SIDE);
    for layer in doc.layers.iter().filter(|layer| !layer.folder) {
        check()?;
        let regions = layer.pixels.nonzero_tiles(doc.width, doc.height);
        gzip.write_all(&(regions.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        for rect in regions {
            check()?;
            let index = rect[1] / SIDE * columns + rect[0] / SIDE;
            gzip.write_all(&(index as u32).to_le_bytes())
                .map_err(|e| e.to_string())?;
            gzip.write_all(&layer.pixels.region(doc.width, rect))
                .map_err(|e| e.to_string())?;
        }
    }
    for mask in doc.layers.iter().filter_map(|l| l.mask.as_ref()) {
        check()?;
        mask.coverage.encode(&mut gzip, cancel).map_err(|error| {
            if error == "Fill canceled" {
                crate::storage::SAVE_CANCELED.into()
            } else {
                error
            }
        })?;
    }
    if let Some(mask) = &doc.selection_mask {
        gzip.write_all(MASK_TRAILER).map_err(|e| e.to_string())?;
        for chunk in mask.bytes().chunks(64 * 1024) {
            check()?;
            gzip.write_all(chunk).map_err(|e| e.to_string())?;
        }
    }
    crate::reference_layers::encode_trailer(&doc.references, &mut gzip, &check)?;
    check()?;
    gzip.finish().map_err(|e| e.to_string())?;
    check()?;
    Ok(())
}
fn read_u32(reader: &mut impl Read) -> Result<usize, String> {
    let mut bytes = [0; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Damaged tile record: {e}"))?;
    Ok(u32::from_le_bytes(bytes) as usize)
}
pub fn decode(reader: impl Read) -> Result<Document, String> {
    decode_inner(reader, None, Container::Plain)
}
pub(crate) fn decode_masked(reader: impl Read) -> Result<Document, String> {
    decode_inner(reader, None, Container::Masked)
}
pub(crate) fn decode_styled(reader: impl Read) -> Result<Document, String> {
    decode_inner(reader, None, Container::Styled)
}
/// Replay fixtures reject multi-layer metadata and over-budget tile counts
/// before decoding pixel data, rather than checking after allocating it.
pub(crate) fn decode_single_layer(
    reader: impl Read,
    payload_budget: usize,
) -> Result<Document, String> {
    decode_inner(reader, Some(payload_budget), Container::Plain)
}
fn decode_inner(
    reader: impl Read,
    single_layer_budget: Option<usize>,
    container: Container,
) -> Result<Document, String> {
    let mut gzip = GzDecoder::new(std::io::BufReader::new(reader));
    let mut doc = binary_project::read_metadata(&mut gzip)?;
    let version_ok = match container {
        Container::Plain => doc.version == 1,
        Container::Masked => doc.version == 2,
        // Effects and adjustments come with or without masks.
        Container::Styled => matches!(doc.version, 1 | 2),
    };
    if !version_ok {
        return Err("Document version differs from project container".into());
    }
    let mut payload_used = 0usize;
    if single_layer_budget.is_some()
        && (doc.layers.len() != 1 || doc.layers[0].folder || doc.active != 0)
    {
        return Err("Stroke replay requires one active raster layer".into());
    }
    let columns = doc.width.div_ceil(SIDE);
    let tile_count = columns * doc.height.div_ceil(SIDE);
    for layer in doc.layers.iter_mut().filter(|layer| !layer.folder) {
        let count = read_u32(&mut gzip)?;
        if count > tile_count {
            return Err("Invalid layer tile count".into());
        }
        if single_layer_budget.is_some_and(|budget| count * SIDE * SIDE * 4 > budget) {
            return Err("Stroke replay tile payload exceeds budget".into());
        }
        layer.pixels = Pixels::sparse(doc.width, doc.height);
        let mut previous = None;
        let mut writer = layer.pixels.writer();
        for _ in 0..count {
            let index = read_u32(&mut gzip)?;
            if index >= tile_count || previous.is_some_and(|previous| index <= previous) {
                return Err("Invalid or unordered tile index".into());
            }
            previous = Some(index);
            let left = index % columns * SIDE;
            let top = index / columns * SIDE;
            let right = (left + SIDE).min(doc.width);
            let bottom = (top + SIDE).min(doc.height);
            let length = (right - left) * (bottom - top) * 4;
            payload_used += length;
            let budget = if doc.version == 2 {
                crate::document::MAX_MASKED_PAYLOAD
            } else {
                crate::document::MAX_PAYLOAD
            };
            if payload_used > budget {
                return Err("Document payload exceeds budget".into());
            }
            let mut bytes = vec![0; length];
            gzip.read_exact(&mut bytes)
                .map_err(|e| format!("Damaged tile pixels: {e}"))?;
            if bytes.iter().all(|byte| *byte == 0) {
                return Err("Empty tile records must be omitted".into());
            }
            writer.write_region(doc.width, [left, top, right, bottom], &bytes);
        }
    }
    for mask in doc.layers.iter_mut().filter_map(|l| l.mask.as_mut()) {
        mask.coverage = crate::layer_mask::Mask::decode(
            &mut gzip,
            [doc.width, doc.height],
            crate::document::MAX_MASKED_PAYLOAD - payload_used,
            None,
        )?;
        payload_used += mask.coverage.payload_bytes();
    }
    let mut tag = next_trailer(&mut gzip)?;
    if let Some(found) = tag {
        if &found != MASK_TRAILER {
            if &found == crate::reference_layers::TRAILER
                || &found == crate::reference_layers::STACK_TRAILER
            {
                read_references(&mut doc, &mut gzip, single_layer_budget)?;
                return binary_project::finish_decode(gzip, doc);
            }
            return Err("Unexpected trailing project payload".into());
        }
        if single_layer_budget.is_some_and(|budget| {
            doc.layers[0]
                .pixels
                .capacity()
                .saturating_add(doc.width * doc.height)
                > budget
        }) {
            return Err("Stroke replay selection coverage exceeds snapshot budget".into());
        }
        let mut bytes = vec![0; doc.width * doc.height];
        gzip.read_exact(&mut bytes)
            .map_err(|e| format!("Damaged selection coverage: {e}"))?;
        doc.selection_mask = Some(crate::selection_mask::Mask::from_coverage(
            doc.width, doc.height, bytes,
        )?);
        tag = next_trailer(&mut gzip)?;
        if let Some(found) = tag {
            if &found != crate::reference_layers::TRAILER
                && &found != crate::reference_layers::STACK_TRAILER
            {
                return Err("Unexpected trailing project payload".into());
            }
            read_references(&mut doc, &mut gzip, single_layer_budget)?;
            return binary_project::finish_decode(gzip, doc);
        }
    }
    if !doc.references.is_empty() {
        return Err("Reference layer images are missing".into());
    }
    binary_project::finish_decode(gzip, doc)
}
/// The next optional trailer tag, or None at the end of the payload.
fn next_trailer(reader: &mut impl Read) -> Result<Option<[u8; 5]>, String> {
    let mut first = [0];
    if reader
        .read(&mut first)
        .map_err(|e| format!("Damaged project checksum: {e}"))?
        == 0
    {
        return Ok(None);
    }
    let mut tag = [0u8; 5];
    tag[0] = first[0];
    reader
        .read_exact(&mut tag[1..])
        .map_err(|e| format!("Damaged project trailer: {e}"))?;
    Ok(Some(tag))
}
fn read_references(
    doc: &mut Document,
    reader: &mut impl Read,
    single_layer_budget: Option<usize>,
) -> Result<(), String> {
    if single_layer_budget.is_some() {
        return Err("Stroke replay fixtures cannot contain reference layers".into());
    }
    crate::reference_layers::decode_trailer(&mut doc.references, reader)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn styled_header_marks_projects_with_effects_or_adjustments() {
        let mut doc = Document::new(4, 4);
        assert_eq!(magic(&doc), MAGIC);
        doc.layers[0]
            .effects
            .push(crate::layer_effects::LayerEffect::new(
                crate::layer_effects::Effect::preset("Stroke").unwrap(),
            ));
        assert_eq!(magic(&doc), STYLE_MAGIC);
        // Switched-off effects still need a reader that keeps them.
        doc.layers[0].effects[0].enabled = false;
        assert_eq!(magic(&doc), STYLE_MAGIC);
        doc.layers[0].effects.clear();
        doc.layers[0].adjustment = Some(crate::adjustment::Adjustment::new(
            crate::adjustment::Settings::Invert,
        ));
        assert_eq!(magic(&doc), STYLE_MAGIC);
        let mut bytes = Vec::new();
        encode(&doc, &mut bytes).unwrap();
        let loaded = decode_styled(&bytes[STYLE_MAGIC.len()..]).unwrap();
        assert!(loaded.layers[0].adjustment.is_some());
        // A styled container still checks the document schema version.
        let mut masked = loaded;
        masked.version = 3;
        assert!(masked.validate_metadata().is_err());
    }
    #[test]
    fn encoding_can_cancel_between_payload_writes_without_changing_uncanceled_roundtrip() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct CancelWriter<'a> {
            output: Vec<u8>,
            flag: &'a AtomicBool,
        }
        impl Write for CancelWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.output.extend_from_slice(bytes);
                if self.output.len() > 1024 {
                    self.flag.store(true, Ordering::Relaxed);
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut doc = Document::new(512, 256);
        let mut random = 71u32;
        let mut source = vec![0; 512 * 256 * 4];
        for pixel in source.chunks_exact_mut(4) {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            pixel.copy_from_slice(&random.to_le_bytes());
        }
        doc.layers[0].pixels = source.into();
        let cancel = AtomicBool::new(false);
        let mut writer = CancelWriter {
            output: Vec::new(),
            flag: &cancel,
        };
        assert_eq!(
            encode_cancelable(&doc, &mut writer, Some(&cancel)),
            Err(crate::storage::SAVE_CANCELED.into())
        );
        assert!(cancel.load(Ordering::Relaxed));
        let mut ordinary = Vec::new();
        encode_cancelable(&doc, &mut ordinary, None).unwrap();
        cancel.store(false, Ordering::Relaxed);
        let mut controlled = Vec::new();
        encode_cancelable(&doc, &mut controlled, Some(&cancel)).unwrap();
        assert_eq!(ordinary, controlled);
        let reopened = decode(&controlled[MAGIC.len()..]).unwrap();
        assert_eq!(reopened.layers[0].pixels, doc.layers[0].pixels);
    }
    #[test]
    fn maximum_coverage_trailer_roundtrips_and_legacy_encoder_rejects_mask_loss() {
        let mut doc = Document::new(2048, 2048);
        let coverage: Vec<u8> = (0..2048 * 2048)
            .map(|index| ((index * 37) % 256) as u8)
            .collect();
        let mask = crate::selection_mask::Mask::from_coverage(2048, 2048, coverage).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask.clone());
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(256 * 2048 + 256)
            .copy_from_slice(&[7, 5, 3, 0]);
        let mut encoded = Vec::new();
        encode(&doc, &mut encoded).unwrap();
        let decoded = decode(&encoded[MAGIC.len()..]).unwrap();
        assert_eq!(decoded.selection_mask, Some(mask));
        assert_eq!(decoded.layers[0].pixels, doc.layers[0].pixels);
        assert!(binary_project::encode(&doc, Vec::new()).is_err());
        assert!(decode_single_layer(&encoded[MAGIC.len()..], 2048 * 2048).is_err());
        let json = serde_json::to_vec(&doc).unwrap();
        let from_json: Document = serde_json::from_slice(&json).unwrap();
        assert!(from_json.validate().is_ok());
        assert_eq!(from_json.selection_mask, doc.selection_mask);
    }
    #[test]
    fn selection_trailer_rejects_truncation_bounds_mismatch_and_duplicate_payload() {
        let mut doc = Document::new(2, 2);
        doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 2,
            bottom: 2,
        });
        for payload in [
            [
                0u32.to_le_bytes().as_slice(),
                MASK_TRAILER,
                &[255, 255, 255],
            ]
            .concat(),
            [0u32.to_le_bytes().as_slice(), MASK_TRAILER, &[0, 0, 0, 255]].concat(),
            [
                0u32.to_le_bytes().as_slice(),
                MASK_TRAILER,
                &[255, 255, 255, 255],
                MASK_TRAILER,
            ]
            .concat(),
        ] {
            assert!(decode(raw(&doc, &payload).as_slice()).is_err());
        }
    }
    fn raw(doc: &Document, payload: &[u8]) -> Vec<u8> {
        let metadata = binary_project::metadata(doc).unwrap();
        let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
        gzip.write_all(&(metadata.len() as u32).to_le_bytes())
            .unwrap();
        gzip.write_all(&metadata).unwrap();
        gzip.write_all(payload).unwrap();
        gzip.finish().unwrap()
    }
    #[test]
    fn sparse_edge_records_preserve_hidden_rgb_metadata_and_legacy_loading() {
        let mut doc = Document::new(517, 259);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 255]);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(258 * 517 + 516)
            .copy_from_slice(&[9, 8, 7, 0]);
        doc.add_layer();
        doc.add_layer();
        doc.layers[2].pixels = vec![0; 517 * 259 * 4].into();
        doc.layers[2]
            .pixels
            .writer()
            .rgba_mut(255 * 517 + 256)
            .copy_from_slice(&[4, 5, 6, 128]);
        doc.layers[2].blend = crate::blend::BlendMode::Multiply;
        doc.layers[2].opacity = 0.4;
        doc.layers[2].csp_byte_opacity = true;
        doc.selection = Some(crate::selection::Selection {
            left: 255,
            top: 255,
            right: 517,
            bottom: 259,
        });
        let mut encoded = Vec::new();
        encode(&doc, &mut encoded).unwrap();
        assert!(encoded.starts_with(MAGIC));
        assert!(encoded.len() < 8192);
        let decoded = decode(&encoded[MAGIC.len()..]).unwrap();
        assert_eq!(decoded.active, doc.active);
        assert_eq!(decoded.selection, doc.selection);
        assert_eq!(decoded.layers[2].blend, doc.layers[2].blend);
        assert_eq!(decoded.layers[2].opacity, doc.layers[2].opacity);
        assert!(decoded.layers[2].csp_byte_opacity);
        assert_eq!(decoded.layers[1].pixels.capacity(), 0);
        assert_eq!(decoded.layers[0].pixels.capacity(), 2 * SIDE * SIDE * 4);
        for (old, new) in doc.layers.iter().zip(&decoded.layers) {
            for index in 0..517 * 259 {
                assert_eq!(old.pixels.rgba(index), new.pixels.rgba(index));
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.dudel");
        let mut legacy = Vec::new();
        binary_project::encode(&doc, &mut legacy).unwrap();
        std::fs::write(&path, &legacy).unwrap();
        let loaded = crate::storage::load_project(&path).unwrap();
        assert!(loaded.layers[2].csp_byte_opacity);
        crate::storage::save_project(&path, &loaded).unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(MAGIC));
        // Older recovery wrappers embed v3 projects, and remain readable.
        let mut recovery = b"DRCV\x01\r\n".to_vec();
        recovery.extend_from_slice(&4u32.to_le_bytes());
        recovery.extend_from_slice(b"null");
        recovery.extend_from_slice(&legacy);
        let recovery_path = directory.path().join("legacy-recovery.gz");
        std::fs::write(&recovery_path, recovery).unwrap();
        let recovered = crate::storage::read_recovery(&recovery_path).unwrap();
        assert_eq!(
            recovered.document.layers[0].pixels.rgba(258 * 517 + 516),
            [9, 8, 7, 0]
        );
    }
    #[test]
    fn corrupt_counts_indexes_payloads_checksums_and_trailing_data_are_rejected() {
        let doc = Document::new(2, 1);
        for payload in [
            u32::MAX.to_le_bytes().to_vec(), // Count exceeds metadata geometry.
            [1u32.to_le_bytes(), 1u32.to_le_bytes()].concat(), // Index outside grid.
            [1u32.to_le_bytes(), 0u32.to_le_bytes()].concat(), // Missing pixels.
            [1u32.to_le_bytes(), 0u32.to_le_bytes(), [0; 4], [0; 4]].concat(), // Empty tile.
            [0u32.to_le_bytes(), [1; 4]].concat(), // Extra uncompressed payload.
        ] {
            assert!(decode(raw(&doc, &payload).as_slice()).is_err());
        }
        let wide = Document::new(257, 1);
        let mut duplicate = 2u32.to_le_bytes().to_vec();
        duplicate.extend_from_slice(&0u32.to_le_bytes());
        duplicate.extend_from_slice(&[1; 256 * 4]);
        duplicate.extend_from_slice(&0u32.to_le_bytes());
        duplicate.extend_from_slice(&[1; 256 * 4]);
        assert!(decode(raw(&wide, &duplicate).as_slice()).is_err());
        let valid = raw(&doc, &0u32.to_le_bytes());
        for end in [0, 1, valid.len() / 2, valid.len() - 1] {
            assert!(decode(&valid[..end]).is_err());
        }
        let mut checksum = valid.clone();
        let crc = checksum.len() - 8;
        checksum[crc] ^= 1;
        assert!(decode(checksum.as_slice()).is_err());
        let mut extra = valid.clone();
        extra.extend_from_slice(b"unexpected");
        assert!(decode(extra.as_slice()).is_err());
        let mut members = valid.clone();
        members.extend_from_slice(&valid);
        assert!(decode(members.as_slice()).is_err());
        let mut oversized = GzEncoder::new(Vec::new(), Compression::fast());
        oversized.write_all(&u32::MAX.to_le_bytes()).unwrap();
        assert!(decode(oversized.finish().unwrap().as_slice()).is_err());
    }
}
