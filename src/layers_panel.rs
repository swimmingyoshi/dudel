use crate::{
    Dudel,
    blend::BlendMode,
    egui,
    icons::{self, Icon},
};
use egui::{Color32, FontId, Sense, Stroke, StrokeKind, pos2, vec2};

const ROW_HEIGHT: f32 = 50.0;
const FOLDER_ROW_HEIGHT: f32 = 40.0;
const INDENT: f32 = 14.0;

enum RowAction {
    ReferencePosition {
        id: u64,
        gap: usize,
        depth: u8,
    },
    Select(usize),
    PaintTarget(usize, Option<crate::document::LayerId>),
    ToggleVisible(usize),
    ToggleCollapsed(usize),
    Rename(usize),
    Duplicate(usize),
    Merge(usize),
    Group(usize),
    Ungroup(usize),
    Step(usize, bool),
    Clear(usize),
    Delete(usize),
    /// Leave the entry out of the pop-out preview window, or bring it back.
    TogglePreview(usize),
    /// Move an entry above stack position `gap` (0 = bottom) at `depth`.
    Reorder {
        from: usize,
        gap: usize,
        depth: u8,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Drop {
    Above,
    Into,
    Below,
}
pub(crate) struct OpacityEdit {
    target: crate::document::LayerId,
    mode: OpacityMode,
}
enum OpacityMode {
    Immediate { original: f32, modified: bool },
    Queued(u64),
}

/// Mouse-wheel distance that steps one blend mode.
pub(crate) const BLEND_WHEEL_STEP: f32 = 30.0;

/// Blend mode dropdown, grouped like other painting apps. The open list uses
/// the window height before scrolling. Over the closed box the mouse wheel
/// steps through modes; Up/Down step while it is hovered, focused or open, so
/// each mode can be judged on the canvas in turn.
pub(crate) fn blend_selector(ui: &mut egui::Ui, current: BlendMode, folder: bool) -> BlendMode {
    let mut blend = current;
    let response = ui
        .add(
            egui::Button::new(blend.name())
                .right_text("⏷")
                .min_size(egui::vec2(130.0, 0.0)),
        )
        .on_hover_text("Blend mode · scroll or press Up/Down to step through modes");
    let popup = response.id.with("blend list");
    // Compact rows, and all the room on the side the list opens before it
    // scrolls, so the whole set is visible at once on ordinary screens.
    let screen = ui.ctx().content_rect();
    let room =
        (response.rect.top() - screen.top()).max(screen.bottom() - response.rect.bottom()) - 24.0;
    egui::Popup::menu(&response)
        .id(popup)
        .width(response.rect.width().max(170.0))
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().interact_size.y = 24.0;
            ui.spacing_mut().button_padding.y = 3.0;
            egui::ScrollArea::vertical()
                .max_height(room.max(120.0))
                .min_scrolled_height(room.max(120.0))
                .show(ui, |ui| {
                    let pass = folder.then_some(&[BlendMode::PassThrough][..]);
                    for (index, group) in pass.into_iter().chain(BlendMode::GROUPS).enumerate() {
                        if index > 0 {
                            ui.separator();
                        }
                        for mode in group.iter() {
                            let selected = ui.add(
                                egui::Button::selectable(blend == *mode, mode.name())
                                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                            );
                            if selected.clicked() {
                                blend = *mode;
                                ui.close();
                            }
                        }
                    }
                });
        });
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let mut step = 0;
    if response.hovered() && !open {
        let wheel = response.id.with("blend wheel");
        let mut travel = ui.data(|d| d.get_temp::<f32>(wheel)).unwrap_or(0.0)
            + ui.input(|i| i.raw_scroll_delta.y);
        // Wheel up moves up the list, like the open menu.
        while travel >= BLEND_WHEEL_STEP {
            step -= 1;
            travel -= BLEND_WHEEL_STEP;
        }
        while travel <= -BLEND_WHEEL_STEP {
            step += 1;
            travel += BLEND_WHEEL_STEP;
        }
        ui.data_mut(|d| d.insert_temp(wheel, travel));
    }
    if response.hovered() || response.has_focus() || open {
        let (down, up) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            )
        });
        step += i32::from(down) - i32::from(up);
    }
    if step != 0 {
        blend = blend.stepped(step, folder);
    }
    blend
}

