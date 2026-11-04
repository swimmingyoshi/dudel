//! Session selection and atomic, identity-targeted layer stack edits.
use crate::{
    Dudel,
    document::{Document, Layer, LayerId},
    egui,
};

#[derive(Default)]
pub(crate) struct Selection {
    pub ids: Vec<LayerId>,
    anchor: Option<LayerId>,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Visible(bool),
    Locked(bool),
    Delete,
    Group(LayerId),
    Step(bool),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub targets: Vec<LayerId>,
    pub action: Action,
}

impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.targets.is_empty() || self.targets.len() > crate::document::MAX_ENTRIES {
            return Err("Invalid batch layer count".into());
        }
        Ok(())
    }

    fn roots(&self, doc: &Document) -> Result<Vec<usize>, String> {
        let mut indices = Vec::new();
        for id in &self.targets {
            let index = doc
                .layers
                .iter()
                .position(|l| l.id == *id)
                .ok_or("A selected layer no longer exists")?;
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
        indices.sort_unstable();
        Ok(indices
            .iter()
            .copied()
            .filter(|i| !indices.iter().any(|j| j != i && doc.block(*j).contains(i)))
            .collect())
    }

    pub fn apply(&self, doc: &mut Document) -> Result<bool, String> {
        self.validate()?;
        let roots = self.roots(doc)?;
        let active = doc.layers[doc.active].id;
        if let Action::Visible(value) | Action::Locked(value) = self.action {
            let mut changed = false;
            for index in roots {
                let property = if matches!(self.action, Action::Visible(_)) {
                    &mut doc.layers[index].visible
                } else {
                    &mut doc.layers[index].locked
                };
                changed |= *property != value;
                *property = value;
            }
            return Ok(changed);
        }
        // A folder includes its descendants; an inherited lock protects children.
        for &root in &roots {
            let mut ancestor = Some(root);
            while let Some(index) = ancestor {
                if doc.layers[index].locked {
                    return Err("Unlock selected layers and their parent folders first".into());
                }
                ancestor = doc.parent(index);
            }
            if doc.layers[doc.block(root)].iter().any(|l| l.locked) {
                return Err("Unlock selected folder contents first".into());
            }
        }
        let ids: Vec<_> = roots.iter().map(|&i| doc.layers[i].id).collect();
        match self.action {
            Action::Delete => {
                for id in ids.iter().rev() {
                    doc.active = doc.layers.iter().position(|l| l.id == *id).unwrap();
                    doc.delete_active()?;
                }
            }
            Action::Group(folder_id) => {
                if roots.iter().any(|&i| doc.parent(i) != doc.parent(roots[0])) {
                    return Err("Select layers in the same folder to group them".into());
                }
                if !doc.has_room(0, 1) {
                    return Err("The layer stack is full".into());
                }
                if doc.layers.iter().any(|l| {
                    l.id == folder_id || l.mask.as_ref().is_some_and(|m| m.id == folder_id)
                }) {
                    return Err("Folder identity already exists".into());
                }
                let depth = doc.layers[roots[0]].depth;
                let mut taken = vec![false; doc.layers.len()];
                for &root in &roots {
                    for index in doc.block(root) {
                        if doc.layers[index].depth >= crate::document::MAX_DEPTH {
                            return Err("Maximum folder depth reached".into());
                        }
                        taken[index] = true;
                    }
                }
                let top = *roots.last().unwrap();
                let gap = taken[..=top].iter().filter(|&&v| !v).count();
                let mut contents = Vec::new();
                let mut remaining = Vec::new();
                for (index, mut layer) in doc.layers.drain(..).enumerate() {
                    if taken[index] {
                        layer.depth += 1;
                        contents.push(layer);
                    } else {
                        remaining.push(layer);
                    }
                }
                let mut folder = Layer::new_folder("Selected layers".into());
                folder.id = folder_id;
                folder.depth = depth;
                contents.push(folder);
                remaining.splice(gap..gap, contents);
                doc.layers = remaining;
                doc.active = doc.layers.iter().position(|l| l.id == folder_id).unwrap();
            }
            Action::Step(up) => {
                if roots.iter().any(|&i| doc.parent(i) != doc.parent(roots[0])) {
                    return Err("Select layers in the same folder to move them together".into());
                }
                let before: Vec<_> = doc.layers.iter().map(|l| l.id).collect();
                let mut order = ids.clone();
                if up {
                    order.reverse();
                }
                for id in order {
                    let index = doc.layers.iter().position(|l| l.id == id).unwrap();
                    let depth = doc.layers[index].depth;
                    let neighbor = if up {
                        (index + 1..doc.layers.len())
                            .find(|&i| doc.layers[i].depth <= depth)
                            .filter(|&i| doc.layers[i].depth == depth)
                    } else {
                        doc.block(index)
                            .start
                            .checked_sub(1)
                            .filter(|&i| doc.layers[i].depth == depth)
                    };
                    if neighbor.is_some_and(|i| !ids.contains(&doc.layers[i].id)) {
                        doc.active = index;
                        doc.step_active(up)?;
                    }
                }
                doc.active = doc.layers.iter().position(|l| l.id == active).unwrap();
                return Ok(before != doc.layers.iter().map(|l| l.id).collect::<Vec<_>>());
            }
            Action::Visible(_) | Action::Locked(_) => unreachable!(),
        }
        if !matches!(self.action, Action::Group(_))
            && let Some(index) = doc.layers.iter().position(|l| l.id == active)
        {
            doc.active = index;
        }
        doc.validate_structure()?;
        Ok(true)
    }
}

