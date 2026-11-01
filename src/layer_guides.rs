//! Scope-aware controls shared by layer badges, layer menus and stroke options.
use crate::{
    Dudel, egui,
    symmetry::{Mode, Symmetry},
};

/// Width of the label column in the symmetry and guide editors.
const LABEL_WIDTH: f32 = 64.0;

/// A small capitalized heading, like the side panels' section titles.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .size(11.0)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
}

/// Undo the menu style toolbar popups apply (frameless, tightly padded
/// buttons), so these controls look the same in a menu as in the layer popup.
fn plain_buttons(ui: &mut egui::Ui) {
    let global = ui.ctx().style();
    let style = ui.style_mut();
    style.visuals.widgets = global.visuals.widgets.clone();
    style.visuals.button_frame = global.visuals.button_frame;
    style.spacing.button_padding = global.spacing.button_padding;
}

/// A row with a fixed-width label so controls line up.
fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_WIDTH, ui.spacing().interact_size.y],
            egui::Label::new(label).truncate(),
        );
        add(ui)
    })
    .inner
}

fn editor(ui: &mut egui::Ui, symmetry: &mut Symmetry) {
    // Equal cells in rows of three keep the choices in a steady grid at any
    // popup width instead of wrapping wherever a name runs out of room.
    let spacing = ui.spacing().item_spacing.x;
    let cell = ((ui.available_width() - 2.0 * spacing) / 3.0).floor();
    for modes in Mode::ALL.chunks(3) {
        ui.horizontal(|ui| {
            for &mode in modes {
                if ui
                    .add_sized(
                        [cell, ui.spacing().interact_size.y],
                        egui::Button::selectable(symmetry.mode == mode, mode.name()),
                    )
                    .clicked()
                {
                    symmetry.mode = mode;
                }
            }
        });
    }
    if symmetry.mode == Mode::Radial {
        row(ui, "Copies", |ui| {
            ui.add(egui::Slider::new(&mut symmetry.segments, 2..=16));
        });
    }
    if symmetry.active() {
        row(ui, "Rotation", |ui| {
            ui.add(egui::Slider::new(&mut symmetry.angle, -180.0..=180.0).suffix("°"));
        });
        row(ui, "", |ui| {
            for (label, angle) in [("0°", 0.0), ("45°", 45.0), ("90°", 90.0)] {
                if ui
                    .selectable_label(symmetry.angle == angle, label)
                    .clicked()
                {
                    symmetry.angle = angle;
                }
            }
            if ui
                .add_enabled(symmetry.center.is_some(), egui::Button::new("Center"))
                .on_hover_text("Move the symmetry center back to the middle of the canvas")
                .clicked()
            {
                symmetry.center = None;
            }
        });
    }
}

impl Dudel {
    pub(crate) fn global_symmetry_ui(&mut self, ui: &mut egui::Ui) {
        plain_buttons(ui);
        ui.set_max_width(ui.available_width().min(290.0));
        section(ui, "Global symmetry");
        ui.weak("Used by layers without an override");
        editor(ui, &mut self.symmetry);
        ui.checkbox(&mut self.guides_locked, "Lock guide handles");
    }

    pub(crate) fn layer_guides_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        let Some(layer) = self.doc.layers.get(index) else {
            return;
        };
        let id = layer.id;
        plain_buttons(ui);
        ui.set_max_width(ui.available_width().min(290.0));
        ui.add(egui::Label::new(egui::RichText::new(&layer.name).strong()).truncate());

        section(ui, "Symmetry");
        let mut own = self.layer_symmetry.contains_key(&id);
        if ui.checkbox(&mut own, "Layer symmetry override").changed() {
            if own {
                self.layer_symmetry.insert(id, self.symmetry);
            } else {
                self.layer_symmetry.remove(&id);
            }
        }
        if own {
            editor(ui, self.layer_symmetry.get_mut(&id).unwrap());
            ui.horizontal_wrapped(|ui| {
                if ui
                    .small_button("Copy global")
                    .on_hover_text("Replace this layer's settings with the global ones")
                    .clicked()
                {
                    self.layer_symmetry.insert(id, self.symmetry);
                }
                if ui
                    .small_button("Use globally")
                    .on_hover_text("Make these the global settings and remove the override")
                    .clicked()
                {
                    self.symmetry = self.layer_symmetry.remove(&id).unwrap();
                }
            });
            ui.menu_button("Global symmetry settings", |ui| self.global_symmetry_ui(ui));
        } else {
            ui.weak(format!("Using global: {}", self.symmetry.mode.name()));
            editor(ui, &mut self.symmetry);
            if self.symmetry.active() && ui.button("Disable symmetry on this layer").clicked() {
                self.layer_symmetry.insert(id, Symmetry::default());
            }
        }

