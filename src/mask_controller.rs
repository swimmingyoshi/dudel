//! Captured mask focus and metadata projection at the application boundary.
use crate::{
    Dudel,
    document::LayerId,
    layer_mask_edits::{Command, Seed},
    paint_target::Target,
};

const ARTWORK_REQUIRED: &str = "This action needs artwork. Select the layer image thumbnail first";

impl Dudel {
    pub(crate) fn paint_color(&self) -> [u8; 3] {
        if self.input_stroke_target().is_raster() {
            self.color
        } else {
            [self.mask_colors[0]; 3]
        }
    }
    pub(crate) fn swap_paint_colors(&mut self) {
        if self.input_stroke_target().is_raster() {
            std::mem::swap(&mut self.color, &mut self.secondary);
        } else {
            self.mask_colors.swap(0, 1);
        }
    }
    pub(crate) fn paint_color_chips(&mut self, ui: &mut crate::egui::Ui, side: f32) {
        if self.input_stroke_target().is_raster() {
            crate::workspace::color_chips(ui, &mut self.color, &mut self.secondary, side);
        } else {
            let mut main = [self.mask_colors[0]; 3];
            let mut secondary = [self.mask_colors[1]; 3];
            crate::workspace::color_chips(ui, &mut main, &mut secondary, side);
            self.mask_colors = [main[0], secondary[0]];
        }
    }
    pub(crate) fn mask_palette_ui(&mut self, ui: &mut crate::egui::Ui) {
        ui.label("Mask coverage");
        if self
            .input_properties()
            .and_then(|p| p.mask)
            .is_some_and(|m| !m.enabled)
        {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Mask disabled: coverage edits are stored; enable the mask to see them on artwork.",
            );
        }
        ui.horizontal(|ui| {
            self.paint_color_chips(ui, 40.0);
            if ui.button("Conceal").clicked() {
                self.mask_colors[0] = 0;
            }
            if ui.button("Reveal").clicked() {
                self.mask_colors[0] = 255;
            }
        });
        let mut percent = f32::from(self.mask_colors[0]) * 100.0 / 255.0;
        if ui
            .add(
                crate::egui::Slider::new(&mut percent, 0.0..=100.0)
                    .text("Reveal amount")
                    .suffix("%"),
            )
            .changed()
        {
            self.mask_colors[0] = (percent * 255.0 / 100.0).round() as u8;
        }
        ui.label("Black conceals; white reveals.");
        ui.label("The eraser always reveals artwork.");
    }
    pub(crate) fn mask_controls_available(&self) -> bool {
        true
    }
    pub(crate) fn input_stroke_target(&self) -> Target {
        match (self.mask_focus, self.input_target()) {
            (Some((layer, mask)), Some(active)) if layer == active => Target::Mask(mask),
            _ => Target::Raster,
        }
    }
    pub(crate) fn stroke_target_blocker(
        &self,
        target: Target,
        erase: bool,
    ) -> Option<&'static str> {
        let Some(layer) = self.input_target() else {
            return Some("Selected layer no longer exists");
        };
        if let Some(context) = &self.pending_layers {
            context.target_blocker(layer, target, erase)
        } else if let Some(index) = self.input_layer() {
            target.blocker(&self.doc, index, erase)
        } else {
            Some("Selected layer no longer exists")
        }
    }
    pub(crate) fn require_artwork_target(&mut self) -> bool {
        if self.input_stroke_target().is_raster() {
            true
        } else {
            self.status = ARTWORK_REQUIRED.into();
            false
        }
    }
    pub(crate) fn canvas_paint_blocker(&self) -> Option<&'static str> {
        self.stroke_target_blocker(
            self.input_stroke_target(),
            self.tool == crate::tools::Tool::Eraser,
        )
    }
    pub(crate) fn select_paint_target(&mut self, index: usize, mask: Option<LayerId>) -> bool {
        let Some(layer) = self.doc.layers.get(index).map(|l| l.id) else {
            return false;
        };
        let properties = self.pending_layers.as_ref().map_or_else(
            || {
                self.doc
                    .layers
                    .get(index)
                    .map(crate::pending_layers::Properties::from_layer)
            },
            |context| context.properties(layer),
        );
        let target = if let Some(expected) = mask {
            let Some(slot) = properties.and_then(|p| p.mask).filter(|m| m.id == expected) else {
                self.status = "The displayed mask was removed or replaced".into();
                return false;
            };
            Some((layer, slot.id))
        } else {
            None
        };
        if !self.select_layer_target(index) {
            return false;
        }
        if self.mask_focus != target {
            self.last_stroke_end = None;
        }
        self.mask_focus = target;
        self.status = if mask.is_some() {
            "Editing mask: black conceals, white reveals"
        } else {
            "Editing layer artwork"
        }
        .into();
        true
    }
    pub(crate) fn focus_current_mask(&mut self, mask: bool) {
        let Some(layer) = self.input_target() else {
            return;
        };
        let target = if mask {
            let Some(slot) = self.input_properties().and_then(|p| p.mask) else {
                return;
            };
            Some((layer, slot.id))
        } else {
            None
        };
        self.close_input_gestures();
        if self.mask_focus != target {
            self.last_stroke_end = None;
        }
        self.mask_focus = target;
        self.status = if mask {
            "Editing mask: black conceals, white reveals"
        } else {
            "Editing layer artwork"
        }
        .into();
    }
    /// The Artwork/Mask target switch and mask actions. Shown only when the
    /// selected layer has a mask; masks are added from the layer toolbar or
    /// the Layer menu.
    pub(crate) fn mask_controls_ui(&mut self, ui: &mut crate::egui::Ui) {
        let Some(properties) = self.input_properties() else {
            return;
        };
        if properties.mask.is_none() {
            return;
        }
        let narrow = ui.available_width() < 300.0;
        ui.horizontal(|ui| {
            if narrow {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.spacing_mut().button_padding.x = 6.0;
            } else {
                ui.label("Target");
            }
            if ui
                .selectable_label(self.input_stroke_target().is_raster(), "Artwork")
                .clicked()
            {
                self.focus_current_mask(false);
            }
            if ui
                .add(
                    crate::egui::Button::new("Mask")
                        .selected(!self.input_stroke_target().is_raster()),
                )
                .on_hover_text("Black conceals, white reveals; erasing reveals the artwork")
                .clicked()
            {
                self.focus_current_mask(true);
            }
            ui.menu_button("Mask actions", |ui| self.mask_menu_items(ui));
        });
    }
    /// Mask commands for the selected layer: add one when it has none,
    /// otherwise view, enable, link, invert, apply or remove it.
    pub(crate) fn mask_menu_items(&mut self, ui: &mut crate::egui::Ui) {
        let Some(properties) = self.input_properties() else {
            ui.label("Select a layer first");
            return;
        };
        if let Some(mask) = properties.mask {
            let mut viewing = self.mask_view && !self.input_stroke_target().is_raster();
            if ui
                .checkbox(&mut viewing, "Show mask on canvas")
                .on_hover_text("White reveals, black conceals · also Alt+click the mask thumbnail")
                .changed()
            {
                if viewing {
                    self.focus_current_mask(true);
                }
                self.mask_view = viewing;
                self.dirty = true;
            }
            ui.separator();
            let mut enabled = mask.enabled;
            let mut linked = mask.linked;
            ui.add_enabled_ui(!properties.locked, |ui| {
                if ui.checkbox(&mut enabled, "Enable mask").changed() {
                    self.edit_canvas_mask(Command::Enabled {
                        id: mask.id,
                        value: enabled,
                    });
                }
                if ui.checkbox(&mut linked, "Link to artwork").changed() {
                    self.edit_canvas_mask(Command::Linked {
                        id: mask.id,
                        value: linked,
                    });
                }
                ui.separator();
                if ui.button("Invert mask").clicked() {
                    self.edit_canvas_mask(Command::Invert(mask.id));
                    ui.close();
                }
                let can_apply =
                    !self.input_is_folder() && (!mask.enabled || !properties.alpha_locked);
                if ui
                    .add_enabled(can_apply, crate::egui::Button::new("Apply mask to artwork"))
                    .clicked()
                {
                    self.edit_canvas_mask(Command::Apply(mask.id));
                    ui.close();
                }
                if ui.button("Remove mask").clicked() {
                    self.edit_canvas_mask(Command::Remove(mask.id));
                    ui.close();
                }
            });
        } else {
            ui.add_enabled_ui(!properties.locked, |ui| {
                for (label, seed) in [
                    ("Add mask showing all", Seed::RevealAll),
                    ("Add mask hiding all", Seed::HideAll),
                    ("Add mask from selection", Seed::Selection),
                ] {
                    let enabled = !matches!(seed, Seed::Selection)
                        || self.doc.selection.is_some()
                        || self.doc.selection_mask.is_some();
                    if ui
                        .add_enabled(enabled, crate::egui::Button::new(label))
                        .clicked()
                    {
                        self.create_canvas_mask(seed);
                        ui.close();
                    }
                }
            });
        }
    }
    /// Add mask, in one click: the selection when there is one, else a mask
    /// that shows everything.
    pub(crate) fn add_mask(&mut self) -> bool {
        let selected = self.doc.selection.is_some() || self.doc.selection_mask.is_some();
        self.create_canvas_mask(if selected {
            Seed::Selection
        } else {
            Seed::RevealAll
        })
    }
    pub(crate) fn create_canvas_mask(&mut self, seed: Seed) -> bool {
        self.edit_canvas_mask(Command::Create {
            id: LayerId::new(),
            seed,
        })
    }
    pub(crate) fn edit_canvas_mask(&mut self, command: Command) -> bool {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(target) = self.input_target() else {
            return false;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_mask_command(target, command) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        if !self.dispatch_document_edit(
            crate::document_edits::Edit::LayerMask { target, command },
            match command {
                Command::Fill { .. } => "Mask filled",
                Command::Clear { .. } => "Mask cleared to reveal artwork",
                _ => "Mask updated",
            },
            "Mask unchanged",
        ) {
            return false;
        }
        if self.render_queue.busy() {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(target);
        }
        match command {
            Command::Create { id, .. } => {
                self.mask_focus = Some((target, id));
                self.last_stroke_end = None;
            }
            Command::Remove(_) | Command::Apply(_) => {
                self.mask_focus = None;
                self.last_stroke_end = None;
            }
            _ => {}
        }
        true
    }
    pub(crate) fn reconcile_mask_focus(&mut self) {
        if let Some((layer, mask)) = self.mask_focus
            && !self.render_queue.busy()
            && !self
                .doc
                .layers
                .iter()
                .any(|l| l.id == layer && l.mask.as_ref().is_some_and(|m| m.id == mask))
        {
            self.mask_focus = None;
            self.last_stroke_end = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, stroke::Sample, stroke_buffer::Settings};

    fn app() -> Dudel {
        Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        }
    }
    fn settings(app: &Dudel) -> Settings {
        Settings {
            target: app.input_stroke_target(),
            layer: app.input_layer().unwrap_or(0),
            brush: crate::brush::Brush {
                size: 6.0,
                hardness: 1.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [0; 3],
            erase: false,
            symmetry: Default::default(),
        }
    }
    fn paint(app: &mut Dudel, at: f32) {
        assert!(app.begin_canvas_stroke(settings(app)));
        app.deliver_canvas_dabs(vec![Sample {
            x: at,
            y: 16.5,
            pressure: 1.0,
            ..Default::default()
        }]);
    }
    #[test]
    fn add_mask_shows_everything_or_only_the_selection() {
        let mut whole = app();
        assert!(whole.add_mask());
        whole.flush_render_queue();
        let coverage = &whole.doc.layers[0].mask.as_ref().unwrap().coverage;
        assert_eq!([coverage.at(0, 0), coverage.at(31, 31)], [255, 255]);
        assert!(!whole.input_stroke_target().is_raster(), "and paints on it");

        let mut selected = app();
        selected.doc.selection = Some(crate::selection::Selection {
            left: 4,
            top: 4,
            right: 12,
            bottom: 12,
        });
        assert!(selected.add_mask());
        selected.flush_render_queue();
        let coverage = &selected.doc.layers[0].mask.as_ref().unwrap().coverage;
        assert_eq!([coverage.at(6, 6), coverage.at(20, 20)], [255, 0]);
    }

    #[test]
    fn reserved_layer_mask_and_duplicate_accept_following_mask_contacts_before_execution() {
        let mut app = app();
        // Keep an earlier contact queued so the new layer exists only in metadata.
        paint(&mut app, 4.5);
        app.finish_painted_stroke();
        app.new_layer();
        let original = app.input_target().unwrap();
        assert!(app.input_layer().is_none());
        assert!(app.create_canvas_mask(Seed::RevealAll));
        let first = app.input_stroke_target();
        assert!(!first.is_raster());
        paint(&mut app, 10.5);
        app.finish_painted_stroke();
        app.duplicate_layer();
        let copied = app.input_target().unwrap();
        assert_ne!(copied, original);
        assert_eq!(app.input_stroke_target(), Target::Mask(copied));
        paint(&mut app, 22.5);
        app.finish_painted_stroke();
        app.flush_render_queue();
        let original = app.doc.layers.iter().find(|l| l.id == original).unwrap();
        let copy = app.doc.layers.iter().find(|l| l.id == copied).unwrap();
        assert!(original.pixels.iter().all(|b| *b == 0));
        assert!(copy.pixels.iter().all(|b| *b == 0));
        assert_eq!(original.mask.as_ref().unwrap().coverage.at(10, 16), 0);
        assert_eq!(original.mask.as_ref().unwrap().coverage.at(22, 16), 255);
        assert_eq!(copy.mask.as_ref().unwrap().coverage.at(10, 16), 0);
        assert_eq!(copy.mask.as_ref().unwrap().coverage.at(22, 16), 0);
        assert_ne!(
            original.mask.as_ref().unwrap().id,
            copy.mask.as_ref().unwrap().id
        );
        app.undo();
        assert_eq!(
            app.doc.layers[app.doc.active]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(22, 16),
            255
        );
        assert_eq!(app.input_stroke_target(), Target::Mask(copied));
        app.undo();
        app.reconcile_mask_focus();
        assert!(app.input_stroke_target().is_raster());
    }
    #[test]
    fn failed_creation_cannot_redirect_a_projected_mask_contact_to_raster() {
        let mut app = app();
        let before = app.doc.clone();
        assert!(app.create_canvas_mask(Seed::Selection)); // No selection at execution.
        paint(&mut app, 16.5);
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert!(!app.history.can_undo() && !app.modified);
        assert!(app.mask_focus.is_none());
    }
    #[test]
    fn recording_rechecks_mask_document_after_pending_commands_are_flushed() {
        let mut app = app();
        assert!(app.create_canvas_mask(Seed::RevealAll));
        app.focus_current_mask(false);
        app.replay.enabled = true;
        assert!(!app.begin_canvas_stroke(settings(&app)));
        assert_eq!(app.doc.version, 2);
        assert!(!app.render_queue.busy() && !app.stroking && !app.history.has_pending_stroke());
        assert!(app.status.contains("recording"));
    }
    #[test]
    fn target_switch_closes_contact_without_redirecting_its_queued_dabs() {
        let mut app = app();
        assert!(app.create_canvas_mask(Seed::RevealAll));
        paint(&mut app, 8.5);
        app.focus_current_mask(false); // Completes the preceding captured mask contact.
        paint(&mut app, 24.5);
        app.finish_painted_stroke();
        app.flush_render_queue();
        let layer = &app.doc.layers[0];
        assert_eq!(layer.mask.as_ref().unwrap().coverage.at(8, 16), 0);
        assert_eq!(layer.mask.as_ref().unwrap().coverage.at(24, 16), 255);
        assert_eq!(layer.pixels.rgba(16 * 32 + 8)[3], 0);
        assert_eq!(layer.pixels.rgba(16 * 32 + 24)[3], 255);
    }
    #[test]
    fn stale_thumbnail_cannot_select_the_projected_replacement_mask() {
        let mut app = app();
        assert!(app.create_canvas_mask(Seed::RevealAll));
        app.flush_render_queue();
        let old = app.doc.layers[0].mask.as_ref().unwrap().id;
        assert!(app.edit_canvas_mask(Command::Remove(old)));
        assert!(app.create_canvas_mask(Seed::RevealAll));
        let replacement = app.input_stroke_target();
        assert_ne!(replacement, Target::Mask(old));
        assert!(!app.select_paint_target(0, Some(old)));
        assert_eq!(app.input_stroke_target(), replacement);
        let mut old_settings = settings(&app);
        old_settings.target = Target::Mask(old);
        assert!(!app.begin_canvas_stroke(old_settings));
        paint(&mut app, 16.5);
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.input_stroke_target(), replacement);
        assert_eq!(
            app.doc.layers[0].mask.as_ref().unwrap().coverage.at(16, 16),
            0
        );
        assert!(app.doc.layers[0].pixels.iter().all(|b| *b == 0));
    }
    #[test]
    fn unsupported_mask_adjustments_and_filters_preserve_source() {
        let mut app = app();
        assert!(app.create_canvas_mask(Seed::RevealAll));
        app.flush_render_queue();
        let before = app.doc.clone();
        let revision = app.revision;
        app.start_adjustment(crate::adjustments::Kind::Levels);
        app.filter("pixel.r = 0;");
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert_eq!(app.revision, revision);
        assert!(!app.render_queue.busy() && app.pending_clipboard.is_empty());
        assert!(
            app.adjustment.is_none() && app.affine_preview.is_none() && app.filter_job.is_none()
        );
        assert!(app.status.contains("image thumbnail"));
    }
    #[test]
    fn projected_mask_commands_and_geometry_metadata_match_execution() {
        let mut doc = Document::new(8, 8);
        let target = doc.layers[0].id;
        let id = LayerId::new();
        let mut projected = crate::pending_layers::PendingLayers::from_document(&doc);
        for command in [
            Command::Create {
                id,
                seed: Seed::RevealAll,
            },
            Command::Enabled { id, value: false },
            Command::Linked { id, value: false },
            Command::Invert(id),
            Command::Apply(id),
            Command::Create {
                id: LayerId::new(),
                seed: Seed::HideAll,
            },
        ] {
            projected = projected.with_mask_command(target, command).unwrap();
            doc = crate::document_edits::Edit::LayerMask { target, command }
                .prepare(&doc)
                .unwrap()
                .0;
            assert_eq!(
                projected.properties(target),
                Some(crate::pending_layers::Properties::from_layer(
                    &doc.layers[0]
                ))
            );
        }
        doc.add_layer();
        projected = crate::pending_layers::PendingLayers::from_document(&doc);
        let upper = doc.layers[1].id;
        let (merged, _, folder_result) = projected.with_merge(upper).unwrap();
        assert!(folder_result.is_none());
        doc = crate::document_edits::Edit::MergeTarget {
            target: upper,
            folder_result,
        }
        .prepare(&doc)
        .unwrap()
        .0;
        assert!(doc.layers[0].mask.is_none());
        assert_eq!(
            merged.properties(target),
            Some(crate::pending_layers::Properties::from_layer(
                &doc.layers[0]
            ))
        );
        doc.group_active().unwrap();
        let folder = doc.layers[doc.active].id;
        let mask = LayerId::new();
        crate::layer_mask_edits::apply(
            &mut doc,
            1,
            Command::Create {
                id: mask,
                seed: Seed::RevealAll,
            },
            None,
        )
        .unwrap();
        projected = crate::pending_layers::PendingLayers::from_document(&doc);
        assert!(projected.with_ungroup(folder).is_err());
        let (merged, selected, reserved) = projected.with_merge(folder).unwrap();
        doc = crate::document_edits::Edit::MergeTarget {
            target: folder,
            folder_result: reserved,
        }
        .prepare(&doc)
        .unwrap()
        .0;
        assert_eq!(
            merged.properties(selected),
            Some(crate::pending_layers::Properties::from_layer(
                &doc.layers[doc.active]
            ))
        );
        assert!(
            projected
                .target_blocker(folder, Target::Mask(mask), true)
                .is_none()
        );
        assert!(projected.paint_blocker(folder).is_some());
    }
}