impl Dudel {
    #[cfg(test)]
    pub(crate) fn set_layer_opacity(&mut self, index: usize, opacity: f32) {
        if let Some(target) = self.doc.layers.get(index).map(|layer| layer.id) {
            self.set_target_opacity(target, opacity);
        }
    }
    pub(crate) fn set_target_opacity(&mut self, target: crate::document::LayerId, opacity: f32) {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return;
        }
        if self
            .opacity_edit
            .as_ref()
            .is_some_and(|edit| edit.target != target)
        {
            self.finish_opacity_edit();
        }
        if self.opacity_edit.is_none() {
            self.finish_layer_rename(true);
            self.close_input_gestures();
        }
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let Some(properties) = context.properties(target) else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        if properties.opacity == opacity {
            return;
        }
        if self.render_queue.busy() {
            let active = self.input_target().unwrap_or(target);
            let (projected, _) = match context.with_property(
                target,
                active,
                &crate::document_edits::Property::Opacity(opacity),
            ) {
                Ok(result) => result,
                Err(error) => {
                    self.status = error;
                    return;
                }
            };
            let result = match self.opacity_edit.as_ref().map(|edit| &edit.mode) {
                Some(OpacityMode::Queued(ticket)) => self
                    .render_queue
                    .update_opacity(*ticket, target, opacity)
                    .map(|()| *ticket),
                None => self.render_queue.start_opacity(target, opacity),
                Some(OpacityMode::Immediate { .. }) => {
                    Err("Close the immediate opacity gesture before queued rendering".into())
                }
            };
            match result {
                Ok(ticket) => {
                    self.opacity_edit = Some(OpacityEdit {
                        target,
                        mode: OpacityMode::Queued(ticket),
                    });
                    self.pending_layers = Some(projected);
                    self.status = "Opacity preview queued".into();
                }
                Err(error) => {
                    self.finish_opacity_edit();
                    self.status = error;
                }
            }
        } else {
            let Some(index) = self.doc.layers.iter().position(|layer| layer.id == target) else {
                return;
            };
            if self.opacity_edit.is_none() {
                self.history.begin_gesture_checkpoint(&self.doc);
                self.opacity_edit = Some(OpacityEdit {
                    target,
                    mode: OpacityMode::Immediate {
                        original: self.doc.layers[index].opacity,
                        modified: self.modified,
                    },
                });
            }
            self.doc.layers[index].opacity = opacity;
            self.changed();
        }
    }
    pub(crate) fn finish_opacity_edit(&mut self) {
        let Some(edit) = self.opacity_edit.take() else {
            return;
        };
        match edit.mode {
            OpacityMode::Queued(ticket) => {
                if let Err(error) = self.render_queue.end_opacity(ticket) {
                    self.status = error;
                }
            }
            OpacityMode::Immediate { original, modified } => {
                if self
                    .doc
                    .layers
                    .iter()
                    .find(|layer| layer.id == edit.target)
                    .is_some_and(|layer| layer.opacity == original)
                {
                    self.history.discard_pending_checkpoint(&self.doc);
                    self.modified = modified;
                } else {
                    self.history.settle(&self.doc);
                }
            }
        }
    }
    pub(crate) fn finish_layer_rename(&mut self, commit: bool) {
        let Some(index) = self.renaming.take() else {
            return;
        };
        let name = std::mem::take(&mut self.rename_draft);
        if !commit
            || index >= self.doc.layers.len()
            || (self.doc.layers[index].name == name && !self.render_queue.busy())
        {
            return;
        }
        if name.len() > 4096 {
            self.status = "Layer names must fit in 4096 UTF-8 bytes".into();
            return;
        }
        self.dispatch_document_edit(
            crate::document_edits::Edit::Property {
                target: crate::document_edits::Target::Identity(self.doc.layers[index].id),
                property: crate::document_edits::Property::Name(name),
            },
            "Layer renamed",
            "Layer name unchanged",
        );
    }
    /// New layer above the active entry; inside an open folder that is
    /// selected, it becomes the folder's top layer.
    pub(crate) fn new_layer(&mut self) {
        self.create_canvas_layer();
    }
    pub(crate) fn new_folder(&mut self) {
        self.create_canvas_folder(false);
    }
    pub(crate) fn group_into_folder(&mut self) {
        self.create_canvas_folder(true);
    }
    pub(crate) fn ungroup_folder(&mut self) {
        self.ungroup_canvas_folder();
    }
    pub(crate) fn duplicate_layer(&mut self) {
        self.duplicate_canvas_layer();
    }
    /// Merge down for layers; flatten for folders.
    pub(crate) fn merge_down(&mut self) {
        self.merge_canvas_layer();
    }
    pub(crate) fn delete_layer(&mut self) {
        self.delete_canvas_layer();
    }
    fn reorder_layer(&mut self, from: usize, gap: usize, depth: u8) {
        self.reorder_canvas_layer(from, gap, depth);
    }

    pub(crate) fn layers_ui(&mut self, ui: &mut egui::Ui, inline_controls: bool) {
        if self.reference_layers.active.is_some() {
            self.reference_properties_ui(ui);
            ui.add_space(4.0);
            self.layer_rows_ui(ui);
            return;
        }
        if inline_controls {
            self.layer_controls_ui(ui);
        }
        ui.add_space(4.0);
        self.layer_rows_ui(ui);
    }

    pub(crate) fn layer_controls_ui(&mut self, ui: &mut egui::Ui) {
        let target_ready = self.input_layer().is_some();
        let active = self.input_layer().unwrap_or(self.doc.active);
        let layer = &self.doc.layers[active];
        let folder = self.input_is_folder();
        let block = self.doc.block(active);
        let can_merge = if folder {
            true
        } else {
            active > 0
                && !self.doc.layers[active - 1].folder
                && self.doc.layers[active - 1].depth == layer.depth
        };
        // Nine actions share this row, including the three right-aligned locks.
        let action_size = ((ui.available_width() - 16.0) / 9.0).clamp(20.0, 30.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if icons::button(ui, Icon::Plus, action_size, false, self.doc.has_room(1, 1))
                .on_hover_text("New layer · Ctrl+Shift+N")
                .clicked()
            {
                self.new_layer();
            }
            if icons::button(
                ui,
                Icon::NewFolder,
                action_size,
                false,
                self.doc.has_room(0, 1),
            )
            .on_hover_text("New folder · Ctrl+G puts the selected layer in a folder")
            .clicked()
            {
                self.new_folder();
            }
            if icons::button(ui, Icon::Duplicate, action_size, false, target_ready)
                .on_hover_text("Duplicate · Ctrl+J")
                .clicked()
            {
                self.duplicate_layer();
            }
            if icons::button(
                ui,
                Icon::MergeDown,
                action_size,
                false,
                target_ready && can_merge,
            )
            .on_hover_text(if folder {
                "Merge folder into one layer · Ctrl+E"
            } else {
                "Merge down · Ctrl+E"
            })
            .clicked()
            {
                self.merge_down();
            }
            if icons::button(
                ui,
                Icon::Trash,
                action_size,
                false,
                target_ready && block.len() < self.doc.layers.len(),
            )
            .on_hover_text(if folder {
                "Delete folder and its contents"
            } else {
                "Delete layer"
            })
            .clicked()
            {
                self.delete_layer();
            }
            if icons::button(
                ui,
                Icon::Image,
                action_size,
                false,
                self.reference_layers.layers.len() < crate::reference_layers::MAX_REFERENCES,
            )
            .on_hover_text("Add reference image: place a picture on the canvas to draw from")
            .clicked()
            {
                self.add_reference_layer();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let Some(target) = self.input_target() else {
                    return;
                };
                let Some(properties) = self.input_properties() else {
                    return;
                };
                let folder = self.input_is_folder();
                let (mut clipped, mut locked, mut alpha_locked) = (
                    properties.clipped,
                    properties.locked,
                    properties.alpha_locked,
                );
                if icons::button(ui, Icon::Lock, action_size, locked, target_ready && !folder)
                    .on_hover_text("Lock layer: protect pixels from painting, filters and deletion")
                    .clicked()
                {
                    locked = !locked;
                }
                if icons::button(
                    ui,
                    Icon::AlphaLock,
                    action_size,
                    alpha_locked,
                    target_ready && !folder,
                )
                .on_hover_text(
                    "Lock transparency: recolor existing marks without painting outside them",
                )
                .clicked()
                {
                    alpha_locked = !alpha_locked;
                }
                if icons::button(
                    ui,
                    Icon::Clip,
                    action_size,
                    clipped,
                    target_ready && (block.start > 0 || clipped),
                )
                .on_hover_text("Clip to the layer below: paint only inside its shapes")
                .clicked()
                {
                    clipped = !clipped;
                }
                use crate::document_edits::Property;
                if clipped != properties.clipped {
                    self.set_canvas_property(target, Property::Clipped(clipped));
                }
                if locked != properties.locked {
                    self.set_canvas_property(target, Property::Locked(locked));
                }
                if alpha_locked != properties.alpha_locked {
                    self.set_canvas_property(target, Property::AlphaLocked(alpha_locked));
                }
            });
        });
        self.layer_properties_ui(ui);
        if self.mask_controls_available() {
            self.mask_controls_ui(ui);
        }
    }

    fn layer_rows_ui(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        self.reference_layers.bind_positions(&self.doc);
        let reveal_key = self
            .input_layer()
            .and_then(|_| self.input_target())
            .map(|id| {
                (
                    id,
                    self.input_stroke_target(),
                    [
                        ui.available_width().round() as i32,
                        ui.available_height().round() as i32,
                    ],
                )
            });
        if self.reference_layers.active.is_none() {
            ui.ctx()
                .data_mut(|d| d.remove::<(u64, [i32; 2])>(egui::Id::new("revealed reference")));
        }
        let reveal = self.layer_reveal != reveal_key;
        self.layer_reveal = reveal_key;
        egui::ScrollArea::vertical()
            .id_salt("layer rows")
            .auto_shrink([false, false])
            .max_height(ui.available_height())
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                self.reference_rows_at(ui, self.doc.layers.len());
                // Top of the stack first; contents of collapsed folders are hidden.
                let mut hide_deeper_than: Option<u8> = None;
                for index in (0..self.doc.layers.len()).rev() {
                    let layer = &self.doc.layers[index];
                    if let Some(depth) = hide_deeper_than {
                        if layer.depth > depth {
                            self.reference_rows_at(ui, index);
                            continue;
                        }
                        hide_deeper_than = None;
                    }
                    if layer.folder && layer.collapsed {
                        hide_deeper_than = Some(layer.depth);
                    }
                    let id = layer.id;
                    ui.push_id(id, |ui| self.layer_row(ui, index, reveal, &mut action));
                    self.reference_rows_at(ui, index);
                }
            });
        if let Some(from) = egui::DragAndDrop::payload::<usize>(ui.ctx())
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
            && let Some(layer) = self.doc.layers.get(*from)
        {
            egui::Area::new(egui::Id::new("layer drag preview"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pointer + vec2(12.0, 4.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(&layer.name);
                    });
                });
        }
        if let Some(from) =
            egui::DragAndDrop::payload::<crate::reference_stack::ReferenceDrag>(ui.ctx())
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
            && let Some(board) = self.reference_layers.layers.iter().find(|b| b.id == from.0)
        {
            egui::Area::new(egui::Id::new("reference drag preview"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pointer + vec2(12.0, 4.0))
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(&board.name);
                    });
                });
        }
        self.apply_row_action(action);
    }

    fn apply_row_action(&mut self, action: Option<RowAction>) {
        let Some(action) = action else {
            return;
        };
        if let RowAction::ReferencePosition { id, gap, depth } = action {
            self.place_reference(id, gap, depth);
            return;
        }
        self.finish_layer_rename(true);
        // Any artwork row returns from reference editing to that layer.
        self.deselect_reference();
        if let RowAction::PaintTarget(index, mask) = action {
            self.select_paint_target(index, mask);
            return;
        }
        if let RowAction::Select(index) = action {
            self.select_layer_target(index);
            return;
        }
        if let RowAction::Reorder { from, gap, depth } = action {
            self.reorder_layer(from, gap, depth);
            return;
        }
        if let RowAction::Group(index) = action {
            if self.select_layer_target(index) {
                self.group_into_folder();
            }
            return;
        }
        if let RowAction::Ungroup(index) = action {
            if self.select_layer_target(index) {
                self.ungroup_folder();
            }
            return;
        }
        if let RowAction::Duplicate(index) = action {
            if self.select_layer_target(index) {
                self.duplicate_layer();
            }
            return;
        }
        if let RowAction::Delete(index) = action {
            if self.select_layer_target(index) {
                self.delete_layer();
            }
            return;
        }
        if let RowAction::Clear(index) = action {
            if self.select_layer_target(index) {
                self.clear_layer_pixels();
            }
            return;
        }
        if let RowAction::Merge(index) = action {
            if self.select_layer_target(index) {
                self.merge_down();
            }
            return;
        }
        if let RowAction::Step(index, up) = action {
            if self.select_layer_target(index) {
                self.step_canvas_layer(up);
            }
            return;
        }
        if let RowAction::ToggleVisible(index) = action {
            self.toggle_canvas_layer(index, false);
            return;
        }
        if let RowAction::ToggleCollapsed(index) = action {
            self.toggle_canvas_layer(index, true);
            return;
        }
        let target = match &action {
            RowAction::Rename(index) => self.doc.layers.get(*index).map(|layer| layer.id),
            _ => None,
        };
        self.cancel_gestures();
        let action = if let Some(target) = target {
            let Some(index) = self.doc.layers.iter().position(|layer| layer.id == target) else {
                self.status = "Selected layer no longer exists".into();
                return;
            };
            match action {
                RowAction::Rename(_) => RowAction::Rename(index),
                other => other,
            }
        } else {
            action
        };
        match action {
            RowAction::Select(index) => {
                if self.doc.active != index {
                    self.doc.active = index;
                }
            }
            RowAction::ToggleVisible(index) => {
                self.toggle_canvas_layer(index, false);
            }
            RowAction::ToggleCollapsed(index) => {
                self.toggle_canvas_layer(index, true);
            }
            RowAction::Rename(index) => {
                self.doc.active = index;
                self.renaming = Some(index);
                self.rename_draft = self.doc.layers[index].name.clone();
            }
            RowAction::Duplicate(index) => {
                self.doc.active = index;
                self.duplicate_layer();
            }
            RowAction::Merge(index) => {
                self.doc.active = index;
                self.merge_down();
            }
            RowAction::Group(index) => {
                self.doc.active = index;
                self.group_into_folder();
            }
            RowAction::Ungroup(index) => {
                self.doc.active = index;
                self.ungroup_folder();
            }
            RowAction::Step(index, up) => {
                self.doc.active = index;
                self.step_canvas_layer(up);
            }
            RowAction::Clear(index) => {
                self.doc.active = index;
                self.clear_layer_pixels();
            }
            RowAction::TogglePreview(index) => {
                let id = self.doc.layers[index].id;
                self.preview.toggle_hidden(id);
                self.status = if self.preview.hidden.contains(&id) {
                    "Hidden in the preview window"
                } else {
                    "Shown in the preview window"
                }
                .into();
            }
            RowAction::Delete(index) => {
                self.doc.active = index;
                self.delete_layer();
            }
            RowAction::Reorder { from, gap, depth } => self.reorder_layer(from, gap, depth),
            RowAction::ReferencePosition { .. } => unreachable!(),
            RowAction::PaintTarget(index, mask) => {
                self.select_paint_target(index, mask);
            }
        }
    }

    fn layer_properties_ui(&mut self, ui: &mut egui::Ui) {
        if self.input_properties().is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Updating layers…");
            });
            return;
        }
        let compact = ui.available_width() < 250.0;
        ui.horizontal(|ui| {
            let Some(target) = self.input_target() else {
                return;
            };
            let Some(properties) = self.input_properties() else {
                return;
            };
            let blend = blend_selector(ui, properties.blend, properties.folder);
            if blend != properties.blend {
                self.set_canvas_property(target, crate::document_edits::Property::Blend(blend));
            }
            if ui
                .selectable_label(self.effects_window_open, "fx")
                .on_hover_text("Layer effects: shadows, glows, strokes, overlays, bevels")
                .clicked()
            {
                self.effects_window_open = !self.effects_window_open;
            }
            let mut percent = (properties.opacity * 100.0).round();
            let slider = if compact {
                ui.add(
                    egui::DragValue::new(&mut percent)
                        .range(0.0..=100.0)
                        .speed(1.0)
                        .suffix("%")
                        .max_decimals(0),
                )
                .on_hover_text("Layer opacity: drag the percentage or click to type")
            } else {
                ui.spacing_mut().slider_width = (ui.available_width()
                    - ui.spacing().interact_size.x
                    - ui.spacing().item_spacing.x)
                    .max(12.0);
                ui.add(
                    egui::Slider::new(&mut percent, 0.0..=100.0)
                        .suffix("%")
                        .integer()
                        .trailing_fill(true),
                )
            };
            if slider.changed() {
                self.set_target_opacity(target, percent / 100.0);
            }
            if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
                self.finish_opacity_edit();
            }
        });
    }

    /// Where a dragged entry would land when released over row `index`.
    fn drop_target(&self, index: usize, fraction: f32) -> (usize, u8, Drop) {
        let layer = &self.doc.layers[index];
        if layer.folder {
            if fraction < 0.3 {
                (index + 1, layer.depth, Drop::Above)
            } else if fraction < 0.7 || !layer.collapsed {
                // Into the folder as its top layer.
                (index, layer.depth + 1, Drop::Into)
            } else {
                (self.doc.block(index).start, layer.depth, Drop::Below)
            }
        } else if fraction < 0.5 {
            (index + 1, layer.depth, Drop::Above)
        } else {
            (index, layer.depth, Drop::Below)
        }
    }

    fn layer_row(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        reveal: bool,
        action: &mut Option<RowAction>,
    ) {
        let layer = &self.doc.layers[index];
        let height = if layer.folder {
            FOLDER_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        };
        let width = ui
            .available_width()
            .min(ui.clip_rect().right() - ui.next_widget_position().x)
            .max(0.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let selected = self.reference_layers.active.is_none() && self.input_layer() == Some(index);
        if selected && reveal {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        let shown = self.doc.shown(index);
        let visuals = ui.visuals().clone();
        let painter = ui.painter_at(rect.expand(2.0));
        let fill = if selected {
            visuals.selection.bg_fill.gamma_multiply(0.85)
        } else if response.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else if layer.folder {
            visuals.faint_bg_color
        } else {
            visuals.extreme_bg_color.gamma_multiply(0.6)
        };
        painter.rect_filled(rect, 6.0, fill);
        let text = if selected {
            visuals.selection.stroke.color
        } else if shown {
            visuals.text_color()
        } else {
            visuals.weak_text_color()
        };
        let weak = if selected {
            visuals.selection.stroke.color.gamma_multiply(0.8)
        } else {
            visuals.weak_text_color()
        };

        // Visibility toggle.
        let eye = egui::Rect::from_min_size(rect.min + vec2(2.0, 0.0), vec2(28.0, height));
        let eye_response = ui.interact(eye, response.id.with("eye"), Sense::click());
        icons::paint(
            &painter,
            egui::Rect::from_center_size(eye.center(), vec2(18.0, 18.0)),
            if layer.visible {
                Icon::Eye
            } else {
                Icon::EyeOff
            },
            if layer.visible {
                text
            } else {
                weak.gamma_multiply(0.7)
            },
        );
        if eye_response.on_hover_text("Show or hide").clicked() {
            *action = Some(RowAction::ToggleVisible(index));
        }

        // Nesting indent, then the clipping connector for clipped entries.
        let mut x = eye.right() + 2.0 + layer.depth as f32 * INDENT;
        for level in 0..layer.depth {
            let guide_x = eye.right() + 8.0 + level as f32 * INDENT;
            painter.line_segment(
                [pos2(guide_x, rect.top()), pos2(guide_x, rect.bottom())],
                Stroke::new(1.0_f32, Color32::from_white_alpha(28)),
            );
        }
        if layer.clipped {
            painter.line_segment(
                [
                    pos2(x + 5.0, rect.top() + 6.0),
                    pos2(x + 5.0, rect.center().y),
                ],
                Stroke::new(2.0_f32, weak),
            );
            painter.line_segment(
                [
                    pos2(x + 5.0, rect.center().y),
                    pos2(x + 12.0, rect.center().y),
                ],
                Stroke::new(2.0_f32, weak),
            );
            x += 16.0;
        }
        let thumb_side = height - 10.0;
        if layer.folder {
            let chevron = egui::Rect::from_min_size(pos2(x, rect.top()), vec2(20.0, height));
            let chevron_response =
                ui.interact(chevron, response.id.with("chevron"), Sense::click());
            icons::paint(
                &painter,
                egui::Rect::from_center_size(chevron.center(), vec2(14.0, 14.0)),
                if layer.collapsed {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronDown
                },
                text,
            );
            if chevron_response
                .on_hover_text(if layer.collapsed {
                    "Expand folder"
                } else {
                    "Collapse folder"
                })
                .clicked()
            {
                *action = Some(RowAction::ToggleCollapsed(index));
            }
            x = chevron.right() + 2.0;
            icons::paint(
                &painter,
                egui::Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(thumb_side, thumb_side)),
                Icon::Folder,
                text,
            );
        } else {
            let thumb =
                egui::Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(thumb_side, thumb_side));
            if let Some(texture) = self.layer_thumbnails.get(index) {
                painter.image(
                    texture.id(),
                    thumb,
                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    if shown {
                        Color32::WHITE
                    } else {
                        Color32::from_white_alpha(110)
                    },
                );
            }
            painter.rect_stroke(
                thumb,
                2.0,
                Stroke::new(
                    1.0_f32,
                    if selected && self.input_stroke_target().is_raster() {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(70)
                    },
                ),
                StrokeKind::Outside,
            );
        }

        let image_rect =
            egui::Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(thumb_side, thumb_side));
        let masks_visible = self.mask_controls_available();
        if masks_visible
            && ui
                .interact(
                    image_rect,
                    response.id.with("artwork target"),
                    Sense::click(),
                )
                .on_hover_text("Edit layer artwork")
                .clicked()
        {
            *action = Some(RowAction::PaintTarget(index, None));
        }
        let mut text_left = x + thumb_side + 10.0;
        if masks_visible && let Some(mask) = &layer.mask {
            let thumb = egui::Rect::from_min_size(
                pos2(text_left - 4.0, rect.top() + 5.0),
                vec2(thumb_side, thumb_side),
            );
            if let Some(Some(texture)) = self.mask_thumbnails.get(index) {
                painter.image(
                    texture.id(),
                    thumb,
                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            let focused = selected
                && self.input_stroke_target() == crate::paint_target::Target::Mask(mask.id);
            painter.rect_stroke(
                thumb,
                2.0,
                Stroke::new(
                    if focused { 2.0_f32 } else { 1.0_f32 },
                    if focused {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(70)
                    },
                ),
                StrokeKind::Outside,
            );
            if !mask.enabled {
                painter.line_segment(
                    [thumb.left_top(), thumb.right_bottom()],
                    Stroke::new(2.0_f32, Color32::LIGHT_RED),
                );
                painter.line_segment(
                    [thumb.right_top(), thumb.left_bottom()],
                    Stroke::new(2.0_f32, Color32::LIGHT_RED),
                );
            }
            if ui
                .interact(
                    thumb,
                    response.id.with(("mask target", mask.id)),
                    Sense::click(),
                )
                .on_hover_text("Edit mask: black conceals, white reveals")
                .clicked()
            {
                *action = Some(RowAction::PaintTarget(index, Some(mask.id)));
            }
            text_left = thumb.right() + 8.0;
        }

        // Name and summary.
        let own_symmetry = self.layer_symmetry.get(&layer.id).copied();
        let symmetry_badge = own_symmetry.is_some() || self.symmetry.active();
        let guides_badge =
            (0..self.rulers.len()).any(|ruler| self.ruler_owner(ruler) == Some(layer.id));
        let badges = (layer.locked as usize + layer.alpha_locked as usize) as f32 * 20.0
            + (symmetry_badge as usize + guides_badge as usize) as f32 * 24.0;
        let text_width = (rect.right() - text_left - 8.0 - badges).max(0.0);
        let name_top = if layer.folder {
            rect.top() + 3.0
        } else {
            rect.top() + 6.0
        };
        let name_rect =
            egui::Rect::from_min_size(pos2(text_left, name_top), vec2(text_width, 20.0));
        if self.renaming == Some(index) {
            // Consume Escape before TextEdit so losing focus cannot commit the
            // draft after the editor handles the same key.
            let cancel = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let mut name = std::mem::take(&mut self.rename_draft);
            let edit = ui.put(
                name_rect.expand2(vec2(2.0, 1.0)),
                egui::TextEdit::singleline(&mut name).char_limit(1024),
            );
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            self.rename_draft = name;
            if cancel || edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.finish_layer_rename(!cancel);
                ui.memory_mut(|memory| memory.surrender_focus(edit.id));
            }
        } else {
            let mut job = egui::text::LayoutJob::simple_singleline(
                layer.name.clone(),
                FontId::proportional(14.0),
                text,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
            painter.galley(name_rect.min, painter.layout_job(job), text);
        }
        let layer = &self.doc.layers[index];
        let mut summary = format!("{} · {:.0}%", layer.blend.name(), layer.opacity * 100.0);
        if layer.has_effects() {
            summary.push_str(" · fx");
        }
        if layer.folder {
            let items = self.doc.layers[self.doc.block(index)]
                .iter()
                .filter(|l| l.depth == layer.depth + 1)
                .count()
                + self
                    .reference_layers
                    .layers
                    .iter()
                    .filter(|b| self.reference_layers.location(b, &self.doc).2 == Some(index))
                    .count();
            summary = format!(
                "{items} item{} · {summary}",
                if items == 1 { "" } else { "s" }
            );
        }
        if layer.clipped {
            summary.push_str(" · clipped");
        }
        let mut job =
            egui::text::LayoutJob::simple_singleline(summary, FontId::proportional(11.5), weak);
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
        let galley = painter.layout_job(job);
        painter.galley(
            pos2(text_left, rect.bottom() - 6.0 - galley.size().y),
            galley,
            weak,
        );
        let mut badge_x = rect.right() - 8.0;
        if layer.locked {
            let r = egui::Rect::from_min_max(
                pos2(badge_x - 16.0, rect.center().y - 8.0),
                pos2(badge_x, rect.center().y + 8.0),
            );
            icons::paint(&painter, r, Icon::Lock, weak);
            badge_x -= 20.0;
        }
        if layer.alpha_locked {
            let r = egui::Rect::from_min_max(
                pos2(badge_x - 14.0, rect.center().y - 7.0),
                pos2(badge_x, rect.center().y + 7.0),
            );
            icons::paint(&painter, r, Icon::AlphaLock, weak);
            badge_x -= 20.0;
        }
        let mut symmetry_response = None;
        let mut guides_response = None;
        if symmetry_badge {
            let r = egui::Rect::from_center_size(
                pos2(badge_x - 10.0, rect.center().y),
                vec2(22.0, 26.0),
            );
            let effective = own_symmetry.unwrap_or(self.symmetry);
            icons::paint(
                &painter,
                r.shrink(3.0),
                if own_symmetry.is_some() {
                    Icon::Symmetry
                } else {
                    Icon::Global
                },
                if effective.active() { text } else { weak },
            );
            if !effective.active() {
                painter.line_segment(
                    [
                        r.left_top() + vec2(3.0, 3.0),
                        r.right_bottom() - vec2(3.0, 3.0),
                    ],
                    Stroke::new(1.5_f32, weak),
                );
            }
            symmetry_response = Some(
                ui.interact(
                    r,
                    egui::Id::new(("layer symmetry", layer.id)),
                    Sense::click(),
                )
                .on_hover_text(format!(
                    "{} symmetry: {} · click to edit",
                    if own_symmetry.is_some() {
                        "Layer"
                    } else {
                        "Global"
                    },
                    effective.mode.name()
                )),
            );
            badge_x -= 24.0;
        }
        if guides_badge {
            let r = egui::Rect::from_center_size(
                pos2(badge_x - 10.0, rect.center().y),
                vec2(22.0, 26.0),
            );
            icons::paint(&painter, r.shrink(3.0), Icon::Guide, text);
            guides_response = Some(
                ui.interact(r, egui::Id::new(("layer guides", layer.id)), Sense::click())
                    .on_hover_text("Layer guides · click to edit ownership, add or remove guides"),
            );
        }

        // Drag to reorder: above, below, or into a folder row.
        if response.drag_started() {
            response.dnd_set_drag_payload(index);
        }
        if let Some(from) = response.dnd_hover_payload::<usize>()
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            let fraction = (pointer.y - rect.top()) / rect.height();
            let (gap, depth, place) = self.drop_target(index, fraction);
            if *from != index {
                let accent = visuals.selection.stroke.color;
                let left = eye.right() + depth as f32 * INDENT;
                match place {
                    Drop::Above | Drop::Below => {
                        let y = if place == Drop::Above {
                            rect.top()
                        } else {
                            rect.bottom()
                        };
                        painter.line_segment(
                            [pos2(left, y), pos2(rect.right(), y)],
                            Stroke::new(3.0_f32, accent),
                        );
                        painter.circle_filled(pos2(left, y), 4.0, accent);
                    }
                    Drop::Into => {
                        painter.rect_stroke(
                            rect.shrink(1.0),
                            6.0,
                            Stroke::new(2.0_f32, accent),
                            StrokeKind::Inside,
                        );
                    }
                }
            }
            if let Some(from) = response.dnd_release_payload::<usize>()
                && *from != index
            {
                *action = Some(RowAction::Reorder {
                    from: *from,
                    gap,
                    depth,
                });
            }
        }

        if response
            .dnd_hover_payload::<crate::reference_stack::ReferenceDrag>()
            .is_some()
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            let (gap, depth, _) = self.drop_target(index, (pointer.y - rect.top()) / rect.height());
            painter.rect_stroke(
                rect.shrink(1.0),
                4.0,
                Stroke::new(2.0_f32, visuals.selection.stroke.color),
                StrokeKind::Inside,
            );
            if let Some(from) =
                response.dnd_release_payload::<crate::reference_stack::ReferenceDrag>()
            {
                *action = Some(RowAction::ReferencePosition {
                    id: from.0,
                    gap,
                    depth,
                });
            }
        }

        let badge_clicked = symmetry_response.as_ref().is_some_and(|r| r.clicked())
            || guides_response.as_ref().is_some_and(|r| r.clicked());
        if response.double_clicked() && action.is_none() && !badge_clicked {
            *action = Some(RowAction::Rename(index));
        } else if response.clicked() && action.is_none() && !badge_clicked {
            *action = Some(RowAction::Select(index));
        }
        let folder = layer.folder;
        let layer_id = layer.id;
        let mask_target = selected && !self.input_stroke_target().is_raster();
        for badge in [symmetry_response, guides_response].into_iter().flatten() {
            egui::Popup::from_response(&badge)
                .open_memory(badge.clicked().then_some(egui::SetOpenCommand::Bool(true)))
                .layout(egui::Layout::top_down(egui::Align::Min))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.layer_guides_popup(ui, index));
        }
        // A primary click can also reach the containing row. Its context-menu
        // handler would close the popup the badge has just opened.
        if badge_clicked {
            return;
        }
        response
            .on_hover_text(
                "Click to select · double-click to rename · drag to reorder or drop onto a folder · right-click for more",
            )
            .context_menu(|ui| {
                ui.menu_button("Symmetry and guides", |ui| self.layer_guides_popup(ui, index));
                ui.separator();
                if ui.button("Rename").clicked() {
                    *action = Some(RowAction::Rename(index));
                    ui.close();
                }
                if ui
                    .add(egui::Button::new("Duplicate").shortcut_text("Ctrl+J"))
                    .clicked()
                {
                    *action = Some(RowAction::Duplicate(index));
                    ui.close();
                }
                let merge = if folder { "Merge folder" } else { "Merge down" };
                if ui
                    .add(egui::Button::new(merge).shortcut_text("Ctrl+E"))
                    .clicked()
                {
                    *action = Some(RowAction::Merge(index));
                    ui.close();
                }
                ui.separator();
                if ui
                    .add(egui::Button::new("Put in new folder").shortcut_text("Ctrl+G"))
                    .clicked()
                {
                    *action = Some(RowAction::Group(index));
                    ui.close();
                }
                if folder
                    && ui
                        .add(egui::Button::new("Ungroup folder").shortcut_text("Ctrl+Shift+G"))
                        .clicked()
                {
                    *action = Some(RowAction::Ungroup(index));
                    ui.close();
                }
                if ui.button("Move up").clicked() {
                    *action = Some(RowAction::Step(index, true));
                    ui.close();
                }
                if ui.button("Move down").clicked() {
                    *action = Some(RowAction::Step(index, false));
                    ui.close();
                }
                ui.separator();
                let mut shown = !self.preview.hidden.contains(&layer_id);
                if ui
                    .checkbox(&mut shown, "Show in preview window")
                    .on_hover_text("View › Preview window shows the artwork on another screen or in OBS")
                    .changed()
                {
                    *action = Some(RowAction::TogglePreview(index));
                    ui.close();
                }
                if (!folder || mask_target) && ui.button(if mask_target { "Clear mask" } else { "Clear layer" }).clicked() {
                    *action = Some(RowAction::Clear(index));
                    ui.close();
                }
                if ui
                    .button(if folder {
                        "Delete folder and contents"
                    } else {
                        "Delete layer"
                    })
                    .clicked()
                {
                    *action = Some(RowAction::Delete(index));
                    ui.close();
                }
            });
    }
}

