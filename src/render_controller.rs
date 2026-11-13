//! Application ownership boundary for resumable canvas rendering. Existing
//! synchronous document actions currently flush here before mutating the document;
//! they must eventually become ordered asynchronous commands too.
use crate::{Dudel, render_queue::Progress, stroke_buffer::Settings};
use std::time::{Duration, Instant};

/// Queued dabs at which painting takes a larger share of each frame.
const PAINT_CATCH_UP_DABS: usize = 3;
/// Queued dabs at which painting takes a longer turn still, and at which
/// it takes most of each frame: the line is visibly behind the pen and
/// falling further back, and showing it sooner matters more than showing it
/// often. Frames then come about 60, and 30 to 45, times a second until the
/// queue is short again. A pen that felt heavy at 40 frames a second with
/// 16 dabs waiting (2026-11-13) is why the long turn starts this late.
const PAINT_BEHIND_DABS: usize = 16;
const PAINT_FAR_BEHIND_DABS: usize = 64;

pub struct EditFeedback {
    pub changed: &'static str,
    pub unchanged: &'static str,
}

/// Captured at FIFO execution, after preceding edits have been accepted.
pub(crate) struct StrokePreview {
    id: crate::render_queue::StrokeId,
    modified: bool,
    content_revision: u64,
}

/// Persistence baseline of a live Move/Transform, kept through FIFO settlement.
pub(crate) struct GeometryPreview {
    modified: bool,
    content_revision: u64,
}