impl Dudel {
    pub(crate) fn sync_layer_selection(&mut self) {
        if self.reference_layers.active.is_some() {
            self.layer_selection = Default::default();
            return;
        }
        self.layer_selection
            .ids
            .retain(|id| self.doc.layers.iter().any(|l| l.id == *id));
        if !self
            .input_target()
            .is_some_and(|id| self.layer_selection.ids.contains(&id))
        {
            self.layer_selection = Default::default();
        }
    }

    pub(crate) fn multiple_layers(&self) -> bool {
        self.reference_layers.active.is_none()
            && self.layer_selection.ids.len() > 1
            && self
                .input_target()
                .is_some_and(|id| self.layer_selection.ids.contains(&id))
    }

    fn visible_layer_indices(&self) -> Vec<usize> {
        (0..self.doc.layers.len())
            .rev()
            .filter(|&i| {
                if self.layer_navigation.active() {
                    return self.layer_navigation.matches(&self.doc.layers[i]);
                }
                let mut parent = self.doc.parent(i);
                while let Some(index) = parent {
                    if self.doc.layers[index].collapsed {
                        return false;
                    }
                    parent = self.doc.parent(index);
                }
                true
            })
            .collect()
    }

    pub(crate) fn select_layer_row(&mut self, index: usize, toggle: bool, range: bool) {
        self.sync_layer_selection();
        let id = self.doc.layers[index].id;
        let active = self.input_target();
        let mut ids = self.layer_selection.ids.clone();
        if ids.is_empty() {
            ids.extend(active);
        }
        let anchor = self.layer_selection.anchor.or(active).unwrap_or(id);
        if range {
            let visible = self.visible_layer_indices();
            let a = visible
                .iter()
                .position(|&i| self.doc.layers[i].id == anchor);
            let b = visible.iter().position(|&i| self.doc.layers[i].id == id);
            if !toggle {
                ids.clear();
            }
            if let (Some(a), Some(b)) = (a, b) {
                for &i in &visible[a.min(b)..=a.max(b)] {
                    let entry = self.doc.layers[i].id;
                    if !ids.contains(&entry) {
                        ids.push(entry);
                    }
                }
            } else if !ids.contains(&id) {
                ids.push(id);
            }
        } else if toggle {
            if ids.contains(&id) {
                ids.retain(|entry| *entry != id);
            } else {
                ids.push(id);
            }
        } else {
            ids = vec![id];
        }
        if ids.is_empty() {
            ids.push(id);
        }
        let target = if ids.contains(&id) { id } else { ids[0] };
        let target_index = self.doc.layers.iter().position(|l| l.id == target).unwrap();
        if self.select_layer_target(target_index) {
            self.layer_selection.ids = ids;
            self.layer_selection.anchor = Some(if range { anchor } else { id });
        }
    }

