//! App side of `fill`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::fill`, re-exported
//! here so `crate::fill::…` paths keep working.
use crate::{Dudel, egui};
pub(crate) use dudel_core::fill::*;

impl Dudel {
    pub(crate) fn fill_edge_options_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.input_stroke_target().is_raster(), |ui| {
            ui.checkbox(&mut self.fill_options.fill_edges, "Fill to edges")
                .on_hover_text("Fill beneath soft outlines without a pale seam. Follows up to 16 pixels of soft edging, stopping when the boundary fades toward another region. Applies to artwork, not mask coverage.");
            let lines = self.doc.layers.iter().any(|layer| layer.vector.is_some());
            ui.add_enabled_ui(lines, |ui| {
                ui.checkbox(&mut self.fill_options.line_centres, "Stop at line centres")
                    .on_hover_text("Vector lines stop the fill at their middles, so color runs under them with no gap. Uses every shown vector layer, or only the chosen one when the reference is a vector layer.");
            });
        });
        ui.add(egui::Slider::new(&mut self.fill_options.close_gap, 0..=8).text("Close gaps (px)"))
            .on_hover_text("Closes small breaks in outlines. Larger values can skip narrow areas.");
        ui.add(egui::Slider::new(&mut self.fill_options.expand, -16..=16).text("Expand (px)"))
            .on_hover_text("Grow the fill under outlines, or use negative values to shrink. Stays inside the selection.");
    }
    pub(crate) fn fill_canvas(&mut self, x: usize, y: usize) {
        if self.canvas_edit_ticket.is_some() {
            self.status = "Wait for the canvas bounds change to finish".into();
            return;
        }
        if let crate::paint_target::Target::Mask(id) = self.input_stroke_target() {
            self.edit_canvas_mask(crate::layer_mask_edits::Command::Fill {
                id,
                request: Request {
                    point: [x, y],
                    color: self.paint_color(),
                    opacity: self.brush.opacity,
                    tolerance: self.fill_tolerance,
                    source: self.fill_source,
                    options: self.fill_options,
                },
            });
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        self.close_input_gestures();
        let edit = crate::document_edits::Edit::FillTarget {
            target,
            source: self.fill_source,
            point: [x, y],
            color: self.color,
            opacity: self.brush.opacity,
            tolerance: self.fill_tolerance,
            options: self.fill_options,
        };
        match self.render_queue.edit(edit) {
            Ok(ticket) => {
                self.edit_feedback.insert(
                    ticket,
                    crate::render_controller::EditFeedback {
                        changed: "Filled connected area",
                        unchanged: "Fill made no changes",
                    },
                );
                self.status = "Fill queued".into();
                self.note_recent_color();
            }
            Err(error) => self.status = error,
        }
    }
    pub(crate) fn fill_source_ui(&mut self, ui: &mut egui::Ui) {
        if self.render_queue.has_background_edit() && ui.button("Cancel background edit").clicked()
        {
            self.render_queue.cancel_background_edit();
            self.status = "Canceling background edit".into();
        }
        let active_label = if self.input_stroke_target().is_raster() {
            "Active layer"
        } else {
            "Active mask"
        };
        let label = match self.fill_source {
            Source::Active => active_label,
            Source::Visible => "Visible layers",
            Source::Layer(id) => self
                .doc
                .layers
                .iter()
                .find(|layer| layer.id == id)
                .map_or("Missing reference layer", |layer| layer.name.as_str()),
        };
        egui::ComboBox::from_id_salt("fill reference")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.fill_source, Source::Active, active_label);
                ui.selectable_value(&mut self.fill_source, Source::Visible, "Visible layers");
                ui.separator();
                for layer in self.doc.layers.iter().rev().filter(|layer| !layer.folder) {
                    ui.selectable_value(
                        &mut self.fill_source,
                        Source::Layer(layer.id),
                        &layer.name,
                    );
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dudel, document::LayerId};
    use crate::{document::Document, document_edits::Edit};

    fn line_art() -> Document {
        let mut doc = Document::new(5, 3);
        doc.add_layer();
        for y in 0..3 {
            doc.layers[1]
                .pixels
                .writer()
                .rgba_mut(y * 5 + 2)
                .copy_from_slice(&[0, 0, 0, 255]);
        }
        doc.active = 0;
        doc
    }

    #[test]
    fn invalid_reference_target_and_opacity_are_atomic_and_preserve_redo() {
        let doc = line_art();
        let target = doc.layers[0].id;
        for (captured, source, opacity) in [
            (target, Source::Layer(crate::document::LayerId::new()), 1.0),
            (doc.layers[1].id, Source::Active, 1.0),
            (target, Source::Active, f32::NAN),
        ] {
            assert!(
                Edit::FillTarget {
                    target: captured,
                    source,
                    point: [0, 0],
                    color: [17, 83, 201],
                    opacity,
                    tolerance: 0,
                    options: crate::fill::Options::default(),
                }
                .prepare(&doc)
                .is_err()
            );
            assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        }
        let mut app = Dudel {
            doc,
            ..Default::default()
        };
        app.set_canvas_property(target, crate::document_edits::Property::Opacity(0.5));
        app.undo();
        app.render_queue.edit(Edit::SelectLayer(0)).unwrap();
        app.fill_source = Source::Layer(crate::document::LayerId::new());
        app.fill_canvas(0, 0);
        app.flush_render_queue();
        assert!(app.status.contains("reference layer no longer exists"));
        assert!(app.history.can_redo() && !app.history.can_undo());
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn queued_reference_paint_then_fill_uses_completed_pixels_and_separate_undo() {
        use crate::{
            stroke::Sample,
            stroke_buffer::{Settings, StrokeBuffer},
        };
        let mut app = Dudel {
            doc: Document::new(13, 7),
            ..Default::default()
        };
        app.doc.add_layer();
        let line_id = app.doc.layers[1].id;
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 1.0,
                pressure_size: false,
                tip: crate::brush::Tip::Square,
                hardness: 1.0,
                flow: 1.0,
                ..Default::default()
            },
            color: [0; 3],
            secondary: [0; 3],
            erase: false,
            layer: 1,
            symmetry: Default::default(),
        };
        let points: Vec<_> = (0..7)
            .map(|y| Sample {
                x: 6.5,
                y: y as f32 + 0.5,
                pressure: 1.0,
                distance: y as f64,
                ..Default::default()
            })
            .collect();
        let mut expected = app.doc.clone();
        let mut buffer = StrokeBuffer::default();
        for &point in &points {
            buffer.paint(&mut expected, point, &settings);
        }
        buffer.finish_tail(&mut expected, &settings);
        let line = expected.layers[1].pixels.clone();
        assert!((0..7).all(|y| line.rgba(y * 13 + 6)[3] == 255));
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(points);
        assert!(app.select_layer_target(0));
        app.fill_source = Source::Layer(line_id);
        app.color = [17, 83, 201];
        app.brush.opacity = 1.0;
        app.fill_tolerance = 0;
        app.fill_canvas(0, 3);
        assert!(app.doc.layers[1].pixels.iter().all(|byte| *byte == 0));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].pixels, line);
        for y in 0..7 {
            for x in 0..13 {
                assert_eq!(
                    app.doc.layers[0].pixels.rgba(y * 13 + x),
                    if x < 6 { [17, 83, 201, 255] } else { [0; 4] }
                );
            }
        }
        app.undo();
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert_eq!(app.doc.layers[1].pixels, line);
        app.undo();
        assert!(app.doc.layers[1].pixels.iter().all(|byte| *byte == 0));
        assert!(!app.history.can_undo());
    }

    #[test]
    fn folder_references_reject_and_document_replacement_resets_session_layer_reference() {
        let mut doc = Document::new(5, 3);
        doc.group_active().unwrap();
        doc.active = 0;
        let folder = Source::Layer(doc.layers[1].id);
        assert!(doc.fill_using(0, 0, [17, 83, 201], 1.0, 0, folder).is_err());
        assert!(
            doc.fill_using(0, 0, [17, 83, 201], 0.0, 0, Source::Layer(LayerId::new()))
                .is_err()
        );
        assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        let mut app = Dudel {
            fill_source: Source::Layer(doc.layers[0].id),
            doc,
            ..Default::default()
        };
        app.apply_action(crate::document_actions::Action::New(5, 3, 300.0));
        assert_eq!(app.fill_source, Source::Active);
        app.fill_source = Source::Visible;
        app.apply_action(crate::document_actions::Action::New(5, 3, 300.0));
        assert_eq!(app.fill_source, Source::Visible);
    }

    #[test]
    fn saved_worker_fill_excludes_later_fill_and_keeps_two_undo_states() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker-fill.dudel");
        let mut app = Dudel {
            doc: Document::new(75, 73),
            path: Some(path.clone()),
            color: [17, 83, 201],
            ..Default::default()
        };
        app.brush.opacity = 1.0;
        app.fill_canvas(0, 0);
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        app.save(false);
        assert!(app.queued_save.is_some() && app.save_job.is_none());
        app.color = [201, 17, 83];
        app.fill_canvas(0, 0);
        app.flush_render_queue();
        assert!(
            app.doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [201, 17, 83, 255])
        );
        let job = app.save_job.take().unwrap();
        assert_ne!(job.revision, app.revision);
        job.worker.join().unwrap().unwrap();
        assert!(
            crate::storage::load_project(&path).unwrap().layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
        assert!(app.modified);
        app.undo();
        assert!(
            app.doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
        app.undo();
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(!app.history.can_undo());
    }
}
