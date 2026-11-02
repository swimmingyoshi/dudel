//! The Layer Effects window: add, switch off, reorder, remove and adjust a
//! layer's effects. Changes apply live; a drag becomes one undo step when the
//! pointer is released, like the opacity slider.
use crate::{
    Dudel, egui,
    layer_effects::{BevelStyle, Effect, Kind, LayerEffect, Paint, Position, Technique},
};

/// An effects change being dragged: the list before it, to tell whether it
/// changed anything and whether the project was modified before.
pub(crate) struct Edit {
    target: crate::document::LayerId,
    original: Vec<LayerEffect>,
    modified: bool,
    /// While the render queue is busy the document is not edited directly:
    /// the gesture's latest list waits here and is sent as one edit when the
    /// pointer lifts.
    queued: Option<Vec<LayerEffect>>,
}

impl Dudel {
    /// The Layer Effects window for the selected layer, when open.
    pub(crate) fn effects_window(&mut self, ctx: &egui::Context) {
        if !self.effects_window_open {
            self.finish_effects_edit();
            return;
        }
        let mut open = true;
        // Sized to its contents, so added effects show their settings; long
        // lists scroll.
        egui::Window::new("Layer Effects")
            .open(&mut open)
            .resizable(false)
            .min_width(300.0)
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() * 0.75).max(200.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.effects_ui(ui));
            });
        self.effects_window_open = open;
        // A click or typed value counts as soon as the pointer is up.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.finish_effects_edit();
        }
    }

    fn effects_ui(&mut self, ui: &mut egui::Ui) {
        // Room for each slider's value and label inside the window.
        ui.spacing_mut().slider_width = 130.0;
        let Some(target) = self.input_target() else {
            ui.label("No layer selected.");
            return;
        };
        if !self.render_queue.busy() {
            self.effects_sent = None;
        }
        let Some(layer) = self.doc.layers.iter().find(|l| l.id == target) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Updating layers…");
            });
            return;
        };
        ui.strong(&layer.name);
        // Build on what the user last set, even before the render queue has
        // published it: the open gesture's draft, then the last list sent.
        let original = self
            .effects_edit
            .as_ref()
            .filter(|edit| edit.target == target)
            .and_then(|edit| edit.queued.clone())
            .or_else(|| {
                self.effects_sent
                    .as_ref()
                    .filter(|(sent, _)| *sent == target)
                    .map(|(_, effects)| effects.clone())
            })
            .unwrap_or_else(|| layer.effects.clone());
        let mut effects = original.clone();
        ui.menu_button("Add effect", |ui| {
            for name in crate::layer_effects::PRESETS {
                let needed = if name == "Bevel and Emboss" { 2 } else { 1 };
                if ui
                    .add_enabled(
                        effects.len() + needed <= crate::document::MAX_EFFECTS,
                        egui::Button::new(name),
                    )
                    .clicked()
                {
                    if name == "Bevel and Emboss" {
                        let pair = effects
                            .iter()
                            .filter_map(|e| match e.effect.kind {
                                Kind::Bevel { pair, .. } => Some(pair),
                                _ => None,
                            })
                            .max()
                            .unwrap_or(0)
                            + 1;
                        effects.extend(Effect::bevel_presets(pair).map(LayerEffect::new));
                    } else if let Some(effect) = Effect::preset(name) {
                        effects.push(LayerEffect::new(effect));
                    }
                    ui.close();
                }
            }
        });
        if effects.is_empty() {
            ui.label(
                "No effects. Effects are drawn from the layer's pixels and follow edits to them.",
            );
        }
        // Listed top first, as drawn.
        let mut remove = None;
        let mut swap = None;
        let count = effects.len();
        for index in (0..count).rev() {
            ui.separator();
            let id = ui.id().with(("effect", index));
            let entry = &mut effects[index];
            ui.horizontal(|ui| {
                ui.checkbox(&mut entry.enabled, entry.effect.name());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    use crate::icons::{Icon, button};
                    if button(ui, Icon::Trash, 22.0, false, true)
                        .on_hover_text("Remove effect")
                        .clicked()
                    {
                        remove = Some(index);
                    }
                    if button(ui, Icon::ChevronDown, 22.0, false, index > 0)
                        .on_hover_text("Move down")
                        .clicked()
                    {
                        swap = Some(index - 1);
                    }
                    if button(ui, Icon::ChevronUp, 22.0, false, index + 1 < count)
                        .on_hover_text("Move up")
                        .clicked()
                    {
                        swap = Some(index);
                    }
                });
            });
            ui.push_id(id, |ui| effect_settings(ui, &mut entry.effect));
        }
        if let Some(index) = remove {
            effects.remove(index);
        }
        if let Some(index) = swap {
            effects.swap(index, index + 1);
        }
        sync_bevel_halves(&original, &mut effects);
        if effects != original {
            self.apply_effects(target, original, effects);
        }
    }

    /// Apply a changed effect list: live under one gesture when possible,
    /// otherwise as a queued property edit once the pointer is released.
    pub(crate) fn apply_effects(
        &mut self,
        target: crate::document::LayerId,
        original: Vec<LayerEffect>,
        effects: Vec<LayerEffect>,
    ) {
        if effects.len() > crate::document::MAX_EFFECTS {
            self.status = "Too many layer effects".into();
            return;
        }
        if let Err(error) = effects.iter().try_for_each(LayerEffect::validate) {
            self.status = error;
            return;
        }
        if self.effects_edit.as_ref().is_some_and(|edit| {
            edit.target != target || edit.queued.is_some() != self.render_queue.busy()
        }) {
            self.finish_effects_edit();
        }
        if self.render_queue.busy() {
            match &mut self.effects_edit {
                Some(edit) => edit.queued = Some(effects),
                None => {
                    self.effects_edit = Some(Edit {
                        target,
                        original,
                        modified: self.modified,
                        queued: Some(effects),
                    });
                }
            }
            return;
        }
        let Some(index) = self.doc.layers.iter().position(|l| l.id == target) else {
            return;
        };
        if self.effects_edit.is_none() {
            self.finish_layer_rename(true);
            self.close_input_gestures();
            self.history.begin_gesture_checkpoint(&self.doc);
            self.effects_edit = Some(Edit {
                target,
                original: self.doc.layers[index].effects.clone(),
                modified: self.modified,
                queued: None,
            });
        }
        self.doc.layers[index].effects = effects;
        self.changed();
    }

    pub(crate) fn finish_effects_edit(&mut self) {
        let Some(edit) = self.effects_edit.take() else {
            return;
        };
        if let Some(queued) = edit.queued {
            // One queued edit per gesture, after the pointer lifts.
            if queued != edit.original {
                self.set_canvas_property(
                    edit.target,
                    crate::document_edits::Property::Effects(queued.clone()),
                );
                self.effects_sent = Some((edit.target, queued));
            }
            return;
        }
        let unchanged = self
            .doc
            .layers
            .iter()
            .find(|l| l.id == edit.target)
            .is_none_or(|l| l.effects == edit.original);
        if unchanged {
            self.history.discard_pending_checkpoint(&self.doc);
            self.modified = edit.modified;
        } else {
            self.history.settle(&self.doc);
        }
    }
}

