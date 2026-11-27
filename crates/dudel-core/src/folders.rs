//! Layer folder operations on the flat layer stack. A folder is a header
//! entry; its contents are the contiguous, deeper entries directly below it.
use crate::document::{Document, Layer, LayerId, MAX_DEPTH};

/// A plain layer holding `pixels`.
fn raster(id: LayerId, name: String, pixels: Vec<u8>) -> Layer {
    let mut layer = Layer::raster(name, 0, 0);
    layer.id = id;
    layer.pixels = pixels.into();
    layer
}

/// Ways of making one layer out of several, beside Merge down and merging a
/// folder.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Combine {
    /// The active layer goes into the layer above, which keeps its name,
    /// opacity and blend mode (Merge down keeps the lower layer's).
    Up,
    /// These entries, folders with their contents, become one layer where
    /// the topmost of them was. Hidden ones are left as they are.
    Selected(Vec<LayerId>),
    /// Every visible top-level entry becomes one layer; hidden layers stay.
    Visible,
    /// A new top-level layer above the active entry shows everything
    /// visible; the layers it was made from stay.
    VisibleToNew,
    /// Everything visible becomes the only layer; hidden layers are dropped.
    Flatten,
}

impl Document {
    /// Carry out `how`; a newly made layer gets the identity `id`.
    pub fn combine(&mut self, how: &Combine, id: LayerId) -> Result<(), String> {
        if self.layers.iter().any(|l| l.id == id) {
            return Err("Layer identity is already present".into());
        }
        let whole_stack = matches!(
            how,
            Combine::Visible | Combine::VisibleToNew | Combine::Flatten
        );
        if whole_stack && self.animation.is_some() {
            return Err(
                "With the animation timeline on, each top-level layer is a frame; \
                 turn the timeline off to merge them"
                    .into(),
            );
        }
        match how {
            Combine::Up => self.merge_up(),
            Combine::Selected(targets) => {
                let mut roots = Vec::new();
                for target in targets {
                    let index = self
                        .layers
                        .iter()
                        .position(|l| l.id == *target)
                        .ok_or("A selected layer no longer exists")?;
                    if !roots.contains(&index) {
                        roots.push(index);
                    }
                }
                roots.sort_unstable();
                let all = roots.clone();
                // A selected folder already brings its selected contents.
                roots.retain(|&i| !all.iter().any(|&j| j != i && self.block(j).contains(&i)));
                roots.retain(|&i| self.layers[i].visible);
                if roots.len() < 2 && !roots.iter().any(|&i| self.layers[i].folder) {
                    return Err("Select two or more visible layers to merge".into());
                }
                let place = *roots.last().unwrap();
                let pixels = self.lifted(&roots).composite_transparent();
                let name = self.layers[place].name.clone();
                self.replace_blocks(&roots, place, raster(id, name, pixels))
            }
            Combine::Visible => {
                if !self.can_merge_visible() {
                    return Err("Fewer than two layers are visible; nothing to merge".into());
                }
                let roots: Vec<usize> = (0..self.layers.len())
                    .filter(|&i| self.layers[i].depth == 0 && self.layers[i].visible)
                    .collect();
                let top = self.root(self.active);
                let place = if roots.contains(&top) {
                    top
                } else {
                    *roots.last().unwrap()
                };
                // The whole stack, so the picture is exactly what shows.
                let pixels = self.lifted(&self.roots()).composite_transparent();
                let name = self.layers[place].name.clone();
                self.replace_blocks(&roots, place, raster(id, name, pixels))
            }
            Combine::VisibleToNew => {
                if !self.has_room(1, 1) {
                    return Err("The layer stack is full".into());
                }
                let pixels = self.lifted(&self.roots()).composite_transparent();
                let index = self.block(self.root(self.active)).end;
                self.layers
                    .insert(index, raster(id, "Merged visible".into(), pixels));
                self.active = index;
                self.validate_structure()
            }
            Combine::Flatten => {
                if self.layers.len() == 1 {
                    return Err("There is only one layer".into());
                }
                if self.layers.iter().any(|l| l.locked) {
                    return Err("Unlock every layer to flatten the image".into());
                }
                let pixels = self.lifted(&self.roots()).composite_transparent();
                self.layers = vec![raster(id, "Flattened".into(), pixels)];
                self.active = 0;
                self.validate_structure()
            }
        }
    }

    /// Whether the layer at `index` has a layer directly above it in the
    /// same folder to merge up into.
    pub fn layer_above(&self, index: usize) -> bool {
        let above = index + 1;
        !self.layers[index].folder
            && above < self.layers.len()
            && !self.layers[above].folder
            && self.layers[above].depth == self.layers[index].depth
    }

