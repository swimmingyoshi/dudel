//! Restartable history patch construction. The checkpoint stays intact until
//! every region is prepared; the caller serializes all document/history edits.
use super::{Document, Entry, History, Patch, RegionPatch, compact_layer};
use crate::pixels::SIDE;

pub struct Settlement {
    before: Document,
    after: Document,
    undo_len: usize,
    pending: bool,
    layer: usize,
    block: usize,
    blocks: usize,
    row: usize,
    bounds: [usize; 4],
    copying: Option<([usize; 4], usize, Vec<u8>)>,
    regions: Vec<RegionPatch>,
    complete: bool,
    peak_payload: usize,
    region_payload: usize,
    full_snapshot_limit: Option<usize>,
    fill_plans: Option<Vec<[usize; 4]>>,
    planned_payload: usize,
    next_plan: usize,
}
#[derive(Default)]
pub struct SettlementStep {
    pub work_units: usize,
    pub complete: bool,
}
pub(crate) fn same_snapshot(a: &Document, b: &Document) -> bool {
    a.version == b.version
        && a.width == b.width
        && a.height == b.height
        && a.dpi == b.dpi
        && a.canvas_frame == b.canvas_frame
        && a.active == b.active
        && a.selection == b.selection
        && a.selection_mask == b.selection_mask
        && a.layers.len() == b.layers.len()
        && a.layers.iter().zip(&b.layers).all(|(a, b)| {
            a.id == b.id
                && a.name == b.name
                && a.visible == b.visible
                && a.opacity == b.opacity
                && a.csp_byte_opacity == b.csp_byte_opacity
                && a.fill == b.fill
                && a.blend == b.blend
                && a.clipped == b.clipped
                && a.locked == b.locked
                && a.alpha_locked == b.alpha_locked
                && a.folder == b.folder
                && a.depth == b.depth
                && a.collapsed == b.collapsed
                && a.effects == b.effects
                && a.adjustment == b.adjustment
                && crate::layer_mask::same_snapshot(&a.mask, &b.mask)
                && a.pixels.shares(&b.pixels)
        })
}
impl History {
    pub(crate) fn retain_gesture_snapshot(&mut self, source: &Document, doc: &Document) -> bool {
        if !self.has_pending_stroke()
            || !matches!(self.undo.last(),Some(Entry::Full(before)) if same_snapshot(before,source))
        {
            return false;
        }
        let Entry::Full(before) = self.undo.pop().unwrap() else {
            unreachable!()
        };
        self.undo.push(Entry::RetainedFull(before));
        self.pending_redo = None;
        self.trim(doc, true);
        true
    }
    pub fn begin_settle(&self, doc: &Document) -> Option<Settlement> {
        let Entry::Full(before) = self.undo.last()? else {
            return None;
        };
        let edited = compact_layer(before, doc)?;
        let layer = edited.unwrap_or(0);
        if edited.is_some()
            && (!before.layers[layer]
                .pixels
                .has_geometry(doc.width, doc.height)
                || !doc.layers[layer].pixels.has_geometry(doc.width, doc.height))
        {
            return None;
        }
        Some(Settlement {
            before: before.clone(),
            after: doc.clone(),
            undo_len: self.undo.len(),
            pending: self.has_pending_stroke(),
            layer,
            block: 0,
            blocks: if edited.is_some() {
                doc.width.div_ceil(SIDE) * doc.height.div_ceil(SIDE)
            } else {
                0
            },
            row: 0,
            bounds: [doc.width, doc.height, 0, 0],
            copying: None,
            regions: Vec::new(),
            complete: false,
            peak_payload: 0,
            region_payload: 0,
            full_snapshot_limit: None,
            fill_plans: None,
            planned_payload: 0,
            next_plan: 0,
        })
    }
}
impl Settlement {
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        // Both embedded Documents are accounted below, including their headers.
        registry.add_owned(
            std::mem::size_of::<Self>() - 2 * std::mem::size_of::<Document>()
                + self.regions.capacity() * std::mem::size_of::<RegionPatch>()
                + self
                    .copying
                    .as_ref()
                    .map_or(0, |(_, _, bytes)| bytes.capacity())
                + self.fill_plans.as_ref().map_or(0, |plans| {
                    plans.capacity() * std::mem::size_of::<[usize; 4]>()
                }),
        );
        registry.add_document(&self.before);
        registry.add_document(&self.after);
        for region in &self.regions {
            registry.add_buffer(
                region.pixels.allocation_identity(),
                region.pixels.capacity(),
            );
        }
    }
    /// For broad fills, a sparse original can cost less than its raster patch.
    /// This upper bound ignores sharing with older history (which only helps Full).
    pub(crate) fn prefer_smaller_full_snapshot(&mut self) {
        let mut seen = std::collections::HashSet::new();
        for layer in &self.after.layers {
            layer.pixels.visit_allocations(|id, _| {
                seen.insert(id);
            });
        }
        let mut bytes = std::mem::size_of::<Document>()
            + self.before.layers.capacity() * std::mem::size_of::<super::Layer>();
        if !std::sync::Arc::ptr_eq(&self.before.canvas_frame, &self.after.canvas_frame) {
            bytes += std::mem::size_of::<crate::canvas_size::Frame>();
        }
        for layer in &self.before.layers {
            bytes += layer.name.capacity();
            layer.pixels.visit_allocations(|id, capacity| {
                if seen.insert(id) {
                    bytes += capacity;
                }
            });
        }
        self.full_snapshot_limit = Some(bytes);
        self.fill_plans = Some(Vec::new());
    }
    /// Keep the original full undo entry when only the observed document changed.
    /// Never rewrite a checkpoint another owner has replaced or consumed.
    pub(crate) fn retain_full(&self, history: &mut History, doc: &Document) -> bool {
        if history.undo.len() != self.undo_len
            || history.has_pending_stroke() != self.pending
            || !matches!(history.undo.last(), Some(Entry::Full(before)) if same_snapshot(before, &self.before))
        {
            return false;
        }
        let Entry::Full(before) = history.undo.pop().unwrap() else {
            unreachable!("checkpoint identity checked above")
        };
        history.undo.push(Entry::RetainedFull(before));
        history.pending_redo = None;
        history.trim(doc, true);
        true
    }
    fn record_payload(&mut self) {
        let bytes = self.regions.capacity() * std::mem::size_of::<RegionPatch>()
            + self.region_payload
            + self.fill_plans.as_ref().map_or(0, |plans| {
                plans.capacity() * std::mem::size_of::<[usize; 4]>()
            })
            + self
                .copying
                .as_ref()
                .map_or(0, |(_, _, bytes)| bytes.capacity());
        self.peak_payload = self.peak_payload.max(bytes);
    }
    pub fn peak_payload_capacity_bytes(&self) -> usize {
        self.peak_payload
    }
    pub fn step(
        &mut self,
        history: &mut History,
        doc: &Document,
        max_work: usize,
    ) -> Result<SettlementStep, String> {
        if self.complete {
            return Ok(SettlementStep {
                complete: true,
                ..Default::default()
            });
        }
        if history.undo.len() != self.undo_len
            || history.has_pending_stroke() != self.pending
            || !matches!(history.undo.last(), Some(Entry::Full(before)) if same_snapshot(before, &self.before))
            || !same_snapshot(doc, &self.after)
        {
            return Err("Document or checkpoint changed during history settlement".into());
        }
        let mut result = SettlementStep::default();
        while result.work_units < max_work && !self.complete {
            result.work_units += 1;
            if self
                .full_snapshot_limit
                .is_some_and(|limit| self.peak_payload >= limit)
            {
                if !self.retain_full(history, doc) {
                    return Err("Checkpoint changed while retaining full fill snapshot".into());
                }
                self.complete = true;
                break;
            }
            if let Some((rect, y, bytes)) = &mut self.copying {
                bytes.extend_from_slice(self.before.layers[self.layer].pixels.tile_row(
                    doc.width,
                    *y,
                    rect[0],
                    (rect[2] - rect[0]) * 4,
                ));
                *y += 1;
                if *y == rect[3] {
                    let (rect, _, bytes) = self.copying.take().unwrap();
                    let pixels = crate::history_pixels::Payload::new(bytes);
                    self.region_payload += pixels.capacity();
                    self.regions.push(RegionPatch { rect, pixels });
                    if self.fill_plans.is_none() {
                        self.block += 1;
                    }
                    self.row = 0;
                    self.bounds = [doc.width, doc.height, 0, 0];
                }
                self.record_payload();
                continue;
            }
            if self.block == self.blocks {
                if let Some(plans) = &self.fill_plans
                    && self.next_plan < plans.len()
                {
                    // Fill scans all region costs before allocating any pixel patch.
                    let rect = plans[self.next_plan];
                    self.next_plan += 1;
                    self.copying = Some((
                        rect,
                        rect[1],
                        Vec::with_capacity((rect[2] - rect[0]) * (rect[3] - rect[1]) * 4),
                    ));
                    self.record_payload();
                    continue;
                }
                let patch = Patch {
                    layer: self.layer,
                    regions: std::mem::take(&mut self.regions),
                    active: self.before.active,
                    selection: self.before.selection,
                    selection_mask: self.before.selection_mask.clone(),
                };
                *history.undo.last_mut().unwrap() = Entry::Patch(patch);
                history.pending_redo = None;
                history.trim(doc, true);
                self.complete = true;
                continue;
            }
            let source = &self.before.layers[self.layer].pixels;
            let target = &self.after.layers[self.layer].pixels;
            if self.row == 0 && source.shares_tile(target, self.block) {
                self.block += 1;
                continue;
            }
            let columns = doc.width.div_ceil(SIDE);
            let left = self.block % columns * SIDE;
            let top = self.block / columns * SIDE;
            let y = top + self.row;
            let row_bytes = SIDE.min(doc.width - left) * 4;
            let old = source.tile_row(doc.width, y, left, row_bytes);
            let new = target.tile_row(doc.width, y, left, row_bytes);
            if old != new {
                let differs = |x: usize| old[x * 4..x * 4 + 4] != new[x * 4..x * 4 + 4];
                let first = (0..row_bytes / 4).find(|&x| differs(x)).unwrap();
                let last = (0..row_bytes / 4).rfind(|&x| differs(x)).unwrap();
                self.bounds[0] = self.bounds[0].min(left + first);
                self.bounds[1] = self.bounds[1].min(y);
                self.bounds[2] = self.bounds[2].max(left + last + 1);
                self.bounds[3] = self.bounds[3].max(y + 1);
            }
            self.row += 1;
            if self.row == SIDE.min(doc.height - top) {
                if self.bounds[2] > self.bounds[0] {
                    let payload =
                        (self.bounds[2] - self.bounds[0]) * (self.bounds[3] - self.bounds[1]) * 4;
                    if let Some(plans) = &mut self.fill_plans {
                        let projected = self
                            .planned_payload
                            .saturating_add(std::mem::size_of::<Patch>())
                            .saturating_add(payload)
                            .saturating_add((plans.len() + 1) * std::mem::size_of::<RegionPatch>())
                            .saturating_add(plans.capacity() * std::mem::size_of::<[usize; 4]>());
                        if self
                            .full_snapshot_limit
                            .is_some_and(|limit| projected >= limit)
                        {
                            if !self.retain_full(history, doc) {
                                return Err(
                                    "Checkpoint changed while retaining full fill snapshot".into(),
                                );
                            }
                            self.complete = true;
                            continue;
                        }
                        self.planned_payload += payload;
                        plans.push(self.bounds);
                        self.block += 1;
                        self.row = 0;
                        self.bounds = [doc.width, doc.height, 0, 0];
                        self.record_payload();
                        continue;
                    }
                    let prepared = std::mem::size_of::<Patch>()
                        + self.region_payload
                        + self.regions.capacity() * std::mem::size_of::<RegionPatch>();
                    if self
                        .full_snapshot_limit
                        .is_some_and(|limit| prepared.saturating_add(payload) >= limit)
                    {
                        if !self.retain_full(history, doc) {
                            return Err(
                                "Checkpoint changed while retaining full fill snapshot".into()
                            );
                        }
                        self.complete = true;
                        continue;
                    }
                    let bytes = Vec::with_capacity(payload);
                    self.copying = Some((self.bounds, self.bounds[1], bytes));
                    self.record_payload();
                } else {
                    self.block += 1;
                    self.row = 0;
                    self.bounds = [doc.width, doc.height, 0, 0];
                }
            }
        }
        result.complete = self.complete;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settlement_registry_tracks_partial_patches_and_deduplicates_checkpoint_pixels() {
        for flat in [false, true] {
            let (width, height) = (512, 256);
            let original = [9, 11, 13, 0].repeat(width * height);
            let mut doc = Document::new(width, height);
            doc.layers[0].pixels = if flat {
                original.clone().into()
            } else {
                crate::pixels::Pixels::read_rgba(&mut &original[..], width, height).unwrap()
            };
            let mut history = History::new();
            history.begin_stroke_checkpoint(&doc);
            doc.layers[0].pixels.writer().write_region(
                width,
                [0, 0, 256, 256],
                &[17, 83, 201, 128].repeat(256 * 256),
            );
            let edited = doc.layers[0].pixels.clone();
            history.refresh_memory(&doc);
            let mut cursor = history.begin_settle(&doc).unwrap();
            let before = crate::snapshot_memory::Snapshot::capture(&cursor.before);
            let after = crate::snapshot_memory::Snapshot::capture(&cursor.after);
            let snapshot_metadata =
                crate::snapshot_memory::beyond_live(&doc, [&before, &after]).metadata_bytes;
            let mut registry = crate::snapshot_memory::Registry::new(&doc);
            history.register_allocations(&mut registry);
            assert_eq!(registry.additional_bytes(), history.retained_bytes());
            let history_bytes = registry.additional_bytes();
            cursor.register_allocations(&mut registry);
            let metadata = std::mem::size_of::<Settlement>() - 2 * std::mem::size_of::<Document>()
                + snapshot_metadata;
            assert_eq!(registry.additional_bytes() - history_bytes, metadata);
            let mut observed_partial = false;
            let mut observed_prepared = false;
            let mut calls = 0;
            loop {
                calls += 1;
                assert!(calls < 2000);
                let result = cursor.step(&mut history, &doc, 1).unwrap();
                if !result.complete {
                    let mut registry = crate::snapshot_memory::Registry::new(&doc);
                    history.register_allocations(&mut registry);
                    cursor.register_allocations(&mut registry);
                    let capacity = cursor.regions.capacity() * std::mem::size_of::<RegionPatch>()
                        + cursor
                            .regions
                            .iter()
                            .map(|region| region.pixels.capacity())
                            .sum::<usize>()
                        + cursor
                            .copying
                            .as_ref()
                            .map_or(0, |(_, _, bytes)| bytes.capacity());
                    assert_eq!(
                        registry.additional_bytes(),
                        history_bytes + metadata + capacity
                    );
                    observed_partial |= cursor.copying.is_some();
                    observed_prepared |= !cursor.regions.is_empty();
                    assert!(matches!(history.undo.last(), Some(Entry::Full(_))));
                } else {
                    break;
                }
            }
            assert!(observed_partial && observed_prepared);
            drop(cursor);
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, original);
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels, edited);
        }
    }
    #[test]
    fn cost_planned_snapshot_survives_next_checkpoint_cancellation_and_history_inversion() {
        let mut doc = Document::new(512, 512);
        doc.layers[0].pixels = [9, 11, 13, 0].repeat(512 * 512).into();
        let before = doc.clone();
        let mut history = History::new();
        history.begin_gesture_checkpoint(&doc);
        doc.layers[0].pixels = [17, 83, 201, 255].repeat(512 * 512).into();
        let after = doc.clone();
        let mut cursor = history.begin_settle(&doc).unwrap();
        cursor.prefer_smaller_full_snapshot();
        while !cursor.step(&mut history, &doc, 31).unwrap().complete {}
        drop(cursor);
        let retained = history.retained_bytes();
        for _ in 0..3 {
            history.settle(&doc);
            assert!(matches!(history.undo.last(), Some(Entry::RetainedFull(_))));
            assert!(history.begin_settle(&doc).is_none());
            assert_eq!(history.retained_bytes(), retained);
        }
        history.checkpoint(&doc);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(512 * 512 - 1)
            .copy_from_slice(&[7, 5, 3, 0]);
        let next = doc.clone();
        history.settle(&doc);
        assert!(matches!(history.undo.last(), Some(Entry::Patch(_))));
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after.layers[0].pixels);
        assert!(matches!(history.undo.last(), Some(Entry::RetainedFull(_))));
        history.begin_gesture_checkpoint(&doc);
        assert!(history.discard_pending_checkpoint(&doc));
        assert!(history.can_redo());
        assert!(matches!(history.undo.last(), Some(Entry::RetainedFull(_))));
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        assert!(matches!(history.redo.last(), Some(Entry::RetainedFull(_))));
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after.layers[0].pixels);
        assert!(matches!(history.undo.last(), Some(Entry::RetainedFull(_))));
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, next.layers[0].pixels);
    }
    #[test]
    fn broad_dense_fill_selects_full_undo_before_allocating_pixel_patches() {
        let mut doc = Document::new(512, 512);
        doc.layers[0].pixels = [9, 11, 13, 255].repeat(512 * 512).into();
        let before = doc.clone();
        let mut history = History::new();
        history.begin_gesture_checkpoint(&doc);
        doc.layers[0].pixels = [17, 83, 201, 255].repeat(512 * 512).into();
        let after = doc.clone();
        let mut cursor = history.begin_settle(&doc).unwrap();
        cursor.prefer_smaller_full_snapshot();
        let mut calls = 0;
        while !cursor.step(&mut history, &doc, 1).unwrap().complete {
            calls += 1;
            assert!(calls < 3000);
        }
        assert!(cursor.peak_payload_capacity_bytes() < 4096);
        assert!(matches!(history.undo.last(), Some(Entry::RetainedFull(_))));
        assert!(!history.has_pending_stroke());
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after.layers[0].pixels);
    }

    #[test]
    fn planned_fill_patches_cross_tile_edges_and_match_every_pixel() {
        for flat in [false, true] {
            for budget in [1, 31, 4096] {
                let (width, height) = (517, 259);
                let bytes = [9, 11, 13, 255].repeat(width * height);
                let mut doc = Document::new(width, height);
                doc.layers[0].pixels = if flat {
                    bytes.clone().into()
                } else {
                    crate::pixels::Pixels::read_rgba(&mut &bytes[..], width, height).unwrap()
                };
                let mut history = History::new();
                history.begin_gesture_checkpoint(&doc);
                doc.layers[0].pixels.writer().write_region(
                    width,
                    [253, 252, 263, 258],
                    &[17, 83, 201, 255].repeat(60),
                );
                let expected = doc.layers[0].pixels.clone();
                let mut cursor = history.begin_settle(&doc).unwrap();
                cursor.prefer_smaller_full_snapshot();
                while !cursor.step(&mut history, &doc, budget).unwrap().complete {}
                let Some(Entry::Patch(patch)) = history.undo.last() else {
                    panic!("Small fill must use patches")
                };
                assert_eq!(patch.regions.len(), 4);
                assert!(history.retained_bytes() < 4096);
                assert!(history.undo(&mut doc));
                assert_eq!(doc.layers[0].pixels, bytes);
                assert!(history.redo(&mut doc));
                assert_eq!(doc.layers[0].pixels, expected);
            }
        }
    }

    #[test]
    fn settlement_is_atomic_and_matches_exact_patches_for_all_storage_pairs() {
        for budget in [1, 31, 256] {
            for before_flat in [false, true] {
                for after_flat in [false, true] {
                    let (width, height) = (517, 259);
                    let bytes = vec![255; width * height * 4];
                    let make = |bytes: &[u8], flat| {
                        if flat {
                            crate::pixels::Pixels::from(bytes.to_vec())
                        } else {
                            crate::pixels::Pixels::read_rgba(&mut &bytes[..], width, height)
                                .unwrap()
                        }
                    };
                    let mut doc = Document::new(width, height);
                    doc.layers[0].pixels = make(&bytes, before_flat);
                    let before = doc.clone();
                    let mut history = History::new();
                    history.begin_stroke_checkpoint(&doc);
                    let mut edited = bytes.clone();
                    for (x, y) in [(0, 0), (128, 0), (256, 256), (516, 258)] {
                        edited[(y * width + x) * 4..(y * width + x) * 4 + 4]
                            .copy_from_slice(&[7, 8, 9, 0]);
                    }
                    doc.layers[0].pixels = make(&edited, after_flat);
                    let frozen = doc.layers[0].pixels.clone();
                    let source_capacity = before.layers[0].pixels.capacity();
                    let target_capacity = frozen.capacity();
                    let expected = super::super::compact(&before, &doc).unwrap();
                    let mut cursor = history.begin_settle(&doc).unwrap();
                    let paused = cursor.step(&mut history, &doc, 0).unwrap();
                    assert_eq!(paused.work_units, 0);
                    assert!(!paused.complete);
                    loop {
                        let step = cursor.step(&mut history, &doc, budget).unwrap();
                        assert!(step.work_units <= budget);
                        assert!(doc.layers[0].pixels.shares(&frozen));
                        if step.complete {
                            break;
                        }
                        assert!(matches!(history.undo.last(), Some(Entry::Full(_))));
                        assert!(history.has_pending_stroke());
                    }
                    let Some(Entry::Patch(patch)) = history.undo.last_mut() else {
                        panic!("missing complete patch")
                    };
                    assert_eq!(patch.regions.len(), expected.regions.len());
                    for (actual, expected) in patch.regions.iter_mut().zip(expected.regions) {
                        assert_eq!(actual.rect, expected.rect);
                        assert_eq!(
                            *actual.pixels.raw_mut(),
                            before.layers[0].pixels.region(width, expected.rect)
                        );
                    }
                    assert!(!history.has_pending_stroke());
                    assert_eq!(before.layers[0].pixels.capacity(), source_capacity);
                    assert_eq!(doc.layers[0].pixels.capacity(), target_capacity);
                    assert!(cursor.step(&mut history, &doc, budget).unwrap().complete);
                    assert!(history.undo(&mut doc));
                    assert_eq!(doc.layers[0].pixels, bytes);
                    assert!(history.redo(&mut doc));
                    assert_eq!(doc.layers[0].pixels, edited);
                }
            }
        }
    }
    #[test]
    fn changed_document_or_checkpoint_rejects_partial_publication() {
        let mut doc = Document::new(37, 39);
        let mut history = History::new();
        history.begin_stroke_checkpoint(&doc);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let original = doc.clone();
        let mut cursor = history.begin_settle(&doc).unwrap();
        cursor.step(&mut history, &doc, 5).unwrap();
        doc.layers[0].opacity = 0.5;
        assert!(cursor.step(&mut history, &doc, 64).is_err());
        assert!(matches!(history.undo.last(), Some(Entry::Full(_))));
        doc.layers[0].opacity = original.layers[0].opacity;
        while !cursor.step(&mut history, &doc, 7).unwrap().complete {}
        assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
        history.checkpoint(&doc);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[1, 2, 3, 255]);
        let mut stale = history.begin_settle(&doc).unwrap();
        stale.step(&mut history, &doc, 1).unwrap();
        assert!(history.undo(&mut doc));
        let restored = doc.layers[0].pixels.clone();
        assert!(stale.step(&mut history, &doc, 64).is_err());
        assert_eq!(doc.layers[0].pixels, restored);
        assert!(history.can_redo());
    }
    #[test]
    fn metadata_and_multiple_layer_edits_keep_the_full_snapshot_path() {
        let mut doc = Document::new(37, 39);
        doc.add_layer();
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.layers[0].opacity = 0.5;
        assert!(history.begin_settle(&doc).is_none());
        history.settle(&doc);
        assert!(matches!(history.undo.last(), Some(Entry::Full(_))));
        assert!(history.undo(&mut doc));
        history.checkpoint(&doc);
        for layer in &mut doc.layers {
            layer
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[1, 2, 3, 255]);
        }
        assert!(history.begin_settle(&doc).is_none());
    }
}
