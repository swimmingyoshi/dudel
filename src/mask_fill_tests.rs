use crate::{
    document::{Document, LayerId},
    document_edits::Edit,
    fill::{Options, Request, Source},
    layer_mask::{LayerMask, Mask},
    layer_mask_edits::Command,
};
use std::sync::atomic::AtomicBool;
fn fixture(w: usize, h: usize, value: u8) -> Document {
    let mut doc = Document::new(w, h);
    doc.version = 2;
    doc.layers[0].pixels = [17, 83, 201, 129].repeat(w * h).into();
    doc.layers[0].mask = Some(LayerMask {
        id: LayerId::new(),
        enabled: true,
        linked: false,
        coverage: Mask::new(w, h, value).unwrap(),
    });
    doc
}
fn request() -> Request {
    Request {
        point: [0, 0],
        color: [0; 3],
        opacity: 1.0,
        tolerance: 0,
        source: Source::Active,
        options: Options::default(),
    }
}
fn edit(
    doc: &Document,
    command: Command,
) -> Result<(Document, crate::document_edits::Outcome), String> {
    Edit::LayerMask {
        target: doc.layers[doc.active].id,
        command,
    }
    .prepare(doc)
}
#[test]
fn clear_reveals_fractional_selection_and_whole_mask_without_touching_artwork() {
    let mut doc = fixture(5, 1, 64);
    doc.layers[0].alpha_locked = true;
    doc.layers[0].mask.as_mut().unwrap().enabled = false;
    doc.selection = Some(crate::selection::Selection {
        left: 1,
        top: 0,
        right: 4,
        bottom: 1,
    });
    doc.selection_mask =
        Some(crate::selection_mask::Mask::from_coverage(5, 1, vec![0, 64, 128, 255, 0]).unwrap());
    let id = doc.layers[0].mask.as_ref().unwrap().id;
    let (cleared, outcome) = edit(
        &doc,
        Command::Clear {
            id,
            whole_layer: false,
        },
    )
    .unwrap();
    assert!(outcome.changed);
    assert_eq!(
        (0..5)
            .map(|x| cleared.layers[0].mask.as_ref().unwrap().coverage.at(x, 0))
            .collect::<Vec<_>>(),
        [64, 112, 160, 255, 64]
    );
    assert!(cleared.layers[0].pixels.shares(&doc.layers[0].pixels));
    assert_eq!(cleared.selection_mask, doc.selection_mask);
    assert!(!cleared.layers[0].mask.as_ref().unwrap().enabled);
    assert!(!cleared.layers[0].mask.as_ref().unwrap().linked);
    let (whole, _) = edit(
        &cleared,
        Command::Clear {
            id,
            whole_layer: true,
        },
    )
    .unwrap();
    assert_eq!(
        whole.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        0
    );
    assert_eq!(
        whole.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        255
    );
    let (no_op, outcome) = edit(
        &whole,
        Command::Clear {
            id,
            whole_layer: true,
        },
    )
    .unwrap();
    assert!(!outcome.changed && crate::document::same_snapshot(&whole, &no_op));
}
#[test]
fn active_bucket_reads_mask_while_explicit_reference_reads_raw_artwork() {
    let mut doc = fixture(33, 3, 0);
    for y in 0..3 {
        for x in 16..33 {
            doc.layers[0]
                .mask
                .as_mut()
                .unwrap()
                .coverage
                .writer()
                .set(x, y, 255);
        }
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(y * 33 + 4)
            .copy_from_slice(&[0, 0, 0, 255]);
    }
    let id = doc.layers[0].mask.as_ref().unwrap().id;
    let req = Request {
        color: [128; 3],
        ..request()
    };
    for (source, boundary) in [(Source::Active, 16), (Source::Layer(doc.layers[0].id), 4)] {
        let (filled, _) = edit(
            &doc,
            Command::Fill {
                id,
                request: Request { source, ..req },
            },
        )
        .unwrap();
        for y in 0..3 {
            for x in 0..33 {
                assert_eq!(
                    filled.layers[0].mask.as_ref().unwrap().coverage.at(x, y),
                    if x < boundary {
                        128
                    } else if x < 16 {
                        0
                    } else {
                        255
                    }
                );
            }
        }
        assert!(filled.layers[0].pixels.shares(&doc.layers[0].pixels));
    }
    let mut white = fixture(33, 3, 255);
    white.selection_mask =
        Some(crate::selection_mask::Mask::from_coverage(33, 3, vec![255; 99]).unwrap());
    white.selection = white.selection_mask.as_ref().unwrap().bounds();
    let (full, _) = edit(
        &white,
        Command::Fill {
            id: white.layers[0].mask.as_ref().unwrap().id,
            request: req,
        },
    )
    .unwrap();
    assert_eq!(
        full.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        0
    );
    let (revealed, _) = edit(
        &full,
        Command::Clear {
            id: full.layers[0].mask.as_ref().unwrap().id,
            whole_layer: false,
        },
    )
    .unwrap();
    assert_eq!(
        revealed.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        0
    );
    assert_eq!(
        revealed.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        255
    );
}
#[test]
fn advanced_fill_matches_independent_opaque_grayscale_raster_with_selection_and_opacity() {
    for source_kind in 0..2 {
        for expand in [-1, 0, 1] {
            let mut mask = fixture(17, 17, 200);
            mask.layers[0].alpha_locked = true;
            mask.layers[0].mask.as_mut().unwrap().enabled = false;
            mask.add_layer();
            for y in 0..17 {
                if y != 8 {
                    mask.layers[1]
                        .pixels
                        .writer()
                        .rgba_mut(y * 17 + 8)
                        .copy_from_slice(&[0, 0, 0, 255]);
                }
            }
            mask.active = 0;
            let mut raster = mask.clone();
            raster.layers[0].mask = None;
            raster.layers[0].pixels = [200, 200, 200, 255].repeat(17 * 17).into();
            let selection = crate::selection_mask::Mask::from_coverage(
                17,
                17,
                (0..289).map(|i| [64, 128, 255][i % 3]).collect(),
            )
            .unwrap();
            mask.selection_mask = Some(selection.clone());
            raster.selection_mask = Some(selection);
            mask.selection = mask.selection_mask.as_ref().unwrap().bounds();
            raster.selection = mask.selection;
            let color = [17, 83, 201];
            let id = mask.layers[0].mask.as_ref().unwrap().id;
            let gray = crate::paint_target::Target::Mask(id)
                .params(color, false, false)
                .0;
            let source = if source_kind == 0 {
                Source::Layer(mask.layers[1].id)
            } else {
                Source::Visible
            };
            // Visible reference is frozen from the same source document, independently
            // of the opaque destination used for the grayscale painting oracle.
            let oracle_source = if source_kind == 1 {
                raster.add_layer();
                raster.layers[2].pixels = mask.composite_transparent().into();
                raster.active = 0;
                Source::Layer(raster.layers[2].id)
            } else {
                source
            };
            let req = Request {
                point: [1, 8],
                color,
                opacity: 0.5,
                source,
                options: Options {
                    close_gap: 1,
                    expand,
                    fill_edges: false,
                },
                ..request()
            };
            let (filled, outcome) = edit(&mask, Command::Fill { id, request: req }).unwrap();
            assert!(outcome.changed);
            assert!(
                raster
                    .fill_request(
                        Request {
                            source: oracle_source,
                            color: gray,
                            ..req
                        },
                        None
                    )
                    .unwrap()
            );
            for y in 0..17 {
                for x in 0..17 {
                    assert_eq!(
                        filled.layers[0].mask.as_ref().unwrap().coverage.at(x, y),
                        raster.layers[0].pixels.rgba(y * 17 + x)[0]
                    );
                }
            }
            assert!(filled.layers[0].pixels.shares(&mask.layers[0].pixels));
            assert_eq!(filled.layers[1].pixels, mask.layers[1].pixels);
        }
    }
}
#[test]
fn locks_hidden_stale_invalid_and_canceled_mask_operations_publish_nothing() {
    let mut doc = fixture(9, 9, 0);
    let id = doc.layers[0].mask.as_ref().unwrap().id;
    for command in [
        Command::Clear {
            id,
            whole_layer: true,
        },
        Command::Fill {
            id,
            request: Request {
                color: [255; 3],
                ..request()
            },
        },
    ] {
        let before = doc.clone();
        assert!(
            Edit::LayerMask {
                target: doc.layers[0].id,
                command
            }
            .prepare_control(&doc, Some(&AtomicBool::new(true)))
            .is_err()
        );
        assert!(crate::document::same_snapshot(&doc, &before));
        doc.layers[0].locked = true;
        assert!(edit(&doc, command).is_err());
        doc.layers[0].locked = false;
        doc.layers[0].visible = false;
        assert!(edit(&doc, command).is_err());
        doc.layers[0].visible = true;
    }
    for req in [
        Request {
            opacity: f32::NAN,
            ..request()
        },
        Request {
            opacity: 2.0,
            ..request()
        },
        Request {
            options: Options {
                close_gap: 9,
                expand: 0,
                fill_edges: false,
            },
            ..request()
        },
        Request {
            source: Source::Layer(LayerId::new()),
            ..request()
        },
    ] {
        assert!(edit(&doc, Command::Fill { id, request: req }).is_err());
    }
    assert!(
        edit(
            &doc,
            Command::Clear {
                id: LayerId::new(),
                whole_layer: true
            }
        )
        .is_err()
    );
    doc.group_active().unwrap();
    let folder = doc.active;
    crate::layer_mask_edits::apply(
        &mut doc,
        folder,
        Command::Create {
            id: LayerId::new(),
            seed: crate::layer_mask_edits::Seed::HideAll,
        },
        None,
    )
    .unwrap();
    let id = doc.layers[folder].mask.as_ref().unwrap().id;
    doc.layers[folder].alpha_locked = true;
    assert!(
        edit(
            &doc,
            Command::Fill {
                id,
                request: Request {
                    color: [255; 3],
                    opacity: 0.5,
                    ..request()
                }
            }
        )
        .unwrap()
        .1
        .changed
    );
}

