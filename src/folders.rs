//! Layer folder operations on the flat layer stack. A folder is a header
//! entry; its contents are the contiguous, deeper entries directly below it.
use crate::document::{Document, Layer, MAX_DEPTH};

impl Document {
    /// Move an entry (with its contents, for folders) so it sits directly
    /// above stack position `gap` (0 = bottom) at nesting level `depth`.
    pub fn move_block(&mut self, from: usize, gap: usize, depth: u8) -> Result<(), String> {
        let block = self.block(from);
        if gap > block.start && gap < block.end {
            return Err("A folder cannot be moved inside itself".into());
        }
        let old_depth = self.layers[from].depth;
        let deepest = self.layers[block.clone()]
            .iter()
            .map(|l| l.depth)
            .max()
            .unwrap_or(0);
        if deepest - old_depth + depth > MAX_DEPTH {
            return Err(format!("Folders nest at most {MAX_DEPTH} levels deep"));
        }
        let mut moved: Vec<Layer> = self.layers.drain(block.clone()).collect();
        for layer in &mut moved {
            layer.depth = layer.depth - old_depth + depth;
        }
        let gap = if gap >= block.end {
            gap - block.len()
        } else {
            gap
        };
        let count = moved.len();
        self.layers.splice(gap..gap, moved);
        self.active = gap + count - 1;
        self.validate_structure()
    }

    /// New empty folder directly above the active entry.
    pub fn add_folder(&mut self) -> Result<(), String> {
        if !self.has_room(0, 1) {
            return Err("The layer stack is full".into());
        }
        let block = self.block(self.active);
        let depth = self.layers[self.active].depth;
        let number = self.layers.iter().filter(|l| l.folder).count() + 1;
        let mut folder = Layer::new_folder(format!("Folder {number}"));
        folder.depth = depth;
        self.layers.insert(block.end, folder);
        self.active = block.end;
        self.validate_structure()
    }

    /// Put the active entry (and its contents) into a new folder.
    pub fn group_active(&mut self) -> Result<(), String> {
        if !self.has_room(0, 1) {
            return Err("The layer stack is full".into());
        }
        let block = self.block(self.active);
        let depth = self.layers[self.active].depth;
        if self.layers[block.clone()]
            .iter()
            .any(|l| l.depth >= MAX_DEPTH)
        {
            return Err(format!("Folders nest at most {MAX_DEPTH} levels deep"));
        }
        for layer in &mut self.layers[block.clone()] {
            layer.depth += 1;
        }
        let number = self.layers.iter().filter(|l| l.folder).count() + 1;
        let mut folder = Layer::new_folder(format!("Folder {number}"));
        folder.depth = depth;
        self.layers.insert(block.end, folder);
        self.active = block.end;
        self.validate_structure()
    }

    /// Remove the active folder, keeping its contents in place.
    pub fn ungroup_active(&mut self) -> Result<(), String> {
        if !self.layers[self.active].folder {
            return Err("Select a folder to ungroup".into());
        }
        if self.layers[self.active].mask.is_some() {
            return Err("Remove the folder mask before ungrouping, or merge the folder to retain its appearance".into());
        }
        let block = self.block(self.active);
        for layer in &mut self.layers[block.start..block.end - 1] {
            layer.depth -= 1;
        }
        self.layers.remove(self.active);
        if self.layers.is_empty() {
            return Err("A document needs at least one layer".into());
        }
        self.active = self.active.saturating_sub(1).min(self.layers.len() - 1);
        self.validate_structure()
    }

    /// Flatten the active folder into one layer that keeps the folder name,
    /// visibility, opacity and blend mode, so the composite is unchanged.
    pub fn merge_folder(&mut self) -> Result<(), String> {
        let header = &self.layers[self.active];
        if !header.folder {
            return Err("Select a folder to merge".into());
        }
        let block = self.block(self.active);
        let depth = header.depth;
        if self.layers[block.clone()].iter().all(|l| l.folder) && !self.has_room(1, 0) {
            return Err("The layer stack is full".into());
        }
        let mut contents = Document::new(1, 1);
        contents.version = self.version;
        contents.width = self.width;
        contents.height = self.height;
        contents.selection = None;
        contents.layers = self.layers[block.start..block.end - 1]
            .iter()
            .cloned()
            .map(|mut layer| {
                layer.depth -= depth + 1;
                layer
            })
            .collect();
        let mut merged = Layer::raster(header.name.clone(), 0, 0);
        merged.visible = header.visible;
        merged.opacity = header.opacity;
        merged.blend = header.blend;
        merged.clipped = header.clipped;
        merged.depth = depth;
        merged.mask = header.mask.clone();
        merged.pixels = if contents.layers.is_empty() {
            vec![0; self.width * self.height * 4].into()
        } else {
            contents.active = 0;
            contents.composite_transparent().into()
        };
        self.layers.splice(block.clone(), [merged]);
        self.active = block.start;
        self.validate_structure()
    }

