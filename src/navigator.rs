use crate::{
    Dudel, egui,
    icons::{self, Icon},
    view,
};
use egui::{Color32, Pos2, Sense, Stroke, pos2, vec2};

pub(crate) struct PaintRequest {
    painter: egui::Painter,
    slot: egui::layers::ShapeIdx,
    area: egui::Rect,
    view: view::View,
    dimensions: [usize; 2],
    pass: u64,
}

impl Dudel {
    pub(crate) fn finish_navigator_paint(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.navigator_paint.take() {
            if request.pass != ctx.cumulative_pass_nr() {
                return;
            }
            if request.dimensions == [self.doc.width, self.doc.height] {
                request.painter.set(
                    request.slot,
                    egui::Shape::Vec(self.layered_canvas_shapes(request.area, request.view, true)),
                );
            } else {
                ctx.request_repaint();
            }
        }
    }
    /// Whole-artwork preview with the visible area outlined. Click or drag in
    /// the preview to move the view there.
    pub(crate) fn navigator_ui(&mut self, ui: &mut egui::Ui, maximum_height: f32) {
        const MARGIN: f32 = 8.0;
        let width = ui.available_width();
        let aspect = self.doc.width as f32 / self.doc.height as f32;
        // Default to the height that shows the canvas at the full panel width,
        // capped for tall canvases; the handle below the preview resizes it.
        let row_height = ui
            .spacing()
            .interact_size
            .y
            .max(
                ui.text_style_height(&egui::TextStyle::Button)
                    + 2.0 * ui.spacing().button_padding.y,
            )
            .max(30.0);
        // Zoom and rotation live in the status bar; the navigator keeps one
        // row of view buttons, or a menu when the area is too short for it.
        let controls = 10.0 + row_height + 2.0 * ui.spacing().item_spacing.y;
        let compact = maximum_height < 80.0 + controls;
        let height = self
            .navigator_height
            .unwrap_or(((width - MARGIN) / aspect + MARGIN).min(280.0))
            .clamp(80.0, 600.0)
            .min((maximum_height - controls).max(if compact { 36.0 } else { 80.0 }));
        let (frame, response) =
            ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        // Fit the whole canvas inside the frame, whichever side limits it.
        let room = (frame.size() - vec2(MARGIN, MARGIN)).max(vec2(1.0, 1.0));
        let size = if room.x / room.y > aspect {
            vec2(room.y * aspect, room.y)
        } else {
            vec2(room.x, room.x / aspect)
        };
        let preview = egui::Rect::from_center_size(frame.center(), size);
        let painter = ui.painter_at(frame);
        painter.rect_filled(frame, 6.0, ui.visuals().extreme_bg_color);
        let scale = preview.width() / self.doc.width as f32;
        // Reserve the image below the outline, then fill it after canvas upload.
        // Sidebar layout runs first, so painting now could capture an older frame.
        self.navigator_paint = Some(PaintRequest {
            painter: painter.clone(),
            slot: painter.add(egui::Shape::Noop),
            area: preview,
            view: view::View {
                origin: preview.center(),
                zoom: scale,
                rotation: 0.0,
                mirror: false,
                half: vec2(self.doc.width as f32 / 2.0, self.doc.height as f32 / 2.0),
            },
            dimensions: [self.doc.width, self.doc.height],
            pass: ui.ctx().cumulative_pass_nr(),
        });
        if self.canvas_area.width() > 0.0 && self.zoom > 0.0 {
            let view = self.view(self.canvas_area);
            let area = self.canvas_area;
            let outline: Vec<Pos2> = [
                area.left_top(),
                area.right_top(),
                area.right_bottom(),
                area.left_bottom(),
            ]
            .map(|corner| preview.min + view.to_doc(corner).to_vec2() * scale)
            .to_vec();
            let accent = ui.visuals().selection.stroke.color;
            painter.add(egui::Shape::closed_line(
                outline.clone(),
                Stroke::new(3.0_f32, Color32::from_black_alpha(120)),
            ));
            painter.add(egui::Shape::closed_line(
                outline,
                Stroke::new(1.5_f32, accent),
            ));
        }
        if (response.dragged() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let point = ((pointer - preview.min) / scale).to_pos2();
            self.center_view_on(pos2(
                point.x.clamp(0.0, self.doc.width as f32),
                point.y.clamp(0.0, self.doc.height as f32),
            ));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        response.on_hover_text("Click or drag to move the view");
        match crate::workspace::split_handle(ui) {
            crate::workspace::SplitDrag::Reset => self.navigator_height = None,
            crate::workspace::SplitDrag::By(delta) => {
                self.navigator_height = Some((height + delta).clamp(80.0, 600.0))
            }
            crate::workspace::SplitDrag::None => {}
        }
        if compact {
            ui.menu_button("View options", |ui| self.navigator_view_controls(ui));
        } else {
            self.navigator_view_controls(ui);
        }
    }

    fn navigator_view_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if icons::button(ui, Icon::Fit, 30.0, self.auto_fit, true)
                .on_hover_text(
                    self.command_hint(crate::commands::Action::Fit, "Fit the whole canvas"),
                )
                .clicked()
            {
                self.fit_view();
            }
            if ui
                .add(egui::Button::new("100%"))
                .on_hover_text(
                    self.command_hint(crate::commands::Action::ActualSize, "Actual pixels"),
                )
                .clicked()
            {
                self.actual_size();
            }
            if ui
                .add(egui::Button::new("Flip canvas").selected(self.mirror))
                .on_hover_text(self.command_hint(
                    crate::commands::Action::FlipView,
                    "Flip the view horizontally to check proportions (view only)",
                ))
                .clicked()
            {
                self.toggle_flip();
            }
        });
    }

    /// Zoom out / slider / zoom in with a percentage field.
    pub(crate) fn zoom_controls(&mut self, ui: &mut egui::Ui, slider_width: f32) {
        let parts: [fn(&mut Self, &mut egui::Ui); 4] = [
            |app, ui| {
                if icons::button(ui, Icon::ZoomOut, 26.0, false, true)
                    .on_hover_text(app.command_hint(crate::commands::Action::ZoomOut, "Zoom out"))
                    .clicked()
                {
                    app.step_zoom(0.8);
                }
            },
            |app, ui| {
                // Logarithmic percent so every zoom level gets equal slider travel.
                let mut percent = app.zoom * 100.0;
                let slider = ui.add(
                    egui::Slider::new(
                        &mut percent,
                        view::MIN_ZOOM * 100.0..=view::MAX_ZOOM * 100.0,
                    )
                    .logarithmic(true)
                    .show_value(false),
                );
                if slider.changed() {
                    app.set_zoom(percent / 100.0);
                }
            },
            |app, ui| {
                if icons::button(ui, Icon::ZoomIn, 26.0, false, true)
                    .on_hover_text(app.command_hint(crate::commands::Action::ZoomIn, "Zoom in"))
                    .clicked()
                {
                    app.step_zoom(1.25);
                }
            },
            |app, ui| {
                let mut shown = (app.zoom * 100.0).round();
                if ui
                    .add(
                        egui::DragValue::new(&mut shown)
                            .range(view::MIN_ZOOM * 100.0..=view::MAX_ZOOM * 100.0)
                            .suffix("%")
                            .speed(1.0),
                    )
                    .on_hover_text("Zoom · scroll on the canvas, or hold Ctrl+Space and drag")
                    .changed()
                {
                    app.set_zoom(shown / 100.0);
                }
            },
        ];
        self.control_group(ui, slider_width, &parts);
    }

    /// Rotate left / slider / rotate right with an angle field; the field
    /// tooltip names the drag shortcuts, and double-clicking it resets.
    pub(crate) fn rotation_controls(&mut self, ui: &mut egui::Ui, slider_width: f32) {
        let parts: [fn(&mut Self, &mut egui::Ui); 4] = [
            |app, ui| {
                if icons::button(ui, Icon::RotateLeft, 26.0, false, true)
                    .on_hover_text("Rotate view 15° left · 4")
                    .clicked()
                {
                    app.rotate_view(-15f32.to_radians());
                }
            },
            |app, ui| {
                let mut degrees = app.rotation.to_degrees();
                if ui
                    .add(egui::Slider::new(&mut degrees, -180.0..=180.0).show_value(false))
                    .on_hover_text("Rotate the view (artwork is unchanged)")
                    .changed()
                {
                    app.rotate_view(degrees.to_radians() - app.rotation);
                }
            },
            |app, ui| {
                if icons::button(ui, Icon::RotateRight, 26.0, false, true)
                    .on_hover_text("Rotate view 15° right · 6")
                    .clicked()
                {
                    app.rotate_view(15f32.to_radians());
                }
            },
            |app, ui| {
                let mut shown = app.rotation.to_degrees().round();
                let field = ui
                    .add(
                        egui::DragValue::new(&mut shown)
                            .range(-180.0..=180.0)
                            .suffix("°")
                            .speed(1.0),
                    )
                    .on_hover_text(
                        "View rotation · hold R (or Shift+Space) and drag on the canvas
Double-click to reset · 5",
                    );
                if field.double_clicked() {
                    app.reset_rotation();
                } else if field.changed() {
                    app.rotate_view(shown.to_radians() - app.rotation);
                }
            },
        ];
        self.control_group(ui, slider_width, &parts);
    }

    /// Lay out `parts` left to right even inside a right-to-left row, where
    /// `ui.horizontal` would otherwise mirror the group.
    fn control_group(
        &mut self,
        ui: &mut egui::Ui,
        slider_width: f32,
        parts: &[fn(&mut Self, &mut egui::Ui)],
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().slider_width = slider_width.clamp(50.0, 220.0);
            if ui.layout().prefer_right_to_left() {
                for part in parts.iter().rev() {
                    part(self, ui);
                }
            } else {
                for part in parts {
                    part(self, ui);
                }
            }
        });
    }
}