/// Settings shared by a bevel's highlight and shadow follow each other: the
/// other half is the bevel with the same pair number (or, for bevels saved
/// before pairing, an unpaired neighbor).
pub(crate) fn sync_bevel_halves(original: &[LayerEffect], effects: &mut [LayerEffect]) {
    for index in 0..effects.len() {
        let Kind::Bevel {
            highlight,
            style,
            technique,
            depth,
            size,
            soften,
            light,
            up,
            pair,
        } = effects[index].effect.kind
        else {
            continue;
        };
        if original
            .get(index)
            .is_some_and(|o| o.effect.kind == effects[index].effect.kind)
        {
            continue;
        }
        let partner = |other: usize| {
            other != index
                && matches!(
                    effects[other].effect.kind,
                    Kind::Bevel { highlight: h, pair: p, .. } if h != highlight && p == pair
                )
        };
        let other = if pair != 0 {
            (0..effects.len()).find(|&other| partner(other))
        } else {
            [index.wrapping_sub(1), index + 1]
                .into_iter()
                .find(|&other| other < effects.len() && partner(other))
        };
        if let Some(other) = other {
            effects[other].effect.kind = Kind::Bevel {
                highlight: !highlight,
                style,
                technique,
                depth,
                size,
                soften,
                light,
                up,
                pair,
            };
        }
        return;
    }
}

fn pixels(ui: &mut egui::Ui, label: &str, value: &mut f64, max: f64) {
    crate::workspace::labeled_slider(
        ui,
        label,
        egui::Slider::new(value, 0.0..=max)
            .suffix(" px")
            .max_decimals(1),
    );
}
fn percent(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    let mut p = *value * 100.0;
    if crate::workspace::labeled_slider(
        ui,
        label,
        egui::Slider::new(&mut p, 0.0..=100.0).suffix("%").integer(),
    )
    .changed()
    {
        *value = p / 100.0;
    }
}
fn angle(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    crate::workspace::labeled_slider(
        ui,
        label,
        egui::Slider::new(value, -180.0..=180.0)
            .suffix("°")
            .integer(),
    );
}

