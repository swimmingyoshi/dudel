//! App side of `canvas_size`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::canvas_size`, re-exported
//! here so `crate::canvas_size::…` paths keep working.
pub(crate) use dudel_core::canvas_size::*;

impl crate::Dudel {
    pub(crate) fn open_canvas_size(&mut self) {
        if self.canvas_edit_ticket.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
        {
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.canvas_resize = Some(Settings {
            width: self.doc.width,
            height: self.doc.height,
            dpi: Some(self.doc.dpi),
            anchor: [1, 1],
        });
    }
    pub(crate) fn dispatch_canvas_size(&mut self, edit: crate::document_edits::Edit) {
        if self.canvas_edit_ticket.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
        {
            self.status = "A canvas bounds change is already running".into();
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let (changed, unchanged) = match &edit {
            crate::document_edits::Edit::CanvasSize(_) => {
                ("Canvas resized", "Canvas size unchanged")
            }
            crate::document_edits::Edit::CropCanvas => (
                "Canvas cropped to selection bounds",
                "Canvas bounds unchanged",
            ),
            crate::document_edits::Edit::Rotate(_) => ("Image rotated", "Image unchanged"),
            _ => {
                self.status = "Invalid canvas operation".into();
                return;
            }
        };
        match self.render_queue.edit(edit) {
            Ok(ticket) => {
                self.canvas_edit_ticket = Some(ticket);
                self.edit_feedback.insert(
                    ticket,
                    crate::render_controller::EditFeedback { changed, unchanged },
                );
                self.status = "Canvas change queued after preceding artwork".into();
            }
            Err(error) => self.status = error,
        }
    }
    pub(crate) fn canvas_size_ui(&mut self, ctx: &crate::egui::Context) {
        use crate::egui;
        if let Some(ticket) = self.canvas_edit_ticket {
            egui::Modal::new(egui::Id::new("canvas bounds progress")).show(ctx, |ui| {
                ui.set_width(300.0);
                ui.heading("Updating canvas");
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finishing the canvas change…");
                });
                if ui.button("Cancel operation").clicked() {
                    self.render_queue.cancel_edit(ticket);
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
            return;
        }
        let Some(mut settings) = self.canvas_resize else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("canvas size")).show(ctx, |ui| {
            ui.set_width(400.0);
            cancel |= crate::dialogs::dialog_title(ui, "Canvas size");
            egui::ScrollArea::vertical().id_salt("canvas size controls")
                .max_height((ctx.content_rect().height() - 96.0).max(80.0))
                .show(ui, |ui| {
            ui.label(format!("Current: {} × {} px", self.doc.width, self.doc.height));
            ui.horizontal(|ui| {
                ui.label("Width");
                ui.add(egui::DragValue::new(&mut settings.width).range(1..=crate::document::MAX_SIDE).suffix(" px"));
                ui.label("Height");
                ui.add(egui::DragValue::new(&mut settings.height).range(1..=crate::document::MAX_SIDE).suffix(" px"));
            });
            let dpi = settings.dpi.get_or_insert(self.doc.dpi);
            ui.horizontal(|ui| {
                ui.label("Resolution");
                ui.add(egui::DragValue::new(dpi).range(1.0..=12000.0).speed(1.0).suffix(" DPI"));
            });
            ui.small(format!("Print size: {:.2} \u{00d7} {:.2} cm", settings.width as f64 / *dpi * 2.54, settings.height as f64 / *dpi * 2.54));
            ui.small("Changing DPI changes print size; pixels stay the same.");
            ui.label("Anchor existing artwork");
            ui.horizontal(|ui| {
                egui::Grid::new("canvas anchor").spacing(egui::vec2(4.0,4.0)).show(ui, |ui| {
                    let names = [["Top left", "Top center", "Top right"], ["Middle left", "Center", "Middle right"], ["Bottom left", "Bottom center", "Bottom right"]];
                    for (y,row) in names.iter().enumerate() { for (x,name) in row.iter().enumerate() {
                        let anchor = [x as u8,y as u8];
                        if anchor_button(ui,anchor,settings.anchor == anchor,name).clicked() { settings.anchor = anchor; }
                    } ui.end_row(); }
                });
                let (area, _) = ui.allocate_exact_size(egui::vec2(144.0,120.0), egui::Sense::hover());
                let offset = settings.offset(self.doc.width, self.doc.height);
                let old = egui::Rect::from_min_size(egui::pos2(offset[0] as f32,offset[1] as f32), egui::vec2(self.doc.width as f32,self.doc.height as f32));
                let new = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(settings.width as f32,settings.height as f32));
                let both = old.union(new);
                let scale = (area.width()/both.width()).min(area.height()/both.height()) * 0.85;
                let rect = |r: egui::Rect| egui::Rect::from_min_max(area.center() + (r.min-both.center()) * scale, area.center() + (r.max-both.center()) * scale);
                let painter = ui.painter_at(area);
                painter.rect_filled(rect(new), 0.0, egui::Color32::from_rgb(43,73,104));
                painter.rect_stroke(rect(new), 0.0, egui::Stroke::new(2.0_f32,egui::Color32::from_rgb(140,190,240)),egui::StrokeKind::Inside);
                painter.rect_stroke(rect(old), 0.0, egui::Stroke::new(1.0_f32,egui::Color32::LIGHT_GRAY),egui::StrokeKind::Inside);
            });
            ui.small("Blue: new canvas · outline: current canvas");
            ui.label("Changes canvas bounds on every layer without scaling the artwork. Added space is transparent. Pixels outside the new bounds are removed; Undo restores them.");
            });
            let valid = settings.validate();
            if let Err(error) = &valid { ui.colored_label(ui.visuals().error_fg_color, error); }
            crate::dialogs::dialog_buttons(ui, |ui| {
                apply = ui.add_enabled(valid.is_ok() && ([settings.width,settings.height] != [self.doc.width,self.doc.height] || settings.dpi.is_some_and(|dpi| dpi != self.doc.dpi)), crate::workspace::primary_button(ui, "Apply")).clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if apply {
            self.canvas_resize = None;
            self.dispatch_canvas_size(crate::document_edits::Edit::CanvasSize(settings));
        } else if cancel || response.should_close() {
            self.canvas_resize = None;
        } else {
            self.canvas_resize = Some(settings);
        }
    }
}
fn anchor_button(
    ui: &mut crate::egui::Ui,
    anchor: [u8; 2],
    selected: bool,
    name: &str,
) -> crate::egui::Response {
    use crate::egui;
    let response = ui
        .add(
            egui::Button::new("")
                .min_size(egui::vec2(32.0, 32.0))
                .selected(selected),
        )
        .on_hover_text(name);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, name)
    });
    let center = response.rect.center();
    let color = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    if anchor == [1, 1] {
        ui.painter().circle_filled(center, 3.0, color);
    } else {
        let direction = egui::vec2(anchor[0] as f32 - 1.0, anchor[1] as f32 - 1.0).normalized();
        let tip = center + direction * 7.0;
        let back = direction * 5.0;
        let across = egui::vec2(-direction.y, direction.x) * 4.0;
        let stroke = egui::Stroke::new(1.5_f32, color);
        ui.painter()
            .line_segment([center - direction * 5.0, tip], stroke);
        ui.painter()
            .line_segment([tip - back + across, tip], stroke);
        ui.painter()
            .line_segment([tip - back - across, tip], stroke);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    use crate::{Dudel, document_edits::Edit, selection_mask::Mask};

    #[test]
    fn dpi_edit_preserves_pixels_and_survives_history_and_save() {
        let mut app = crate::Dudel {
            doc: source(7, 9),
            ..Default::default()
        };
        let original = serde_json::to_value(&app.doc).unwrap();
        app.dispatch_canvas_size(crate::document_edits::Edit::CanvasSize(Settings {
            width: 7,
            height: 9,
            anchor: [1, 1],
            dpi: Some(600.5),
        }));
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(app.doc.dpi, 600.5);
        assert!(app.modified);
        let mut expected = original.clone();
        expected["dpi"] = serde_json::json!(600.5);
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected);
        app.undo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), original);
        app.redo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dpi.dudel");
        crate::storage::save_project(&path, &app.doc).unwrap();
        let loaded = crate::storage::load_project(&path).unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), expected);
        let mut legacy = expected.clone();
        legacy.as_object_mut().unwrap().remove("dpi");
        let legacy: Document = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.dpi, 72.0);
        for dpi in [0.0, -1.0, f64::NAN, f64::INFINITY, 12001.0] {
            assert!(
                resize(
                    &app.doc,
                    Settings {
                        width: 7,
                        height: 9,
                        anchor: [1, 1],
                        dpi: Some(dpi)
                    },
                    None
                )
                .is_err()
            );
        }
    }

    fn source(width: usize, height: usize) -> Document {
        let mut doc = Document::new(width, height);
        doc.add_layer();
        for (layer_index, layer) in doc.layers.iter_mut().enumerate() {
            let mut writer = layer.pixels.writer();
            for y in 0..height {
                for x in 0..width {
                    writer.rgba_mut_at(width, x, y).copy_from_slice(&[
                        x as u8,
                        (y * 29) as u8,
                        (layer_index * 80 + 11) as u8,
                        [0, 64, 255][(x + y) % 3],
                    ]);
                }
            }
        }
        doc.layers[0].locked = true;
        doc.layers[1].alpha_locked = true;
        doc.layers[1].opacity = 0.7;
        doc.layers[1].blend = crate::blend::BlendMode::Multiply;
        doc.add_folder().unwrap();
        let mut coverage = vec![0; width * height];
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                coverage[y * width + x] = (1 + (x + y) % 255) as u8;
            }
        }
        let mask = Mask::from_coverage(width, height, coverage).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        doc.validate().unwrap();
        doc
    }

    #[test]
    fn resize_crop_rotation_and_undo_map_current_guides_and_roundtrip_off_canvas_positions() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            doc: source(7, 5),
            ..Default::default()
        };
        app.guide_lines = vec![
            crate::guides::GuideLine {
                vertical: true,
                position: 4.0,
            },
            crate::guides::GuideLine {
                vertical: false,
                position: 0.0,
            },
        ];
        app.symmetry.center = Some([2.0, 2.0]);
        app.push_ruler(crate::rulers::Ruler::Focus { center: [3.0, 4.0] });
        let original = serde_json::to_value(&app.doc).unwrap();
        app.dispatch_canvas_size(Edit::CanvasSize(Settings {
            width: 11,
            height: 9,
            anchor: [1, 1],
            dpi: None,
        }));
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(app.symmetry.center, Some([4.0, 4.0]));
        assert_eq!(app.guide_lines[0].position, 6.0);
        app.guide_lines[0].position += 1.0; // Independently edited guides must not be undone.
        app.undo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), original);
        assert_eq!(app.guide_lines[0].position, 5.0);
        app.redo();
        assert_eq!(app.guide_lines[0].position, 7.0);
        let resized = serde_json::to_value(&app.doc).unwrap();
        app.rotate_canvas(1);
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!([app.doc.width, app.doc.height], [9, 11]);
        assert!(!app.guide_lines[0].vertical);
        assert_eq!(app.guide_lines[0].position, 7.0);
        app.undo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), resized);
        assert!(app.guide_lines[0].vertical);
        assert_eq!(app.symmetry.center, Some([4.0, 4.0]));
        app.dispatch_canvas_size(Edit::CropCanvas);
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!([app.doc.width, app.doc.height], [5, 3]);
        assert_eq!(app.guide_lines[1].position, -1.0);
        let expected = serde_json::to_value(&app.doc).unwrap();
        let guides = app.guides_record(&app.doc);
        app.undo();
        app.redo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected);
        assert_eq!(app.guides_record(&app.doc), guides);
        let project = directory.path().join("cropped.dudel");
        app.path = Some(project.clone());
        app.save(false);
        crate::workflow_tests::finish_jobs(&mut app);
        let mut reopened = Dudel::default();
        reopened.load_document(project, false);
        crate::workflow_tests::finish_jobs(&mut reopened);
        assert_eq!(serde_json::to_value(&reopened.doc).unwrap(), expected);
        assert_eq!(reopened.guides_record(&reopened.doc), guides);
        assert_eq!(*reopened.doc.canvas_frame, Frame::default());
    }

    #[test]
    fn ordered_canvas_change_cancellation_has_no_partial_artwork_or_history() {
        let mut app = Dudel {
            doc: source(7, 5),
            ..Default::default()
        };
        let before = app.doc.clone();
        app.dispatch_canvas_size(Edit::CanvasSize(Settings {
            width: 9,
            height: 9,
            anchor: [2, 2],
            dpi: None,
        }));
        let ticket = app.canvas_edit_ticket.unwrap();
        assert!(app.render_queue.cancel_edit(ticket));
        crate::workflow_tests::finish_jobs(&mut app);
        assert!(app.canvas_edit_ticket.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert!(!app.history.can_undo());
        assert!(app.status.contains("canceled"));
    }

    #[test]
    fn queued_stroke_resize_save_and_export_share_the_new_bounds_and_reject_old_input() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            doc: Document::new(33, 35),
            ..Default::default()
        };
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 9.0,
                taper_out: 20.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [30, 40, 50],
            secondary: [30, 40, 50],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let sample = crate::stroke::Sample {
            x: 7.5,
            y: 9.5,
            pressure: 1.0,
            ..Default::default()
        };
        let mut painted = app.doc.clone();
        let mut reference = crate::stroke_buffer::StrokeBuffer::default();
        reference.paint(&mut painted, sample, &settings);
        reference.finish_tail(&mut painted, &settings);
        assert!(app.begin_canvas_stroke(settings.clone()));
        app.deliver_canvas_dabs(vec![sample]);
        let bounds = Settings {
            width: 41,
            height: 39,
            anchor: [1, 1],
            dpi: None,
        };
        app.dispatch_canvas_size(Edit::CanvasSize(bounds));
        assert!(app.canvas_edit_ticket.is_some() && !app.stroking);
        assert!(!app.begin_canvas_stroke(settings));
        let project = directory.path().join("resized.dudel");
        let png = directory.path().join("resized.png");
        app.path = Some(project.clone());
        app.save(false);
        app.export_to(png.clone(), true);
        crate::workflow_tests::finish_jobs(&mut app);
        let (expected, _) = resize(&painted, bounds, None).unwrap();
        let reopened = crate::storage::load_project(&project).unwrap();
        assert_eq!([reopened.width, reopened.height], [41, 39]);
        assert_eq!(reopened.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(
            image::open(png).unwrap().into_rgba8().as_raw(),
            &expected.composite_transparent()
        );
        assert!(!app.modified);
        app.undo();
        assert_eq!([app.doc.width, app.doc.height], [33, 35]);
        assert_eq!(app.doc.layers[0].pixels, painted.layers[0].pixels);
        app.undo();
        assert!(app.doc.layers[0].pixels.nonzero_tiles(33, 35).is_empty());
    }
}