    /// Delete the active entry and, for folders, everything inside.
    pub fn delete_active(&mut self) -> Result<(), String> {
        let block = self.block(self.active);
        if self.layers[block.clone()].iter().any(|l| l.locked) {
            return Err("Unlock the layer before deleting it".into());
        }
        if block.len() == self.layers.len()
            || self
                .layers
                .iter()
                .enumerate()
                .all(|(i, l)| l.folder || block.contains(&i))
        {
            return Err("A document needs at least one layer".into());
        }
        self.layers.drain(block.clone());
        self.active = block.start.min(self.layers.len() - 1);
        self.validate_structure()
    }

    /// Copy the active entry (with folder contents) directly above itself.
    pub fn duplicate_active(&mut self) -> Result<(), String> {
        let block = self.block(self.active);
        let rasters = self.layers[block.clone()]
            .iter()
            .filter(|l| !l.folder)
            .count();
        if !self.has_room(rasters, block.len()) {
            return Err("The layer stack is full".into());
        }
        let mut copy: Vec<Layer> = self.layers[block.clone()].to_vec();
        for layer in &mut copy {
            layer.id = crate::document::LayerId::new();
            if let Some(mask) = &mut layer.mask {
                mask.id = crate::document::LayerId::new();
            }
        }
        if let Some(header) = copy.last_mut() {
            header.name = format!("{} copy", header.name);
        }
        self.layers.splice(block.end..block.end, copy);
        self.active = block.end + block.len() - 1;
        self.validate_structure()
    }