#[test]
fn queued_mask_fill_save_clear_and_undo_preserve_order_and_saved_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mask-fill.dudel");
    let mut app = crate::Dudel {
        doc: Document::new(8, 8),
        path: Some(path.clone()),
        ..Default::default()
    };
    assert!(app.create_canvas_mask(crate::layer_mask_edits::Seed::HideAll));
    app.canvas_edit_ticket = Some(12345);
    let queued_bytes = app.render_queue.queued_input_bytes();
    app.fill_canvas(0, 0);
    assert_eq!(app.render_queue.queued_input_bytes(), queued_bytes);
    assert!(app.status.contains("bounds change"));
    app.canvas_edit_ticket = None;
    app.brush.opacity = 0.5;
    app.color = [255; 3];
    app.mask_colors[0] = 255;
    app.fill_canvas(0, 0);
    app.save(false);
    app.clear_layer_pixels();
    app.flush_render_queue();
    assert_eq!(
        app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        255
    );
    let ctx = crate::egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.save_job.is_some() {
        assert!(std::time::Instant::now() < deadline);
        let _ = ctx.run(Default::default(), |ctx| app.poll_save(ctx));
        std::thread::yield_now();
    }
    assert!(app.modified);
    let saved = crate::storage::load_project(&path).unwrap();
    assert_eq!(
        saved.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        128
    );
    for value in [128, 0] {
        app.undo();
        assert_eq!(
            app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
            value
        );
    }
    app.undo();
    assert!(app.doc.layers[0].mask.is_none());
    for _ in 0..3 {
        app.redo();
    }
    assert_eq!(
        app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        255
    );
}
#[test]
fn canceled_worker_preserves_redo_and_over_budget_fill_or_clear_is_atomic() {
    let mut doc = fixture(17, 17, 0);
    let mut history = crate::document::History::new();
    history.checkpoint(&doc);
    doc.layers[0].name = "Redo me".into();
    history.settle(&doc);
    assert!(history.undo(&mut doc));
    let before = doc.clone();
    let mut queue = crate::render_queue::Queue::default();
    queue
        .edit(Edit::LayerMask {
            target: doc.layers[0].id,
            command: Command::Fill {
                id: doc.layers[0].mask.as_ref().unwrap().id,
                request: Request {
                    color: [255; 3],
                    ..request()
                },
            },
        })
        .unwrap();
    queue.advance(&mut doc, &mut history, 1).unwrap();
    assert!(queue.cancel_background_edit());
    while queue.busy() {
        if queue.waiting_edit() {
            queue.wait_edit();
        }
        queue.advance(&mut doc, &mut history, 16).unwrap();
    }
    assert!(crate::document::same_snapshot(&doc, &before));
    assert!(history.can_redo() && !history.can_undo());

    let mut doc = fixture(2048, 2048, 0);
    let source = doc.layers[0].clone();
    for _ in 1..32 {
        let mut copy = source.clone();
        copy.id = LayerId::new();
        copy.mask = None;
        doc.layers.push(copy);
    }
    doc.selection = Some(crate::selection::Selection {
        left: 0,
        top: 0,
        right: 2,
        bottom: 2,
    });
    doc.validate().unwrap(); // Exactly 512 MiB of logical raster payload, shared physically.
    let before = doc.clone();
    let id = doc.layers[0].mask.as_ref().unwrap().id;
    for command in [
        Command::Clear {
            id,
            whole_layer: false,
        },
        Command::Fill {
            id,
            request: Request {
                color: [255; 3],
                opacity: 0.5,
                ..request()
            },
        },
    ] {
        let error = match edit(&doc, command) {
            Ok(_) => panic!("Over-budget mask edit published"),
            Err(e) => e,
        };
        assert!(
            error.contains("payload") || error.contains("512"),
            "{error}"
        );
        assert!(crate::document::same_snapshot(&doc, &before));
    }
}
#[test]
fn plain_region_visits_only_the_component_and_boundary() {
    let calls = std::cell::Cell::new(0usize);
    let region = crate::fill::region_mask(
        [128, 128],
        64 * 128 + 64,
        Options::default(),
        |at| {
            calls.set(calls.get() + 1);
            at == 64 * 128 + 64
        },
        None,
    )
    .unwrap();
    assert_eq!(region.iter().map(|w| w.count_ones()).sum::<u32>(), 1);
    assert_eq!(calls.get(), 5);
}
