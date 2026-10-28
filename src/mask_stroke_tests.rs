use crate::{
    brush::Brush,
    document::{Document, LayerId},
    layer_mask::{LayerMask, Mask},
    paint_target::Target,
    stroke::Sample,
    stroke_buffer::{Settings, StrokeBuffer},
};
use std::sync::Arc;
fn source(w: usize, h: usize, value: u8) -> Document {
    let mut doc = Document::new(w, h);
    doc.version = 2;
    doc.layers[0].pixels = [17, 83, 201, 129].repeat(w * h).into();
    doc.layers[0].mask = Some(LayerMask {
        id: LayerId::new(),
        enabled: true,
        linked: true,
        coverage: Mask::new(w, h, value).unwrap(),
    });
    doc
}
fn stroke_settings(doc: &Document) -> Settings {
    Settings {
        target: Target::Mask(doc.layers[doc.active].mask.as_ref().unwrap().id),
        brush: Brush {
            size: 8.0,
            hardness: 1.0,
            pressure_size: false,
            opacity: 0.5,
            flow: 1.0,
            ..Default::default()
        },
        color: [0; 3],
        erase: false,
        layer: doc.active,
        symmetry: Default::default(),
    }
}
fn sample(x: f32, y: f32, distance: f64) -> Sample {
    Sample {
        x,
        y,
        pressure: 1.0,
        distance,
        ..Default::default()
    }
}
fn coverage(doc: &Document, x: usize, y: usize) -> u8 {
    doc.layers[doc.active]
        .mask
        .as_ref()
        .unwrap()
        .coverage
        .at(x, y)
}
#[test]
fn mask_black_white_eraser_and_stroke_opacity_do_not_change_raster_or_transparency() {
    for (value, color, erase) in [
        (255, [0; 3], false),
        (0, [255; 3], false),
        (0, [0; 3], true),
    ] {
        let mut doc = source(17, 17, value);
        doc.layers[0].alpha_locked = true;
        let original = doc.clone();
        let mut settings = stroke_settings(&doc);
        settings.color = color;
        settings.erase = erase;
        let mut buffer = StrokeBuffer::default();
        for i in 0..40 {
            buffer.paint(&mut doc, sample(8.5, 8.5, i as f64), &settings);
        }
        assert_eq!(coverage(&doc, 8, 8), 128);
        assert!(doc.layers[0].pixels.shares(&original.layers[0].pixels));
        buffer.restore(&mut doc);
        assert!(crate::document::same_snapshot(&doc, &original));
    }
}
#[test]
fn mask_pressure_grain_symmetry_taper_and_sliced_replay_match_opaque_grayscale_painting() {
    let mut mask = source(65, 67, 255);
    let mut reference = Document::new(65, 67);
    reference.layers[0].pixels = [255; 4].repeat(65 * 67).into();
    reference.layers[0].alpha_locked = true;
    let selected = crate::selection_mask::Mask::from_coverage(
        65,
        67,
        (0..65 * 67).map(|i| [0, 64, 128, 255][i % 4]).collect(),
    )
    .unwrap();
    mask.selection = selected.bounds();
    mask.selection_mask = Some(selected.clone());
    reference.selection = selected.bounds();
    reference.selection_mask = Some(selected);
    let mut settings = stroke_settings(&mask);
    settings.color = [37; 3];
    settings.brush.opacity = 0.4;
    settings.brush.flow = 0.3;
    settings.brush.pressure_size = true;
    settings.brush.taper_out = 18.0;
    settings.brush.grain_amount = 0.6;
    settings.symmetry.mode = crate::symmetry::Mode::LeftRight;
    let mut raster_settings = settings.clone();
    raster_settings.target = Target::Raster;
    let mut sliced = mask.clone();
    let mut mask_buffer = StrokeBuffer::default();
    let mut sliced_buffer = StrokeBuffer::default();
    let mut raster_buffer = StrokeBuffer::default();
    for i in 0..32 {
        let sample = Sample {
            pressure: 0.2 + (i % 5) as f32 * 0.2,
            ..sample(50.0 + i as f32 * 0.5, 61.0, i as f64)
        };
        mask_buffer.paint(&mut mask, sample, &settings);
        raster_buffer.paint(&mut reference, sample, &raster_settings);
        let mut cursor = sliced_buffer
            .begin_dab(&sliced, sample, Arc::new(settings.clone()))
            .unwrap();
        while !cursor
            .step(&mut sliced_buffer, &mut sliced, 3)
            .unwrap()
            .complete
        {}
    }
    mask_buffer.finish_tail(&mut mask, &settings);
    raster_buffer.finish_tail(&mut reference, &raster_settings);
    let mut tail = sliced_buffer.begin_tail(&sliced, &settings).unwrap();
    while !tail
        .step(&mut sliced_buffer, &mut sliced, 2)
        .unwrap()
        .complete
    {}
    for y in 0..67 {
        for x in 0..65 {
            assert_eq!(
                coverage(&mask, x, y),
                reference.layers[0].pixels.rgba(y * 65 + x)[0]
            );
            assert_eq!(coverage(&sliced, x, y), coverage(&mask, x, y));
        }
    }
    assert!(mask_buffer.has_visible_changes(&mask));
}
#[test]
fn partial_taper_and_resumable_mask_rollback_restore_the_original_shared_coverage() {
    let mut doc = source(65, 67, 255);
    let original = doc.clone();
    let mut settings = stroke_settings(&doc);
    settings.brush.taper_out = 20.0;
    let mut buffer = StrokeBuffer::default();
    for i in 0..24 {
        buffer.paint(
            &mut doc,
            sample(55.0 + i as f32 * 0.4, 63.0, i as f64),
            &settings,
        );
    }
    let mut tail = buffer.begin_tail(&doc, &settings).unwrap();
    for _ in 0..7 {
        tail.step(&mut buffer, &mut doc, 1).unwrap();
    }
    drop(tail);
    let mut restore = buffer.begin_restore(&doc);
    let mut units = 0;
    loop {
        let step = restore.step(&mut doc, 1).unwrap();
        units += step.work_units;
        if step.complete {
            break;
        }
    }
    assert!(units > 1);
    assert!(crate::document::same_snapshot(&doc, &original));
}
#[test]
fn dab_and_rollback_reject_replaced_layers_even_if_the_mask_slot_was_cloned() {
    let mut doc = source(17, 17, 255);
    let settings = stroke_settings(&doc);
    let mut buffer = StrokeBuffer::default();
    let mut cursor = buffer
        .begin_dab(&doc, sample(8.5, 8.5, 0.0), Arc::new(settings.clone()))
        .unwrap();
    doc.layers[0].id = LayerId::new();
    let replacement = doc.clone();
    assert!(cursor.step(&mut buffer, &mut doc, 1).is_err());
    assert!(crate::document::same_snapshot(&doc, &replacement));
    let mut doc = source(17, 17, 255);
    let settings = stroke_settings(&doc);
    let mut buffer = StrokeBuffer::default();
    buffer.paint(&mut doc, sample(8.5, 8.5, 0.0), &settings);
    let mut restore = buffer.begin_restore(&doc);
    doc.layers[0].id = LayerId::new();
    let replacement = doc.clone();
    assert!(restore.step(&mut doc, 1).is_err());
    assert!(crate::document::same_snapshot(&doc, &replacement));
}
#[test]
fn queued_creation_stroke_save_export_and_two_undo_steps_preserve_mask_and_raster() {
    let mut app = crate::Dudel {
        doc: Document::new(17, 17),
        ..Default::default()
    };
    app.doc.layers[0].pixels = [17, 83, 201, 255].repeat(17 * 17).into();
    let original = app.doc.clone();
    let slot = LayerId::new();
    let layer = app.doc.layers[0].id;
    app.render_queue
        .edit(crate::document_edits::Edit::LayerMask {
            target: layer,
            command: crate::layer_mask_edits::Command::Create {
                id: slot,
                seed: crate::layer_mask_edits::Seed::RevealAll,
            },
        })
        .unwrap();
    let mut settings = stroke_settings(&source(17, 17, 255));
    settings.target = Target::Mask(slot);
    let id = app
        .render_queue
        .begin_layer(Arc::new(settings), layer)
        .unwrap();
    for i in 0..40 {
        app.render_queue
            .dab(id, sample(8.5, 8.5, i as f64))
            .unwrap();
    }
    app.render_queue.end(id).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("stroke.dudel");
    let png = dir.path().join("stroke.png");
    app.path = Some(project.clone());
    app.save(false);
    app.export_to(png.clone(), true);
    crate::workflow_tests::finish_jobs(&mut app);
    assert_eq!(coverage(&app.doc, 8, 8), 128);
    assert!(app.doc.layers[0].pixels.shares(&original.layers[0].pixels));
    assert!(!app.modified);
    let reopened = crate::storage::load_project(&project).unwrap();
    assert_eq!(coverage(&reopened, 8, 8), 128);
    assert_eq!(
        image::open(png).unwrap().into_rgba8().as_raw(),
        &app.doc.composite_transparent()
    );
    app.undo();
    crate::workflow_tests::finish_jobs(&mut app);
    assert_eq!(coverage(&app.doc, 8, 8), 255);
    app.undo();
    crate::workflow_tests::finish_jobs(&mut app);
    assert!(app.doc.layers[0].mask.is_none());
    assert_eq!(app.doc.version, 1);
}
#[test]
fn queued_mask_strokes_reject_replaced_locked_or_hidden_targets() {
    for case in 0..3 {
        let mut doc = source(17, 17, 255);
        let settings = stroke_settings(&doc);
        match case {
            0 => doc.layers[0].mask.as_mut().unwrap().id = LayerId::new(),
            1 => doc.layers[0].locked = true,
            _ => doc.layers[0].visible = false,
        };
        let frozen = doc.clone();
        let mut queue = crate::render_queue::Queue::default();
        let id = queue
            .begin_layer(Arc::new(settings), doc.layers[0].id)
            .unwrap();
        queue.dab(id, sample(8.5, 8.5, 0.0)).unwrap();
        queue.end(id).unwrap();
        let mut history = crate::document::History::new();
        let mut canceled = false;
        while queue.busy() {
            let progress = queue.advance(&mut doc, &mut history, 64).unwrap();
            canceled |= progress.completed.iter().any(|c| c.cancelled);
        }
        assert!(canceled);
        assert!(!history.can_undo());
        assert!(crate::document::same_snapshot(&doc, &frozen));
    }
}
#[test]
fn disabled_folder_masks_can_be_painted_without_editing_folder_contents() {
    let mut doc = Document::new(1, 1);
    doc.layers[0].pixels = vec![17, 83, 201, 255].into();
    doc.group_active().unwrap();
    doc.version = 2;
    let slot = LayerId::new();
    doc.layers[doc.active].mask = Some(LayerMask {
        id: slot,
        enabled: false,
        linked: true,
        coverage: Mask::new(1, 1, 255).unwrap(),
    });
    doc.layers[doc.active].alpha_locked = true;
    let original = doc.clone();
    let mut settings = stroke_settings(&doc);
    settings.brush.size = 8.0;
    settings.brush.opacity = 1.0;
    let mut queue = crate::render_queue::Queue::default();
    let id = queue
        .begin_layer(Arc::new(settings), doc.layers[doc.active].id)
        .unwrap();
    queue.dab(id, sample(0.5, 0.5, 0.0)).unwrap();
    queue.end(id).unwrap();
    let mut history = crate::document::History::new();
    while queue.busy() {
        if queue.waiting_edit() {
            queue.wait_edit();
        }
        queue.advance(&mut doc, &mut history, 512).unwrap();
    }
    assert_eq!(coverage(&doc, 0, 0), 0);
    assert_eq!(
        doc.composite_transparent(),
        original.composite_transparent()
    );
    assert!(doc.layers[0].pixels.shares(&original.layers[0].pixels));
    doc.layers[doc.active].mask.as_mut().unwrap().enabled = true;
    assert_eq!(doc.composite_transparent(), [0; 4]);
}
#[test]
fn over_limit_mask_or_raster_contacts_roll_back_before_a_queued_snapshot() {
    for raster in [false, true] {
        let mut doc = source(2048, 2048, 255);
        let pixels: crate::pixels::Pixels = [17, 83, 201, 255].repeat(2048 * 2048).into();
        for i in 0..32 {
            if i != 0 {
                doc.add_layer();
            }
            doc.layers[i].pixels = pixels.clone();
        }
        doc.active = 0;
        if raster {
            doc.layers[0].pixels =
                crate::pixels::Pixels::read_rgba(&mut pixels.as_slice(), 2048, 2048).unwrap();
            doc.layers[0].pixels.clear_region(2048, [0, 0, 256, 256]);
            let mut writer = doc.layers[0].mask.as_mut().unwrap().coverage.writer();
            for tile in 0..64 {
                writer.set(tile % 32 * 64, tile / 32 * 64, 0);
            }
        }
        doc.validate().unwrap();
        let original = doc.clone();
        let mut settings = stroke_settings(&doc);
        settings.brush.size = 4.0;
        settings.brush.opacity = 1.0;
        settings.color = [0; 3];
        if raster {
            settings.target = Target::Raster;
        }
        let mut queue = crate::render_queue::Queue::default();
        let id = queue
            .begin_layer(Arc::new(settings), doc.layers[0].id)
            .unwrap();
        queue.dab(id, sample(128.5, 128.5, 0.0)).unwrap();
        queue.end(id).unwrap();
        queue.snapshot().unwrap();
        let mut history = crate::document::History::new();
        let mut snapshot = None;
        let mut error = None;
        while queue.busy() {
            let progress = queue.advance(&mut doc, &mut history, 64).unwrap();
            if let Some((_, captured)) = progress.snapshot {
                snapshot = Some(captured);
            }
            if progress.render_error.is_some() {
                error = progress.render_error;
            }
        }
        assert!(error.is_some_and(|e| e.contains("payload budget")));
        assert!(!history.can_undo());
        let captured = snapshot.unwrap();
        captured.validate().unwrap();
        assert!(
            captured.layers[0].pixels == original.layers[0].pixels,
            "Quota rollback changed raster bytes"
        );
        assert!(crate::layer_mask::same_snapshot(
            &captured.layers[0].mask,
            &original.layers[0].mask
        ));
    }
}
