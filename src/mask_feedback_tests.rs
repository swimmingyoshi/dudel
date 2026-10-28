use crate::{
    Dudel,
    document::{Document, LayerId},
    document_edits::Edit,
    layer_mask::{LayerMask, Mask},
    paint_target::Target,
    selection_mask::Combine,
    wand::Settings,
};
fn fixture() -> (Document, LayerId) {
    let mut doc = Document::new(5, 2);
    doc.version = 2;
    doc.layers[0].pixels = [17, 83, 201, 128].repeat(10).into();
    let id = LayerId::new();
    doc.layers[0].mask = Some(LayerMask {
        id,
        enabled: false,
        linked: false,
        coverage: Mask::from_coverage(5, 2, &[0, 0, 64, 255, 0, 0, 0, 128, 255, 0], None).unwrap(),
    });
    (doc, id)
}
#[test]
fn mask_colors_swap_independently_and_do_not_pollute_artwork_color_history() {
    let (doc, id) = fixture();
    let mut app = Dudel {
        mask_focus: Some((doc.layers[0].id, id)),
        doc,
        color: [17, 83, 201],
        secondary: [219, 107, 43],
        ..Default::default()
    };
    let colors = (app.color, app.secondary);
    assert_eq!(app.paint_color(), [0; 3]);
    app.mask_colors = [64, 192];
    app.swap_paint_colors();
    assert_eq!(app.paint_color(), [192; 3]);
    app.note_recent_color();
    assert!(app.recent_colors.is_empty());
    app.focus_current_mask(false);
    assert_eq!((app.color, app.secondary), colors);
    assert_eq!(app.paint_color(), colors.0);
    app.swap_paint_colors();
    assert_eq!((app.color, app.secondary), (colors.1, colors.0));
    app.focus_current_mask(true);
    assert_eq!(app.mask_colors, [192, 64]);
}
#[test]
fn active_mask_wand_uses_raw_coverage_with_locked_hidden_disabled_and_folder_targets() {
    let (mut doc, id) = fixture();
    let layer = doc.layers[0].id;
    let original = doc.clone();
    for folder in [false, true] {
        doc.layers[0].locked = true;
        doc.layers[0].visible = false;
        if folder {
            let mask = doc.layers[0].mask.clone();
            doc.layers[0] = crate::document::Layer::new_folder("Folder".into());
            doc.layers[0].id = layer;
            doc.layers[0].mask = mask;
        }
        for contiguous in [false, true] {
            let settings = Settings {
                mask: Some((layer, id)),
                tolerance: 0,
                contiguous,
                ..Default::default()
            };
            let selected = crate::wand::mask(&doc, [0, 0], settings, None).unwrap();
            for y in 0..2 {
                for x in 0..5 {
                    assert_eq!(
                        selected.at(x, y),
                        if x < 2 || (!contiguous && x == 4) {
                            255
                        } else {
                            0
                        }
                    );
                }
            }
            let mut app = Dudel {
                doc: doc.clone(),
                ..Default::default()
            };
            app.render_queue
                .edit(Edit::SelectionWand {
                    point: [2, 0],
                    settings,
                    mode: Combine::Replace,
                })
                .unwrap();
            app.flush_render_queue();
            assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(2, 0), 255);
            assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(2, 1), 0);
            assert!(app.history.undo(&mut app.doc));
            assert!(crate::document::same_snapshot(&app.doc, &doc));
        }
    }
    assert_eq!(original.layers[0].pixels.rgba(0), [17, 83, 201, 128]);
}
#[test]
fn stale_wand_slot_mismatched_reference_and_cancellation_leave_source_unchanged() {
    let (doc, id) = fixture();
    let layer = doc.layers[0].id;
    let before = doc.clone();
    for settings in [
        Settings {
            mask: Some((layer, LayerId::new())),
            ..Default::default()
        },
        Settings {
            mask: Some((layer, id)),
            source: crate::fill::Source::Visible,
            ..Default::default()
        },
    ] {
        assert!(crate::wand::mask(&doc, [0, 0], settings, None).is_err());
    }
    let cancel = std::sync::atomic::AtomicBool::new(true);
    assert!(
        crate::wand::mask(
            &doc,
            [0, 0],
            Settings {
                mask: Some((layer, id)),
                ..Default::default()
            },
            Some(&cancel)
        )
        .is_err()
    );
    assert!(crate::document::same_snapshot(&doc, &before));
    let legacy: Settings =
        serde_json::from_str(r#"{"source":"Active","tolerance":12,"contiguous":true}"#).unwrap();
    assert!(legacy.mask.is_none());
    assert!(serde_json::to_value(legacy).unwrap().get("mask").is_none());
}
#[test]
fn selection_coverage_defines_raw_masks_and_artwork_alpha_including_empty_and_stale_results() {
    let (doc, id) = fixture();
    let layer = doc.layers[0].id;
    for enabled in [false, true] {
        for target in [Target::Raster, Target::Mask(id)] {
            let mut source = doc.clone();
            source.layers[0].mask.as_mut().unwrap().enabled = enabled;
            source.layers[0].locked = true;
            source.layers[0].visible = false;
            source.layers[0].opacity = 0.25;
            let (selected, outcome) = Edit::SelectionCoverage {
                target: layer,
                paint_target: target,
            }
            .prepare(&source)
            .unwrap();
            assert!(outcome.changed);
            for y in 0..2 {
                for x in 0..5 {
                    let m = source.layers[0].mask.as_ref().unwrap().coverage.at(x, y);
                    let expected = if !target.is_raster() {
                        m
                    } else if enabled {
                        ((128u16 * u16::from(m) + 127) / 255) as u8
                    } else {
                        128
                    };
                    assert_eq!(selected.selection_mask.as_ref().unwrap().at(x, y), expected);
                }
            }
            assert_eq!(selected.layers[0].pixels, source.layers[0].pixels);
            assert!(crate::layer_mask::same_snapshot(
                &selected.layers[0].mask,
                &source.layers[0].mask
            ));
        }
    }
    assert!(
        Edit::SelectionCoverage {
            target: layer,
            paint_target: Target::Mask(LayerId::new())
        }
        .prepare(&doc)
        .is_err()
    );
    let mut empty = doc.clone();
    empty.layers[0].mask.as_mut().unwrap().coverage = Mask::new(5, 2, 0).unwrap();
    let (selected, _) = Edit::SelectionCoverage {
        target: layer,
        paint_target: Target::Mask(id),
    }
    .prepare(&empty)
    .unwrap();
    assert!(selected.selection.is_none());
    assert!(selected.selection_mask.as_ref().unwrap().bounds().is_none());
}

#[test]
fn mask_wand_contact_captures_focus_before_later_focus_changes_and_pending_creation() {
    let mut app = Dudel {
        doc: Document::new(5, 2),
        selection_shape: crate::selection_ui::Shape::Wand,
        ..Default::default()
    };
    assert!(app.create_canvas_mask(crate::layer_mask_edits::Seed::HideAll));
    let layer = app.doc.layers[0].id;
    let id = match app.input_stroke_target() {
        Target::Mask(id) => id,
        _ => panic!("Mask focus"),
    };
    app.update_selection_drag([0.0, 0.0], [0, 0], true, false, true);
    app.update_selection_drag([0.0, 0.0], [0, 0], false, true, true);
    app.focus_current_mask(false);
    // Changing focus after release cannot redirect the queued reference.
    app.flush_render_queue();
    assert_eq!(app.doc.layers[0].id, layer);
    assert_eq!(app.doc.layers[0].mask.as_ref().unwrap().id, id);
    assert!(
        app.doc
            .selection_mask
            .as_ref()
            .unwrap()
            .bytes()
            .iter()
            .all(|v| *v == 255)
    );
    assert!(app.input_stroke_target().is_raster());
    assert!(app.history.undo(&mut app.doc));
    assert!(app.doc.selection_mask.is_none());
}
