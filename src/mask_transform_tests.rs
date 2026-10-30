use super::*;
use crate::{
    affine::{Matrix, Mesh, Settings, Warp},
    document::{History, LayerId},
    layer_mask::LayerMask,
    paint_target::Target,
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn fixture(w: usize, h: usize) -> (Document, LayerId) {
    let mut doc = Document::new(w, h);
    doc.version = 2;
    doc.layers[0].pixels = [17, 83, 201, 127].repeat(w * h).into();
    let id = LayerId::new();
    doc.layers[0].mask = Some(LayerMask {
        id,
        enabled: false,
        linked: false,
        coverage: Mask::from_coverage(
            w,
            h,
            &(0..w * h).map(|i| (i * 37 % 256) as u8).collect::<Vec<_>>(),
            None,
        )
        .unwrap(),
    });
    (doc, id)
}
fn at(doc: &Document, x: usize, y: usize) -> u8 {
    doc.layers[0].mask.as_ref().unwrap().coverage.at(x, y)
}
fn unchanged_artwork(before: &Document, after: &Document) {
    assert_eq!(before.layers[0].pixels, after.layers[0].pixels);
    assert_eq!(
        before.layers[0].mask.as_ref().unwrap().enabled,
        after.layers[0].mask.as_ref().unwrap().enabled
    );
    assert_eq!(
        before.layers[0].mask.as_ref().unwrap().linked,
        after.layers[0].mask.as_ref().unwrap().linked
    );
    after.validate().unwrap();
}
#[test]
fn integer_moves_and_flips_change_only_captured_coverage_including_extreme_offsets() {
    let (original, id) = fixture(65, 67);
    for offset in [[3, -2], [i64::MIN, 0], [i64::MAX, i64::MAX]] {
        let mut doc = original.clone();
        assert!(apply(&mut doc, id, &Operation::Move(offset), None).unwrap());
        for y in 0..67 {
            for x in 0..65 {
                let (sx, sy) = (x as i128 - offset[0] as i128, y as i128 - offset[1] as i128);
                let expected = if (0..65).contains(&sx) && (0..67).contains(&sy) {
                    at(&original, sx as usize, sy as usize)
                } else {
                    255
                };
                assert_eq!(at(&doc, x, y), expected);
            }
        }
        unchanged_artwork(&original, &doc);
    }
    for (flip, h, v) in [
        (Transform::FlipHorizontal, true, false),
        (Transform::FlipVertical, false, true),
        (Transform::RotateHalfTurn, true, true),
    ] {
        let mut doc = original.clone();
        apply(&mut doc, id, &Operation::Flip(flip), None).unwrap();
        for y in 0..67 {
            for x in 0..65 {
                assert_eq!(
                    at(&doc, x, y),
                    at(
                        &original,
                        if h { 64 - x } else { x },
                        if v { 66 - y } else { y }
                    )
                );
            }
        }
        unchanged_artwork(&original, &doc);
    }
}
#[test]
fn fractional_selection_overlap_and_rectangle_metadata_follow_moved_mask() {
    let (mut original, id) = fixture(6, 1);
    original.selection_mask = Some(
        crate::selection_mask::Mask::from_coverage(6, 1, vec![0, 64, 128, 255, 0, 0]).unwrap(),
    );
    original.selection = original.selection_mask.as_ref().unwrap().bounds();
    let mut doc = original.clone();
    apply(&mut doc, id, &Operation::Move([1, 0]), None).unwrap();
    let incoming = [0, 0, 64, 128, 255, 0];
    let departing = [0, 64, 128, 255, 0, 0];
    for x in 0..6 {
        let retained = 255 - departing[x].max(incoming[x]);
        let reveal = (departing[x] as i32 - incoming[x] as i32).max(0) as u32;
        let source = if x > 0 { at(&original, x - 1, 0) } else { 255 };
        let expected = (at(&original, x, 0) as u32 * retained as u32
            + source as u32 * incoming[x] as u32
            + 255 * reveal
            + 127)
            / 255;
        assert_eq!(at(&doc, x, 0), expected as u8);
        assert_eq!(doc.selection_mask.as_ref().unwrap().at(x, 0), incoming[x]);
    }
    unchanged_artwork(&original, &doc);
    doc = original.clone();
    doc.selection_mask = None;
    doc.selection = Some(Selection {
        left: 1,
        top: 0,
        right: 3,
        bottom: 1,
    });
    apply(&mut doc, id, &Operation::Move([2, 0]), None).unwrap();
    assert_eq!(
        doc.selection,
        Some(Selection {
            left: 3,
            top: 0,
            right: 5,
            bottom: 1
        })
    );
    assert!(doc.selection_mask.is_none());
    apply(&mut doc, id, &Operation::Move([99, 0]), None).unwrap();
    assert!(doc.selection.is_none());
    assert!(doc.selection_mask.as_ref().unwrap().bounds().is_none());
    let frozen = doc.clone();
    assert!(!apply(&mut doc, id, &Operation::Move([-99, 0]), None).unwrap());
    assert!(crate::document::same_snapshot(&frozen, &doc));
}
#[test]
fn selected_interpolation_normalizes_coverage_and_keeps_constant_mask_edges() {
    let (mut doc, _) = fixture(6, 4);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(6, 4, 43).unwrap();
    doc.selection = Some(Selection {
        left: 1,
        top: 1,
        right: 4,
        bottom: 3,
    });
    let mask = &doc.layers[0].mask.as_ref().unwrap().coverage;
    assert_eq!(
        sample_selected(&doc, mask, [1.0, 1.5], Sampling::Bilinear),
        (43, 128)
    );
    assert_eq!(
        sample_selected(&doc, mask, [0.5, 1.5], Sampling::Bilinear),
        (255, 0)
    );
    for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Bicubic] {
        for point in [[1.1, 1.4], [2.3, 2.4], [3.9, 2.8]] {
            let (value, coverage) = sample_selected(&doc, mask, point, sampling);
            assert!(coverage > 0);
            assert_eq!(value, 43);
        }
    }
    let (mut doc, id) = fixture(6, 4);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(6, 4, 43).unwrap();
    doc.selection = Some(Selection {
        left: 1,
        top: 1,
        right: 4,
        bottom: 3,
    });
    let before = doc.clone();
    apply_mapping(
        &mut doc,
        id,
        Mapping::Affine(Matrix([1.0, 0.0, 0.0, 1.0, 0.5, 0.0])),
        Sampling::Bilinear,
        None,
    )
    .unwrap();
    assert_eq!(doc.selection_mask.as_ref().unwrap().at(1, 1), 128);
    assert_eq!(at(&doc, 1, 1), 149); // half departs, half is replaced
    assert_eq!(at(&doc, 2, 1), 43); // shared selection does not pale
    unchanged_artwork(&before, &doc);
}
#[test]
fn perspective_and_mesh_translate_only_mask_with_nearest_coordinate_oracle() {
    let (original, id) = fixture(13, 11);
    for warp in [
        Warp::Corners([[2.0, -1.0], [15.0, -1.0], [15.0, 10.0], [2.0, 10.0]]),
        Warp::Mesh(Mesh::from_fn(4, 4, |c, r| {
            [13.0 * c as f64 / 3.0 + 2.0, 11.0 * r as f64 / 3.0 - 1.0]
        })),
    ] {
        let settings = Settings {
            warp: Some(warp),
            sampling: Sampling::Nearest,
            ..Default::default()
        };
        let mut doc = original.clone();
        apply(&mut doc, id, &Operation::Affine(Box::new(settings)), None).unwrap();
        for y in 0..11 {
            for x in 0..13 {
                assert_eq!(
                    at(&doc, x, y),
                    if x >= 2 && y + 1 < 11 {
                        at(&original, x - 2, y + 1)
                    } else {
                        255
                    }
                );
            }
        }
        unchanged_artwork(&original, &doc);
    }
}
#[test]
fn locked_hidden_replaced_and_cancelled_targets_never_mutate_source() {
    let (original, id) = fixture(65, 67);
    let cancel = AtomicBool::new(true);
    let mut doc = original.clone();
    assert!(apply(&mut doc, id, &Operation::Move([1, 0]), Some(&cancel)).is_err());
    assert!(crate::document::same_snapshot(&original, &doc));
    cancel.store(false, Ordering::Relaxed);
    assert!(apply(&mut doc, LayerId::new(), &Operation::Move([1, 0]), None).is_err());
    assert!(crate::document::same_snapshot(&original, &doc));
    for hidden in [false, true] {
        let mut doc = original.clone();
        if hidden {
            doc.layers[0].visible = false;
        } else {
            doc.layers[0].locked = true;
        }
        let frozen = doc.clone();
        assert!(apply(&mut doc, id, &Operation::Move([1, 0]), None).is_err());
        assert!(crate::document::same_snapshot(&frozen, &doc));
    }
}