        section(ui, "Guides");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.snapping, "Snap strokes to guides");
            ui.checkbox(&mut self.guides_locked, "Lock guides");
        });
        let mut remove = None;
        self.ruler_owners.resize(self.rulers.len(), None);
        for ruler in 0..self.rulers.len() {
            if self.ruler_owner(ruler).is_some_and(|owner| owner != id) {
                continue;
            }
            ui.push_id(ruler, |ui| {
                row(ui, self.rulers[ruler].kind().name(), |ui| {
                    let mut owned = self.ruler_owner(ruler) == Some(id);
                    if ui
                        .checkbox(&mut owned, "Layer only")
                        .on_hover_text("Only strokes on this layer snap to this guide")
                        .changed()
                    {
                        self.ruler_owners[ruler] = owned.then_some(id);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(ruler);
                        }
                    });
                });
            });
        }
        if let Some(ruler) = remove {
            self.remove_ruler(ruler);
        }
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("Add layer guide", |ui| {
                for kind in crate::rulers::Kind::ALL {
                    if ui
                        .add_enabled(
                            self.rulers.len() < crate::rulers::MAX_RULERS,
                            egui::Button::new(kind.name()),
                        )
                        .clicked()
                    {
                        let center = [self.doc.width as f32 * 0.5, self.doc.height as f32 * 0.5];
                        let mut ruler = crate::rulers::Ruler::new(kind, center);
                        ruler.drag_out(
                            center,
                            [
                                center[0] + self.doc.width.max(32) as f32 * 0.25,
                                center[1] + self.doc.height.max(32) as f32 * 0.25,
                            ],
                        );
                        self.push_ruler(ruler);
                        *self.ruler_owners.last_mut().unwrap() = Some(id);
                        self.snapping = true;
                        self.select_layer_target(index);
                        self.select_tool(crate::tools::Tool::Pointer);
                        self.ruler_create = None;
                        self.guides_locked = false;
                        ui.close();
                    }
                }
            });
            if ui
                .button("Move handles")
                .on_hover_text(
                    "Select this layer and switch to Pointer; drag the symmetry center or ruler handles on the canvas",
                )
                .clicked()
            {
                self.select_layer_target(index);
                self.select_tool(crate::tools::Tool::Pointer);
                self.ruler_create = None;
                self.guides_locked = false;
                ui.close();
            }
        });
        ui.menu_button("Global grid and guide lines", |ui| self.guides_ui(ui));
    }

    pub(crate) fn layer_guide_controls(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 4.0;
        crate::icons::mark(
            ui,
            crate::icons::Icon::Global,
            16.0,
            if self.symmetry.active() {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().weak_text_color()
            },
        );
        let button = egui::Button::new("Global").selected(self.symmetry.active());
        let response = ui
            .push_id("global symmetry button", |ui| ui.add(button))
            .inner;
        egui::Popup::from_toggle_button_response(&response)
            .id(egui::Id::new("global symmetry settings popup"))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(280.0);
                self.global_symmetry_ui(ui);
            });
        response.on_hover_text(format!(
            "Global symmetry: {} · layer overrides stay independent",
            self.symmetry.mode.name()
        ));
        if let Some(index) = self.input_layer() {
            let global = self.guide_lines.len()
                + (0..self.rulers.len())
                    .filter(|&index| self.ruler_owner(index).is_none())
                    .count()
                + usize::from(self.grid.visible);
            let label = if global == 0 {
                "Guides".into()
            } else {
                format!("Guides: {global}")
            };
            let response = ui
                .push_id("layer guides button", |ui| ui.button(label))
                .inner;
            self.open_capture_popup_id(
                ui.ctx(),
                egui::Id::new("layer guides settings popup"),
                crate::capture::Popup::Guides,
            );
            egui::Popup::from_toggle_button_response(&response)
                .id(egui::Id::new("layer guides settings popup"))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.layer_guides_popup(ui, index));
            response.on_hover_text("Symmetry and guides for the selected layer; the count includes global guides and grid");
        }
    }

    pub(crate) fn layer_guides_popup(&mut self, ui: &mut egui::Ui, index: usize) {
        ui.set_width(290.0);
        ui.spacing_mut().interact_size.y = 26.0;
        ui.spacing_mut().button_padding.y = 3.0;
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        let height = (ui.ctx().content_rect().height() - 40.0).clamp(120.0, 560.0);
        let key = ui.id().with("guide popup last frame");
        let frame = ui.ctx().cumulative_frame_nr();
        let previous = ui.data_mut(|data| {
            let previous = data.get_temp::<u64>(key);
            data.insert_temp(key, frame);
            previous
        });
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("layer guide settings")
            .max_height(height);
        if previous.is_none_or(|previous| previous + 1 < frame) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| self.layer_guides_ui(ui, index));
    }
}
