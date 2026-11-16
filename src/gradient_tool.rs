//! App side of `gradient_tool`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::gradient_tool`, re-exported
//! here so `crate::gradient_tool::…` paths keep working.
pub(crate) use dudel_core::gradient_tool::*;

impl crate::Dudel {
    /// The secondary color, or the secondary mask coverage while a mask is
    /// the painting target.
    pub(crate) fn secondary_paint_color(&self) -> [u8; 3] {
        if self.input_stroke_target().is_raster() {
            self.secondary
        } else {
            [self.mask_colors[1]; 3]
        }
    }

    /// Pointer input while the Fill tool is in Gradient mode: press to start
    /// the line, drag to aim it (Shift snaps to 15°), release to paint.
    pub(crate) fn gradient_packet(
        &mut self,
        point: [f64; 2],
        start: bool,
        end: bool,
        can_start: bool,
        shift: bool,
    ) {
        use crate::selection_ui::Drag;
        if start && can_start {
            if self.canvas_edit_ticket.is_some() {
                self.status = "Wait for the canvas bounds change to finish".into();
                return;
            }
            let paint_target = self.input_stroke_target();
            if let Some(blocker) = self.stroke_target_blocker(paint_target, false) {
                self.status = blocker.into();
                return;
            }
            let Some(target) = self.input_target() else {
                return;
            };
            let layer = self.input_layer().and_then(|i| self.doc.layers.get(i));
            if paint_target.is_raster()
                && layer.is_some_and(|l| l.vector.is_some() || l.text.is_some())
            {
                self.status =
                    "Gradients paint on raster layers — add a new layer, or rasterize this one"
                        .into();
                return;
            }
            self.selection_drag = Some(Drag::Gradient(Box::new(Settings {
                target,
                paint_target,
                start: point,
                end: point,
                style: self.gradient.style,
                reverse: self.gradient.reverse,
                ramp: self
                    .gradient
                    .ramp(self.paint_color(), self.secondary_paint_color()),
                opacity: self.brush.opacity,
            })));
        }
        let Some(Drag::Gradient(settings)) = &mut self.selection_drag else {
            return;
        };
        let mut point = point;
        if shift {
            let (dx, dy) = (point[0] - settings.start[0], point[1] - settings.start[1]);
            let step = 15f64.to_radians();
            let angle = (dy.atan2(dx) / step).round() * step;
            let length = dx.hypot(dy);
            point = [
                settings.start[0] + angle.cos() * length,
                settings.start[1] + angle.sin() * length,
            ];
        }
        settings.end = point;
        if !end {
            return;
        }
        let Some(Drag::Gradient(settings)) = self.selection_drag.take() else {
            return;
        };
        if length(settings.start, settings.end) < 1.0 {
            self.status = "Drag a line to draw a gradient".into();
            return;
        }
        let colored = settings.paint_target.is_raster() && self.gradient.colors != Colors::Custom;
        if self.dispatch_document_edit(
            crate::document_edits::Edit::Gradient(settings),
            "Gradient applied",
            "Gradient made no changes",
        ) && colored
        {
            self.note_recent_color();
        }
    }

    /// The line being dragged, with its ends marked.
    pub(crate) fn paint_gradient_line(
        &self,
        painter: &crate::egui::Painter,
        view: crate::view::View,
    ) {
        use crate::egui::{Color32, Stroke, pos2};
        let Some(crate::selection_ui::Drag::Gradient(settings)) = &self.selection_drag else {
            return;
        };
        let at = |p: [f64; 2]| view.to_screen(pos2(p[0] as f32, p[1] as f32));
        let (a, b) = (at(settings.start), at(settings.end));
        painter.line_segment([a, b], Stroke::new(3.0_f32, Color32::BLACK));
        painter.line_segment([a, b], Stroke::new(1.0_f32, Color32::WHITE));
        for (point, filled) in [(a, true), (b, false)] {
            painter.circle_filled(point, 5.0, Color32::BLACK);
            painter.circle(
                point,
                3.5,
                if filled {
                    Color32::WHITE
                } else {
                    Color32::BLACK
                },
                Stroke::new(1.5_f32, Color32::WHITE),
            );
        }
    }

    /// Bucket or Gradient, at the top of the Fill tool's options.
    pub(crate) fn fill_mode_ui(&mut self, ui: &mut crate::egui::Ui) {
        let key = self
            .shortcuts
            .label(crate::commands::Action::Tool(crate::tools::Tool::Fill));
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!self.gradient.active, "Bucket")
                .on_hover_text(format!(
                    "Fill an area of similar color · tap {key} to switch"
                ))
                .clicked()
            {
                self.gradient.active = false;
            }
            if ui
                .selectable_label(self.gradient.active, "Gradient")
                .on_hover_text(format!("Drag to paint a gradient · tap {key} to switch"))
                .clicked()
            {
                self.gradient.active = true;
            }
        });
    }

    /// The Gradient mode's options, in the tool panel or the options bar's
    /// menu (which has its own opacity slider beside it).
    pub(crate) fn gradient_options_ui(&mut self, ui: &mut crate::egui::Ui, opacity: bool) {
        use crate::egui;
        let ramp = self
            .gradient
            .ramp(self.paint_color(), self.secondary_paint_color());
        let reverse = self.gradient.reverse;
        crate::effects_panel::gradient_bar(ui, |t| ramp.sample(if reverse { 1.0 - t } else { t }));
        ui.label("Shape");
        ui.horizontal_wrapped(|ui| {
            for style in Style::ALL {
                ui.selectable_value(&mut self.gradient.style, style, style.name());
            }
        });
        ui.label("Colors");
        egui::ComboBox::from_id_salt("gradient colors")
            .selected_text(self.gradient.colors.name())
            .width(ui.available_width().min(240.0))
            .show_ui(ui, |ui| {
                for colors in Colors::ALL {
                    ui.selectable_value(&mut self.gradient.colors, colors, colors.name());
                }
            });
        if self.gradient.colors == Colors::Custom {
            crate::effects_panel::gradient_stops_editor(ui, &mut self.gradient.custom);
        }
        ui.checkbox(&mut self.gradient.reverse, "Reverse");
        if opacity {
            ui.label("Opacity");
            let mut percent = (self.brush.opacity * 100.0).round();
            if ui
                .add(
                    egui::Slider::new(&mut percent, 0.0..=100.0)
                        .suffix("%")
                        .integer(),
                )
                .changed()
            {
                self.brush.opacity = percent / 100.0;
            }
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(if self.input_stroke_target().is_raster() {
                "Drag across the canvas: the gradient starts where you press and ends where \
                 you release. Shift snaps the angle. A selection limits it; Lock \
                 transparency recolors only existing paint."
            } else {
                "Drag across the canvas to fade the mask: the colors' brightness becomes \
                 mask coverage, so black conceals and white reveals. Shift snaps the angle."
            })
            .small()
            .weak(),
        );
    }
}
