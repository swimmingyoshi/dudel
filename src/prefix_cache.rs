//! Bounded compositing state below the active layer, retaining clipping/folder state.
use crate::document::{CompositeState, Document};
const SIDE: usize = 16;
const LIMIT: usize = 128;
pub(crate) const BUDGET_MIB: usize = 8;
struct Source {
    metadata: (
        bool,
        u32,
        crate::blend::BlendMode,
        bool,
        bool,
        u8,
        bool,
        usize,
    ),
    pixels: crate::pixels::RegionStamp,
    mask: crate::layer_mask::LayerStamp,
}
fn metadata(
    layer: &crate::document::Layer,
    rect: [usize; 4],
) -> (
    bool,
    u32,
    crate::blend::BlendMode,
    bool,
    bool,
    u8,
    bool,
    usize,
) {
    (
        layer.visible,
        layer.opacity.to_bits(),
        layer.blend,
        layer.clipped,
        layer.folder,
        layer.depth,
        layer.csp_byte_opacity,
        layer.display_region_key(rect),
    )
}
struct Entry {
    used: u64,
    rect: [usize; 4],
    active: usize,
    dimensions: [usize; 2],
    sources: Vec<Source>,
    states: Vec<CompositeState>,
}
impl Entry {
    fn heap_bytes(&self) -> usize {
        self.sources.capacity() * std::mem::size_of::<Source>()
            + self.states.capacity() * std::mem::size_of::<CompositeState>()
            + self
                .sources
                .iter()
                .map(|source| source.pixels.heap_bytes() + source.mask.heap_bytes())
                .sum::<usize>()
    }
}
#[derive(Default, Clone, Copy, serde::Serialize)]
pub(crate) struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub bypasses: u64,
    pub evictions: u64,
}
#[derive(Default)]
pub(crate) struct PrefixCache {
    index: std::collections::HashMap<[usize; 4], usize>,
    clock: u64,
    pub stats: Stats,
    entries: Vec<Entry>,
    payload_bytes: usize,
}
impl PrefixCache {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Entry>()
            + self.payload_bytes
            + self.index.capacity() * (std::mem::size_of::<([usize; 4], usize)>() + 1)
    }
    fn remove(&mut self, index: usize) -> Entry {
        let removed = self.entries.swap_remove(index);
        self.index.remove(&removed.rect);
        if let Some(moved) = self.entries.get(index) {
            self.index.insert(moved.rect, index);
        }
        self.payload_bytes -= removed.heap_bytes();
        removed
    }
    pub(crate) fn composite_region(
        &mut self,
        doc: &Document,
        rect: [usize; 4],
        transparent: bool,
    ) -> Vec<u8> {
        let [left, top, right, bottom] = [
            rect[0].min(doc.width),
            rect[1].min(doc.height),
            rect[2].min(doc.width),
            rect[3].min(doc.height),
        ];
        if left >= right || top >= bottom {
            return Vec::new();
        }
        let columns = right.div_ceil(SIDE) - left / SIDE;
        let rows = bottom.div_ceil(SIDE) - top / SIDE;
        let cached_width = (right.div_ceil(SIDE) * SIDE).min(doc.width) - left / SIDE * SIDE;
        let cached_height = (bottom.div_ceil(SIDE) * SIDE).min(doc.height) - top / SIDE * SIDE;
        // A request larger than the cache would evict its own freshly built
        // states. Compose it directly instead of cycling through the LRU.
        let minimum_bytes = cached_width * cached_height * std::mem::size_of::<CompositeState>()
            + columns
                * rows
                * (std::mem::size_of::<Entry>() + doc.active * std::mem::size_of::<Source>());
        if doc.active == 0 || columns * rows > LIMIT || minimum_bytes > BUDGET_MIB * 1024 * 1024 {
            self.stats.bypasses += 1;
            return doc.composite_display_region([left, top, right, bottom], transparent);
        }
        let mut out = vec![0; (right - left) * (bottom - top) * 4];
        for ty in (top / SIDE)..=(bottom - 1) / SIDE {
            for tx in (left / SIDE)..=(right - 1) / SIDE {
                let bounds = [
                    tx * SIDE,
                    ty * SIDE,
                    ((tx + 1) * SIDE).min(doc.width),
                    ((ty + 1) * SIDE).min(doc.height),
                ];
                self.clock = self.clock.saturating_add(1);
                let found = self.index.get(&bounds).copied().filter(|&index| {
                    let entry = &self.entries[index];
                    entry.rect == bounds
                        && entry.dimensions == [doc.width, doc.height]
                        && entry.active == doc.active
                        && entry.sources.iter().zip(&doc.layers[..doc.active]).all(
                            |(old, layer)| {
                                old.metadata == metadata(layer, bounds)
                                    && old.pixels.matches(&layer.pixels)
                                    && old.mask.matches(layer.mask.as_ref())
                            },
                        )
                });
                let mut entry = if let Some(index) = found {
                    self.stats.hits += 1;
                    self.remove(index)
                } else {
                    self.stats.misses += 1;
                    if let Some(index) = self.index.get(&bounds).copied() {
                        self.remove(index);
                    }
                    let sources = doc.layers[..doc.active]
                        .iter()
                        .map(|layer| Source {
                            metadata: metadata(layer, bounds),
                            mask: crate::layer_mask::LayerStamp::new(layer.mask.as_ref(), bounds),
                            pixels: layer.pixels.region_stamp(if layer.folder {
                                [0; 4]
                            } else {
                                bounds
                            }),
                        })
                        .collect();
                    let states = doc.prefix_states(bounds);
                    Entry {
                        used: self.clock,
                        rect: bounds,
                        active: doc.active,
                        dimensions: [doc.width, doc.height],
                        sources,
                        states,
                    }
                };
                entry.used = self.clock;
                for y in top.max(bounds[1])..bottom.min(bounds[3]) {
                    for x in left.max(bounds[0])..right.min(bounds[2]) {
                        let state =
                            entry.states[(y - bounds[1]) * (bounds[2] - bounds[0]) + x - bounds[0]];
                        let pixel =
                            doc.composite_from_prefix(y * doc.width + x, state, transparent);
                        let offset = ((y - top) * (right - left) + x - left) * 4;
                        out[offset..offset + 4].copy_from_slice(&pixel);
                    }
                }
                if self.entries.capacity() == 0 {
                    self.entries.reserve_exact(LIMIT);
                    self.index.reserve(LIMIT);
                }
                let bytes = entry.heap_bytes();
                while !self.entries.is_empty()
                    && (self.entries.len() >= LIMIT
                        || self.retained_bytes().saturating_add(bytes) > BUDGET_MIB * 1024 * 1024)
                {
                    let oldest = self
                        .entries
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, entry)| entry.used)
                        .unwrap()
                        .0;
                    self.remove(oldest);
                    self.stats.evictions += 1;
                }
                if self.retained_bytes().saturating_add(bytes) <= BUDGET_MIB * 1024 * 1024 {
                    self.payload_bytes += bytes;
                    self.index.insert(bounds, self.entries.len());
                    self.entries.push(entry);
                }
            }
        }
        out
    }
}

