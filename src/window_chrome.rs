//! Compact client-side window chrome. Movement and sizing stay with the OS /
//! compositor, while every close entry point uses the document save workflow.
use crate::{Dudel, document_actions::Action, egui};

fn control(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::vec2(38.0, 28.0));
    let response = ui.interact(
        rect,
        egui::Id::new(("window-control", name)),
        egui::Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    let close_hover = name == "Close" && response.hovered();
    let visuals = ui.style().interact(&response);
    if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
        ui.painter().rect_filled(
            rect,
            3.0,
            if close_hover {
                egui::Color32::from_rgb(190, 47, 57)
            } else {
                visuals.bg_fill
            },
        );
    }
    let stroke = egui::Stroke::new(
        1.3_f32,
        if close_hover {
            egui::Color32::WHITE
        } else {
            visuals.fg_stroke.color
        },
    );
    let icon = egui::Rect::from_center_size(rect.center(), egui::vec2(10.0, 10.0));
    match name {
        "Close" => {
            ui.painter()
                .line_segment([icon.left_top(), icon.right_bottom()], stroke);
            ui.painter()
                .line_segment([icon.right_top(), icon.left_bottom()], stroke);
        }
        "Minimize" => {
            ui.painter()
                .hline(icon.x_range(), icon.center().y + 3.0, stroke);
        }
        "Restore" => {
            let back = icon.translate(egui::vec2(2.0, -2.0)).shrink(1.0);
            ui.painter()
                .line_segment([back.left_top(), back.right_top()], stroke);
            ui.painter()
                .line_segment([back.right_top(), back.right_bottom()], stroke);
            ui.painter().rect_stroke(
                icon.translate(egui::vec2(-1.0, 1.0)).shrink(1.0),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        _ => {
            ui.painter()
                .rect_stroke(icon, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }
    response.on_hover_text(name)
}

impl Dudel {
    pub(crate) fn window_title_ui(&mut self, ui: &mut egui::Ui) {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 28.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if control(ui, "Close").clicked() {
                    self.request_replace(Action::Close);
                }
                ui.add_enabled_ui(!fullscreen, |ui| {
                    if control(ui, if maximized { "Restore" } else { "Maximize" }).clicked() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                });
                if control(ui, "Minimize").clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.add_space(10.0);
                let (_, title_rect) = ui.allocate_space(egui::vec2(ui.available_width(), 28.0));
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(title_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        self.document_status_ui(ui);
                    },
                );
                let drag = ui.interact(
                    title_rect,
                    egui::Id::new("window-title-drag"),
                    egui::Sense::CLICK | egui::Sense::DRAG,
                );
                if !fullscreen {
                    if drag.double_clicked() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    } else if drag.drag_started_by(egui::PointerButton::Primary) {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
            },
        );
    }

    pub(crate) fn window_resize_ui(&self, ctx: &egui::Context) {
        if ctx.input(|i| {
            i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
        }) {
            return;
        }
        let rect = ctx.content_rect();
        let (w, h) = (rect.width(), rect.height());
        use egui::{CursorIcon as C, ResizeDirection as D};
        // Thin edges leave sidebar splitters and canvas controls accessible;
        // larger corner targets make diagonal sizing easier.
        for (direction, cursor, x, y, width, height) in [
            (D::NorthWest, C::ResizeNwSe, 0.0, 0.0, 10.0, 10.0),
            (D::NorthEast, C::ResizeNeSw, w - 10.0, 0.0, 10.0, 10.0),
            (D::SouthWest, C::ResizeNeSw, 0.0, h - 10.0, 10.0, 10.0),
            (D::SouthEast, C::ResizeNwSe, w - 10.0, h - 10.0, 10.0, 10.0),
            (D::North, C::ResizeVertical, 10.0, 0.0, w - 20.0, 4.0),
            (D::South, C::ResizeVertical, 10.0, h - 4.0, w - 20.0, 4.0),
            (D::West, C::ResizeHorizontal, 0.0, 10.0, 4.0, h - 20.0),
            (D::East, C::ResizeHorizontal, w - 4.0, 10.0, 4.0, h - 20.0),
        ] {
            egui::Area::new(egui::Id::new(("window-edge", direction as u8)))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min + egui::vec2(x, y))
                .movable(false)
                .sense(egui::Sense::hover())
                .constrain(false)
                .show(ctx, |ui| {
                    let (_, response) = ui.allocate_exact_size(
                        egui::vec2(width, height),
                        egui::Sense::CLICK | egui::Sense::DRAG,
                    );
                    let response = response.on_hover_cursor(cursor);
                    if response.is_pointer_button_down_on()
                        && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                    }
                });
        }
    }
}
