use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, OnceLock, Weak},
};
pub(crate) const SIDE: usize = 256;
static ZERO_TILE: [u8; SIDE * SIDE * 4] = [0; SIDE * SIDE * 4];
/// Separate identities for storage ownership and independently shared tile data.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AllocationId {
    Storage(usize),
    Tile(usize),
}
#[derive(Clone)]
struct Tiles {
    width: usize,
    height: usize,
    columns: usize,
    blocks: Vec<Option<Arc<Vec<u8>>>>,
}
struct Storage {
    flat: Option<Vec<u8>>,
    tiles: Option<Tiles>,
    cache: OnceLock<Vec<u8>>,
}
impl Clone for Storage {
    fn clone(&self) -> Self {
        Self {
            flat: self.flat.clone(),
            tiles: self.tiles.clone(),
            cache: OnceLock::new(),
        }
    }
}
impl Storage {
    fn bytes(&self) -> &Vec<u8> {
        if let Some(flat) = &self.flat {
            return flat;
        }
        self.cache.get_or_init(|| {
            let tiles = self.tiles.as_ref().unwrap();
            let mut bytes = vec![0; tiles.width * tiles.height * 4];
            for index in 0..tiles.width * tiles.height {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&tiles.rgba(index));
            }
            bytes
        })
    }
}
impl Tiles {
    fn location(&self, index: usize) -> (usize, usize) {
        assert!(index < self.width * self.height);
        let (x, y) = (index % self.width, index / self.width);
        (
            (y / SIDE) * self.columns + x / SIDE,
            ((y % SIDE) * SIDE + x % SIDE) * 4,
        )
    }
    fn rgba(&self, index: usize) -> [u8; 4] {
        let (block, offset) = self.location(index);
        self.blocks[block].as_ref().map_or([0; 4], |bytes| {
            bytes[offset..offset + 4].try_into().unwrap()
        })
    }
    #[inline]
    fn rgba_at(&self, x: usize, y: usize) -> [u8; 4] {
        assert!(x < self.width && y < self.height);
        let block = (y / SIDE) * self.columns + x / SIDE;
        let offset = ((y % SIDE) * SIDE + x % SIDE) * 4;
        self.blocks[block].as_ref().map_or([0; 4], |bytes| {
            bytes[offset..offset + 4].try_into().unwrap()
        })
    }
}
/// Sparse copy-on-write pixels, with explicit flat compatibility for older tools.
#[derive(Clone)]
pub struct Pixels(Arc<Storage>);
/// Weak identities retain allocation headers, never artwork payload ownership.
/// Mutations through Arc::make_mut invalidate these identities without retaining
/// a strong snapshot of the active stroke's pixels.
pub(crate) struct RegionStamp {
    storage: Weak<Storage>,
    tiles: Option<Vec<TileStamp>>,
    dimensions: Option<[usize; 2]>,
}
type TileStamp = (usize, Option<Weak<Vec<u8>>>);
impl RegionStamp {
    #[cfg(test)]
    pub(crate) fn source_is_alive(&self) -> bool {
        self.storage.strong_count() != 0
    }
    pub(crate) fn heap_bytes(&self) -> usize {
        self.tiles.as_ref().map_or(0, |tiles| {
            tiles.capacity() * std::mem::size_of::<TileStamp>()
        })
    }
    pub(crate) fn matches(&self, pixels: &Pixels) -> bool {
        if self.storage.ptr_eq(&Arc::downgrade(&pixels.0)) {
            return true;
        }
        let (Some(stamps), Some(tiles)) = (&self.tiles, &pixels.0.tiles) else {
            return false;
        };
        if self.dimensions != Some([tiles.width, tiles.height]) {
            return false;
        }
        stamps.iter().all(|(index, old)| {
            match (old, tiles.blocks.get(*index).and_then(Option::as_ref)) {
                (None, None) => true,
                (Some(old), Some(current)) => old.ptr_eq(&Arc::downgrade(current)),
                _ => false,
            }
        })
    }
}
impl Pixels {
    /// Conservative sparse occupancy, without decoding or filling the flat cache.
    /// A stored all-transparent tile still counts as present (hidden RGB is data).
    pub(crate) fn region_has_payload(&self, rect: [usize; 4]) -> bool {
        if rect[0] >= rect[2] || rect[1] >= rect[3] {
            return false;
        }
        let Some(tiles) = &self.0.tiles else {
            return self.0.flat.is_some();
        };
        assert!(rect[2] <= tiles.width && rect[3] <= tiles.height);
        for y in rect[1] / SIDE..=(rect[3] - 1) / SIDE {
            for x in rect[0] / SIDE..=(rect[2] - 1) / SIDE {
                if tiles.blocks[y * tiles.columns + x].is_some() {
                    return true;
                }
            }
        }
        false
    }
    /// Whether any pixel under `rect` has nonzero alpha. Alpha-zero pixels keep
    /// hidden RGB but never contribute to a composite, so property changes on a
    /// layer without coverage here cannot change how this area looks. Only
    /// stored tiles overlapping `rect` are read, and only their alpha bytes.
    pub(crate) fn region_has_coverage(&self, width: usize, rect: [usize; 4]) -> bool {
        let [left, top, right, bottom] = rect;
        if left >= right || top >= bottom {
            return false;
        }
        let Some(tiles) = &self.0.tiles else {
            let Some(flat) = &self.0.flat else {
                return false;
            };
            return (top..bottom).any(|y| {
                flat[(y * width + left) * 4..(y * width + right) * 4]
                    .chunks_exact(4)
                    .any(|pixel| pixel[3] != 0)
            });
        };
        assert!(right <= tiles.width && bottom <= tiles.height);
        for ty in top / SIDE..=(bottom - 1) / SIDE {
            for tx in left / SIDE..=(right - 1) / SIDE {
                let Some(block) = &tiles.blocks[ty * tiles.columns + tx] else {
                    continue;
                };
                let x0 = left.max(tx * SIDE) - tx * SIDE;
                let x1 = right.min((tx + 1) * SIDE) - tx * SIDE;
                for y in top.max(ty * SIDE) - ty * SIDE..bottom.min((ty + 1) * SIDE) - ty * SIDE {
                    let row = &block[(y * SIDE + x0) * 4..(y * SIDE + x1) * 4];
                    if row.chunks_exact(4).any(|pixel| pixel[3] != 0) {
                        return true;
                    }
                }
            }
        }
        false
    }
    pub(crate) fn region_stamp(&self, rect: [usize; 4]) -> RegionStamp {
        let tiles = self.0.tiles.as_ref().map(|tiles| {
            let mut stamps = Vec::new();
            if rect[0] < rect[2] && rect[1] < rect[3] {
                for y in rect[1] / SIDE..=(rect[3] - 1) / SIDE {
                    for x in rect[0] / SIDE..=(rect[2] - 1) / SIDE {
                        let index = y * tiles.columns + x;
                        stamps.push((index, tiles.blocks[index].as_ref().map(Arc::downgrade)));
                    }
                }
            }
            stamps
        });
        RegionStamp {
            storage: Arc::downgrade(&self.0),
            tiles,
            dimensions: self
                .0
                .tiles
                .as_ref()
                .map(|tiles| [tiles.width, tiles.height]),
        }
    }
}
impl Default for Pixels {
    fn default() -> Self {
        Vec::new().into()
    }
}
pub enum Writer<'a> {
    Flat(&'a mut [u8]),
    Tiled(TileWriter<'a>),
}
pub struct TileWriter<'a> {
    tiles: &'a mut Tiles,
}
impl Writer<'_> {
    /// Edit a nonempty horizontal span, resolving copy-on-write ownership once
    /// per tile segment. Callers omit zero-coverage spans to preserve sparsity.
    pub(crate) fn edit_span(
        &mut self,
        width: usize,
        x: usize,
        y: usize,
        count: usize,
        mut edit: impl FnMut(usize, &mut [u8]),
    ) {
        assert!(x <= width && count <= width - x);
        match self {
            Self::Flat(bytes) => {
                let start = (y * width + x) * 4;
                for (offset, pixel) in bytes[start..start + count * 4]
                    .chunks_exact_mut(4)
                    .enumerate()
                {
                    edit(offset, pixel);
                }
            }
            Self::Tiled(writer) => {
                assert_eq!(width, writer.tiles.width);
                assert!(y < writer.tiles.height);
                let mut at = x;
                while at < x + count {
                    let end = (x + count).min((at / SIDE + 1) * SIDE);
                    let block = y / SIDE * writer.tiles.columns + at / SIDE;
                    let offset = ((y % SIDE) * SIDE + at % SIDE) * 4;
                    let bytes = writer.tiles.blocks[block]
                        .get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                    for (i, pixel) in Arc::make_mut(bytes)[offset..offset + (end - at) * 4]
                        .chunks_exact_mut(4)
                        .enumerate()
                    {
                        edit(at - x + i, pixel);
                    }
                    at = end;
                }
            }
        }
    }

    /// Copy packed rows, omitting zero segments in previously absent tiles.
    /// Hidden RGB is data even when alpha is zero.
    pub fn write_region(&mut self, width: usize, rect: [usize; 4], pixels: &[u8]) {
        let [left, top, right, bottom] = rect;
        assert!(left <= right && right <= width && top <= bottom);
        assert_eq!(pixels.len(), (right - left) * (bottom - top) * 4);
        let row_bytes = (right - left) * 4;
        for (band, y) in (top..bottom).enumerate() {
            let row = &pixels[band * row_bytes..(band + 1) * row_bytes];
            match self {
                Self::Flat(bytes) => {
                    let offset = (y * width + left) * 4;
                    bytes[offset..offset + row_bytes].copy_from_slice(row);
                }
                Self::Tiled(writer) => {
                    assert_eq!(width, writer.tiles.width);
                    let mut x = left;
                    while x < right {
                        let end = right.min((x / SIDE + 1) * SIDE);
                        let segment = &row[(x - left) * 4..(end - left) * 4];
                        let (block, offset) = writer.tiles.location(y * width + x);
                        if writer.tiles.blocks[block].is_some()
                            || segment.iter().any(|byte| *byte != 0)
                        {
                            let bytes = writer.tiles.blocks[block]
                                .get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                            Arc::make_mut(bytes)[offset..offset + segment.len()]
                                .copy_from_slice(segment);
                        }
                        x = end;
                    }
                }
            }
        }
    }
    /// Exchange a packed rectangle a row segment at a time, preserving tiles.
    pub fn swap_region(&mut self, width: usize, rect: [usize; 4], pixels: &mut [u8]) {
        let [left, top, right, bottom] = rect;
        assert!(left <= right && right <= width && top <= bottom);
        assert_eq!(pixels.len(), (right - left) * (bottom - top) * 4);
        let row_bytes = (right - left) * 4;
        for (band, y) in (top..bottom).enumerate() {
            let row = &mut pixels[band * row_bytes..(band + 1) * row_bytes];
            match self {
                Self::Flat(bytes) => {
                    let offset = (y * width + left) * 4;
                    bytes[offset..offset + row_bytes].swap_with_slice(row);
                }
                Self::Tiled(writer) => {
                    assert_eq!(width, writer.tiles.width);
                    let mut x = left;
                    while x < right {
                        let end = right.min((x / SIDE + 1) * SIDE);
                        let (block, offset) = writer.tiles.location(y * width + x);
                        let bytes = writer.tiles.blocks[block]
                            .get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                        Arc::make_mut(bytes)[offset..offset + (end - x) * 4]
                            .swap_with_slice(&mut row[(x - left) * 4..(end - left) * 4]);
                        x = end;
                    }
                }
            }
        }
        self.reclaim_region(width, rect);
    }
    /// Release all-byte-zero tiles intersecting a completed edit region.
    pub(crate) fn reclaim_region(&mut self, width: usize, rect: [usize; 4]) {
        let [left, top, right, bottom] = rect;
        assert!(left <= right && right <= width && top <= bottom);
        if left < right
            && top < bottom
            && let Self::Tiled(writer) = self
        {
            assert_eq!(width, writer.tiles.width);
            assert!(bottom <= writer.tiles.height);
            for block_y in top / SIDE..bottom.div_ceil(SIDE) {
                for block_x in left / SIDE..right.div_ceil(SIDE) {
                    let index = block_y * writer.tiles.columns + block_x;
                    if writer.tiles.blocks[index]
                        .as_ref()
                        .is_some_and(|bytes| bytes.as_slice() == ZERO_TILE.as_slice())
                    {
                        writer.tiles.blocks[index] = None;
                    }
                }
            }
        }
    }
    pub fn rgba_mut(&mut self, index: usize) -> &mut [u8] {
        match self {
            Self::Flat(bytes) => &mut bytes[index * 4..index * 4 + 4],
            Self::Tiled(writer) => {
                let (block, offset) = writer.tiles.location(index);
                let bytes = writer.tiles.blocks[block]
                    .get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                &mut Arc::make_mut(bytes)[offset..offset + 4]
            }
        }
    }
    /// Rasterizers already know x/y; avoid dividing a packed index for each dab pixel.
    pub(crate) fn rgba_mut_at(&mut self, width: usize, x: usize, y: usize) -> &mut [u8] {
        match self {
            Self::Flat(bytes) => &mut bytes[(y * width + x) * 4..(y * width + x) * 4 + 4],
            Self::Tiled(writer) => {
                assert_eq!(width, writer.tiles.width);
                assert!(x < width && y < writer.tiles.height);
                let block = y / SIDE * writer.tiles.columns + x / SIDE;
                let offset = ((y % SIDE) * SIDE + x % SIDE) * 4;
                let bytes = writer.tiles.blocks[block]
                    .get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                &mut Arc::make_mut(bytes)[offset..offset + 4]
            }
        }
    }
}
impl Pixels {
    /// Packed row segments in document order, skipping absent tiles. Fallible
    /// visitors can cancel without materializing a whole-layer compatibility cache.
    pub(crate) fn try_visit_rows<E>(
        &self,
        width: usize,
        height: usize,
        rect: [usize; 4],
        mut visit: impl FnMut(usize, usize, &[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        let [left, top, right, bottom] = rect;
        assert!(left <= right && right <= width && top <= bottom && bottom <= height);
        assert_eq!(self.len(), width * height * 4);
        for y in top..bottom {
            for bx in left / SIDE..right.div_ceil(SIDE) {
                let x0 = left.max(bx * SIDE);
                let x1 = right.min((bx + 1) * SIDE);
                if let Some(flat) = &self.0.flat {
                    visit(x0, y, &flat[(y * width + x0) * 4..(y * width + x1) * 4])?;
                } else {
                    let tiles = self.0.tiles.as_ref().unwrap();
                    assert_eq!((width, height), (tiles.width, tiles.height));
                    if let Some(bytes) = &tiles.blocks[y / SIDE * tiles.columns + bx] {
                        let offset = ((y % SIDE) * SIDE + x0 % SIDE) * 4;
                        visit(x0, y, &bytes[offset..offset + (x1 - x0) * 4])?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn read_rgba(
        reader: &mut impl std::io::Read,
        width: usize,
        height: usize,
    ) -> std::io::Result<Self> {
        let mut pixels = Self::sparse(width, height);
        let mut row = vec![0; width * 4];
        if width == 0 || height == 0 {
            return Ok(pixels);
        }
        let tiles = Arc::get_mut(&mut pixels.0).unwrap().tiles.as_mut().unwrap();
        for y in 0..height {
            reader.read_exact(&mut row)?;
            for (column, segment) in row.chunks(SIDE * 4).enumerate() {
                if segment.iter().all(|byte| *byte == 0) {
                    continue;
                }
                let block = (y / SIDE) * tiles.columns + column;
                let offset = (y % SIDE) * SIDE * 4;
                let bytes =
                    tiles.blocks[block].get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                Arc::make_mut(bytes)[offset..offset + segment.len()].copy_from_slice(segment);
            }
        }
        Ok(pixels)
    }
    #[cfg(test)]
    pub fn changed_rect(&self, other: &Self, width: usize, height: usize) -> Option<[usize; 4]> {
        self.changed_regions(other, width, height)
            .into_iter()
            .reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            })
    }
    /// Borrow a row segment within one tile without materializing a flat cache.
    pub(crate) fn tile_row(&self, width: usize, y: usize, left: usize, row_bytes: usize) -> &[u8] {
        if let Some(flat) = &self.0.flat {
            let offset = (y * width + left) * 4;
            &flat[offset..offset + row_bytes]
        } else {
            let tiles = self.0.tiles.as_ref().unwrap();
            let block = (y / SIDE) * tiles.columns + left / SIDE;
            let offset = ((y % SIDE) * SIDE + left % SIDE) * 4;
            tiles.blocks[block]
                .as_ref()
                .map_or(&ZERO_TILE[..row_bytes], |bytes| {
                    &bytes[offset..offset + row_bytes]
                })
        }
    }
    pub(crate) fn has_geometry(&self, width: usize, height: usize) -> bool {
        self.len() == width * height * 4
            && self
                .0
                .tiles
                .as_ref()
                .is_none_or(|t| (t.width, t.height) == (width, height))
    }
    pub(crate) fn shares_tile(&self, other: &Self, block: usize) -> bool {
        if self.shares(other) {
            return true;
        }
        if let (Some(a), Some(b)) = (&self.0.tiles, &other.0.tiles) {
            let (old, new) = (&a.blocks[block], &b.blocks[block]);
            matches!((old, new), (None, None))
                || matches!((old, new), (Some(x), Some(y)) if Arc::ptr_eq(x, y))
        } else {
            false
        }
    }
    /// Exact bounds within each changed tile, including flat/mixed storage.
    /// Shared tiles are skipped without reading their pixels. No flat cache is
    /// created merely to compare an action's before/after pixels.
    pub fn changed_regions(&self, other: &Self, width: usize, height: usize) -> Vec<[usize; 4]> {
        let fallback = || {
            crate::document::changed_rect(self, other, width, height)
                .into_iter()
                .collect()
        };
        if [&self.0.tiles, &other.0.tiles].iter().any(|tiles| {
            tiles
                .as_ref()
                .is_some_and(|t| (t.width, t.height) != (width, height))
        }) {
            return fallback();
        }
        if self.shares(other) {
            return Vec::new();
        }
        // Flat row comparisons cheaply narrow localized edits. This borrows
        // existing packed bytes, never asks tiled storage for a flat cache.
        let candidate = if let (Some(a), Some(b)) = (&self.0.flat, &other.0.flat) {
            let Some(rect) = crate::document::changed_rect(a, b, width, height) else {
                return Vec::new();
            };
            rect
        } else {
            [0, 0, width, height]
        };
        let mut regions = Vec::new();
        let columns = width.div_ceil(SIDE);
        for block in 0..columns * height.div_ceil(SIDE) {
            if let (Some(a), Some(b)) = (&self.0.tiles, &other.0.tiles) {
                let (old, new) = (&a.blocks[block], &b.blocks[block]);
                if matches!((old, new), (None, None))
                    || matches!((old, new), (Some(x), Some(y)) if Arc::ptr_eq(x, y))
                {
                    continue;
                }
            }
            let left = (block % columns) * SIDE;
            let top = (block / columns) * SIDE;
            if left >= candidate[2]
                || left + SIDE <= candidate[0]
                || top >= candidate[3]
                || top + SIDE <= candidate[1]
            {
                continue;
            }
            let mut bounds = [width, height, 0, 0];
            let row_bytes = (SIDE.min(width - left)) * 4;
            let first_row = candidate[1].saturating_sub(top);
            let end_row = SIDE.min(height - top).min(candidate[3] - top);
            for local_y in first_row..end_row {
                let old_row = self.tile_row(width, top + local_y, left, row_bytes);
                let new_row = other.tile_row(width, top + local_y, left, row_bytes);
                if old_row == new_row {
                    continue;
                }
                let pixels = row_bytes / 4;
                let differs = |x: usize| old_row[x * 4..x * 4 + 4] != new_row[x * 4..x * 4 + 4];
                let chunk_differs = |x: usize| {
                    let end = (x + 8).min(pixels) * 4;
                    old_row[x * 4..end] != new_row[x * 4..end]
                };
                let first_chunk = (0..pixels).step_by(8).find(|&x| chunk_differs(x)).unwrap();
                let last_chunk = (0..pixels).step_by(8).rfind(|&x| chunk_differs(x)).unwrap();
                let first = (first_chunk..(first_chunk + 8).min(pixels))
                    .find(|&x| differs(x))
                    .unwrap();
                let last = (last_chunk..(last_chunk + 8).min(pixels))
                    .rfind(|&x| differs(x))
                    .unwrap();
                bounds[0] = bounds[0].min(left + first);
                bounds[1] = bounds[1].min(top + local_y);
                bounds[2] = bounds[2].max(left + last + 1);
                bounds[3] = bounds[3].max(top + local_y + 1);
            }
            if bounds[2] > bounds[0] && bounds[3] > bounds[1] {
                regions.push(bounds);
            }
        }
        regions
    }
    /// Clear visible pixels only, retaining hidden RGB on fully transparent
    /// pixels. No-op regions keep sharing; all-zero touched tiles are released.
    pub fn clear_visible(&mut self, width: usize, rect: [usize; 4]) -> bool {
        self.clear_region_inner(width, rect, true)
    }
    /// Lift all stored bytes, including hidden RGB, from a rectangle.
    pub fn clear_region(&mut self, width: usize, rect: [usize; 4]) -> bool {
        self.clear_region_inner(width, rect, false)
    }
    fn clear_region_inner(&mut self, width: usize, rect: [usize; 4], visible_only: bool) -> bool {
        let should_clear = |pixel: &[u8]| {
            if visible_only {
                pixel[3] > 0
            } else {
                pixel != [0; 4]
            }
        };
        let [left, top, right, bottom] = rect;
        assert!(width > 0 && left <= right && right <= width && top <= bottom);
        assert!(bottom <= self.len() / 4 / width);
        if let Some(flat) = &self.0.flat {
            let changed = (top..bottom).any(|y| {
                flat[(y * width + left) * 4..(y * width + right) * 4]
                    .chunks_exact(4)
                    .any(should_clear)
            });
            if !changed {
                return false;
            }
            let storage = Arc::make_mut(&mut self.0);
            storage.cache.take();
            let flat = storage.flat.as_mut().unwrap();
            for y in top..bottom {
                for pixel in
                    flat[(y * width + left) * 4..(y * width + right) * 4].chunks_exact_mut(4)
                {
                    if should_clear(pixel) {
                        pixel.fill(0);
                    }
                }
            }
            return true;
        }
        let tiles = self.0.tiles.as_ref().unwrap();
        assert_eq!(width, tiles.width);
        let mut affected = Vec::new();
        for block_y in top / SIDE..bottom.div_ceil(SIDE) {
            for block_x in left / SIDE..right.div_ceil(SIDE) {
                let index = block_y * tiles.columns + block_x;
                let Some(bytes) = &tiles.blocks[index] else {
                    continue;
                };
                let x0 = left.max(block_x * SIDE) - block_x * SIDE;
                let x1 = right.min((block_x + 1) * SIDE) - block_x * SIDE;
                let y0 = top.max(block_y * SIDE) - block_y * SIDE;
                let y1 = bottom.min((block_y + 1) * SIDE) - block_y * SIDE;
                if (y0..y1).any(|y| {
                    bytes[(y * SIDE + x0) * 4..(y * SIDE + x1) * 4]
                        .chunks_exact(4)
                        .any(should_clear)
                }) {
                    affected.push((index, x0, y0, x1, y1));
                }
            }
        }
        if affected.is_empty() {
            return false;
        }
        let storage = Arc::make_mut(&mut self.0);
        storage.cache.take();
        let tiles = storage.tiles.as_mut().unwrap();
        for (index, x0, y0, x1, y1) in affected {
            let bytes = Arc::make_mut(tiles.blocks[index].as_mut().unwrap());
            for y in y0..y1 {
                for pixel in bytes[(y * SIDE + x0) * 4..(y * SIDE + x1) * 4].chunks_exact_mut(4) {
                    if should_clear(pixel) {
                        pixel.fill(0);
                    }
                }
            }
            if bytes.as_slice() == ZERO_TILE.as_slice() {
                tiles.blocks[index] = None;
            }
        }
        true
    }
    /// Canonical nonempty tile rectangles, also adapting legacy flat storage.
    pub(crate) fn nonzero_tiles(&self, width: usize, height: usize) -> Vec<[usize; 4]> {
        assert_eq!(self.len(), width * height * 4);
        let mut regions = Vec::new();
        let columns = width.div_ceil(SIDE);
        for top in (0..height).step_by(SIDE) {
            for left in (0..width).step_by(SIDE) {
                let right = (left + SIDE).min(width);
                let bottom = (top + SIDE).min(height);
                let nonzero = if let Some(tiles) = &self.0.tiles {
                    assert_eq!((width, height), (tiles.width, tiles.height));
                    tiles.blocks[top / SIDE * columns + left / SIDE]
                        .as_ref()
                        .is_some_and(|bytes| bytes.as_slice() != ZERO_TILE.as_slice())
                } else {
                    let flat = self.0.flat.as_ref().unwrap();
                    (top..bottom).any(|y| {
                        flat[(y * width + left) * 4..(y * width + right) * 4]
                            .iter()
                            .any(|byte| *byte != 0)
                    })
                };
                if nonzero {
                    regions.push([left, top, right, bottom]);
                }
            }
        }
        regions
    }
    /// Visit nonzero RGBA pixels inside a region, skipping absent tiles and
    /// preserving hidden RGB. Reads never materialize compatibility caches.
    pub fn visit_nonzero(
        &self,
        width: usize,
        height: usize,
        rect: [usize; 4],
        mut visit: impl FnMut(usize, usize, [u8; 4]),
    ) {
        let [left, top, right, bottom] = rect;
        assert!(left <= right && right <= width && top <= bottom && bottom <= height);
        assert_eq!(self.len(), width * height * 4);
        if let Some(flat) = &self.0.flat {
            for y in top..bottom {
                for (offset, pixel) in flat[(y * width + left) * 4..(y * width + right) * 4]
                    .chunks_exact(4)
                    .enumerate()
                {
                    if pixel != [0; 4] {
                        visit(left + offset, y, pixel.try_into().unwrap());
                    }
                }
            }
        } else {
            let tiles = self.0.tiles.as_ref().unwrap();
            assert_eq!((width, height), (tiles.width, tiles.height));
            for by in top / SIDE..bottom.div_ceil(SIDE) {
                for bx in left / SIDE..right.div_ceil(SIDE) {
                    let Some(bytes) = &tiles.blocks[by * tiles.columns + bx] else {
                        continue;
                    };
                    let x0 = left.max(bx * SIDE);
                    let x1 = right.min((bx + 1) * SIDE);
                    for y in top.max(by * SIDE)..bottom.min((by + 1) * SIDE) {
                        let start = ((y % SIDE) * SIDE + x0 % SIDE) * 4;
                        for (offset, pixel) in bytes[start..start + (x1 - x0) * 4]
                            .chunks_exact(4)
                            .enumerate()
                        {
                            if pixel != [0; 4] {
                                visit(x0 + offset, y, pixel.try_into().unwrap());
                            }
                        }
                    }
                }
            }
        }
    }
    /// Pack a rectangle directly from flat bytes or sparse tile row segments.
    pub fn region(&self, width: usize, rect: [usize; 4]) -> Vec<u8> {
        let [left, top, right, bottom] = rect;
        assert!(left <= right && right <= width && top <= bottom);
        let row_bytes = (right - left) * 4;
        let mut pixels = vec![0; row_bytes * (bottom - top)];
        for (band, y) in (top..bottom).enumerate() {
            let row = &mut pixels[band * row_bytes..(band + 1) * row_bytes];
            if let Some(flat) = &self.0.flat {
                let offset = (y * width + left) * 4;
                row.copy_from_slice(&flat[offset..offset + row_bytes]);
            } else {
                let tiles = self.0.tiles.as_ref().unwrap();
                assert_eq!(width, tiles.width);
                let mut x = left;
                while x < right {
                    let end = right.min((x / SIDE + 1) * SIDE);
                    let (block, offset) = tiles.location(y * width + x);
                    if let Some(bytes) = &tiles.blocks[block] {
                        row[(x - left) * 4..(end - left) * 4]
                            .copy_from_slice(&bytes[offset..offset + (end - x) * 4]);
                    }
                    x = end;
                }
            }
        }
        pixels
    }
    pub fn sparse(width: usize, height: usize) -> Self {
        if width == 0 || height == 0 {
            return Self::default();
        }
        let columns = width.div_ceil(SIDE);
        Self(Arc::new(Storage {
            flat: None,
            tiles: Some(Tiles {
                width,
                height,
                columns,
                blocks: vec![None; columns * height.div_ceil(SIDE)],
            }),
            cache: OnceLock::new(),
        }))
    }
    /// Every pixel `rgba`, all tiles sharing one allocation.
    pub(crate) fn solid(width: usize, height: usize, rgba: [u8; 4]) -> Self {
        if width == 0 || height == 0 {
            return Self::default();
        }
        let columns = width.div_ceil(SIDE);
        let tile = Arc::new(rgba.repeat(SIDE * SIDE));
        Self(Arc::new(Storage {
            flat: None,
            tiles: Some(Tiles {
                width,
                height,
                columns,
                blocks: vec![Some(tile); columns * height.div_ceil(SIDE)],
            }),
            cache: OnceLock::new(),
        }))
    }
    pub fn len(&self) -> usize {
        self.0.flat.as_ref().map_or_else(
            || {
                let t = self.0.tiles.as_ref().unwrap();
                t.width * t.height * 4
            },
            Vec::len,
        )
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Upper bound for packed tile bytes, without scanning pixels or caches.
    pub(crate) fn payload_bound(&self) -> usize {
        if let Some(flat) = &self.0.flat {
            return flat.len();
        }
        let tiles = self.0.tiles.as_ref().unwrap();
        tiles
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, tile)| tile.is_some())
            .map(|(i, _)| {
                let x = i % tiles.columns * SIDE;
                let y = i / tiles.columns * SIDE;
                (tiles.width - x).min(SIDE) * (tiles.height - y).min(SIDE) * 4
            })
            .sum()
    }
    pub fn capacity(&self) -> usize {
        self.0.flat.as_ref().map_or_else(
            || {
                self.0
                    .tiles
                    .as_ref()
                    .unwrap()
                    .blocks
                    .iter()
                    .flatten()
                    .map(|b| b.capacity())
                    .sum::<usize>()
                    + self.0.cache.get().map_or(0, Vec::capacity)
            },
            Vec::capacity,
        )
    }
    pub fn writer(&mut self) -> Writer<'_> {
        let storage = Arc::make_mut(&mut self.0);
        storage.cache.take();
        if let Some(flat) = &mut storage.flat {
            Writer::Flat(flat)
        } else {
            Writer::Tiled(TileWriter {
                tiles: storage.tiles.as_mut().unwrap(),
            })
        }
    }
    pub(crate) fn is_tiled(&self) -> bool {
        self.0.tiles.is_some()
    }
    /// Separate mutable rows of tiles for bands of `band` rows (a divisor of
    /// the tile side): `bands` are band indices and `columns` tile columns,
    /// indexed `[band][column]`, each slice `band` tile rows of the tile.
    /// Tiles `needed(band, column)` are created; other absent tiles are `None`.
    pub(crate) fn band_tiles(
        &mut self,
        band: usize,
        bands: std::ops::Range<usize>,
        columns: std::ops::Range<usize>,
        needed: impl Fn(usize, usize) -> bool,
    ) -> Vec<Vec<Option<&mut [u8]>>> {
        assert!(band > 0 && SIDE.is_multiple_of(band));
        let per_tile = SIDE / band;
        let storage = Arc::make_mut(&mut self.0);
        storage.cache.take();
        let tiles = storage.tiles.as_mut().expect("band painting needs tiles");
        let mut places = vec![false; tiles.blocks.len()];
        for tile_row in bands.start / per_tile..bands.end.div_ceil(per_tile) {
            for column in columns.clone() {
                let index = tile_row * tiles.columns + column;
                let rows = (tile_row * per_tile).max(bands.start)
                    ..((tile_row + 1) * per_tile).min(bands.end);
                if rows.clone().any(|b| needed(b, column)) {
                    tiles.blocks[index].get_or_insert_with(|| Arc::new(vec![0; SIDE * SIDE * 4]));
                }
                places[index] = true;
            }
        }
        let mut out: Vec<Vec<Option<&mut [u8]>>> = bands
            .clone()
            .map(|_| columns.clone().map(|_| None).collect())
            .collect();
        let width = tiles.columns;
        for (index, (block, place)) in tiles.blocks.iter_mut().zip(places).enumerate() {
            let Some(bytes) = block.as_mut().filter(|_| place) else {
                continue;
            };
            let (tile_row, column) = (index / width, index % width);
            for (part, rows) in Arc::make_mut(bytes).chunks_mut(band * SIDE * 4).enumerate() {
                let b = tile_row * per_tile + part;
                if bands.contains(&b) {
                    out[b - bands.start][column - columns.start] = Some(rows);
                }
            }
        }
        out
    }
    pub fn rgba(&self, index: usize) -> [u8; 4] {
        self.0.flat.as_ref().map_or_else(
            || self.0.tiles.as_ref().unwrap().rgba(index),
            |b| b[index * 4..index * 4 + 4].try_into().unwrap(),
        )
    }
    /// `rgba` by coordinates, for loops that already know them; tiled
    /// storage then needs no division by the width.
    #[inline]
    pub(crate) fn rgba_at(&self, width: usize, x: usize, y: usize) -> [u8; 4] {
        match &self.0.tiles {
            Some(tiles) => tiles.rgba_at(x, y),
            None => self.rgba(y * width + x),
        }
    }
    pub fn write_rgba(&self, writer: &mut impl std::io::Write) -> std::io::Result<()> {
        if let Some(flat) = &self.0.flat {
            return writer.write_all(flat);
        }
        let tiles = self.0.tiles.as_ref().unwrap();
        let mut row = vec![0; tiles.width * 4];
        for y in 0..tiles.height {
            row.fill(0);
            for (column, segment) in row.chunks_mut(SIDE * 4).enumerate() {
                if let Some(block) = &tiles.blocks[(y / SIDE) * tiles.columns + column] {
                    let offset = (y % SIDE) * SIDE * 4;
                    segment.copy_from_slice(&block[offset..offset + segment.len()]);
                }
            }
            writer.write_all(&row)?;
        }
        Ok(())
    }
    pub fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// The identity `visit_allocations` reports for the storage itself.
    pub(crate) fn storage_allocation(&self) -> AllocationId {
        AllocationId::Storage(Arc::as_ptr(&self.0) as usize)
    }
    /// Owned payload capacities and Arc/value headers, without flattening.
    /// The caller deduplicates identities across live layers and snapshots.
    /// Allocator bookkeeping is deliberately excluded.
    pub(crate) fn visit_allocations(&self, mut visit: impl FnMut(AllocationId, usize)) {
        let arc_header = 2 * std::mem::size_of::<usize>();
        let mut owned = std::mem::size_of::<Storage>() + arc_header;
        owned += self.0.flat.as_ref().map_or(0, Vec::capacity);
        owned += self.0.cache.get().map_or(0, Vec::capacity);
        if let Some(tiles) = &self.0.tiles {
            owned += tiles.blocks.capacity() * std::mem::size_of::<Option<Arc<Vec<u8>>>>();
            for tile in tiles.blocks.iter().flatten() {
                visit(
                    AllocationId::Tile(Arc::as_ptr(tile) as usize),
                    tile.capacity() + std::mem::size_of::<Vec<u8>>() + arc_header,
                );
            }
        }
        visit(AllocationId::Storage(Arc::as_ptr(&self.0) as usize), owned);
    }
    pub fn into_vec(self) -> Vec<u8> {
        match Arc::try_unwrap(self.0) {
            Ok(storage) => storage.flat.unwrap_or_else(|| {
                storage.cache.into_inner().unwrap_or_else(|| {
                    let tiles = storage.tiles.unwrap();
                    (0..tiles.width * tiles.height)
                        .flat_map(|i| tiles.rgba(i))
                        .collect()
                })
            }),
            Err(storage) => storage.bytes().clone(),
        }
    }
}
impl Deref for Pixels {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        self.0.bytes()
    }
}
impl DerefMut for Pixels {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        if self.0.flat.is_none() {
            let bytes = self.0.bytes().clone();
            self.0 = Arc::new(Storage {
                flat: Some(bytes),
                tiles: None,
                cache: OnceLock::new(),
            });
        }
        Arc::make_mut(&mut self.0).flat.as_mut().unwrap()
    }
}
impl From<Vec<u8>> for Pixels {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Arc::new(Storage {
            flat: Some(bytes),
            tiles: None,
            cache: OnceLock::new(),
        }))
    }
}
impl FromIterator<u8> for Pixels {
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Self {
        Vec::from_iter(iter).into()
    }
}
impl PartialEq for Pixels {
    fn eq(&self, other: &Self) -> bool {
        if self.shares(other) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.0.flat, &other.0.flat) {
            return a == b;
        }
        if let (Some(a), Some(b)) = (&self.0.tiles, &other.0.tiles)
            && (a.width, a.height) == (b.width, b.height)
        {
            return a.blocks.iter().zip(&b.blocks).all(|(a, b)| match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
                (Some(bytes), None) | (None, Some(bytes)) => {
                    bytes.as_slice() == ZERO_TILE.as_slice()
                }
            });
        }
        // Legacy flat/tiled comparisons and different geometries compare packed
        // logical pixels without filling either compatibility cache.
        (0..self.len() / 4).all(|index| self.rgba(index) == other.rgba(index))
    }
}
impl Eq for Pixels {}
impl PartialEq<Vec<u8>> for Pixels {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.deref() == other
    }
}
impl<const N: usize> PartialEq<[u8; N]> for Pixels {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.deref().as_slice() == other
    }
}
impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}
impl Serialize for Pixels {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.deref().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Pixels {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(Self::from)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn coverage_reads_alpha_only_inside_the_rectangle() {
        let mut pixels = super::Pixels::sparse(600, 300);
        pixels
            .writer()
            .write_region(600, [260, 10, 262, 12], &[0, 0, 0, 255].repeat(4));
        // Hidden RGB with zero alpha is stored but adds no coverage.
        pixels
            .writer()
            .write_region(600, [10, 10, 12, 12], &[9, 9, 9, 0].repeat(4));
        assert!(pixels.region_has_payload([0, 0, 256, 256]));
        assert!(!pixels.region_has_coverage(600, [0, 0, 256, 256]));
        // The stored tile next door is present, but its pixels are elsewhere.
        assert!(pixels.region_has_payload([200, 100, 288, 200]));
        assert!(!pixels.region_has_coverage(600, [200, 100, 288, 200]));
        assert!(pixels.region_has_coverage(600, [200, 0, 288, 50]));
        assert!(pixels.region_has_coverage(600, [261, 11, 262, 12]));
        assert!(!pixels.region_has_coverage(600, [262, 11, 300, 12]));
        let flat: super::Pixels = [0u8, 0, 0, 0, 1, 2, 3, 4].to_vec().into();
        assert!(flat.region_has_coverage(2, [1, 0, 2, 1]));
        assert!(!flat.region_has_coverage(2, [0, 0, 1, 1]));
    }
    #[test]
    fn sparse_occupancy_is_conservative_at_boundaries_and_never_fills_flat_cache() {
        let mut pixels = Pixels::sparse(517, 259);
        assert!(!pixels.region_has_payload([0, 0, 517, 259]));
        pixels
            .writer()
            .rgba_mut(258 * 517 + 516)
            .copy_from_slice(&[9, 11, 13, 0]);
        assert!(pixels.region_has_payload([516, 258, 517, 259]));
        assert!(!pixels.region_has_payload([0, 0, 512, 256]));
        assert!(!pixels.region_has_payload([516, 258, 516, 259]));
        let snapshot = pixels.clone();
        pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 255]);
        assert!(pixels.region_has_payload([0, 0, 1, 1]));
        assert!(!snapshot.region_has_payload([0, 0, 1, 1]));
        assert!(pixels.0.cache.get().is_none() && snapshot.0.cache.get().is_none());
        let flat: Pixels = vec![0; 517 * 259 * 4].into();
        assert!(flat.region_has_payload([0, 0, 1, 1])); // Unknown flat occupancy is not scanned.
    }
    #[test]
    fn span_edits_match_scalar_writes_across_tiles_and_preserve_snapshots() {
        let (width, height) = (773, 259);
        for flat in [false, true] {
            let mut actual = if flat {
                Pixels::from(vec![0; width * height * 4])
            } else {
                Pixels::sparse(width, height)
            };
            actual
                .writer()
                .rgba_mut(258 * width + 770)
                .copy_from_slice(&[19, 23, 61, 255]);
            let snapshot = actual.clone();
            let stamp = snapshot.region_stamp([0, 256, width, height]);
            let mut expected = actual.clone();
            // Cross three tile boundaries, including the partial last tile,
            // then overlap the same row so edits must observe previous pixels.
            for (x, y, count) in [(250, 258, 523), (510, 258, 263), (255, 255, 3)] {
                let edit = |offset: usize, pixel: &mut [u8]| {
                    crate::document::paint_pixel(pixel, 0.4, [offset as u8, 70, 130], false, false);
                };
                actual.writer().edit_span(width, x, y, count, edit);
                let mut writer = expected.writer();
                for offset in 0..count {
                    edit(offset, writer.rgba_mut_at(width, x + offset, y));
                }
            }
            assert_eq!(actual, expected);
            assert!(stamp.matches(&snapshot));
            assert!(!stamp.matches(&actual));
            assert_eq!(snapshot.rgba(258 * width + 770), [19, 23, 61, 255]);
            assert_eq!(snapshot.rgba(258 * width + 250), [0; 4]);
            if !flat {
                assert_eq!(actual.capacity(), 6 * SIDE * SIDE * 4);
                assert!(actual.0.cache.get().is_none());
            }
        }
    }
    #[test]
    fn pixel_equality_does_not_populate_flat_caches_and_checks_odd_tile_edges() {
        let mut a = Pixels::sparse(517, 259);
        a.writer()
            .rgba_mut(258 * 517 + 516)
            .copy_from_slice(&[7, 8, 9, 0]);
        let mut b = a.clone();
        b.writer().rgba_mut(0).copy_from_slice(&[10, 20, 30, 255]);
        assert_ne!(a, b);
        assert_eq!(a.capacity(), SIDE * SIDE * 4);
        assert_eq!(b.capacity(), 2 * SIDE * SIDE * 4);
        let mut c = vec![0; 517 * 259 * 4];
        c[(258 * 517 + 516) * 4..].copy_from_slice(&[7, 8, 9, 0]);
        assert_eq!(a, Pixels::from(c.clone()));
        // Geometry is storage metadata; equality is the packed RGBA sequence.
        let mut different_geometry = Pixels::sparse(259, 517);
        different_geometry
            .writer()
            .rgba_mut(258 * 517 + 516)
            .copy_from_slice(&[7, 8, 9, 0]);
        assert_eq!(a, different_geometry);
        c[0] = 1;
        assert_ne!(a, Pixels::from(c));
        assert_eq!(a.capacity(), SIDE * SIDE * 4);
        assert_eq!(different_geometry.capacity(), SIDE * SIDE * 4);
        assert!(a.0.cache.get().is_none());
        assert!(b.0.cache.get().is_none());
        assert!(different_geometry.0.cache.get().is_none());
    }
    #[test]
    fn region_stamps_validate_geometry_and_do_not_retain_strong_pixel_snapshots() {
        let mut pixels = super::Pixels::sparse(512, 256);
        pixels.writer().rgba_mut(0).copy_from_slice(&[1, 2, 3, 255]);
        let stamp = pixels.region_stamp([0, 0, 256, 256]);
        assert!(stamp.matches(&pixels));
        assert_eq!(std::sync::Arc::strong_count(&pixels.0), 1);
        let block = pixels.0.tiles.as_ref().unwrap().blocks[0].as_ref().unwrap();
        assert_eq!(std::sync::Arc::strong_count(block), 1);
        pixels.writer().rgba_mut(0).copy_from_slice(&[4, 5, 6, 255]);
        assert!(!stamp.matches(&pixels));
        let empty = super::Pixels::sparse(512, 256);
        let empty_stamp = empty.region_stamp([0, 0, 256, 256]);
        assert!(empty_stamp.matches(&super::Pixels::sparse(512, 256)));
        assert!(!empty_stamp.matches(&super::Pixels::sparse(256, 512)));
        let other_stamp = pixels.region_stamp([256, 0, 512, 256]);
        pixels.writer().rgba_mut(0).copy_from_slice(&[7, 8, 9, 255]);
        assert!(other_stamp.matches(&pixels));
    }
    #[test]
    fn completed_swap_reclaims_only_empty_touched_tiles_and_preserves_hidden_rgb() {
        let (width, height) = (517, 259);
        let mut pixels = Pixels::sparse(width, height);
        pixels.writer().rgba_mut(0).copy_from_slice(&[1, 2, 3, 255]);
        pixels
            .writer()
            .rgba_mut(258 * width + 516)
            .copy_from_slice(&[4, 5, 6, 128]);
        let snapshot = pixels.clone();
        let rect = [512, 256, width, height];
        let mut inverse = vec![0; 5 * 3 * 4];
        pixels.writer().swap_region(width, rect, &mut inverse);
        assert_eq!(pixels.capacity(), SIDE * SIDE * 4);
        assert_eq!(pixels.rgba(0), [1, 2, 3, 255]);
        assert_eq!(snapshot.rgba(258 * width + 516), [4, 5, 6, 128]);
        pixels.writer().swap_region(width, rect, &mut inverse);
        assert_eq!(pixels.capacity(), 2 * SIDE * SIDE * 4);
        assert_eq!(pixels.rgba(258 * width + 516), [4, 5, 6, 128]);
        let last = inverse.len() - 4;
        inverse[last..].copy_from_slice(&[7, 8, 9, 0]);
        pixels.writer().swap_region(width, rect, &mut inverse);
        assert_eq!(pixels.rgba(258 * width + 516), [7, 8, 9, 0]);
        assert_eq!(pixels.capacity(), 2 * SIDE * SIDE * 4);
        assert!(pixels.0.cache.get().is_none());
    }
    #[test]
    fn sparse_clear_preserves_hidden_rgb_sharing_and_releases_zero_tiles() {
        let (width, height) = (517, 259);
        let mut pixels = Pixels::sparse(width, height);
        for (x, y, rgba) in [
            (0, 0, [10, 20, 30, 255]),
            (255, 255, [30, 40, 50, 255]),
            (256, 255, [60, 70, 80, 128]),
            (516, 258, [90, 100, 110, 255]),
            (510, 258, [120, 130, 140, 0]),
        ] {
            pixels
                .writer()
                .rgba_mut(y * width + x)
                .copy_from_slice(&rgba);
        }
        let snapshot = pixels.clone();
        assert!(!pixels.clear_visible(width, [1, 1, 8, 8]));
        assert!(pixels.shares(&snapshot));
        let mut flat: Pixels = pixels.region(width, [0, 0, width, height]).into();
        let _ = pixels.as_slice(); // Deliberately prime the compatibility cache.
        assert!(pixels.0.cache.get().is_some());
        let rect = [250, 250, width, height];
        assert!(pixels.clear_visible(width, rect));
        assert!(flat.clear_visible(width, rect));
        assert!(pixels.0.cache.get().is_none());
        assert_eq!(pixels.capacity(), 2 * SIDE * SIDE * 4);
        for index in 0..width * height {
            assert_eq!(pixels.rgba(index), flat.rgba(index));
        }
        assert_eq!(snapshot.rgba(258 * width + 516), [90, 100, 110, 255]);
        assert_eq!(pixels.rgba(258 * width + 510), [120, 130, 140, 0]);
        assert!(pixels.clear_visible(width, [0, 0, width, height]));
        assert_eq!(pixels.capacity(), SIDE * SIDE * 4); // Hidden RGB survives.
        let transparent = pixels.clone();
        assert!(!pixels.clear_visible(width, [0, 0, width, height]));
        assert!(pixels.shares(&transparent));
        let mut blank = Pixels::sparse(width, height);
        assert!(!blank.clear_visible(width, [0, 0, width, height]));
        assert_eq!(blank.capacity(), 0);
    }
    #[test]
    fn row_region_write_clears_existing_tiles_without_allocating_blank_neighbors() {
        let (width, height) = (517, 259);
        let rect = [250, 250, width, height];
        let mut pixels = Pixels::sparse(width, height);
        let blank = vec![0; (width - 250) * (height - 250) * 4];
        pixels.writer().write_region(width, rect, &blank);
        assert_eq!(pixels.capacity(), 0);
        pixels
            .writer()
            .rgba_mut(258 * width + 516)
            .copy_from_slice(&[7, 0, 0, 0]);
        let snapshot = pixels.clone();
        let mut packed = blank.clone();
        packed[..4].copy_from_slice(&[1, 2, 3, 255]);
        pixels.writer().write_region(width, rect, &packed);
        assert_eq!(pixels.rgba(258 * width + 516), [0; 4]);
        assert_eq!(snapshot.rgba(258 * width + 516), [7, 0, 0, 0]);
        assert_eq!(pixels.rgba(250 * width + 250), [1, 2, 3, 255]);
        assert_eq!(pixels.capacity(), 2 * SIDE * SIDE * 4);
        assert_eq!(pixels.region(width, rect), packed);
        assert!(pixels.0.cache.get().is_none());
        assert!(snapshot.0.cache.get().is_none());
    }
    #[test]
    fn row_region_swap_crosses_tiles_preserves_snapshot_and_flat_parity() {
        let (width, height) = (517, 259);
        let rect = [250, 250, width, height];
        let mut pixels = Pixels::sparse(width, height);
        {
            let mut writer = pixels.writer();
            for y in 249..height {
                for x in (249..width).step_by(41) {
                    writer
                        .rgba_mut(y * width + x)
                        .copy_from_slice(&[x as u8, y as u8, 27, 0]);
                }
            }
        }
        let snapshot = pixels.clone();
        let mut flat: Pixels = pixels.region(width, [0, 0, width, height]).into();
        let original: Vec<u8> = (250..height)
            .flat_map(|y| {
                let snapshot = &snapshot;
                (250..width).flat_map(move |x| snapshot.rgba(y * width + x))
            })
            .collect();
        assert_eq!(pixels.region(width, rect), original);
        let replacement: Vec<u8> = (0..original.len()).map(|i| (i % 251) as u8).collect();
        let mut sparse_inverse = replacement.clone();
        let mut flat_inverse = replacement.clone();
        pixels
            .writer()
            .swap_region(width, rect, &mut sparse_inverse);
        flat.writer().swap_region(width, rect, &mut flat_inverse);
        assert_eq!(sparse_inverse, original);
        assert_eq!(flat_inverse, original);
        assert_eq!(pixels.region(width, rect), replacement);
        for i in 0..width * height {
            assert_eq!(pixels.rgba(i), flat.rgba(i));
            if i % width < 250 || i / width < 250 {
                assert_eq!(pixels.rgba(i), snapshot.rgba(i));
            }
        }
        pixels
            .writer()
            .swap_region(width, rect, &mut sparse_inverse);
        for i in 0..width * height {
            assert_eq!(pixels.rgba(i), snapshot.rgba(i));
        }
        assert!(pixels.0.cache.get().is_none());
        assert!(snapshot.0.cache.get().is_none());
    }
    use super::*;
    #[test]
    fn flat_and_mixed_region_diffs_keep_tile_bounds_and_never_fill_caches() {
        let (width, height) = (517, 259);
        let original = vec![255; width * height * 4];
        let mut edited = original.clone();
        for (x, y) in [(0, 0), (128, 0), (256, 256), (516, 258)] {
            edited[(y * width + x) * 4..(y * width + x) * 4 + 4].copy_from_slice(&[7, 8, 9, 0]);
        }
        let make = |bytes: &[u8], flat| {
            if flat {
                Pixels::from(bytes.to_vec())
            } else {
                Pixels::read_rgba(&mut &bytes[..], width, height).unwrap()
            }
        };
        for before_flat in [false, true] {
            for after_flat in [false, true] {
                let before = make(&original, before_flat);
                let after = make(&edited, after_flat);
                let expected = vec![[0, 0, 129, 1], [256, 256, 257, 257], [516, 258, 517, 259]];
                assert_eq!(before.changed_regions(&after, width, height), expected);
                assert_eq!(after.changed_regions(&before, width, height), expected);
                let identical = make(&original, after_flat);
                assert!(before.changed_regions(&identical, width, height).is_empty());
                assert!(before.0.cache.get().is_none());
                assert!(after.0.cache.get().is_none());
                assert!(identical.0.cache.get().is_none());
            }
        }
    }
    #[test]
    fn streaming_sparse_decode_preserves_hidden_rgb_and_rejects_truncation() {
        let mut bytes = vec![0; 257 * 259 * 4];
        let index = (258 * 257 + 256) * 4;
        bytes[index..index + 4].copy_from_slice(&[10, 20, 30, 0]);
        let pixels = Pixels::read_rgba(&mut bytes.as_slice(), 257, 259).unwrap();
        assert_eq!(pixels.capacity(), SIDE * SIDE * 4);
        assert_eq!(pixels.rgba(258 * 257 + 256), [10, 20, 30, 0]);
        let mut output = Vec::new();
        pixels.write_rgba(&mut output).unwrap();
        assert_eq!(output, bytes);
        assert!(pixels.0.cache.get().is_none());
        assert!(Pixels::read_rgba(&mut &bytes[..bytes.len() - 1], 257, 259).is_err());
        let zero = vec![0; 257 * 259 * 4];
        assert_eq!(
            Pixels::read_rgba(&mut zero.as_slice(), 257, 259)
                .unwrap()
                .capacity(),
            0
        );
    }
    #[test]
    fn sparse_diff_tracks_edge_pixels_without_flattening_snapshots() {
        let before = Pixels::sparse(257, 259);
        let mut after = before.clone();
        after
            .writer()
            .rgba_mut(258 * 257 + 256)
            .copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(
            before.changed_rect(&after, 257, 259),
            Some([256, 258, 257, 259])
        );
        assert!(before.0.cache.get().is_none());
        assert!(after.0.cache.get().is_none());
        let mut identical = after.clone();
        identical.writer().rgba_mut(0).copy_from_slice(&[0; 4]);
        assert_eq!(after.changed_rect(&identical, 257, 259), None);
        assert!(identical.0.cache.get().is_none());
    }
    #[test]
    fn clones_share_until_written() {
        let a = Pixels::from(vec![1, 2, 3, 4]);
        let mut b = a.clone();
        assert!(a.shares(&b));
        b[0] = 9;
        assert!(!a.shares(&b));
        assert_eq!(a, vec![1, 2, 3, 4]);
        assert_eq!(b, [9, 2, 3, 4]);
        assert_eq!(b.into_vec(), vec![9, 2, 3, 4]);
    }
    #[test]
    fn sparse_snapshots_copy_only_touched_tiles_and_stream_edge_rows() {
        let mut a = Pixels::sparse(257, 259);
        assert_eq!(a.capacity(), 0);
        a.writer().rgba_mut(256).copy_from_slice(&[1, 2, 3, 4]);
        let snapshot = a.clone();
        a.writer().rgba_mut(256).copy_from_slice(&[9, 10, 11, 12]);
        assert_eq!(snapshot.rgba(256), [1, 2, 3, 4]);
        a.writer()
            .rgba_mut(258 * 257 + 256)
            .copy_from_slice(&[5, 6, 7, 8]);
        assert_eq!(snapshot.rgba(258 * 257 + 256), [0; 4]);
        assert_eq!(a.rgba(256), [9, 10, 11, 12]);
        assert_eq!(a.capacity(), 2 * SIDE * SIDE * 4);
        let mut streamed = Vec::new();
        a.write_rgba(&mut streamed).unwrap();
        assert_eq!(streamed.len(), 257 * 259 * 4);
        assert_eq!(&streamed[(258 * 257 + 256) * 4..], &[5, 6, 7, 8]);
        assert!(a.0.cache.get().is_none());
    }
    #[test]
    fn compatibility_cache_is_invalidated_after_tile_writes() {
        let mut pixels = Pixels::sparse(257, 2);
        assert_eq!(pixels[0], 0);
        assert!(pixels.0.cache.get().is_some());
        pixels.writer().rgba_mut(0).copy_from_slice(&[1, 2, 3, 4]);
        assert!(pixels.0.cache.get().is_none());
        assert_eq!(&pixels[..4], &[1, 2, 3, 4]);
        let snapshot = pixels.clone();
        pixels[0] = 9;
        assert_eq!(snapshot.rgba(0), [1, 2, 3, 4]);
        assert_eq!(pixels.rgba(0), [9, 2, 3, 4]);
    }
}
