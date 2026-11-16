//! Session selection and atomic, identity-targeted layer stack edits.
use crate::document::{Document, Layer, LayerId};

#[derive(Default)]
pub struct Selection {
    pub ids: Vec<LayerId>,
    pub anchor: Option<LayerId>,
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
}
