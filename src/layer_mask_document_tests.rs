//! Persistence, ownership and independently expected compositing for layer masks.
use crate::{
    document::{Document, History, Layer},
    layer_mask::{LayerMask, Mask},
};
use std::io::{Read, Write};
fn attach(doc: &mut Document, index: usize, coverage: Mask) {
    doc.version = 2;
    doc.layers[index].mask = Some(LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage,
    });
}
fn fixture() -> Document {
    let mut doc = Document::new(65, 67);
    for y in 0..67 {
        for x in 0..65 {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut_at(65, x, y)
                .copy_from_slice(&[x as u8, (y * 3) as u8, 173, if x == 64 { 0 } else { 129 }]);
        }
    }
    let coverage: Vec<u8> = (0..65 * 67)
        .map(|i| ((i * 19 + i / 65) % 256) as u8)
        .collect();
    attach(
        &mut doc,
        0,
        Mask::from_coverage(65, 67, &coverage, None).unwrap(),
    );
    doc.layers[0].mask.as_mut().unwrap().linked = false;
    doc
}
fn json(doc: &Document) -> serde_json::Value {
    serde_json::to_value(doc).unwrap()
}
#[test]
fn mask_validation_version_gate_and_bounded_json_roundtrip() {
    let mut doc = fixture();
    doc.validate().unwrap();
    let wire = serde_json::to_vec(&doc).unwrap();
    let restored: Document = serde_json::from_slice(&wire).unwrap();
    restored.validate().unwrap();
    assert_eq!(json(&restored), json(&doc));
    doc.version = 1;
    assert!(doc.validate().is_err());
    doc.version = 2;
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(64, 67, 255).unwrap();
    assert!(doc.validate().is_err());
    let doc = fixture();
    let metadata = crate::binary_project::metadata(&doc).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&metadata).unwrap();
    assert_eq!(
        value["layers"][0]["mask"],
        serde_json::json!({"enabled":true,"linked":false})
    );
    let placeholder: Document = serde_json::from_slice(&metadata).unwrap();
    assert!(placeholder.validate_metadata().is_ok());
    assert!(placeholder.validate().is_err());
    let mut bytes = Vec::new();
    doc.layers[0]
        .mask
        .as_ref()
        .unwrap()
        .coverage
        .encode(&mut bytes, None)
        .unwrap();
    bytes.push(0);
    assert!(serde_json::from_value::<Mask>(serde_json::json!(bytes)).is_err());
    assert!(serde_json::from_value::<Mask>(serde_json::json!([0, 1, 2])).is_err());
}
#[test]
fn native_project_recovery_and_json_preserve_folder_masks_flags_and_exact_pixels() {
    let mut doc = fixture();
    doc.add_folder().unwrap();
    let folder = doc.layers.len() - 1;
    attach(&mut doc, folder, Mask::new(65, 67, 128).unwrap());
    doc.layers[folder].mask.as_mut().unwrap().enabled = false;
    doc.selection_mask =
        Some(crate::selection_mask::Mask::from_coverage(65, 67, vec![128; 65 * 67]).unwrap());
    doc.selection = doc.selection_mask.as_ref().unwrap().bounds();
    let expected = json(&doc);
    let transparent = doc.composite_transparent();
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("masked.dudel");
    let recovery = directory.path().join("masked.gz");
    crate::storage::save_project(&project, &doc).unwrap();
    assert!(
        std::fs::read(&project)
            .unwrap()
            .starts_with(crate::tile_project::MASK_MAGIC)
    );
    let reopened = crate::storage::load_project(&project).unwrap();
    assert_eq!(json(&reopened), expected);
    assert_eq!(reopened.composite_transparent(), transparent);
    crate::storage::save_recovery(
        &recovery,
        &crate::storage::Recovery {
            document: doc.clone(),
            original_path: Some(project.clone()),
        },
    )
    .unwrap();
    let restored = crate::storage::read_recovery(&recovery).unwrap();
    assert_eq!(restored.original_path, Some(project));
    assert_eq!(json(&restored.document), expected);
    let legacy_json = directory.path().join("json.dudel");
    std::fs::write(&legacy_json, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(
        json(&crate::storage::load_project(&legacy_json).unwrap()),
        expected
    );
    assert!(crate::binary_project::encode(&doc, Vec::new()).is_err());
}
#[test]
fn unmasked_encoding_stays_v4_and_masked_metadata_cannot_be_smuggled_into_old_containers() {
    let unmasked = Document::new(2, 2);
    let mut bytes = Vec::new();
    crate::tile_project::encode(&unmasked, &mut bytes).unwrap();
    assert!(bytes.starts_with(crate::tile_project::MAGIC));
    let metadata = crate::binary_project::metadata(&unmasked).unwrap();
    assert!(!String::from_utf8(metadata).unwrap().contains("mask"));
    let mut masked = unmasked;
    attach(&mut masked, 0, Mask::new(2, 2, 255).unwrap());
    let mut bytes = Vec::new();
    crate::tile_project::encode(&masked, &mut bytes).unwrap();
    assert!(crate::tile_project::decode(&bytes[crate::tile_project::MASK_MAGIC.len()..]).is_err());
    assert!(
        crate::binary_project::decode(&bytes[crate::tile_project::MASK_MAGIC.len()..]).is_err()
    );
    assert!(
        crate::tile_project::decode_single_layer(
            &bytes[crate::tile_project::MASK_MAGIC.len()..],
            1024
        )
        .is_err()
    );
}
#[test]
fn malformed_new_format_and_failed_saves_preserve_prior_artwork() {
    let mut doc = Document::new(2, 2);
    let mut mask = Mask::new(2, 2, 255).unwrap();
    mask.writer().set(0, 0, 0);
    attach(&mut doc, 0, mask);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.dudel");
    crate::storage::save_project(&path, &doc).unwrap();
    let encoded = std::fs::read(&path).unwrap();
    let mut payload = Vec::new();
    flate2::read::GzDecoder::new(&encoded[crate::tile_project::MASK_MAGIC.len()..])
        .read_to_end(&mut payload)
        .unwrap();
    let metadata_len = u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize;
    let offset = 4 + metadata_len + 4;
    assert_eq!(&payload[offset..offset + 5], b"LMAS\x01");
    let mut variants = vec![
        payload[..payload.len() - 1].to_vec(),
        [payload.as_slice(), b"extra"].concat(),
    ];
    let mut bad = payload.clone();
    bad[offset + 5..offset + 9].copy_from_slice(&3u32.to_le_bytes());
    variants.push(bad);
    let mut bad = payload.clone();
    bad[offset + 14..offset + 18].copy_from_slice(&2u32.to_le_bytes());
    variants.push(bad);
    for raw in variants {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&raw).unwrap();
        assert!(crate::tile_project::decode_masked(gzip.finish().unwrap().as_slice()).is_err());
    }
    for length in [0, encoded.len() - 1, encoded.len() - 7] {
        std::fs::write(directory.path().join("bad.dudel"), &encoded[..length]).unwrap();
        assert!(crate::storage::load_project(&directory.path().join("bad.dudel")).is_err());
    }
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(1, 2, 255).unwrap();
    assert!(crate::storage::save_project(&path, &doc).is_err());
    assert_eq!(std::fs::read(path).unwrap(), encoded);
}
#[test]
fn coverage_compositing_matches_independent_alpha_oracle_and_disabled_mask_is_identity() {
    let mut doc = fixture();
    let source = doc.layers[0].pixels.clone();
    let result = doc.composite_transparent();
    for y in 0..67 {
        for x in 0..65 {
            let pixel = source.rgba(y * 65 + x);
            let coverage = ((y * 65 + x) * 19 + y) % 256;
            let expected = if pixel[3] == 0 || coverage == 0 {
                [0; 4]
            } else {
                [
                    pixel[0],
                    pixel[1],
                    pixel[2],
                    ((u32::from(pixel[3]) * coverage as u32 + 127) / 255) as u8,
                ]
            };
            assert_eq!(&result[(y * 65 + x) * 4..(y * 65 + x + 1) * 4], &expected);
        }
    }
    doc.layers[0].mask.as_mut().unwrap().enabled = false;
    let disabled = doc.composite_transparent();
    doc.layers[0].mask = None;
    assert_eq!(doc.composite_transparent(), disabled);
    assert!(source.shares(&doc.layers[0].pixels));
}
#[test]
fn masks_scale_clipping_base_and_isolated_folder_coverage_once() {
    let mut doc = Document::new(1, 1);
    doc.layers[0].pixels = vec![255, 0, 0, 255].into();
    doc.add_layer();
    doc.layers[1].pixels = vec![0, 0, 255, 255].into();
    doc.layers[1].clipped = true;
    attach(&mut doc, 0, Mask::new(1, 1, 128).unwrap());
    assert_eq!(doc.composite_transparent(), [0, 0, 255, 128]);
    doc.layers[0].depth = 1;
    doc.layers[1].depth = 1;
    doc.layers.push(Layer::new_folder("Group".into()));
    attach(&mut doc, 2, Mask::new(1, 1, 128).unwrap());
    doc.validate().unwrap();
    assert_eq!(doc.composite_transparent(), [0, 0, 255, 64]);
    doc.layers[2].mask.as_mut().unwrap().enabled = false;
    assert_eq!(doc.composite_transparent(), [0, 0, 255, 128]);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(1, 1, 0).unwrap();
    assert_eq!(doc.composite_transparent(), [0, 0, 0, 0]);
}
#[test]
fn mask_coverage_state_and_version_changes_have_exact_undo_redo_and_strict_source_guards() {
    let mut doc = Document::new(65, 67);
    let original = json(&doc);
    let mut history = History::new();
    history.checkpoint(&doc);
    attach(&mut doc, 0, Mask::new(65, 67, 255).unwrap());
    history.settle(&doc);
    let attached = json(&doc);
    let frozen = doc.clone();
    history.checkpoint(&doc);
    doc.layers[0]
        .mask
        .as_mut()
        .unwrap()
        .coverage
        .writer()
        .set(64, 66, 0);
    history.settle(&doc);
    let painted = json(&doc);
    assert!(!crate::document::same_snapshot(&doc, &frozen));
    history.checkpoint(&doc);
    doc.layers[0].mask.as_mut().unwrap().enabled = false;
    doc.layers[0].mask.as_mut().unwrap().linked = false;
    history.settle(&doc);
    let toggled = json(&doc);
    for expected in [&painted, &attached, &original] {
        assert!(history.undo(&mut doc));
        assert_eq!(&json(&doc), expected);
    }
    assert!(!history.can_undo());
    for expected in [&attached, &painted, &toggled] {
        assert!(history.redo(&mut doc));
        assert_eq!(&json(&doc), expected);
    }
    assert!(!history.can_redo());
    assert_eq!(
        frozen.layers[0].mask.as_ref().unwrap().coverage.at(64, 66),
        255
    );
}
#[test]
fn snapshot_memory_tracks_only_detached_mask_allocations_and_keeps_worker_sources() {
    let mut doc = fixture();
    let original = doc.clone();
    let descriptor = crate::snapshot_memory::Snapshot::capture(&original);
    assert_eq!(
        crate::snapshot_memory::beyond_live(&doc, [&descriptor]).pixel_bytes_beyond_live,
        0
    );
    doc.layers[0]
        .mask
        .as_mut()
        .unwrap()
        .coverage
        .writer()
        .set(64, 66, 0);
    let memory = crate::snapshot_memory::beyond_live(&doc, [&descriptor]);
    assert!(memory.pixel_bytes_beyond_live > 0 && memory.pixel_bytes_beyond_live < 16384);
    let mut registry = crate::snapshot_memory::Registry::new(&doc);
    registry.add_document(&original);
    let once = registry.additional_bytes();
    // Layer metadata is per document; coverage payload must deduplicate.
    registry.add_snapshot(&descriptor);
    assert_eq!(
        registry.additional_bytes() - once,
        std::mem::size_of::<Document>()
            + original.layers.capacity() * std::mem::size_of::<Layer>()
            + original.layers[0].name.capacity()
    );
}
#[test]
fn masked_project_loading_accepts_clean_canvas_and_preserves_unsaved_canvas_until_answered() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("masked.dudel");
    crate::storage::save_project(&path, &fixture()).unwrap();
    for modified in [false, true] {
        let mut app = crate::Dudel {
            doc: Document::new(3, 2),
            modified,
            ..Default::default()
        };
        let original = app.doc.clone();
        app.load_document(path.clone(), false);
        crate::workflow_tests::finish_jobs(&mut app);
        if modified {
            assert!(crate::document::same_snapshot(&app.doc, &original));
            assert!(app.unsaved_action.is_some() && app.modified);
        } else {
            assert_eq!(
                serde_json::to_value(&app.doc).unwrap(),
                serde_json::to_value(fixture()).unwrap()
            );
            assert_eq!(app.path.as_ref(), Some(&path));
            assert!(!app.modified && app.unsaved_action.is_none());
            assert!(app.mask_controls_available());
        }
    }
}
#[test]
fn indexed_writes_reject_masks_before_creating_or_modifying_a_destination() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.didx");
    let missing = directory.path().join("new.didx");
    let masked = fixture();
    assert!(crate::indexed_commit::Session::save(None, &missing, &masked, &|| false).is_err());
    assert!(!missing.exists());
    let plain = Document::new(2, 2);
    let session = crate::indexed_commit::Session::save(None, &path, &plain, &|| false).unwrap();
    drop(session);
    let bytes = std::fs::read(&path).unwrap();
    assert!(crate::indexed_commit::Session::save(None, &path, &masked, &|| false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let mut emitted = 0;
    assert!(
        crate::indexed_document::visit(
            &masked,
            &crate::indexed_document::Keys::default(),
            &|| false,
            &mut |_, _| {
                emitted += 1;
                Ok(())
            }
        )
        .is_err()
    );
    assert_eq!(emitted, 0);
}
#[test]
fn combined_document_payload_budget_includes_masks_without_raising_supported_limits() {
    let mut doc = Document::new(2048, 2048);
    let shared: crate::pixels::Pixels = [1, 2, 3, 255].repeat(2048 * 2048).into();
    for index in 0..crate::document::MAX_MASKED_PAYLOAD / (2048 * 2048 * 4) {
        if index != 0 {
            doc.add_layer();
        }
        doc.layers[index].pixels = shared.clone();
    }
    attach(&mut doc, 0, Mask::new(2048, 2048, 255).unwrap());
    assert!(doc.validate().is_ok());
    doc.layers[0]
        .mask
        .as_mut()
        .unwrap()
        .coverage
        .writer()
        .set(0, 0, 0);
    assert!(doc.validate().unwrap_err().contains("payload budget"));
}
#[test]
fn returning_mask_bytes_to_default_encodes_canonically_without_mutating_snapshots() {
    let mut doc = Document::new(2, 2);
    let mut mask = Mask::new(2, 2, 255).unwrap();
    mask.writer().set(0, 0, 0);
    mask.writer().set(0, 0, 255);
    let snapshot = mask.clone();
    attach(&mut doc, 0, mask);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("restored.dudel");
    crate::storage::save_project(&path, &doc).unwrap();
    let reopened = crate::storage::load_project(&path).unwrap();
    assert_eq!(
        reopened.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        255
    );
    assert_eq!(
        reopened.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        0
    );
    assert!(
        doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .shares(&snapshot)
    );
}
