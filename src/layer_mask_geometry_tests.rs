//! Mask geometry uses independent coordinate/coverage oracles, plus queue ownership.
use crate::{
    affine::{Matrix, Sampling},
    document::{Document, History, Layer},
    layer_mask::{LayerMask, Mask},
    transform::Transform,
};
use std::sync::atomic::AtomicBool;
fn source(w: usize, h: usize) -> Document {
    let mut doc = Document::new(w, h);
    doc.version = 2;
    for y in 0..h {
        for x in 0..w {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut_at(w, x, y)
                .copy_from_slice(&[x as u8, y as u8, 83, if x % 3 == 0 { 0 } else { 129 }]);
        }
    }
    let bytes: Vec<u8> = (0..w * h)
        .map(|i| ((i * 37 + i / w * 13) % 256) as u8)
        .collect();
    doc.layers[0].mask = Some(LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage: Mask::from_coverage(w, h, &bytes, None).unwrap(),
    });
    doc
}
fn at(doc: &Document, index: usize, x: usize, y: usize) -> u8 {
    doc.layers[index].mask.as_ref().unwrap().coverage.at(x, y)
}
fn json(doc: &Document) -> serde_json::Value {
    serde_json::to_value(doc).unwrap()
}
#[test]
fn all_canvas_anchors_map_every_mask_including_disabled_unlinked_and_folder_entries() {
    let mut original = source(65, 67);
    original.add_layer();
    original.layers[1].mask = original.layers[0].mask.clone();
    original.layers[1].mask.as_mut().unwrap().enabled = false;
    original.layers[1].mask.as_mut().unwrap().linked = false;
    original.layers[0].depth = 1;
    original.layers[1].depth = 1;
    original.layers.push(Layer::new_folder("Folder".into()));
    original.layers[2].mask = original.layers[0].mask.clone();
    let frozen = original.clone();
    for [w, h] in [[69, 71], [61, 63], [61, 71]] {
        for ay in 0..3 {
            for ax in 0..3 {
                let settings = crate::canvas_size::Settings {
                    width: w,
                    height: h,
                    anchor: [ax, ay],
                };
                let offset = settings.offset(65, 67);
                let (result, changed) =
                    crate::canvas_size::resize(&original, settings, None).unwrap();
                assert!(changed);
                result.validate().unwrap();
                for index in 0..3 {
                    for y in 0..h {
                        for x in 0..w {
                            let (sx, sy) = (x as i64 - offset[0], y as i64 - offset[1]);
                            let expected = if sx >= 0 && sy >= 0 && sx < 65 && sy < 67 {
                                at(&original, index, sx as usize, sy as usize)
                            } else {
                                255
                            };
                            assert_eq!(at(&result, index, x, y), expected);
                        }
                    }
                }
                assert!(
                    !result.layers[1].mask.as_ref().unwrap().enabled
                        && !result.layers[1].mask.as_ref().unwrap().linked
                );
                assert!(crate::document::same_snapshot(&original, &frozen));
            }
        }
    }
}
#[test]
fn quarter_turns_preserve_mask_flags_and_exact_coverage_and_cancel_atomically() {
    let mut doc = source(65, 67);
    doc.layers[0].mask.as_mut().unwrap().enabled = false;
    doc.layers[0].mask.as_mut().unwrap().linked = false;
    let original = doc.clone();
    let original_json = json(&doc);
    doc.rotate_canvas_clockwise_control(None).unwrap();
    doc.validate().unwrap();
    assert_eq!([doc.width, doc.height], [67, 65]);
    for y in 0..65 {
        for x in 0..67 {
            assert_eq!(at(&doc, 0, x, y), at(&original, 0, y, 66 - x));
        }
    }
    for _ in 0..3 {
        doc.rotate_canvas_clockwise_control(None).unwrap();
    }
    assert_eq!(json(&doc), original_json);
    let frozen = doc.clone();
    let cancel = AtomicBool::new(true);
    assert!(doc.rotate_canvas_clockwise_control(Some(&cancel)).is_err());
    assert!(crate::document::same_snapshot(&doc, &frozen));
    assert!(
        crate::canvas_size::resize(
            &doc,
            crate::canvas_size::Settings {
                width: 70,
                height: 71,
                anchor: [1, 1]
            },
            Some(&cancel)
        )
        .is_err()
    );
    assert!(crate::document::same_snapshot(&doc, &frozen));
}
#[test]
fn linked_whole_layer_flips_follow_artwork_and_selections_or_unlinked_masks_stay_put() {
    for linked in [false, true] {
        for enabled in [false, true] {
            for selected in [false, true] {
                for transform in [
                    Transform::FlipHorizontal,
                    Transform::FlipVertical,
                    Transform::RotateHalfTurn,
                ] {
                    let mut doc = source(65, 67);
                    let mask = doc.layers[0].mask.as_mut().unwrap();
                    mask.linked = linked;
                    mask.enabled = enabled;
                    if selected {
                        doc.selection = Some(crate::selection::Selection {
                            left: 1,
                            top: 1,
                            right: 64,
                            bottom: 66,
                        });
                    }
                    let original = doc.clone();
                    doc.transform_active(transform).unwrap();
                    for y in 0..67 {
                        for x in 0..65 {
                            let [sx, sy] = if linked && !selected {
                                match transform {
                                    Transform::FlipHorizontal => [64 - x, y],
                                    Transform::FlipVertical => [x, 66 - y],
                                    Transform::RotateHalfTurn => [64 - x, 66 - y],
                                }
                            } else {
                                [x, y]
                            };
                            assert_eq!(at(&doc, 0, x, y), at(&original, 0, sx, sy));
                        }
                    }
                    if !linked || selected {
                        assert!(
                            doc.layers[0]
                                .mask
                                .as_ref()
                                .unwrap()
                                .coverage
                                .shares(&original.layers[0].mask.as_ref().unwrap().coverage)
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn mask_only_flips_are_undoable_and_uniform_masks_keep_identity_on_noop() {
    let mut doc = Document::new(4, 2);
    doc.version = 2;
    let mut mask = Mask::new(4, 2, 255).unwrap();
    mask.writer().set(0, 0, 0);
    doc.layers[0].mask = Some(LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage: mask,
    });
    let original = doc.clone();
    let mut history = History::new();
    history.checkpoint(&doc);
    assert!(doc.transform_active(Transform::FlipHorizontal).unwrap());
    history.settle(&doc);
    assert!(doc.layers[0].pixels.shares(&original.layers[0].pixels));
    assert_eq!(at(&doc, 0, 3, 0), 0);
    assert!(history.undo(&mut doc));
    assert_eq!(json(&doc), json(&original));
    assert!(!history.can_undo());
    assert!(history.redo(&mut doc));
    assert_eq!(at(&doc, 0, 3, 0), 0);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(4, 2, 255).unwrap();
    let frozen = doc.clone();
    assert!(!doc.transform_active(Transform::FlipHorizontal).unwrap());
    assert!(crate::document::same_snapshot(&doc, &frozen));
}
#[test]
fn integer_move_and_affine_sampling_agree_on_linked_mask_coordinates_and_outside_reveal() {
    for linked in [false, true] {
        for enabled in [false, true] {
            for selected in [false, true] {
                let mut original = source(65, 67);
                let mask = original.layers[0].mask.as_mut().unwrap();
                mask.linked = linked;
                mask.enabled = enabled;
                if selected {
                    let selection =
                        crate::selection_mask::Mask::from_coverage(65, 67, vec![128; 65 * 67])
                            .unwrap();
                    original.selection = selection.bounds();
                    original.selection_mask = Some(selection);
                }
                for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Bicubic] {
                    let mut moved = original.clone();
                    crate::move_preview::apply(&mut moved, [3, -2], None).unwrap();
                    let mut affine = original.clone();
                    crate::affine::apply(
                        &mut affine,
                        Matrix([1.0, 0.0, 0.0, 1.0, 3.0, -2.0]),
                        sampling,
                        None,
                    )
                    .unwrap();
                    for y in 0..67 {
                        for x in 0..65 {
                            let expected = if linked && !selected {
                                if x >= 3 && y + 2 < 67 {
                                    at(&original, 0, x - 3, y + 2)
                                } else {
                                    255
                                }
                            } else {
                                at(&original, 0, x, y)
                            };
                            assert_eq!(at(&moved, 0, x, y), expected);
                            assert_eq!(at(&affine, 0, x, y), expected);
                        }
                    }
                    let first = (0..65 * 67).find(|&i| {
                        moved.layers[0].pixels.rgba(i) != affine.layers[0].pixels.rgba(i)
                    });
                    assert!(
                        first.is_none(),
                        "Raster mismatch linked={linked} enabled={enabled} selected={selected} sampling={sampling:?}: {:?}",
                        first.map(|i| (
                            i,
                            moved.layers[0].pixels.rgba(i),
                            affine.layers[0].pixels.rgba(i)
                        ))
                    );
                }
            }
        }
    }
}
#[test]
fn fractional_layer_mask_sampling_uses_straight_grayscale_with_clamped_cubic_edges() {
    let mut original = Document::new(4, 1);
    original.version = 2;
    original.layers[0].mask = Some(LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage: Mask::from_coverage(4, 1, &[0, 64, 128, 255], None).unwrap(),
    });
    for (sampling, expected) in [
        (Sampling::Nearest, [0, 64, 128, 255]),
        (Sampling::Bilinear, [128, 32, 96, 192]),
        (Sampling::Bicubic, [124, 12, 92, 196]),
    ] {
        let mut doc = original.clone();
        crate::affine::apply(
            &mut doc,
            Matrix([1.0, 0.0, 0.0, 1.0, 0.5, 0.0]),
            sampling,
            None,
        )
        .unwrap();
        for (x, expected) in expected.into_iter().enumerate() {
            assert_eq!(at(&doc, 0, x, 0), expected);
        }
        assert!(doc.layers[0].pixels.shares(&original.layers[0].pixels));
    }
}
#[test]
fn move_preview_net_zero_restores_shared_mask_and_queued_cancel_restores_mask_before_later_edits() {
    let original = source(65, 67);
    let mut doc = original.clone();
    let mut engine = crate::move_preview::Engine::new(&doc);
    engine.request([5, 2]);
    let candidate = engine.finish(&doc).unwrap().unwrap();
    engine.publish(&mut doc, candidate).unwrap();
    assert!(engine.net_changed);
    engine.request([0, 0]);
    let candidate = engine.finish(&doc).unwrap().unwrap();
    engine.publish(&mut doc, candidate).unwrap();
    assert!(!engine.net_changed);
    assert!(crate::document::same_snapshot(&doc, &original));
    engine.request([5, 2]);
    let candidate = engine.finish(&doc).unwrap().unwrap();
    let mut app = crate::Dudel {
        doc,
        ..Default::default()
    };
    app.history.begin_gesture_checkpoint(&app.doc);
    engine.publish(&mut app.doc, candidate).unwrap();
    assert!(app.render_queue.finish_move(engine, true, false).is_ok());
    assert!(app.render_queue.cancel_background_edit());
    let id = app.doc.layers[0].id;
    app.render_queue
        .edit(crate::document_edits::Edit::Property {
            target: crate::document_edits::Target::Identity(id),
            property: crate::document_edits::Property::Name("After cancel".into()),
        })
        .unwrap();
    crate::workflow_tests::finish_jobs(&mut app);
    assert!(
        app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .shares(&original.layers[0].mask.as_ref().unwrap().coverage)
    );
    assert_eq!(app.doc.layers[0].name, "After cancel");
    app.undo();
    crate::workflow_tests::finish_jobs(&mut app);
    assert_eq!(json(&app.doc), json(&original));
}
#[test]
fn stale_mask_workers_and_pre_canceled_affine_preserve_newer_state() {
    let original = source(65, 67);
    let mut doc = original.clone();
    let (mut engine, gate) = crate::move_preview::Engine::gated(&doc, [2, 0]);
    doc.layers[0]
        .mask
        .as_mut()
        .unwrap()
        .coverage
        .writer()
        .set(0, 0, 255);
    let newer = doc.clone();
    gate.send(()).unwrap();
    assert!(engine.finish(&doc).is_err());
    assert!(crate::document::same_snapshot(&doc, &newer));
    let cancel = AtomicBool::new(true);
    assert!(
        crate::affine::apply(
            &mut doc,
            Matrix([1.0, 0.0, 0.0, 1.0, 2.0, 0.0]),
            Sampling::Bilinear,
            Some(&cancel)
        )
        .is_err()
    );
    assert!(crate::document::same_snapshot(&doc, &newer));
}
#[test]
fn queued_canvas_geometry_moves_save_export_and_undo_keep_mask_state() {
    use crate::document_edits::Edit;
    let original = source(9, 7);
    let id = original.layers[0].id;
    let mut app = crate::Dudel {
        doc: original.clone(),
        ..Default::default()
    };
    let edits = [
        Edit::CanvasSize(crate::canvas_size::Settings {
            width: 13,
            height: 11,
            anchor: [1, 1],
        }),
        Edit::Rotate(1),
        Edit::MoveTarget {
            target: id,
            offset: [1, -1],
        },
    ];
    let mut expected = original.clone();
    for edit in &edits {
        expected = edit.prepare(&expected).unwrap().0;
        app.render_queue.edit(edit.clone()).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("geometry.dudel");
    let png = dir.path().join("geometry.png");
    app.path = Some(path.clone());
    app.save(false);
    app.export_to(png.clone(), true);
    crate::workflow_tests::finish_jobs(&mut app);
    assert!(
        json(&app.doc) == json(&expected),
        "Live queued artwork differs"
    );
    let mut saved = crate::storage::load_project(&path).unwrap();
    saved.guides = None;
    assert!(
        json(&saved) == json(&expected),
        "Saved artwork and masks differ from queued result"
    );
    assert_eq!(
        image::open(png).unwrap().into_rgba8().as_raw(),
        &expected.composite_transparent()
    );
    for _ in 0..3 {
        app.undo();
        crate::workflow_tests::finish_jobs(&mut app);
    }
    assert_eq!(json(&app.doc), json(&original));
}
#[test]
fn perspective_and_mesh_transform_the_linked_mask_with_the_same_inverse_geometry() {
    use crate::affine::{Mesh, Settings, Warp};
    let original = source(65, 67);
    // x'=x/(1+k*y), y'=y/(1+k*y); the independent inverse is
    // x=X/(1-k*Y), y=Y/(1-k*Y). These samples avoid integer boundaries.
    let k = 0.0073;
    let mut perspective = original.clone();
    let settings = Settings {
        warp: Some(Warp::Corners([
            [0.0, 0.0],
            [65.0, 0.0],
            [65.0 / (1.0 + k * 67.0), 67.0 / (1.0 + k * 67.0)],
            [0.0, 67.0 / (1.0 + k * 67.0)],
        ])),
        sampling: Sampling::Nearest,
        ..Default::default()
    };
    let mapping = settings.mapping(&perspective).unwrap();
    crate::affine::apply_mapping(&mut perspective, mapping, Sampling::Nearest, None).unwrap();
    for y in 0..67 {
        for x in 0..65 {
            let denominator = 1.0 - k * (y as f64 + 0.5);
            let sx = ((x as f64 + 0.5) / denominator).floor() as usize;
            let sy = ((y as f64 + 0.5) / denominator).floor() as usize;
            let expected = if sx < 65 && sy < 67 {
                at(&original, 0, sx, sy)
            } else {
                255
            };
            assert_eq!(at(&perspective, 0, x, y), expected);
        }
    }
    let mut mesh = original.clone();
    let settings = Settings {
        warp: Some(Warp::Mesh(Mesh::from_fn(4, 4, |c, r| {
            [65.0 * c as f64 / 3.0 + 3.0, 67.0 * r as f64 / 3.0 - 2.0]
        }))),
        sampling: Sampling::Nearest,
        ..Default::default()
    };
    let mapping = settings.mapping(&mesh).unwrap();
    crate::affine::apply_mapping(&mut mesh, mapping, Sampling::Nearest, None).unwrap();
    for y in 0..67 {
        for x in 0..65 {
            assert_eq!(
                at(&mesh, 0, x, y),
                if x >= 3 && y + 2 < 67 {
                    at(&original, 0, x - 3, y + 2)
                } else {
                    255
                }
            );
        }
    }
}
#[test]
fn over_budget_mask_geometry_fails_before_replacing_the_document() {
    let mut doc = Document::new(2048, 2048);
    doc.version = 2;
    let pixels: crate::pixels::Pixels = [1, 2, 3, 255].repeat(2048 * 2048).into();
    for i in 0..crate::document::MAX_MASKED_PAYLOAD / (2048 * 2048 * 4) {
        if i != 0 {
            doc.add_layer();
        }
        doc.layers[i].pixels = pixels.clone();
    }
    doc.active = 0;
    doc.layers[0].mask = Some(LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage: Mask::new(2048, 2048, 0).unwrap(),
    });
    doc.validate().unwrap();
    let original = doc.clone();
    assert!(
        crate::move_preview::apply(&mut doc, [1, 0], None)
            .unwrap_err()
            .contains("payload budget")
    );
    assert!(crate::document::same_snapshot(&doc, &original));
    assert!(
        crate::affine::apply(
            &mut doc,
            Matrix([1.0, 0.0, 0.0, 1.0, 1.0, 0.0]),
            Sampling::Nearest,
            None
        )
        .unwrap_err()
        .contains("payload budget")
    );
    assert!(crate::document::same_snapshot(&doc, &original));
}
#[test]
fn whole_layer_mask_flip_admits_to_worker_before_publishing_pixels_or_history() {
    let original = source(65, 67);
    let mut doc = original.clone();
    let mut history = History::new();
    let mut queue = crate::render_queue::Queue::default();
    queue
        .edit(crate::document_edits::Edit::Transform(
            Transform::FlipHorizontal,
        ))
        .unwrap();
    let progress = queue.advance(&mut doc, &mut history, 1).unwrap();
    assert!(progress.edits.is_empty() && queue.has_background_edit());
    assert!(crate::document::same_snapshot(&doc, &original));
    assert!(!history.can_undo());
    let mut app = crate::Dudel {
        doc,
        history,
        render_queue: queue,
        ..Default::default()
    };
    crate::workflow_tests::finish_jobs(&mut app);
    assert!(app.history.can_undo());
    assert_eq!(at(&app.doc, 0, 0, 0), at(&original, 0, 64, 0));
}