    /// Whether Merge visible has something to do: two visible top-level
    /// entries, or a visible folder.
    pub fn can_merge_visible(&self) -> bool {
        let mut visible = self.layers.iter().filter(|l| l.depth == 0 && l.visible);
        let count = visible.clone().count();
        count >= 2 || visible.any(|l| l.folder)
    }

    /// Every top-level entry, bottom first.
    fn roots(&self) -> Vec<usize> {
        (0..self.layers.len())
            .filter(|&i| self.layers[i].depth == 0)
            .collect()
    }

    /// The top-level entry holding `index`.
    fn root(&self, index: usize) -> usize {
        let mut top = index;
        while let Some(parent) = self.parent(top) {
            top = parent;
        }
        top
    }

    /// A document of only `roots` (bottom first, each with its contents),
    /// lifted to the top level, to composite them on their own. A root keeps
    /// its clipping only when what it clips to comes along.
    fn lifted(&self, roots: &[usize]) -> Document {
        let mut part = Document::new(1, 1);
        part.version = self.version;
        part.width = self.width;
        part.height = self.height;
        part.selection = None;
        part.layers.clear();
        for (n, &root) in roots.iter().enumerate() {
            let block = self.block(root);
            let depth = self.layers[root].depth;
            let had_base = block.start > 0 && self.layers[block.start - 1].depth == depth;
            let base_comes = n > 0 && roots[n - 1] + 1 == block.start;
            part.layers.extend(self.layers[block].iter().map(|layer| {
                let mut layer = layer.clone();
                layer.depth -= depth;
                layer.locked = false;
                layer
            }));
            if had_base && !base_comes {
                part.layers.last_mut().unwrap().clipped = false;
            }
        }
        part.active = 0;
        part
    }

    /// Put `merged` where `place` was and remove the blocks of `roots`.
    /// Hidden layers inside them were not part of the picture, so they stay,
    /// directly below `merged`. Fails, changing nothing, when a layer that
    /// would go is locked, by itself or through a folder around it.
    fn replace_blocks(
        &mut self,
        roots: &[usize],
        place: usize,
        mut merged: Layer,
    ) -> Result<(), String> {
        let depth = self.layers[place].depth;
        let mut taken = vec![false; self.layers.len()];
        // The new depth of each hidden layer that stays.
        let mut kept: Vec<Option<u8>> = vec![None; self.layers.len()];
        for &root in roots {
            let mut ancestor = self.parent(root);
            while let Some(index) = ancestor {
                if self.layers[index].locked {
                    return Err("Unlock the folders around the layers to merge them".into());
                }
                ancestor = self.parent(index);
            }
            // Whether `index` and the folders between it and `root` all show.
            let showing = |mut index: usize| loop {
                if !self.layers[index].visible {
                    return false;
                }
                if index == root {
                    return true;
                }
                index = self.parent(index).unwrap();
            };
            for index in self.block(root) {
                taken[index] = true;
                if index != root
                    && !self.layers[index].visible
                    && self.parent(index).is_some_and(showing)
                {
                    let top = self.layers[index].depth;
                    for inner in self.block(index) {
                        kept[inner] = Some(self.layers[inner].depth - top + depth);
                    }
                }
            }
        }
        if (0..self.layers.len()).any(|i| taken[i] && kept[i].is_none() && self.layers[i].locked) {
            return Err("Unlock the layers to merge them".into());
        }
        merged.depth = depth;
        let mut hidden = Vec::new();
        let mut rest = Vec::new();
        for (index, mut layer) in std::mem::take(&mut self.layers).into_iter().enumerate() {
            if let Some(depth) = kept[index] {
                layer.depth = depth;
                hidden.push(layer);
            } else if !taken[index] || index == place {
                rest.push((index, layer));
            }
        }
        let mut merged = Some(merged);
        for (index, layer) in rest {
            if index == place {
                self.layers.append(&mut hidden);
                self.active = self.layers.len();
                self.layers.extend(merged.take());
            } else {
                self.layers.push(layer);
            }
        }
        self.validate_structure()
    }