#[test]
fn folder_masks_ignore_raster_alpha_lock_and_constants_remain_sparse() {
    let (mut doc, id) = fixture(65, 67);
    doc.layers[0] = crate::document::Layer::new_folder("Masked group".into());
    doc.layers[0].alpha_locked = true;
    doc.layers[0].mask = Some(LayerMask {
        id,
        enabled: false,
        linked: false,
        coverage: Mask::new(65, 67, 0).unwrap(),
    });
    let original = doc.clone();
    assert!(
        !apply(
            &mut doc,
            id,
            &Operation::Flip(Transform::FlipHorizontal),
            None
        )
        .unwrap()
    );
    assert!(crate::document::same_snapshot(&doc, &original));
    apply(&mut doc, id, &Operation::Move([i64::MAX, 0]), None).unwrap();
    assert_eq!(
        doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        0
    );
    assert_eq!(at(&doc, 0, 0), 255);
    unchanged_artwork(&original, &doc);
}

#[test]
fn mask_target_payload_overflow_rejects_private_integer_and_affine_results() {
    let (mut doc, id) = fixture(2048, 2048);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(2048, 2048, 0).unwrap();
    let pixels = doc.layers[0].pixels.clone();
    for _ in 1..crate::document::MAX_MASKED_PAYLOAD / (2048 * 2048 * 4) {
        doc.add_layer();
        doc.layers.last_mut().unwrap().pixels = pixels.clone();
    }
    doc.active = 0;
    doc.validate().unwrap();
    // Only a small selected region changes, but even one coverage tile exceeds
    // this document's unchanged combined payload ceiling.
    doc.selection = Some(Selection {
        left: 0,
        top: 0,
        right: 2,
        bottom: 2,
    });
    let original = doc.clone();
    for op in [
        Operation::Move([1, 0]),
        Operation::Affine(Box::new(Settings {
            translation: [1.0, 0.0],
            sampling: Sampling::Nearest,
            ..Default::default()
        })),
    ] {
        assert!(
            apply(&mut doc, id, &op, None)
                .unwrap_err()
                .contains("payload budget")
        );
        assert!(crate::document::same_snapshot(&doc, &original));
    }
}
fn poll(engine: &mut crate::move_preview::Engine, doc: &mut Document) {
    let start = Instant::now();
    loop {
        if let Some(candidate) = engine.poll(doc).unwrap() {
            engine.publish(doc, candidate).unwrap();
        }
        if engine.finished() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::yield_now();
    }
}
#[test]
fn coalesced_mask_preview_uses_frozen_source_and_queue_has_one_undo() {
    let (original, id) = fixture(65, 67);
    let mut doc = original.clone();
    let mut engine = crate::move_preview::Engine::with_target(&doc, Target::Mask(id));
    let gate = engine.gate([1, 0]);
    engine.request([3, -2]);
    gate.send(()).unwrap();
    poll(&mut engine, &mut doc);
    assert_eq!(at(&doc, 4, 1), at(&original, 1, 3));
    engine.request([0, 0]);
    poll(&mut engine, &mut doc);
    assert!(crate::document::same_snapshot(&doc, &original));
    let mut history = History::new();
    let mut queue = crate::render_queue::Queue::default();
    queue
        .edit(crate::document_edits::Edit::MaskTransform {
            target: doc.layers[0].id,
            mask: id,
            operation: Operation::Move([3, -2]),
        })
        .unwrap();
    let progress = queue.advance(&mut doc, &mut history, 1).unwrap();
    assert!(progress.edits.is_empty());
    assert!(crate::document::same_snapshot(&doc, &original));
    let mut app = crate::Dudel {
        doc,
        history,
        render_queue: queue,
        ..Default::default()
    };
    crate::workflow_tests::finish_jobs(&mut app);
    unchanged_artwork(&original, &app.doc);
    assert_eq!(at(&app.doc, 4, 1), at(&original, 1, 3));
    assert!(app.history.undo(&mut app.doc));
    assert!(crate::document::same_snapshot(&app.doc, &original));
    assert!(!app.history.can_undo());
    assert!(app.history.redo(&mut app.doc));
    assert_eq!(at(&app.doc, 4, 1), at(&original, 1, 3));
}
#[test]
fn mask_affine_application_can_preview_cancel_and_apply_with_alpha_lock() {
    let (mut doc, id) = fixture(13, 11);
    doc.layers[0].alpha_locked = true;
    let original = doc.clone();
    let mut app = crate::Dudel {
        mask_focus: Some((doc.layers[0].id, id)),
        doc,
        ..Default::default()
    };
    app.affine_settings = Settings {
        translation: [2.0, -1.0],
        sampling: Sampling::Nearest,
        ..Default::default()
    };
    app.start_affine_preview();
    assert!(app.affine_preview.is_some(), "{}", app.status);
    let start = Instant::now();
    while crate::document::same_snapshot(&app.doc, &original) {
        app.poll_affine_preview();
        assert!(start.elapsed() < Duration::from_secs(5), "{}", app.status);
        std::thread::yield_now();
    }
    unchanged_artwork(&original, &app.doc);
    app.cancel_affine_preview();
    assert!(crate::document::same_snapshot(&app.doc, &original));
    assert!(!app.history.can_undo());
    app.start_affine_preview();
    app.finish_affine_preview();
    crate::workflow_tests::finish_jobs(&mut app);
    assert_eq!(at(&app.doc, 3, 1), at(&original, 1, 2));
    assert!(app.history.undo(&mut app.doc));
    assert!(crate::document::same_snapshot(&app.doc, &original));
}

