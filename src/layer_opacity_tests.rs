use crate::{
    document::{Document, History},
    pixels::Pixels,
};

#[test]
fn csp_coverage_matches_integer_floor_for_all_bytes_and_legacy_stays_continuous() {
    let mut doc = Document::new(256, 1);
    doc.layers[0].pixels = [0, 0, 0, 255].repeat(256).into();
    doc.add_layer();
    doc.layers[1].opacity = 156. / 256.;
    let source: Vec<_> = (0..=255u8).flat_map(|alpha| [0, 255, 0, alpha]).collect();
    doc.layers[1].pixels = source.clone().into();
    let legacy = doc.composite_transparent();
    doc.layers[1].csp_byte_opacity = true;
    let csp = doc.composite_transparent();
    for alpha in 0..256usize {
        assert_eq!(csp[alpha * 4 + 1], (alpha * 156 / 256) as u8);
        assert_eq!(
            legacy[alpha * 4 + 1],
            (alpha as f32 * 156. / 256.).round() as u8
        );
    }
    assert_eq!(doc.layers[1].pixels.as_slice(), source);
    assert_eq!(doc.layers[1].opacity, 156. / 256.);
    let mut folder = crate::document::Layer::new_folder("unverified group mode".into());
    folder.csp_byte_opacity = true;
    doc.layers.push(folder);
    assert!(doc.validate_metadata().is_err());
}

#[test]
fn mode_survives_history_json_duplicate_and_pending_properties_with_legacy_default() {
    let mut doc = Document::new(1, 1);
    doc.layers[0].pixels = Pixels::from(vec![10, 20, 30, 1]);
    doc.layers[0].opacity = 156. / 256.;
    let old = doc.clone();
    let mut history = History::new();
    history.checkpoint(&doc);
    doc.layers[0].csp_byte_opacity = true;
    history.settle(&doc);
    assert!(!crate::document::same_snapshot(&doc, &old));
    assert!(history.undo(&mut doc));
    assert!(!doc.layers[0].csp_byte_opacity);
    assert!(history.redo(&mut doc));
    assert!(doc.layers[0].csp_byte_opacity);
    let encoded = serde_json::to_vec(&doc).unwrap();
    let loaded: Document = serde_json::from_slice(&encoded).unwrap();
    assert!(loaded.layers[0].csp_byte_opacity);
    let mut json = serde_json::to_value(&doc).unwrap();
    json["layers"][0]
        .as_object_mut()
        .unwrap()
        .remove("csp_byte_opacity");
    let legacy: Document = serde_json::from_value(json).unwrap();
    assert!(!legacy.layers[0].csp_byte_opacity);
    let pending = crate::pending_layers::PendingLayers::from_document(&doc);
    let id = doc.layers[0].id;
    assert!(pending.properties(id).unwrap().csp_byte_opacity);
    let (next, copies, selected) = pending.with_duplicate(id).unwrap();
    let duplicated = crate::document_edits::Edit::DuplicateTarget { target: id, copies }
        .prepare(&doc)
        .unwrap()
        .0;
    assert!(next.properties(selected).unwrap().csp_byte_opacity);
    assert!(duplicated.layers[duplicated.active].csp_byte_opacity);
}

#[test]
fn prefix_metadata_invalidates_when_only_opacity_mode_changes() {
    let mut doc = Document::new(1, 1);
    doc.layers[0].pixels = vec![0, 0, 0, 255].into();
    doc.add_layer();
    doc.layers[1].pixels = vec![0, 255, 0, 1].into();
    doc.layers[1].opacity = 156. / 256.;
    doc.add_layer();
    let mut cache = crate::prefix_cache::PrefixCache::default();
    let before = cache.composite_region(&doc, [0, 0, 1, 1], false);
    doc.layers[1].csp_byte_opacity = true;
    let after = cache.composite_region(&doc, [0, 0, 1, 1], false);
    assert_ne!(before, after);
    assert_eq!(after, doc.composite_transparent());
}