    /// The active layer merged into the layer above it, which keeps its
    /// identity, name, opacity and blend mode. The active layer's blend mode,
    /// opacity, mask and effects are rendered into the merged pixels.
    pub fn merge_up(&mut self) -> Result<(), String> {
        let lower = self.active;
        let upper = lower + 1;
        if self.layers[lower].folder {
            return Err("Use Merge folder to flatten a folder".into());
        }
        if !self.layer_above(lower) {
            return Err("There is no layer above in this folder to merge into".into());
        }
        if !self.layers[upper].visible || !self.layers[lower].visible {
            return Err("Show both layers before merging".into());
        }
        if self.layers[upper].adjustment.is_some() {
            return Err(
                "The layer above is an adjustment layer; it has no pixels to merge into".into(),
            );
        }
        if self.layers[upper].locked || self.layers[upper].alpha_locked {
            return Err("Unlock the layer above (pixels and transparency) to merge".into());
        }
        if self.layers[lower].locked {
            return Err("Unlock this layer to merge it".into());
        }
        let mut pair = Document::new(1, 1);
        pair.version = self.version;
        pair.width = self.width;
        pair.height = self.height;
        let mut base = self.layers[lower].clone();
        base.clipped = false;
        base.depth = 0;
        let mut top = self.layers[upper].clone();
        top.opacity = 1.0;
        top.blend = crate::blend::BlendMode::Normal;
        top.locked = false;
        top.depth = 0;
        pair.layers = vec![base, top];
        pair.selection = None;
        let merged = pair.composite_transparent();
        let clipped = self.layers[lower].clipped;
        let layer = &mut self.layers[upper];
        layer.pixels = merged.into();
        layer.clipped = clipped;
        layer.mask = None;
        layer.effects.clear();
        layer.fill = 1.0;
        layer.vector = None;
        layer.text = None;
        self.layers.remove(lower);
        self.active = lower;
        Ok(())
    }

    /// Move an entry (with its contents, for folders) so it sits directly
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

    /// New folders named `names` at the top of the stack, each holding an
    /// empty layer; the topmost folder's layer becomes active.
    pub fn add_named_folders(&mut self, names: &[String]) -> Result<(), String> {
        if names.is_empty() {
            return Ok(());
        }
        if self.animation.is_some() {
            return Err(
                "With the animation timeline on, each top-level layer is a frame;                  turn the timeline off to add folders at the top"
                    .into(),
            );
        }
        if !self.has_room(names.len(), names.len() * 2) {
            return Err("The layer stack is full".into());
        }
        for name in names {
            self.add_layer();
            let mut layer = self.layers.pop().unwrap();
            layer.depth = 1;
            self.layers.push(layer);
            self.layers.push(Layer::new_folder(name.clone()));
        }
        self.active = self.layers.len() - 2;
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
        // Effects follow the folder's composite, which these pixels now are.
        merged.effects = header.effects.clone();
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
    use super::Combine;
    use crate::{
        blend::BlendMode,
        document::{Document, LayerId},
    };

    fn names(doc: &Document) -> Vec<(String, u8)> {
        doc.layers
            .iter()
            .map(|l| (l.name.clone(), l.depth))
            .collect()
    }

    fn close(a: &[u8], b: &[u8]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!(x.abs_diff(*y) <= 1, "{a:?} vs {b:?}");
        }
    }

    /// Bottom to top: Base, Folder 1 [A, B], Top, each with paint.
    fn painted() -> Document {
        let mut doc = sample();
        doc.layers[0].pixels = vec![10, 200, 30, 255].into();
        doc.layers[1].pixels = vec![200, 0, 0, 128].into();
        doc.layers[2].pixels = vec![0, 0, 250, 64].into();
        doc.layers[3].opacity = 0.7;
        doc.add_layer();
        let top = doc.layers.len() - 1;
        doc.layers[top].name = "Top".into();
        doc.layers[top].pixels = vec![240, 220, 40, 150].into();
        doc.layers[top].blend = BlendMode::Multiply;
        doc.active = top;
        doc.validate().unwrap();
        doc
    }

    #[test]
    fn merging_visible_layers_keeps_the_picture_and_the_hidden_layers() {
        let mut doc = painted();
        doc.layers[2].visible = false;
        let hidden = doc.layers[2].id;
        let expected = doc.composite();
        doc.active = 0;
        let id = LayerId::new();
        doc.combine(&Combine::Visible, id).unwrap();
        close(&expected, &doc.composite());
        // Where the active layer was, with hidden B kept just below it.
        assert_eq!(names(&doc), [("B".into(), 0), ("Base".into(), 0)]);
        assert_eq!((doc.layers[0].id, doc.layers[1].id), (hidden, id));
        assert_eq!(doc.active, 1);
        assert!(!doc.layers[0].visible);
    }

    #[test]
    fn flattening_keeps_the_picture_and_drops_hidden_layers() {
        let mut doc = painted();
        doc.layers[1].visible = false;
        let expected = doc.composite();
        doc.combine(&Combine::Flatten, LayerId::new()).unwrap();
        assert_eq!(names(&doc), [("Flattened".into(), 0)]);
        close(&expected, &doc.composite());
        assert!(
            doc.combine(&Combine::Flatten, LayerId::new()).is_err(),
            "one layer is flat already"
        );
        let mut locked = painted();
        locked.layers[0].locked = true;
        assert!(locked.combine(&Combine::Flatten, LayerId::new()).is_err());
        // With the timeline on, top-level layers are frames.
        let mut animated = painted();
        animated.animation = Some(Default::default());
        for how in [Combine::Flatten, Combine::Visible, Combine::VisibleToNew] {
            assert!(animated.clone().combine(&how, LayerId::new()).is_err());
        }
    }

