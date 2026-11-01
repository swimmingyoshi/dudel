//! Immutable selection coverage, independent of document/UI integration.
//! Polygon membership uses even/odd crossings at half-open pixel centers.
use crate::selection::Selection;
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Combine {
    Replace,
    Add,
    Subtract,
    Intersect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    width: usize,
    height: usize,
    coverage: Arc<Vec<u8>>,
    bounds: Option<Selection>,
}

/// Zero-initialized coverage with bounds accumulated during sparse generation.
/// A clearing write invalidates the cache and uses the checked full scan.
pub(crate) struct Builder {
    width: usize,
    height: usize,
    coverage: Vec<u8>,
    bounds: Selection,
    needs_scan: bool,
}
impl Builder {
    pub(crate) fn new(width: usize, height: usize) -> Result<Self, String> {
        let count = pixels(width, height)?;
        Ok(Self {
            width,
            height,
            coverage: vec![0; count],
            bounds: Selection {
                left: width,
                top: height,
                right: 0,
                bottom: 0,
            },
            needs_scan: false,
        })
    }
    pub(crate) fn set(&mut self, x: usize, y: usize, value: u8) {
        assert!(x < self.width && y < self.height);
        let index = y * self.width + x;
        if value == 0 && self.coverage[index] != 0 {
            self.needs_scan = true;
        }
        self.coverage[index] = value;
        if value > 0 {
            self.bounds.left = self.bounds.left.min(x);
            self.bounds.top = self.bounds.top.min(y);
            self.bounds.right = self.bounds.right.max(x + 1);
            self.bounds.bottom = self.bounds.bottom.max(y + 1);
        }
    }
    pub(crate) fn finish(self, cancel: Option<&AtomicBool>) -> Result<Mask, String> {
        crate::fill::check_cancel(cancel)?;
        if self.needs_scan {
            return Mask::from_coverage_control(self.width, self.height, self.coverage, cancel);
        }
        Ok(Mask {
            width: self.width,
            height: self.height,
            coverage: Arc::new(self.coverage),
            bounds: self
                .bounds
                .valid(self.width, self.height)
                .then_some(self.bounds),
        })
    }
}
pub(crate) fn pixels(width: usize, height: usize) -> Result<usize, String> {
    let count = width
        .checked_mul(height)
        .ok_or("Selection dimensions overflow")?;
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || count > crate::document::MAX_PIXELS
    {
        return Err("Unsupported selection dimensions".into());
    }
    Ok(count)
}
/// Blend a filter result through selection coverage in premultiplied RGBA.
/// Full coverage preserves the program's exact bytes, including hidden RGB.
pub(crate) fn mix_rgba(before: [u8; 4], after: [u8; 4], coverage: u8) -> [u8; 4] {
    if coverage == 0 {
        return before;
    }
    if coverage == 255 {
        return after;
    }
    let (old, new) = (255 - u32::from(coverage), u32::from(coverage));
    let alpha = u32::from(before[3]) * old + u32::from(after[3]) * new;
    let mut result = [0; 4];
    result[3] = ((alpha + 127) / 255) as u8;
    for c in 0..3 {
        result[c] = (u32::from(before[c]) * u32::from(before[3]) * old
            + u32::from(after[c]) * u32::from(after[3]) * new
            + alpha / 2)
            .checked_div(alpha)
            .unwrap_or_else(|| (u32::from(before[c]) * old + u32::from(after[c]) * new + 127) / 255)
            as u8;
    }
    result
}
impl Mask {
    pub(crate) fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.coverage, &other.coverage)
    }
    pub(crate) fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.coverage
    }
    pub(crate) fn rotated_clockwise_control(
        &self,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        crate::fill::check_cancel(cancel)?;
        let mut bytes = vec![0; self.coverage.len()];
        for y in 0..self.height {
            crate::fill::check_cancel(cancel)?;
            for x in 0..self.width {
                bytes[x * self.height + self.height - 1 - y] = self.at(x, y);
            }
        }
        Self::from_coverage_control(self.height, self.width, bytes, cancel)
    }
    pub(crate) fn from_coverage(
        width: usize,
        height: usize,
        coverage: Vec<u8>,
    ) -> Result<Self, String> {
        Self::from_coverage_control(width, height, coverage, None)
    }
    pub(crate) fn from_coverage_control(
        width: usize,
        height: usize,
        coverage: Vec<u8>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        crate::fill::check_cancel(cancel)?;
        if pixels(width, height)? != coverage.len() {
            return Err("Invalid selection coverage length".into());
        }
        let mut bounds = Selection {
            left: width,
            top: height,
            right: 0,
            bottom: 0,
        };
        for (index, &value) in coverage.iter().enumerate() {
            if index % 4096 == 0 {
                crate::fill::check_cancel(cancel)?;
            }
            if value > 0 {
                let (x, y) = (index % width, index / width);
                bounds.left = bounds.left.min(x);
                bounds.top = bounds.top.min(y);
                bounds.right = bounds.right.max(x + 1);
                bounds.bottom = bounds.bottom.max(y + 1);
            }
        }
        let bounds = bounds.valid(width, height).then_some(bounds);
        Ok(Self {
            width,
            height,
            coverage: Arc::new(coverage),
            bounds,
        })
    }
    pub(crate) fn rectangle(width: usize, height: usize, rect: Selection) -> Result<Self, String> {
        Self::rectangle_control(width, height, rect, None)
    }
    pub(crate) fn rectangle_control(
        width: usize,
        height: usize,
        rect: Selection,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        crate::fill::check_cancel(cancel)?;
        let mut coverage = vec![0; pixels(width, height)?];
        if !rect.valid(width, height) {
            return Err("Invalid selection rectangle".into());
        }
        for y in rect.top..rect.bottom {
            crate::fill::check_cancel(cancel)?;
            coverage[y * width + rect.left..y * width + rect.right].fill(255);
        }
        Self::from_coverage_control(width, height, coverage, cancel)
    }
    pub(crate) fn polygon(
        width: usize,
        height: usize,
        points: &[[f64; 2]],
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        let count = pixels(width, height)?;
        if !(3..=16_384).contains(&points.len())
            || points
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1.0e9)
        {
            return Err("Invalid or oversized lasso path".into());
        }
        crate::fill::check_cancel(cancel)?;
        let mut coverage = vec![0; count];
        let mut crossings = Vec::with_capacity(points.len());
        for y in 0..height {
            crate::fill::check_cancel(cancel)?;
            crossings.clear();
            let scan = y as f64 + 0.5;
            for i in 0..points.len() {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                if (a[1] <= scan && scan < b[1]) || (b[1] <= scan && scan < a[1]) {
                    crossings.push(a[0] + (scan - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            crossings.sort_unstable_by(f64::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let left = (pair[0] - 0.5).ceil().clamp(0.0, width as f64) as usize;
                let right = (pair[1] - 0.5).ceil().clamp(0.0, width as f64) as usize;
                coverage[y * width + left..y * width + right].fill(255);
            }
        }
        crate::fill::check_cancel(cancel)?;
        Self::from_coverage_control(width, height, coverage, cancel)
    }
    pub(crate) fn at(&self, x: usize, y: usize) -> u8 {
        if x < self.width && y < self.height {
            self.coverage[y * self.width + x]
        } else {
            0
        }
    }
    pub(crate) fn allocation(&self) -> (usize, usize) {
        (self.coverage.as_ptr() as usize, self.coverage.capacity())
    }
    pub(crate) fn bounds(&self) -> Option<Selection> {
        self.bounds
    }
    /// Every pixel's coverage reversed: unselected becomes fully selected.
    pub(crate) fn inverted(&self, cancel: Option<&AtomicBool>) -> Result<Self, String> {
        let coverage = self.coverage.iter().map(|value| 255 - value).collect();
        Self::from_coverage_control(self.width, self.height, coverage, cancel)
    }
    pub(crate) fn combine(
        &self,
        other: &Self,
        mode: Combine,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        if (self.width, self.height) != (other.width, other.height) {
            return Err("Selection dimensions differ".into());
        }
        crate::fill::check_cancel(cancel)?;
        if matches!(mode, Combine::Replace) {
            return Ok(other.clone());
        }
        let mut coverage = Vec::with_capacity(self.coverage.len());
        for (index, (&a, &b)) in self.coverage.iter().zip(other.coverage.iter()).enumerate() {
            if index & 1023 == 0 {
                crate::fill::check_cancel(cancel)?;
            }
            coverage.push(match mode {
                Combine::Replace => unreachable!(),
                Combine::Add => a.max(b),
                Combine::Intersect => a.min(b),
                Combine::Subtract => a.saturating_sub(b),
            });
        }
        crate::fill::check_cancel(cancel)?;
        Self::from_coverage_control(self.width, self.height, coverage, cancel)
    }
    /// Separable box feather. Canvas-edge windows are clipped; each pass rounds
    /// to the nearest byte. Zero radius shares the original allocation.
    pub(crate) fn feather(
        &self,
        radius: usize,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        if radius > 64 {
            return Err("Unsupported selection feather radius".into());
        }
        crate::fill::check_cancel(cancel)?;
        if radius == 0 {
            return Ok(self.clone());
        }
        let mut source = self.coverage.as_ref().clone();
        for vertical in [false, true] {
            let mut output = vec![0; source.len()];
            let (lines, length) = if vertical {
                (self.width, self.height)
            } else {
                (self.height, self.width)
            };
            for line in 0..lines {
                crate::fill::check_cancel(cancel)?;
                let index = |p| {
                    if vertical {
                        p * self.width + line
                    } else {
                        line * self.width + p
                    }
                };
                let mut sum: usize = (0..=radius.min(length - 1))
                    .map(|p| usize::from(source[index(p)]))
                    .sum();
                for p in 0..length {
                    let count = (p + radius + 1).min(length) - p.saturating_sub(radius);
                    output[index(p)] = ((sum + count / 2) / count) as u8;
                    if p >= radius {
                        sum -= usize::from(source[index(p - radius)]);
                    }
                    if p + radius + 1 < length {
                        sum += usize::from(source[index(p + radius + 1)]);
                    }
                }
            }
            source = output;
        }
        crate::fill::check_cancel(cancel)?;
        Self::from_coverage_control(self.width, self.height, source, cancel)
    }
}

impl serde::Serialize for Mask {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(serde::Serialize)]
        struct Wire<'a> {
            width: usize,
            height: usize,
            coverage: &'a [u8],
        }
        Wire {
            width: self.width,
            height: self.height,
            coverage: self.bytes(),
        }
        .serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for Mask {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            width: usize,
            height: usize,
            #[serde(deserialize_with = "read_coverage")]
            coverage: Vec<u8>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::from_coverage(wire.width, wire.height, wire.coverage)
            .map_err(serde::de::Error::custom)
    }
}
fn read_coverage<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    struct Coverage;
    impl<'de> serde::de::Visitor<'de> for Coverage {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded selection coverage bytes")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut bytes = Vec::with_capacity(
                seq.size_hint()
                    .unwrap_or(0)
                    .min(crate::document::MAX_PIXELS),
            );
            while let Some(byte) = seq.next_element::<u8>()? {
                if bytes.len() == crate::document::MAX_PIXELS {
                    return Err(serde::de::Error::custom(
                        "Selection coverage exceeds supported area",
                    ));
                }
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }
    deserializer.deserialize_seq(Coverage)
}

pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        let p = size as f64;
        let started = std::time::Instant::now();
        let lasso = Mask::polygon(
            size,
            size,
            &[
                [p / 4.0, p / 4.0],
                [p * 3.0 / 4.0, p / 4.0],
                [p * 3.0 / 4.0, p * 3.0 / 4.0],
                [p / 4.0, p * 3.0 / 4.0],
            ],
            None,
        )?;
        let polygon_ms = started.elapsed().as_secs_f64() * 1000.0;
        let rect = Mask::rectangle(
            size,
            size,
            Selection {
                left: size / 4,
                top: size / 4,
                right: size * 3 / 4,
                bottom: size * 3 / 4,
            },
        )?;
        if lasso.coverage != rect.coverage {
            return Err("Lasso rectangle pixels differ".into());
        }
        let right = Mask::rectangle(
            size,
            size,
            Selection {
                left: size / 2,
                top: 0,
                right: size,
                bottom: size,
            },
        )?;
        for mode in [
            Combine::Replace,
            Combine::Add,
            Combine::Subtract,
            Combine::Intersect,
        ] {
            let started = std::time::Instant::now();
            let combined = lasso.combine(&right, mode, None)?;
            let combine_ms = started.elapsed().as_secs_f64() * 1000.0;
            for y in 0..size {
                for x in 0..size {
                    let a = x >= size / 4 && x < size * 3 / 4 && y >= size / 4 && y < size * 3 / 4;
                    let b = x >= size / 2;
                    let included = match mode {
                        Combine::Replace => b,
                        Combine::Add => a || b,
                        Combine::Subtract => a && !b,
                        Combine::Intersect => a && b,
                    };
                    if combined.at(x, y) != if included { 255 } else { 0 } {
                        return Err("Selection combination pixels differ".into());
                    }
                }
            }
            let started = std::time::Instant::now();
            let feathered = combined.feather(8, None)?;
            let feather_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut doc = crate::document::Document::new(size, size);
            doc.selection = lasso.bounds();
            doc.selection_mask = Some(lasso.clone());
            let edit = crate::document_edits::Edit::SelectionPolygon {
                points: vec![[p / 2.0, 0.0], [p, 0.0], [p, p], [p / 2.0, p]],
                mode,
            };
            let (mut queued, mut history, mut measurement) =
                crate::render_queue::measure_edit(&doc, edit)?;
            if queued.selection_mask.as_ref() != Some(&combined) {
                return Err("Queued selection combination differs".into());
            }
            if !history.undo(&mut queued)
                || queued.selection_mask != doc.selection_mask
                || !history.redo(&mut queued)
                || queued.selection_mask.as_ref() != Some(&combined)
            {
                return Err("Queued selection undo/redo differs".into());
            }
            let (feather_doc, _, feather_measurement) = crate::render_queue::measure_edit(
                &queued,
                crate::document_edits::Edit::FeatherSelection(8),
            )?;
            if feather_doc.selection_mask.as_ref() != Some(&feathered) {
                return Err("Queued feather selection differs".into());
            }
            measurement["coverage_undo_redo_verified"] = serde_json::json!(true);
            let (_, capacity) = feathered.allocation();
            if feathered.bounds().is_none() || lasso.clone().allocation() != lasso.allocation() {
                return Err("Selection mask ownership/bounds differ".into());
            }
            cases.push(serde_json::json!({"size":size,"mode":mode,"polygon_ms":polygon_ms,"combine_ms":combine_ms,"feather_ms":feather_ms,"result_capacity_bytes":capacity,"combination_pixels_verified":true,"clone_shares_payload":true,"queued_combination":measurement,"queued_feather":feather_measurement}));
        }
    }
    crate::storage::atomic_write(path,&serde_json::to_vec_pretty(&serde_json::json!({"version":3,"scope":"One repetition per size/mode. Direct polygon/combine/box-feather includes construction/bounds, excludes verification. Queued combination and feather measure admission and owner advance with a 2 ms yield deadline, including worker start/source checks/publication/history/snapshot. Worker joins are outside owner timing, inside wall time. Coverage matches direct engine results; combination independently checked against rectangle boolean oracle, undo/redo checked; feather oracle is in unit tests. Payload capacity excludes transient buffers and RSS. No GUI, display, input, persistence or physical lasso delivery measurement.","cases":cases})).map_err(|e|e.to_string())?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn sparse_builder_matches_scanned_bounds_including_overwrites_and_cleared_edges() {
        let updates = [
            (64, 66, 255),
            (0, 0, 64),
            (32, 31, 128),
            (64, 66, 0),
            (32, 31, 192),
            (0, 0, 0),
            (32, 31, 0),
            (63, 65, 1),
        ];
        for end in 0..=updates.len() {
            let mut builder = Builder::new(65, 67).unwrap();
            let mut bytes = vec![0; 65 * 67];
            for &(x, y, value) in updates.iter().take(end) {
                builder.set(x, y, value);
                bytes[y * 65 + x] = value;
            }
            let mask = builder.finish(None).unwrap();
            let reference = Mask::from_coverage(65, 67, bytes).unwrap();
            assert_eq!(mask, reference);
        }
        let mut builder = Builder::new(65, 67).unwrap();
        builder.set(32, 31, 255);
        assert!(builder.finish(Some(&AtomicBool::new(true))).is_err());
        assert!(Builder::new(0, 67).is_err());
        assert!(Builder::new(10001, 10000).is_err());
        assert!(Builder::new(16385, 1).is_err());
        assert!(Builder::new(4096, 4096).is_ok());
    }
    use super::*;
    #[test]
    fn filter_interpolation_preserves_endpoints_and_uses_premultiplied_color() {
        assert_eq!(
            mix_rgba([0, 0, 255, 255], [255, 0, 0, 0], 128),
            [0, 0, 255, 127]
        );
        assert_eq!(
            mix_rgba([0, 0, 255, 255], [255, 0, 0, 255], 128),
            [128, 0, 127, 255]
        );
        assert_eq!(mix_rgba([1, 2, 3, 0], [5, 7, 9, 0], 0), [1, 2, 3, 0]);
        assert_eq!(mix_rgba([1, 2, 3, 0], [5, 7, 9, 0], 255), [5, 7, 9, 0]);
    }
    fn selected_doc(width: usize, height: usize, coverage: Vec<u8>) -> crate::document::Document {
        let mask = Mask::from_coverage(width, height, coverage).unwrap();
        let mut doc = crate::document::Document::new(width, height);
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        doc
    }
    #[test]
    fn masked_clear_noop_keeps_sharing_and_reclaims_only_zero_tiles() {
        let mut doc = selected_doc(517, 259, vec![255; 517 * 259]);
        let blank = doc.layers[0].pixels.clone();
        assert!(!doc.clear_selected().unwrap());
        assert!(doc.layers[0].pixels.shares(&blank));
        assert_eq!(doc.layers[0].pixels.capacity(), blank.capacity());
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(256 * 517 + 256)
            .copy_from_slice(&[7, 5, 3, 255]);
        assert!(doc.clear_selected().unwrap());
        assert_eq!(doc.layers[0].pixels.capacity(), blank.capacity());
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(256 * 517 + 256)
            .copy_from_slice(&[7, 5, 3, 0]);
        let hidden = doc.layers[0].pixels.clone();
        assert!(!doc.clear_selected().unwrap());
        assert!(doc.layers[0].pixels.shares(&hidden));
    }
    #[test]
    fn canvas_rotation_preserves_fractional_mask_bounds_and_four_turns_restore_document() {
        let mut doc = selected_doc(5, 3, vec![0, 0, 0, 0, 0, 0, 128, 255, 0, 0, 0, 0, 64, 0, 0]);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(6)
            .copy_from_slice(&[7, 5, 3, 0]);
        let original = doc.clone();
        doc.rotate_canvas_clockwise();
        assert!(doc.validate().is_ok());
        assert_eq!((doc.width, doc.height), (3, 5));
        assert_eq!(doc.selection_mask.as_ref().unwrap().at(1, 1), 128);
        assert_eq!(doc.selection_mask.as_ref().unwrap().at(0, 2), 64);
        assert_eq!(doc.layers[0].pixels.rgba(4), [7, 5, 3, 0]);
        for _ in 0..3 {
            doc.rotate_canvas_clockwise();
        }
        assert_eq!(doc.selection_mask, original.selection_mask);
        assert_eq!(doc.selection, original.selection);
        assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
    }
    #[test]
    fn queued_mask_fill_snapshot_and_history_restore_fractional_coverage() {
        let mut coverage = vec![0; 24];
        coverage[9] = 255;
        coverage[10] = 128;
        coverage[12] = 255;
        coverage[17..21].fill(255);
        let mask = Mask::from_coverage(8, 3, coverage.clone()).unwrap();
        let mut doc = crate::document::Document::new(8, 3);
        let target = doc.layers[0].id;
        let mut history = crate::document::History::new();
        let mut queue = crate::render_queue::Queue::default();
        queue
            .edit(crate::document_edits::Edit::SelectionMask(Some(
                mask.clone(),
            )))
            .unwrap();
        queue
            .edit(crate::document_edits::Edit::FillTarget {
                target,
                source: crate::fill::Source::Active,
                point: [1, 1],
                color: [17, 83, 201],
                opacity: 1.0,
                tolerance: 0,
                options: crate::fill::Options::default(),
            })
            .unwrap();
        let ticket = queue.snapshot().unwrap();
        let mut snapshot = None;
        while queue.busy() {
            if queue.waiting_edit() {
                queue.wait_edit();
            }
            let progress = queue.advance(&mut doc, &mut history, 4096).unwrap();
            if let Some(value) = progress.snapshot {
                snapshot = Some(value);
            }
        }
        assert_eq!(snapshot.unwrap().0, ticket);
        for (index, &alpha) in coverage.iter().enumerate() {
            assert_eq!(
                doc.layers[0].pixels.rgba(index),
                if alpha > 0 {
                    [17, 83, 201, alpha]
                } else {
                    [0; 4]
                }
            );
        }
        assert!(history.undo(&mut doc));
        assert!(doc.layers[0].pixels.iter().all(|v| *v == 0));
        assert_eq!(doc.selection_mask, Some(mask.clone()));
        assert!(history.undo(&mut doc));
        assert!(doc.selection_mask.is_none() && doc.selection.is_none());
        assert!(history.redo(&mut doc));
        assert_eq!(
            doc.selection_mask.as_ref().unwrap().allocation(),
            mask.allocation()
        );
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers[0].pixels.rgba(10), [17, 83, 201, 128]);
    }
    #[test]
    fn buffered_paint_caps_feather_coverage_and_empty_mask_prevents_all_paint() {
        let mut doc = selected_doc(8, 8, vec![128; 64]);
        let brush = crate::brush::Brush {
            size: 20.0,
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            ..Default::default()
        };
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for _ in 0..10 {
            doc.dab_buffered(
                4.0,
                4.0,
                1.0,
                &brush,
                [17, 83, 201],
                false,
                [1.0, 0.0, 0.0, 1.0],
                Some(&mut buffer),
            );
        }
        assert_eq!(doc.layers[0].pixels.rgba(4 * 8 + 4), [17, 83, 201, 128]);
        let mut empty = selected_doc(8, 8, vec![0; 64]);
        assert!(empty.validate().is_ok());
        empty.dab(4.0, 4.0, 1.0, &brush, [17, 83, 201], false);
        assert!(!empty.fill(4, 4, [17, 83, 201], 1.0, 0).unwrap());
        assert!(!empty.clear_selected().unwrap());
        assert!(empty.layers[0].pixels.iter().all(|v| *v == 0));
    }
    #[test]
    fn masked_clear_preserves_holes_hidden_rgb_and_whole_layer_override() {
        let mut doc = selected_doc(4, 1, vec![255, 128, 0, 255]);
        doc.layers[0].pixels = [9, 11, 13, 255].repeat(4).into();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(3)
            .copy_from_slice(&[7, 5, 3, 0]);
        let mask = doc.selection_mask.clone();
        assert!(doc.clear_selected().unwrap());
        assert_eq!(doc.layers[0].pixels.rgba(0), [0; 4]);
        assert_eq!(doc.layers[0].pixels.rgba(1), [9, 11, 13, 127]);
        assert_eq!(doc.layers[0].pixels.rgba(2), [9, 11, 13, 255]);
        assert_eq!(doc.layers[0].pixels.rgba(3), [7, 5, 3, 0]);
        let (whole, _) = crate::document_edits::Edit::Clear { whole_layer: true }
            .prepare(&doc)
            .unwrap();
        assert_eq!(whole.selection_mask, mask);
        assert_eq!(whole.layers[0].pixels.rgba(2), [0; 4]);
        assert_eq!(whole.layers[0].pixels.rgba(3), [7, 5, 3, 0]);
    }
    #[test]
    fn snapshot_registry_deduplicates_shared_mask_payload_and_source_guard_detects_changes() {
        let doc = selected_doc(17, 19, vec![128; 17 * 19]);
        let frozen = crate::snapshot_memory::Snapshot::capture(&doc);
        let mut plain = doc.clone();
        plain.selection_mask = None;
        plain.selection = None;
        let baseline = crate::snapshot_memory::Snapshot::capture(&plain);
        let mut actual = crate::snapshot_memory::Registry::new(&plain);
        let mut expected = crate::snapshot_memory::Registry::new(&plain);
        for _ in 0..2 {
            actual.add_snapshot(&frozen);
            expected.add_snapshot(&baseline);
        }
        assert_eq!(
            actual.additional_bytes() - expected.additional_bytes(),
            17 * 19
        );
        let changed = selected_doc(17, 19, vec![129; 17 * 19]);
        assert_eq!(changed.selection, doc.selection);
        assert!(!crate::document::same_snapshot(&doc, &changed));
        let mismatched = crate::document_edits::Edit::SelectionMask(Some(
            Mask::from_coverage(1, 1, vec![255]).unwrap(),
        ));
        assert!(mismatched.prepare(&doc).is_err());
    }
    #[test]
    fn polygons_match_independent_pixel_center_parity_in_both_winding_directions() {
        for points in [
            vec![[1.5, 1.5], [9.5, 1.5], [9.5, 7.5], [1.5, 7.5]],
            vec![[-4.0, -2.0], [14.0, 3.0], [3.0, 13.0]],
            vec![[1.0, 1.0], [10.0, 8.0], [1.0, 8.0], [10.0, 1.0]],
            vec![
                [1.0, 1.0],
                [10.0, 1.0],
                [10.0, 3.0],
                [4.0, 3.0],
                [4.0, 8.0],
                [1.0, 8.0],
            ],
        ] {
            for reversed in [false, true] {
                let mut polygon = points.clone();
                if reversed {
                    polygon.reverse();
                }
                let mask = Mask::polygon(13, 11, &polygon, None).unwrap();
                for y in 0..11 {
                    for x in 0..13 {
                        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                        let mut inside = false;
                        for i in 0..polygon.len() {
                            let a = polygon[i];
                            let b = polygon[(i + 1) % polygon.len()];
                            if (a[1] > py) != (b[1] > py)
                                && px < a[0] + (py - a[1]) * (b[0] - a[0]) / (b[1] - a[1])
                            {
                                inside = !inside;
                            }
                        }
                        assert_eq!(
                            mask.at(x, y),
                            if inside { 255 } else { 0 },
                            "({x},{y}) {polygon:?}"
                        );
                    }
                }
                assert_eq!(mask.at(13, 0), 0);
            }
        }
    }
    #[test]
    fn feather_matches_slow_separable_oracle_and_keeps_input_immutable() {
        for (width, height) in [(1usize, 1usize), (2, 7), (17, 19)] {
            let bytes: Vec<u8> = (0..width * height)
                .map(|i| ((i * 37 + i / width * 13) % 256) as u8)
                .collect();
            let input = Mask::from_coverage(width, height, bytes.clone()).unwrap();
            for radius in [0, 1, 3, 64] {
                let actual = input.feather(radius, None).unwrap();
                let mut source = bytes.clone();
                for vertical in [false, true] {
                    let mut output = vec![0; source.len()];
                    for y in 0..height {
                        for x in 0..width {
                            let (p, length) = if vertical { (y, height) } else { (x, width) };
                            let span = p.saturating_sub(radius)..(p + radius + 1).min(length);
                            let count = span.len();
                            let sum: usize = span
                                .map(|n| {
                                    usize::from(
                                        source[if vertical {
                                            n * width + x
                                        } else {
                                            y * width + n
                                        }],
                                    )
                                })
                                .sum();
                            output[y * width + x] = ((sum + count / 2) / count) as u8;
                        }
                    }
                    source = output;
                }
                assert_eq!(*actual.coverage, source);
                assert_eq!(*input.coverage, bytes);
                if radius == 0 {
                    assert_eq!(actual.allocation(), input.allocation());
                }
            }
        }
    }
    #[test]
    fn fractional_combinations_preserve_inputs_dimensions_and_shared_replacement() {
        let a = Mask::from_coverage(2, 2, vec![0, 64, 128, 255]).unwrap();
        let b = Mask::from_coverage(2, 2, vec![255, 128, 64, 0]).unwrap();
        for (mode, expected) in [
            (Combine::Add, [255, 128, 128, 255]),
            (Combine::Subtract, [0, 0, 64, 255]),
            (Combine::Intersect, [0, 64, 64, 0]),
            (Combine::Replace, [255, 128, 64, 0]),
        ] {
            let result = a.combine(&b, mode, None).unwrap();
            assert_eq!(result.coverage.as_slice(), expected);
            if matches!(mode, Combine::Replace) {
                assert_eq!(result.allocation(), b.allocation());
            }
        }
        assert_eq!(a.coverage.as_slice(), [0, 64, 128, 255]);
        let other = Mask::from_coverage(4, 1, vec![255; 4]).unwrap();
        assert!(a.combine(&other, Combine::Replace, None).is_err());
        let empty = a.combine(&a, Combine::Subtract, None).unwrap();
        assert!(empty.bounds().is_none());
        let binary = Mask::rectangle(
            2,
            2,
            Selection {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            },
        )
        .unwrap();
        assert!(
            binary
                .combine(&binary, Combine::Subtract, None)
                .unwrap()
                .bounds()
                .is_none()
        );
    }
    #[test]
    fn bounds_invalid_paths_and_canceled_operations_are_explicit() {
        let mask = Mask::rectangle(
            17,
            19,
            Selection {
                left: 3,
                top: 5,
                right: 12,
                bottom: 16,
            },
        )
        .unwrap();
        assert_eq!(
            mask.bounds(),
            Some(Selection {
                left: 3,
                top: 5,
                right: 12,
                bottom: 16
            })
        );
        assert!(Mask::from_coverage(usize::MAX, 2, Vec::new()).is_err());
        assert!(Mask::from_coverage(2, 2, vec![0; 3]).is_err());
        assert!(Mask::polygon(17, 19, &[[0.0, 0.0], [1.0, 1.0]], None).is_err());
        assert!(Mask::polygon(17, 19, &[[0.0, 0.0], [1.0, f64::NAN], [2.0, 1.0]], None).is_err());
        assert!(Mask::polygon(17, 19, &vec![[0.0, 0.0]; 16_385], None).is_err());
        assert!(mask.feather(65, None).is_err());
        let cancel = AtomicBool::new(true);
        assert!(mask.feather(0, Some(&cancel)).is_err());
        assert!(
            mask.combine(&mask, Combine::Replace, Some(&cancel))
                .is_err()
        );
        assert!(
            Mask::polygon(17, 19, &[[0.0, 0.0], [1.0, 1.0], [2.0, 1.0]], Some(&cancel)).is_err()
        );
    }
}