#[test]
fn bounded_move_matches_full_canvas_scalar_oracle_for_overlaps_gaps_edges_and_extreme_offsets() {
    let (fixture, id) = fixture(65, 67);
    for constant in [None, Some(0), Some(43), Some(255)] {
        let mut original = fixture.clone();
        if let Some(value) = constant {
            original.layers[0].mask.as_mut().unwrap().coverage = Mask::new(65, 67, value).unwrap();
        }
        for bounds in [
            Selection {
                left: 0,
                top: 0,
                right: 3,
                bottom: 4,
            },
            Selection {
                left: 61,
                top: 62,
                right: 65,
                bottom: 67,
            },
            Selection {
                left: 30,
                top: 31,
                right: 35,
                bottom: 37,
            },
            Selection {
                left: 0,
                top: 0,
                right: 65,
                bottom: 67,
            },
        ] {
            for soft in [false, true] {
                let mut source = original.clone();
                source.selection = Some(bounds);
                if soft {
                    let mut bytes = vec![0; 65 * 67];
                    for y in bounds.top..bounds.bottom {
                        for x in bounds.left..bounds.right {
                            bytes[y * 65 + x] = [0, 64, 128, 255][(x + y) % 4];
                        }
                    }
                    let mask = crate::selection_mask::Mask::from_coverage(65, 67, bytes).unwrap();
                    source.selection = mask.bounds();
                    source.selection_mask = Some(mask);
                }
                for offset in [
                    [0, 0],
                    [1, 1],
                    [-1, -1],
                    [31, -29],
                    [-60, 61],
                    [64, 66],
                    [99, -99],
                    [i64::MIN, i64::MAX],
                    [i64::MAX, i64::MIN],
                ] {
                    let mut operations = vec![Operation::Move(offset)];
                    if offset.iter().all(|v| v.unsigned_abs() <= 99) {
                        for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Bicubic] {
                            operations.push(Operation::Affine(Box::new(Settings {
                                translation: [offset[0] as f64, offset[1] as f64],
                                sampling,
                                ..Default::default()
                            })));
                        }
                    }
                    for operation in operations {
                        let mut doc = source.clone();
                        apply(&mut doc, id, &operation, None).unwrap();
                        let mut extent: Option<Selection> = None;
                        for y in 0..67 {
                            for x in 0..65 {
                                let (sx, sy) = (
                                    x as i128 - i128::from(offset[0]),
                                    y as i128 - i128::from(offset[1]),
                                );
                                let (value, incoming) =
                                    if (0..65).contains(&sx) && (0..67).contains(&sy) {
                                        (
                                            at(&source, sx as usize, sy as usize),
                                            selected(&source, sx as usize, sy as usize),
                                        )
                                    } else {
                                        (255, 0)
                                    };
                                let departing = selected(&source, x, y);
                                let retained = 255 - u32::from(departing.max(incoming));
                                let reveal =
                                    u32::from(departing).saturating_sub(u32::from(incoming));
                                let expected = (u32::from(at(&source, x, y)) * retained
                                    + u32::from(value) * u32::from(incoming)
                                    + 255 * reveal
                                    + 127)
                                    / 255;
                                assert_eq!(
                                    at(&doc, x, y),
                                    expected as u8,
                                    "at {x},{y}, offset {offset:?}, soft={soft}"
                                );
                                assert_eq!(selected(&doc, x, y), incoming);
                                if incoming > 0 {
                                    extent = Some(extent.map_or(
                                        Selection {
                                            left: x,
                                            top: y,
                                            right: x + 1,
                                            bottom: y + 1,
                                        },
                                        |s| Selection {
                                            left: s.left.min(x),
                                            top: s.top.min(y),
                                            right: s.right.max(x + 1),
                                            bottom: s.bottom.max(y + 1),
                                        },
                                    ));
                                }
                            }
                        }
                        assert_eq!(doc.selection, extent);
                        assert_eq!(doc.selection_mask.is_some(), soft || extent.is_none());
                        assert!(doc.layers[0].pixels.shares(&source.layers[0].pixels));
                        if constant == Some(255) {
                            assert!(
                                doc.layers[0]
                                    .mask
                                    .as_ref()
                                    .unwrap()
                                    .coverage
                                    .shares(&source.layers[0].mask.as_ref().unwrap().coverage)
                            );
                        }
                    }
                }
            }
        }
    }
}

