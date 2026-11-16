//! App side of `vector_layer`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::vector_layer`, re-exported
//! here so `crate::vector_layer::…` paths keep working.
use crate::{brush::Brush, stroke::Sample};
pub(crate) use dudel_core::vector_layer::*;
use std::sync::Arc;

impl crate::Dudel {
    /// The active layer's index when it is a vector layer drawn on directly
    /// (not its mask).
    pub(crate) fn vector_target(&self) -> Option<usize> {
        if !self.input_stroke_target().is_raster() || self.pending_layers.is_some() {
            return None;
        }
        let index = self.input_layer()?;
        self.doc.layers.get(index)?.vector.as_ref().map(|_| index)
    }
    /// Why the stroke about to start cannot go on this vector layer.
    pub(crate) fn vector_stroke_blocker(&self) -> Option<&'static str> {
        let blocker = self.input_paint_blocker();
        if self.input_stroke_target().is_raster() && blocker == Some(crate::text_layer::BLOCKER) {
            return blocker;
        }
        self.vector_target()?;
        if (self.doc.selection.is_some() || self.doc.selection_mask.is_some())
            && (self.brush.scatter > 0.0 || self.active_symmetry().active())
        {
            Some("Scattered or mirrored lines cannot stop at a selection — deselect to draw here")
        } else if self.scripted_tool.is_some() {
            Some(
                "Scripted tools draw on raster layers — rasterize this vector layer or pick another",
            )
        } else if self.replay.enabled {
            Some("Stroke recording does not support vector layers yet")
        } else {
            None
        }
    }
    /// Start recording the stroke that just began, if it is on a vector layer.
    pub(crate) fn begin_vector_record(&mut self) {
        self.vector_record = None;
        if self.vector_target().is_none() {
            return;
        }
        if let Some(settings) = &self.stroke_settings
            && !settings.erase
        {
            self.vector_record = Some(Stroke::new(
                settings.brush.clone(),
                settings.color,
                settings.symmetry,
            ));
        }
    }

    /// Begin the vector eraser on the active vector layer.
    pub(crate) fn begin_vector_erase(&mut self, brush: Brush) {
        let Some(index) = self.vector_target() else {
            return;
        };
        if let Some(blocker) = self.doc.layers[index]
            .locked
            .then_some("Unlock the layer before erasing")
            .or_else(|| {
                (!self.doc.shown(index)).then_some("Show the layer and its folder before erasing")
            })
        {
            self.status = blocker.into();
            return;
        }
        // Strokes still rendering must land before strokes are removed.
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.flush_render_queue();
        self.vector_erase = Some(Erase {
            layer: self.doc.layers[index].id,
            brush,
            sampler: crate::stroke::Stroke::default(),
            checkpointed: false,
            modified: self.modified,
            removed: 0,
            mode: self.vector_erase_mode,
        });
    }
    /// Feed the vector eraser one pen sample in canvas pixels.
    pub(crate) fn vector_erase_sample(&mut self, sample: Sample, lift: bool) {
        let Some(erase) = &mut self.vector_erase else {
            return;
        };
        let zoom = self.zoom;
        let dabs = if lift {
            erase.sampler.finish_at_zoom(sample, &erase.brush, zoom)
        } else {
            erase.sampler.sample_at_zoom(sample, &erase.brush, zoom)
        };
        let Some(index) = self.doc.layers.iter().position(|l| l.id == erase.layer) else {
            self.vector_erase = None;
            return;
        };
        let size = [self.doc.width, self.doc.height];
        let Some(vector) = &self.doc.layers[index].vector else {
            self.vector_erase = None;
            return;
        };
        let pressure_size = erase.brush.pressure_size;
        let circles: Vec<[f32; 3]> = dabs
            .iter()
            .map(|dab| {
                let scale = if pressure_size {
                    dab.pressure.max(0.1)
                } else {
                    1.0
                };
                [dab.x, dab.y, erase.brush.size * 0.5 * scale]
            })
            .filter(|&[x, y, _]| selected_at(&self.doc, x, y))
            .collect();
        let hit: Vec<Arc<Stroke>> = vector
            .strokes
            .iter()
            .filter(|stroke| {
                circles
                    .iter()
                    .any(|&[x, y, r]| stroke.touches(x, y, r, size))
            })
            .cloned()
            .collect();
        if hit.is_empty() {
            return;
        }
        if !erase.checkpointed {
            erase.checkpointed = true;
            self.history.begin_gesture_checkpoint(&self.doc);
        }
        erase.removed += hit.len();
        let rect = hit.iter().filter_map(|s| s.bounds(size)).reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        });
        let mode = erase.mode;
        let layer = &mut self.doc.layers[index];
        let vector = layer.vector.as_mut().unwrap();
        let all = vector.strokes.clone();
        // Each touched line is removed, or replaced by what is left of it.
        vector.strokes = std::mem::take(&mut vector.strokes)
            .into_iter()
            .flat_map(|stroke| {
                if !hit.iter().any(|h| Arc::ptr_eq(h, &stroke)) {
                    return vec![stroke];
                }
                match mode {
                    EraseMode::Touched => stroke
                        .cut(&circles)
                        .unwrap_or_default()
                        .into_iter()
                        .map(Arc::new)
                        .collect(),
                    EraseMode::Whole => Vec::new(),
                    EraseMode::Crossings => stroke
                        .cut_to_crossings(&circles, &all, size)
                        .unwrap_or_default()
                        .into_iter()
                        .map(Arc::new)
                        .collect(),
                }
            })
            .collect();
        if let Some(rect) = rect {
            let vector = vector.clone();
            if let Err(error) = vector.redraw(&mut layer.pixels, rect, size, &|| false) {
                self.status = error;
            }
            self.dirty_region.include(rect);
        }
        self.changed();
    }
    /// Lift the vector eraser: one undo step for everything it removed.
    pub(crate) fn finish_vector_erase(&mut self) {
        let Some(erase) = self.vector_erase.take() else {
            return;
        };
        if erase.checkpointed {
            self.history.settle(&self.doc);
            self.status = format!(
                "Erased {} {} vector stroke{}",
                if erase.mode == EraseMode::Whole {
                    "from"
                } else {
                    "parts of"
                },
                erase.removed,
                if erase.removed == 1 { "" } else { "s" }
            );
        } else {
            self.modified = erase.modified;
        }
    }
    /// Give the active vector layer's strokes (those in the selection, or
    /// all) the current color, or the current brush.
    pub(crate) fn restyle_vector(&mut self, use_color: bool) {
        let Some(target) = self.input_target() else {
            return;
        };
        if !use_color && self.tool != crate::tools::Tool::Brush {
            self.status = "Pick the brush to apply first (Brush tool)".into();
            return;
        }
        self.close_input_gestures();
        let edit = crate::document_edits::Edit::VectorStyle {
            target,
            color: use_color.then_some(self.color),
            brush: (!use_color).then(|| Box::new(self.brush.clone())),
        };
        self.dispatch_document_edit(
            edit,
            if use_color {
                "Recolored the vector strokes"
            } else {
                "Redrew the vector strokes with the current brush"
            },
            "The strokes already look like that",
        );
    }
    /// Turn the active vector or text layer into an ordinary raster layer.
    pub(crate) fn rasterize_layer(&mut self) {
        let Some(target) = self.input_target() else {
            return;
        };
        self.close_input_gestures();
        self.dispatch_document_edit(
            crate::document_edits::Edit::Rasterize(target),
            "Rasterized the layer",
            "The layer is not a vector or text layer",
        );
    }
    /// Add an empty vector layer above the selected one.
    pub(crate) fn create_vector_layer(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let id = crate::document::LayerId::new();
        self.flush_render_queue();
        self.dispatch_document_edit(
            crate::document_edits::Edit::NewVector(id),
            "New vector layer: strokes stay editable — the eraser removes whole strokes",
            "No layer created",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dudel, egui, input::Packet, tools::Tool};
    use crate::{brush::Brush, document::Document, pixels::Pixels, stroke::Sample};
    use std::sync::Arc;

    fn app() -> Dudel {
        let mut app = Dudel {
            doc: Document::new(96, 64),
            brush: Brush {
                size: 6.0,
                hardness: 0.7,
                stabilization: 4.0,
                taper_out: 6.0,
                scatter: 0.2,
                scatter_seed: 7,
                ..Brush::default()
            },
            zoom: 4.0,
            auto_fit: false,
            ..Default::default()
        };
        app.create_vector_layer();
        app.flush_render_queue();
        app
    }
    /// A pen stroke through real input, from `from` to `to` in canvas pixels.
    fn stroke(app: &mut Dudel, from: [f32; 2], to: [f32; 2]) {
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let n = 16;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let p = egui::pos2(
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t + 6.0 * (t * 6.0).sin(),
            );
            let screen = app.view(app.canvas_area).to_screen(p);
            let sample = Sample {
                x: screen.x,
                y: screen.y,
                pressure: 0.3 + 0.6 * t,
                pressure_known: true,
                time: Some(i as f64 * 0.012),
                ..Default::default()
            };
            let packet = match i {
                0 => Packet::Start(sample),
                i if i == n => Packet::End(sample),
                _ => Packet::Move(sample),
            };
            let mut input = raw();
            input.events.push(egui::Event::PointerMoved(screen));
            let _ = ctx.run(input, |ctx| app.canvas(ctx, &[packet]));
        }
        app.flush_render_queue();
    }
    fn layer(app: &Dudel) -> &crate::document::Layer {
        &app.doc.layers[app.doc.active]
    }
    fn rendered(app: &Dudel) -> Pixels {
        let doc = &app.doc;
        layer(app)
            .vector
            .as_ref()
            .unwrap()
            .render([doc.width, doc.height], &|| false)
            .unwrap()
    }
    fn same_pixels(a: &Pixels, b: &Pixels, count: usize) -> bool {
        (0..count).all(|i| a.rgba(i) == b.rgba(i))
    }

    #[test]
    fn strokes_on_a_vector_layer_are_kept_and_redraw_exactly() {
        let mut app = app();
        assert_eq!(layer(&app).name, "Vector 1");
        stroke(&mut app, [8.0, 16.0], [88.0, 20.0]);
        app.color = [200, 40, 30];
        stroke(&mut app, [10.0, 50.0], [80.0, 12.0]);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes.len(), 2);
        assert_eq!(vector.strokes[1].color, [200, 40, 30]);
        let count = 96 * 64;
        assert!((0..count).any(|i| layer(&app).pixels.rgba(i)[3] > 0));
        // Replaying the recordings gives the very pixels drawn live.
        assert!(same_pixels(&rendered(&app), &layer(&app).pixels, count));
        // Undo takes back a stroke and its recording together.
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(same_pixels(&rendered(&app), &layer(&app).pixels, count));
        app.redo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
    }

    #[test]
    fn the_eraser_removes_whole_strokes_in_one_undo_step() {
        let mut app = app();
        stroke(&mut app, [8.0, 14.0], [88.0, 14.0]);
        stroke(&mut app, [8.0, 50.0], [88.0, 50.0]);
        let before = layer(&app).pixels.clone();
        app.select_tool(Tool::Eraser);
        app.vector_erase_mode = EraseMode::Whole;
        app.brush.size = 4.0;
        // Across the top stroke only.
        stroke(&mut app, [48.0, 2.0], [48.0, 24.0]);
        assert!(app.vector_erase.is_none());
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes.len(), 1, "{}", app.status);
        let count = 96 * 64;
        assert!(same_pixels(&rendered(&app), &layer(&app).pixels, count));
        // The top line is gone, the bottom one untouched.
        assert!((0..96).all(|x| layer(&app).pixels.rgba(14 * 96 + x)[3] == 0));
        assert!((0..96).any(|x| layer(&app).pixels.rgba(50 * 96 + x)[3] > 0));
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
        assert!(same_pixels(&before, &layer(&app).pixels, count));
    }

    #[test]
    fn vector_layers_refuse_pixel_edits_and_scattered_lines_in_selections() {
        let mut app = app();
        stroke(&mut app, [8.0, 14.0], [88.0, 14.0]);
        assert!(app.doc.paint_blocker().unwrap().contains("vector layer"));
        // A scattered line has no single path to stop at the selection.
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 40,
            bottom: 40,
        });
        stroke(&mut app, [8.0, 30.0], [88.0, 30.0]);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(app.status.contains("deselect"), "{}", app.status);
        app.doc.selection = None;
        // Clearing the layer removes every stroke.
        let edit = crate::document_edits::Edit::Clear { whole_layer: true };
        let (cleared, outcome) = edit.prepare(&app.doc).unwrap();
        assert!(outcome.changed);
        let layer = &cleared.layers[cleared.active];
        assert!(layer.vector.as_ref().unwrap().strokes.is_empty());
        assert!((0..96 * 64).all(|i| layer.pixels.rgba(i)[3] == 0));
        // Rasterizing keeps the pixels as an ordinary layer.
        app.rasterize_layer();
        app.flush_render_queue();
        assert!(app.doc.layers[app.doc.active].vector.is_none());
        assert!(app.doc.paint_blocker().is_none());
    }

    #[test]
    fn vector_layers_survive_saving_and_reopening() {
        let mut app = app();
        stroke(&mut app, [8.0, 16.0], [88.0, 20.0]);
        stroke(&mut app, [10.0, 50.0], [80.0, 12.0]);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let (a, b) = (
            &app.doc.layers[app.doc.active],
            &loaded.layers[loaded.active],
        );
        let (va, vb) = (a.vector.as_ref().unwrap(), b.vector.as_ref().unwrap());
        assert_eq!(va.strokes.len(), vb.strokes.len());
        for (x, y) in va.strokes.iter().zip(&vb.strokes) {
            assert_eq!(**x, **y);
        }
        let count = 96 * 64;
        assert!(same_pixels(&a.pixels, &b.pixels, count));
        let redrawn = vb.render([96, 64], &|| false).unwrap();
        assert!(same_pixels(&redrawn, &b.pixels, count));
    }

    /// The layer's strokes and pixels agree.
    fn consistent(app: &Dudel) {
        let count = app.doc.width * app.doc.height;
        assert!(same_pixels(&rendered(app), &layer(app).pixels, count));
    }
    fn alpha(app: &Dudel, x: usize, y: usize) -> u8 {
        layer(app).pixels.rgba(y * app.doc.width + x)[3]
    }
    fn straight(app: &mut Dudel, y: f32) {
        app.brush.stabilization = 0.0;
        app.brush.scatter = 0.0;
        stroke_line(app, [8.0, y], [88.0, y]);
    }
    /// A straight stroke (no wobble) through real input.
    fn stroke_line(app: &mut Dudel, from: [f32; 2], to: [f32; 2]) {
        drag(app, from, to, egui::Modifiers::NONE);
    }
    fn drag(app: &mut Dudel, from: [f32; 2], to: [f32; 2], modifiers: egui::Modifiers) {
        trace(app, &[from, to], modifiers);
    }
    /// Pen input along `points` (16 samples per segment).
    fn trace(app: &mut Dudel, points: &[[f32; 2]], modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let segments = points.len() - 1;
        let n = 16 * segments;
        for i in 0..=n {
            let segment = (i / 16).min(segments - 1);
            let t = (i - segment * 16) as f32 / 16.0;
            let [from, to] = [points[segment], points[segment + 1]];
            let p = egui::pos2(
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            );
            let screen = app.view(app.canvas_area).to_screen(p);
            let sample = Sample {
                x: screen.x,
                y: screen.y,
                pressure: 1.0,
                pressure_known: true,
                time: Some(i as f64 * 0.012),
                ..Default::default()
            };
            let packet = match i {
                0 => Packet::Start(sample),
                i if i == n => Packet::End(sample),
                _ => Packet::Move(sample),
            };
            let mut input = raw();
            input.events.push(egui::Event::PointerMoved(screen));
            let _ = ctx.run(input, |ctx| app.canvas(ctx, &[packet]));
        }
        app.flush_render_queue();
    }
    fn run(app: &mut Dudel, edit: crate::document_edits::Edit) {
        let (doc, outcome) = edit.prepare(&app.doc).unwrap();
        assert!(outcome.changed);
        app.doc = doc;
    }

    #[test]
    fn moving_and_flipping_move_the_strokes() {
        let mut app = app();
        straight(&mut app, 16.0);
        let id = layer(&app).id;
        run(
            &mut app,
            crate::document_edits::Edit::MoveTarget {
                target: id,
                offset: [0, 20],
            },
        );
        consistent(&app);
        assert!(alpha(&app, 48, 36) > 0 && alpha(&app, 48, 16) == 0);
        run(
            &mut app,
            crate::document_edits::Edit::Transform(crate::transform::Transform::FlipVertical),
        );
        consistent(&app);
        // Flipped about the canvas middle (y 32): 36 lands on 28.
        assert!(alpha(&app, 48, 28) > 0 && alpha(&app, 48, 36) == 0);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
    }

    #[test]
    fn the_move_tool_moves_vector_strokes_with_their_pixels() {
        let mut app = app();
        straight(&mut app, 16.0);
        app.select_tool(Tool::Move);
        stroke_line(&mut app, [48.0, 16.0], [48.0, 40.0]);
        app.flush_render_queue();
        consistent(&app);
        assert!(
            alpha(&app, 48, 40) > 0 && alpha(&app, 48, 16) == 0,
            "{}",
            app.status
        );
        // Reopening redraws the same pixels from the moved strokes.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let reloaded = &loaded.layers[loaded.active];
        let redrawn = reloaded
            .vector
            .as_ref()
            .unwrap()
            .render([96, 64], &|| false)
            .unwrap();
        assert!(same_pixels(&redrawn, &layer(&app).pixels, 96 * 64));
    }

    #[test]
    fn free_transform_scales_strokes_and_their_tips() {
        let mut app = app();
        straight(&mut app, 32.0);
        let size = layer(&app).vector.as_ref().unwrap().strokes[0].brush.size;
        let mut settings = app.affine_settings;
        settings.scale_percent = [50.0, 50.0];
        settings.translation = [0.0, 0.0];
        settings.angle_degrees = 0.0;
        settings.warp = None;
        settings.mirror = None;
        let target = layer(&app).id;
        run(
            &mut app,
            crate::document_edits::Edit::AffineSettingsTarget {
                target,
                settings: Box::new(settings),
            },
        );
        consistent(&app);
        let stroke = &layer(&app).vector.as_ref().unwrap().strokes[0];
        assert!(
            (stroke.brush.size - size * 0.5).abs() < 0.01,
            "{}",
            stroke.brush.size
        );
        // Half as long about the canvas middle: 8..88 becomes 28..68.
        assert!(alpha(&app, 30, 32) > 0 && alpha(&app, 12, 32) == 0 && alpha(&app, 84, 32) == 0);
    }

    #[test]
    fn canvas_changes_keep_vector_layers_vector() {
        let mut app = app();
        straight(&mut app, 16.0);
        run(
            &mut app,
            crate::document_edits::Edit::CanvasSize(crate::canvas_size::Settings {
                width: 120,
                height: 80,
                anchor: [2, 2],
                dpi: None,
            }),
        );
        assert!(layer(&app).vector.is_some());
        consistent(&app);
        // Anchored bottom-right: everything moved by (24, 16).
        assert!(alpha(&app, 72, 32) > 0);
        run(&mut app, crate::document_edits::Edit::Rotate(1));
        assert!(layer(&app).vector.is_some());
        consistent(&app);
        // A clockwise turn sends (72, 32) to (80 - 32, 72).
        assert!(alpha(&app, 48, 72) > 0);
    }

    #[test]
    fn the_eraser_takes_only_the_touched_part_of_a_line() {
        let mut app = app();
        straight(&mut app, 32.0);
        app.select_tool(Tool::Eraser);
        app.brush.size = 6.0;
        assert_eq!(app.vector_erase_mode, EraseMode::Touched);
        stroke_line(&mut app, [48.0, 20.0], [48.0, 44.0]);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert_eq!(strokes.len(), 2, "{}", app.status);
        consistent(&app);
        assert_eq!(alpha(&app, 48, 32), 0);
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 76, 32) > 0);
        // One undo step brings the whole line back.
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(alpha(&app, 48, 32) > 0);
    }

    #[test]
    fn strokes_take_the_current_color_or_brush() {
        let mut app = app();
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        app.color = [10, 200, 30];
        app.restyle_vector(true);
        app.flush_render_queue();
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert!(
            vector.strokes.iter().all(|s| s.color == [10, 200, 30]),
            "{}",
            app.status
        );
        // Only strokes in the selection take the bigger brush.
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 40,
            right: 96,
            bottom: 64,
        });
        app.brush.size = 12.0;
        app.restyle_vector(false);
        app.flush_render_queue();
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes[0].brush.size, 6.0);
        assert_eq!(vector.strokes[1].brush.size, 12.0);
    }

    #[test]
    fn cut_pieces_keep_the_stabilized_line_where_it_was() {
        let mut app = app();
        app.brush.scatter = 0.0;
        app.brush.stabilization = 8.0;
        stroke(&mut app, [8.0, 30.0], [88.0, 34.0]);
        let before = layer(&app).pixels.clone();
        app.select_tool(Tool::Eraser);
        app.brush.size = 4.0;
        stroke_line(&mut app, [48.0, 4.0], [48.0, 60.0]);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
        consistent(&app);
        // Away from the cut the pieces draw the line as it was drawn.
        let (mut total, mut count) = (0u64, 0u64);
        for y in 0..64 {
            for x in (0..30).chain(66..96) {
                let i = y * 96 + x;
                let (a, b) = (before.rgba(i)[3], layer(&app).pixels.rgba(i)[3]);
                total += u64::from(a.abs_diff(b));
                count += 1;
            }
        }
        let mean = total as f64 / count as f64;
        assert!(mean < 1.0, "pieces drift from the line: {mean}");
    }

    /// A vector layer holding two CSP lines, as an import makes it.
    fn csp_app() -> Dudel {
        let line = |y: f64, color: [u8; 3]| crate::clip_vector::Stroke {
            color,
            opacity: 1.0,
            width: 3.0,
            style: 1,
            antialias: 2,
            points: (0..=8)
                .map(|i| crate::clip_vector::Point {
                    x: 8.0 + f64::from(i) * 10.0,
                    y,
                    pressure: 0.6 + f64::from(i) * 0.05,
                })
                .collect(),
        };
        let lines = vec![line(20.0, [0, 0, 0]), line(44.0, [200, 0, 0])];
        let mut app = Dudel {
            doc: Document::new(96, 64),
            zoom: 4.0,
            auto_fit: false,
            ..Default::default()
        };
        let layer = &mut app.doc.layers[0];
        layer.pixels = crate::clip_vector::render(&lines, [96, 64], &|| false).unwrap();
        layer.vector = Some(VectorLayer {
            strokes: lines
                .into_iter()
                .map(|l| Arc::new(Stroke::from_csp(l)))
                .collect(),
        });
        app
    }

    #[test]
    fn csp_lines_are_editable_strokes() {
        let mut app = csp_app();
        consistent(&app);
        // The eraser cuts a CSP line where it covers it.
        app.select_tool(Tool::Eraser);
        app.brush.size = 4.0;
        stroke_line(&mut app, [48.0, 10.0], [48.0, 30.0]);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert_eq!(strokes.len(), 3, "{}", app.status);
        assert!(strokes.iter().all(|s| s.csp.is_some()));
        consistent(&app);
        assert_eq!(alpha(&app, 48, 20), 0);
        assert!(alpha(&app, 20, 20) > 0 && alpha(&app, 76, 20) > 0);
        // Moving and scaling keep them CSP lines; scaling thins them.
        let id = layer(&app).id;
        run(
            &mut app,
            crate::document_edits::Edit::MoveTarget {
                target: id,
                offset: [0, 4],
            },
        );
        consistent(&app);
        assert!(alpha(&app, 20, 24) > 0 && alpha(&app, 20, 20) == 0);
        let matrix = crate::affine::Matrix([0.5, 0.0, 0.0, 0.5, 24.0, 16.0]);
        run(
            &mut app,
            crate::document_edits::Edit::AffineTarget {
                target: id,
                matrix,
                sampling: crate::affine::Sampling::Bilinear,
            },
        );
        consistent(&app);
        let widths: Vec<f64> = layer(&app)
            .vector
            .as_ref()
            .unwrap()
            .strokes
            .iter()
            .map(|s| s.csp.as_ref().unwrap().width)
            .collect();
        assert!(widths.iter().all(|w| (w - 1.5).abs() < 1e-6), "{widths:?}");
    }

    #[test]
    fn csp_lines_recolor_take_a_brush_and_survive_saving() {
        let mut app = csp_app();
        app.color = [0, 120, 255];
        app.restyle_vector(true);
        app.flush_render_queue();
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert!(
            vector
                .strokes
                .iter()
                .all(|s| s.csp.as_ref().unwrap().color == [0, 120, 255])
        );
        // Saved and reopened, the lines draw the same pixels.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let reloaded = loaded.layers[0].vector.as_ref().unwrap();
        assert_eq!(reloaded, vector);
        let redrawn = reloaded.render([96, 64], &|| false).unwrap();
        assert!(same_pixels(&redrawn, &layer(&app).pixels, 96 * 64));
        // A brush turns the selected line into an ordinary stroke.
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 32,
            right: 96,
            bottom: 64,
        });
        app.brush.size = 5.0;
        app.restyle_vector(false);
        app.flush_render_queue();
        consistent(&app);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert!(strokes[0].csp.is_some());
        assert!(strokes[1].csp.is_none() && strokes[1].brush.size == 5.0);
        assert!(alpha(&app, 48, 44) > 0);
    }

    #[test]
    fn the_eraser_can_stop_at_crossing_lines() {
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 32.0);
        stroke_line(&mut app, [30.0, 8.0], [30.0, 56.0]);
        stroke_line(&mut app, [64.0, 8.0], [64.0, 56.0]);
        app.select_tool(Tool::Eraser);
        app.vector_erase_mode = EraseMode::Crossings;
        app.brush.size = 2.0;
        // A dab between the crossing lines takes all of the line between them.
        stroke_line(&mut app, [47.0, 30.0], [47.0, 34.0]);
        consistent(&app);
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes.len(),
            4,
            "{}",
            app.status
        );
        for x in [36, 47, 58] {
            assert_eq!(alpha(&app, x, 32), 0, "x {x}");
        }
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 76, 32) > 0);
        assert!(alpha(&app, 30, 20) > 0 && alpha(&app, 64, 44) > 0);
        // Past the last crossing it goes up to the line's end.
        stroke_line(&mut app, [80.0, 30.0], [80.0, 34.0]);
        consistent(&app);
        assert_eq!(alpha(&app, 76, 32), 0);
        assert_eq!(alpha(&app, 70, 32), 0);
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 64, 20) > 0);
        // A line nothing crosses goes whole.
        app.select_tool(Tool::Brush);
        stroke_line(&mut app, [8.0, 60.0], [20.0, 60.0]);
        app.select_tool(Tool::Eraser);
        let lines = layer(&app).vector.as_ref().unwrap().strokes.len();
        stroke_line(&mut app, [14.0, 58.0], [14.0, 62.0]);
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes.len(),
            lines - 1
        );
    }

    #[test]
    fn the_pointer_picks_moves_and_edits_single_lines() {
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        let before = layer(&app).pixels.clone();
        let count = 96 * 64;
        app.select_tool(Tool::Pointer);
        // A drag that starts on a line moves that line alone.
        stroke_line(&mut app, [40.0, 16.0], [40.0, 26.0]);
        consistent(&app);
        assert_eq!(app.picked_lines().len(), 1, "{}", app.status);
        assert!(alpha(&app, 40, 26) > 0 && alpha(&app, 40, 16) == 0);
        assert!(alpha(&app, 40, 48) > 0);
        // One undo step puts it back; the moved line is no longer picked.
        app.undo();
        app.flush_render_queue();
        assert!(same_pixels(&before, &layer(&app).pixels, count));
        assert!(app.picked_lines().is_empty());
        // Shift+click picks a second line; the panel's actions use both.
        let shift = egui::Modifiers::SHIFT;
        drag(&mut app, [40.0, 16.0], [40.0, 16.0], egui::Modifiers::NONE);
        drag(&mut app, [40.0, 48.0], [40.0, 48.0], shift);
        assert_eq!(app.picked_lines().len(), 2);
        assert!(same_pixels(&before, &layer(&app).pixels, count));
        app.resize_picked_lines(10.0);
        consistent(&app);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert!(strokes.iter().all(|s| s.brush.size == 10.0));
        assert!(alpha(&app, 40, 20) > 0 && alpha(&app, 40, 52) > 0);
        app.color = [0, 200, 0];
        app.recolor_picked_lines();
        consistent(&app);
        assert_eq!(&layer(&app).pixels.rgba(16 * 96 + 40)[..3], &[0, 200, 0]);
        // Shift+click on a picked line lets it go.
        drag(&mut app, [40.0, 48.0], [40.0, 48.0], shift);
        assert_eq!(app.picked_lines().len(), 1);
        app.delete_picked_lines();
        consistent(&app);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert_eq!(alpha(&app, 40, 16), 0);
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
        // A press away from every line lets go of the pick.
        drag(&mut app, [40.0, 16.0], [40.0, 16.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1);
        drag(&mut app, [40.0, 32.0], [40.0, 32.0], egui::Modifiers::NONE);
        assert!(app.picked_lines().is_empty());
    }

    #[test]
    fn picked_csp_lines_move_and_resize() {
        let mut app = csp_app();
        app.select_tool(Tool::Pointer);
        stroke_line(&mut app, [48.0, 20.0], [48.0, 30.0]);
        consistent(&app);
        assert!(
            alpha(&app, 48, 30) > 0 && alpha(&app, 48, 20) == 0,
            "{}",
            app.status
        );
        app.resize_picked_lines(2.0);
        consistent(&app);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert_eq!(strokes[0].csp.as_ref().unwrap().width, 1.0);
        assert_eq!(strokes[1].csp.as_ref().unwrap().width, 3.0);
    }

    /// Times the crossing search on every line of a real file's vector
    /// layers (`DUDEL_CSP_FILE`).
    #[test]
    #[ignore]
    fn crossing_search_timing() {
        let path = std::env::var("DUDEL_CSP_FILE").unwrap();
        let doc = crate::clip_metadata::import_document(path.as_ref(), &|| false)
            .unwrap()
            .document;
        let size = [doc.width, doc.height];
        for layer in &doc.layers {
            let Some(vector) = &layer.vector else {
                continue;
            };
            let started = std::time::Instant::now();
            let mut slowest = std::time::Duration::ZERO;
            let mut pieces = 0;
            for stroke in &vector.strokes {
                let one = std::time::Instant::now();
                let path = stroke.path(size);
                let [x, y] = path[path.len() / 2];
                if let Some(cut) = stroke.cut_to_crossings(&[[x, y, 2.0]], &vector.strokes, size) {
                    pieces += cut.len();
                }
                slowest = slowest.max(one.elapsed());
            }
            println!(
                "{}: {} lines -> {pieces} pieces, {:?} total, slowest {:?}",
                layer.name,
                vector.strokes.len(),
                started.elapsed(),
                slowest
            );
        }
    }

    /// Times one frame of dragging each of a real file's biggest lines.
    #[test]
    #[ignore]
    fn line_drag_timing() {
        let path = std::env::var("DUDEL_CSP_FILE").unwrap();
        let doc = crate::clip_metadata::import_document(path.as_ref(), &|| false)
            .unwrap()
            .document;
        let size = [doc.width, doc.height];
        for layer in &doc.layers {
            let Some(vector) = &layer.vector else {
                continue;
            };
            let mut order: Vec<usize> = (0..vector.strokes.len()).collect();
            order.sort_by_key(|&i| std::cmp::Reverse(vector.strokes[i].path(size).len()));
            for &i in order.iter().take(3) {
                let started = std::time::Instant::now();
                let moved = Arc::new(vector.strokes[i].mapped(&|x, y| [x + 3.0, y + 2.0], size));
                let mut strokes = vector.strokes.clone();
                let rect = [vector.strokes[i].bounds(size), moved.bounds(size)]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| {
                        [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[2].max(b[2]),
                            a[3].max(b[3]),
                        ]
                    })
                    .unwrap();
                strokes[i] = moved;
                let mut pixels = layer.pixels.clone();
                VectorLayer { strokes }
                    .redraw(&mut pixels, rect, size, &|| false)
                    .unwrap();
                println!(
                    "{} {}x{}: line {i} ({} px long, rect {}x{}): {:?}",
                    layer.name,
                    size[0],
                    size[1],
                    vector.strokes[i].path(size).len(),
                    rect[2] - rect[0],
                    rect[3] - rect[1],
                    started.elapsed()
                );
            }
        }
    }

    #[test]
    fn picked_lines_smooth_and_taper() {
        let mut app = app();
        app.brush.scatter = 0.0;
        app.brush.taper_out = 0.0;
        app.brush.stabilization = 0.0;
        // A jittery line: 4 px up and down every 4 px.
        let zigzag: Vec<[f32; 2]> = (0..=20)
            .map(|i| [8.0 + i as f32 * 4.0, if i % 2 == 0 { 28.0 } else { 32.0 }])
            .collect();
        trace(&mut app, &zigzag, egui::Modifiers::NONE);
        let size = [96, 64];
        let wobble = |app: &Dudel| {
            let path = layer(app).vector.as_ref().unwrap().strokes[0]
                .path(size)
                .to_vec();
            let mean = path.iter().map(|p| p[1]).sum::<f32>() / path.len() as f32;
            path.iter().map(|p| (p[1] - mean).abs()).sum::<f32>() / path.len() as f32
        };
        let ends = |app: &Dudel| {
            let commands = &layer(app).vector.as_ref().unwrap().strokes[0].commands;
            let points: Vec<[f32; 2]> = commands
                .iter()
                .filter_map(|c| match c {
                    Command::Sample { sample, .. } => Some([sample.x, sample.y]),
                    _ => None,
                })
                .collect();
            [points[0], *points.last().unwrap()]
        };
        let (before, before_ends) = (wobble(&app), ends(&app));
        app.select_tool(Tool::Pointer);
        drag(&mut app, [8.0, 28.0], [8.0, 28.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1, "{}", app.status);
        app.smooth_picked_lines();
        consistent(&app);
        assert!(
            wobble(&app) < before * 0.5,
            "{} -> {}",
            before,
            wobble(&app)
        );
        assert_eq!(ends(&app), before_ends);
        app.taper_picked_lines(Some(30.0), None);
        consistent(&app);
        let stroke = &layer(&app).vector.as_ref().unwrap().strokes[0];
        assert_eq!(stroke.taper(), [30.0, 0.0]);
        assert_eq!(stroke.brush.taper_in, 30.0);
        // One undo step per change.
        app.undo();
        app.flush_render_queue();
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes[0].taper(),
            [0.0, 0.0]
        );
    }

    #[test]
    fn csp_lines_taper_and_keep_it_saved() {
        let mut app = csp_app();
        let before = alpha(&app, 12, 21);
        app.select_tool(Tool::Pointer);
        drag(&mut app, [48.0, 20.0], [48.0, 20.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1);
        app.taper_picked_lines(Some(40.0), Some(10.0));
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap().clone();
        assert_eq!(vector.strokes[0].taper(), [40.0, 10.0]);
        assert_eq!(vector.strokes[1].taper(), [0.0, 0.0]);
        // The start narrows: rows off its middle lose coverage.
        assert!(before > 0 && alpha(&app, 12, 21) < before, "{before}");
        app.smooth_picked_lines();
        consistent(&app);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let reloaded = loaded.layers[0].vector.as_ref().unwrap();
        assert_eq!(reloaded, layer(&app).vector.as_ref().unwrap());
        let redrawn = reloaded.render([96, 64], &|| false).unwrap();
        assert!(same_pixels(&redrawn, &layer(&app).pixels, 96 * 64));
    }

    #[test]
    fn a_lasso_picks_the_lines_inside_it() {
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        stroke_line(&mut app, [40.0, 30.0], [56.0, 30.0]);
        app.select_tool(Tool::Pointer);
        let none = egui::Modifiers::NONE;
        let around = [
            [30.0, 24.0],
            [66.0, 24.0],
            [66.0, 38.0],
            [30.0, 38.0],
            [30.0, 24.0],
        ];
        trace(&mut app, &around, none);
        let picked = app.picked_lines();
        assert_eq!(picked.len(), 1, "{}", app.status);
        assert!(Arc::ptr_eq(
            &picked[0],
            &layer(&app).vector.as_ref().unwrap().strokes[2]
        ));
        // Lines only partly inside are left out; Shift adds to the pick.
        let wide = [
            [2.0, 8.0],
            [60.0, 8.0],
            [60.0, 56.0],
            [2.0, 56.0],
            [2.0, 8.0],
        ];
        trace(&mut app, &wide, egui::Modifiers::SHIFT);
        assert_eq!(app.picked_lines().len(), 1);
        let all = [
            [2.0, 8.0],
            [94.0, 8.0],
            [94.0, 56.0],
            [2.0, 56.0],
            [2.0, 8.0],
        ];
        trace(&mut app, &all, egui::Modifiers::SHIFT);
        assert_eq!(app.picked_lines().len(), 3);
        // Nothing moved or changed while picking.
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 3);
        // Without the lasso, empty canvas pans instead.
        app.pointer_lasso = false;
        trace(&mut app, &around, none);
        assert!(app.picked_lines().is_empty());
    }

    #[test]
    fn reshaping_bends_a_line_around_the_grab() {
        let mut app = app();
        app.brush.size = 4.0;
        app.brush.taper_out = 0.0;
        straight(&mut app, 32.0);
        straight(&mut app, 56.0);
        let before = layer(&app).pixels.clone();
        app.select_tool(Tool::Pointer);
        app.line_drag = crate::vector_pick::LineDrag::Reshape;
        app.bend_reach = 20.0;
        stroke_line(&mut app, [48.0, 32.0], [48.0, 44.0]);
        consistent(&app);
        assert_eq!(app.status, "Reshaped the line");
        // The grabbed point follows; the line beyond the reach stays.
        assert!(alpha(&app, 48, 44) > 0 && alpha(&app, 48, 32) == 0);
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 76, 32) > 0);
        assert_eq!(alpha(&app, 20, 44), 0);
        // The other line is untouched and the bent one is picked.
        assert!(alpha(&app, 48, 56) > 0);
        assert_eq!(app.picked_lines().len(), 1);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert!(Arc::ptr_eq(&app.picked_lines()[0], &strokes[0]));
        let ends = strokes[0].control_points();
        assert_eq!(ends[0][1], 32.0);
        assert_eq!(ends.last().unwrap()[1], 32.0);
        app.undo();
        app.flush_render_queue();
        assert!(same_pixels(&before, &layer(&app).pixels, 96 * 64));
    }

    #[test]
    fn reshaping_bends_csp_lines_between_their_points() {
        let mut app = csp_app();
        // A straight line of two points: densified, its middle still bends.
        let line = layer(&app).vector.as_ref().unwrap().strokes[0]
            .csp
            .clone()
            .unwrap();
        let two = crate::clip_vector::Stroke {
            points: vec![line.points[0], *line.points.last().unwrap()],
            ..(*line).clone()
        };
        let index = app.doc.active;
        redraw_layer(
            &mut app.doc,
            index,
            VectorLayer {
                strokes: vec![Arc::new(Stroke::from_csp(two))],
            },
        )
        .unwrap();
        app.select_tool(Tool::Pointer);
        app.line_drag = crate::vector_pick::LineDrag::Reshape;
        app.bend_reach = 15.0;
        stroke_line(&mut app, [48.0, 20.0], [48.0, 30.0]);
        consistent(&app);
        assert!(
            alpha(&app, 48, 30) > 0 && alpha(&app, 48, 20) == 0,
            "{}",
            app.status
        );
        assert!(alpha(&app, 14, 20) > 0 && alpha(&app, 82, 20) > 0);
        let stroke = &layer(&app).vector.as_ref().unwrap().strokes[0];
        assert!(stroke.csp.as_ref().unwrap().points.len() > 20);
    }

    #[test]
    fn densified_lines_draw_the_same() {
        let mut app = app();
        app.brush.scatter = 0.0;
        stroke(&mut app, [8.0, 30.0], [88.0, 30.0]);
        let original = layer(&app).vector.as_ref().unwrap().strokes[0].clone();
        let dense = original.densified(1.0);
        assert!(dense.commands.len() > original.commands.len());
        let size = [96, 64];
        let paths = [original.path(size), dense.path(size)];
        // The stabilizer sees more samples, so allow a fraction of a pixel.
        let far = paths[1].iter().map(|p| {
            paths[0]
                .iter()
                .map(|q| (p[0] - q[0]).hypot(p[1] - q[1]))
                .fold(f32::MAX, f32::min)
        });
        assert!(far.fold(0.0, f32::max) < 1.0);
    }

    #[test]
    fn selections_limit_vector_lines_and_the_vector_eraser() {
        let mut app = app();
        app.brush.size = 6.0;
        app.brush.taper_out = 0.0;
        app.brush.pressure_size = false;
        app.doc.selection = Some(crate::selection::Selection {
            left: 30,
            top: 0,
            right: 60,
            bottom: 64,
        });
        straight(&mut app, 20.0);
        // The part inside stays an editable line, and only that was drawn.
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes.len(), 1, "{}", app.status);
        let path = vector.strokes[0].path([96, 64]);
        assert!(
            path.iter().all(|p| p[0] >= 33.0 && p[0] <= 57.0),
            "{path:?}"
        );
        consistent(&app);
        assert_eq!(alpha(&app, 20, 20), 0);
        assert_eq!(alpha(&app, 70, 20), 0);
        assert!(alpha(&app, 45, 20) > 0);
        assert!(
            (0..96)
                .filter(|&x| alpha(&app, x, 20) > 0)
                .all(|x| (30..60).contains(&x))
        );
        // Undo takes the whole stroke back in one step.
        app.undo();
        app.flush_render_queue();
        assert!(layer(&app).vector.as_ref().unwrap().strokes.is_empty());
        assert_eq!(alpha(&app, 45, 20), 0);
        // A line past the selection is erased only inside it.
        app.doc.selection = None;
        straight(&mut app, 20.0);
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 48,
            bottom: 64,
        });
        app.select_tool(Tool::Eraser);
        app.brush.size = 12.0;
        app.brush.scatter = 0.0;
        drag(&mut app, [40.0, 8.0], [40.0, 32.0], egui::Modifiers::NONE);
        drag(&mut app, [56.0, 8.0], [56.0, 32.0], egui::Modifiers::NONE);
        consistent(&app);
        assert_eq!(alpha(&app, 40, 20), 0);
        assert!(alpha(&app, 56, 20) > 0);
    }

    #[test]
    fn a_selection_narrower_than_the_line_still_keeps_its_middle() {
        let mut app = app();
        app.brush.size = 20.0;
        app.brush.taper_out = 0.0;
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 18,
            right: 96,
            bottom: 22,
        });
        straight(&mut app, 20.0);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        consistent(&app);
    }

    #[test]
    fn line_centres_stop_a_fill_under_the_line() {
        // A raster layer beneath a box of vector lines.
        let mut app = app();
        app.brush.size = 4.0;
        app.brush.taper_out = 0.0;
        for (from, to) in [
            ([20.0, 10.0], [70.0, 10.0]),
            ([70.0, 10.0], [70.0, 50.0]),
            ([70.0, 50.0], [20.0, 50.0]),
            ([20.0, 50.0], [20.0, 10.0]),
        ] {
            app.brush.stabilization = 0.0;
            app.brush.scatter = 0.0;
            stroke_line(&mut app, from, to);
        }
        let lines = app.doc.active;
        app.doc.active = 0;
        let vectors = app.doc.layers[lines].vector.clone();
        let request = |source| crate::fill::Request {
            point: [45, 30],
            color: [10, 200, 40],
            opacity: 1.0,
            tolerance: 0,
            source,
            options: crate::fill::Options {
                line_centres: true,
                ..Default::default()
            },
        };
        let filled = |doc: &Document, x: usize, y: usize| doc.layers[0].pixels.rgba(y * 96 + x)[3];
        for source in [
            crate::fill::Source::Active,
            crate::fill::Source::Visible,
            crate::fill::Source::Layer(app.doc.layers[lines].id),
        ] {
            let mut doc = app.doc.clone();
            assert!(
                doc.fill_request(request(source), None).unwrap(),
                "{source:?}"
            );
            // Right up to and under the line's middle, and not past it.
            assert_eq!(filled(&doc, 45, 30), 255, "{source:?}");
            assert_eq!(filled(&doc, 45, 11), 255, "{source:?}");
            assert_eq!(filled(&doc, 45, 10), 255, "{source:?}");
            assert_eq!(filled(&doc, 45, 9), 0, "{source:?}");
            assert_eq!(filled(&doc, 10, 30), 0, "{source:?}");
            assert_eq!(doc.layers[lines].vector, vectors, "{source:?}");
        }
        // A click on a line has nothing to fill; a hidden layer is no wall.
        let mut doc = app.doc.clone();
        let on_line = crate::fill::Request {
            point: [45, 10],
            ..request(crate::fill::Source::Active)
        };
        assert!(!doc.fill_request(on_line, None).unwrap());
        doc.layers[lines].visible = false;
        assert!(
            doc.fill_request(request(crate::fill::Source::Active), None)
                .unwrap()
        );
        assert_eq!(filled(&doc, 10, 30), 255);
    }

    #[test]
    fn handles_scale_and_rotate_picked_lines() {
        let mut app = app();
        app.brush.size = 4.0;
        app.brush.taper_out = 0.0;
        app.brush.pressure_size = false;
        straight(&mut app, 30.0);
        app.select_tool(Tool::Pointer);
        drag(&mut app, [48.0, 30.0], [48.0, 30.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1);
        let extent = |app: &Dudel| {
            let path = app.picked_lines()[0].path([96, 64]).to_vec();
            let span = |axis: usize| {
                let values = path.iter().map(|p| p[axis]);
                values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
            };
            [span(0), span(1)]
        };
        let [wide, _] = extent(&app);
        let view = app.view(app.canvas_area);
        let spot = |handle| {
            let screen = app
                .line_handles(view)
                .unwrap()
                .iter()
                .find(|h| h.0 == handle)
                .unwrap()
                .1;
            let doc = view.to_doc(screen);
            [doc.x, doc.y]
        };
        // The bottom right corner, halfway toward the top left one.
        let (corner, opposite) = (
            spot(crate::vector_pick::Handle::Corner(2)),
            spot(crate::vector_pick::Handle::Corner(0)),
        );
        let middle = [
            (corner[0] + opposite[0]) / 2.0,
            (corner[1] + opposite[1]) / 2.0,
        ];
        drag(&mut app, corner, middle, egui::Modifiers::NONE);
        consistent(&app);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!((extent(&app)[0] - wide / 2.0).abs() < 1.5, "{}", app.status);
        assert_eq!(app.picked_lines()[0].brush.size, 2.0);
        // One undo step, and the pick lets go of the scaled line.
        app.undo();
        app.flush_render_queue();
        assert!(
            (layer(&app).vector.as_ref().unwrap().strokes[0]
                .path([96, 64])
                .len() as f32)
                > 0.0
        );
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes[0].brush.size,
            4.0
        );
        // A quarter turn about the middle stands the line up.
        drag(&mut app, [48.0, 30.0], [48.0, 30.0], egui::Modifiers::NONE);
        let view = app.view(app.canvas_area);
        let handles = app.line_handles(view).unwrap();
        let rotate = handles
            .iter()
            .find(|h| h.0 == crate::vector_pick::Handle::Rotate)
            .unwrap()
            .1;
        let from = view.to_doc(rotate);
        let center = [48.0, 30.0];
        let [dx, dy] = [from.x - center[0], from.y - center[1]];
        drag(
            &mut app,
            [from.x, from.y],
            [center[0] - dy, center[1] + dx],
            egui::Modifiers::NONE,
        );
        consistent(&app);
        let [across, down] = extent(&app);
        assert!(across < 3.0 && (down - wide).abs() < 3.0, "{across} {down}");
        // A click on a handle changes nothing.
        let before = layer(&app).vector.clone();
        let view = app.view(app.canvas_area);
        let screen = app.line_handles(view).unwrap()[0].1;
        let doc = view.to_doc(screen);
        drag(
            &mut app,
            [doc.x, doc.y],
            [doc.x, doc.y],
            egui::Modifiers::NONE,
        );
        assert_eq!(layer(&app).vector, before);
    }

    #[test]
    fn vector_layers_export_to_svg() {
        let mut app = app();
        app.brush.size = 6.0;
        app.brush.taper_out = 0.0;
        app.brush.pressure_size = false;
        app.color = [200, 40, 30];
        straight(&mut app, 20.0);
        app.doc.layers[app.doc.active].name = "Ink & <line>".into();
        let (svg, lines) = crate::svg_export::encode(&app.doc);
        assert_eq!(lines, 1);
        assert!(svg.starts_with("<svg "), "{svg}");
        assert!(svg.contains(r#"viewBox="0 0 96 64""#));
        assert!(svg.contains("Ink &amp; &lt;line&gt;"));
        // One width: a stroked center line.
        assert!(
            svg.contains(r##"stroke="#c8281e" stroke-width="6""##),
            "{svg}"
        );
        assert!(svg.trim_end().ends_with("</svg>"));
        // Pressure gives an outline; a hidden layer exports nothing.
        app.brush.pressure_size = true;
        stroke(&mut app, [8.0, 40.0], [88.0, 44.0]);
        let (svg, lines) = crate::svg_export::encode(&app.doc);
        assert_eq!(lines, 2);
        assert!(svg.contains(r##"fill="#c8281e""##), "{svg}");
        assert!(svg.contains(" A") || svg.contains("A"));
        let active = app.doc.active;
        app.doc.layers[active].visible = false;
        assert_eq!(crate::svg_export::encode(&app.doc).1, 0);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lines.svg");
        app.doc.layers[active].visible = true;
        app.export_svg_to(&path);
        assert!(std::fs::read_to_string(&path).unwrap().contains("<path"));
        assert!(app.status.contains("2 vector line"), "{}", app.status);
    }

    #[test]
    fn line_commands_need_a_pick_and_undo_in_one_step() {
        use crate::{commands::Action, vector_pick::LineDrag};
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        assert!(app.command_disabled(Action::DeleteLines).is_some());
        app.execute_command(Action::LineDrag(LineDrag::Reshape));
        assert!(app.tool == Tool::Pointer);
        assert_eq!(app.line_drag, LineDrag::Reshape);
        app.execute_command(Action::LineDrag(LineDrag::Move));
        drag(&mut app, [48.0, 16.0], [48.0, 16.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1, "{}", app.status);
        app.execute_command(Action::DeleteLines);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(app.command_disabled(Action::DeleteLines).is_some());
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
    }
}