    /// Swap the active entry with its neighbor sibling above (`up`) or below.
    pub fn step_active(&mut self, up: bool) -> Result<(), String> {
        let block = self.block(self.active);
        let depth = self.layers[self.active].depth;
        if up {
            // Entries deeper than this one belong to a sibling folder above;
            // its header is the first entry back at the same depth.
            let mut above = block.end;
            while above < self.layers.len() && self.layers[above].depth > depth {
                above += 1;
            }
            if above >= self.layers.len() || self.layers[above].depth != depth {
                return Err("Already at the top of its folder".into());
            }
            let sibling = self.block(above);
            self.move_block(self.active, sibling.end, depth)
        } else {
            if block.start == 0 || self.layers[block.start - 1].depth < depth {
                return Err("Already at the bottom of its folder".into());
            }
            // Valid stacks never hold a deeper entry directly below a layer
            // (folder contents are part of the folder block), so this is the
            // sibling itself or the header of a sibling folder.
            let below_header = block.start - 1;
            let sibling = self.block(below_header);
            self.move_block(self.active, sibling.start, depth)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{blend::BlendMode, document::Document};

    fn names(doc: &Document) -> Vec<(String, u8)> {
        doc.layers
            .iter()
            .map(|l| (l.name.clone(), l.depth))
            .collect()
    }

    fn sample() -> Document {
        // Stack (bottom to top): Base, Folder 1 [A, B]
        let mut doc = Document::new(1, 1);
        doc.layers[0].name = "Base".into();
        doc.add_layer();
        doc.layers[1].name = "A".into();
        doc.group_active().unwrap();
        doc.active = 1;
        doc.add_layer();
        let b = doc.layers.pop().unwrap();
        doc.layers.insert(2, b);
        doc.layers[2].name = "B".into();
        doc.layers[2].depth = 1;
        doc.validate().unwrap();
        doc
    }

    #[test]
    fn grouping_moving_and_ungrouping_keep_a_valid_structure() {
        let mut doc = sample();
        assert_eq!(
            names(&doc),
            vec![
                ("Base".into(), 0),
                ("A".into(), 1),
                ("B".into(), 1),
                ("Folder 1".into(), 0)
            ]
        );
        assert_eq!(doc.block(3), 1..4);
        assert_eq!(doc.parent(1), Some(3));
        // Move Base into the folder as its top child, then back out to the root.
        doc.move_block(0, 3, 1).unwrap();
        assert_eq!(names(&doc)[2], ("Base".into(), 1));
        assert_eq!(doc.layers[doc.active].name, "Base");
        doc.move_block(2, 4, 0).unwrap();
        assert_eq!(names(&doc)[3], ("Base".into(), 0));
        // A folder cannot be dropped inside itself.
        assert!(doc.move_block(2, 1, 1).is_err());
        doc.active = 2;
        doc.ungroup_active().unwrap();
        assert!(doc.layers.iter().all(|l| l.depth == 0 && !l.folder));
        // Invalid structures are rejected.
        let mut broken = sample();
        broken.layers[3].folder = false;
        broken.layers[3].pixels = vec![0; 4].into();
        assert!(broken.validate().is_err());
    }

    #[test]
    fn folders_composite_as_isolated_groups_with_opacity_visibility_and_blend() {
        let mut doc = sample();
        doc.layers[0].pixels = vec![200, 200, 200, 255].into();
        doc.layers[1].pixels = vec![100, 100, 100, 255].into();
        doc.layers[2].pixels = vec![0, 0, 0, 0].into();
        assert_eq!(doc.composite(), vec![100, 100, 100, 255]);
        doc.layers[3].opacity = 0.5;
        assert_eq!(doc.composite(), vec![150, 150, 150, 255]);
        doc.layers[3].opacity = 1.0;
        doc.layers[3].blend = BlendMode::Multiply;
        assert_eq!(doc.composite(), vec![78, 78, 78, 255]);
        doc.layers[3].visible = false;
        assert_eq!(doc.composite(), vec![200, 200, 200, 255]);
        assert!(!doc.shown(1));
        doc.active = 1;
        assert!(doc.paint_blocker().is_some());
        doc.active = 3;
        assert!(doc.paint_blocker().unwrap().contains("folder"));
    }

    #[test]
    fn merging_a_folder_preserves_the_composite_and_duplicates_copy_contents() {
        let mut doc = sample();
        doc.layers[0].pixels = vec![10, 200, 30, 255].into();
        doc.layers[1].pixels = vec![200, 0, 0, 128].into();
        doc.layers[2].pixels = vec![0, 0, 250, 64].into();
        doc.layers[3].opacity = 0.7;
        doc.active = 3;
        let mut copy = doc.clone();
        copy.duplicate_active().unwrap();
        assert_eq!(copy.layers.len(), 7);
        assert_eq!(copy.layers[6].name, "Folder 1 copy");
        copy.validate().unwrap();
        let expected = doc.composite();
        doc.merge_folder().unwrap();
        assert_eq!(doc.layers.len(), 2);
        assert!(!doc.layers[1].folder);
        let merged = doc.composite();
        for (a, b) in expected.iter().zip(&merged) {
            assert!(a.abs_diff(*b) <= 1, "{expected:?} vs {merged:?}");
        }
        doc.active = 1;
        doc.delete_active().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert!(doc.delete_active().is_err());
    }

    #[test]
    fn stepping_swaps_with_whole_sibling_blocks() {
        let mut doc = sample();
        doc.active = 0;
        doc.step_active(true).unwrap();
        assert_eq!(names(&doc)[3], ("Base".into(), 0));
        assert!(doc.step_active(true).is_err());
        doc.step_active(false).unwrap();
        assert_eq!(names(&doc)[0], ("Base".into(), 0));
        doc.active = 2;
        assert!(doc.step_active(true).is_err());
        doc.step_active(false).unwrap();
        assert_eq!(names(&doc)[1], ("B".into(), 1));
    }

    #[test]
    fn folders_round_trip_through_the_binary_project_format() {
        let mut doc = sample();
        doc.layers[3].collapsed = true;
        doc.layers[1].pixels = vec![1, 2, 3, 4].into();
        let mut bytes = vec![];
        crate::binary_project::encode(&doc, &mut bytes).unwrap();
        let restored =
            crate::binary_project::decode(&bytes[crate::binary_project::MAGIC.len()..]).unwrap();
        assert_eq!(names(&restored), names(&doc));
        assert!(restored.layers[3].folder && restored.layers[3].collapsed);
        assert!(restored.layers[3].pixels.is_empty());
        assert_eq!(restored.layers[1].pixels, vec![1, 2, 3, 4]);
    }
}
