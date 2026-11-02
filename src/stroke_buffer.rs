#[path = "dab_paint.rs"]
mod dab_paint;
#[path = "tail_replay.rs"]
mod tail_replay;
pub use dab_paint::DabPaint;
use std::collections::{HashMap, HashSet, VecDeque};
pub use tail_replay::{TailReplay, TailStep};
/// A touched pixel's original value and the stroke's coverage there
/// (negative until the stroke first reaches it).
pub(crate) type PixelState = ([u8; 4], f32);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    #[serde(
        default,
        skip_serializing_if = "crate::paint_target::Target::is_raster"
    )]
    pub target: crate::paint_target::Target,
    pub brush: crate::brush::Brush,
    pub color: [u8; 3],
    pub erase: bool,
    pub layer: usize,
    pub symmetry: crate::symmetry::Symmetry,
}

#[derive(Default)]
pub struct Painted {
    pub region: crate::render_region::Damage,
    pub dabs: usize,
}
impl Painted {
    fn merge(&mut self, other: Self) {
        self.region.merge(other.region);
        self.dabs += other.dabs;
    }
}

/// Maximum 300px out taper / minimum .5px dab spacing, plus initial/end dabs.
pub const MAX_TAIL_DABS: usize = 602;

#[derive(Default)]
pub struct StrokeBuffer {
    pub(crate) target: crate::paint_target::Target,
    target_set: bool,
    layer_id: Option<crate::document::LayerId>,
    mask_original: Option<crate::layer_mask::Mask>,
    indices: HashMap<(usize, usize, usize), usize>,
    blocks: Vec<Box<[PixelState]>>,
    cached: Option<((usize, usize, usize), usize)>,
    prefix: Vec<Option<Box<[f32]>>>,
    prefix_pass: bool,
    rebuilding: bool,
    tail: VecDeque<crate::stroke::Sample>,
    tail_peak: usize,
}
/// Own original pixels until rollback completes. The controller must prevent
/// document replacement/edits and retain the history checkpoint during restore.
pub struct Restore {
    buffer: StrokeBuffer,
    keys: std::collections::hash_map::IntoIter<(usize, usize, usize), usize>,
    key_capacity_bytes: usize,
    current: Option<((usize, usize, usize), usize, usize)>,
    reclaim: Option<(usize, [usize; 4])>,
    dimensions: [usize; 2],
    layers: usize,
    active: usize,
    complete: bool,
}
#[derive(Default)]
pub struct RestoreStep {
    pub work_units: usize,
    pub changed_region: crate::render_region::Damage,
    pub complete: bool,
}
impl Restore {
    pub fn payload_capacity_bytes(&self) -> usize {
        self.buffer.payload_capacity_bytes() + self.key_capacity_bytes
    }
    pub fn step(
        &mut self,
        doc: &mut crate::document::Document,
        max_work: usize,
    ) -> Result<RestoreStep, String> {
        if self.dimensions != [doc.width, doc.height]
            || self.layers != doc.layers.len()
            || self.active != doc.active
        {
            return Err("Document changed during stroke rollback".into());
        }
        let mut result = RestoreStep::default();
        while !self.complete && result.work_units < max_work {
            if let Some(layer) = self
                .current
                .map(|(key, _, _)| key.0)
                .or(self.reclaim.map(|(layer, _)| layer))
                && (layer >= doc.layers.len()
                    || !self
                        .buffer
                        .target_valid(&doc.layers[layer], [doc.width, doc.height]))
            {
                return Err("Stroke rollback target changed".into());
            }
            result.work_units += 1;
            if let Some((layer, rect)) = self.reclaim.take() {
                self.buffer
                    .target
                    .reclaim(&mut doc.layers[layer], doc.width, rect);
                continue;
            }
            if let Some(((layer, bx, by), block, row)) = self.current {
                let y = by * 32 + row;
                if y < doc.height {
                    let target = self.buffer.target;
                    let layer_data = &mut doc.layers[layer];
                    let mut edits = [(0usize, [0u8; 4]); 32];
                    let mut count = 0;
                    for x in 0..32 {
                        let px = bx * 32 + x;
                        let (original, coverage) = self.buffer.blocks[block][row * 32 + x];
                        if coverage >= 0.0
                            && px < doc.width
                            && target.read(layer_data, doc.width, px, y) != original
                        {
                            edits[count] = (y * doc.width + px, original);
                            count += 1;
                            result.changed_region.include([px, y, px + 1, y + 1]);
                        }
                    }
                    if count > 0 {
                        let mut writer = target.writer(layer_data, doc.width);
                        for (index, original) in &edits[..count] {
                            writer.set(index % doc.width, index / doc.width, original);
                        }
                    }
                }
                if row + 1 < 32 {
                    self.current = Some(((layer, bx, by), block, row + 1));
                } else {
                    self.current = None;
                    // Release original/coverage payload incrementally rather
                    // than dropping every block in one final UI operation.
                    self.buffer.blocks[block] = Box::default();
                    self.buffer.prefix[block] = None;
                    self.reclaim = Some((
                        layer,
                        [
                            bx * 32,
                            by * 32,
                            ((bx + 1) * 32).min(doc.width),
                            ((by + 1) * 32).min(doc.height),
                        ],
                    ));
                }
                continue;
            }
            if let Some((key, block)) = self.keys.next() {
                let (layer, bx, by) = key;
                self.current = Some((key, block, 0));
                if layer >= doc.layers.len()
                    || !self
                        .buffer
                        .target_valid(&doc.layers[layer], [doc.width, doc.height])
                    || bx * 32 >= doc.width
                    || by * 32 >= doc.height
                {
                    return Err("Stroke rollback target changed".into());
                }
            } else if !self.buffer.blocks.is_empty() {
                self.buffer.blocks.pop();
                self.buffer.prefix.pop();
            } else {
                self.complete = true;
            }
        }
        result.complete = self.complete;
        if self.complete {
            self.buffer.restore_mask_identity(doc);
        }
        Ok(result)
    }
}
#[cfg(test)]
mod restore_tests {
    use super::*;
    use crate::{document::Document, stroke::Sample};
    #[test]
    fn batched_prefix_rows_match_full_selection_reference_for_softness_grain_and_transforms() {
        use crate::brush::{Brush, SoftnessProfile, Tip};
        let mut brushes = Vec::new();
        for profile in [SoftnessProfile::LegacyLinear, SoftnessProfile::Eased] {
            for hardness in [0.0, 0.65, 1.0] {
                for (aspect, angle) in [(1.0, 0.0), (0.45, 37.0)] {
                    brushes.push(Brush {
                        size: 24.0,
                        hardness,
                        aspect,
                        angle,
                        softness_profile: profile,
                        pressure_size: false,
                        flow: 0.3,
                        opacity: 0.5,
                        taper_out: 30.0,
                        ..Default::default()
                    });
                }
            }
        }
        let basic = brushes[0].clone();
        brushes.extend([
            Brush {
                size: 300.0,
                ..basic.clone()
            },
            Brush {
                size: 1.0,
                ..basic.clone()
            },
            Brush {
                tip: Tip::Square,
                ..basic.clone()
            },
            Brush {
                grain_amount: 0.7,
                ..basic.clone()
            },
            Brush {
                grain_amount: 0.7,
                grain: Some(crate::brush::TipImage {
                    width: 2,
                    height: 1,
                    alpha: (vec![0, 255]).into(),
                }),
                ..basic.clone()
            },
            Brush {
                adjust_flow_by_spacing: true,
                ..basic.clone()
            },
        ]);
        for (case, brush) in brushes.into_iter().enumerate() {
            for segments in [1, 4] {
                let large = brush.size == 300.0;
                let (width, height) = if large { (257, 259) } else { (97, 101) };
                let mut actual = Document::new(width, height);
                let base = [13, 27, 81, 140].repeat(width * height);
                actual.layers[0]
                    .pixels
                    .writer()
                    .write_region(width, [0, 0, width, height], &base);
                actual.layers[0].alpha_locked = case % 3 == 0;
                let mut reference = actual.clone();
                // Full-canvas selection is pixel-equivalent and forces the
                // preserved general kernel, without a test switch in hot code.
                reference.selection = Some(crate::selection::Selection {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                });
                let settings = Settings {
                    target: crate::paint_target::Target::Raster,
                    brush: brush.clone(),
                    color: [38, 44, 59],
                    erase: case % 5 == 0 && !actual.layers[0].alpha_locked,
                    layer: 0,
                    symmetry: crate::symmetry::Symmetry {
                        mode: if segments == 1 {
                            crate::symmetry::Mode::Off
                        } else {
                            crate::symmetry::Mode::Radial
                        },
                        segments,
                        angle: 23.0,
                        ..Default::default()
                    },
                };
                let mut fast = StrokeBuffer::default();
                let mut slow = StrokeBuffer::default();
                for i in 0..if large { 4 } else { 20 } {
                    let sample = crate::stroke::Sample {
                        x: 3.5 + i as f32 * 5.0,
                        y: 33.5 + (i % 4) as f32 * 0.8,
                        pressure: 0.2 + (i % 5) as f32 * 0.2,
                        distance: i as f64 * 2.0,
                        ..Default::default()
                    };
                    fast.paint(&mut actual, sample, &settings);
                    slow.paint(&mut reference, sample, &settings);
                }
                assert_eq!(
                    actual.layers[0].pixels, reference.layers[0].pixels,
                    "live case {case}/{segments}"
                );
                fast.finish_tail(&mut actual, &settings);
                slow.finish_tail(&mut reference, &settings);
                assert_eq!(
                    actual.layers[0].pixels, reference.layers[0].pixels,
                    "final case {case}/{segments}"
                );
                for (key, &block) in &fast.indices {
                    let other = slow.indices[key];
                    let state_bits = |buffer: &StrokeBuffer, index: usize| {
                        buffer.blocks[index]
                            .iter()
                            .map(|(rgba, a)| (*rgba, a.to_bits()))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        state_bits(&fast, block),
                        state_bits(&slow, other),
                        "coverage case {case}/{segments}"
                    );
                    let prefix_bits = |buffer: &StrokeBuffer, index: usize| {
                        buffer.prefix[index]
                            .as_ref()
                            .map(|values| values.iter().map(|a| a.to_bits()).collect::<Vec<_>>())
                    };
                    assert_eq!(
                        prefix_bits(&fast, block),
                        prefix_bits(&slow, other),
                        "prefix case {case}/{segments}"
                    );
                }
            }
        }
    }
    #[test]
    fn finished_prefix_retirement_reduces_memory_without_losing_rollback_pixels() {
        let mut doc = Document::new(64, 32);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        let original = doc.layers[0].pixels.clone();
        let mut buffer = StrokeBuffer::default();
        for x in [0, 32] {
            let mut writer = doc.layers[0].pixels.writer();
            let pixel = writer.rgba_mut(x);
            let alpha = buffer.accumulate_at(0, x, 0, pixel, 0.5, 1.0);
            crate::document::paint_pixel(pixel, alpha, [17, 83, 201], false, false);
            buffer.commit_coverage_at(0, x, 0, 0.2, 1.0);
        }
        buffer.tail.push_back(Sample::default());
        let painted = doc.layers[0].pixels.clone();
        let initial = buffer.payload_capacity_bytes();
        buffer.retire_finished_scratch(0);
        assert!(buffer.payload_capacity_bytes() < initial);
        assert!(buffer.prefix[0].is_none() && buffer.prefix[1].is_some());
        assert!(buffer.tail.is_empty() && buffer.tail.capacity() == 0);
        buffer.retire_finished_scratch(1);
        assert_eq!(doc.layers[0].pixels, painted);
        let mut restore = buffer.begin_restore(&doc);
        let mut previous = restore.payload_capacity_bytes();
        let initial = previous;
        while !restore.step(&mut doc, 1).unwrap().complete {
            let current = restore.payload_capacity_bytes();
            assert!(current <= previous);
            previous = current;
        }
        assert!(
            restore.payload_capacity_bytes() + 2 * 1024 * std::mem::size_of::<PixelState>()
                <= initial
        );
        assert_eq!(doc.layers[0].pixels, original);
    }
    #[test]
    fn row_sliced_restore_preserves_hidden_edges_and_reconstructs_damage() {
        for budget in [1, 7, 64] {
            for flat in [false, true] {
                let mut doc = Document::new(517, 259);
                if flat {
                    doc.layers[0].pixels = vec![0; 517 * 259 * 4].into();
                }
                doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(0)
                    .copy_from_slice(&[9, 8, 7, 0]);
                doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(517 * 259 - 1)
                    .copy_from_slice(&[6, 5, 4, 0]);
                let before = doc.layers[0].pixels.clone();
                let settings = Settings {
                    target: crate::paint_target::Target::Raster,
                    brush: crate::brush::Brush {
                        size: 45.0,
                        pressure_size: false,
                        taper_out: 30.0,
                        ..Default::default()
                    },
                    color: [20, 80, 160],
                    erase: false,
                    layer: 0,
                    symmetry: Default::default(),
                };
                let mut buffer = StrokeBuffer::default();
                for i in 0..23 {
                    buffer.paint(
                        &mut doc,
                        Sample {
                            x: 8.0 + i as f32 * 24.0,
                            y: 248.0,
                            pressure: 1.0,
                            distance: i as f64 * 24.0,
                            ..Default::default()
                        },
                        &settings,
                    );
                }
                assert_ne!(doc.layers[0].pixels, before);
                let mut display = doc.layers[0].pixels.clone();
                let mut cursor = buffer.begin_restore(&doc);
                let paused = cursor.step(&mut doc, 0).unwrap();
                assert_eq!(paused.work_units, 0);
                assert!(!paused.complete);
                let mut calls = 0;
                loop {
                    let step = cursor.step(&mut doc, budget).unwrap();
                    assert!(step.work_units <= budget);
                    for &rect in step.changed_region.rects() {
                        let bytes = doc.layers[0].pixels.region(doc.width, rect);
                        display.writer().write_region(doc.width, rect, &bytes);
                    }
                    assert_eq!(display, doc.layers[0].pixels);
                    calls += 1;
                    assert!(calls < 10000);
                    if step.complete {
                        break;
                    }
                }
                assert_eq!(doc.layers[0].pixels, before);
                assert_eq!(doc.layers[0].pixels.capacity(), before.capacity());
                assert!(cursor.buffer.blocks.is_empty());
                assert!(cursor.buffer.prefix.is_empty());
                let done = cursor.step(&mut doc, budget).unwrap();
                assert!(done.complete);
                assert_eq!(done.work_units, 0);
            }
        }
    }
    #[test]
    fn restore_guard_errors_keep_original_state_available_for_retry() {
        let mut doc = Document::new(37, 39);
        let before = doc.layers[0].pixels.clone();
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 18.0,
                ..Default::default()
            },
            color: [1, 2, 3],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut buffer = StrokeBuffer::default();
        buffer.paint(
            &mut doc,
            Sample {
                x: 16.0,
                y: 17.0,
                pressure: 1.0,
                ..Default::default()
            },
            &settings,
        );
        let preview = doc.layers[0].pixels.clone();
        let mut cursor = buffer.begin_restore(&doc);
        doc.width += 1;
        assert!(cursor.step(&mut doc, 64).is_err());
        assert_eq!(doc.layers[0].pixels, preview);
        doc.width -= 1;
        cursor.step(&mut doc, 1).unwrap();
        doc.layers[0].folder = true;
        assert!(cursor.step(&mut doc, 64).is_err());
        assert_eq!(doc.layers[0].pixels, preview);
        doc.layers[0].folder = false;
        while !cursor.step(&mut doc, 7).unwrap().complete {}
        assert_eq!(doc.layers[0].pixels, before);
    }
}
impl StrokeBuffer {
    fn configure(&mut self, doc: &crate::document::Document, settings: &Settings) -> bool {
        let Some(layer) = doc.layers.get(settings.layer) else {
            return false;
        };
        if !settings.target.valid(layer, [doc.width, doc.height]) {
            return false;
        }
        if self.target_set {
            return self.target == settings.target && self.layer_id == Some(layer.id);
        }
        if !self.indices.is_empty() && self.target != settings.target {
            return false;
        }
        self.target = settings.target;
        self.target_set = true;
        self.layer_id = Some(layer.id);
        if !self.target.is_raster() {
            self.mask_original = layer.mask.as_ref().map(|m| m.coverage.clone());
        }
        true
    }
    pub(crate) fn target_valid(&self, layer: &crate::document::Layer, size: [usize; 2]) -> bool {
        self.target.valid(layer, size) && self.layer_id.is_none_or(|id| id == layer.id)
    }
    fn restore_mask_identity(&self, doc: &mut crate::document::Document) {
        if let Some(source) = &self.mask_original
            && let Some(layer) = doc.layers.get_mut(doc.active)
            && self.target_valid(layer, [doc.width, doc.height])
        {
            layer.mask.as_mut().unwrap().coverage = source.clone();
        }
    }
    pub fn begin_restore(mut self, doc: &crate::document::Document) -> Restore {
        let key_capacity_bytes =
            self.indices.capacity() * std::mem::size_of::<((usize, usize, usize), usize)>();
        let keys = std::mem::take(&mut self.indices).into_iter();
        Restore {
            buffer: self,
            keys,
            key_capacity_bytes,
            current: None,
            reclaim: None,
            dimensions: [doc.width, doc.height],
            layers: doc.layers.len(),
            active: doc.active,
            complete: false,
        }
    }
    pub fn is_prefix_pass(&self) -> bool {
        self.prefix_pass
    }
    pub fn tail_peak_dabs(&self) -> usize {
        self.tail_peak
    }
    pub fn commit_coverage_at(&mut self, layer: usize, x: usize, y: usize, a: f32, ceiling: f32) {
        let key = (layer, x / 32, y / 32);
        let block = if let Some((cached_key, block)) = self.cached
            && cached_key == key
        {
            block
        } else {
            let Some(&block) = self.indices.get(&key) else {
                return;
            };
            self.cached = Some((key, block));
            block
        };
        let offset = (y % 32) * 32 + x % 32;
        if self.blocks[block][offset].1 < 0.0 {
            return;
        }
        let coverage = &mut self.prefix[block]
            .get_or_insert_with(|| vec![0.0; 1024].into_boxed_slice())[offset];
        let delivery = (a / ceiling.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        *coverage += delivery * (1.0 - *coverage);
        if self.rebuilding {
            self.blocks[block][offset].1 = *coverage;
        }
    }
    /// Apply a prefix-only raster row in 32px block spans. Lookup and coverage
    /// allocation happen once per span; per-pixel arithmetic/order is unchanged.
    pub(crate) fn commit_coverage_row(
        &mut self,
        layer: usize,
        left: usize,
        y: usize,
        alpha: &[f32],
        ceiling: f32,
    ) {
        let mut at = 0;
        while at < alpha.len() {
            let x = left + at;
            let count = (32 - x % 32).min(alpha.len() - at);
            let values = &alpha[at..at + count];
            let key = (layer, x / 32, y / 32);
            if let Some(&block) = self.indices.get(&key) {
                self.cached = Some((key, block));
                let offset = (y % 32) * 32 + x % 32;
                let states = &mut self.blocks[block][offset..offset + count];
                if values
                    .iter()
                    .zip(states.iter())
                    .any(|(a, state)| *a > 0.0 && state.1 >= 0.0)
                {
                    let coverage = self.prefix[block]
                        .get_or_insert_with(|| vec![0.0; 1024].into_boxed_slice());
                    for ((state, coverage), a) in states
                        .iter_mut()
                        .zip(coverage[offset..offset + count].iter_mut())
                        .zip(values)
                    {
                        if *a <= 0.0 || state.1 < 0.0 {
                            continue;
                        }
                        let delivery = (*a / ceiling.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
                        *coverage += delivery * (1.0 - *coverage);
                        if self.rebuilding {
                            state.1 = *coverage;
                        }
                    }
                }
            }
            at += count;
        }
    }
    fn render(
        &mut self,
        doc: &mut crate::document::Document,
        sample: crate::stroke::Sample,
        settings: &Settings,
    ) -> Painted {
        let mut result = Painted::default();
        for ([x, y], map) in settings
            .symmetry
            .copies(sample.x, sample.y, doc.width, doc.height)
        {
            let bounds =
                crate::render_region::dab_bounds(x, y, &settings.brush, doc.width, doc.height);
            if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
                continue;
            }
            result.region.include(bounds);
            result.dabs += 1;
            doc.dab_sample_buffered(
                crate::stroke::Sample { x, y, ..sample },
                &settings.brush,
                settings.color,
                settings.erase,
                map,
                Some(self),
            );
        }
        result
    }
    pub fn paint(
        &mut self,
        doc: &mut crate::document::Document,
        sample: crate::stroke::Sample,
        settings: &Settings,
    ) -> Painted {
        if doc.active != settings.layer || !self.configure(doc, settings) {
            return Painted::default();
        }
        let mut result = Painted::default();
        if settings.brush.taper_out > 0.0 {
            if self
                .tail
                .back()
                .is_some_and(|last| sample.distance < last.distance)
            {
                result.merge(self.finish_tail(doc, settings));
            }
            while self
                .tail
                .front()
                .is_some_and(|old| sample.distance - old.distance > settings.brush.taper_out as f64)
                || self.tail.len() >= MAX_TAIL_DABS
            {
                let old = self.tail.pop_front().unwrap();
                self.prefix_pass = true;
                // Commit old coverage without changing live pixels or dirty regions.
                let committed = self.render(doc, old, settings);
                result.dabs += committed.dabs;
                self.prefix_pass = false;
            }
        }
        result.merge(self.render(doc, sample, settings));
        if settings.brush.taper_out > 0.0 {
            self.tail.push_back(sample);
            self.tail_peak = self.tail_peak.max(self.tail.len());
        }
        result
    }
    /// A captured settings Arc is reused for every dab in the stroke. The caller
    /// owns this buffer exclusively until the cursor completes or is canceled.
    pub fn begin_dab(
        &mut self,
        doc: &crate::document::Document,
        sample: crate::stroke::Sample,
        settings: std::sync::Arc<Settings>,
    ) -> Option<DabPaint> {
        if doc.active != settings.layer || !self.configure(doc, &settings) {
            return None;
        }
        Some(DabPaint::new(self, doc, sample, settings))
    }
    /// Begin a resumable finalization operation. The caller exclusively owns this
    /// buffer/document/settings until it completes or restores the stroke.
    pub fn begin_tail(
        &mut self,
        doc: &crate::document::Document,
        settings: &Settings,
    ) -> Option<TailReplay> {
        if self.tail.is_empty()
            || doc.active != settings.layer
            || settings.brush.taper_out <= 0.0
            || self.target != settings.target
            || doc
                .layers
                .get(doc.active)
                .is_none_or(|layer| !self.target_valid(layer, [doc.width, doc.height]))
        {
            return None;
        }
        Some(TailReplay::new(
            std::mem::take(&mut self.tail),
            doc,
            settings,
        ))
    }
    pub fn finish_tail(
        &mut self,
        doc: &mut crate::document::Document,
        settings: &Settings,
    ) -> Painted {
        let Some(mut replay) = self.begin_tail(doc, settings) else {
            return Painted::default();
        };
        let mut result = Painted::default();
        loop {
            let step = replay
                .step(self, doc, 4096)
                .expect("Stable document during synchronous tail finalization");
            result.merge(step.painted);
            if step.complete {
                return result;
            }
        }
    }
    pub fn has_visible_changes(&self, document: &crate::document::Document) -> bool {
        self.indices.iter().any(|(&(layer, bx, by), &block)| {
            let Some(layer) = document.layers.get(layer) else {
                return true;
            };
            if !self.target_valid(layer, [document.width, document.height]) {
                return true;
            }
            self.blocks[block]
                .iter()
                .enumerate()
                .any(|(offset, (original, coverage))| {
                    if *coverage < 0.0 {
                        return false;
                    }
                    let (x, y) = (bx * 32 + offset % 32, by * 32 + offset / 32);
                    if x >= document.width || y >= document.height {
                        return true;
                    }
                    self.target.read(layer, document.width, x, y) != *original
                })
        })
    }
    pub fn restore(self, document: &mut crate::document::Document) {
        for (&(layer, bx, by), &block_index) in &self.indices {
            let Some(layer) = document.layers.get_mut(layer) else {
                continue;
            };
            if !self.target_valid(layer, [document.width, document.height]) {
                continue;
            }
            let mut writer = self.target.writer(layer, document.width);
            for (offset, (original, coverage)) in self.blocks[block_index].iter().enumerate() {
                let (x, y) = (bx * 32 + offset % 32, by * 32 + offset / 32);
                if *coverage < 0.0 || x >= document.width || y >= document.height {
                    continue;
                }
                writer.set(x, y, original);
            }
        }
        self.reclaim_empty_tiles(document);
        self.restore_mask_identity(document);
    }
    /// Inspect each touched document tile once, after all coverage blocks have
    /// finished restoring or committing. Hidden RGB prevents reclamation.
    pub fn reclaim_empty_tiles(&self, document: &mut crate::document::Document) {
        let side = self.target.side();
        let tiles: HashSet<_> = self
            .indices
            .keys()
            .map(|&(layer, bx, by)| (layer, bx * 32 / side, by * 32 / side))
            .collect();
        for (layer, tx, ty) in tiles {
            let Some(layer) = document.layers.get_mut(layer) else {
                continue;
            };
            if !self.target_valid(layer, [document.width, document.height]) {
                continue;
            }
            let (left, top) = (tx * side, ty * side);
            if left >= document.width || top >= document.height {
                continue;
            }
            self.target.reclaim(
                layer,
                document.width,
                [
                    left,
                    top,
                    (left + side).min(document.width),
                    (top + side).min(document.height),
                ],
            );
        }
    }
    pub fn payload_capacity_bytes(&self) -> usize {
        self.blocks
            .iter()
            .map(|block| block.len() * std::mem::size_of::<PixelState>())
            .sum::<usize>()
            + self.blocks.capacity() * std::mem::size_of::<Box<[PixelState]>>()
            + self.indices.capacity() * std::mem::size_of::<((usize, usize, usize), usize)>()
            + self
                .prefix
                .iter()
                .flatten()
                .map(|block| block.len() * std::mem::size_of::<f32>())
                .sum::<usize>()
            + self.prefix.capacity() * std::mem::size_of::<Option<Box<[f32]>>>()
            + self.tail.capacity() * std::mem::size_of::<crate::stroke::Sample>()
    }
    /// After tail finalization, prefix coverage and retained dabs are no longer
    /// needed. Original pixels and total coverage remain available for rollback.
    pub(crate) fn retire_finished_scratch(&mut self, prefix_index: usize) -> usize {
        let mut released = 0;
        if prefix_index == 0 {
            released += self.tail.capacity() * std::mem::size_of::<crate::stroke::Sample>();
            self.tail = VecDeque::new();
        }
        if let Some(prefix) = self.prefix.get_mut(prefix_index)
            && let Some(bytes) = prefix.take()
        {
            released += bytes.len() * std::mem::size_of::<f32>();
        }
        released
    }
    #[cfg(test)]
    pub fn accumulate(
        &mut self,
        layer: usize,
        index: usize,
        width: usize,
        pixel: &mut [u8],
        dab_alpha: f32,
        ceiling: f32,
    ) -> f32 {
        self.accumulate_at(
            layer,
            (index / 4) % width,
            (index / 4) / width,
            pixel,
            dab_alpha,
            ceiling,
        )
    }
    pub fn accumulate_at(
        &mut self,
        layer: usize,
        x: usize,
        y: usize,
        pixel: &mut [u8],
        dab_alpha: f32,
        ceiling: f32,
    ) -> f32 {
        let key = (layer, x / 32, y / 32);
        let block = if let Some((cached_key, block)) = self.cached
            && cached_key == key
        {
            block
        } else {
            let block = self.block(key);
            self.cached = Some((key, block));
            block
        };
        accumulate(
            &mut self.blocks[block][(y % 32) * 32 + x % 32],
            pixel,
            dab_alpha,
            ceiling,
        )
    }
    /// The index of the coverage block for `key`, created untouched if new.
    fn block(&mut self, key: (usize, usize, usize)) -> usize {
        *self.indices.entry(key).or_insert_with(|| {
            let index = self.blocks.len();
            self.blocks
                .push(vec![([0; 4], -1.0); 32 * 32].into_boxed_slice());
            self.prefix.push(None);
            index
        })
    }
    /// Make sure the 32 px block at (`bx`, `by`) of `layer` exists.
    pub(crate) fn touch_block(&mut self, layer: usize, bx: usize, by: usize) {
        self.block((layer, bx, by));
    }
    /// Separate mutable access to the existing blocks of `layer` in block rows
    /// `rows` and columns `columns`, indexed `[row][column]`, so row bands can
    /// accumulate on different threads.
    pub(crate) fn band_blocks(
        &mut self,
        layer: usize,
        rows: std::ops::Range<usize>,
        columns: std::ops::Range<usize>,
    ) -> Vec<Vec<Option<&mut [PixelState]>>> {
        let mut places = vec![None; self.blocks.len()];
        for by in rows.clone() {
            for bx in columns.clone() {
                if let Some(&block) = self.indices.get(&(layer, bx, by)) {
                    places[block] = Some((by - rows.start, bx - columns.start));
                }
            }
        }
        let mut bands: Vec<Vec<Option<&mut [PixelState]>>> = rows
            .map(|_| columns.clone().map(|_| None).collect())
            .collect();
        for (block, place) in self.blocks.iter_mut().zip(places) {
            if let Some((row, column)) = place {
                bands[row][column] = Some(&mut block[..]);
            }
        }
        bands
    }
}

/// Add one dab's delivery to a pixel's stroke coverage, recording the original
/// pixel the first time and restoring it into `pixel`. Returns the alpha to
/// paint over that original.
pub(crate) fn accumulate(
    entry: &mut PixelState,
    pixel: &mut [u8],
    dab_alpha: f32,
    ceiling: f32,
) -> f32 {
    if entry.1 < 0.0 {
        *entry = (pixel.try_into().unwrap(), 0.0);
    }
    let delivery = (dab_alpha / ceiling.max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    entry.1 += delivery * (1.0 - entry.1);
    pixel.copy_from_slice(&entry.0);
    entry.1 * ceiling
}

#[cfg(test)]
mod tests {
    fn crossing_samples(brush: &Brush) -> Vec<crate::stroke::Sample> {
        let mut stroke = crate::stroke::Stroke::default();
        let mut dabs = Vec::new();
        for (i, (x, y)) in [
            (2.0, 3.0),
            (72.0, 68.0),
            (2.0, 68.0),
            (72.0, 3.0),
            (37.0, 36.0),
        ]
        .into_iter()
        .enumerate()
        {
            let sample = crate::stroke::Sample {
                x,
                y,
                pressure: 0.5 + i as f32 * 0.1,
                pressure_known: true,
                time: Some(i as f64 * 0.05),
                ..Default::default()
            };
            dabs.extend(stroke.sample(sample, brush));
            if i == 4 {
                dabs.extend(stroke.finish(sample, brush));
            }
        }
        dabs
    }

    /// Independent whole-stroke reference: taper every emitted dab before any paint.
    fn reference_tail(doc: &mut Document, dabs: &[crate::stroke::Sample], settings: &Settings) {
        let end = dabs.last().unwrap().distance;
        let mut buffer = StrokeBuffer::default();
        for mut sample in dabs.iter().copied() {
            sample.taper = Some(
                sample.taper.unwrap_or(1.0)
                    * (0.05
                        + 0.95
                            * ((end - sample.distance) / settings.brush.taper_out as f64)
                                .clamp(0.0, 1.0) as f32),
            );
            for ([x, y], map) in settings
                .symmetry
                .copies(sample.x, sample.y, doc.width, doc.height)
            {
                doc.dab_sample_buffered(
                    crate::stroke::Sample { x, y, ..sample },
                    &settings.brush,
                    settings.color,
                    settings.erase,
                    map,
                    Some(&mut buffer),
                );
            }
        }
    }

    #[test]
    fn bounded_tail_matches_full_replay_at_crossings_with_symmetry_eraser_and_alpha_lock() {
        for (erase, alpha_locked) in [(false, false), (true, false), (false, true)] {
            let mut doc = Document::new(75, 73);
            for i in 0..75 * 73 {
                doc.layers[0].pixels.writer().rgba_mut(i).copy_from_slice(&[
                    13,
                    67,
                    121,
                    if i % 7 == 0 { 0 } else { 170 },
                ]);
            }
            doc.layers[0].alpha_locked = alpha_locked;
            let before = doc.clone();
            let settings = Settings {
                target: crate::paint_target::Target::Raster,
                brush: Brush {
                    size: 13.0,
                    aspect: 0.7,
                    angle: 37.0,
                    tip: crate::brush::Tip::Square,
                    opacity: 0.6,
                    flow: 0.27,
                    hardness: 0.4,
                    grain: Some(crate::brush::TipImage {
                        width: 2,
                        height: 2,
                        alpha: (vec![40, 190, 255, 100]).into(),
                    }),
                    grain_amount: 0.35,
                    scatter: 0.2,
                    taper_in: 17.0,
                    taper_out: 28.0,
                    adjust_flow_by_spacing: true,
                    ..Default::default()
                },
                color: [210, 90, 35],
                erase,
                layer: 0,
                symmetry: crate::symmetry::Symmetry {
                    mode: crate::symmetry::Mode::FourWay,
                    ..Default::default()
                },
            };
            let samples = crossing_samples(&settings.brush);
            assert!(samples.last().unwrap().distance > 8.0 * settings.brush.taper_out as f64);
            let mut expected = before.clone();
            reference_tail(&mut expected, &samples, &settings);
            let mut history = History::new();
            history.begin_stroke_checkpoint(&doc);
            let mut buffer = StrokeBuffer::default();
            for &sample in &samples {
                buffer.paint(&mut doc, sample, &settings);
            }
            assert!(buffer.prefix.iter().any(Option::is_some));
            assert!(!buffer.tail.is_empty());
            let visible = doc.layers[0].pixels.clone();
            assert!(!buffer.finish_tail(&mut doc, &settings).region.is_empty());
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
            assert_ne!(doc.layers[0].pixels, visible);
            assert!(buffer.finish_tail(&mut doc, &settings).region.is_empty());
            history.settle(&doc);
            assert!(history.undo(&mut doc));
            assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
            assert!(history.redo(&mut doc));
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
            buffer.restore(&mut doc);
            assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
            for budget in [1, 31, 128] {
                let mut sliced = before.clone();
                let mut buffer = StrokeBuffer::default();
                for &sample in &samples {
                    buffer.paint(&mut sliced, sample, &settings);
                }
                let preview = sliced.layers[0].pixels.clone();
                let mut cursor = buffer.begin_tail(&sliced, &settings).unwrap();
                let zero = cursor.step(&mut buffer, &mut sliced, 0).unwrap();
                assert_eq!(zero.work_units, 0);
                assert!(zero.changed_region.is_empty());
                assert!(!zero.complete);
                assert_eq!(sliced.layers[0].pixels, preview);
                loop {
                    let step = cursor.step(&mut buffer, &mut sliced, budget).unwrap();
                    assert!(step.work_units <= budget);
                    assert!(!buffer.prefix_pass && !buffer.rebuilding);
                    if step.complete {
                        break;
                    }
                }
                assert_eq!(sliced.layers[0].pixels, expected.layers[0].pixels);
                assert!(cursor.step(&mut buffer, &mut sliced, 1).unwrap().complete);
                buffer.restore(&mut sliced);
                assert_eq!(sliced.layers[0].pixels, before.layers[0].pixels);
            }
        }
    }

    #[test]
    fn maximum_tail_is_bounded_and_cancellation_restores_live_preview() {
        let mut doc = Document::new(9, 9);
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: Brush {
                size: 1.0,
                taper_out: 300.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [1, 2, 3],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut buffer = StrokeBuffer::default();
        for i in 0..5000 {
            buffer.paint(
                &mut doc,
                crate::stroke::Sample {
                    x: 4.5,
                    y: 4.5,
                    pressure: 1.0,
                    distance: i as f64 * 0.5,
                    ..Default::default()
                },
                &settings,
            );
            assert!(buffer.tail.len() <= MAX_TAIL_DABS);
        }
        assert_eq!(buffer.tail_peak_dabs(), 601);
        assert!(buffer.payload_capacity_bytes() < 150_000);
        buffer.restore(&mut doc);
        assert_eq!(doc.layers[0].pixels.capacity(), 0);
    }

    #[test]
    fn restarting_arc_seals_previous_segment_without_losing_overlap() {
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: Brush {
                size: 7.0,
                taper_out: 12.0,
                opacity: 0.5,
                flow: 0.2,
                ..Default::default()
            },
            color: [1, 80, 200],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let samples = crossing_samples(&settings.brush);
        let mut a = Document::new(75, 73);
        let mut b = a.clone();
        let mut buffer = StrokeBuffer::default();
        for &sample in &samples {
            buffer.paint(&mut a, sample, &settings);
        }
        buffer.finish_tail(&mut a, &settings);
        for &sample in &samples {
            buffer.paint(&mut a, sample, &settings);
        }
        buffer.finish_tail(&mut a, &settings);
        let mut buffer = StrokeBuffer::default();
        for &sample in samples.iter().chain(samples.iter()) {
            buffer.paint(&mut b, sample, &settings);
        }
        buffer.finish_tail(&mut b, &settings);
        assert_eq!(a.layers[0].pixels, b.layers[0].pixels);
    }

    #[test]
    fn rollback_reclaims_new_tiles_but_keeps_existing_hidden_rgb_across_layers() {
        let mut doc = Document::new(517, 259);
        doc.add_layer();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(258 * 517 + 516)
            .copy_from_slice(&[7, 8, 9, 0]);
        let before = doc.clone();
        let mut buffer = StrokeBuffer::default();
        let brush = Brush {
            size: 18.0,
            pressure_size: false,
            ..Brush::default()
        };
        for layer in 0..2 {
            doc.active = layer;
            for (x, y) in [(255.5, 255.5), (516.5, 258.5)] {
                doc.dab_buffered(
                    x,
                    y,
                    1.0,
                    &brush,
                    [100, 80, 60],
                    false,
                    IDENTITY,
                    Some(&mut buffer),
                );
            }
        }
        assert!(buffer.has_visible_changes(&doc));
        assert!(doc.layers[0].pixels.capacity() > before.layers[0].pixels.capacity());
        buffer.restore(&mut doc);
        assert_eq!(
            doc.layers[0].pixels.capacity(),
            crate::pixels::SIDE * crate::pixels::SIDE * 4
        );
        assert_eq!(doc.layers[1].pixels.capacity(), 0);
        for layer in 0..2 {
            for index in 0..517 * 259 {
                assert_eq!(
                    doc.layers[layer].pixels.rgba(index),
                    before.layers[layer].pixels.rgba(index)
                );
            }
        }
    }
    use super::*;
    #[test]
    fn canceled_checkpoint_preserves_redo_and_committed_stroke_clears_it() {
        use crate::{
            brush::Brush,
            document::{Document, History},
        };
        let mut doc = Document::new(8, 8);
        let mut history = History::new();
        history.checkpoint(&doc);
        doc.dab(4.0, 4.0, 1.0, &Brush::default(), [20, 40, 60], false);
        let painted = doc.layers[0].pixels.clone();
        assert!(history.undo(&mut doc));
        assert!(history.can_redo());
        let blank = doc.layers[0].pixels.clone();
        history.begin_stroke_checkpoint(&doc);
        doc.dab(2.0, 2.0, 1.0, &Brush::default(), [90, 80, 70], false);
        assert!(history.discard_pending_checkpoint(&doc));
        doc.layers[0].pixels = blank;
        assert!(history.can_redo());
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, painted);
        assert!(history.undo(&mut doc));
        history.begin_stroke_checkpoint(&doc);
        doc.dab(2.0, 2.0, 1.0, &Brush::default(), [90, 80, 70], false);
        history.settle(&doc);
        assert!(!history.can_redo());
        assert!(history.undo(&mut doc));
        assert!(doc.layers[0].pixels.iter().all(|value| *value == 0));
    }
    #[test]
    fn block_boundaries_and_layer_switches_match_pixel_reference() {
        let mut buffer = StrokeBuffer::default();
        let mut reference = HashMap::<(usize, usize), PixelState>::new();
        for step in 0..2000 {
            let layer = step % 3;
            let index = ((step * 31) % (37 * 65)) * 4;
            let mut pixel = [step as u8, 40, 80, 120];
            let entry = reference.entry((layer, index)).or_insert((pixel, 0.0));
            entry.1 += 0.2 * (1.0 - entry.1);
            let alpha = buffer.accumulate(layer, index, 37, &mut pixel, 0.1, 0.5);
            assert_eq!(pixel, entry.0);
            assert_eq!(alpha, entry.1 * 0.5);
        }
    }
    use crate::{
        brush::Brush,
        document::{Document, History},
        symmetry::IDENTITY,
    };
    #[test]
    fn repeated_dabs_respect_stroke_ceiling_and_undo_exactly() {
        let mut doc = Document::new(16, 16);
        let before = doc.layers[0].pixels.clone();
        let brush = Brush {
            size: 8.0,
            hardness: 1.0,
            opacity: 0.5,
            flow: 1.0,
            pressure_size: false,
            ..Brush::default()
        };
        let mut history = History::new();
        history.checkpoint(&doc);
        let mut buffer = StrokeBuffer::default();
        for _ in 0..20 {
            doc.dab_buffered(
                8.5,
                8.5,
                1.0,
                &brush,
                [30, 50, 70],
                false,
                IDENTITY,
                Some(&mut buffer),
            );
        }
        assert_eq!(doc.layers[0].pixels[(8 * 16 + 8) * 4 + 3], 128);
        assert!(doc.layers[0].pixels.chunks_exact(4).all(|p| p[3] <= 128));
        let after = doc.layers[0].pixels.clone();
        history.settle(&doc);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, before);
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels, after);
    }
    #[test]
    fn subbyte_flow_accumulates_and_eraser_uses_original_alpha() {
        let mut doc = Document::new(16, 16);
        let brush = Brush {
            size: 8.0,
            hardness: 1.0,
            opacity: 0.5,
            flow: 0.001,
            pressure_size: false,
            ..Brush::default()
        };
        let mut buffer = StrokeBuffer::default();
        for _ in 0..1000 {
            doc.dab_buffered(
                8.5,
                8.5,
                1.0,
                &brush,
                [0, 0, 0],
                false,
                IDENTITY,
                Some(&mut buffer),
            );
        }
        let index = (8 * 16 + 8) * 4 + 3;
        assert!((79..=82).contains(&doc.layers[0].pixels[index]));
        doc.layers[0].pixels[index] = 255;
        let brush = Brush { flow: 1.0, ..brush };
        let mut buffer = StrokeBuffer::default();
        for _ in 0..10 {
            doc.dab_buffered(
                8.5,
                8.5,
                1.0,
                &brush,
                [0, 0, 0],
                true,
                IDENTITY,
                Some(&mut buffer),
            );
        }
        assert_eq!(doc.layers[0].pixels[index], 128);
    }
}