/// Controls for one effect's settings.
fn effect_settings(ui: &mut egui::Ui, effect: &mut Effect) {
    ui.horizontal(|ui| {
        effect.blend = crate::layers_panel::blend_selector(ui, effect.blend, false);
        let mut opacity = f64::from(effect.opacity);
        percent(ui, "Opacity", &mut opacity);
        effect.opacity = opacity as f32;
    });
    match &mut effect.paint {
        Paint::Color(color) => {
            ui.horizontal(|ui| {
                ui.label("Color");
                egui::color_picker::color_edit_button_srgb(ui, color);
            });
        }
        Paint::Gradient(gradient) => {
            ui.horizontal(|ui| {
                ui.label("Gradient");
                let [mut start, mut end] = gradient.ends();
                let a = egui::color_picker::color_edit_button_srgb(ui, &mut start).changed();
                let b = egui::color_picker::color_edit_button_srgb(ui, &mut end).changed();
                if a || b {
                    gradient.set_ends([start, end]);
                }
            });
            let mut value = gradient.angle();
            angle(ui, "Angle", &mut value);
            gradient.set_angle(value);
        }
    }
    match &mut effect.kind {
        Kind::DropShadow {
            offset,
            spread,
            size,
            knockout,
        } => {
            shadow_offset(ui, offset);
            percent(ui, "Spread", spread);
            pixels(ui, "Size", size, 250.0);
            ui.checkbox(knockout, "Layer knocks out shadow");
        }
        Kind::InnerShadow {
            offset,
            choke,
            size,
        } => {
            shadow_offset(ui, offset);
            percent(ui, "Choke", choke);
            pixels(ui, "Size", size, 250.0);
        }
        Kind::OuterGlow {
            spread,
            size,
            precise,
            range,
        } => {
            percent(ui, "Spread", spread);
            pixels(ui, "Size", size, 250.0);
            percent(ui, "Range", range);
            *range = range.max(0.01);
            ui.checkbox(precise, "Precise");
        }
        Kind::InnerGlow {
            choke,
            size,
            center,
            precise,
            range,
        } => {
            percent(ui, "Choke", choke);
            pixels(ui, "Size", size, 250.0);
            percent(ui, "Range", range);
            *range = range.max(0.01);
            ui.horizontal(|ui| {
                ui.radio_value(center, false, "Edge");
                ui.radio_value(center, true, "Center");
                ui.checkbox(precise, "Precise");
            });
        }
        Kind::ColorOverlay | Kind::GradientOverlay => {}
        Kind::Stroke { size, position } => {
            pixels(ui, "Size", size, 250.0);
            ui.horizontal(|ui| {
                ui.radio_value(position, Position::Outside, "Outside");
                ui.radio_value(position, Position::Inside, "Inside");
                ui.radio_value(position, Position::Center, "Center");
            });
        }
        Kind::Bevel {
            style,
            technique,
            depth,
            size,
            soften,
            light,
            up,
            ..
        } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("style")
                    .selected_text(match style {
                        BevelStyle::Inner => "Inner bevel",
                        BevelStyle::Outer => "Outer bevel",
                        BevelStyle::Emboss => "Emboss",
                        BevelStyle::Pillow => "Pillow emboss",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(style, BevelStyle::Inner, "Inner bevel");
                        ui.selectable_value(style, BevelStyle::Outer, "Outer bevel");
                        ui.selectable_value(style, BevelStyle::Emboss, "Emboss");
                        ui.selectable_value(style, BevelStyle::Pillow, "Pillow emboss");
                    });
                egui::ComboBox::from_id_salt("technique")
                    .selected_text(match technique {
                        Technique::Smooth => "Smooth",
                        Technique::ChiselHard => "Chisel hard",
                        Technique::ChiselSoft => "Chisel soft",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(technique, Technique::Smooth, "Smooth");
                        ui.selectable_value(technique, Technique::ChiselHard, "Chisel hard");
                        ui.selectable_value(technique, Technique::ChiselSoft, "Chisel soft");
                    });
            });
            let mut d = *depth * 100.0;
            if crate::workspace::labeled_slider(
                ui,
                "Depth",
                egui::Slider::new(&mut d, 1.0..=1000.0)
                    .suffix("%")
                    .integer(),
            )
            .changed()
            {
                *depth = d / 100.0;
            }
            ui.horizontal(|ui| {
                ui.radio_value(up, true, "Up");
                ui.radio_value(up, false, "Down");
            });
            pixels(ui, "Size", size, 250.0);
            pixels(ui, "Soften", soften, 16.0);
            let mut a = (-light[1]).atan2(light[0]).to_degrees();
            let mut altitude = light[2].clamp(-1.0, 1.0).asin().to_degrees();
            let changed = crate::workspace::labeled_slider(
                ui,
                "Angle",
                egui::Slider::new(&mut a, -180.0..=180.0)
                    .suffix("°")
                    .integer(),
            )
            .changed()
                | crate::workspace::labeled_slider(
                    ui,
                    "Altitude",
                    egui::Slider::new(&mut altitude, 0.0..=90.0)
                        .suffix("°")
                        .integer(),
                )
                .changed();
            if changed {
                *light = crate::layer_effects::light(a, altitude);
            }
        }
    }
}

