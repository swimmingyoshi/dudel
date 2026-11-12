//! Single-channel, sparse copy-on-write coverage for persistent layer masks.
//! This engine is deliberately independent of selection bounds and raster RGB.
//! Document, storage, rendering and captured editing targets preserve coverage.
use std::{
    io::{Read, Write},
    sync::{Arc, Weak, atomic::AtomicBool},
};

const SIDE: usize = 64;
const MAGIC: &[u8; 5] = b"LMAS\x01";
type Tile = Arc<Vec<u8>>;

#[derive(Clone)]
struct Data {
    dimensions: [usize; 2],
    default: u8,
    tiles: Vec<Option<Tile>>,
}
#[derive(Clone)]
pub struct Mask(Arc<Data>);
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LayerMask {
    #[serde(skip, default = "crate::document::LayerId::new")]
    pub id: crate::document::LayerId,
    pub enabled: bool,
    pub linked: bool,
    #[serde(default)]
    pub coverage: Mask,
}
pub(crate) fn same_snapshot(a: &Option<LayerMask>, b: &Option<LayerMask>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.id == b.id
                && a.enabled == b.enabled
                && a.linked == b.linked
                && a.coverage.shares(&b.coverage)
        }
        _ => false,
    }
}
pub(crate) fn equivalent_state(
    a: &Option<LayerMask>,
    b: &Option<LayerMask>,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    match (a, b) {
        (None, None) => Ok(true),
        (Some(a), Some(b)) if a.id == b.id && a.enabled == b.enabled && a.linked == b.linked => {
            a.coverage.equivalent(&b.coverage, cancel)
        }
        _ => Ok(false),
    }
}
/// Linking applies to whole-layer raster transforms. An explicit selection edits
/// selected raster pixels behind the stationary layer mask; mask-target editing
/// is a separate captured target. Disabled linked masks still follow the layer.
pub(crate) fn for_raster_transform(
    doc: &crate::document::Document,
    cancel: Option<&AtomicBool>,
    apply: impl FnOnce(&Mask) -> Result<Mask, String>,
) -> Result<Option<LayerMask>, String> {
    let mut result = doc.layers[doc.active].mask.clone();
    if doc.selection.is_none()
        && doc.selection_mask.is_none()
        && let Some(mask) = &mut result
        && mask.linked
    {
        let coverage = apply(&mask.coverage)?;
        if !mask.coverage.equivalent(&coverage, cancel)? {
            mask.coverage = coverage;
        }
    }
    Ok(result)
}
/// Empty geometry is a private metadata placeholder; Document::validate rejects
/// it until the versioned container has supplied the bounded coverage payload.
impl Default for Mask {
    fn default() -> Self {
        Self(Arc::new(Data {
            dimensions: [0, 0],
            default: 255,
            tiles: Vec::new(),
        }))
    }
}
impl serde::Serialize for Mask {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::new();
        self.encode(&mut bytes, None)
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_bytes(&bytes)
    }
}
impl<'de> serde::Deserialize<'de> for Mask {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bytes;
        impl<'de> serde::de::Visitor<'de> for Bytes {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bounded layer mask subrecord bytes")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Vec<u8>, A::Error> {
                const LIMIT: usize = crate::document::MAX_PIXELS + 1024 * 4 + 18;
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(LIMIT));
                while let Some(byte) = seq.next_element::<u8>()? {
                    if bytes.len() == LIMIT {
                        return Err(serde::de::Error::custom(
                            "Layer mask record exceeds supported area",
                        ));
                    }
                    bytes.push(byte);
                }
                Ok(bytes)
            }
        }
        let bytes = deserializer.deserialize_seq(Bytes)?;
        if bytes.len() < 13 {
            return Err(serde::de::Error::custom("Truncated layer mask header"));
        }
        let dimensions = [
            u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize,
            u32::from_le_bytes(bytes[9..13].try_into().unwrap()) as usize,
        ];
        let mut input = bytes.as_slice();
        let mask = Self::decode(&mut input, dimensions, crate::document::MAX_PIXELS, None)
            .map_err(serde::de::Error::custom)?;
        if !input.is_empty() {
            return Err(serde::de::Error::custom(
                "Unexpected layer mask trailing bytes",
            ));
        }
        Ok(mask)
    }
}
pub(crate) struct LayerStamp(Option<(bool, bool, Stamp)>);
impl LayerStamp {
    pub(crate) fn new(mask: Option<&LayerMask>, rect: [usize; 4]) -> Self {
        Self(mask.map(|m| (m.enabled, m.linked, m.coverage.stamp(rect))))
    }
    pub(crate) fn matches(&self, mask: Option<&LayerMask>) -> bool {
        match (&self.0, mask) {
            (None, None) => true,
            (Some((enabled, linked, stamp)), Some(mask)) => {
                *enabled == mask.enabled && *linked == mask.linked && stamp.matches(&mask.coverage)
            }
            _ => false,
        }
    }
    pub(crate) fn heap_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |(_, _, s)| {
            s.tiles.capacity() * std::mem::size_of::<(usize, Option<Weak<Vec<u8>>>)>()
        })
    }
}
pub(crate) struct Writer<'a>(&'a mut Mask);
/// Weak tile identities detect edits without retaining any mask byte payload.
pub(crate) struct Stamp {
    dimensions: [usize; 2],
    default: u8,
    tiles: Vec<(usize, Option<Weak<Vec<u8>>>)>,
}
fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), String> {
    crate::fill::check_cancel(cancel)
}
impl Data {
    fn columns(&self) -> usize {
        self.dimensions[0].div_ceil(SIDE)
    }
    fn rect(&self, index: usize) -> [usize; 4] {
        let [w, h] = self.dimensions;
        let x = index % self.columns() * SIDE;
        let y = index / self.columns() * SIDE;
        [x, y, (x + SIDE).min(w), (y + SIDE).min(h)]
    }
    #[inline]
    fn location(&self, x: usize, y: usize) -> (usize, usize) {
        assert!(x < self.dimensions[0] && y < self.dimensions[1]);
        let column = x / SIDE;
        let tile = y / SIDE * self.columns() + column;
        let tile_width = (self.dimensions[0] - column * SIDE).min(SIDE);
        (tile, (y % SIDE) * tile_width + x % SIDE)
    }
}
impl Mask {
    pub(crate) fn equivalent(
        &self,
        other: &Self,
        cancel: Option<&AtomicBool>,
    ) -> Result<bool, String> {
        check_cancel(cancel)?;
        if self.shares(other) {
            return Ok(true);
        }
        if self.dimensions() != other.dimensions() {
            return Ok(false);
        }
        for (index, (a, b)) in self.0.tiles.iter().zip(&other.0.tiles).enumerate() {
            check_cancel(cancel)?;
            match (a, b) {
                (None, None) if self.0.default == other.0.default => {}
                (Some(a), Some(b)) if a == b => {}
                _ => {
                    let [l, t, r, b] = self.0.rect(index);
                    for y in t..b {
                        for x in l..r {
                            if self.at(x, y) != other.at(x, y) {
                                return Ok(false);
                            }
                        }
                    }
                }
            }
        }
        Ok(true)
    }
    pub(crate) fn new(width: usize, height: usize, default: u8) -> Result<Self, String> {
        crate::selection_mask::pixels(width, height)?;
        Ok(Self(Arc::new(Data {
            dimensions: [width, height],
            default,
            tiles: vec![None; width.div_ceil(SIDE) * height.div_ceil(SIDE)],
        })))
    }
    pub(crate) fn dimensions(&self) -> [usize; 2] {
        self.0.dimensions
    }
    /// Coverage wherever no tile was painted.
    pub(crate) fn default_value(&self) -> u8 {
        self.0.default
    }
    /// Bounds of the painted tiles; outside them every byte is the default.
    pub(crate) fn painted_bounds(&self) -> Option<[usize; 4]> {
        (0..self.0.tiles.len())
            .filter(|&index| self.0.tiles[index].is_some())
            .map(|index| self.0.rect(index))
            .reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            })
    }
    /// Rectangles of the painted tiles.
    pub(crate) fn painted_tiles(&self) -> Vec<[usize; 4]> {
        (0..self.0.tiles.len())
            .filter(|&index| self.0.tiles[index].is_some())
            .map(|index| self.0.rect(index))
            .collect()
    }
    /// Rectangles of the tiles that differ in storage from `other` (same
    /// dimensions): edits copy a tile before changing it, so shared tiles
    /// are equal.
    pub(crate) fn changed_tiles(&self, other: &Self) -> Vec<[usize; 4]> {
        if self.shares(other) {
            return Vec::new();
        }
        if self.dimensions() != other.dimensions() || self.0.default != other.0.default {
            return (0..self.0.tiles.len()).map(|i| self.0.rect(i)).collect();
        }
        self.0
            .tiles
            .iter()
            .zip(&other.0.tiles)
            .enumerate()
            .filter(|(_, (a, b))| match (a, b) {
                (None, None) => false,
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                _ => true,
            })
            .map(|(index, _)| self.0.rect(index))
            .collect()
    }
    pub(crate) fn payload_bytes(&self) -> usize {
        self.0.tiles.iter().flatten().map(|tile| tile.len()).sum()
    }
    /// Present-tile bounds for a mostly-white mask. These bound every possible
    /// non-white source byte; they do not describe masks with another default.
    /// Whether the mask hides all of `rect` (clamped to the mask): every
    /// coverage byte there is 0.
    pub(crate) fn region_is_clear(&self, rect: [usize; 4]) -> bool {
        let [w, h] = self.0.dimensions;
        let [left, top, right, bottom] = [
            rect[0].min(w),
            rect[1].min(h),
            rect[2].min(w),
            rect[3].min(h),
        ];
        if left >= right || top >= bottom {
            return true;
        }
        let columns = self.0.columns();
        for row in top / SIDE..=(bottom - 1) / SIDE {
            for column in left / SIDE..=(right - 1) / SIDE {
                let index = row * columns + column;
                let [x0, y0, x1, y1] = self.0.rect(index);
                let Some(bytes) = &self.0.tiles[index] else {
                    if self.0.default != 0 {
                        return false;
                    }
                    continue;
                };
                let (from, to) = (left.max(x0) - x0, right.min(x1) - x0);
                for y in top.max(y0)..bottom.min(y1) {
                    let start = (y - y0) * (x1 - x0);
                    if bytes[start + from..start + to].iter().any(|v| *v != 0) {
                        return false;
                    }
                }
            }
        }
        true
    }
    pub(crate) fn white_sparse_regions(&self) -> Option<Vec<[usize; 4]>> {
        let [w, h] = self.dimensions();
        if self.0.default != 255 || self.payload_bytes() > w * h / 4 {
            return None;
        }
        Some(
            self.0
                .tiles
                .iter()
                .enumerate()
                .filter_map(|(i, t)| t.as_ref().map(|_| self.0.rect(i)))
                .collect(),
        )
    }
    fn uniform_value(&self, cancel: Option<&AtomicBool>) -> Result<Option<u8>, String> {
        let value = self.at(0, 0);
        for tile in &self.0.tiles {
            check_cancel(cancel)?;
            if tile.as_ref().map_or(self.0.default != value, |bytes| {
                bytes.iter().any(|v| *v != value)
            }) {
                return Ok(None);
            }
        }
        Ok(Some(value))
    }
    /// Build complete unpublished tiles, retaining the original default and
    /// identical tile allocations. This avoids per-pixel COW and preserves both
    /// renderer stamps and packed payload limits for unchanged/default regions.
    fn generate_tiles(
        &self,
        cancel: Option<&AtomicBool>,
        workers: usize,
        fill: impl Fn([usize; 4], &mut [u8]) -> Result<(), String> + Sync,
    ) -> Result<Self, String> {
        check_cancel(cancel)?;
        let make = |start: usize, end: usize| {
            let mut tiles = Vec::with_capacity(end - start);
            for index in start..end {
                check_cancel(cancel)?;
                let rect = self.0.rect(index);
                let mut bytes = vec![self.0.default; (rect[2] - rect[0]) * (rect[3] - rect[1])];
                fill(rect, &mut bytes)?;
                tiles.push(
                    if let Some(old) = self.0.tiles[index]
                        .as_ref()
                        .filter(|old| old.as_slice() == bytes.as_slice())
                    {
                        Some(old.clone())
                    } else if bytes.iter().all(|v| *v == self.0.default) {
                        None
                    } else {
                        Some(Arc::new(bytes))
                    },
                );
            }
            Ok::<_, String>(tiles)
        };
        let count = self.0.tiles.len();
        let parts = if workers <= 1 || count <= 1 {
            vec![(0, make(0, count)?)]
        } else {
            let chunk = count.div_ceil(workers.min(count));
            let starts: Vec<usize> = (0..count).step_by(chunk).collect();
            // Every band has finished, even after cancellation or an error.
            let outputs = crate::parallel::run(starts.len(), |index| {
                make(starts[index], (starts[index] + chunk).min(count))
            });
            starts
                .into_iter()
                .zip(outputs)
                .map(|(start, tiles)| {
                    tiles
                        .unwrap_or_else(|_| Err("Mask sampling worker failed".into()))
                        .map(|tiles| (start, tiles))
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        check_cancel(cancel)?;
        let mut result = self.clone();
        let data = Arc::make_mut(&mut result.0);
        for (start, tiles) in parts {
            for (slot, tile) in data.tiles[start..start + tiles.len()].iter_mut().zip(tiles) {
                *slot = tile;
            }
        }
        if let Some(value) = result.uniform_value(cancel)?
            && result.payload_bytes() != 0
        {
            return Self::new(self.0.dimensions[0], self.0.dimensions[1], value);
        }
        Ok(result)
    }
    pub(crate) fn generate_shared(
        &self,
        cancel: Option<&AtomicBool>,
        sample: impl Fn(usize, usize) -> u8 + Sync,
    ) -> Result<Self, String> {
        let pixels = self.0.dimensions[0] * self.0.dimensions[1];
        let workers = if pixels >= 1024 * 1024 {
            let available = std::thread::available_parallelism().map_or(1, |n| n.get());
            pixels
                .div_ceil(512 * 1024)
                .min(available.saturating_sub(1).max(1))
                .min(4)
        } else {
            1
        };
        self.generate_tiles(cancel, workers, |[l, t, r, b], bytes| {
            for y in t..b {
                check_cancel(cancel)?;
                for x in l..r {
                    bytes[(y - t) * (r - l) + x - l] = sample(x, y);
                }
            }
            Ok(())
        })
    }
    pub(crate) fn offset(
        &self,
        [dx, dy]: [i64; 2],
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        check_cancel(cancel)?;
        if dx == 0 && dy == 0 {
            return Ok(self.clone());
        }
        let [w, h] = self.dimensions();
        if dx.unsigned_abs() >= w as u64 || dy.unsigned_abs() >= h as u64 {
            return Self::new(w, h, 255);
        }
        // Offsets are now bounded by the canvas dimensions, so row arithmetic
        // cannot overflow. Copy at most two source tile spans per target row.
        self.generate_tiles(cancel, 1, |[l, t, r, b], bytes| {
            bytes.fill(255);
            let left = l.max(dx.max(0) as usize);
            let right = r.min((w as i64 + dx).min(w as i64).max(0) as usize);
            for y in t..b {
                check_cancel(cancel)?;
                let sy = y as i64 - dy;
                if sy < 0 || sy >= h as i64 {
                    continue;
                }
                let mut x = left;
                while x < right {
                    let sx = (x as i64 - dx) as usize;
                    let (tile, offset) = self.0.location(sx, sy as usize);
                    let count = (SIDE - sx % SIDE).min(right - x);
                    let start = (y - t) * (r - l) + x - l;
                    let target = &mut bytes[start..start + count];
                    if let Some(source) = &self.0.tiles[tile] {
                        target.copy_from_slice(&source[offset..offset + count]);
                    } else {
                        target.fill(self.0.default);
                    }
                    x += count;
                }
            }
            Ok(())
        })
    }
    pub(crate) fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    #[inline]
    pub(crate) fn at(&self, x: usize, y: usize) -> u8 {
        let (tile, offset) = self.0.location(x, y);
        self.0.tiles[tile]
            .as_ref()
            .map_or(self.0.default, |bytes| bytes[offset])
    }
    /// Four adjacent, in-bounds bilinear taps. Most footprints share one tile;
    /// resolve its directory entry and row stride once instead of four times.
    #[inline]
    pub(crate) fn bilinear_taps(&self, x: usize, y: usize) -> [u8; 4] {
        debug_assert!(x + 1 < self.0.dimensions[0] && y + 1 < self.0.dimensions[1]);
        if x % SIDE != SIDE - 1 && y % SIDE != SIDE - 1 {
            let (tile, offset) = self.0.location(x, y);
            let stride = (self.0.dimensions[0] - x / SIDE * SIDE).min(SIDE);
            self.0.tiles[tile]
                .as_ref()
                .map_or([self.0.default; 4], |bytes| {
                    [
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + stride],
                        bytes[offset + stride + 1],
                    ]
                })
        } else {
            [
                self.at(x, y),
                self.at(x + 1, y),
                self.at(x, y + 1),
                self.at(x + 1, y + 1),
            ]
        }
    }
    pub(crate) fn writer(&mut self) -> Writer<'_> {
        Writer(self)
    }
    /// Bulk construction writes each tile once. Per-pixel copy-on-write is useful
    /// for live edits, but unnecessary for an unpublished worker result.
    pub(crate) fn generate(
        width: usize,
        height: usize,
        default: u8,
        cancel: Option<&AtomicBool>,
        sample: impl Fn(usize, usize) -> Result<u8, String>,
    ) -> Result<Self, String> {
        check_cancel(cancel)?;
        let mut result = Self::new(width, height, default)?;
        let data = Arc::get_mut(&mut result.0).unwrap();
        for index in 0..data.tiles.len() {
            let [left, top, right, bottom] = data.rect(index);
            let mut bytes = Vec::with_capacity((right - left) * (bottom - top));
            for y in top..bottom {
                check_cancel(cancel)?;
                for x in left..right {
                    bytes.push(sample(x, y)?);
                }
            }
            if bytes.iter().any(|byte| *byte != default) {
                data.tiles[index] = Some(Arc::new(bytes));
            }
        }
        check_cancel(cancel)?;
        Ok(result)
    }
    pub(crate) fn from_coverage(
        width: usize,
        height: usize,
        bytes: &[u8],
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        if bytes.len() != crate::selection_mask::pixels(width, height)? {
            return Err("Layer mask coverage length differs from dimensions".into());
        }
        check_cancel(cancel)?;
        let mut zero = 0;
        let mut white = 0;
        for row in bytes.chunks(width) {
            check_cancel(cancel)?;
            for value in row {
                zero += usize::from(*value == 0);
                white += usize::from(*value == 255);
            }
        }
        let default = if white >= zero { 255 } else { 0 };
        Self::generate(width, height, default, cancel, |x, y| {
            Ok(bytes[y * width + x])
        })
    }
    pub(crate) fn inverted(&self, cancel: Option<&AtomicBool>) -> Result<Self, String> {
        check_cancel(cancel)?;
        let mut result = self.clone();
        let data = Arc::make_mut(&mut result.0);
        data.default = 255 - data.default;
        for tile in data.tiles.iter_mut().flatten() {
            check_cancel(cancel)?;
            for byte in Arc::make_mut(tile) {
                *byte = 255 - *byte;
            }
        }
        check_cancel(cancel)?;
        Ok(result)
    }
    /// Destination-to-source mapping supports linked transforms and canvas bounds.
    /// Missing source coordinates use explicit coverage, never implicit selection.
    pub(crate) fn remap(
        &self,
        width: usize,
        height: usize,
        outside: u8,
        cancel: Option<&AtomicBool>,
        map: impl Fn([usize; 2]) -> Option<[usize; 2]>,
    ) -> Result<Self, String> {
        let result = Self::generate(width, height, self.0.default, cancel, |x, y| {
            match map([x, y]) {
                Some([sx, sy]) if sx < self.0.dimensions[0] && sy < self.0.dimensions[1] => {
                    Ok(self.at(sx, sy))
                }
                Some(_) => Err("Layer mask mapping is outside source dimensions".into()),
                None => Ok(outside),
            }
        })?;
        Ok(if self.equivalent(&result, cancel)? {
            self.clone()
        } else {
            result
        })
    }
    /// Bake coverage into alpha, retaining RGB even when coverage becomes zero.
    pub(crate) fn apply_rgba(&self, x: usize, y: usize, mut rgba: [u8; 4]) -> [u8; 4] {
        rgba[3] = ((u16::from(rgba[3]) * u16::from(self.at(x, y)) + 127) / 255) as u8;
        rgba
    }
    pub(crate) fn stamp(&self, rect: [usize; 4]) -> Stamp {
        let [w, h] = self.dimensions();
        let [left, top, right, bottom] = [
            rect[0].min(w),
            rect[1].min(h),
            rect[2].min(w),
            rect[3].min(h),
        ];
        let mut tiles = Vec::new();
        if left < right && top < bottom {
            for y in top / SIDE..bottom.div_ceil(SIDE) {
                for x in left / SIDE..right.div_ceil(SIDE) {
                    let i = y * self.0.columns() + x;
                    tiles.push((i, self.0.tiles[i].as_ref().map(Arc::downgrade)));
                }
            }
        }
        Stamp {
            dimensions: [w, h],
            default: self.0.default,
            tiles,
        }
    }
    /// The identity `visit_allocations` reports for the tile table itself.
    pub(crate) fn storage_allocation(&self) -> crate::pixels::AllocationId {
        crate::pixels::AllocationId::Storage(Arc::as_ptr(&self.0) as usize)
    }
    pub(crate) fn visit_allocations(
        &self,
        mut visit: impl FnMut(crate::pixels::AllocationId, usize),
    ) {
        use crate::pixels::AllocationId;
        visit(
            AllocationId::Storage(Arc::as_ptr(&self.0) as usize),
            std::mem::size_of::<Data>()
                + self.0.tiles.capacity() * std::mem::size_of::<Option<Tile>>(),
        );
        for tile in self.0.tiles.iter().flatten() {
            visit(
                AllocationId::Tile(Arc::as_ptr(tile) as usize),
                std::mem::size_of::<Vec<u8>>() + tile.capacity(),
            );
        }
    }
    /// Canonical bounded subrecord used by version-5 native project payloads.
    pub(crate) fn encode(
        &self,
        mut output: impl Write,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let io = |e: std::io::Error| e.to_string();
        output.write_all(MAGIC).map_err(io)?;
        for value in self.dimensions() {
            output
                .write_all(&(value as u32).to_le_bytes())
                .map_err(io)?;
        }
        output.write_all(&[self.0.default]).map_err(io)?;
        let count = self
            .0
            .tiles
            .iter()
            .flatten()
            .filter(|tile| tile.iter().any(|v| *v != self.0.default))
            .count();
        output
            .write_all(&(count as u32).to_le_bytes())
            .map_err(io)?;
        for (index, tile) in self.0.tiles.iter().enumerate() {
            check_cancel(cancel)?;
            if let Some(tile) = tile {
                if tile.iter().all(|v| *v == self.0.default) {
                    continue;
                }
                output
                    .write_all(&(index as u32).to_le_bytes())
                    .map_err(io)?;
                output.write_all(tile).map_err(io)?;
            }
        }
        check_cancel(cancel)
    }
    pub(crate) fn decode(
        mut input: impl Read,
        expected: [usize; 2],
        payload_budget: usize,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        check_cancel(cancel)?;
        let read = |input: &mut dyn Read, bytes: &mut [u8]| {
            input
                .read_exact(bytes)
                .map_err(|e| format!("Damaged layer mask: {e}"))
        };
        let mut tag = [0; 5];
        read(&mut input, &mut tag)?;
        if &tag != MAGIC {
            return Err("Unsupported layer mask record".into());
        }
        let number = |input: &mut dyn Read| -> Result<usize, String> {
            let mut bytes = [0; 4];
            read(input, &mut bytes)?;
            Ok(u32::from_le_bytes(bytes) as usize)
        };
        let dimensions = [number(&mut input)?, number(&mut input)?];
        if dimensions != expected {
            return Err("Layer mask dimensions differ from document".into());
        }
        let mut default = [0];
        read(&mut input, &mut default)?;
        let mut result = Self::new(dimensions[0], dimensions[1], default[0])?;
        let count = number(&mut input)?;
        if count > result.0.tiles.len() || count > payload_budget {
            return Err("Layer mask tile count exceeds budget".into());
        }
        let data = Arc::make_mut(&mut result.0);
        let mut previous = None;
        let mut payload = 0usize;
        for _ in 0..count {
            check_cancel(cancel)?;
            let index = number(&mut input)?;
            if index >= data.tiles.len() || previous.is_some_and(|old| index <= old) {
                return Err("Invalid or unordered layer mask tile".into());
            }
            previous = Some(index);
            let [l, t, r, b] = data.rect(index);
            let length = (r - l) * (b - t);
            payload = payload
                .checked_add(length)
                .ok_or("Layer mask byte count overflow")?;
            if payload > payload_budget {
                return Err("Layer mask coverage exceeds payload budget".into());
            }
            let mut bytes = vec![0; length];
            read(&mut input, &mut bytes)?;
            if bytes.iter().all(|v| *v == default[0]) {
                return Err("Redundant layer mask tile".into());
            }
            data.tiles[index] = Some(Arc::new(bytes));
        }
        check_cancel(cancel)?;
        Ok(result)
    }
}
impl Writer<'_> {
    pub(crate) fn at(&self, x: usize, y: usize) -> u8 {
        self.0.at(x, y)
    }
    pub(crate) fn reclaim_region(&mut self, rect: [usize; 4]) {
        let [w, h] = self.0.dimensions();
        let [l, t, r, b] = [
            rect[0].min(w),
            rect[1].min(h),
            rect[2].min(w),
            rect[3].min(h),
        ];
        if l >= r || t >= b {
            return;
        }
        for y in t / SIDE..b.div_ceil(SIDE) {
            for x in l / SIDE..r.div_ceil(SIDE) {
                let i = y * self.0.0.columns() + x;
                if self.0.0.tiles[i]
                    .as_ref()
                    .is_some_and(|tile| tile.iter().all(|v| *v == self.0.0.default))
                {
                    Arc::make_mut(&mut self.0.0).tiles[i] = None;
                }
            }
        }
    }
    pub(crate) fn set(&mut self, x: usize, y: usize, value: u8) -> bool {
        let (index, offset) = self.0.0.location(x, y);
        if self.0.0.tiles[index]
            .as_ref()
            .map_or(self.0.0.default, |tile| tile[offset])
            == value
        {
            return false;
        }
        let data = Arc::make_mut(&mut self.0.0);
        let [l, t, r, b] = data.rect(index);
        let tile = data.tiles[index]
            .get_or_insert_with(|| Arc::new(vec![data.default; (r - l) * (b - t)]));
        Arc::make_mut(tile)[offset] = value;
        true
    }
    /// Brush coverage interpolates toward grayscale: black conceals, white reveals.
    pub(crate) fn paint(&mut self, x: usize, y: usize, target: u8, coverage: f32) -> bool {
        if !coverage.is_finite() || coverage <= 0.0 {
            return false;
        }
        let coverage = coverage.min(1.0);
        let old = self.0.at(x, y);
        self.set(
            x,
            y,
            (f32::from(old) * (1.0 - coverage) + f32::from(target) * coverage).round() as u8,
        )
    }
    pub(crate) fn reclaim(&mut self) {
        // Avoid detaching metadata on the common case with no redundant tile.
        if self
            .0
            .0
            .tiles
            .iter()
            .flatten()
            .any(|tile| tile.iter().all(|v| *v == self.0.0.default))
        {
            let data = Arc::make_mut(&mut self.0.0);
            for tile in &mut data.tiles {
                if tile
                    .as_ref()
                    .is_some_and(|bytes| bytes.iter().all(|v| *v == data.default))
                {
                    *tile = None;
                }
            }
        }
    }
}
impl Stamp {
    pub(crate) fn matches(&self, mask: &Mask) -> bool {
        self.dimensions == mask.dimensions()
            && self.default == mask.0.default
            && self
                .tiles
                .iter()
                .all(|(i, tile)| match (tile, &mask.0.tiles[*i]) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.ptr_eq(&Arc::downgrade(b)),
                    _ => false,
                })
    }
}

pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512, 2048] {
        let source = Mask::new(size, size, 255)?;
        let stamp = source.stamp([0, 0, 64, 64]);
        let start = std::time::Instant::now();
        let mut edited = source.clone();
        {
            let mut writer = edited.writer();
            for y in 80..144 {
                for x in 80..144 {
                    writer.paint(x, y, 0, 0.5);
                }
            }
            writer.reclaim();
        }
        let edit_ms = start.elapsed().as_secs_f64() * 1000.0;
        if source.at(80, 80) != 255
            || edited.at(80, 80) != 128
            || !stamp.matches(&edited)
            || source.shares(&edited)
        {
            return Err("Layer mask copy-on-write verification failed".into());
        }
        let mut registry = crate::snapshot_memory::Registry::default();
        source.visit_allocations(|id, n| registry.add_pixels(id, n));
        let source_bytes = registry.additional_bytes();
        edited.visit_allocations(|id, n| registry.add_pixels(id, n));
        let additional_bytes = registry.additional_bytes() - source_bytes;
        let mut encoded = Vec::new();
        edited.encode(&mut encoded, None)?;
        let decoded = Mask::decode(encoded.as_slice(), [size, size], size * size, None)?;
        let inverse = decoded.inverted(None)?;
        if decoded.apply_rgba(80, 80, [5, 7, 11, 129]) != [5, 7, 11, 65] {
            return Err("Layer mask codec/coverage verification failed".into());
        }
        for y in 0..size {
            for x in 0..size {
                let expected = if (80..144).contains(&x) && (80..144).contains(&y) {
                    128
                } else {
                    255
                };
                if edited.at(x, y) != expected
                    || decoded.at(x, y) != expected
                    || inverse.at(x, y) != 255 - expected
                    || source.at(x, y) != 255
                {
                    return Err("Layer mask sparse edit/codec/inverse oracle failed".into());
                }
            }
        }
        let coverage: Vec<u8> = (0..size * size)
            .map(|i| ((i * 37 + i / size) % 256) as u8)
            .collect();
        let start = std::time::Instant::now();
        let dense = Mask::from_coverage(size, size, &coverage, None)?;
        let dense_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = std::time::Instant::now();
        let rotated = dense.remap(size, size, 255, None, |[x, y]| Some([y, size - 1 - x]))?;
        let remap_ms = start.elapsed().as_secs_f64() * 1000.0;
        for y in 0..size {
            for x in 0..size {
                if rotated.at(x, y) != coverage[(size - 1 - x) * size + y] {
                    return Err("Layer mask remap verification failed".into());
                }
            }
        }
        cases.push(serde_json::json!({"size":size,"verified":true,"edit_64_square_ms":edit_ms,"dense_construction_ms":dense_ms,"dense_rotation_ms":remap_ms,"white_source_bytes":source_bytes,"edited_additional_bytes":additional_bytes,"sparse_record_bytes":encoded.len()}));
    }
    let report = serde_json::json!({"version":1,"scope":"Layer mask engine only; excludes document, UI, history, project containers, display and physical input","cases":cases});
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?
        .write_all(&bytes)
        .map_err(|e| e.to_string())?;
    println!("Layer mask engine verification: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_tile_generation_preserves_shared_tiles_and_joins_failed_work() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let source = Mask::generate(
            129,
            131,
            43,
            None,
            |x, y| Ok(((x * 7 + y * 11) % 251) as u8),
        )
        .unwrap();
        let stamp = source.stamp([0, 0, 64, 64]);
        for workers in [1, 4] {
            let result = source
                .generate_tiles(None, workers, |[l, t, r, b], bytes| {
                    for y in t..b {
                        for x in l..r {
                            bytes[(y - t) * (r - l) + x - l] = if x < 64 && y < 64 {
                                source.at(x, y)
                            } else {
                                ((x * 13 + y * 29) % 256) as u8
                            };
                        }
                    }
                    Ok(())
                })
                .unwrap();
            assert!(stamp.matches(&result));
            for y in 0..131 {
                for x in 0..129 {
                    assert_eq!(
                        result.at(x, y),
                        if x < 64 && y < 64 {
                            source.at(x, y)
                        } else {
                            ((x * 13 + y * 29) % 256) as u8
                        }
                    );
                }
            }
        }
        for failure in 0..3 {
            let active = AtomicUsize::new(0);
            let cancel = AtomicBool::new(false);
            let result = source.generate_tiles(Some(&cancel), 4, |[l, t, _, _], bytes| {
                active.fetch_add(1, Ordering::SeqCst);
                let _guard = Active(&active);
                bytes.fill(255);
                if l == 64 && t == 0 {
                    match failure {
                        0 => cancel.store(true, Ordering::Relaxed),
                        1 => return Err("Injected sampling error".into()),
                        _ => panic!("Injected sampling panic"),
                    }
                }
                check_cancel(Some(&cancel))
            });
            assert!(result.is_err());
            assert_eq!(active.load(Ordering::SeqCst), 0);
            assert!(stamp.matches(&source));
            for y in 0..131 {
                for x in 0..129 {
                    assert_eq!(source.at(x, y), ((x * 7 + y * 11) % 251) as u8);
                }
            }
        }
    }
    use std::sync::atomic::Ordering;
    fn bytes(mask: &Mask) -> Vec<u8> {
        let [w, h] = mask.dimensions();
        (0..w * h).map(|i| mask.at(i % w, i / w)).collect()
    }
    fn record(mask: &Mask) -> Vec<u8> {
        let mut bytes = Vec::new();
        mask.encode(&mut bytes, None).unwrap();
        bytes
    }
    #[test]
    fn constant_masks_allocate_no_tiles_and_noop_edits_retain_storage() {
        for [w, h] in [[1, 1], [1, 4096], [4096, 1], [65, 67], [2048, 2048]] {
            for default in [0, 127, 255] {
                let mut mask = Mask::new(w, h, default).unwrap();
                let source = mask.clone();
                assert!(!mask.writer().set(w - 1, h - 1, default));
                assert!(!mask.writer().paint(0, 0, default, 1.0));
                assert!(!mask.writer().paint(0, 0, 255, f32::NAN));
                mask.writer().reclaim();
                assert!(source.shares(&mask));
                assert!(mask.0.tiles.iter().all(Option::is_none));
                assert_eq!(record(&mask).len(), 18);
                let restored = Mask::decode(record(&mask).as_slice(), [w, h], 0, None).unwrap();
                assert_eq!(restored.at(w - 1, h - 1), default);
            }
        }
        assert!(Mask::new(0, 1, 255).is_err());
        assert!(Mask::new(16385, 1, 255).is_err());
        assert!(Mask::new(10001, 10000, 255).is_err());
        assert!(Mask::from_coverage(2, 2, &[0; 3], None).is_err());
    }
    #[test]
    fn painting_uses_grayscale_coverage_and_preserves_unmodified_shared_tiles() {
        let mut mask = Mask::new(131, 67, 255).unwrap();
        for (x, y, v) in [(0, 0, 0), (63, 63, 128), (64, 64, 37), (130, 66, 190)] {
            mask.writer().set(x, y, v);
        }
        let source = mask.clone();
        let before = bytes(&source);
        let tile = source.0.tiles[0].clone().unwrap();
        let untouched = mask.stamp([0, 0, 64, 64]);
        let changed = mask.stamp([64, 64, 131, 67]);
        assert!(mask.writer().paint(64, 64, 255, 0.5));
        assert_eq!(mask.at(64, 64), 146);
        assert!(untouched.matches(&mask));
        assert!(!changed.matches(&mask));
        assert!(Arc::ptr_eq(&tile, mask.0.tiles[0].as_ref().unwrap()));
        assert_eq!(bytes(&source), before);
        assert_eq!(source.at(64, 64), 37);
        mask.writer().paint(64, 64, 0, 2.0);
        assert_eq!(mask.at(64, 64), 0);
        assert!(!mask.writer().paint(64, 64, 255, -1.0));
        assert_eq!(mask.apply_rgba(64, 64, [7, 13, 29, 128]), [7, 13, 29, 0]);
        assert_eq!(mask.apply_rgba(63, 63, [7, 13, 29, 128]), [7, 13, 29, 64]);
        assert_eq!(mask.apply_rgba(63, 63, [7, 13, 29, 0]), [7, 13, 29, 0]);
    }
    #[test]
    fn full_tile_restoration_reclaims_payload_and_weak_stamps_do_not_retain_bytes() {
        let mut mask = Mask::new(65, 65, 255).unwrap();
        mask.writer().set(64, 64, 0);
        let weak = Arc::downgrade(mask.0.tiles[3].as_ref().unwrap());
        let stamp = mask.stamp([64, 64, 65, 65]);
        mask.writer().set(64, 64, 255);
        assert!(
            weak.upgrade().is_none(),
            "An edit must invalidate the weak tile identity"
        );
        assert!(!stamp.matches(&mask));
        mask.writer().reclaim();
        assert!(mask.0.tiles[3].is_none());
        assert_eq!(record(&mask).len(), 18);
        let snapshot = mask.clone();
        mask.writer().reclaim();
        assert!(snapshot.shares(&mask));
    }
    #[test]
    fn packed_edge_tiles_codec_and_inverse_match_dense_oracle() {
        for [w, h] in [[1, 65], [65, 1], [63, 67], [129, 131]] {
            let coverage: Vec<u8> = (0..w * h)
                .map(|i| ((i * 71 + i / w * 13) % 256) as u8)
                .collect();
            let mask = Mask::from_coverage(w, h, &coverage, None).unwrap();
            assert_eq!(bytes(&mask), coverage);
            let encoded = record(&mask);
            let mut input = encoded.as_slice();
            let decoded = Mask::decode(&mut input, [w, h], w * h, None).unwrap();
            assert!(input.is_empty());
            assert_eq!(bytes(&decoded), coverage);
            assert_eq!(
                record(&decoded),
                encoded,
                "Canonical order survives reopening"
            );
            assert_eq!(
                bytes(&mask.inverted(None).unwrap()),
                coverage.iter().map(|v| 255 - v).collect::<Vec<_>>()
            );
            assert_eq!(
                bytes(&mask.inverted(None).unwrap().inverted(None).unwrap()),
                coverage
            );
            for tile in mask
                .0
                .tiles
                .iter()
                .enumerate()
                .filter_map(|(i, t)| t.as_ref().map(|t| (i, t)))
            {
                let [l, t, r, b] = mask.0.rect(tile.0);
                assert_eq!(tile.1.len(), (r - l) * (b - t));
            }
        }
    }
    #[test]
    fn geometry_maps_growth_crop_rotation_and_flips_without_changing_source() {
        let (w, h) = (65, 67);
        let source: Vec<u8> = (0..w * h).map(|i| ((i * 11 + i / w) % 256) as u8).collect();
        let mask = Mask::from_coverage(w, h, &source, None).unwrap();
        let grown = mask
            .remap(w + 5, h + 7, 255, None, |[x, y]| {
                Some([x.checked_sub(2)?, y.checked_sub(3)?]).filter(|[x, y]| *x < w && *y < h)
            })
            .unwrap();
        for y in 0..h + 7 {
            for x in 0..w + 5 {
                assert_eq!(
                    grown.at(x, y),
                    if (2..w + 2).contains(&x) && (3..h + 3).contains(&y) {
                        source[(y - 3) * w + x - 2]
                    } else {
                        255
                    }
                );
            }
        }
        let cropped = grown
            .remap(w, h, 0, None, |[x, y]| Some([x + 2, y + 3]))
            .unwrap();
        assert_eq!(bytes(&cropped), source);
        let rotated = mask
            .remap(h, w, 0, None, |[x, y]| Some([y, h - 1 - x]))
            .unwrap();
        for y in 0..w {
            for x in 0..h {
                assert_eq!(rotated.at(x, y), source[(h - 1 - x) * w + y]);
            }
        }
        let flipped = mask
            .remap(w, h, 0, None, |[x, y]| Some([w - 1 - x, y]))
            .unwrap();
        let restored = flipped
            .remap(w, h, 0, None, |[x, y]| Some([w - 1 - x, y]))
            .unwrap();
        assert_eq!(bytes(&restored), source);
        assert!(mask.remap(w, h, 255, None, |_| Some([w, h])).is_err());
        assert_eq!(bytes(&mask), source);
    }
    #[test]
    fn cancellation_is_atomic_and_checked_during_mapping() {
        let mut mask = Mask::new(65, 65, 255).unwrap();
        mask.writer().set(0, 0, 0);
        let source = mask.clone();
        let cancel = AtomicBool::new(true);
        assert!(mask.inverted(Some(&cancel)).is_err());
        assert!(mask.remap(65, 65, 255, Some(&cancel), Some).is_err());
        assert!(Mask::from_coverage(65, 65, &vec![0; 65 * 65], Some(&cancel)).is_err());
        assert!(mask.encode(Vec::new(), Some(&cancel)).is_err());
        assert!(Mask::decode(record(&mask).as_slice(), [65, 65], 65 * 65, Some(&cancel)).is_err());
        cancel.store(false, Ordering::Relaxed);
        let mapped = std::cell::Cell::new(0);
        assert!(
            mask.remap(65, 65, 255, Some(&cancel), |point| {
                mapped.set(mapped.get() + 1);
                if mapped.get() == 70 {
                    cancel.store(true, Ordering::Relaxed);
                }
                Some(point)
            })
            .is_err()
        );
        assert_eq!(
            mapped.get(),
            128,
            "Cancellation is checked between packed tile rows"
        );
        assert!(mask.shares(&source));
        assert_eq!(mask.at(0, 0), 0);
    }
    #[test]
    fn decoder_rejects_malformed_and_over_budget_records_before_publication() {
        let mut mask = Mask::new(65, 65, 255).unwrap();
        mask.writer().set(0, 0, 0);
        mask.writer().set(64, 64, 0);
        let encoded = record(&mask);
        for length in [0, 4, 5, 9, 13, 17, 20, encoded.len() - 1] {
            assert!(Mask::decode(&encoded[..length], [65, 65], 65 * 65, None).is_err());
        }
        assert!(Mask::decode(encoded.as_slice(), [64, 65], 65 * 65, None).is_err());
        assert!(Mask::decode(encoded.as_slice(), [65, 65], 4096, None).is_err());
        let mut corrupted = encoded.clone();
        corrupted[0] = b'?';
        assert!(Mask::decode(corrupted.as_slice(), [65, 65], 65 * 65, None).is_err());
        let mut corrupted = encoded.clone();
        corrupted[14..18].copy_from_slice(&5u32.to_le_bytes());
        assert!(Mask::decode(corrupted.as_slice(), [65, 65], 65 * 65, None).is_err());
        let mut corrupted = encoded.clone();
        corrupted[18..22].copy_from_slice(&4u32.to_le_bytes());
        assert!(Mask::decode(corrupted.as_slice(), [65, 65], 65 * 65, None).is_err());
        let mut corrupted = encoded.clone();
        corrupted[22..22 + 4096].fill(255);
        assert!(Mask::decode(corrupted.as_slice(), [65, 65], 65 * 65, None).is_err());
        let mut corrupted = encoded.clone();
        corrupted[22 + 4096..26 + 4096].copy_from_slice(&0u32.to_le_bytes());
        assert!(Mask::decode(corrupted.as_slice(), [65, 65], 65 * 65, None).is_err());
    }
    #[test]
    fn allocation_registry_deduplicates_snapshots_and_counts_changed_tiles() {
        let mut mask = Mask::new(128, 128, 255).unwrap();
        mask.writer().set(0, 0, 0);
        mask.writer().set(64, 64, 0);
        let mut registry = crate::snapshot_memory::Registry::default();
        mask.visit_allocations(|id, n| registry.add_pixels(id, n));
        let first = registry.additional_bytes();
        let frozen = mask.clone();
        frozen.visit_allocations(|id, n| registry.add_pixels(id, n));
        assert_eq!(registry.additional_bytes(), first);
        mask.writer().set(1, 1, 128);
        mask.visit_allocations(|id, n| registry.add_pixels(id, n));
        let new_metadata = std::mem::size_of::<Data>() + 4 * std::mem::size_of::<Option<Tile>>();
        assert_eq!(
            registry.additional_bytes() - first,
            new_metadata + std::mem::size_of::<Vec<u8>>() + SIDE * SIDE
        );
        assert_eq!(frozen.at(1, 1), 255);
        assert_eq!(mask.at(1, 1), 128);
    }
}
