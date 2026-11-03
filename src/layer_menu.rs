//! The Layers panel's empty space: a click there opens a menu of new
//! layers, so adding one never needs the Layer menu.
use crate::{Dudel, egui};

impl Dudel {
    /// Fill the rest of the layer list with a clickable space that opens
    /// the new-layer menu (left or right click).
    pub(crate) fn layer_space_ui(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height().max(36.0);
        let (_, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::click(),
        );
        let response = response.on_hover_text("Click for new layers");
        egui::Popup::menu(&response)
            .open_memory(
                (response.clicked() || response.secondary_clicked())
                    .then_some(egui::SetOpenCommand::Toggle),
            )
            .show(|ui| self.new_layer_menu(ui));
    }

    /// New layers of every kind; each choice closes the menu.
    pub(crate) fn new_layer_menu(&mut self, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool| {
            ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
                .clicked()
        };
        let room = self.doc.has_room(1, 1);
        if item(ui, "New layer", "Ctrl+Shift+N", room) {
            self.new_layer();
            ui.close();
        }
        if item(ui, "New vector layer", "", room) {
            self.create_vector_layer();
            ui.close();
        }
        if item(ui, "New folder", "", self.doc.has_room(0, 1)) {
            self.new_folder();
            ui.close();
        }
        ui.add_enabled_ui(room, |ui| {
            ui.menu_button("New adjustment layer", |ui| {
                for settings in crate::adjustment::presets() {
                    if ui.button(settings.name()).clicked() {
                        self.create_adjustment_layer(settings);
                        self.adjustment_window_open = true;
                        ui.close();
                    }
                }
            });
        });
        ui.separator();
        if item(
            ui,
            "Add reference image…",
            "",
            self.reference_layers.layers.len() < crate::reference_layers::MAX_REFERENCES,
        ) {
            self.add_reference_layer();
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_below_the_layers_opens_the_new_layer_menu() {
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let run = |app: &mut Dudel, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.layer_space_ui(ui));
            });
        };
        let below = egui::pos2(200.0, 560.0);
        let click = |pressed| egui::Event::PointerButton {
            pos: below,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut app, vec![egui::Event::PointerMoved(below)]);
        assert!(!egui::Popup::is_any_open(&ctx));
        run(&mut app, vec![click(true)]);
        run(&mut app, vec![click(false)]);
        run(&mut app, vec![]);
        assert!(egui::Popup::is_any_open(&ctx));
        let layers = app.doc.layers.len();
        assert_eq!(layers, 1);
    }
}