    pub(crate) fn batch_layers(&mut self, action: Action) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        // Settle earlier painting/metadata before resolving the whole selection.
        // Dispatch one COW candidate so every batch has one undo checkpoint.
        self.flush_render_queue();
        self.sync_layer_selection();
        let targets = if self.multiple_layers() {
            self.layer_selection.ids.clone()
        } else {
            self.input_target().into_iter().collect()
        };
        let previous_target = self.input_target();
        self.dispatch_document_edit(
            crate::document_edits::Edit::LayerBatch(Request { targets, action }),
            "Selected layers updated",
            "Selected layers unchanged",
        );
        if self.input_target() != previous_target {
            self.mask_focus = None;
            self.last_stroke_end = None;
        }
        self.sync_layer_selection();
    }

    pub(crate) fn layer_batch_ui(&mut self, ui: &mut egui::Ui) {
        self.sync_layer_selection();
        if !self.multiple_layers() {
            return;
        }
        let visible = self.visible_layer_indices();
        let hidden = self
            .layer_selection
            .ids
            .iter()
            .filter(|id| !visible.iter().any(|&i| self.doc.layers[i].id == **id))
            .count();
        ui.horizontal_wrapped(|ui| {
            let count = self.layer_selection.ids.len();
            ui.small(if hidden == 0 { format!("{count} selected") } else { format!("{count} selected ({hidden} hidden)") });
            ui.menu_button("Batch", |ui| {
                for (label, action) in [
                    ("Show selected", Action::Visible(true)), ("Hide selected", Action::Visible(false)),
                    ("Lock selected", Action::Locked(true)), ("Unlock selected", Action::Locked(false)),
                    ("Move selected up", Action::Step(true)), ("Move selected down", Action::Step(false)),
                    ("Group selected", Action::Group(LayerId::new())), ("Delete selected", Action::Delete),
                ] {
                    if ui.button(label).clicked() { ui.close(); self.batch_layers(action); }
                }
                ui.separator();
                ui.small("Folders include their contents. Move/group require the same parent. Painting still uses the active layer.");
            });
            if ui.small_button("Only active").clicked() { self.layer_selection = Default::default(); }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_edits::Edit;

    fn stack() -> Document {
        let mut doc = Document::new(8, 8);
        for _ in 0..3 {
            doc.add_layer();
        }
        for (i, layer) in doc.layers.iter_mut().enumerate() {
            layer.name = format!("Layer {i}");
        }
        doc
    }
    fn edit(doc: &Document, indices: &[usize], action: Action) -> Edit {
        Edit::LayerBatch(Request {
            targets: indices.iter().map(|&i| doc.layers[i].id).collect(),
            action,
        })
    }
    fn names(doc: &Document) -> Vec<&str> {
        doc.layers.iter().map(|l| l.name.as_str()).collect()
    }

    #[test]
    fn batch_group_preserves_noncontiguous_order_payloads_and_one_undo() {
        let mut app = Dudel {
            doc: stack(),
            ..Default::default()
        };
        app.doc.version = 2;
        app.doc.layers[1].mask = Some(crate::layer_mask::LayerMask {
            id: LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(8, 8, 127).unwrap(),
        });
        app.doc.active = 3;
        let original = app.doc.clone();
        app.layer_selection.ids = vec![app.doc.layers[1].id, app.doc.layers[3].id];
        app.batch_layers(Action::Group(LayerId::new()));
        assert_eq!(
            names(&app.doc),
            [
                "Layer 0",
                "Layer 2",
                "Layer 1",
                "Layer 3",
                "Selected layers"
            ]
        );
        assert_eq!(
            app.doc.layers.iter().map(|l| l.depth).collect::<Vec<_>>(),
            [0, 0, 1, 1, 0]
        );
        assert!(app.doc.layers[2].mask.is_some());
        let grouped = app.doc.clone();
        app.undo();
        app.flush_render_queue();
        assert!(crate::document::same_snapshot(&app.doc, &original));
        assert!(!app.history.can_undo());
        app.redo();
        app.flush_render_queue();
        assert!(crate::document::same_snapshot(&app.doc, &grouped));
    }

    #[test]
    fn batch_step_preserves_order_and_stops_at_sibling_boundaries() {
        let doc = stack();
        let (up, _) = edit(&doc, &[0, 2], Action::Step(true))
            .prepare(&doc)
            .unwrap();
        assert_eq!(names(&up), ["Layer 1", "Layer 0", "Layer 3", "Layer 2"]);
        let (back, _) = edit(&up, &[1, 3], Action::Step(false))
            .prepare(&up)
            .unwrap();
        assert_eq!(names(&back), names(&doc));
        let (unchanged, outcome) = edit(&doc, &[2, 3], Action::Step(true))
            .prepare(&doc)
            .unwrap();
        assert!(!outcome.changed);
        assert!(crate::document::same_snapshot(&doc, &unchanged));
    }

    #[test]
    fn batch_rejections_are_atomic_and_folder_targets_are_deduplicated() {
        let mut doc = stack();
        let original = doc.clone();
        assert!(
            edit(&doc, &[0, 1, 2, 3], Action::Delete)
                .prepare(&doc)
                .is_err()
        );
        assert!(crate::document::same_snapshot(&original, &doc));
        doc.layers[1].locked = true;
        for action in [
            Action::Delete,
            Action::Step(true),
            Action::Group(LayerId::new()),
        ] {
            assert!(edit(&doc, &[0, 1], action).prepare(&doc).is_err());
        }
        let missing = Edit::LayerBatch(Request {
            targets: vec![doc.layers[0].id, LayerId::new()],
            action: Action::Visible(false),
        });
        assert!(missing.prepare(&doc).is_err());
        doc.layers[1].locked = false;
        doc.active = 1;
        doc.group_active().unwrap();
        assert!(
            edit(&doc, &[1, 4], Action::Group(LayerId::new()))
                .prepare(&doc)
                .is_err()
        );
        let (hidden, _) = edit(&doc, &[1, 2], Action::Visible(false))
            .prepare(&doc)
            .unwrap();
        assert!(
            hidden.layers[1].visible,
            "A selected child is covered by its selected folder"
        );
        assert!(!hidden.layers[2].visible);
        let (deleted, _) = edit(&doc, &[1, 2], Action::Delete).prepare(&doc).unwrap();
        assert_eq!(names(&deleted), ["Layer 0", "Layer 2", "Layer 3"]);
    }

    #[test]
    fn batch_property_noops_do_not_create_history_and_undo_restores_all_targets() {
        let mut app = Dudel {
            doc: stack(),
            ..Default::default()
        };
        app.layer_selection.ids = vec![app.doc.layers[1].id, app.doc.layers[3].id];
        app.batch_layers(Action::Visible(true));
        assert!(!app.history.can_undo());
        app.batch_layers(Action::Locked(true));
        assert!(app.doc.layers[1].locked && app.doc.layers[3].locked);
        assert!(!app.doc.layers[0].locked && !app.doc.layers[2].locked);
        app.undo();
        app.flush_render_queue();
        assert!(app.doc.layers.iter().all(|l| !l.locked));
        assert!(!app.history.can_undo());
    }

    #[test]
    fn batch_range_uses_visible_rows_and_filter_changes_keep_selection() {
        let mut app = Dudel {
            doc: stack(),
            ..Default::default()
        };
        app.doc.active = 1;
        app.doc.group_active().unwrap();
        app.doc.layers[2].collapsed = true;
        app.select_layer_row(4, false, false);
        app.select_layer_row(0, false, true);
        assert_eq!(app.layer_selection.ids.len(), 4);
        assert!(!app.layer_selection.ids.contains(&app.doc.layers[1].id));
        app.layer_navigation.query = "Layer".into();
        app.sync_layer_selection();
        assert_eq!(app.layer_selection.ids.len(), 4);
        app.select_layer_row(4, false, false);
        app.select_layer_row(0, false, true);
        assert_eq!(app.layer_selection.ids.len(), 4);
        assert!(app.layer_selection.ids.contains(&app.doc.layers[1].id));
        assert!(!app.layer_selection.ids.contains(&app.doc.layers[2].id));
        app.select_layer_row(0, true, false);
        assert_eq!(app.layer_selection.ids.len(), 3);
        assert!(
            app.layer_selection
                .ids
                .contains(&app.doc.layers[app.doc.active].id)
        );
    }

    #[test]
    fn batch_settles_earlier_queued_work_and_targets_surviving_identities() {
        let mut app = Dudel {
            doc: stack(),
            ..Default::default()
        };
        let a = app.doc.layers[1].id;
        let b = app.doc.layers[3].id;
        app.layer_selection.ids = vec![a, b];
        app.render_queue
            .edit(Edit::Property {
                target: crate::document_edits::Target::Identity(a),
                property: crate::document_edits::Property::Name("Queued name".into()),
            })
            .unwrap();
        app.batch_layers(Action::Visible(false));
        assert_eq!(
            app.doc.layers.iter().find(|l| l.id == a).unwrap().name,
            "Queued name"
        );
        assert!(
            app.doc
                .layers
                .iter()
                .filter(|l| [a, b].contains(&l.id))
                .all(|l| !l.visible)
        );
        app.undo();
        app.flush_render_queue();
        assert!(app.doc.layers.iter().all(|l| l.visible));
        assert_eq!(
            app.doc.layers.iter().find(|l| l.id == a).unwrap().name,
            "Queued name"
        );
    }
}