#[cfg(test)]
mod tests {
    fn pending_reordered_rows() -> (
        crate::Dudel,
        crate::document::LayerId,
        crate::document::LayerId,
    ) {
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(32, 32),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 255]);
        app.doc.add_layer();
        app.doc.layers[1]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[201, 17, 83, 255]);
        app.doc.active = 0;
        let a = app.doc.layers[0].id;
        let b = app.doc.layers[1].id;
        app.render_queue.redo().unwrap();
        app.reorder_canvas_layer(0, 2, 0);
        (app, a, b)
    }
    #[test]
    fn sidebar_clear_rename_visibility_and_opacity_keep_identity_across_reorder_fences() {
        let (mut app, a, b) = pending_reordered_rows();
        app.apply_row_action(Some(super::RowAction::Clear(0)));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].id, b);
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [201, 17, 83, 255]);
        assert_eq!(app.doc.layers[1].id, a);
        assert_eq!(app.doc.layers[1].pixels.rgba(0), [0; 4]);

        let (mut app, a, b) = pending_reordered_rows();
        app.apply_row_action(Some(super::RowAction::Rename(0)));
        assert_eq!(app.renaming, Some(1));
        app.rename_draft = "Captured A".into();
        app.finish_layer_rename(true);
        assert_eq!(app.doc.layers[1].id, a);
        assert_eq!(app.doc.layers[1].name, "Captured A");
        assert_eq!(app.doc.layers[0].id, b);
        assert_ne!(app.doc.layers[0].name, "Captured A");

        let (mut app, a, b) = pending_reordered_rows();
        app.apply_row_action(Some(super::RowAction::ToggleVisible(0)));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, a);
        assert!(!app.doc.layers[1].visible);
        assert_eq!(app.doc.layers[0].id, b);
        assert!(app.doc.layers[0].visible);

        let (mut app, a, b) = pending_reordered_rows();
        app.set_layer_opacity(0, 0.5);
        app.finish_opacity_edit();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, a);
        assert_eq!(app.doc.layers[1].opacity, 0.5);
        assert_eq!(app.doc.layers[0].id, b);
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        app.undo();
        assert_eq!(app.doc.layers[1].opacity, 1.0);
    }
    #[test]
    fn sidebar_property_and_merge_fences_resolve_the_captured_layer() {
        let (mut app, a, b) = pending_reordered_rows();
        assert!(app.set_canvas_property(a, crate::document_edits::Property::AlphaLocked(true)));
        assert!(!app.doc.layers[0].alpha_locked);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, a);
        assert!(app.doc.layers[1].alpha_locked);
        assert_eq!(app.doc.layers[0].id, b);
        assert!(!app.doc.layers[0].alpha_locked);

        let (mut app, _, b) = pending_reordered_rows();
        let mut expected = app.doc.clone();
        expected.move_block(0, 2, 0).unwrap();
        expected.merge_down().unwrap();
        app.apply_row_action(Some(super::RowAction::Merge(0)));
        assert_eq!(app.doc.layers.len(), 2);
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].id, b);
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
    }
    use super::RowAction;
    #[test]
    fn recovery_waits_for_committed_opacity_instead_of_serializing_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.gz");
        let mut app = Dudel {
            doc: crate::document::Document::new(1, 1),
            recovery_path: Some(path.clone()),
            recovery_timer: std::time::Instant::now() - std::time::Duration::from_secs(16),
            ..Default::default()
        };
        app.set_layer_opacity(0, 0.4);
        let ctx = crate::egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
        assert!(app.recovery_job.is_none());
        app.finish_opacity_edit();
        let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
        let job = app.recovery_job.take().unwrap();
        assert_eq!(job.join().unwrap().unwrap(), app.revision);
        assert_eq!(
            crate::storage::read_recovery(&path)
                .unwrap()
                .document
                .layers[0]
                .opacity,
            0.4
        );
    }
    #[test]
    fn save_finishes_opacity_and_writes_the_committed_value_and_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opacity.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(1, 1),
            path: Some(path.clone()),
            ..Default::default()
        };
        app.set_layer_opacity(0, 0.4);
        app.save(false);
        assert!(app.opacity_edit.is_none() && !app.history.has_pending_stroke());
        let job = app.save_job.take().unwrap();
        assert_eq!(job.revision, app.revision);
        assert!(job.worker.join().unwrap().is_ok());
        assert_eq!(
            crate::storage::load_project(&path).unwrap().layers[0].opacity,
            0.4
        );
        app.undo();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        app.redo();
        assert_eq!(app.doc.layers[0].opacity, 0.4);
    }
    #[test]
    fn opacity_noop_and_return_to_original_preserve_redo_and_unsaved_state() {
        let mut app = Dudel::default();
        app.new_layer();
        app.undo();
        app.modified = false;
        let opacity = app.doc.layers[0].opacity;
        app.set_layer_opacity(0, opacity);
        app.finish_opacity_edit();
        assert!(!app.history.can_undo());
        assert!(app.history.can_redo());
        assert!(!app.modified);
        let revision = app.revision;
        app.set_layer_opacity(0, 0.3);
        assert!(app.opacity_edit.is_some() && app.history.has_pending_stroke());
        app.set_layer_opacity(0, opacity);
        app.finish_opacity_edit();
        assert!(!app.history.can_undo() && app.history.can_redo());
        assert!(!app.modified);
        assert_ne!(
            app.revision, revision,
            "preview epochs must remain monotonic"
        );
        app.redo();
        assert_eq!(app.doc.layers.len(), 2);
    }
    #[test]
    fn a_changed_opacity_gesture_commits_one_action_and_switching_targets_finishes_it() {
        let mut app = Dudel::default();
        app.doc.add_layer();
        app.set_layer_opacity(0, 0.9);
        app.set_layer_opacity(0, 0.6);
        app.set_layer_opacity(0, 0.4);
        app.apply_row_action(Some(RowAction::Select(1)));
        assert!(app.opacity_edit.is_none() && !app.history.has_pending_stroke());
        assert_eq!(app.doc.layers[0].opacity, 0.4);
        app.undo();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        assert!(!app.history.can_undo());
        app.redo();
        assert_eq!(app.doc.layers[0].opacity, 0.4);
    }
    #[test]
    fn opacity_settles_taper_and_layer_creation_keeps_separate_history_actions() {
        let (mut app, settled) = crate::edit_order_tests::pending_taper();
        app.set_layer_opacity(0, 0.4);
        assert!(!app.stroking);
        assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
        app.new_layer();
        assert_eq!(app.doc.layers.len(), 3);
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(app.doc.layers[0].opacity, 0.4);
        app.undo();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels.rgba(9 * 33 + 7), [0; 4]);
    }
    #[test]
    fn escape_cancels_a_rename_even_before_the_text_editor_has_focus() {
        let mut app = Dudel::default();
        let original = app.doc.layers[0].name.clone();
        app.apply_row_action(Some(RowAction::Rename(0)));
        app.rename_draft = "discard this draft".into();
        let ctx = crate::egui::Context::default();
        let _ = ctx.run(
            crate::egui::RawInput {
                events: vec![crate::egui::Event::Key {
                    key: crate::egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: crate::egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| {
                app.shortcuts(ctx);
                crate::egui::CentralPanel::default().show(ctx, |ui| {
                    app.layer_row(ui, 0, false, &mut None);
                });
            },
        );
        assert!(app.renaming.is_none());
        assert_eq!(app.doc.layers[0].name, original);
        assert!(!app.modified && !app.history.can_undo());
    }
    use crate::Dudel;
    #[test]
    fn layer_target_changes_finish_the_original_stroke_before_switching() {
        for action in [RowAction::Select(1), RowAction::Rename(1)] {
            let (mut app, expected) = crate::edit_order_tests::pending_taper();
            app.apply_row_action(Some(action));
            assert_eq!(app.doc.active, 1);
            assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(app.doc.layers[1].pixels.capacity(), 0);
            assert!(!app.history.has_pending_stroke());
            app.undo();
            assert_eq!(app.doc.active, 0);
            assert_eq!(app.doc.layers[0].pixels.rgba(9 * 33 + 7), [0; 4]);
        }
    }
    #[test]
    fn rename_commits_once_undoes_and_noop_or_cancel_preserve_redo() {
        let mut app = Dudel::default();
        let original = app.doc.layers[0].name.clone();
        app.apply_row_action(Some(RowAction::Rename(0)));
        app.rename_draft = "Sketch".into();
        assert_eq!(app.doc.layers[0].name, original);
        assert!(!app.modified && !app.history.can_undo());
        let revision = app.revision;
        app.finish_layer_rename(true);
        assert_eq!(app.doc.layers[0].name, "Sketch");
        assert!(app.modified && app.revision != revision);
        app.undo();
        assert_eq!(app.doc.layers[0].name, original);
        assert!(!app.history.can_undo());
        app.apply_row_action(Some(RowAction::Rename(0)));
        app.rename_draft = "discard".into();
        app.finish_layer_rename(false);
        assert_eq!(app.doc.layers[0].name, original);
        assert!(app.history.can_redo());
        let revision = app.revision;
        app.apply_row_action(Some(RowAction::Rename(0)));
        app.finish_layer_rename(true);
        assert_eq!(app.revision, revision);
        assert!(app.history.can_redo());
        app.redo();
        assert_eq!(app.doc.layers[0].name, "Sketch");
    }
    #[test]
    fn new_layer_commits_a_name_draft_before_the_structure_action() {
        let mut app = Dudel::default();
        let original = app.doc.layers[0].name.clone();
        app.apply_row_action(Some(RowAction::Rename(0)));
        app.rename_draft = "Ink".into();
        app.new_layer();
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(app.doc.layers[0].name, "Ink");
        app.undo();
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].name, "Ink");
        app.undo();
        assert_eq!(app.doc.layers[0].name, original);
    }
    #[test]
    fn collapse_preserves_active_target_in_history_and_increments_save_revision() {
        let mut app = Dudel::default();
        app.group_into_folder();
        app.doc.active = 0;
        app.dirty = false;
        let revision = app.revision;
        app.apply_row_action(Some(RowAction::ToggleCollapsed(1)));
        assert!(app.doc.layers[1].collapsed);
        assert_eq!(app.doc.active, 1);
        assert_ne!(app.revision, revision);
        assert!(
            !app.dirty,
            "folder panel state must not recomposite artwork"
        );
        app.undo();
        assert!(!app.doc.layers[1].collapsed);
        assert_eq!(app.doc.active, 0);
        app.redo();
        assert!(app.doc.layers[1].collapsed);
        assert_eq!(app.doc.active, 1);
    }
    #[test]
    fn whole_layer_clear_preserves_selection_through_undo_redo() {
        let (mut app, settled) = crate::edit_order_tests::pending_taper();
        app.cancel_gestures();
        let selection = crate::selection::Selection {
            left: 20,
            top: 20,
            right: 25,
            bottom: 25,
        };
        app.set_selection(Some(selection));
        app.apply_row_action(Some(RowAction::Clear(0)));
        assert_eq!(app.doc.selection, Some(selection));
        assert_eq!(app.doc.layers[0].pixels.rgba(9 * 33 + 7), [0; 4]);
        app.undo();
        assert_eq!(app.doc.selection, Some(selection));
        assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
        app.redo();
        assert_eq!(app.doc.selection, Some(selection));
    }
    #[test]
    fn layer_commands_insert_above_reorder_and_undo_as_single_steps() {
        let mut app = Dudel::default();
        app.new_layer();
        app.new_layer();
        assert_eq!(app.doc.layers.len(), 3);
        app.doc.active = 0;
        app.new_layer();
        assert_eq!(
            app.doc.active, 1,
            "new layers go directly above the active one"
        );
        let names: Vec<String> = app.doc.layers.iter().map(|l| l.name.clone()).collect();
        app.reorder_layer(1, 4, 0);
        assert_eq!(app.doc.layers[3].name, names[1]);
        assert_eq!(app.doc.active, 3);
        app.undo();
        let restored: Vec<String> = app.doc.layers.iter().map(|l| l.name.clone()).collect();
        assert_eq!(restored, names);
        app.duplicate_layer();
        assert_eq!(app.doc.layers.len(), 5);
        app.merge_down();
        assert_eq!(app.doc.layers.len(), 4);
        app.undo();
        assert_eq!(app.doc.layers.len(), 5);
        app.doc.layers[app.doc.active].locked = true;
        app.delete_layer();
        assert_eq!(app.doc.layers.len(), 5);
    }
    #[test]
    fn folder_commands_nest_new_layers_and_undo_in_one_step() {
        let mut app = Dudel::default();
        app.group_into_folder();
        assert!(app.doc.layers[1].folder);
        assert_eq!(app.doc.layers[0].depth, 1);
        app.new_layer();
        assert_eq!(
            app.doc.active, 1,
            "new layer becomes the open folder's top layer"
        );
        assert_eq!(app.doc.layers[1].depth, 1);
        app.doc.active = 2;
        app.merge_down();
        assert_eq!(app.doc.layers.len(), 1);
        assert!(!app.doc.layers[0].folder);
        app.undo();
        assert_eq!(app.doc.layers.len(), 3);
        app.doc.active = 2;
        app.ungroup_folder();
        assert!(app.doc.layers.iter().all(|l| !l.folder && l.depth == 0));
        assert!(app.doc.validate().is_ok());
    }
}
