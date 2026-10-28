//! Atomic uniform shape painting through the existing selection and alpha locks.
use crate::{document::Document, document::LayerId, fill::check_cancel, selection_mask::Mask};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub target: LayerId,
    #[serde(
        default,
        skip_serializing_if = "crate::paint_target::Target::is_raster"
    )]
    pub paint_target: crate::paint_target::Target,
    pub color: [u8; 3],
    pub opacity: f32,
    pub erase: bool,
}
/// Outline the Lasso tool draws before filling or erasing it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    #[default]
    Freehand,
    Rectangle,
    Ellipse,
}
impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Freehand, Shape::Rectangle, Shape::Ellipse];
    pub fn name(self) -> &'static str {
        match self {
            Self::Freehand => "Freehand",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
        }
    }
}
/// Lasso tool choices that persist between contacts.
#[derive(Clone, Copy, Default, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Options {
    #[serde(default)]
    pub erase: bool,
    #[serde(default)]
    pub shape: Shape,
    /// Rectangles become squares and ellipses circles.
    #[serde(default)]
    pub constrain: bool,
    /// Rectangles and ellipses grow from the press point instead of a corner.
    #[serde(default)]
    pub from_center: bool,
}
/// Polygon for a dragged rectangle or ellipse; freehand returns None.
pub(crate) fn shape_points(
    options: Options,
    anchor: [f64; 2],
    point: [f64; 2],
) -> Option<Vec<[f64; 2]>> {
    let mut delta = [point[0] - anchor[0], point[1] - anchor[1]];
    if options.constrain {
        let side = delta[0].abs().max(delta[1].abs());
        delta = [side.copysign(delta[0]), side.copysign(delta[1])];
    }
    let (min, max) = if options.from_center {
        (
            [anchor[0] - delta[0].abs(), anchor[1] - delta[1].abs()],
            [anchor[0] + delta[0].abs(), anchor[1] + delta[1].abs()],
        )
    } else {
        let end = [anchor[0] + delta[0], anchor[1] + delta[1]];
        (
            [anchor[0].min(end[0]), anchor[1].min(end[1])],
            [anchor[0].max(end[0]), anchor[1].max(end[1])],
        )
    };
    match options.shape {
        Shape::Freehand => None,
        Shape::Rectangle => Some(vec![min, [max[0], min[1]], max, [min[0], max[1]]]),
        Shape::Ellipse => {
            let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
            let radius = [(max[0] - min[0]) / 2.0, (max[1] - min[1]) / 2.0];
            // Enough segments that the edge stays round on large canvases.
            let segments = ((radius[0].max(radius[1]) * 0.75) as usize).clamp(24, 256);
            Some(
                (0..segments)
                    .map(|i| {
                        let t = i as f64 / segments as f64 * std::f64::consts::TAU;
                        [
                            center[0] + radius[0] * t.cos(),
                            center[1] + radius[1] * t.sin(),
                        ]
                    })
                    .collect(),
            )
        }
    }
}
pub(crate) fn apply(
    doc: &mut Document,
    points: &[[f64; 2]],
    settings: Settings,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    check_cancel(cancel)?;
    if doc.layers[doc.active].id != settings.target {
        return Err("Captured lasso target is no longer active".into());
    }
    if let Some(blocker) = settings
        .paint_target
        .blocker(doc, doc.active, settings.erase)
    {
        return Err(blocker.into());
    }
    let alpha_locked = doc.layers[doc.active].alpha_locked;
    if settings.erase && alpha_locked && settings.paint_target.is_raster() {
        return Err("Turn off Lock transparency to use lasso eraser".into());
    }
    if !settings.opacity.is_finite() || !(0.0..=1.0).contains(&settings.opacity) {
        return Err("Lasso opacity must be between zero and one".into());
    }
    let mask = Mask::polygon(doc.width, doc.height, points, cancel)?;
    let Some(bounds) = mask.bounds() else {
        return Ok(false);
    };
    if settings.opacity == 0.0 {
        return Ok(false);
    }
    if let crate::paint_target::Target::Mask(id) = settings.paint_target {
        let value = settings
            .paint_target
            .params(settings.color, settings.erase, false)
            .0[0];
        if let Some(coverage) = crate::mask_fill::paint_shape(
            doc,
            doc.active,
            id,
            &mask,
            value,
            settings.opacity,
            cancel,
        )? {
            doc.layers[doc.active].mask.as_mut().unwrap().coverage = coverage;
            return Ok(true);
        }
        return Ok(false);
    }
    let selection = doc.selection;
    let selected = doc.selection_mask.clone();
    let original = doc.layers[doc.active].pixels.clone();
    let mut writer = doc.layers[doc.active].pixels.writer();
    let mut changed = false;
    for y in bounds.top..bounds.bottom {
        check_cancel(cancel)?;
        for x in bounds.left..bounds.right {
            if selection.is_some_and(|s| !s.contains(x, y)) {
                continue;
            }
            let coverage = f32::from(mask.at(x, y)) / 255.0
                * selected
                    .as_ref()
                    .map_or(1.0, |m| f32::from(m.at(x, y)) / 255.0);
            if coverage == 0.0 {
                continue;
            }
            let index = y * doc.width + x;
            let before = original.rgba(index);
            let mut after = before;
            crate::document::paint_pixel(
                &mut after,
                settings.opacity * coverage,
                settings.color,
                settings.erase,
                alpha_locked,
            );
            if before != after {
                writer.rgba_mut(index).copy_from_slice(&after);
                changed = true;
            }
        }
    }
    writer.reclaim_region(
        doc.width,
        [bounds.left, bounds.top, bounds.right, bounds.bottom],
    );
    check_cancel(cancel)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangle_and_ellipse_shapes_follow_constraints() {
        let mut options = Options {
            shape: Shape::Rectangle,
            ..Default::default()
        };
        assert!(shape_points(Options::default(), [0.0; 2], [5.0; 2]).is_none());
        let rect = shape_points(options, [10.0, 10.0], [4.0, 20.0]).unwrap();
        assert_eq!(
            rect,
            vec![[4.0, 10.0], [10.0, 10.0], [10.0, 20.0], [4.0, 20.0]]
        );
        options.constrain = true;
        options.from_center = true;
        let square = shape_points(options, [10.0, 10.0], [12.0, 15.0]).unwrap();
        assert_eq!(square[0], [5.0, 5.0]);
        assert_eq!(square[2], [15.0, 15.0]);
        options.shape = Shape::Ellipse;
        let circle = shape_points(options, [50.0, 50.0], [60.0, 50.0]).unwrap();
        assert!(circle.len() >= 24);
        for p in circle {
            assert!(((p[0] - 50.0).hypot(p[1] - 50.0) - 10.0).abs() < 1e-9);
        }
    }
    use crate::{document::History, document_edits::Edit, render_queue::Queue};
    fn edit(doc: &Document, erase: bool) -> Edit {
        Edit::LassoPaint {
            points: vec![
                [0.0, 0.0],
                [doc.width as f64, 0.0],
                [doc.width as f64, doc.height as f64],
                [0.0, doc.height as f64],
            ],
            settings: Settings {
                target: doc.layers[doc.active].id,
                paint_target: crate::paint_target::Target::Raster,
                color: [0, 0, 200],
                opacity: 0.5,
                erase,
            },
        }
    }
    fn masked(w: usize, h: usize, value: u8) -> Document {
        let mut doc = Document::new(w, h);
        doc.version = 2;
        doc.layers[0].pixels = [17, 83, 201, 129].repeat(w * h).into();
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: LayerId::new(),
            enabled: true,
            linked: false,
            coverage: crate::layer_mask::Mask::new(w, h, value).unwrap(),
        });
        doc
    }
    fn mask_edit(doc: &Document, erase: bool) -> Edit {
        let mut operation = edit(doc, erase);
        if let Edit::LassoPaint { settings, .. } = &mut operation {
            settings.paint_target =
                crate::paint_target::Target::Mask(doc.layers[doc.active].mask.as_ref().unwrap().id);
        }
        operation
    }
    #[test]
    fn mask_lasso_antialias_selection_and_opacity_match_grayscale_raster() {
        for erase in [false, true] {
            for enabled in [false, true] {
                let mut doc = masked(17, 19, 91);
                doc.layers[0].alpha_locked = true;
                doc.layers[0].mask.as_mut().unwrap().enabled = enabled;
                let selection = Mask::from_coverage(
                    17,
                    19,
                    (0..17 * 19).map(|i| [0, 64, 128, 255][i % 4]).collect(),
                )
                .unwrap();
                doc.selection = selection.bounds();
                doc.selection_mask = Some(selection.clone());
                let mut raster = Document::new(17, 19);
                raster.layers[0].pixels = [91, 91, 91, 255].repeat(17 * 19).into();
                raster.layers[0].alpha_locked = true;
                raster.selection = doc.selection;
                raster.selection_mask = Some(selection);
                let mut operation = mask_edit(&doc, erase);
                if let Edit::LassoPaint { points, settings } = &mut operation {
                    *points = vec![[-2.4, 1.3], [15.7, 2.1], [8.4, 17.8], [3.2, 9.6]];
                    settings.opacity = 0.63;
                    settings.color = [18, 169, 223];
                }
                let mut reference = operation.clone();
                if let Edit::LassoPaint { settings, .. } = &mut reference {
                    settings.target = raster.layers[0].id;
                    settings.color = settings.paint_target.params(settings.color, erase, false).0;
                    settings.paint_target = crate::paint_target::Target::Raster;
                    settings.erase = false;
                }
                let (result, outcome) = operation.prepare(&doc).unwrap();
                let (reference, _) = reference.prepare(&raster).unwrap();
                assert!(outcome.changed);
                for y in 0..19 {
                    for x in 0..17 {
                        assert_eq!(
                            result.layers[0].mask.as_ref().unwrap().coverage.at(x, y),
                            reference.layers[0].pixels.rgba(y * 17 + x)[0]
                        );
                    }
                }
                assert!(result.layers[0].pixels.shares(&doc.layers[0].pixels));
                assert_eq!(result.selection_mask, doc.selection_mask);
                assert_eq!(result.layers[0].mask.as_ref().unwrap().enabled, enabled);
                assert!(!result.layers[0].mask.as_ref().unwrap().linked);
            }
        }
    }
    #[test]
    fn mask_lasso_no_op_cancellation_stale_slot_and_history_keep_exact_source() {
        for phase in 0..4 {
            let mut doc = masked(32, 32, 255);
            let mut history = History::new();
            history.checkpoint(&doc);
            doc.layers[0].name = "Redo me".into();
            history.settle(&doc);
            assert!(history.undo(&mut doc));
            let mut queue = Queue::default();
            let operation = mask_edit(&doc, phase == 0); // Revealing white is a no-op.
            if phase == 3 {
                doc.layers[0].mask.as_mut().unwrap().id = LayerId::new();
                assert!(operation.prepare(&doc).is_err());
            }
            let before = doc.clone();
            queue.edit(operation).unwrap();
            if phase == 1 {
                assert!(queue.cancel_background_edit());
            }
            if phase == 2 {
                queue.advance(&mut doc, &mut history, 1).unwrap();
                queue.wait_edit();
                assert!(queue.cancel_background_edit());
            }
            while queue.busy() {
                if queue.waiting_edit() {
                    queue.wait_edit();
                }
                queue.advance(&mut doc, &mut history, 64).unwrap();
            }
            assert!(crate::document::same_snapshot(&doc, &before));
            assert!(history.can_redo() && !history.can_undo());
        }
        let mut doc = masked(16, 16, 0);
        let before = doc.clone();
        let operation = mask_edit(&doc, true);
        let mut history = History::new();
        let mut queue = Queue::default();
        queue.edit(operation).unwrap();
        while queue.busy() {
            if queue.waiting_edit() {
                queue.wait_edit();
            }
            queue.advance(&mut doc, &mut history, 64).unwrap();
        }
        assert_eq!(doc.layers[0].mask.as_ref().unwrap().coverage.at(8, 8), 128);
        assert!(history.undo(&mut doc));
        assert!(crate::document::same_snapshot(&doc, &before));
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].mask.as_ref().unwrap().coverage.at(8, 8), 128);
    }
    #[test]
    fn mask_lasso_folder_constant_storage_and_payload_rejection() {
        let mut doc = masked(8, 8, 0);
        doc.group_active().unwrap();
        let folder = doc.active;
        crate::layer_mask_edits::apply(
            &mut doc,
            folder,
            crate::layer_mask_edits::Command::Create {
                id: LayerId::new(),
                seed: crate::layer_mask_edits::Seed::HideAll,
            },
            None,
        )
        .unwrap();
        doc.layers[folder].alpha_locked = true;
        doc.layers[folder].mask.as_mut().unwrap().enabled = false;
        let mut operation = mask_edit(&doc, true);
        if let Edit::LassoPaint { settings, .. } = &mut operation {
            settings.opacity = 1.0;
        }
        let (result, _) = operation.prepare(&doc).unwrap();
        assert_eq!(
            result.layers[folder]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(0, 0),
            255
        );
        assert_eq!(
            result.layers[folder]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .payload_bytes(),
            0
        );
        assert!(result.layers[0].pixels.shares(&doc.layers[0].pixels));
        doc.layers[folder].visible = false;
        assert!(operation.prepare(&doc).is_err());
        doc.layers[folder].visible = true;
        doc.layers[folder].locked = true;
        assert!(operation.prepare(&doc).is_err());

        let mut doc = masked(2048, 2048, 0);
        let original = doc.layers[0].clone();
        for _ in 1..32 {
            let mut copy = original.clone();
            copy.id = LayerId::new();
            copy.mask = None;
            doc.layers.push(copy);
        }
        doc.validate().unwrap();
        let before = doc.clone();
        let mut operation = mask_edit(&doc, true);
        if let Edit::LassoPaint { points, .. } = &mut operation {
            *points = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        }
        let error = match operation.prepare(&doc) {
            Ok(_) => panic!("Oversized mask lasso published"),
            Err(e) => e,
        };
        assert!(
            error.contains("payload") || error.contains("512"),
            "{error}"
        );
        assert!(crate::document::same_snapshot(&doc, &before));
    }
    #[test]
    fn legacy_lasso_settings_keep_raster_json_and_new_targets_roundtrip() {
        let doc = Document::new(2, 2);
        let Edit::LassoPaint { mut settings, .. } = edit(&doc, false) else {
            unreachable!()
        };
        let old = serde_json::to_string(&settings).unwrap();
        assert!(!old.contains("paint_target"));
        let decoded: Settings = serde_json::from_str(&old).unwrap();
        assert!(decoded.paint_target.is_raster());
        settings.paint_target = crate::paint_target::Target::Mask(LayerId::new());
        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.paint_target, settings.paint_target);
    }
    #[test]
    fn pending_layer_mask_lasso_and_duplicate_capture_independent_slots() {
        let mut app = crate::Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        app.render_queue.edit(Edit::SelectLayer(0)).unwrap();
        app.new_layer();
        let original = app.input_target().unwrap();
        assert!(app.create_canvas_mask(crate::layer_mask_edits::Seed::HideAll));
        app.select_tool(crate::tools::Tool::Lasso);
        app.lasso.erase = true;
        app.brush.opacity = 1.0;
        assert!(app.input_layer().is_none());
        let points = [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]];
        for (i, p) in points.into_iter().enumerate() {
            app.update_selection_drag(p, [p[0] as usize, p[1] as usize], i == 0, i == 3, true);
        }
        app.duplicate_layer();
        let copied = app.input_target().unwrap();
        assert_eq!(
            app.input_stroke_target(),
            crate::paint_target::Target::Mask(copied)
        );
        app.select_tool(crate::tools::Tool::Lasso);
        app.lasso.erase = false;
        app.color = [0; 3];
        let points = [[0.0, 0.0], [4.0, 0.0], [4.0, 8.0], [0.0, 8.0]];
        for (i, p) in points.into_iter().enumerate() {
            app.update_selection_drag(p, [p[0] as usize, p[1] as usize], i == 0, i == 3, true);
        }
        app.flush_render_queue();
        let original = app.doc.layers.iter().find(|l| l.id == original).unwrap();
        let copy = app.doc.layers.iter().find(|l| l.id == copied).unwrap();
        assert_eq!(original.mask.as_ref().unwrap().coverage.at(2, 2), 255);
        assert_eq!(copy.mask.as_ref().unwrap().coverage.at(2, 2), 0);
        assert_eq!(copy.mask.as_ref().unwrap().coverage.at(6, 2), 255);
        assert!(original.pixels.iter().all(|b| *b == 0) && copy.pixels.iter().all(|b| *b == 0));
        app.undo();
        assert_eq!(
            app.doc.layers[app.doc.active]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(2, 2),
            255
        );
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        app.undo();
        assert_eq!(
            app.doc.layers[app.doc.active]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(2, 2),
            0
        );
    }
    #[test]
    fn uniform_shape_respects_soft_selection_hidden_rgb_and_alpha_lock() {
        for erase in [false, true] {
            for alpha_locked in [false, true] {
                let mut doc = Document::new(4, 1);
                doc.layers[0].pixels.writer().write_region(
                    4,
                    [0, 0, 4, 1],
                    &[
                        200, 0, 0, 255, 200, 0, 0, 255, 200, 0, 0, 255, 77, 66, 55, 0,
                    ],
                );
                let mask = Mask::from_coverage(4, 1, vec![255, 128, 0, 255]).unwrap();
                doc.selection = mask.bounds();
                doc.selection_mask = Some(mask.clone());
                doc.layers[0].alpha_locked = alpha_locked;
                let operation = edit(&doc, erase);
                if erase && alpha_locked {
                    assert!(operation.prepare(&doc).is_err());
                    continue;
                }
                let (result, outcome) = operation.prepare(&doc).unwrap();
                assert!(outcome.changed && outcome.undoable && outcome.redraw);
                let expected = if erase {
                    [
                        [200, 0, 0, 128],
                        [200, 0, 0, 191],
                        [200, 0, 0, 255],
                        [77, 66, 55, 0],
                    ]
                } else {
                    [
                        [100, 0, 100, 255],
                        [150, 0, 50, 255],
                        [200, 0, 0, 255],
                        if alpha_locked {
                            [77, 66, 55, 0]
                        } else {
                            [0, 0, 200, 128]
                        },
                    ]
                };
                for (index, pixel) in expected.into_iter().enumerate() {
                    assert_eq!(result.layers[0].pixels.rgba(index), pixel);
                }
                assert_eq!(result.selection, doc.selection);
                assert_eq!(result.selection_mask, Some(mask));
                assert_eq!(doc.layers[0].pixels.rgba(0), [200, 0, 0, 255]);
            }
        }
    }
    #[test]
    fn invalid_targets_paths_and_protected_layers_preserve_source() {
        for protection in 0..5 {
            let mut doc = Document::new(8, 8);
            if protection == 0 {
                doc.layers[0].locked = true;
            }
            if protection == 1 {
                doc.layers[0].visible = false;
            }
            if protection == 2 {
                doc.group_active().unwrap();
            }
            let mut operation = edit(&doc, false);
            if let Edit::LassoPaint { points, settings } = &mut operation {
                if protection == 3 {
                    settings.target = LayerId::new();
                }
                if protection == 4 {
                    points[0][0] = f64::NAN;
                }
            }
            let before = doc.clone();
            assert!(operation.prepare(&doc).is_err());
            assert!(crate::document::same_snapshot(&doc, &before));
        }
    }
    #[test]
    fn no_op_eraser_keeps_redo_and_empty_sparse_storage() {
        let mut doc = Document::new(512, 512);
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.layers[0].opacity = 0.5;
        history.settle(&doc);
        assert!(history.undo(&mut doc));
        let mut queue = Queue::default();
        queue.edit(edit(&doc, true)).unwrap();
        while queue.busy() {
            if queue.waiting_edit() {
                queue.wait_edit();
            }
            queue.advance(&mut doc, &mut history, 64).unwrap();
        }
        assert!(history.can_redo() && !history.can_undo());
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
    }
    #[test]
    fn canceled_or_stale_shape_workers_preserve_artwork_and_redo() {
        for phase in 0..3 {
            let mut doc = Document::new(32, 32);
            let mut history = History::new();
            history.checkpoint(&doc);
            doc.layers[0].opacity = 0.5;
            history.settle(&doc);
            assert!(history.undo(&mut doc));
            let mut queue = Queue::default();
            let ticket = queue.edit(edit(&doc, false)).unwrap();
            if phase > 0 {
                queue.advance(&mut doc, &mut history, 1).unwrap();
                queue.wait_edit();
            }
            if phase == 2 {
                doc.layers[0].name = "Newer state".into();
            } else {
                assert!(queue.cancel_background_edit());
            }
            let progress = queue.advance(&mut doc, &mut history, 4096).unwrap();
            assert_eq!(progress.edits[0].0, ticket);
            assert!(progress.edits[0].1.is_err());
            assert_eq!(doc.layers[0].pixels.capacity(), 0);
            assert!(history.can_redo() && !history.can_undo());
        }
    }
}