/// Angle and distance controls for a shadow's offset.
fn shadow_offset(ui: &mut egui::Ui, offset: &mut [f64; 2]) {
    let distance = offset[0].hypot(offset[1]);
    let mut a = if distance > 0.0 {
        offset[1].atan2(-offset[0]).to_degrees()
    } else {
        120.0
    };
    let mut d = distance;
    let changed = crate::workspace::labeled_slider(
        ui,
        "Angle",
        egui::Slider::new(&mut a, -180.0..=180.0)
            .suffix("°")
            .integer(),
    )
    .changed()
        | crate::workspace::labeled_slider(
            ui,
            "Distance",
            egui::Slider::new(&mut d, 0.0..=500.0)
                .suffix(" px")
                .max_decimals(1),
        )
        .changed();
    if changed {
        *offset = crate::layer_effects::offset(a, d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bevel_halves_follow_their_own_pair_only() {
        let size = |e: &LayerEffect| match e.effect.kind {
            Kind::Bevel { size, .. } => size,
            _ => unreachable!(),
        };
        // Two bevels, with another effect moved between the first's halves.
        let [shadow, highlight] = Effect::bevel_presets(1).map(LayerEffect::new);
        let mut effects = vec![
            shadow,
            LayerEffect::new(Effect::preset("Stroke").unwrap()),
            highlight,
        ];
        effects.extend(Effect::bevel_presets(2).map(LayerEffect::new));
        let original = effects.clone();
        if let Kind::Bevel { size, .. } = &mut effects[2].effect.kind {
            *size = 12.0;
        }
        sync_bevel_halves(&original, &mut effects);
        assert_eq!(size(&effects[0]), 12.0, "its own shadow follows");
        assert_eq!(
            (size(&effects[3]), size(&effects[4])),
            (5.0, 5.0),
            "the other bevel stays"
        );
    }

    #[test]
    fn busy_queue_sends_one_edit_per_gesture_and_builds_on_it() {
        let mut app = Dudel {
            doc: crate::document::Document::new(4, 4),
            ..Default::default()
        };
        let target = app.doc.layers[0].id;
        app.render_queue.redo().unwrap();
        assert!(app.render_queue.busy());
        let overlay = |opacity: f32| {
            let mut effect = Effect::preset("Color Overlay").unwrap();
            effect.opacity = opacity;
            LayerEffect::new(effect)
        };
        // A drag through three values while rendering is busy.
        for opacity in [0.2, 0.4, 0.6] {
            app.apply_effects(target, Vec::new(), vec![overlay(opacity)]);
        }
        assert!(
            app.doc.layers[0].effects.is_empty(),
            "nothing applied mid-drag"
        );
        app.finish_effects_edit();
        // The next change starts from what was sent, not the stale document.
        assert_eq!(app.effects_sent, Some((target, vec![overlay(0.6)])));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].effects, vec![overlay(0.6)]);
        // One undo step for the whole drag.
        app.undo();
        app.flush_render_queue();
        assert!(app.doc.layers[0].effects.is_empty());
    }

    #[test]
    fn effects_window_rejects_over_limit_edits_before_mutating_history() {
        let mut app = Dudel {
            doc: crate::document::Document::new(1, 1),
            ..Default::default()
        };
        let target = app.doc.layers[0].id;
        let effect = LayerEffect::new(Effect::preset("Color Overlay").unwrap());
        app.doc.layers[0].effects = vec![effect.clone(); crate::document::MAX_EFFECTS];
        app.apply_effects(
            target,
            app.doc.layers[0].effects.clone(),
            vec![effect; crate::document::MAX_EFFECTS + 1],
        );
        app.finish_effects_edit();
        assert!(
            app.doc.validate().is_ok(),
            "the effects window made an unsavable document"
        );
        assert_eq!(
            app.doc.layers[0].effects.len(),
            crate::document::MAX_EFFECTS
        );
        assert!(!app.history.can_undo());
        assert!(!app.modified);
    }
}