pub(crate) fn benchmark_plan(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for layers in [8usize, 32] {
        for dense in [false, true] {
            let mut doc = Document::new(96, 96);
            for index in 0..layers {
                if index > 0 {
                    doc.add_layer();
                }
                if dense || index == 0 || index == layers - 1 {
                    doc.layers[index].pixels =
                        [17 + index as u8, 83, 201, 128].repeat(96 * 96).into();
                }
            }
            for side in [16usize, 80] {
                let rect = [0, 0, side, side];
                let expected = doc.composite_region(0, 0, side, side);
                let mut samples = [Vec::<f64>::new(), Vec::<f64>::new()];
                for repetition in 0..30 {
                    for optimized in if repetition % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        let started = std::time::Instant::now();
                        let states = if optimized {
                            doc.prefix_states(rect)
                        } else {
                            (0..side * side)
                                .map(|offset| doc.prefix_state(offset / side * 96 + offset % side))
                                .collect()
                        };
                        samples[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1000.0);
                        for (offset, state) in states.into_iter().enumerate() {
                            let rgba = doc.composite_from_prefix(
                                offset / side * 96 + offset % side,
                                state,
                                false,
                            );
                            if rgba != expected[offset * 4..offset * 4 + 4] {
                                return Err("Prefix-plan benchmark pixels mismatch".into());
                            }
                        }
                    }
                }
                let summaries: Vec<_> = samples
                    .into_iter()
                    .map(|mut times| {
                        times.sort_by(f64::total_cmp);
                        serde_json::json!({"median_ms":(times[14]+times[15])/2.0,
                        "p95_ms":times[28],"max_ms":times[29],"samples_ms":times})
                    })
                    .collect();
                cases.push(
                    serde_json::json!({"layers":layers,"dense_lower_layers":dense,
                    "region_side":side,"baseline":summaries[0],"optimized":summaries[1]}),
                );
            }
        }
    }
    std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"version":1,
        "scope":"8/32-layer 96-square flat documents; either dense lower layers or a dense base plus empty sparse intermediates. 16/80-square prefix construction only, 30 alternating baseline/optimized pairs in one process. Final pixels verified against the unchanged compositor outside timing. Construction includes allocation, plan building and plan drop; state retention/validation/drop, setup, cache lookup, canvas upload, GUI/GPU, input and total memory excluded. Dense plans fall back to the original scalar construction. These descriptive samples are not an end-to-end latency or statistical speedup claim.","cases":cases}))
        .map_err(|error|error.to_string())?).map_err(|error|error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_display_paths_agree_with_transparent_export() {
        let mut doc = Document::new(70, 50);
        doc.layers[0].pixels = (0..70 * 50)
            .flat_map(|i| [(i % 251) as u8, 90, 200, (i * 7 % 256) as u8])
            .collect::<Vec<u8>>()
            .into();
        for i in 0..4 {
            doc.add_layer();
            doc.layers[doc.active].opacity = 0.5 + i as f32 * 0.1;
            let mut writer = doc.layers[doc.active].pixels.writer();
            for y in 10 + i..30 + i {
                for x in 5 + i * 9..40 + i * 9 {
                    writer
                        .rgba_mut(y * 70 + x)
                        .copy_from_slice(&[40 * i as u8, 120, 30, 150]);
                }
            }
        }
        let export = doc.composite_transparent();
        let rect = [0, 0, 70, 50];
        assert_eq!(doc.composite_display_region(rect, true), export);
        // Active layer 4 makes the cached path use prefix states.
        assert!(doc.active >= 4);
        assert_eq!(
            PrefixCache::default().composite_region(&doc, rect, true),
            export
        );
        let mut cursor = crate::composite_cursor::CompositeCursor::new(&doc, rect, true);
        while !cursor.step(997) {}
        assert_eq!(cursor.finish(), export);
        // Paper mode is unchanged.
        assert_eq!(
            PrefixCache::default().composite_region(&doc, rect, false),
            doc.composite()
        );
    }
    #[test]
    fn mask_only_edits_invalidate_prefix_pixels_but_other_tiles_keep_their_states() {
        let mut doc = Document::new(128, 128);
        doc.layers[0].pixels = [201, 17, 83, 255].repeat(128 * 128).into();
        doc.version = 2;
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(128, 128, 255).unwrap(),
        });
        doc.add_layer();
        let mut cache = PrefixCache::default();
        let rect = [0, 0, 16, 16];
        assert_eq!(
            cache.composite_region(&doc, rect, false),
            [201, 17, 83, 255].repeat(16 * 16)
        );
        let states = cache.entries[0].states.as_ptr();
        doc.layers[0]
            .mask
            .as_mut()
            .unwrap()
            .coverage
            .writer()
            .set(80, 80, 0);
        assert_eq!(
            cache.composite_region(&doc, rect, false),
            [201, 17, 83, 255].repeat(16 * 16)
        );
        assert_eq!(cache.entries[0].states.as_ptr(), states);
        doc.layers[0]
            .mask
            .as_mut()
            .unwrap()
            .coverage
            .writer()
            .set(0, 0, 0);
        let result = cache.composite_region(&doc, rect, false);
        assert_eq!(&result[..4], &[255; 4]);
        assert_eq!(result, doc.composite_region(0, 0, 16, 16));
        doc.layers[0].mask.as_mut().unwrap().enabled = false;
        assert_eq!(
            cache.composite_region(&doc, rect, false),
            [201, 17, 83, 255].repeat(16 * 16)
        );
    }
    #[test]
    fn oversized_requests_preserve_warm_cache_and_match_composite_pixels() {
        let mut doc = Document::new(512, 512);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(512 * 512).into();
        for _ in 0..7 {
            doc.add_layer();
        }
        doc.layers[doc.active].pixels.writer().write_region(
            512,
            [240, 240, 496, 496],
            &[201, 17, 83, 127].repeat(256 * 256),
        );
        let mut cache = PrefixCache::default();
        assert_eq!(
            cache.composite_region(&doc, [10, 10, 12, 12], false),
            doc.composite_region(10, 10, 12, 12)
        );
        let bytes = cache.retained_bytes();
        assert!(bytes > 0);
        for rect in [[240, 240, 496, 496], [0, 0, 128, 256]] {
            assert_eq!(
                cache.composite_region(&doc, rect, false),
                doc.composite_region(rect[0], rect[1], rect[2], rect[3])
            );
            assert_eq!(cache.retained_bytes(), bytes);
            assert_eq!(cache.entries.len(), 1);
        }
    }

    #[test]
    fn cached_prefix_preserves_open_groups_and_invalidates_lower_edits_and_active_changes() {
        let mut doc = Document::new(257, 65);
        doc.add_layer();
        for index in 0..2 {
            doc.layers[index].depth = 1;
            doc.layers[index]
                .pixels
                .writer()
                .rgba_mut(63 * 257 + 64)
                .copy_from_slice(&[30 + index as u8 * 90, 150, 70, 180]);
        }
        doc.layers[1].clipped = true;
        doc.layers
            .push(crate::document::Layer::new_folder("Group".into()));
        doc.layers[2].opacity = 0.6;
        let mut cache = PrefixCache::default();
        let rect = [60, 60, 70, 65];
        for mode in crate::blend::BlendMode::ALL {
            doc.layers[0].blend = mode;
            doc.layers[2].blend = mode;
            for active in 0..doc.layers.len() {
                doc.active = active;
                assert_eq!(
                    cache.composite_region(&doc, rect, false),
                    doc.composite_region(60, 60, 70, 65)
                );
                assert_eq!(
                    cache.composite_region(&doc, rect, false),
                    doc.composite_region(60, 60, 70, 65)
                );
            }
        }
        doc.active = 1;
        let snapshot = doc.clone();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(63 * 257 + 64)
            .copy_from_slice(&[220, 20, 100, 80]);
        assert_eq!(
            cache.composite_region(&doc, rect, false),
            doc.composite_region(60, 60, 70, 65)
        );
        doc = snapshot;
        assert_eq!(
            cache.composite_region(&doc, rect, false),
            doc.composite_region(60, 60, 70, 65)
        );
        doc.layers[0].visible = false;
        assert_eq!(
            cache.composite_region(&doc, rect, false),
            doc.composite_region(60, 60, 70, 65)
        );
        assert!(
            cache
                .composite_region(&doc, [10, 10, 0, 0], false)
                .is_empty()
        );
    }
    #[test]
    fn prefix_cache_is_bounded_and_evicted_regions_rebuild_correctly() {
        let mut doc = Document::new(2304, 65);
        doc.add_layer();
        doc.active = 1;
        let mut cache = PrefixCache::default();
        for x in (0..2304).step_by(SIDE) {
            let rect = [x, 0, (x + 2).min(2304), 2];
            assert_eq!(
                cache.composite_region(&doc, rect, false),
                doc.composite_region(rect[0], rect[1], rect[2], rect[3])
            );
            assert!(cache.entries.len() <= LIMIT);
            assert!(cache.retained_bytes() <= BUDGET_MIB * 1024 * 1024);
        }
        assert!(
            cache.entries.len() < LIMIT,
            "The byte budget must evict before the entry-count limit"
        );
        assert!(
            !cache
                .entries
                .iter()
                .any(|entry| entry.rect == [0, 0, 16, 16])
        );
        assert_eq!(
            cache.composite_region(&doc, [0, 0, 2, 2], false),
            doc.composite_region(0, 0, 2, 2)
        );
        assert!(cache.retained_bytes() <= BUDGET_MIB * 1024 * 1024);
    }
}