fn sparse_fixture() -> (Document, LayerId) {
    let (mut doc, id) = fixture(257, 259);
    let mut mask = Mask::new(257, 259, 255).unwrap();
    {
        let mut writer = mask.writer();
        for (x, y, value) in [(0, 0, 0), (63, 63, 64), (128, 129, 128), (256, 258, 43)] {
            writer.set(x, y, value);
        }
        for y in 128..131 {
            for x in 127..130 {
                writer.set(x, y, 128);
            }
        }
    }
    assert!(
        mask.white_sparse_regions().is_some(),
        "Fixture must exercise the white sparse path"
    );
    doc.layers[0].mask.as_mut().unwrap().coverage = mask;
    (doc, id)
}
#[test]
fn white_sparse_whole_moves_and_flips_match_independent_coordinate_oracle() {
    let (original, id) = sparse_fixture();
    for index in 0..7 {
        let mut doc = original.clone();
        let offsets = [
            [17, -9],
            [-256, 0],
            [i64::MIN, i64::MAX],
            [i64::MAX, i64::MIN],
        ];
        let operation = if index < 4 {
            Operation::Move(offsets[index])
        } else {
            Operation::Flip(
                [
                    Transform::FlipHorizontal,
                    Transform::FlipVertical,
                    Transform::RotateHalfTurn,
                ][index - 4],
            )
        };
        apply(&mut doc, id, &operation, None).unwrap();
        for y in 0..259 {
            for x in 0..257 {
                let (sx, sy) = if index < 4 {
                    (
                        x as i128 - i128::from(offsets[index][0]),
                        y as i128 - i128::from(offsets[index][1]),
                    )
                } else {
                    (
                        (if index == 4 || index == 6 { 256 - x } else { x }) as i128,
                        (if index == 5 || index == 6 { 258 - y } else { y }) as i128,
                    )
                };
                let expected = if (0..257).contains(&sx) && (0..259).contains(&sy) {
                    at(&original, sx as usize, sy as usize)
                } else {
                    255
                };
                assert_eq!(at(&doc, x, y), expected);
            }
        }
        unchanged_artwork(&original, &doc);
    }
}
#[test]
fn white_sparse_affine_halos_clipping_mirrors_and_large_zoom_match_full_canvas_sampling() {
    let (original, id) = sparse_fixture();
    let frame = Selection {
        left: 0,
        top: 0,
        right: 257,
        bottom: 259,
    };
    for default in [255, 43] {
        let mut source = original.clone();
        if default != 255 {
            source.layers[0].mask.as_mut().unwrap().coverage =
                Mask::new(257, 259, default).unwrap();
            source.layers[0]
                .mask
                .as_mut()
                .unwrap()
                .coverage
                .writer()
                .set(128, 129, 128);
            assert!(
                source.layers[0]
                    .mask
                    .as_ref()
                    .unwrap()
                    .coverage
                    .white_sparse_regions()
                    .is_none()
            );
        }
        for matrix in [
            Matrix([1.0, 0.0, 0.0, 1.0, 17.5, -8.5]),
            Matrix([0.8, 0.15, -0.1, 0.9, 13.25, -9.5]),
            Matrix([-0.8, 0.0, 0.1, 0.8, 230.0, 3.5]),
            Matrix([0.0, 1.0, -1.0, 0.0, 257.0, 0.0]),
            Matrix([
                100.0,
                0.0,
                0.0,
                100.0,
                128.5 * (1.0 - 100.0),
                129.5 * (1.0 - 100.0),
            ]),
        ] {
            for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Bicubic] {
                let mapping = Mapping::Affine(matrix);
                let inverse = crate::affine::Inverse::new(&mapping, frame, frame).unwrap();
                let before = &source.layers[0].mask.as_ref().unwrap().coverage;
                // Deliberately sample the full canvas, without any tile/region
                // culling, to detect missing destination pixels and filter halos.
                let reference = Mask::generate(257, 259, 255, None, |x, y| {
                    Ok(crate::affine::sample_layer_mask(
                        before,
                        inverse.source(x, y),
                        sampling,
                    ))
                })
                .unwrap();
                let mut doc = source.clone();
                apply_mapping(&mut doc, id, mapping, sampling, None).unwrap();
                for y in 0..259 {
                    for x in 0..257 {
                        assert_eq!(
                            at(&doc, x, y),
                            reference.at(x, y),
                            "at {x},{y}, {matrix:?}, {sampling:?}, default {default}"
                        );
                    }
                }
                if default == 255 && matrix.0[0] == 100.0 && sampling == Sampling::Nearest {
                    assert_eq!(at(&doc, 0, 0), 128);
                    assert_eq!(
                        doc.layers[0]
                            .mask
                            .as_ref()
                            .unwrap()
                            .coverage
                            .payload_bytes(),
                        0,
                        "Large projected regions must retain constant-mask compaction"
                    );
                }
                unchanged_artwork(&source, &doc);
            }
        }
    }
}

