use super::*;
use crate::{
    document::LayerId,
    layer_mask::{LayerMask, Mask},
    paint_target::Target,
};
fn source(w: usize, h: usize, value: u8) -> Document {
    let mut doc = Document::new(w, h);
    doc.version = 2;
    doc.layers[0].pixels = [17, 83, 201, 129].repeat(w * h).into();
    doc.layers[0].mask = Some(LayerMask {
        id: LayerId::new(),
        enabled: false,
        linked: false,
        coverage: Mask::new(w, h, value).unwrap(),
    });
    doc
}
fn ids(doc: &Document) -> (LayerId, LayerId) {
    (
        doc.layers[doc.active].id,
        doc.layers[doc.active].mask.as_ref().unwrap().id,
    )
}
fn raster(w: usize, h: usize, value: u8, alpha: u8) -> Raster {
    Raster {
        width: w,
        height: h,
        rgba: [value, value, value, alpha].repeat(w * h),
    }
}
struct Memory {
    image: Option<Raster>,
    fail: bool,
    writes: usize,
}
impl ClipboardIo for Memory {
    fn read(&mut self) -> Result<Raster, String> {
        let r = self.image.as_ref().ok_or("No image")?;
        Ok(Raster {
            width: r.width,
            height: r.height,
            rgba: r.rgba.clone(),
        })
    }
    fn write(&mut self, r: &Raster) -> Result<(), String> {
        self.writes += 1;
        if self.fail {
            return Err("Write failed".into());
        }
        self.image = Some(Raster {
            width: r.width,
            height: r.height,
            rgba: r.rgba.clone(),
        });
        Ok(())
    }
}
fn memory() -> Memory {
    Memory {
        image: None,
        fail: false,
        writes: 0,
    }
}
fn app(doc: Document) -> Dudel {
    let (layer, mask) = ids(&doc);
    Dudel {
        doc,
        mask_focus: Some((layer, mask)),
        ..Default::default()
    }
}
fn reach(app: &mut Dudel) -> (u64, Action) {
    loop {
        if app.render_queue.waiting_edit() {
            app.render_queue.wait_edit();
        }
        let p = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        if let Some(ticket) = p.barrier {
            return (ticket, app.pending_clipboard.remove(&ticket).unwrap());
        }
    }
}
#[test]
fn raw_mask_copy_and_cut_cover_selection_without_baking_layer_style_or_alpha_lock() {
    let mut doc = source(5, 1, 0);
    doc.layers[0].mask.as_mut().unwrap().coverage =
        Mask::from_coverage(5, 1, &[0, 64, 128, 255, 80], None).unwrap();
    let selected =
        crate::selection_mask::Mask::from_coverage(5, 1, vec![255, 128, 0, 64, 255]).unwrap();
    doc.selection = selected.bounds();
    doc.selection_mask = Some(selected);
    doc.layers[0].opacity = 0.2;
    doc.layers[0].blend = crate::blend::BlendMode::Multiply;
    doc.layers[0].alpha_locked = true;
    let (target, mask) = ids(&doc);
    let before = doc.clone();
    let mut io = memory();
    doc.layers[0].locked = true;
    assert!(
        execute_action(
            &doc,
            Action::CopyMask {
                target,
                mask,
                cut: false
            },
            &mut io
        )
        .unwrap()
        .0
        .is_none()
    );
    assert_eq!(
        io.image.as_ref().unwrap().rgba,
        [
            0, 0, 0, 255, 64, 64, 64, 128, 0, 0, 0, 0, 255, 255, 255, 64, 80, 80, 80, 255
        ]
    );
    assert!(
        execute_action(
            &doc,
            Action::CopyMask {
                target,
                mask,
                cut: true
            },
            &mut io
        )
        .is_err()
    );
    doc.layers[0].locked = false;
    io.fail = true;
    assert!(
        execute_action(
            &doc,
            Action::CopyMask {
                target,
                mask,
                cut: true
            },
            &mut io
        )
        .is_err()
    );
    assert!(crate::document::same_snapshot(&doc, &before));
    io.fail = false;
    let updated = execute_action(
        &doc,
        Action::CopyMask {
            target,
            mask,
            cut: true,
        },
        &mut io,
    )
    .unwrap()
    .0
    .unwrap();
    assert_eq!(
        (0..5)
            .map(|x| updated.layers[0].mask.as_ref().unwrap().coverage.at(x, 0))
            .collect::<Vec<_>>(),
        [255, 160, 128, 255, 255]
    );
    assert!(updated.layers[0].pixels.shares(&before.layers[0].pixels));
    assert_eq!(updated.selection_mask, before.selection_mask);
    assert!(updated.layers[0].alpha_locked);
    assert!(
        !updated.layers[0].mask.as_ref().unwrap().enabled
            && !updated.layers[0].mask.as_ref().unwrap().linked
    );
}
#[test]
fn mask_paste_centers_luminance_and_multiplies_clipboard_and_selection_coverage() {
    let mut doc = source(7, 3, 100);
    doc.layers[0].alpha_locked = true;
    let mut bytes = vec![0; 21];
    bytes[9] = 255;
    bytes[10] = 128;
    bytes[11] = 64;
    let selection = crate::selection_mask::Mask::from_coverage(7, 3, bytes).unwrap();
    doc.selection = selection.bounds();
    doc.selection_mask = Some(selection);
    let (target, mask) = ids(&doc);
    let input = Raster {
        width: 3,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0],
    };
    let updated = crate::mask_clipboard::paste(&doc, &input, target, mask)
        .unwrap()
        .unwrap();
    for y in 0..3 {
        for x in 0..7 {
            assert_eq!(
                updated.layers[0].mask.as_ref().unwrap().coverage.at(x, y),
                match (x, y) {
                    (2, 1) => 77,
                    (3, 1) => 112,
                    _ => 100,
                }
            );
        }
    }
    assert_eq!(updated.layers.len(), 1);
    assert!(updated.layers[0].pixels.shares(&doc.layers[0].pixels));
    assert_eq!(updated.selection_mask, doc.selection_mask);
    assert_eq!(updated.selection, doc.selection);
    assert!(
        crate::mask_clipboard::paste(&doc, &raster(3, 1, 255, 0), target, mask)
            .unwrap()
            .is_none()
    );
    let mut full = source(7, 3, 100);
    let (target, mask) = ids(&full);
    let updated = crate::mask_clipboard::paste(&full, &raster(7, 3, 200, 128), target, mask)
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .payload_bytes(),
        0
    );
    assert_eq!(
        updated.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        150
    );
    full.group_active().unwrap();
    let folder = full.active;
    full.layers[folder].mask = Some(LayerMask {
        id: LayerId::new(),
        enabled: false,
        linked: true,
        coverage: Mask::new(7, 3, 0).unwrap(),
    });
    full.layers[folder].alpha_locked = true;
    let (target, mask) = ids(&full);
    let updated = crate::mask_clipboard::paste(&full, &raster(7, 3, 255, 255), target, mask)
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.layers[folder]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(0, 0),
        255
    );
    assert!(updated.layers[0].pixels.shares(&full.layers[0].pixels));
    let mut full = source(2, 2, 0);
    let layer = full.layers[0].clone();
    for _ in 1..32 {
        let mut copy = layer.clone();
        copy.id = LayerId::new();
        copy.mask = None;
        full.layers.push(copy);
    }
    full.validate().unwrap();
    let (target, mask) = ids(&full);
    let updated = crate::mask_clipboard::paste(&full, &raster(2, 2, 200, 255), target, mask)
        .unwrap()
        .unwrap();
    assert_eq!(updated.layers.len(), 32);
    assert_eq!(
        updated.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        200
    );
}
#[test]
fn mask_cut_worker_writes_before_publication_and_stale_source_never_reaches_sink() {
    for phase in 0..3 {
        let mut app = app(source(8, 8, 0));
        let before = app.doc.clone();
        app.history.checkpoint(&app.doc);
        app.doc.layers[0].name = "Redo".into();
        app.history.settle(&app.doc);
        assert!(app.history.undo(&mut app.doc));
        app.copy_canvas(true);
        let (ticket, action) = reach(&mut app);
        let Action::CopyMask { target, mask, cut } = action else {
            panic!("Wrong clipboard target")
        };
        app.start_copy_target_job(ticket, target, Target::Mask(mask), cut);
        let mut io = memory();
        io.fail = phase == 0;
        if phase == 2 {
            app.doc.layers[0].name = "Newer source".into();
        }
        app.finish_copy_with_io(true, &mut io);
        app.finish_clipboard_settlement();
        if phase == 1 {
            assert_eq!(
                app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
                255
            );
            assert!(app.history.can_undo());
            app.undo();
            assert!(crate::document::same_snapshot(&app.doc, &before));
        } else {
            assert_eq!(
                app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
                0
            );
            assert!(app.history.can_redo() && !app.history.can_undo());
        }
        assert_eq!(io.writes, usize::from(phase != 2));
        assert!(!app.render_queue.busy());
    }
}
#[test]
fn pending_mask_paste_blocks_following_captured_stroke_and_snapshot() {
    let mut app = Dudel {
        doc: Document::new(8, 8),
        ..Default::default()
    };
    assert!(app.create_canvas_mask(crate::layer_mask_edits::Seed::HideAll));
    app.paste_canvas_with_files(|_| Vec::new());
    let settings = crate::stroke_buffer::Settings {
        target: app.input_stroke_target(),
        layer: 0,
        brush: crate::brush::Brush {
            size: 1.0,
            hardness: 1.0,
            pressure_size: false,
            ..Default::default()
        },
        color: [255; 3],
        secondary: [255; 3],
        erase: false,
        symmetry: Default::default(),
    };
    assert!(app.begin_canvas_stroke(settings.clone()));
    let point = crate::stroke::Sample {
        x: 1.5,
        y: 1.5,
        pressure: 1.0,
        ..Default::default()
    };
    app.deliver_canvas_dabs(vec![point]);
    app.finish_painted_stroke();
    let snapshot = app.render_queue.snapshot().unwrap();
    let (ticket, action) = reach(&mut app);
    let Action::PasteMask { target, mask } = action else {
        panic!("Wrong paste target")
    };
    app.start_mask_paste_job(ticket, raster(8, 8, 64, 255), target, mask);
    app.focus_current_mask(false);
    assert!(app.render_queue.busy());
    assert_eq!(
        app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        0
    );
    app.finish_paste_job(true);
    app.finish_clipboard_settlement();
    assert_eq!(app.doc.layers.len(), 1);
    assert_eq!(
        app.doc.layers[0].mask.as_ref().unwrap().coverage.at(0, 0),
        64
    );
    let mut expected = app.doc.clone();
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    buffer.paint(&mut expected, point, &settings);
    buffer.finish_tail(&mut expected, &settings);
    let expected_value = expected.layers[0].mask.as_ref().unwrap().coverage.at(1, 1);
    assert!(expected_value > 64);
    let mut frozen = None;
    while app.render_queue.busy() {
        let p = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 64)
            .unwrap();
        if let Some((id, doc)) = p.snapshot {
            assert_eq!(id, snapshot);
            frozen = Some(doc);
        }
    }
    let frozen = frozen.unwrap();
    assert_eq!(
        frozen.layers[0].mask.as_ref().unwrap().coverage.at(1, 1),
        expected_value
    );
    assert_eq!(
        frozen.layers[0].mask.as_ref().unwrap().coverage.at(7, 7),
        64
    );
    assert!(frozen.layers[0].pixels.iter().all(|b| *b == 0));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pasted-mask.dudel");
    crate::storage::save_project(&path, &frozen).unwrap();
    let reopened = crate::storage::load_project(&path).unwrap();
    assert_eq!(
        reopened.layers[0].mask.as_ref().unwrap().coverage.at(1, 1),
        expected_value
    );
    assert_eq!(
        reopened.layers[0].mask.as_ref().unwrap().coverage.at(7, 7),
        64
    );
    assert!(app.history.undo(&mut app.doc));
    assert_eq!(
        app.doc.layers[0].mask.as_ref().unwrap().coverage.at(1, 1),
        64
    );
    assert!(app.history.undo(&mut app.doc));
    assert_eq!(
        app.doc.layers[0].mask.as_ref().unwrap().coverage.at(1, 1),
        0
    );
    assert!(app.history.undo(&mut app.doc));
    assert!(app.doc.layers[0].mask.is_none());
}
#[test]
fn empty_stale_locked_invalid_and_over_budget_clipboard_work_preserves_source() {
    let mut doc = source(9, 9, 0);
    let (target, mask) = ids(&doc);
    let empty = crate::selection_mask::Mask::from_coverage(9, 9, vec![0; 81]).unwrap();
    doc.selection = empty.bounds();
    doc.selection_mask = Some(empty);
    let mut io = memory();
    assert!(
        execute_action(
            &doc,
            Action::CopyMask {
                target,
                mask,
                cut: false
            },
            &mut io
        )
        .is_err()
    );
    assert_eq!(io.writes, 0);
    assert!(
        crate::mask_clipboard::paste(&doc, &raster(9, 9, 255, 255), target, mask)
            .unwrap()
            .is_none()
    );
    doc.selection_mask = None;
    assert!(crate::mask_clipboard::paste(&doc, &raster(10, 9, 255, 255), target, mask).is_err());
    assert!(
        crate::mask_clipboard::paste(&doc, &raster(9, 9, 255, 255), target, LayerId::new())
            .is_err()
    );
    doc.layers[0].locked = true;
    assert!(crate::mask_clipboard::paste(&doc, &raster(9, 9, 255, 255), target, mask).is_err());
    doc.layers[0].locked = false;
    doc.layers[0].visible = false;
    assert!(crate::mask_clipboard::paste(&doc, &raster(9, 9, 255, 255), target, mask).is_err());
    let mut doc = source(2048, 2048, 0);
    let layer = doc.layers[0].clone();
    for _ in 1..crate::document::MAX_MASKED_PAYLOAD / (2048 * 2048 * 4) {
        let mut copy = layer.clone();
        copy.id = LayerId::new();
        copy.mask = None;
        doc.layers.push(copy);
    }
    doc.selection = Some(Selection {
        left: 0,
        top: 0,
        right: 2,
        bottom: 2,
    });
    doc.validate().unwrap();
    let before = doc.clone();
    let (target, mask) = ids(&doc);
    assert!(crate::mask_clipboard::paste(&doc, &raster(2, 2, 255, 255), target, mask).is_err());
    assert!(
        execute_action(
            &doc,
            Action::CopyMask {
                target,
                mask,
                cut: true
            },
            &mut io
        )
        .is_err()
    );
    assert_eq!(io.writes, 0);
    assert!(crate::document::same_snapshot(&doc, &before));
}
#[test]
fn mask_paste_no_op_and_stale_or_invalid_workers_preserve_history_and_release_barrier() {
    for phase in 0..3 {
        let mut app = app(source(8, 8, 255));
        app.history.checkpoint(&app.doc);
        app.doc.layers[0].name = "Redo".into();
        app.history.settle(&app.doc);
        assert!(app.history.undo(&mut app.doc));
        app.paste_canvas_with_files(|_| Vec::new());
        let (ticket, action) = reach(&mut app);
        let Action::PasteMask { target, mask } = action else {
            panic!("Wrong paste target")
        };
        let image = if phase == 2 {
            Raster {
                width: 0,
                height: 0,
                rgba: vec![],
            }
        } else {
            raster(8, 8, 0, if phase == 0 { 0 } else { 255 })
        };
        app.start_mask_paste_job(ticket, image, target, mask);
        if phase == 1 {
            app.doc.layers[0].mask.as_mut().unwrap().id = LayerId::new();
        }
        let before = app.doc.clone();
        app.finish_paste_job(true);
        app.finish_clipboard_settlement();
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert!(app.history.can_redo() && !app.history.can_undo() && !app.modified);
        assert!(!app.render_queue.busy());
        if phase == 0 {
            assert_eq!(app.status, "Paste made no changes");
        } else {
            assert!(app.status.starts_with("Clipboard:"));
        }
    }
}
