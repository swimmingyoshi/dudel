use crate::{document::Document, pixels::Pixels, selection::Selection};

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Transform {
    FlipHorizontal,
    FlipVertical,
    RotateHalfTurn,
}
#[derive(Clone, Copy)]
pub(crate) enum Remap {
    Flip(Transform, Selection),
    Offset([i64; 2]),
}
impl Remap {
    pub(crate) fn point(
        self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        inverse: bool,
    ) -> Option<[usize; 2]> {
        let (tx, ty) = match self {
            Self::Flip(transform, bounds) => {
                if !bounds.contains(x, y) {
                    return None;
                }
                let point = match transform {
                    Transform::FlipHorizontal => (bounds.left + bounds.right - 1 - x, y),
                    Transform::FlipVertical => (x, bounds.top + bounds.bottom - 1 - y),
                    Transform::RotateHalfTurn => (
                        bounds.left + bounds.right - 1 - x,
                        bounds.top + bounds.bottom - 1 - y,
                    ),
                };
                (point.0 as i64, point.1 as i64)
            }
            Self::Offset([dx, dy]) => {
                if inverse {
                    ((x as i64).checked_sub(dx)?, (y as i64).checked_sub(dy)?)
                } else {
                    ((x as i64).checked_add(dx)?, (y as i64).checked_add(dy)?)
                }
            }
        };
        (tx >= 0 && ty >= 0 && tx < width as i64 && ty < height as i64)
            .then_some([tx as usize, ty as usize])
    }
}
/// Coverage shared by old/new selection is replaced, while coverage arriving
/// outside the old shape is composited over retained pixels. This avoids alpha
/// erosion when feathered regions overlap. Coverage overlap uses min(old,new).
pub(crate) fn transferred(
    before: [u8; 4],
    source: [u8; 4],
    departing: u8,
    incoming: u8,
) -> [u8; 4] {
    let (d, i, sa, da) = (
        u64::from(departing),
        u64::from(incoming),
        u64::from(source[3]),
        u64::from(before[3]),
    );
    let old_weight = (255 - d) * 255 - i.saturating_sub(d) * sa;
    let new_weight = i * 255;
    let alpha = da * old_weight + sa * new_weight;
    if alpha == 0 {
        return if incoming == 255 {
            source
        } else if departing == 255 {
            [0; 4]
        } else {
            before
        };
    }
    let mut result = [0; 4];
    result[3] = ((alpha + 32_512) / 65_025) as u8;
    for c in 0..3 {
        result[c] = (u64::from(before[c]) * da * old_weight
            + u64::from(source[c]) * sa * new_weight
            + alpha / 2)
            .checked_div(alpha)
            .unwrap_or(0) as u8;
    }
    result
}
/// Compute each pixel from frozen original data, preserving overlap coverage.
pub(crate) fn remapped_masked(
    source: &Pixels,
    width: usize,
    height: usize,
    mask: &crate::selection_mask::Mask,
    map: Remap,
) -> Result<(Pixels, crate::selection_mask::Mask), String> {
    remapped_masked_control(source, width, height, mask, map, None)
}
pub(crate) fn remapped_masked_control(
    source: &Pixels,
    width: usize,
    height: usize,
    mask: &crate::selection_mask::Mask,
    map: Remap,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(Pixels, crate::selection_mask::Mask), String> {
    crate::fill::check_cancel(cancel)?;
    if mask.dimensions() != [width, height] || source.len() != width * height * 4 {
        return Err("Mask transform dimensions differ".into());
    }
    let Some(bounds) = mask.bounds() else {
        return Ok((source.clone(), mask.clone()));
    };
    if matches!(map, Remap::Offset([0, 0])) {
        return Ok((source.clone(), mask.clone()));
    }
    let mut coverage = vec![0; width * height];
    for y in bounds.top..bounds.bottom {
        crate::fill::check_cancel(cancel)?;
        for x in bounds.left..bounds.right {
            if let Some([tx, ty]) = map.point(x, y, width, height, false) {
                coverage[ty * width + tx] = mask.at(x, y);
            }
        }
    }
    let mut moved =
        crate::selection_mask::Mask::from_coverage_control(width, height, coverage, cancel)?;
    if moved == *mask {
        moved = mask.clone();
    }
    let target = moved.bounds();
    let rect = target.map_or(bounds, |b| Selection {
        left: bounds.left.min(b.left),
        top: bounds.top.min(b.top),
        right: bounds.right.max(b.right),
        bottom: bounds.bottom.max(b.bottom),
    });
    let mut out = source.clone();
    let mut changed = false;
    {
        let mut writer = out.writer();
        for y in rect.top..rect.bottom {
            crate::fill::check_cancel(cancel)?;
            for x in rect.left..rect.right {
                let original = source.rgba(y * width + x);
                let departing = mask.at(x, y);
                let (pixel, incoming) = map
                    .point(x, y, width, height, true)
                    .map_or(([0; 4], 0), |[sx, sy]| {
                        (source.rgba(sy * width + sx), mask.at(sx, sy))
                    });
                let result = transferred(original, pixel, departing, incoming);
                if result != original {
                    writer.rgba_mut(y * width + x).copy_from_slice(&result);
                    changed = true;
                }
            }
        }
        if changed {
            writer.reclaim_region(width, [rect.left, rect.top, rect.right, rect.bottom]);
        }
    }
    crate::fill::check_cancel(cancel)?;
    Ok((if changed { out } else { source.clone() }, moved))
}

impl Document {
    pub fn transform_active(&mut self, transform: Transform) -> Result<bool, String> {
        self.transform_active_control(transform, None)
    }
    pub(crate) fn transform_active_control(
        &mut self,
        transform: Transform,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<bool, String> {
        if self.version == 2 {
            let mut result = self.clone();
            let changed = result.transform_active_pixels(transform, cancel)?;
            result.validate()?;
            crate::fill::check_cancel(cancel)?;
            *self = result;
            return Ok(changed);
        }
        self.transform_active_pixels(transform, cancel)
    }
    fn transform_active_pixels(
        &mut self,
        transform: Transform,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<bool, String> {
        crate::fill::check_cancel(cancel)?;
        let layer = &self.layers[self.active];
        if layer.folder {
            return Err("Select a layer inside the folder to transform it".into());
        }
        if layer.locked || layer.alpha_locked {
            return Err("Unlock pixels and transparency before transforming".into());
        }
        let bounds = self.selection.unwrap_or(Selection {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        });
        let rect = [bounds.left, bounds.top, bounds.right, bounds.bottom];
        if let Some(mask) = &self.selection_mask {
            let (pixels, moved) = remapped_masked_control(
                &layer.pixels,
                self.width,
                self.height,
                mask,
                Remap::Flip(transform, bounds),
                cancel,
            )?;
            let changed = !pixels.shares(&layer.pixels) || moved != *mask;
            if changed {
                self.layers[self.active].pixels = pixels;
                self.selection = moved.bounds();
                self.selection_mask = Some(moved);
            }
            return Ok(changed);
        }
        let linked_mask = crate::layer_mask::for_raster_transform(self, cancel, |mask| {
            mask.remap(self.width, self.height, 255, cancel, |[x, y]| {
                Some(match transform {
                    Transform::FlipHorizontal => [self.width - 1 - x, y],
                    Transform::FlipVertical => [x, self.height - 1 - y],
                    Transform::RotateHalfTurn => [self.width - 1 - x, self.height - 1 - y],
                })
            })
        })?;
        let mask_changed = !crate::layer_mask::same_snapshot(&linked_mask, &layer.mask);
        let whole = rect == [0, 0, self.width, self.height];
        let mut pixels = if whole {
            Pixels::sparse(self.width, self.height)
        } else {
            layer.pixels.clone()
        };
        if !whole {
            pixels.clear_region(self.width, rect);
        }
        let mut changed = false;
        {
            let mut writer = pixels.writer();
            layer
                .pixels
                .visit_nonzero(self.width, self.height, rect, |x, y, pixel| {
                    let (tx, ty) = match transform {
                        Transform::FlipHorizontal => (bounds.left + bounds.right - 1 - x, y),
                        Transform::FlipVertical => (x, bounds.top + bounds.bottom - 1 - y),
                        Transform::RotateHalfTurn => (
                            bounds.left + bounds.right - 1 - x,
                            bounds.top + bounds.bottom - 1 - y,
                        ),
                    };
                    changed |= layer.pixels.rgba(ty * self.width + tx) != pixel;
                    writer
                        .rgba_mut(ty * self.width + tx)
                        .copy_from_slice(&pixel);
                });
        }
        crate::fill::check_cancel(cancel)?;
        if changed {
            self.layers[self.active].pixels = pixels;
        }
        if mask_changed {
            self.layers[self.active].mask = linked_mask;
        }
        Ok(changed || mask_changed)
    }

    /// Rotate the document coordinate system, retaining every layer pixel,
    /// layer property and selection. Rectangular dimensions swap without cropping.
    pub fn rotate_canvas_clockwise(&mut self) {
        self.rotate_canvas_clockwise_control(None)
            .expect("Uncancelled valid canvas rotation");
    }
    pub(crate) fn rotate_canvas_clockwise_control(
        &mut self,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<(), String> {
        self.validate()?;
        crate::fill::check_cancel(cancel)?;
        let old_width = self.width;
        let old_height = self.height;
        let mut result = self.clone();
        for layer in &mut result.layers {
            if let Some(mask) = &mut layer.mask {
                mask.coverage =
                    mask.coverage
                        .remap(old_height, old_width, 255, cancel, |[x, y]| {
                            Some([y, old_height - 1 - x])
                        })?;
            }
            if layer.folder {
                continue;
            }
            let mut pixels = Pixels::sparse(old_height, old_width);
            {
                let mut writer = pixels.writer();
                layer.pixels.try_visit_rows(
                    old_width,
                    old_height,
                    [0, 0, old_width, old_height],
                    |x, y, row| {
                        crate::fill::check_cancel(cancel)?;
                        for (offset, pixel) in row.chunks_exact(4).enumerate() {
                            if pixel != [0; 4] {
                                writer
                                    .rgba_mut((x + offset) * old_height + old_height - 1 - y)
                                    .copy_from_slice(pixel);
                            }
                        }
                        Ok::<_, String>(())
                    },
                )?;
            }
            layer.pixels = pixels;
        }
        result.selection_mask = self
            .selection_mask
            .as_ref()
            .map(|m| m.rotated_clockwise_control(cancel))
            .transpose()?;
        result.selection = self.selection.map(|s| Selection {
            left: old_height - s.bottom,
            top: s.left,
            right: old_height - s.top,
            bottom: s.right,
        });
        result.width = old_height;
        result.height = old_width;
        crate::fill::check_cancel(cancel)?;
        result.validate()?;
        *self = result;
        Ok(())
    }

    /// Composite the active layer onto the layer below it, keeping the lower
    /// layer's own opacity, blend mode and clipping. The upper layer's blend
    /// mode, opacity and clipping are rendered into the merged pixels.
    pub fn merge_down(&mut self) -> Result<(), String> {
        let upper = self.active;
        if upper == 0 {
            return Err("There is no layer below to merge into".into());
        }
        let lower = upper - 1;
        if self.layers[upper].folder {
            return Err("Use Merge folder to flatten a folder".into());
        }
        if self.layers[lower].folder || self.layers[lower].depth != self.layers[upper].depth {
            return Err("The layer below is a folder or outside this folder".into());
        }
        if !self.layers[upper].visible || !self.layers[lower].visible {
            return Err("Show both layers before merging".into());
        }
        if self.layers[lower].adjustment.is_some() {
            return Err(
                "The layer below is an adjustment layer; it has no pixels to merge into".into(),
            );
        }
        if self.layers[lower].locked || self.layers[lower].alpha_locked {
            return Err("Unlock the layer below (pixels and transparency) to merge".into());
        }
        let mut pair = Document::new(1, 1);
        pair.version = self.version;
        pair.width = self.width;
        pair.height = self.height;
        let mut base = self.layers[lower].clone();
        base.opacity = 1.0;
        base.blend = crate::blend::BlendMode::Normal;
        base.clipped = false;
        base.depth = 0;
        let mut top = self.layers[upper].clone();
        top.locked = false;
        top.depth = 0;
        pair.layers = vec![base, top];
        pair.selection = None;
        let merged = pair.composite_transparent();
        self.layers[lower].pixels = merged.into();
        // Both enabled leaf masks and both layers' effects have already
        // contributed to this raster.
        self.layers[lower].mask = None;
        self.layers[lower].effects.clear();
        // The lower layer's fill opacity is in the merged pixels too.
        self.layers[lower].fill = 1.0;
        // Merged pixels are no longer the strokes of a vector layer.
        self.layers[lower].vector = None;
        self.layers[lower].text = None;
        self.layers.remove(upper);
        self.active = lower;
        Ok(())
    }
}

/// The layer with `region` lifted and composited back at an integer offset.
/// Content moved past the canvas edge is clipped.
pub fn translated(
    source: &Pixels,
    width: usize,
    height: usize,
    region: Selection,
    dx: i64,
    dy: i64,
) -> Pixels {
    translated_control(source, width, height, region, [dx, dy], None)
        .expect("uncontrolled translation cannot be canceled")
}
pub(crate) fn translated_control(
    source: &Pixels,
    width: usize,
    height: usize,
    region: Selection,
    [dx, dy]: [i64; 2],
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Pixels, String> {
    crate::fill::check_cancel(cancel)?;
    let rect = [region.left, region.top, region.right, region.bottom];
    let whole = rect == [0, 0, width, height];
    let mut out = if whole {
        Pixels::sparse(width, height)
    } else {
        source.clone()
    };
    if !whole {
        out.clear_region(width, rect);
    }
    {
        let mut writer = out.writer();
        for row in region.top..region.bottom {
            crate::fill::check_cancel(cancel)?;
            source.visit_nonzero(
                width,
                height,
                [region.left, row, region.right, row + 1],
                |x, y, s| {
                    let Some(tx) = (x as i64).checked_add(dx) else {
                        return;
                    };
                    let Some(ty) = (y as i64).checked_add(dy) else {
                        return;
                    };
                    if tx < 0 || ty < 0 || tx >= width as i64 || ty >= height as i64 {
                        return;
                    }
                    let d = writer.rgba_mut(ty as usize * width + tx as usize);
                    // Keep hidden RGB when the destination is transparent.
                    // A transparent source still contributes nothing over opaque
                    // retained artwork outside the lifted selection.
                    if s[3] == 0 && d[3] != 0 {
                        return;
                    }
                    if s[3] == 255 || d[3] == 0 {
                        d.copy_from_slice(&s);
                        return;
                    }
                    let sa = s[3] as f32 / 255.0;
                    let da = d[3] as f32 / 255.0;
                    let oa = sa + da * (1.0 - sa);
                    for c in 0..3 {
                        d[c] =
                            ((s[c] as f32 * sa + d[c] as f32 * da * (1.0 - sa)) / oa).round() as u8;
                    }
                    d[3] = (oa * 255.0).round() as u8;
                },
            );
        }
    }
    crate::fill::check_cancel(cancel)?;
    Ok(out)
}

impl Selection {
    /// Shift by an offset, clamped to the canvas; None if nothing remains.
    pub fn offset(self, dx: i64, dy: i64, width: usize, height: usize) -> Option<Selection> {
        let clamp = |v: i64, max: usize| v.clamp(0, max as i64) as usize;
        let moved = Selection {
            left: clamp((self.left as i64).saturating_add(dx), width),
            top: clamp((self.top as i64).saturating_add(dy), height),
            right: clamp((self.right as i64).saturating_add(dx), width),
            bottom: clamp((self.bottom as i64).saturating_add(dy), height),
        };
        moved.valid(width, height).then_some(moved)
    }
}

pub(crate) fn benchmark_masks(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        let region = Selection {
            left: 0,
            top: 0,
            right: size / 2,
            bottom: size,
        };
        let mask = crate::selection_mask::Mask::rectangle(size, size, region)?;
        for storage in ["sparse", "dense-flat", "dense-tiled"] {
            let mut source = Pixels::sparse(size, size);
            {
                let mut writer = source.writer();
                for y in 0..size {
                    for x in 0..size {
                        if storage != "sparse" || (x % 257 == 0 && y % 251 == 0) {
                            writer.rgba_mut(y * size + x).copy_from_slice(&[
                                (x % 251) as u8,
                                (y % 239) as u8,
                                ((x + y) % 233) as u8,
                                255,
                            ]);
                        }
                    }
                }
            }
            if storage == "dense-flat" {
                source = source.to_vec().into();
            }
            let frozen = source.clone();
            for movement in [false, true] {
                let map = if movement {
                    Remap::Offset([(size / 4) as i64, 0])
                } else {
                    Remap::Flip(Transform::FlipHorizontal, region)
                };
                let started = std::time::Instant::now();
                let (result, mapped) = remapped_masked(&source, size, size, &mask, map)?;
                let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                let expected = if movement {
                    translated(&source, size, size, region, (size / 4) as i64, 0)
                } else {
                    let mut doc = Document::new(size, size);
                    doc.layers[0].pixels = source.clone();
                    doc.selection = Some(region);
                    doc.transform_active(Transform::FlipHorizontal)?;
                    doc.layers[0].pixels.clone()
                };
                if result != expected || !source.shares(&frozen) {
                    return Err("Mask transform compatibility/source ownership differs".into());
                }
                let expected_bounds = if movement {
                    region.offset((size / 4) as i64, 0, size, size)
                } else {
                    Some(region)
                };
                if mapped.bounds() != expected_bounds {
                    return Err("Mask transform bounds differ".into());
                }
                for y in 0..size {
                    for x in 0..size {
                        let coverage = if expected_bounds.is_some_and(|b| b.contains(x, y)) {
                            255
                        } else {
                            0
                        };
                        if mapped.at(x, y) != coverage {
                            return Err("Mask transform coverage differs".into());
                        }
                    }
                }
                let queued = if movement {
                    None
                } else {
                    let mut doc = Document::new(size, size);
                    doc.layers[0].pixels = source.clone();
                    doc.selection = mask.bounds();
                    doc.selection_mask = Some(mask.clone());
                    let edit = crate::document_edits::Edit::TransformTarget {
                        target: doc.layers[0].id,
                        transform: Transform::FlipHorizontal,
                    };
                    let (mut changed, mut history, mut measurement) =
                        crate::render_queue::measure_edit(&doc, edit)?;
                    if changed.layers[0].pixels != result
                        || changed.selection_mask.as_ref() != Some(&mapped)
                    {
                        return Err("Queued masked flip RGBA/coverage differs".into());
                    }
                    if !history.undo(&mut changed)
                        || changed.layers[0].pixels != source
                        || changed.selection_mask != doc.selection_mask
                    {
                        return Err("Queued masked flip undo differs".into());
                    }
                    if !history.redo(&mut changed)
                        || changed.layers[0].pixels != result
                        || changed.selection_mask.as_ref() != Some(&mapped)
                    {
                        return Err("Queued masked flip redo differs".into());
                    }
                    measurement["rgba_mask_undo_redo_verified"] = serde_json::json!(true);
                    Some(measurement)
                };
                cases.push(serde_json::json!({"size":size,"storage":storage,"operation":if movement {"offset"} else {"flip-horizontal"},"elapsed_ms":elapsed_ms,"mask_capacity_bytes":mapped.allocation().1,"legacy_rgba_verified":true,"coverage_verified":true,"source_shares_frozen_payload":true,"queued_flip":queued}));
            }
        }
    }
    crate::storage::atomic_write(path, &serde_json::to_vec_pretty(&serde_json::json!({
        "version":2,
        "scope":"One repetition per case. Direct synchronous remap uses binary half-canvas masks and sparse/opaque patterned dense pixels, includes candidate pixels, mask construction/bounds and reclamation, excludes setup/verification and undo settlement. Queued flip additionally measures admission and owner advance with a 2 ms yield deadline, including worker start, source checks, publication, sliced history settlement and snapshot; worker joins are outside owner timing, inside wall time. Exact queued RGBA/mask and undo/redo verified outside timing. Excludes GUI, display, OS IO, input latency, transient memory and RSS; mask payload capacity only. Moves still use synchronous previews. Fractional overlap/hidden RGB/extreme clipping and gesture cancel have separate unit tests.",
        "cases":cases
    })).map_err(|e|e.to_string())?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn fractional_transfer_matches_binary_subpixel_overlap_oracle() {
        for departing in [0u8, 1, 127, 128, 254, 255] {
            for incoming in [0u8, 1, 127, 128, 254, 255] {
                for old_alpha in [1u8, 128, 255] {
                    for new_alpha in [0u8, 128, 255] {
                        let before = [17, 83, 201, old_alpha];
                        let source = [223, 31, 97, new_alpha];
                        let actual = super::transferred(before, source, departing, incoming);
                        let mut alpha = 0.0;
                        let mut colors = [0.0; 3];
                        for slot in 0..255 {
                            let da = if slot < usize::from(departing) {
                                0.0
                            } else {
                                f64::from(old_alpha) / 255.0
                            };
                            let sa = if slot < usize::from(incoming) {
                                f64::from(new_alpha) / 255.0
                            } else {
                                0.0
                            };
                            alpha += sa + da * (1.0 - sa);
                            for c in 0..3 {
                                colors[c] += f64::from(source[c]) * sa
                                    + f64::from(before[c]) * da * (1.0 - sa);
                            }
                        }
                        assert_eq!(actual[3], alpha.round() as u8);
                        if alpha > 0.0 {
                            for c in 0..3 {
                                assert!(actual[c].abs_diff((colors[c] / alpha).round() as u8) <= 1);
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn masked_flip_preserves_holes_and_overlapping_feather_keeps_opaque_alpha() {
        let mut doc = Document::new(3, 1);
        doc.layers[0].pixels = vec![255, 0, 0, 255, 9, 11, 13, 255, 0, 0, 255, 255].into();
        let mask = crate::selection_mask::Mask::from_coverage(3, 1, vec![255, 0, 128]).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        let before = doc.clone();
        let mut history = crate::document::History::new();
        history.checkpoint(&doc);
        assert!(
            doc.transform_active(super::Transform::FlipHorizontal)
                .unwrap()
        );
        history.settle(&doc);
        assert_eq!(doc.layers[0].pixels.rgba(0), [0, 0, 255, 128]);
        assert_eq!(doc.layers[0].pixels.rgba(1), [9, 11, 13, 255]);
        assert_eq!(doc.layers[0].pixels.rgba(2), [255, 0, 0, 255]);
        assert_eq!(doc.selection_mask.as_ref().unwrap().at(0, 0), 128);
        let after = doc.clone();
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        assert_eq!(doc.selection_mask, before.selection_mask);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after.layers[0].pixels);
        assert_eq!(doc.selection_mask, after.selection_mask);
        let mut doc = Document::new(2, 1);
        doc.layers[0].pixels = vec![255, 0, 0, 255, 0, 0, 255, 255].into();
        let mask = crate::selection_mask::Mask::from_coverage(2, 1, vec![128, 128]).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        doc.transform_active(super::Transform::FlipHorizontal)
            .unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(0), [127, 0, 128, 255]);
        assert_eq!(doc.layers[0].pixels.rgba(1), [128, 0, 127, 255]);
    }
    #[test]
    fn rectangular_masks_match_legacy_flips_across_tiles_including_hidden_rgb() {
        let rect = Selection {
            left: 255,
            top: 255,
            right: 258,
            bottom: 259,
        };
        for transform in [
            super::Transform::FlipHorizontal,
            super::Transform::FlipVertical,
            super::Transform::RotateHalfTurn,
        ] {
            let mut old = Document::new(517, 259);
            old.selection = Some(rect);
            for y in rect.top..rect.bottom {
                for x in rect.left..rect.right {
                    old.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut(y * 517 + x)
                        .copy_from_slice(&[
                            (x - 254) as u8,
                            (y - 254) as u8,
                            17,
                            if y == 256 { 0 } else { 128 },
                        ]);
                }
            }
            let mut masked = old.clone();
            masked.selection_mask =
                Some(crate::selection_mask::Mask::rectangle(517, 259, rect).unwrap());
            old.transform_active(transform).unwrap();
            masked.transform_active(transform).unwrap();
            assert_eq!(masked.layers[0].pixels, old.layers[0].pixels);
            assert!(masked.validate().is_ok());
            masked.layers[0].locked = true;
            assert!(masked.transform_active(transform).is_err());
        }
    }
    #[test]
    fn masked_translation_handles_zero_extreme_clipping_and_fractional_overlap() {
        let mut doc = Document::new(4, 1);
        doc.layers[0].pixels =
            vec![255, 0, 0, 255, 0, 0, 255, 255, 9, 11, 13, 0, 5, 7, 9, 255].into();
        let mask = crate::selection_mask::Mask::from_coverage(4, 1, vec![128, 0, 255, 0]).unwrap();
        let (same, shared) = super::remapped_masked(
            &doc.layers[0].pixels,
            4,
            1,
            &mask,
            super::Remap::Offset([0, 0]),
        )
        .unwrap();
        assert!(same.shares(&doc.layers[0].pixels));
        assert_eq!(shared.allocation(), mask.allocation());
        let (moved, shape) = super::remapped_masked(
            &doc.layers[0].pixels,
            4,
            1,
            &mask,
            super::Remap::Offset([1, 0]),
        )
        .unwrap();
        assert_eq!(moved.rgba(0), [255, 0, 0, 127]);
        assert_eq!(moved.rgba(1), [128, 0, 127, 255]);
        assert_eq!(moved.rgba(2), [0; 4]);
        assert_eq!(moved.rgba(3), [5, 7, 9, 255]);
        assert_eq!(shape.at(1, 0), 128);
        assert_eq!(shape.at(3, 0), 255);
        for offset in [[i64::MAX, i64::MAX], [i64::MIN, i64::MIN]] {
            let (clipped, empty) = super::remapped_masked(
                &doc.layers[0].pixels,
                4,
                1,
                &mask,
                super::Remap::Offset(offset),
            )
            .unwrap();
            assert!(empty.bounds().is_none());
            assert_eq!(clipped.rgba(0), [255, 0, 0, 127]);
            assert_eq!(clipped.rgba(1), [0, 0, 255, 255]);
            assert_eq!(clipped.rgba(2), [0; 4]);
        }
    }
    #[test]
    fn sparse_flips_and_rotation_preserve_odd_edges_hidden_rgb_and_history() {
        let mut doc = Document::new(517, 259);
        for (x, y, pixel) in [
            (0, 0, [1, 2, 3, 255]),
            (255, 255, [4, 5, 6, 128]),
            (516, 258, [7, 8, 9, 255]),
            (512, 256, [10, 11, 12, 0]),
        ] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(y * 517 + x)
                .copy_from_slice(&pixel);
        }
        doc.add_layer();
        doc.active = 0;
        let original = doc.clone();
        let capacity = original.layers[0].pixels.capacity();
        for transform in [
            Transform::FlipHorizontal,
            Transform::FlipVertical,
            Transform::RotateHalfTurn,
        ] {
            let mut history = crate::document::History::new();
            history.checkpoint(&doc);
            assert!(doc.transform_active(transform).unwrap());
            assert!(
                doc.layers[0].pixels.capacity()
                    <= 3 * crate::pixels::SIDE * crate::pixels::SIDE * 4
            );
            assert!(history.undo(&mut doc));
            for index in 0..517 * 259 {
                assert_eq!(
                    doc.layers[0].pixels.rgba(index),
                    original.layers[0].pixels.rgba(index)
                );
            }
            assert!(history.redo(&mut doc));
            assert!(doc.transform_active(transform).unwrap());
            for index in 0..517 * 259 {
                assert_eq!(
                    doc.layers[0].pixels.rgba(index),
                    original.layers[0].pixels.rgba(index)
                );
            }
        }
        for _ in 0..4 {
            doc.rotate_canvas_clockwise();
            assert_eq!(doc.layers[1].pixels.capacity(), 0);
        }
        assert_eq!((doc.width, doc.height), (517, 259));
        for index in 0..517 * 259 {
            assert_eq!(
                doc.layers[0].pixels.rgba(index),
                original.layers[0].pixels.rgba(index)
            );
        }
        assert_eq!(original.layers[0].pixels.capacity(), capacity);
    }
    #[test]
    fn sparse_selected_move_blends_over_unselected_pixels_without_flattening() {
        let (width, height) = (517, 259);
        let mut source = Pixels::sparse(width, height);
        source
            .writer()
            .rgba_mut(255 * width + 255)
            .copy_from_slice(&[255, 0, 0, 128]);
        source
            .writer()
            .rgba_mut(258 * width + 516)
            .copy_from_slice(&[0, 0, 255, 255]);
        source.writer().rgba_mut(0).copy_from_slice(&[9, 8, 7, 0]);
        let source_capacity = source.capacity();
        let region = Selection {
            left: 250,
            top: 250,
            right: 260,
            bottom: 259,
        };
        let moved = translated(&source, width, height, region, 261, 3);
        assert_eq!(moved.rgba(255 * width + 255), [0; 4]);
        assert_eq!(moved.rgba(258 * width + 516), [128, 0, 127, 255]);
        assert_eq!(moved.rgba(0), [9, 8, 7, 0]);
        assert_eq!(source.rgba(255 * width + 255), [255, 0, 0, 128]);
        assert_eq!(source.capacity(), source_capacity);
        assert_eq!(
            moved.capacity(),
            2 * crate::pixels::SIDE * crate::pixels::SIDE * 4
        );
        let clipped = translated(&source, width, height, region, i64::MAX, i64::MAX);
        assert_eq!(clipped.rgba(258 * width + 516), [0, 0, 255, 255]);
        assert_eq!(region.offset(i64::MAX, i64::MAX, width, height), None);
    }
    #[test]
    fn no_op_flip_preserves_storage_and_flat_inputs_produce_uncached_sparse_results() {
        let mut blank = Document::new(257, 259);
        let before = blank.layers[0].pixels.clone();
        assert!(!blank.transform_active(Transform::FlipHorizontal).unwrap());
        assert!(blank.layers[0].pixels.shares(&before));
        assert_eq!(blank.layers[0].pixels.capacity(), 0);
        let mut flat = Document::new(11, 7);
        let mut bytes = vec![0; 11 * 7 * 4];
        bytes[..4].copy_from_slice(&[9, 8, 7, 0]);
        flat.layers[0].pixels = bytes.into();
        assert!(flat.transform_active(Transform::FlipHorizontal).unwrap());
        assert_eq!(flat.layers[0].pixels.rgba(10), [9, 8, 7, 0]);
        assert_eq!(
            flat.layers[0].pixels.capacity(),
            crate::pixels::SIDE * crate::pixels::SIDE * 4
        );
    }
    use super::*;
    #[test]
    fn translation_moves_selected_pixels_over_the_rest_and_clips_at_edges() {
        let mut source = vec![0u8; 4 * 4];
        source[0..4].copy_from_slice(&[255, 0, 0, 255]);
        source[12..16].copy_from_slice(&[0, 0, 255, 255]);
        let source: Pixels = source.into();
        let all = Selection {
            left: 0,
            top: 0,
            right: 4,
            bottom: 1,
        };
        let moved = translated(&source, 4, 1, all, 1, 0);
        assert_eq!(&moved[4..8], &[255, 0, 0, 255]);
        assert_eq!(&moved[0..4], &[0; 4]);
        assert_eq!(&moved[12..16], &[0; 4]); // blue moved past the edge
        let first = Selection {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        let floated = translated(&source, 4, 1, first, 3, 0);
        assert_eq!(&floated[12..16], &[255, 0, 0, 255]); // lifted red covers blue
        assert_eq!(
            first.offset(3, 0, 4, 1),
            Some(Selection {
                left: 3,
                top: 0,
                right: 4,
                bottom: 1
            })
        );
        assert_eq!(first.offset(9, 0, 4, 1), None);
    }
    #[test]
    fn merge_down_renders_upper_blend_and_opacity_into_the_lower_layer() {
        let mut doc = Document::new(1, 1);
        doc.layers[0].pixels = vec![200, 200, 200, 255].into();
        doc.add_layer();
        doc.layers[1].pixels = vec![100, 100, 100, 255].into();
        doc.layers[1].blend = crate::blend::BlendMode::Multiply;
        let expected = doc.composite();
        doc.merge_down().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.active, 0);
        assert_eq!(doc.composite(), expected);
        assert!(doc.merge_down().is_err());
    }
    #[test]
    fn selected_flip_preserves_outside_pixels_and_is_reversible() {
        let mut doc = Document::new(4, 1);
        doc.layers[0].pixels = vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255].into();
        doc.selection = Some(Selection {
            left: 1,
            top: 0,
            right: 3,
            bottom: 1,
        });
        let before = doc.layers[0].pixels.clone();
        doc.transform_active(Transform::FlipHorizontal).unwrap();
        assert_eq!(
            doc.layers[0].pixels,
            vec![1, 0, 0, 255, 3, 0, 0, 255, 2, 0, 0, 255, 4, 0, 0, 255]
        );
        doc.transform_active(Transform::FlipHorizontal).unwrap();
        assert_eq!(doc.layers[0].pixels, before);
        doc.layers[0].locked = true;
        assert!(doc.transform_active(Transform::FlipVertical).is_err());
        assert_eq!(doc.layers[0].pixels, before);
    }
    #[test]
    fn rectangular_canvas_rotation_preserves_every_layer_and_selection() {
        let mut doc = Document::new(3, 2);
        doc.layers[0].pixels = (0..24).collect();
        doc.add_layer();
        doc.layers[1].pixels = (100..124).collect();
        doc.layers[1].locked = true;
        doc.selection = Some(Selection {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        });
        let original = serde_json::to_vec(&doc).unwrap();
        doc.rotate_canvas_clockwise();
        doc.validate().unwrap();
        assert_eq!((doc.width, doc.height), (2, 3));
        assert_eq!(
            doc.selection,
            Some(Selection {
                left: 1,
                top: 0,
                right: 2,
                bottom: 2
            })
        );
        for _ in 0..3 {
            doc.rotate_canvas_clockwise();
        }
        assert_eq!(serde_json::to_vec(&doc).unwrap(), original);
    }
}