#[test]
fn bulk_moves_preserve_unchanged_tiles_and_default_storage_near_combined_payload_limit() {
    let (mut doc, id) = fixture(257, 259);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::generate(257, 259, 255, None, |x, y| {
        Ok(((x % 64 * 3 + y % 64 * 5) % 251) as u8)
    })
    .unwrap();
    let source = doc.clone();
    let stamp = doc.layers[0]
        .mask
        .as_ref()
        .unwrap()
        .coverage
        .stamp([64, 64, 128, 128]);
    apply(&mut doc, id, &Operation::Move([64, 0]), None).unwrap();
    assert!(stamp.matches(&doc.layers[0].mask.as_ref().unwrap().coverage));
    for y in 0..259 {
        for x in 0..257 {
            assert_eq!(
                at(&doc, x, y),
                if x < 64 { 255 } else { at(&source, x - 64, y) }
            );
        }
    }
    assert_eq!(
        doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        (257 - 64) * 259
    );
    unchanged_artwork(&source, &doc);

    let (mut doc, id) = fixture(2048, 2047);
    doc.layers[0].mask.as_mut().unwrap().coverage = Mask::new(2048, 2047, 43).unwrap();
    let pixels = doc.layers[0].pixels.clone();
    for _ in 1..32 {
        doc.add_layer();
        doc.layers.last_mut().unwrap().pixels = pixels.clone();
    }
    doc.active = 0;
    doc.selection = Some(Selection {
        left: 0,
        top: 0,
        right: 2048,
        bottom: 2047,
    });
    doc.validate().unwrap();
    let stamp = doc.layers[0]
        .mask
        .as_ref()
        .unwrap()
        .coverage
        .stamp([64, 64, 128, 128]);
    apply(&mut doc, id, &Operation::Move([1, 0]), None).unwrap();
    doc.validate().unwrap();
    assert_eq!(
        doc.selection,
        Some(Selection {
            left: 1,
            top: 0,
            right: 2048,
            bottom: 2047
        })
    );
    assert!(doc.selection_mask.is_none());
    assert!(stamp.matches(&doc.layers[0].mask.as_ref().unwrap().coverage));
    assert_eq!(
        doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        64 * 2047
    );
    assert!(doc.layers.iter().all(|l| l.pixels.shares(&pixels)));
    assert_eq!(at(&doc, 0, 2046), 255);
    assert_eq!(at(&doc, 1, 2046), 43);
}

