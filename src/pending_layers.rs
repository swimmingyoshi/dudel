//! Bounded metadata only; no pixel payloads or names are retained by this context.
//! Used to plan reserved raster creation while earlier painting is still pending.
use crate::document::{Document, LayerId, MAX_DEPTH, MAX_ENTRIES, MAX_RASTER_LAYERS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaskProperties {
    pub id: LayerId,
    pub enabled: bool,
    pub linked: bool,
}
impl MaskProperties {
    fn from_mask(mask: &crate::layer_mask::LayerMask) -> Self {
        Self {
            id: mask.id,
            enabled: mask.enabled,
            linked: mask.linked,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Properties {
    pub opacity: f32,
    pub csp_byte_opacity: bool,
    pub blend: crate::blend::BlendMode,
    pub clipped: bool,
    pub locked: bool,
    pub alpha_locked: bool,
    pub mask: Option<MaskProperties>,
}
impl Properties {
    pub fn from_layer(layer: &crate::document::Layer) -> Self {
        Self {
            opacity: layer.opacity,
            csp_byte_opacity: layer.csp_byte_opacity,
            blend: layer.blend,
            clipped: layer.clipped,
            locked: layer.locked,
            alpha_locked: layer.alpha_locked,
            mask: layer.mask.as_ref().map(MaskProperties::from_mask),
        }
    }
}

#[derive(Clone)]
struct Entry {
    id: LayerId,
    folder: bool,
    depth: u8,
    collapsed: bool,
    visible: bool,
    locked: bool,
    alpha_locked: bool,
    opacity: f32,
    csp_byte_opacity: bool,
    blend: crate::blend::BlendMode,
    clipped: bool,
    mask: Option<MaskProperties>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_projection_matches_nested_document_siblings_and_boundaries() {
        let mut doc = Document::new(8, 8);
        doc.add_layer();
        doc.group_active().unwrap();
        doc.group_active().unwrap();
        doc.duplicate_active().unwrap();
        doc.add_layer();
        doc.validate().unwrap();
        for index in 0..doc.layers.len() {
            for up in [false, true] {
                let mut expected = doc.clone();
                expected.active = index;
                let mut projected = PendingLayers::from_document(&doc);
                let target = doc.layers[index].id;
                for _ in 0..5 {
                    let result = expected.step_active(up);
                    let projection = projected.with_step(target, up);
                    assert_eq!(projection.is_ok(), result.is_ok(), "index={index}, up={up}");
                    if let Ok(next) = projection {
                        projected = next;
                        assert_eq!(
                            projected
                                .entries
                                .iter()
                                .map(|entry| (entry.id, entry.folder, entry.depth))
                                .collect::<Vec<_>>(),
                            expected
                                .layers
                                .iter()
                                .map(|layer| (layer.id, layer.folder, layer.depth))
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(expected.layers[expected.active].id, target);
                    }
                }
            }
        }
    }

    #[test]
    fn projected_properties_match_typed_edits_through_folder_merge_and_copy() {
        use crate::document_edits::{Edit, Property, Target};
        let mut doc = Document::new(8, 8);
        doc.group_active().unwrap();
        doc.active = 0;
        let folder = doc.layers[1].id;
        let child = doc.layers[0].id;
        let mut context = PendingLayers::from_document(&doc);
        for (target, property) in [
            (child, Property::Locked(true)),
            (child, Property::AlphaLocked(true)),
            (folder, Property::Opacity(0.4)),
            (folder, Property::Blend(crate::blend::BlendMode::Multiply)),
            (folder, Property::Clipped(true)),
            (folder, Property::Visible(false)),
            (folder, Property::Collapsed(true)),
            (folder, Property::Collapsed(false)),
        ] {
            let (next, active) = context
                .with_property(target, doc.layers[doc.active].id, &property)
                .unwrap();
            doc = Edit::Property {
                target: Target::Identity(target),
                property,
            }
            .prepare(&doc)
            .unwrap()
            .0;
            context = next;
            assert_eq!(active, doc.layers[doc.active].id);
            for layer in &doc.layers {
                assert_eq!(
                    context.properties(layer.id),
                    Some(Properties::from_layer(layer))
                );
                assert_eq!(
                    context.paint_blocker(layer.id).is_some(),
                    doc.paint_blocker_for(
                        doc.layers
                            .iter()
                            .position(|candidate| candidate.id == layer.id)
                            .unwrap()
                    )
                    .is_some()
                );
            }
        }
        let (next, merged, folder_result) = context.with_merge(folder).unwrap();
        doc = Edit::MergeTarget {
            target: folder,
            folder_result,
        }
        .prepare(&doc)
        .unwrap()
        .0;
        assert_eq!(
            next.properties(merged),
            Some(Properties::from_layer(&doc.layers[doc.active]))
        );
        assert!(!next.alpha_locked(merged));
        let (copied, copies, selected) = next.with_duplicate(merged).unwrap();
        doc = Edit::DuplicateTarget {
            target: merged,
            copies,
        }
        .prepare(&doc)
        .unwrap()
        .0;
        assert_eq!(
            copied.properties(selected),
            Some(Properties::from_layer(&doc.layers[doc.active]))
        );
        assert_eq!(copied.properties(selected), copied.properties(merged));
    }
}
#[derive(Clone)]
pub struct PendingLayers {
    entries: Vec<Entry>,
}
impl PendingLayers {
    pub fn properties(&self, id: LayerId) -> Option<Properties> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| Properties {
                opacity: entry.opacity,
                csp_byte_opacity: entry.csp_byte_opacity,
                blend: entry.blend,
                clipped: entry.clipped,
                locked: entry.locked,
                alpha_locked: entry.alpha_locked,
                mask: entry.mask,
            })
    }
    pub fn with_property(
        &self,
        target: LayerId,
        active: LayerId,
        property: &crate::document_edits::Property,
    ) -> Result<(Self, LayerId), String> {
        use crate::document_edits::{Edit, Property, Target};
        Edit::Property {
            target: Target::Identity(target),
            property: property.clone(),
        }
        .validate_arguments()?;
        let block = self.block(target)?;
        let index = block.end - 1;
        let mut updated = self.clone();
        let entry = &mut updated.entries[index];
        let mut selected = active;
        match property {
            Property::Name(_) => {} // Names are deliberately not retained.
            Property::Visible(value) => entry.visible = *value,
            Property::Opacity(value) => entry.opacity = *value,
            Property::Blend(value) => entry.blend = *value,
            Property::Clipped(value) => entry.clipped = *value,
            Property::Locked(value) => entry.locked = *value,
            Property::AlphaLocked(value) => entry.alpha_locked = *value,
            Property::Collapsed(value) => {
                if !entry.folder {
                    return Err("Collapse applies only to folders".into());
                }
                if entry.collapsed != *value
                    && *value
                    && self.entries[block].iter().any(|entry| entry.id == active)
                {
                    selected = target;
                }
                entry.collapsed = *value;
            }
        }
        Ok((updated, selected))
    }
    pub fn with_toggle(
        &self,
        target: LayerId,
        active: LayerId,
        collapse: bool,
    ) -> Result<(Self, bool, LayerId), String> {
        let block = self.block(target)?;
        let index = block.end - 1;
        let mut updated = self.clone();
        let value = if collapse {
            if !self.entries[index].folder {
                return Err("Select a folder to collapse".into());
            }
            updated.entries[index].collapsed = !self.entries[index].collapsed;
            updated.entries[index].collapsed
        } else {
            updated.entries[index].visible = !self.entries[index].visible;
            updated.entries[index].visible
        };
        let active =
            if collapse && value && self.entries[block].iter().any(|entry| entry.id == active) {
                target
            } else {
                active
            };
        Ok((updated, value, active))
    }
    pub fn with_step(&self, active: LayerId, up: bool) -> Result<Self, String> {
        let block = self.block(active)?;
        let depth = self.entries[block.end - 1].depth;
        let gap = if up {
            let mut above = block.end;
            while above < self.entries.len() && self.entries[above].depth > depth {
                above += 1;
            }
            if above >= self.entries.len() || self.entries[above].depth != depth {
                return Err("Already at the top of its folder".into());
            }
            self.block(self.entries[above].id)?.end
        } else {
            if block.start == 0 || self.entries[block.start - 1].depth < depth {
                return Err("Already at the bottom of its folder".into());
            }
            self.block(self.entries[block.start - 1].id)?.start
        };
        let before = self.entries.get(gap).map(|entry| entry.id);
        self.with_reorder(active, before, depth)
            .map(|(context, _)| context)
    }
    pub fn with_merge(&self, active: LayerId) -> Result<(Self, LayerId, Option<LayerId>), String> {
        let block = self.block(active)?;
        let header = &self.entries[block.end - 1];
        let mut updated = self.clone();
        let (active, reserved) = if header.folder {
            let rasters = self.entries.iter().filter(|entry| !entry.folder).count();
            let removed = self.entries[block.clone()]
                .iter()
                .filter(|entry| !entry.folder)
                .count();
            if rasters - removed + 1 > MAX_RASTER_LAYERS {
                return Err("The layer stack is full".into());
            }
            let id = LayerId::new();
            let mut merged = header.clone();
            merged.id = id;
            merged.folder = false;
            // Layer::raster supplies these defaults in Document::merge_folder.
            merged.collapsed = false;
            merged.locked = false;
            merged.alpha_locked = false;
            updated.entries.splice(block, [merged]);
            (id, Some(id))
        } else {
            let upper = block.start;
            let lower = upper
                .checked_sub(1)
                .ok_or("There is no layer below to merge into")?;
            let base = &self.entries[lower];
            if base.folder || base.depth != header.depth {
                return Err("The layer below is a folder or outside this folder".into());
            }
            if !base.visible || !header.visible {
                return Err("Show both layers before merging".into());
            }
            if base.locked || base.alpha_locked {
                return Err("Unlock the layer below (pixels and transparency) to merge".into());
            }
            updated.entries.remove(upper);
            updated.entries[lower].mask = None; // Both leaf masks are baked once.
            (base.id, None)
        };
        crate::document::validate_folder_depths(
            updated
                .entries
                .iter()
                .map(|entry| (entry.folder, entry.depth)),
        )?;
        Ok((updated, active, reserved))
    }
    pub fn alpha_locked(&self, id: LayerId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.id == id && entry.alpha_locked)
    }
    pub fn with_duplicate(&self, active: LayerId) -> Result<(Self, Vec<LayerId>, LayerId), String> {
        let block = self.block(active)?;
        let rasters = self.entries.iter().filter(|entry| !entry.folder).count();
        let copies = self.entries[block.clone()]
            .iter()
            .filter(|entry| !entry.folder)
            .count();
        if self.entries.len() + block.len() > MAX_ENTRIES || rasters + copies > MAX_RASTER_LAYERS {
            return Err("Layer stack capacity reached".into());
        }
        let mut copied = self.entries[block.clone()].to_vec();
        let ids: Vec<_> = copied
            .iter_mut()
            .map(|entry| {
                entry.id = LayerId::new();
                if let Some(mask) = &mut entry.mask {
                    mask.id = entry.id;
                }
                entry.id
            })
            .collect();
        let active = *ids.last().unwrap();
        let mut updated = self.clone();
        updated.entries.splice(block.end..block.end, copied);
        crate::document::validate_folder_depths(
            updated
                .entries
                .iter()
                .map(|entry| (entry.folder, entry.depth)),
        )?;
        Ok((updated, ids, active))
    }
    /// Source identities in the same bottom-to-top order as duplicate copies,
    /// including nested folder headers. Works before queued entries exist in Doc.
    pub fn block_ids(&self, active: LayerId) -> Result<Vec<LayerId>, String> {
        Ok(self.entries[self.block(active)?]
            .iter()
            .map(|entry| entry.id)
            .collect())
    }
    pub fn with_delete(&self, active: LayerId) -> Result<(Self, LayerId), String> {
        let block = self.block(active)?;
        if self.entries[block.clone()].iter().any(|entry| entry.locked) {
            return Err("Unlock the layer before deleting it".into());
        }
        if block.len() == self.entries.len()
            || !self
                .entries
                .iter()
                .enumerate()
                .any(|(index, entry)| !entry.folder && !block.contains(&index))
        {
            return Err("A document needs at least one layer".into());
        }
        let mut updated = self.clone();
        updated.entries.drain(block.clone());
        let active = updated.entries[block.start.min(updated.entries.len() - 1)].id;
        crate::document::validate_folder_depths(
            updated
                .entries
                .iter()
                .map(|entry| (entry.folder, entry.depth)),
        )?;
        Ok((updated, active))
    }
    pub fn is_folder(&self, id: LayerId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.id == id && entry.folder)
    }
    fn block(&self, id: LayerId) -> Result<std::ops::Range<usize>, String> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or("Selected layer no longer exists")?;
        let mut start = index;
        if self.entries[index].folder {
            while start > 0 && self.entries[start - 1].depth > self.entries[index].depth {
                start -= 1;
            }
        }
        Ok(start..index + 1)
    }
    pub fn with_folder(&self, active: LayerId, id: LayerId, group: bool) -> Result<Self, String> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err("Layer stack capacity reached".into());
        }
        let block = self.block(active)?;
        let depth = self.entries[block.end - 1].depth;
        let mut updated = self.clone();
        if group {
            if self.entries[block.clone()]
                .iter()
                .any(|entry| entry.depth >= MAX_DEPTH)
            {
                return Err("Folder nesting capacity reached".into());
            }
            for entry in &mut updated.entries[block.clone()] {
                entry.depth += 1;
            }
        }
        updated.entries.insert(
            block.end,
            Entry {
                id,
                folder: true,
                depth,
                collapsed: false,
                visible: true,
                locked: false,
                alpha_locked: false,
                opacity: 1.0,
                csp_byte_opacity: false,
                blend: Default::default(),
                clipped: false,
                mask: None,
            },
        );
        crate::document::validate_folder_depths(
            updated
                .entries
                .iter()
                .map(|entry| (entry.folder, entry.depth)),
        )?;
        Ok(updated)
    }
    pub fn with_ungroup(&self, active: LayerId) -> Result<(Self, LayerId), String> {
        let block = self.block(active)?;
        let index = block.end - 1;
        if !self.entries[index].folder {
            return Err("Select a folder to ungroup".into());
        }
        if self.entries[index].mask.is_some() {
            return Err("Remove the folder mask or merge the folder before ungrouping".into());
        }
        if self.entries.len() == 1 {
            return Err("A document needs at least one layer".into());
        }
        let mut updated = self.clone();
        for entry in &mut updated.entries[block.start..index] {
            entry.depth -= 1;
        }
        updated.entries.remove(index);
        let active = updated.entries[index.saturating_sub(1).min(updated.entries.len() - 1)].id;
        crate::document::validate_folder_depths(
            updated
                .entries
                .iter()
                .map(|entry| (entry.folder, entry.depth)),
        )?;
        Ok((updated, active))
    }
    pub fn with_reorder(
        &self,
        from: LayerId,
        before: Option<LayerId>,
        depth: u8,
    ) -> Result<(Self, bool), String> {
        let from = self
            .entries
            .iter()
            .position(|entry| entry.id == from)
            .ok_or("Moved layer no longer exists")?;
        let gap = if let Some(before) = before {
            self.entries
                .iter()
                .position(|entry| entry.id == before)
                .ok_or("Destination layer no longer exists")?
        } else {
            self.entries.len()
        };
        let old_depth = self.entries[from].depth;
        let mut start = from;
        if self.entries[from].folder {
            while start > 0 && self.entries[start - 1].depth > old_depth {
                start -= 1;
            }
        }
        let end = from + 1;
        if (gap == start || gap == end) && depth == old_depth {
            return Ok((self.clone(), false));
        }
        if gap > start && gap < end {
            return Err("A folder cannot be moved inside itself".into());
        }
        let deepest = self.entries[start..end]
            .iter()
            .map(|entry| entry.depth)
            .max()
            .unwrap();
        if depth > MAX_DEPTH || deepest - old_depth + depth > MAX_DEPTH {
            return Err("Folder nesting capacity reached".into());
        }
        let mut updated = self.clone();
        let mut moved: Vec<_> = updated.entries.drain(start..end).collect();
        for entry in &mut moved {
            entry.depth = entry.depth - old_depth + depth;
        }
        let gap = if gap >= end { gap - moved.len() } else { gap };
        updated.entries.splice(gap..gap, moved);
        crate::document::validate_folder_depths(
            updated
                .entries
                .iter()
                .map(|entry| (entry.folder, entry.depth)),
        )?;
        Ok((updated, true))
    }
    pub fn contains(&self, id: LayerId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
    pub fn from_document(doc: &Document) -> Self {
        Self {
            entries: doc
                .layers
                .iter()
                .map(|layer| Entry {
                    id: layer.id,
                    folder: layer.folder,
                    depth: layer.depth,
                    collapsed: layer.collapsed,
                    visible: layer.visible,
                    locked: layer.locked,
                    alpha_locked: layer.alpha_locked,
                    mask: layer.mask.as_ref().map(MaskProperties::from_mask),
                    opacity: layer.opacity,
                    csp_byte_opacity: layer.csp_byte_opacity,
                    blend: layer.blend,
                    clipped: layer.clipped,
                })
                .collect(),
        }
    }
    pub fn with_new_layer(&self, active: LayerId, id: LayerId) -> Result<Self, String> {
        if self.entries.len() >= MAX_ENTRIES
            || self.entries.iter().filter(|entry| !entry.folder).count() >= MAX_RASTER_LAYERS
        {
            return Err("Document layer capacity reached".into());
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == active)
            .ok_or("Selected layer no longer exists")?;
        let active = &self.entries[index];
        let (index, depth) = if active.folder && !active.collapsed {
            (index, active.depth + 1)
        } else {
            (index + 1, active.depth)
        };
        if depth > MAX_DEPTH {
            return Err("Folder nesting capacity reached".into());
        }
        let mut updated = self.clone();
        updated.entries.insert(
            index,
            Entry {
                id,
                folder: false,
                depth,
                collapsed: false,
                visible: true,
                locked: false,
                alpha_locked: false,
                opacity: 1.0,
                csp_byte_opacity: false,
                blend: Default::default(),
                clipped: false,
                mask: None,
            },
        );
        Ok(updated)
    }
    /// Visibility follows projected ancestry. Existing layer pixel locks are
    /// checked separately against the authoritative document.
    pub fn paint_blocker(&self, id: LayerId) -> Option<&'static str> {
        self.target_blocker(id, crate::paint_target::Target::Raster, false)
    }
    pub fn target_blocker(
        &self,
        id: LayerId,
        target: crate::paint_target::Target,
        erase: bool,
    ) -> Option<&'static str> {
        let Some(mut index) = self.entries.iter().position(|entry| entry.id == id) else {
            return Some("Selected layer no longer exists");
        };
        if target.is_raster() && self.entries[index].folder {
            return Some("Select a raster layer to draw");
        }
        if let crate::paint_target::Target::Mask(mask) = target
            && !self.entries[index].mask.is_some_and(|m| m.id == mask)
        {
            return Some("Captured mask was removed or replaced");
        }
        if target.is_raster() && erase && self.entries[index].alpha_locked {
            return Some("Unlock transparency before erasing");
        }
        if self.entries[index].locked {
            return Some("Layer is locked — unlock it to draw");
        }
        loop {
            if !self.entries[index].visible {
                return Some("Layer or its folder is hidden");
            }
            let depth = self.entries[index].depth;
            if depth == 0 {
                return None;
            }
            let Some(parent) =
                (index + 1..self.entries.len()).find(|&parent| self.entries[parent].depth < depth)
            else {
                return Some("Invalid pending folder context");
            };
            index = parent;
        }
    }
    pub fn with_mask_command(
        &self,
        target: LayerId,
        command: crate::layer_mask_edits::Command,
    ) -> Result<Self, String> {
        use crate::layer_mask_edits::Command;
        let index = self.block(target)?.end - 1;
        let source = &self.entries[index];
        if source.locked {
            return Err("Unlock the layer before editing its mask".into());
        }
        let mut updated = self.clone();
        let mask = &mut updated.entries[index].mask;
        match command {
            Command::Create { id, .. } => {
                if mask.is_some() {
                    return Err("This layer already has a mask".into());
                }
                *mask = Some(MaskProperties {
                    id,
                    enabled: true,
                    linked: true,
                });
            }
            Command::Enabled { id, .. }
            | Command::Linked { id, .. }
            | Command::Invert(id)
            | Command::Apply(id)
            | Command::Remove(id) => {
                let current = mask
                    .as_mut()
                    .filter(|m| m.id == id)
                    .ok_or("Captured mask was removed or replaced")?;
                match command {
                    Command::Enabled { value, .. } => current.enabled = value,
                    Command::Linked { value, .. } => current.linked = value,
                    Command::Apply(_) => {
                        if source.folder {
                            return Err("Merge the folder before applying its mask".into());
                        }
                        if current.enabled && source.alpha_locked {
                            return Err("Unlock transparency before applying the mask".into());
                        }
                        *mask = None;
                    }
                    Command::Remove(_) => *mask = None,
                    Command::Invert(_) => {}
                    Command::Create { .. } | Command::Clear { .. } | Command::Fill { .. } => {
                        unreachable!()
                    }
                }
            }
            Command::Clear { id, .. } | Command::Fill { id, .. } => {
                if let Some(error) =
                    self.target_blocker(target, crate::paint_target::Target::Mask(id), false)
                {
                    return Err(error.into());
                }
                if let Command::Fill { request, .. } = command {
                    request.validate()?;
                }
            }
        }
        Ok(updated)
    }
}