impl Dudel {
    pub(crate) fn begin_geometry_preview(&mut self) {
        self.track_guide_changes();
        self.geometry_preview = Some(GeometryPreview {
            modified: self.modified,
            content_revision: self.content_revision,
        });
    }
    pub(crate) fn geometry_preview_changed(&mut self) {
        if self.geometry_preview.is_none() {
            self.changed();
            return;
        }
        self.sync_canvas_frame();
        self.dirty = true;
        self.dirty_region = Default::default();
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
    }
    /// Return whether a captured baseline handled persistence settlement.
    pub(crate) fn finish_geometry_preview(&mut self, changed: bool) -> bool {
        let Some(preview) = self.geometry_preview.take() else {
            return false;
        };
        self.track_guide_changes();
        if changed {
            self.metadata_changed();
        } else if self.saved_content_revision == Some(self.content_revision)
            && !self.guides_changed()
        {
            self.modified = false;
            self.clear_recovery_snapshot();
        } else {
            self.modified = preview.modified
                || self.content_revision != preview.content_revision
                || self.guides_changed();
        }
        true
    }
    pub(crate) fn input_properties(&self) -> Option<crate::pending_layers::Properties> {
        let target = self.input_target()?;
        if let Some(context) = &self.pending_layers {
            context.properties(target)
        } else {
            self.doc
                .layers
                .iter()
                .find(|layer| layer.id == target)
                .map(crate::pending_layers::Properties::from_layer)
        }
    }
    pub(crate) fn set_canvas_property(
        &mut self,
        target: crate::document::LayerId,
        property: crate::document_edits::Property,
    ) -> bool {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(active) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return false;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, active) = match context.with_property(target, active, &property) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let accepted = self.dispatch_document_edit(
            crate::document_edits::Edit::Property {
                target: crate::document_edits::Target::Identity(target),
                property,
            },
            "Layer updated",
            "Layer unchanged",
        );
        if accepted && self.render_queue.busy() {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(active);
        }
        accepted
    }
    pub(crate) fn toggle_canvas_layer(&mut self, index: usize, collapse: bool) {
        let Some(target) = self.doc.layers.get(index).map(|layer| layer.id) else {
            return;
        };
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(active) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, value, active) = match context.with_toggle(target, active, collapse) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let property = if collapse {
            crate::document_edits::Property::Collapsed(value)
        } else {
            crate::document_edits::Property::Visible(value)
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::Property {
                target: crate::document_edits::Target::Identity(target),
                property,
            },
            "Layer updated",
            "Layer unchanged",
        ) && self.render_queue.busy()
        {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(active);
        }
    }
    pub(crate) fn step_canvas_layer(&mut self, up: bool) {
        if self.multiple_layers() {
            self.batch_layers(crate::layer_batch::Action::Step(up));
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        if !self.render_queue.busy() {
            self.dispatch_document_edit(
                crate::document_edits::Edit::Step(up),
                "Layer moved",
                "Layer unchanged",
            );
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_step(target, up) {
            Ok(context) => context,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::StepTarget { target, up },
            "Layer moved",
            "Layer unchanged",
        ) {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(target);
        }
    }
    pub(crate) fn merge_canvas_layer(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let changed = if self.input_is_folder() {
            "Folder merged into one layer"
        } else {
            "Merged into the layer below"
        };
        if !self.render_queue.busy() {
            self.dispatch_document_edit(
                crate::document_edits::Edit::Merge,
                changed,
                "Layer unchanged",
            );
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, active, folder_result) = match context.with_merge(target) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::MergeTarget {
                target,
                folder_result,
            },
            changed,
            "Layer unchanged",
        ) {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(active);
        }
    }
    pub(crate) fn duplicate_canvas_layer(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let mask_focused = !self.input_stroke_target().is_raster();
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, copies, active) = match context.with_duplicate(target) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let symmetries: Vec<_> = context
            .block_ids(target)
            .expect("duplicate projection validated the source block")
            .into_iter()
            .zip(&copies)
            .filter_map(|(source, copy)| {
                self.layer_symmetry
                    .get(&source)
                    .map(|symmetry| (*copy, *symmetry))
            })
            .collect();
        if self.dispatch_document_edit(
            crate::document_edits::Edit::DuplicateTarget { target, copies },
            "Duplicated",
            "Layer unchanged",
        ) {
            if mask_focused {
                self.mask_focus = projected
                    .properties(active)
                    .and_then(|p| p.mask)
                    .map(|m| (active, m.id));
                self.last_stroke_end = None;
            }
            self.layer_symmetry.extend(symmetries);
            if self.render_queue.busy() {
                self.pending_layers = Some(projected);
                self.logical_layer = Some(active);
            }
        }
    }
    pub(crate) fn delete_canvas_layer(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        if !self.render_queue.busy() {
            self.dispatch_document_edit(
                crate::document_edits::Edit::Delete,
                "Deleted",
                "Layer unchanged",
            );
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, active) = match context.with_delete(target) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::DeleteTarget(target),
            "Deleted",
            "Layer unchanged",
        ) {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(active);
        }
    }
    pub(crate) fn input_is_folder(&self) -> bool {
        if let (Some(context), Some(id)) = (&self.pending_layers, self.input_target()) {
            context.is_folder(id)
        } else {
            self.input_layer()
                .is_some_and(|index| self.doc.layers[index].folder)
        }
    }
    pub(crate) fn create_canvas_folder(&mut self, group: bool) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let changed = if group {
            "Layer placed in a new folder"
        } else {
            "New folder"
        };
        if !self.render_queue.busy() {
            self.dispatch_document_edit(
                if group {
                    crate::document_edits::Edit::Group
                } else {
                    crate::document_edits::Edit::NewFolder
                },
                changed,
                "Folder unchanged",
            );
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let folder = crate::document::LayerId::new();
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_folder(target, folder, group) {
            Ok(projected) => projected,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::FolderTarget {
                target,
                folder,
                group,
            },
            changed,
            "Folder unchanged",
        ) {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(folder);
        }
    }
    pub(crate) fn ungroup_canvas_folder(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        if !self.render_queue.busy() {
            self.dispatch_document_edit(
                crate::document_edits::Edit::Ungroup,
                "Folder removed; its layers were kept",
                "Folder unchanged",
            );
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, active) = match context.with_ungroup(target) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::UngroupTarget(target),
            "Folder removed; its layers were kept",
            "Folder unchanged",
        ) {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(active);
        }
    }
    pub(crate) fn input_target(&self) -> Option<crate::document::LayerId> {
        let id = self
            .logical_layer
            .unwrap_or(self.doc.layers[self.doc.active].id);
        if self.doc.layers.iter().any(|layer| layer.id == id) {
            Some(id)
        } else if self
            .pending_layers
            .as_ref()
            .is_some_and(|context| context.contains(id))
        {
            // The context verifies existence/visibility when painting begins;
            // the renderer independently rejects a failed creation dependency.
            Some(id)
        } else {
            None
        }
    }
    pub(crate) fn input_paint_blocker(&self) -> Option<&'static str> {
        if let (Some(context), Some(id)) = (&self.pending_layers, self.input_target()) {
            context.paint_blocker(id)
        } else if let Some(index) = self.input_layer() {
            self.doc.paint_blocker_for(index)
        } else {
            Some("Selected layer no longer exists")
        }
    }
    pub(crate) fn input_alpha_locked(&self) -> bool {
        if let (Some(context), Some(id)) = (&self.pending_layers, self.input_target()) {
            context.alpha_locked(id)
        } else if let Some(index) = self.input_layer() {
            self.doc.layers[index].alpha_locked
        } else {
            false
        }
    }
    pub(crate) fn create_canvas_layer(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        if !self.render_queue.busy() {
            self.dispatch_document_edit(
                crate::document_edits::Edit::NewLayer,
                "New layer",
                "No layer created",
            );
            return;
        }
        let Some(active) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let id = crate::document::LayerId::new();
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_new_layer(active, id) {
            Ok(projected) => projected,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::NewLayerWithId(id),
            "New layer",
            "No layer created",
        ) {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(id);
        }
    }
    /// A new adjustment layer above the selected entry, selected afterwards.
    pub(crate) fn create_adjustment_layer(&mut self, settings: crate::adjustment::Settings) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(active) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let id = crate::document::LayerId::new();
        let edit = crate::document_edits::Edit::NewAdjustment { id, settings };
        if !self.render_queue.busy() {
            self.dispatch_document_edit(edit, "New adjustment layer", "No layer created");
            return;
        }
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_new_layer(active, id) {
            Ok(projected) => projected,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(edit, "New adjustment layer", "No layer created") {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(id);
        }
    }
    pub(crate) fn reorder_canvas_layer(&mut self, from: usize, gap: usize, depth: u8) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(source) = self.doc.layers.get(from) else {
            self.status = "Moved layer no longer exists".into();
            return;
        };
        if gap > self.doc.layers.len() {
            self.status = "Invalid layer destination".into();
            return;
        }
        let id = source.id;
        let before = self.doc.layers.get(gap).map(|layer| layer.id);
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let (projected, changed) = match context.with_reorder(id, before, depth) {
            Ok(result) => result,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(
            crate::document_edits::Edit::ReorderTarget {
                from: id,
                before,
                depth,
            },
            "Layer moved",
            "Layer order unchanged",
        ) && self.render_queue.busy()
        {
            self.pending_layers = Some(projected);
            if changed {
                self.logical_layer = Some(id);
            }
        }
    }
    pub(crate) fn input_layer(&self) -> Option<usize> {
        if let Some(id) = self.logical_layer {
            self.doc.layers.iter().position(|layer| layer.id == id)
        } else {
            Some(self.doc.active)
        }
    }
    pub(crate) fn select_layer_target(&mut self, index: usize) -> bool {
        let Some(layer) = self.doc.layers.get(index) else {
            self.status = "Selected layer no longer exists".into();
            return false;
        };
        let id = layer.id;
        let changed_target = self.input_target() != Some(id);
        if self
            .pending_layers
            .as_ref()
            .is_some_and(|context| !context.contains(id))
        {
            self.status = "Selected layer no longer exists".into();
            return false;
        }
        let accepted = self.dispatch_document_edit(
            crate::document_edits::Edit::SelectTarget(crate::document_edits::Target::Identity(id)),
            "Layer selected",
            "Layer already selected",
        );
        if accepted && self.render_queue.busy() {
            self.logical_layer = Some(id);
        }
        if accepted && changed_target {
            self.mask_focus = None;
            self.last_stroke_end = None;
        }
        accepted
    }
    pub(crate) fn dispatch_document_edit(
        &mut self,
        edit: crate::document_edits::Edit,
        changed: &'static str,
        unchanged: &'static str,
    ) -> bool {
        if matches!(
            &edit,
            crate::document_edits::Edit::Transform(_)
                | crate::document_edits::Edit::MoveTarget { .. }
                | crate::document_edits::Edit::TransformTarget { .. }
                | crate::document_edits::Edit::AffineTarget { .. }
                | crate::document_edits::Edit::AffineSettingsTarget { .. }
                | crate::document_edits::Edit::FillTarget { .. }
                | crate::document_edits::Edit::Clear { .. }
                | crate::document_edits::Edit::ClearTarget { .. }
        ) && !self.require_artwork_target()
        {
            return false;
        }
        if self.adjustment.is_some() || self.adjustment_ticket.is_some() {
            return false;
        }
        self.close_input_gestures();
        if self.render_queue.busy()
            || edit
                .background_label(
                    self.doc.selection_mask.is_some()
                        || self.doc.layers.iter().any(|l| l.mask.is_some()),
                )
                .is_some()
        {
            match self.render_queue.edit(edit) {
                Ok(ticket) => {
                    self.edit_feedback
                        .insert(ticket, EditFeedback { changed, unchanged });
                    self.status = "Edit queued after preceding artwork".into();
                    true
                }
                Err(error) => {
                    self.status = error;
                    false
                }
            }
        } else {
            match edit.prepare(&self.doc) {
                Ok((updated, outcome)) => {
                    if outcome.changed {
                        if outcome.undoable {
                            self.history.checkpoint(&self.doc);
                        }
                        self.doc = updated;
                        if outcome.undoable {
                            self.history.settle(&self.doc);
                        }
                        if outcome.redraw {
                            self.changed();
                        } else if outcome.undoable {
                            self.metadata_changed();
                        }
                    }
                    self.status = if outcome.changed { changed } else { unchanged }.into();
                    true
                }
                Err(error) => {
                    self.status = error;
                    false
                }
            }
        }
    }
    pub(crate) fn begin_canvas_stroke(&mut self, mut settings: Settings) -> bool {
        if self.canvas_edit_ticket.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
        {
            self.status = "Wait for the canvas bounds change to finish".into();
            return false;
        }
        self.finish_opacity_edit();
        if self.replay.enabled && (self.doc.version == 2 || !settings.target.is_raster()) {
            self.status = "Stroke recording does not yet support layer-mask documents".into();
            return false;
        }
        let blocker = self.stroke_target_blocker(settings.target, settings.erase);
        if let Some(blocker) = blocker {
            self.status = blocker.into();
            return false;
        }
        if self.replay.enabled {
            // Replay recording currently snapshots the immediate source/result.
            // Keep that explicit diagnostic mode synchronous until captures can
            // be attached to individual queued stroke lifetimes.
            self.flush_render_queue();
            if self.doc.version == 2 {
                self.status = "Stroke recording does not yet support layer-mask documents".into();
                return false;
            }
            if let Some(blocker) = self.doc.paint_blocker() {
                self.status = blocker.into();
                return false;
            }
            settings.layer = self.doc.active;
            self.stroke_buffer = Some(Default::default());
            self.replay.begin(&self.doc, &settings);
            self.history.begin_stroke_checkpoint(&self.doc);
        } else {
            let Some(target) = self.input_target() else {
                self.status = "Selected layer no longer exists".into();
                return false;
            };
            settings.layer = self.input_layer().unwrap_or(self.doc.active);
            match self
                .render_queue
                .begin_layer(std::sync::Arc::new(settings.clone()), target)
            {
                Ok(id) => self.queued_stroke = Some(id),
                Err(error) => {
                    self.status = error;
                    return false;
                }
            }
        }
        self.stroke_settings = Some(settings);
        self.stroke_previous_end = self.last_stroke_end;
        self.stroking = true;
        true
    }

    pub(crate) fn deliver_canvas_dabs(&mut self, dabs: Vec<crate::stroke::Sample>) {
        if let Some(id) = self.queued_stroke {
            for dab in dabs {
                if let Err(error) = self.render_queue.dab(id, dab) {
                    // dab() already cancels this contact on overflow. Earlier
                    // completed contacts remain in FIFO order.
                    self.last_stroke_end = self.stroke_previous_end;
                    self.queued_stroke = None;
                    self.stroke_settings = None;
                    self.stroke.reset();
                    self.stroking = false;
                    self.status = error;
                    break;
                }
            }
        } else if let (Some(buffer), Some(settings)) =
            (&mut self.stroke_buffer, &self.stroke_settings)
        {
            let emitted = !dabs.is_empty();
            for dab in dabs {
                let painted = buffer.paint(&mut self.doc, dab, settings);
                self.performance.dabs += painted.dabs;
                self.dirty_region.merge(painted.region);
            }
            if emitted {
                self.metadata_changed();
            }
        }
    }

    fn accept_render_progress(&mut self, progress: Progress) -> bool {
        for (maximum, timing) in self
            .performance
            .phase_max_ms
            .iter_mut()
            .zip(&progress.timings)
        {
            *maximum = maximum.max(timing.max_ms);
        }
        self.sync_canvas_frame();
        self.performance.dabs += progress.dabs;
        if let Some(id) = progress.started {
            self.stroke_preview = Some(StrokePreview {
                id,
                modified: self.modified,
                content_revision: self.content_revision,
            });
        }
        if let Some(event) = progress.opacity {
            if event.started {
                self.opacity_preview_modified = Some((event.ticket, self.modified));
            }
            if event.ended {
                if let Some((ticket, modified)) = self.opacity_preview_modified.take()
                    && ticket == event.ticket
                    && !event.changed
                {
                    self.modified = modified;
                }
            } else if event.changed {
                self.changed();
            }
            if let Some(error) = event.error {
                self.status = error;
            }
        }
        if !progress.damage.is_empty() {
            self.dirty_region.merge(progress.damage);
            if self.stroke_preview.is_some() || self.geometry_preview.is_some() {
                // Invalidate live displays and stale workers, but do not make
                // an unconfirmed contact a committed persistence revision.
                self.modified = true;
                self.revision = self.revision.wrapping_add(1);
            } else {
                self.metadata_changed();
            }
        }
        for completed in progress.completed {
            if self
                .stroke_preview
                .as_ref()
                .is_some_and(|p| p.id == completed.id)
            {
                let preview = self.stroke_preview.take().unwrap();
                if completed.changed {
                    self.metadata_changed();
                } else if self.saved_content_revision == Some(self.content_revision)
                    && !self.guides_changed()
                {
                    self.modified = false;
                    self.clear_recovery_snapshot();
                } else {
                    self.modified = preview.modified
                        || self.content_revision != preview.content_revision
                        || self.guides_changed();
                }
            }
        }
        if let Some(movement) = progress.move_done {
            let settled = self.finish_geometry_preview(movement.changed);
            if let Some(error) = movement.error {
                self.status = error;
            } else {
                if !movement.changed && !settled {
                    self.modified = movement.modified_before;
                }
                self.status = if movement.affine {
                    match (movement.canceled, movement.changed) {
                        (true, _) => "Transform canceled",
                        (false, true) => "Transform applied",
                        (false, false) => "Transform made no changes",
                    }
                    .into()
                } else if movement.canceled {
                    "Move canceled".into()
                } else if movement.changed {
                    format!("Moved by {}, {} px", movement.offset[0], movement.offset[1])
                } else {
                    "Move made no changes".into()
                };
            }
        }
        for (undo, changed) in progress.history_actions {
            self.status = match (undo, changed) {
                (true, true) => "Undone",
                (false, true) => "Redone",
                (true, false) => "Nothing to undo",
                (false, false) => "Nothing to redo",
            }
            .into();
        }
        for (ticket, result) in progress.edits {
            if self.adjustment_ticket == Some(ticket) {
                self.adjustment_ticket = None;
            }
            let canvas_edit = self.canvas_edit_ticket == Some(ticket);
            if canvas_edit {
                self.canvas_edit_ticket = None;
            }
            if self.deferred_move_ticket == Some(ticket) {
                self.deferred_move_ticket = None;
            }
            let feedback = self.edit_feedback.remove(&ticket);
            match result {
                Ok(outcome) => {
                    // Metadata/selection commands have no raster damage but
                    // still advance the document revision before a snapshot.
                    if outcome.changed && outcome.undoable {
                        self.metadata_changed();
                    }
                    if let Some(feedback) = feedback {
                        self.status = if outcome.changed {
                            feedback.changed
                        } else {
                            feedback.unchanged
                        }
                        .into();
                    }
                    if canvas_edit
                        && outcome.changed
                        && self.doc.selection.is_none()
                        && self.doc.selection_mask.is_some()
                    {
                        self.status
                            .push_str(" · selection is empty (deselect to continue)");
                    }
                }
                Err(error) => self.status = error,
            }
        }
        if let Some(error) = progress.render_error.or(progress.history_error) {
            self.status = error;
        }
        if let Some((ticket, snapshot)) = progress.snapshot {
            if self
                .queued_save
                .as_ref()
                .is_some_and(|save| save.ticket == ticket)
            {
                let save = self.queued_save.take().unwrap();
                if save.canceled {
                    self.status = "Project save canceled; artwork remains open".into();
                } else {
                    self.start_save_snapshot(save.path, snapshot);
                }
            } else if self
                .queued_export
                .as_ref()
                .is_some_and(|export| export.ticket == ticket)
            {
                let export = self.queued_export.take().unwrap();
                if export.canceled {
                    self.status = "Export canceled".into();
                } else {
                    self.start_export_snapshot(export.path, export.kind, snapshot);
                }
            } else {
                self.status = "Unrecognized rendering snapshot ticket".into();
            }
        }
        if let Some(ticket) = progress.barrier {
            self.complete_clipboard(ticket);
        }
        if !self.render_queue.busy() {
            self.logical_layer = None;
            self.pending_layers = None;
            // A save can finish while a later export/snapshot fence is still
            // queued. Settling that read-only work does not make newer edits.
            if self.modified
                && self.saved_content_revision == Some(self.content_revision)
                && self.stroke_preview.is_none()
                && self.geometry_preview.is_none()
                && !self.stroking
                && !self.guides_changed()
            {
                self.modified = false;
                self.clear_recovery_snapshot();
            }
        }
        self.reconcile_mask_focus();
        progress.work_units != 0
    }

    pub(crate) fn pump_render_queue(&mut self, ctx: &crate::egui::Context) {
        let _timed = crate::stage_timer::scope(crate::stage_timer::Stage::Pump);
        let started = Instant::now();
        let had_work = self.render_queue.needs_work();
        self.render_queue.set_profiling(self.performance.open);
        if had_work {
            self.performance.note_drawing();
        }
        // Painting gets 2 ms a frame, more while it trails the pen (large
        // brushes), so strokes catch up instead of drawing in slowly. The
        // rest of a frame costs the same however long painting had, so far
        // behind the pen a longer turn gets more painted per second.
        let trailing = self.render_queue.queued_dabs(PAINT_FAR_BEHIND_DABS);
        let budget = if trailing >= PAINT_FAR_BEHIND_DABS {
            20
        } else if trailing >= PAINT_BEHIND_DABS {
            12
        } else if trailing >= PAINT_CATCH_UP_DABS {
            8
        } else {
            2
        };
        let deadline = started + Duration::from_millis(budget);
        let mut remaining = 4096;
        while self.render_queue.needs_work() && remaining > 0 && Instant::now() < deadline {
            match self.render_queue.advance_until(
                &mut self.doc,
                &mut self.history,
                remaining,
                deadline,
            ) {
                Ok(progress) => {
                    remaining = remaining.saturating_sub(progress.work_units);
                    // Preserve observable snapshot/clipboard fences; only the
                    // gesture bookkeeping boundaries may continue this pump.
                    let fence = progress.snapshot.is_some() || progress.barrier.is_some();
                    if !self.accept_render_progress(progress) || fence {
                        break;
                    }
                }
                Err(error) => {
                    self.status = error;
                    break;
                }
            }
        }
        if had_work {
            self.performance
                .drawing
                .record(started.elapsed().as_secs_f64() * 1000.0);
        }
        if self.render_queue.needs_work() || self.clipboard_settlement.is_some() {
            ctx.request_repaint();
        } else if self.render_queue.waiting_edit()
            || self.paste_job.is_some()
            || self.copy_job.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }

    /// Transitional synchronous fence for document actions. Stroke-only input
    /// does not call this, so consecutive strokes can render over several frames.
    pub(crate) fn flush_render_queue(&mut self) {
        while self.render_queue.needs_work()
            || self.render_queue.waiting_edit()
            || self.paste_job.is_some()
            || self.copy_job.is_some()
            || self.clipboard_settlement.is_some()
        {
            if self.copy_job.is_some() {
                self.finish_copy_job(true);
            }
            if self.clipboard_settlement.is_some() {
                self.finish_clipboard_settlement();
            }
            if self.paste_job.is_some() {
                self.wait_paste();
            }
            if self.render_queue.waiting_edit() {
                self.render_queue.wait_edit();
            }
            match self
                .render_queue
                .advance(&mut self.doc, &mut self.history, 4096)
            {
                Ok(progress) => {
                    if !self.accept_render_progress(progress) {
                        break;
                    }
                }
                Err(error) => {
                    self.status = error;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::Document, egui, input::Packet, stroke::Sample, stroke_buffer::StrokeBuffer,
    };

    fn settings(color: [u8; 3]) -> Settings {
        Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 300.0,
                pressure_size: false,
                flow: 0.3,
                taper_out: 80.0,
                ..Default::default()
            },
            color,
            secondary: color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        }
    }
    fn point(x: f32, distance: f64) -> Sample {
        Sample {
            x,
            y: 170.5,
            pressure: 1.0,
            distance,
            ..Default::default()
        }
    }
    fn small_stroke_app(masked: bool) -> (Dudel, Settings, Sample) {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        let mut settings = settings([0; 3]);
        settings.brush.size = 8.0;
        settings.brush.taper_out = 0.0;
        if masked {
            app.doc.version = 2;
            app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: true,
                linked: true,
                coverage: crate::layer_mask::Mask::new(32, 32, 255).unwrap(),
            });
            settings.target =
                crate::paint_target::Target::Mask(app.doc.layers[0].mask.as_ref().unwrap().id);
        }
        (
            app,
            settings,
            Sample {
                y: 16.5,
                ..point(16.5, 0.0)
            },
        )
    }
    #[test]
    fn canceled_stroke_restores_saved_state_but_keeps_earlier_queued_edits_and_guides() {
        for masked in [false, true] {
            for prior_edit in [false, true] {
                for guide_edit in [false, true] {
                    let (mut app, settings, sample) = small_stroke_app(masked);
                    let before = app.doc.clone();
                    if prior_edit {
                        app.render_queue
                            .edit(crate::document_edits::Edit::Property {
                                target: crate::document_edits::Target::Active,
                                property: crate::document_edits::Property::Name(
                                    "Earlier edit".into(),
                                ),
                            })
                            .unwrap();
                    }
                    assert!(app.begin_canvas_stroke(settings));
                    app.deliver_canvas_dabs(vec![sample]);
                    app.flush_render_queue();
                    assert!(app.modified && app.stroke_preview.is_some());
                    assert_eq!(app.content_revision > 0, prior_edit);
                    let preview_revision = app.revision;
                    if guide_edit {
                        app.symmetry.mode = crate::symmetry::Mode::LeftRight;
                        app.track_guide_changes();
                    }
                    app.cancel_painted_stroke();
                    app.flush_render_queue();
                    assert!(app.stroke_preview.is_none());
                    assert_eq!(app.modified, prior_edit || guide_edit);
                    assert!(app.revision > preview_revision);
                    assert_eq!(app.doc.layers[0].pixels, before.layers[0].pixels);
                    assert!(
                        crate::layer_mask::equivalent_state(
                            &app.doc.layers[0].mask,
                            &before.layers[0].mask,
                            None,
                        )
                        .unwrap()
                    );
                    assert_eq!(app.history.can_undo(), prior_edit);
                }
            }
        }
    }
    #[test]
    fn no_op_stroke_keeps_redo_saved_state_and_committed_revision() {
        for masked in [false, true] {
            let (mut app, mut settings, sample) = small_stroke_app(masked);
            app.history.checkpoint(&app.doc);
            app.doc.layers[0].name = "Redo me".into();
            app.history.settle(&app.doc);
            assert!(app.history.undo(&mut app.doc));
            assert!(app.history.can_redo());
            if masked {
                settings.color = [255; 3];
            } else {
                settings.erase = true;
            }
            let before = app.doc.clone();
            assert!(app.begin_canvas_stroke(settings));
            app.deliver_canvas_dabs(vec![sample]);
            app.finish_painted_stroke();
            app.flush_render_queue();
            assert!(!app.modified && app.stroke_preview.is_none());
            assert_eq!(app.content_revision, 0);
            assert_eq!(app.doc.layers[0].pixels, before.layers[0].pixels);
            assert!(
                crate::layer_mask::equivalent_state(
                    &app.doc.layers[0].mask,
                    &before.layers[0].mask,
                    None,
                )
                .unwrap()
            );
            assert!(app.history.can_redo());
        }
    }
    #[test]
    fn canceled_contact_cannot_clear_a_following_queued_edit() {
        let (mut app, settings, sample) = small_stroke_app(true);
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(vec![sample]);
        app.flush_render_queue();
        app.cancel_painted_stroke();
        app.render_queue
            .edit(crate::document_edits::Edit::Property {
                target: crate::document_edits::Target::Active,
                property: crate::document_edits::Property::Name("Later edit".into()),
            })
            .unwrap();
        app.flush_render_queue();
        assert!(app.modified);
        assert_eq!(app.doc.layers[0].name, "Later edit");
        assert_eq!(app.content_revision, 1);
    }
    fn reference(doc: &mut Document, settings: &Settings, points: &[Sample]) {
        let mut buffer = StrokeBuffer::default();
        for &point in points {
            buffer.paint(doc, point, settings);
        }
        buffer.finish_tail(doc, settings);
        buffer.reclaim_empty_tiles(doc);
    }

    #[test]
    fn queued_opacity_previews_return_to_original_preserve_redo_and_modified_state() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.new_layer();
        app.undo();
        app.modified = false;
        let target = app.doc.layers[0].id;
        app.render_queue
            .edit(crate::document_edits::Edit::SelectLayer(0))
            .unwrap();
        app.set_target_opacity(target, 0.3);
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        assert!(!app.modified);
        app.flush_render_queue(); // Render preview, leave gesture open.
        assert_eq!(app.doc.layers[0].opacity, 0.3);
        assert!(app.modified && app.history.has_pending_stroke());
        assert!(app.render_queue.busy() && !app.render_queue.needs_work());
        app.set_target_opacity(target, 0.6);
        app.set_target_opacity(target, 1.0);
        app.finish_opacity_edit();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        assert!(!app.modified && !app.history.has_pending_stroke());
        assert!(app.history.can_redo() && !app.history.can_undo());
        assert!(app.opacity_preview_modified.is_none());
        app.redo();
        assert_eq!(app.doc.layers.len(), 2);
    }

    #[test]
    fn queued_paint_opacity_paint_matches_oracle_and_preserves_prior_modified_state() {
        for reverted in [false, true] {
            let mut app = Dudel {
                doc: Document::new(512, 512),
                ..Default::default()
            };
            let target = app.doc.layers[0].id;
            let first = settings([17, 83, 201]);
            let second = settings([201, 17, 83]);
            let a = [point(130.5, 0.0), point(180.5, 50.0)];
            let b = [point(150.5, 0.0), point(190.5, 40.0)];
            let mut expected = app.doc.clone();
            reference(&mut expected, &first, &a);
            let painted = expected.layers[0].pixels.clone();
            expected.layers[0].opacity = if reverted { 1.0 } else { 0.4 };
            reference(&mut expected, &second, &b);
            assert!(app.begin_canvas_stroke(first));
            app.deliver_canvas_dabs(a.to_vec());
            app.set_target_opacity(target, 0.7);
            assert!(app.opacity_edit.is_some());
            assert!(!app.history.has_pending_stroke()); // No synchronous painting/checkpoint.
            app.flush_render_queue();
            assert_eq!(app.doc.layers[0].pixels, painted);
            assert_eq!(app.doc.layers[0].opacity, 0.7);
            assert!(app.modified);
            app.set_target_opacity(target, expected.layers[0].opacity);
            app.finish_opacity_edit();
            app.flush_render_queue();
            assert!(
                app.modified,
                "a reverted preview must not hide prior queued painting"
            );
            assert!(app.begin_canvas_stroke(second));
            app.deliver_canvas_dabs(b.to_vec());
            app.finish_painted_stroke();
            app.flush_render_queue();
            assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(
                app.doc.composite_transparent(),
                expected.composite_transparent()
            );
            app.undo();
            assert_eq!(app.doc.layers[0].pixels, painted);
            if !reverted {
                app.undo();
                assert_eq!(app.doc.layers[0].opacity, 1.0);
            }
            app.undo();
            assert!(!app.history.can_undo());
        }
    }

    #[test]
    fn reserved_layer_opacity_gestures_switch_targets_without_draining() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        let original = app.doc.layers[0].id;
        app.render_queue.redo().unwrap();
        app.create_canvas_layer();
        let reserved = app.input_target().unwrap();
        app.set_target_opacity(reserved, 0.5);
        app.set_target_opacity(reserved, 0.4);
        app.set_target_opacity(original, 0.6); // Closes previous queued gesture.
        app.finish_opacity_edit();
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, reserved);
        assert_eq!(app.doc.layers[1].opacity, 0.4);
        assert_eq!(app.doc.layers[0].opacity, 0.6);
        app.undo();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        app.undo();
        assert_eq!(app.doc.layers[1].opacity, 1.0);
        app.undo();
        assert_eq!(app.doc.layers.len(), 1);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn queued_scalar_properties_control_following_paint_and_keep_individual_undo() {
        use crate::{blend::BlendMode, document_edits::Property};
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(170 * 512 + 150)
            .copy_from_slice(&[97, 53, 23, 255]);
        app.doc.add_layer();
        let target = app.doc.layers[1].id;
        let original = app.doc.clone();
        let mut first = settings([17, 83, 201]);
        first.layer = 1;
        first.brush.taper_out = 0.0;
        first.brush.flow = 1.0;
        let mut second = settings([201, 17, 83]);
        second.layer = 1;
        second.brush.size = 13.0;
        second.brush.taper_out = 0.0;
        second.brush.flow = 1.0;
        let a = [point(130.5, 0.0)];
        let b = [point(150.5, 0.0), point(290.5, 140.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        let painted = expected.layers[1].pixels.clone();
        expected.layers[1].alpha_locked = true;
        expected.layers[1].blend = BlendMode::Multiply;
        expected.layers[1].clipped = true;
        reference(&mut expected, &second, &b);
        assert_ne!(expected.layers[1].pixels, painted);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        for property in [
            Property::AlphaLocked(true),
            Property::Blend(BlendMode::Multiply),
            Property::Clipped(true),
            Property::Locked(true),
        ] {
            assert!(app.set_canvas_property(target, property));
        }
        let properties = app.input_properties().unwrap();
        assert!(properties.alpha_locked && properties.clipped && properties.locked);
        assert_eq!(properties.blend, BlendMode::Multiply);
        assert!(app.input_alpha_locked());
        assert!(!app.doc.layers[1].alpha_locked && !app.doc.layers[1].locked);
        assert!(!app.begin_canvas_stroke(second.clone()));
        assert!(app.set_canvas_property(target, Property::Locked(false)));
        assert!(app.set_canvas_property(target, Property::Locked(false))); // queued no-op
        assert_eq!(app.input_paint_blocker(), None);
        assert!(app.begin_canvas_stroke(second));
        app.deliver_canvas_dabs(b.to_vec());
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].pixels, expected.layers[1].pixels);
        assert_eq!(
            app.doc.composite_transparent(),
            expected.composite_transparent()
        );
        app.undo();
        assert_eq!(app.doc.layers[1].pixels, painted);
        app.undo();
        assert!(app.doc.layers[1].locked);
        assert!(app.input_paint_blocker().is_some());
        app.undo();
        assert!(!app.doc.layers[1].locked);
        app.undo();
        assert!(!app.doc.layers[1].clipped);
        app.undo();
        assert_eq!(app.doc.layers[1].blend, BlendMode::Normal);
        app.undo();
        assert!(!app.doc.layers[1].alpha_locked);
        app.undo();
        assert_eq!(app.doc.layers[1].pixels, original.layers[1].pixels);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn queued_unlock_and_reserved_copy_properties_use_projected_permissions() {
        use crate::document_edits::Property;
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.layers[0].locked = true;
        let original = app.doc.layers[0].id;
        app.render_queue.redo().unwrap();
        assert!(app.set_canvas_property(original, Property::Locked(false)));
        assert!(app.doc.layers[0].locked);
        assert_eq!(app.input_paint_blocker(), None);
        app.duplicate_layer();
        let copy = app.input_target().unwrap();
        assert_ne!(copy, original);
        assert!(app.set_canvas_property(copy, Property::AlphaLocked(true)));
        assert!(app.input_alpha_locked());
        assert!(app.set_canvas_property(copy, Property::Locked(true)));
        let bytes = app.render_queue.queued_input_bytes();
        app.delete_layer();
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.status.contains("Unlock"));
        assert!(app.set_canvas_property(copy, Property::Locked(false)));
        assert!(app.set_canvas_property(copy, Property::AlphaLocked(false)));
        assert!(!app.input_alpha_locked());
        let bytes = app.render_queue.queued_input_bytes();
        assert!(!app.set_canvas_property(copy, Property::Opacity(f32::NAN)));
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.begin_canvas_stroke(Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            secondary: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default()
        }));
        app.deliver_canvas_dabs(vec![Sample {
            x: 16.5,
            y: 16.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, copy);
        assert!(!app.doc.layers[0].locked);
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(app.doc.layers[1].pixels.iter().any(|byte| *byte != 0));
        assert!(!app.doc.layers[1].locked && !app.doc.layers[1].alpha_locked);
    }

    #[test]
    fn queued_folder_toggles_project_permissions_and_selection_before_publication() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.group_active().unwrap();
        app.doc.active = 0;
        let child = app.doc.layers[0].id;
        let folder = app.doc.layers[1].id;
        let brush = settings([17, 83, 201]);
        let a = [point(130.5, 0.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &brush, &a);
        assert!(app.begin_canvas_stroke(brush.clone()));
        app.deliver_canvas_dabs(a.to_vec());
        app.toggle_canvas_layer(1, false);
        assert!(app.doc.layers[1].visible);
        assert!(app.input_paint_blocker().unwrap().contains("hidden"));
        assert!(!app.begin_canvas_stroke(brush.clone()));
        app.toggle_canvas_layer(1, false);
        assert_eq!(app.input_paint_blocker(), None);
        app.toggle_canvas_layer(1, true);
        assert!(!app.doc.layers[1].collapsed);
        assert_eq!(app.input_target(), Some(folder));
        assert!(!app.begin_canvas_stroke(brush.clone()));
        app.toggle_canvas_layer(1, true);
        assert_eq!(app.input_target(), Some(folder));
        assert!(app.select_layer_target(0));
        assert_eq!(app.input_target(), Some(child));
        assert!(app.begin_canvas_stroke(brush));
        app.deliver_canvas_dabs(a.to_vec());
        app.finish_painted_stroke();
        app.flush_render_queue();
        let brush = settings([17, 83, 201]);
        reference(&mut expected, &brush, &a);
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(app.doc.layers[1].visible && !app.doc.layers[1].collapsed);
        assert_eq!(app.doc.active, 0);
        app.undo(); // second stroke
        app.undo(); // expansion
        assert!(app.doc.layers[1].collapsed);
        assert_eq!(app.doc.active, 1);
        app.undo(); // collapse
        assert!(!app.doc.layers[1].collapsed);
        assert_eq!(app.doc.active, 0);
        app.undo(); // show
        assert!(!app.doc.layers[1].visible);
        app.undo(); // hide
        assert!(app.doc.layers[1].visible);
    }

    #[test]
    fn queued_step_across_folder_keeps_following_paint_target_and_separate_undo() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.add_layer();
        app.doc.group_active().unwrap();
        app.doc.active = 0;
        let target = app.doc.layers[0].id;
        let first = settings([17, 83, 201]);
        let mut second = settings([201, 17, 83]);
        second.brush.size = 13.0;
        let a = [point(130.5, 0.0)];
        let b = [point(150.5, 0.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        let painted = expected.layers[0].pixels.clone();
        expected.step_active(true).unwrap();
        second.layer = expected.active;
        reference(&mut expected, &second, &b);
        let both = expected.layers[expected.active].pixels.clone();
        expected.step_active(false).unwrap();
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        app.step_canvas_layer(true);
        assert_eq!(app.doc.active, 0);
        assert_eq!(app.input_target(), Some(target));
        assert!(app.render_queue.busy());
        assert!(app.begin_canvas_stroke(second));
        app.deliver_canvas_dabs(b.to_vec());
        app.step_canvas_layer(false);
        app.step_canvas_layer(false);
        assert!(app.status.contains("bottom"));
        app.flush_render_queue();
        assert_eq!(app.doc.active, 0);
        assert_eq!(app.doc.layers[0].id, target);
        for (actual, expected) in app.doc.layers.iter().zip(&expected.layers) {
            assert_eq!(
                (actual.id, actual.depth, actual.folder),
                (expected.id, expected.depth, expected.folder)
            );
            assert_eq!(actual.pixels, expected.pixels);
        }
        app.undo();
        assert_eq!(app.doc.active, 2);
        assert_eq!(app.doc.layers[2].pixels, both);
        app.undo();
        assert_eq!(app.doc.layers[2].pixels, painted);
        app.undo();
        assert_eq!(app.doc.active, 0);
        assert_eq!(app.doc.layers[0].id, target);
        assert_eq!(app.doc.layers[0].pixels, painted);
    }

    #[test]
    fn queued_merge_and_following_paint_match_sequential_pixels_and_undo() {
        for folder in [false, true] {
            let mut app = Dudel {
                doc: Document::new(512, 512),
                ..Default::default()
            };
            if !folder {
                app.doc.add_layer();
            }
            let mut first = settings([17, 83, 201]);
            first.layer = app.doc.active;
            let mut second = settings([201, 17, 83]);
            second.brush.size = 13.0;
            let a = [point(130.5, 0.0), point(180.5, 50.0)];
            let b = [point(150.5, 0.0), point(190.5, 40.0)];
            let mut expected = app.doc.clone();
            reference(&mut expected, &first, &a);
            let painted = expected.clone();
            if folder {
                expected.group_active().unwrap();
                expected.merge_folder().unwrap();
            } else {
                expected.merge_down().unwrap();
            }
            let merged = expected.layers[expected.active].pixels.clone();
            second.layer = expected.active;
            reference(&mut expected, &second, &b);
            assert!(app.begin_canvas_stroke(first));
            app.deliver_canvas_dabs(a.to_vec());
            if folder {
                app.create_canvas_folder(true);
            }
            app.merge_down();
            let target = app.input_target().unwrap();
            assert_eq!(app.doc.layers.len(), painted.layers.len());
            assert!(app.begin_canvas_stroke(second));
            app.deliver_canvas_dabs(b.to_vec());
            app.finish_painted_stroke();
            app.flush_render_queue();
            assert_eq!(app.doc.layers.len(), expected.layers.len());
            assert_eq!(app.doc.layers[app.doc.active].id, target);
            assert_eq!(
                app.doc.layers[app.doc.active].pixels,
                expected.layers[expected.active].pixels
            );
            app.undo();
            assert_eq!(app.doc.layers[app.doc.active].pixels, merged);
            app.undo();
            assert_eq!(app.doc.layers[app.doc.active].folder, folder);
            if folder {
                app.undo();
            }
            assert_eq!(
                app.doc.layers[app.doc.active].pixels,
                painted.layers[painted.active].pixels
            );
        }
    }

    #[test]
    fn pending_merge_rejects_protected_lower_layer_without_consuming_paint() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.layers[0].alpha_locked = true;
        app.doc.add_layer();
        let mut brush = settings([17, 83, 201]);
        brush.layer = 1;
        let points = [point(130.5, 0.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &brush, &points);
        assert!(app.begin_canvas_stroke(brush));
        app.deliver_canvas_dabs(points.to_vec());
        app.merge_down();
        assert!(app.status.contains("Unlock"));
        assert!(app.render_queue.busy());
        assert_eq!(app.doc.layers.len(), 2);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].pixels, expected.layers[1].pixels);
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        assert!(app.doc.layers[1].pixels.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn duplicate_preserves_nested_symmetry_before_rendering_and_through_undo() {
        use crate::symmetry::{Mode, Symmetry};
        for queued in [false, true] {
            let mut app = Dudel {
                doc: Document::new(32, 32),
                ..Default::default()
            };
            app.doc.group_active().unwrap();
            app.doc.group_active().unwrap();
            let source: Vec<_> = app.doc.layers.iter().map(|layer| layer.id).collect();
            let own = [Mode::LeftRight, Mode::FourWay, Mode::Radial].map(|mode| Symmetry {
                mode,
                angle: 30.0,
                ..Default::default()
            });
            app.layer_symmetry.extend(source.iter().copied().zip(own));
            if queued {
                app.render_queue.redo().unwrap();
            }
            app.duplicate_layer();
            let first_copy = app.input_target().unwrap();
            assert_eq!(app.active_symmetry(), own[2]);
            if queued {
                assert!(app.input_layer().is_none());
                assert_eq!(app.doc.layers.len(), 3);
            }
            // Duplicate again before the first queued copy has materialized.
            app.duplicate_layer();
            let second_copy = app.input_target().unwrap();
            assert_ne!(first_copy, second_copy);
            assert_eq!(app.active_symmetry(), own[2]);
            app.flush_render_queue();
            assert_eq!(app.doc.layers.len(), 9);
            for (index, layer) in app.doc.layers.iter().enumerate() {
                assert_eq!(app.layer_symmetry[&layer.id], own[index % 3]);
            }
            // Copies own independent values; guide edits remain outside undo.
            app.layer_symmetry.get_mut(&second_copy).unwrap().angle = 65.0;
            assert_eq!(app.layer_symmetry[&first_copy], own[2]);
            assert_eq!(app.layer_symmetry[&source[2]], own[2]);
            app.undo();
            assert_eq!(app.doc.layers.len(), 6);
            assert_eq!(app.active_symmetry(), own[2]);
            app.redo();
            assert_eq!(app.doc.layers.len(), 9);
            assert_eq!(app.input_target(), Some(second_copy));
            assert_eq!(app.active_symmetry().angle, 65.0);
        }
    }

    #[test]
    fn duplicate_keeps_global_symmetry_inherited_and_rejection_adds_no_guides() {
        let mut app = Dudel {
            doc: Document::new(8, 8),
            ..Default::default()
        };
        app.symmetry.mode = crate::symmetry::Mode::FourWay;
        app.duplicate_layer();
        assert!(app.layer_symmetry.is_empty());
        assert_eq!(app.active_symmetry(), app.symmetry);
        while app.doc.layers.len() < crate::document::MAX_RASTER_LAYERS {
            app.doc.add_layer();
        }
        app.layer_symmetry
            .insert(app.doc.layers[app.doc.active].id, app.symmetry);
        let before = app.guides_record(&app.doc);
        app.duplicate_layer();
        assert_eq!(app.guides_record(&app.doc), before);
        assert_eq!(app.layer_symmetry.len(), 1);
    }

    #[test]
    fn queued_duplicate_reads_completed_paint_and_later_paint_only_changes_reserved_copy() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let first = settings([17, 83, 201]);
        let mut second = settings([201, 17, 83]);
        second.brush.size = 13.0;
        let a = [point(130.5, 0.0), point(180.5, 50.0)];
        let b = [point(150.5, 0.0), point(190.5, 40.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        let painted = expected.layers[0].pixels.clone();
        expected.duplicate_active().unwrap();
        second.layer = 1;
        reference(&mut expected, &second, &b);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        app.duplicate_layer();
        let copy = app.input_target().unwrap();
        assert_eq!(app.doc.layers.len(), 1);
        assert!(app.begin_canvas_stroke(second));
        app.deliver_canvas_dabs(b.to_vec());
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, copy);
        assert_eq!(app.doc.layers[0].pixels, painted);
        assert_eq!(app.doc.layers[1].pixels, expected.layers[1].pixels);
        app.undo();
        assert_eq!(app.doc.layers[1].pixels, painted);
        app.undo();
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].pixels, painted);
    }

    #[test]
    fn pending_duplicate_locks_and_delete_survivor_keep_target_permissions() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.layers[0].locked = true;
        app.doc.layers[0].alpha_locked = true;
        app.doc.group_active().unwrap();
        app.render_queue.redo().unwrap();
        app.duplicate_layer();
        let folder = app.input_target().unwrap();
        assert!(app.input_is_folder());
        let bytes = app.render_queue.queued_input_bytes();
        app.delete_layer();
        assert_eq!(app.input_target(), Some(folder));
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.status.contains("Unlock"));
        app.ungroup_folder();
        assert!(app.input_alpha_locked());
        assert!(app.input_paint_blocker().is_some());
        app.flush_render_queue();
        assert!(app.doc.layers[app.doc.active].locked);
        assert!(app.doc.layers[app.doc.active].alpha_locked);
        assert_ne!(app.doc.layers[0].id, app.doc.layers[2].id);

        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.add_layer();
        app.doc.active = 0;
        let deleted = app.doc.layers[0].id;
        let survivor = app.doc.layers[1].id;
        app.render_queue.redo().unwrap();
        app.delete_layer();
        assert_eq!(app.input_target(), Some(survivor));
        assert!(!app.select_layer_target(0)); // stale rendered row is already logically deleted
        assert_eq!(app.input_target(), Some(survivor));
        assert!(app.begin_canvas_stroke(Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            secondary: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default()
        }));
        app.deliver_canvas_dabs(vec![Sample {
            x: 16.5,
            y: 16.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].id, survivor);
        assert!(app.doc.layers[0].pixels.iter().any(|value| *value != 0));
        app.undo();
        app.undo();
        assert_eq!(app.doc.layers[0].id, deleted);
        assert_eq!(app.doc.layers[1].id, survivor);
        assert!(
            app.doc
                .layers
                .iter()
                .all(|layer| layer.pixels.iter().all(|value| *value == 0))
        );
    }

    #[test]
    fn queued_group_reserved_child_and_paint_keep_pixels_and_separate_undo_states() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let original = app.doc.layers[0].pixels.clone();
        let first = settings([17, 83, 201]);
        let mut second = settings([201, 17, 83]);
        second.brush.size = 13.0;
        let a = [point(130.5, 0.0), point(180.5, 50.0)];
        let b = [point(150.5, 0.0), point(190.5, 40.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        let painted = expected.layers[0].pixels.clone();
        expected.group_active().unwrap();
        expected.insert_layer_above().unwrap();
        second.layer = expected.active;
        reference(&mut expected, &second, &b);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        app.group_into_folder();
        let folder = app.input_target().unwrap();
        assert!(app.input_is_folder());
        assert!(app.input_paint_blocker().is_some());
        app.new_layer();
        let child = app.input_target().unwrap();
        assert!(!app.input_is_folder());
        assert!(app.input_paint_blocker().is_none());
        assert_eq!(app.doc.layers.len(), 1);
        assert!(app.begin_canvas_stroke(second));
        app.deliver_canvas_dabs(b.to_vec());
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].id, child);
        assert_eq!(app.doc.layers[2].id, folder);
        for (actual, expected) in app.doc.layers.iter().zip(&expected.layers) {
            assert_eq!(actual.folder, expected.folder);
            assert_eq!(actual.depth, expected.depth);
            assert_eq!(actual.pixels, expected.pixels);
        }
        app.undo();
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        app.undo();
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.doc.layers[0].pixels, painted);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, original);
    }

    #[test]
    fn pending_folder_creation_and_ungroup_restore_selected_child_before_execution() {
        for group in [false, true] {
            let mut app = Dudel {
                doc: Document::new(32, 32),
                ..Default::default()
            };
            let child = app.doc.layers[0].id;
            app.render_queue.redo().unwrap();
            if group {
                app.group_into_folder();
            } else {
                app.new_folder();
            }
            let folder = app.input_target().unwrap();
            assert!(app.input_is_folder());
            app.ungroup_folder();
            assert_eq!(app.input_target(), Some(child));
            assert!(!app.input_is_folder());
            assert_eq!(app.doc.layers.len(), 1);
            app.flush_render_queue();
            assert_eq!(app.doc.layers.len(), 1);
            assert_eq!(app.doc.layers[0].id, child);
            app.undo();
            assert_eq!(app.doc.layers.len(), 2);
            assert_eq!(app.doc.layers[1].id, folder);
            assert_eq!(app.doc.layers[0].depth, u8::from(group));
            app.undo();
            assert_eq!(app.doc.layers.len(), 1);
            assert_eq!(app.doc.layers[0].depth, 0);
            assert!(!app.history.can_undo());
        }
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.render_queue.undo().unwrap();
        for _ in 0..crate::document::MAX_DEPTH {
            app.group_into_folder();
        }
        let target = app.input_target();
        let bytes = app.render_queue.queued_input_bytes();
        app.group_into_folder();
        assert_eq!(app.input_target(), target);
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.status.contains("capacity"));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].depth, crate::document::MAX_DEPTH);
        app.doc.validate().unwrap();
    }

    #[test]
    fn queued_folder_reorder_keeps_children_and_rejects_self_destination_atomically() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.group_active().unwrap();
        app.doc.add_layer();
        app.doc.active = 0;
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let child = app.doc.layers[0].id;
        let folder = app.doc.layers[1].id;
        let other = app.doc.layers[2].id;
        let original = app.doc.layers[0].pixels.clone();
        app.render_queue.undo().unwrap();
        let bytes = app.render_queue.queued_input_bytes();
        app.reorder_canvas_layer(1, 1, 0); // destination inside its own block
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.pending_layers.is_none());
        assert!(app.status.contains("inside itself"));
        app.reorder_canvas_layer(1, 3, 0);
        app.select_layer_target(0);
        app.new_layer();
        let new = app.input_target().unwrap();
        app.flush_render_queue();
        assert_eq!(
            app.doc
                .layers
                .iter()
                .map(|layer| layer.id)
                .collect::<Vec<_>>(),
            vec![other, child, new, folder]
        );
        assert_eq!(app.doc.layers[1].pixels, original);
        assert_eq!(app.doc.layers[1].depth, 1);
        assert_eq!(app.doc.layers[2].depth, 1);
        app.doc.validate().unwrap();
        app.undo();
        app.undo();
        assert_eq!(
            app.doc
                .layers
                .iter()
                .map(|layer| layer.id)
                .collect::<Vec<_>>(),
            vec![child, folder, other]
        );
        assert_eq!(app.doc.layers[0].pixels, original);
    }

    #[test]
    fn pending_reorder_captures_stroke_target_and_restores_order_through_undo() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.add_layer();
        app.doc.active = 0;
        let first_id = app.doc.layers[0].id;
        let other_id = app.doc.layers[1].id;
        let first = settings([17, 83, 201]);
        let mut second = settings([201, 17, 83]);
        second.brush.size = 13.0;
        let a = [point(130.5, 0.0), point(180.5, 50.0)];
        let b = [point(150.5, 0.0), point(190.5, 40.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        let painted = expected.layers[0].pixels.clone();
        expected.move_block(0, 2, 0).unwrap();
        second.layer = 1;
        reference(&mut expected, &second, &b);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        app.reorder_canvas_layer(0, 2, 0);
        assert_eq!(app.doc.layers[0].id, first_id);
        assert_eq!(app.input_target(), Some(first_id));
        assert!(app.begin_canvas_stroke(second));
        app.deliver_canvas_dabs(b.to_vec());
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].id, other_id);
        assert_eq!(app.doc.layers[1].id, first_id);
        for (actual, expected) in app.doc.layers.iter().zip(&expected.layers) {
            assert_eq!(actual.pixels, expected.pixels);
        }
        app.undo();
        assert_eq!(app.doc.layers[1].pixels, painted);
        app.undo();
        assert_eq!(app.doc.layers[0].id, first_id);
        assert_eq!(app.doc.layers[1].id, other_id);
        assert_eq!(app.doc.layers[0].pixels, painted);
    }

    #[test]
    fn projected_reorder_permissions_and_reverting_pending_moves_use_identity_destinations() {
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.add_folder().unwrap();
        app.doc.layers[1].visible = false;
        app.doc.active = 0;
        app.render_queue.undo().unwrap();
        app.reorder_canvas_layer(0, 1, 1); // put the raster inside the hidden folder
        assert!(app.input_paint_blocker().is_some());
        assert!(app.doc.paint_blocker().is_none());
        app.flush_render_queue();
        assert!(app.doc.paint_blocker().is_some());

        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.add_layer();
        app.doc.active = 0;
        let original = [app.doc.layers[0].id, app.doc.layers[1].id];
        app.render_queue.undo().unwrap();
        app.reorder_canvas_layer(0, 2, 0);
        // The rendered rows are still A/B. Targeting the captured B row moves
        // A back before it; comparing the old numeric gap alone would skip this.
        app.reorder_canvas_layer(0, 1, 0);
        app.flush_render_queue();
        assert_eq!([app.doc.layers[0].id, app.doc.layers[1].id], original);
        app.undo();
        assert_eq!(
            [app.doc.layers[0].id, app.doc.layers[1].id],
            [original[1], original[0]]
        );
        app.undo();
        assert_eq!([app.doc.layers[0].id, app.doc.layers[1].id], original);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn actual_canvas_tap_targets_pending_raster_inside_open_folder() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(512, 512),
            brush: settings([17, 83, 201]).brush,
            color: [17, 83, 201],
            ..Default::default()
        };
        app.doc.group_active().unwrap();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        app.render_queue.redo().unwrap();
        app.new_layer();
        let target = app.input_target().unwrap();
        assert!(app.doc.layers[app.doc.active].folder);
        assert_eq!(app.input_layer(), None);
        let position = app
            .view(app.canvas_area)
            .to_screen(egui::pos2(170.5, 170.5));
        let screen = Sample {
            x: position.x,
            y: position.y,
            pressure: 1.0,
            ..Default::default()
        };
        let local_position = app.view(app.canvas_area).to_doc(position);
        let local = Sample {
            x: local_position.x,
            y: local_position.y,
            ..screen
        };
        let mut sampler = crate::stroke::Stroke::default();
        let mut dabs = sampler.sample_at_zoom(local, &app.brush, app.zoom);
        dabs.extend(sampler.finish_at_zoom(local, &app.brush, app.zoom));
        let mut expected = app.doc.clone();
        expected.insert_layer_above().unwrap();
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: app.brush.clone(),
            color: app.color,
            secondary: app.color,
            erase: false,
            layer: expected.active,
            symmetry: app.symmetry,
        };
        reference(&mut expected, &settings, &dabs);
        let mut input = raw();
        input.events.push(egui::Event::PointerMoved(position));
        let _ = ctx.run(input, |ctx| {
            app.canvas(ctx, &[Packet::Start(screen), Packet::End(screen)])
        });
        let mut frames = 0;
        while app.render_queue.busy() {
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            frames += 1;
            assert!(frames < 1000);
        }
        assert_eq!(app.doc.layers[app.doc.active].id, target);
        for (actual, expected) in app.doc.layers.iter().zip(&expected.layers) {
            assert_eq!(actual.depth, expected.depth);
            assert_eq!(actual.folder, expected.folder);
            assert_eq!(actual.pixels, expected.pixels);
        }
        app.undo();
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
    }

    #[test]
    fn queued_creation_chain_targets_reserved_layers_without_publishing_early() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        let first = settings([17, 83, 201]);
        let mut second = settings([201, 17, 83]);
        second.brush.size = 13.0;
        let a = [point(130.5, 0.0), point(180.5, 50.0)];
        let b = [point(150.5, 0.0), point(190.5, 40.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        expected.insert_layer_above().unwrap();
        expected.insert_layer_above().unwrap();
        second.layer = expected.active;
        reference(&mut expected, &second, &b);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        app.new_layer();
        let first_id = app.input_target().unwrap();
        app.new_layer();
        let second_id = app.input_target().unwrap();
        assert_ne!(first_id, second_id);
        assert_eq!(app.doc.layers.len(), 1);
        assert_eq!(app.input_layer(), None);
        assert!(app.input_paint_blocker().is_none());
        assert!(app.begin_canvas_stroke(second));
        app.deliver_canvas_dabs(b.to_vec());
        app.finish_painted_stroke();
        app.flush_render_queue();
        assert!(app.pending_layers.is_none() && app.logical_layer.is_none());
        assert_eq!(app.doc.layers.len(), 3);
        assert_eq!(app.doc.layers[1].id, first_id);
        assert_eq!(app.doc.layers[2].id, second_id);
        for (actual, expected) in app.doc.layers.iter().zip(&expected.layers) {
            assert_eq!(actual.pixels, expected.pixels);
        }
        app.undo();
        assert!(app.doc.layers[2].pixels.iter().all(|value| *value == 0));
        app.undo();
        assert_eq!(app.doc.layers.len(), 2);
        app.undo();
        assert_eq!(app.doc.layers.len(), 1);
        app.undo();
        assert!(app.doc.layers[0].pixels.iter().all(|value| *value == 0));
    }

    #[test]
    fn pending_creation_respects_folder_visibility_and_projected_capacity() {
        for shown in [false, true] {
            let mut app = Dudel {
                doc: Document::new(32, 32),
                ..Default::default()
            };
            app.doc.group_active().unwrap();
            app.doc.layers[app.doc.active].visible = shown;
            app.render_queue.redo().unwrap();
            app.new_layer();
            let id = app.input_target().unwrap();
            assert_eq!(app.input_paint_blocker().is_none(), shown);
            app.flush_render_queue();
            assert_eq!(app.doc.layers[app.doc.active].id, id);
            assert_eq!(app.doc.layers[app.doc.active].depth, 1);
            assert_eq!(app.doc.paint_blocker().is_none(), shown);
        }
        let mut app = Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.render_queue.undo().unwrap();
        // Fill most of the document directly; queue the last layers so the
        // limit is reached through projected (not yet applied) creations.
        while app.doc.layers.len() < crate::document::MAX_RASTER_LAYERS - 3 {
            app.doc.add_layer();
        }
        for _ in 0..3 {
            app.new_layer();
        }
        let last = app.input_target().unwrap();
        let bytes = app.render_queue.queued_input_bytes();
        app.new_layer();
        assert_eq!(app.input_target(), Some(last));
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.status.contains("capacity"));
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), crate::document::MAX_RASTER_LAYERS);
        app.doc.validate().unwrap();
    }

    #[test]
    fn canvas_accepts_pending_unlocked_target_when_old_active_layer_is_locked() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(512, 512),
            brush: settings([17, 83, 201]).brush,
            color: [17, 83, 201],
            ..Default::default()
        };
        app.doc.layers[0].locked = true;
        app.doc.add_layer();
        app.doc.active = 0;
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        app.render_queue.undo().unwrap(); // keep a control pending without painting the locked layer
        app.select_layer_target(1);
        assert_eq!(app.doc.active, 0);
        let position = app
            .view(app.canvas_area)
            .to_screen(egui::pos2(170.5, 170.5));
        let screen = Sample {
            x: position.x,
            y: position.y,
            pressure: 1.0,
            ..Default::default()
        };
        let mut input = raw();
        input.events.push(egui::Event::PointerMoved(position));
        let _ = ctx.run(input, |ctx| {
            app.canvas(ctx, &[Packet::Start(screen), Packet::End(screen)])
        });
        let mut frames = 0;
        while app.render_queue.busy() {
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            frames += 1;
            assert!(frames < 1000);
        }
        assert_eq!(app.doc.active, 1);
        assert!(app.doc.layers[0].pixels.iter().all(|value| *value == 0));
        assert!(app.doc.layers[1].pixels.iter().any(|value| *value != 0));
        assert!(app.history.can_undo());
        app.undo();
        assert!(app.doc.layers[1].pixels.iter().all(|value| *value == 0));
    }

    #[test]
    fn rapid_layer_switches_capture_each_contact_target_before_rendering_catches_up() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.add_layer();
        app.doc.active = 0;
        let first = settings([17, 83, 201]);
        let mut second = settings([201, 17, 83]);
        second.layer = 1;
        second.brush.size = 13.0;
        let mut third = settings([83, 201, 17]);
        third.brush.size = 19.0;
        let a = [point(130.5, 0.0), point(180.5, 50.0)];
        let b = [point(150.5, 0.0), point(190.5, 40.0)];
        let c = [point(170.5, 0.0), point(210.5, 40.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &first, &a);
        let first_pixels = expected.layers[0].pixels.clone();
        expected.active = 1;
        reference(&mut expected, &second, &b);
        expected.active = 0;
        reference(&mut expected, &third, &c);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(a.to_vec());
        app.select_layer_target(1);
        assert_eq!(app.doc.active, 0);
        assert_eq!(app.input_layer(), Some(1));
        // Even a caller's stale index is replaced with the logical target.
        let mut stale = second;
        stale.layer = 0;
        assert!(app.begin_canvas_stroke(stale));
        assert_eq!(app.stroke_settings.as_ref().unwrap().layer, 1);
        app.deliver_canvas_dabs(b.to_vec());
        app.select_layer_target(0);
        assert_eq!(app.input_layer(), Some(0));
        assert!(app.begin_canvas_stroke(third));
        app.deliver_canvas_dabs(c.to_vec());
        app.finish_painted_stroke();
        assert!(!app.history.can_undo());
        app.flush_render_queue();
        assert!(app.logical_layer.is_none());
        assert!(app.edit_feedback.is_empty());
        assert_eq!(app.doc.active, 0);
        for (actual, expected) in app.doc.layers.iter().zip(&expected.layers) {
            assert_eq!(actual.pixels, expected.pixels);
        }
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, first_pixels);
        assert_eq!(app.doc.layers[1].pixels, expected.layers[1].pixels);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, first_pixels);
        assert!(app.doc.layers[1].pixels.iter().all(|value| *value == 0));
    }

    #[test]
    fn queued_rename_revert_and_save_preserve_logical_order_without_draining() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rename.dudel");
        let mut app = Dudel {
            doc: Document::new(32, 32),
            path: Some(path.clone()),
            ..Default::default()
        };
        let original = app.doc.layers[0].name.clone();
        assert!(app.begin_canvas_stroke(Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            secondary: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default()
        }));
        app.renaming = Some(0);
        app.rename_draft = "Queued name".into();
        app.finish_layer_rename(true);
        assert_eq!(app.doc.layers[0].name, original);
        app.renaming = Some(0);
        app.rename_draft = original.clone();
        app.finish_layer_rename(true);
        app.save(false);
        assert_eq!(app.edit_feedback.len(), 2);
        assert_eq!(app.revision, 0);
        let ctx = egui::Context::default();
        while app.queued_save.is_some() {
            app.pump_render_queue(&ctx);
        }
        assert_eq!(app.doc.layers[0].name, original);
        assert!(app.edit_feedback.is_empty());
        let job = app.save_job.take().unwrap();
        job.worker.join().unwrap().unwrap();
        assert_eq!(
            crate::storage::load_project(&path).unwrap().layers[0].name,
            original
        );
        app.undo();
        assert_eq!(app.doc.layers[0].name, "Queued name");
        app.undo();
        assert_eq!(app.doc.layers[0].name, original);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn application_selection_transform_and_clear_queue_after_paint_with_separate_undo() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let original = app.doc.layers[0].pixels.clone();
        let settings = settings([17, 83, 201]);
        let points = [point(130.5, 0.0), point(180.5, 50.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &settings, &points);
        let painted = expected.layers[0].pixels.clone();
        let selection = crate::selection::Selection {
            left: 0,
            top: 0,
            right: 256,
            bottom: 512,
        };
        expected.selection = Some(selection);
        expected
            .transform_active(crate::transform::Transform::FlipHorizontal)
            .unwrap();
        let transformed = expected.layers[0].pixels.clone();
        expected.selection = None;
        expected.clear_selected().unwrap();
        expected.selection = Some(selection);
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(points.to_vec());
        app.set_selection(Some(selection));
        app.transform_active(crate::transform::Transform::FlipHorizontal);
        app.clear_layer_pixels();
        assert_eq!(app.doc.layers[0].pixels, original);
        assert_eq!(app.doc.selection, None);
        assert_eq!(app.edit_feedback.len(), 3);
        app.flush_render_queue();
        assert!(app.edit_feedback.is_empty());
        assert_eq!(app.doc.selection, Some(selection));
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, transformed);
        assert_eq!(app.doc.selection, Some(selection));
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, painted);
        app.undo();
        assert_eq!(app.doc.selection, None);
        assert_eq!(app.doc.layers[0].pixels, painted);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, original);
    }

    #[test]
    fn queued_selection_advances_snapshot_revision_without_raster_damage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("selection.dudel");
        let mut app = Dudel {
            doc: Document::new(32, 32),
            path: Some(path.clone()),
            ..Default::default()
        };
        let original = app.doc.layers[0].pixels.clone();
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            secondary: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        assert!(app.begin_canvas_stroke(settings));
        // No dabs: the stroke is a no-op and must not consume redo or a revision.
        app.select_all();
        app.save(false);
        assert_eq!(app.revision, 0);
        let ctx = egui::Context::default();
        while app.queued_save.is_some() {
            app.pump_render_queue(&ctx);
        }
        assert_eq!(app.doc.layers[0].pixels, original);
        assert_eq!(app.revision, 1);
        let job = app.save_job.take().unwrap();
        assert_eq!(job.revision, 1);
        job.worker.join().unwrap().unwrap();
        let saved = crate::storage::load_project(&path).unwrap();
        assert_eq!(saved.selection, app.doc.selection);
        assert_eq!(saved.layers[0].pixels, original);
        app.undo();
        assert_eq!(app.doc.selection, None);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn application_undo_redo_wait_for_pending_stroke_without_draining_on_request() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        let settings = settings([17, 83, 201]);
        let points = [point(130.5, 0.0), point(180.5, 50.0)];
        let before = app.doc.layers[0].pixels.clone();
        let mut expected = app.doc.clone();
        reference(&mut expected, &settings, &points);
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(points.to_vec());
        app.undo();
        assert!(!app.stroking);
        assert_eq!(app.doc.layers[0].pixels, before);
        assert!(!app.history.can_undo());
        assert!(app.render_queue.busy());
        app.redo();
        assert_eq!(app.doc.layers[0].pixels, before);
        let ctx = egui::Context::default();
        let mut calls = 0;
        while app.render_queue.needs_work() {
            app.pump_render_queue(&ctx);
            calls += 1;
            assert!(calls < 1000);
        }
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(app.status, "Redone");
        assert!(app.history.can_undo());
        assert!(!app.history.can_redo());
    }

    #[test]
    fn consecutive_canvas_contacts_keep_settings_order_and_history() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        let mut expected = app.doc.clone();
        let first = settings([17, 83, 201]);
        let a = [point(130.5, 0.0), point(150.5, 20.0)];
        assert!(app.begin_canvas_stroke(first.clone()));
        app.deliver_canvas_dabs(a.to_vec());
        app.finish_painted_stroke();
        assert!(app.render_queue.busy());
        assert!(!app.stroking);
        assert!(!app.history.can_undo());
        app.color = [201, 17, 83];
        let mut second = settings(app.color);
        second.brush.size = 13.0;
        let b = [point(145.5, 0.0), point(168.5, 23.0)];
        assert!(app.begin_canvas_stroke(second.clone()));
        app.deliver_canvas_dabs(b.to_vec());
        app.finish_painted_stroke();
        reference(&mut expected, &first, &a);
        let first_pixels = expected.layers[0].pixels.clone();
        reference(&mut expected, &second, &b);
        app.flush_render_queue();
        assert!(!app.render_queue.busy());
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(app.modified && app.revision > 0);
        assert!(!app.dirty_region.is_empty());
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, first_pixels);
        app.redo();
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
    }

    #[test]
    fn queued_cancel_restores_partial_writeback_and_shift_anchor() {
        let mut app = Dudel {
            doc: Document::new(512, 512),
            ..Default::default()
        };
        let old_anchor = point(30.5, 0.0);
        app.last_stroke_end = Some(old_anchor);
        let before = app.doc.layers[0].pixels.clone();
        assert!(app.begin_canvas_stroke(settings([17, 83, 201])));
        app.last_stroke_end = Some(point(150.5, 20.0));
        app.deliver_canvas_dabs(vec![point(150.5, 0.0)]);
        for _ in 0..30 {
            let progress = app
                .render_queue
                .advance(&mut app.doc, &mut app.history, 1)
                .unwrap();
            app.accept_render_progress(progress);
        }
        assert_ne!(app.doc.layers[0].pixels, before);
        app.cancel_painted_stroke();
        assert!(!app.stroking && app.queued_stroke.is_none());
        assert_eq!(app.last_stroke_end.unwrap().x, old_anchor.x);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels, before);
        assert!(!app.history.can_undo());
        assert!(!app.history.has_pending_stroke());
    }

    #[test]
    fn actual_canvas_packet_tap_uses_queue_and_pumps_without_further_input() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(512, 512),
            brush: settings([17, 83, 201]).brush,
            color: [17, 83, 201],
            ..Default::default()
        };
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let position = app
            .view(app.canvas_area)
            .to_screen(egui::pos2(170.5, 170.5));
        let screen = Sample {
            x: position.x,
            y: position.y,
            pressure: 1.0,
            ..Default::default()
        };
        let mut expected = app.doc.clone();
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: app.brush.clone(),
            color: app.color,
            secondary: app.color,
            erase: false,
            layer: 0,
            symmetry: app.symmetry,
        };
        let mut sampler = crate::stroke::Stroke::default();
        let local_position = app.view(app.canvas_area).to_doc(position);
        let local = Sample {
            x: local_position.x,
            y: local_position.y,
            ..screen
        };
        let mut dabs = sampler.sample_at_zoom(local, &settings.brush, app.zoom);
        dabs.extend(sampler.finish_at_zoom(local, &settings.brush, app.zoom));
        reference(&mut expected, &settings, &dabs);
        let mut input = raw();
        input.events.push(egui::Event::PointerMoved(position));
        let _ = ctx.run(input, |ctx| {
            app.canvas(ctx, &[Packet::Start(screen), Packet::End(screen)])
        });
        assert!(!app.stroking && app.stroke_buffer.is_none());
        assert!(app.render_queue.busy());
        app.color = [255; 3];
        app.brush.size = 7.0;
        app.rotation = 0.4;
        app.mirror = true;
        let mut frames = 0;
        while app.render_queue.busy() {
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            frames += 1;
            assert!(frames < 1000);
        }
        assert!(app.history.can_undo());
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
    }

    #[test]
    fn save_after_pending_queue_writes_settled_artwork_to_disk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.dudel");
        let mut app = Dudel {
            doc: Document::new(512, 512),
            path: Some(path.clone()),
            ..Default::default()
        };
        let settings = settings([17, 83, 201]);
        let points = [point(130.5, 0.0), point(180.5, 50.0)];
        let mut expected = app.doc.clone();
        reference(&mut expected, &settings, &points);
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(points.to_vec());
        app.finish_painted_stroke();
        assert!(app.render_queue.busy());
        app.save(false);
        assert!(app.render_queue.busy());
        assert!(app.queued_save.is_some());
        assert!(app.save_job.is_none());
        let ctx = egui::Context::default();
        let mut frames = 0;
        while app.queued_save.is_some() {
            app.pump_render_queue(&ctx);
            frames += 1;
            assert!(frames < 1000);
        }
        assert!(!app.render_queue.busy());
        let job = app.save_job.take().unwrap();
        assert_eq!(job.revision, app.revision);
        job.worker.join().unwrap().unwrap();
        let saved = crate::storage::load_project(&path).unwrap();
        assert_eq!(saved.layers[0].pixels, expected.layers[0].pixels);
    }
}