#[test]
fn canceled_bulk_generation_discards_private_tiles_without_mutating_source() {
    let (source, _) = fixture(257, 259);
    let before = source.layers[0].mask.as_ref().unwrap().coverage.clone();
    let cancel = AtomicBool::new(false);
    let result = before.generate_shared(Some(&cancel), |x, y| {
        if x == 63 && y == 0 {
            cancel.store(true, Ordering::Relaxed);
        }
        255 - before.at(x, y)
    });
    assert!(result.is_err());
    assert!(before.shares(&source.layers[0].mask.as_ref().unwrap().coverage));
    assert_eq!(before.at(0, 0), 0);
    cancel.store(false, Ordering::Relaxed);
    let result = before.offset([i64::MIN, i64::MAX], Some(&cancel)).unwrap();
    assert_eq!(result.payload_bytes(), 0);
    assert_eq!(result.at(0, 0), 255);
}

#[test]
fn bilinear_mask_sampling_matches_dense_scalar_oracle_at_tile_canvas_and_rounding_boundaries() {
    // Independent byte storage: no tile addressing or production sampler calls
    // in the reference. Preserve the existing interpolation evaluation order.
    fn reference(bytes: &[u8], w: usize, h: usize, [x, y]: [f64; 2]) -> u8 {
        if !x.is_finite() || !y.is_finite() {
            return 255;
        }
        let (u, v) = (x - 0.5, y - 0.5);
        let (l, t) = (u.floor(), v.floor());
        let (fx, fy) = (u - l, v - t);
        let read = |x: f64, y: f64| {
            if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
                255.0
            } else {
                f64::from(bytes[y as usize * w + x as usize])
            }
        };
        (read(l, t) * (1.0 - fx) * (1.0 - fy)
            + read(l + 1.0, t) * fx * (1.0 - fy)
            + read(l, t + 1.0) * (1.0 - fx) * fy
            + read(l + 1.0, t + 1.0) * fx * fy)
            .round()
            .clamp(0.0, 255.0) as u8
    }
    for [w, h] in [[1, 1], [2, 3], [65, 67], [129, 131]] {
        for default in [0, 43, 255] {
            for sparse in [false, true] {
                let bytes: Vec<u8> = (0..w * h)
                    .map(|i| {
                        let (x, y) = (i % w, i / w);
                        if sparse && (x / 64 + y / 64) % 3 == 0 {
                            default
                        } else {
                            ((x * 37 + y * 19 + x * y * 7) % 256) as u8
                        }
                    })
                    .collect();
                let mask =
                    Mask::generate(w, h, default, None, |x, y| Ok(bytes[y * w + x])).unwrap();
                let check = |point| {
                    assert_eq!(
                        affine::sample_layer_mask(&mask, point, Sampling::Bilinear),
                        reference(&bytes, w, h, point),
                        "{w}x{h}, default {default}, sparse {sparse}, {point:?}"
                    )
                };
                let edges = |size| {
                    [
                        -1.5,
                        -0.5000000001,
                        -0.5,
                        -0.4999999999,
                        0.0,
                        0.5,
                        1.0,
                        63.5,
                        64.0,
                        64.5,
                        127.5,
                        128.0,
                        128.5,
                        size as f64 - 0.5,
                        size as f64,
                        size as f64 + 0.5,
                    ]
                };
                for x in edges(w) {
                    for y in edges(h) {
                        check([x, y]);
                    }
                }
                for y in 0..h {
                    for x in 0..w {
                        for [dx, dy] in
                            [[0.5, 0.5], [1.0, 0.5], [0.5, 1.0], [1.0, 1.0], [0.13, 0.79]]
                        {
                            check([x as f64 + dx, y as f64 + dy]);
                        }
                    }
                }
                for point in [
                    [f64::NAN, 0.5],
                    [0.5, f64::INFINITY],
                    [f64::NEG_INFINITY, 0.5],
                    [f64::MAX, f64::MAX],
                    [-1e100, 1e100],
                ] {
                    check(point);
                }
            }
        }
    }
}