    #[test]
    fn merging_visible_to_a_new_layer_copies_the_picture_and_keeps_every_layer() {
        let before = painted();
        let mut doc = before.clone();
        doc.active = 1;
        doc.combine(&Combine::VisibleToNew, LayerId::new()).unwrap();
        // Above the folder holding A, at the top level.
        let mut expected = names(&before);
        expected.insert(4, ("Merged visible".into(), 0));
        assert_eq!(names(&doc), expected);
        assert_eq!(doc.active, 4);
        assert_eq!(
            doc.layers[4].pixels.rgba(0).to_vec(),
            before.composite_transparent()
        );
    }

    #[test]
    fn merging_selected_layers_puts_them_where_the_topmost_was() {
        let mut doc = painted();
        let mut alone = doc.clone();
        alone.layers[3].visible = false;
        let targets = vec![doc.layers[4].id, doc.layers[0].id];
        doc.combine(&Combine::Selected(targets), LayerId::new())
            .unwrap();
        assert_eq!(
            names(&doc),
            [
                ("A".into(), 1),
                ("B".into(), 1),
                ("Folder 1".into(), 0),
                ("Top".into(), 0)
            ]
        );
        assert_eq!(
            doc.layers[3].pixels.rgba(0).to_vec(),
            alone.composite_transparent()
        );
        // A folder with a hidden layer: the picture stays, the hidden layer too.
        let mut doc = painted();
        doc.layers[1].visible = false;
        let expected = doc.composite();
        let targets = vec![doc.layers[0].id, doc.layers[3].id, doc.layers[2].id];
        doc.combine(&Combine::Selected(targets.clone()), LayerId::new())
            .unwrap();
        assert_eq!(
            names(&doc),
            [("A".into(), 0), ("Folder 1".into(), 0), ("Top".into(), 0)]
        );
        close(&expected, &doc.composite());
        // A locked layer is not merged away, and nothing changes.
        let mut locked = painted();
        locked.layers[2].locked = true;
        let targets = vec![locked.layers[0].id, locked.layers[3].id];
        assert!(
            locked
                .combine(&Combine::Selected(targets), LayerId::new())
                .is_err()
        );
        let one = painted();
        assert!(
            one.clone()
                .combine(&Combine::Selected(vec![one.layers[0].id]), LayerId::new())
                .is_err()
        );
    }

    #[test]
    fn merging_up_keeps_the_upper_layer_and_the_picture() {
        let mut doc = Document::new(1, 1);
        doc.layers[0].name = "Sketch".into();
        doc.layers[0].pixels = vec![50, 60, 70, 255].into();
        doc.layers[0].opacity = 0.4;
        doc.add_layer();
        doc.layers[1].name = "Ink".into();
        doc.layers[1].pixels = vec![0, 0, 0, 100].into();
        let ink = doc.layers[1].id;
        let expected = doc.composite();
        doc.active = 1;
        assert!(!doc.layer_above(1));
        assert!(doc.merge_up().is_err(), "nothing above the top layer");
        doc.active = 0;
        doc.merge_up().unwrap();
        assert_eq!(names(&doc), [("Ink".into(), 0)]);
        assert_eq!((doc.layers[0].id, doc.active), (ink, 0));
        close(&expected, &doc.composite());
        // The upper layer's settings stay; it takes the lower one's place.
        let mut doc = painted();
        doc.add_layer();
        let up = doc.layers.len() - 1;
        doc.layers[up].opacity = 0.6;
        doc.active = up - 1;
        doc.merge_up().unwrap();
        let merged = doc.layers.last().unwrap();
        assert_eq!((merged.opacity, merged.blend), (0.6, BlendMode::Normal));
        // Inside a folder, only a layer in the same folder counts.
        doc.active = 2;
        assert!(doc.merge_up().is_err());
        doc.active = 1;
        doc.merge_up().unwrap();
        assert_eq!(doc.layers[1].name, "B");
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

    #[test]
    fn named_folders_go_on_top_each_with_a_layer_to_paint_on() {
        let mut doc = Document::new(8, 8);
        doc.add_named_folders(&["Body".into(), "Eyes".into()])
            .unwrap();
        let names: Vec<_> = doc
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.folder, l.depth))
            .collect();
        assert_eq!(
            names[1..].iter().map(|n| (n.1, n.2)).collect::<Vec<_>>(),
            [(false, 1), (true, 0), (false, 1), (true, 0)]
        );
        assert_eq!((names[2].0, names[4].0), ("Body", "Eyes"));
        assert_eq!(doc.active, 3);
        assert!(!doc.layers[doc.active].folder);
    }
}
