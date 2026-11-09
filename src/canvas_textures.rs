//! Fixed-size padded display tiles with dirty-region uploads.
use crate::{document::Document, egui, performance::Performance, view::View};
use egui::{Color32, Pos2, Rect, pos2};
const SIDE: usize = 256;
const PAD: usize = 32;
const EXTENT: usize = SIDE + PAD * 2;
struct Tile {
    overview: Option<(egui::TextureHandle, Color32)>,
    level: usize,
    bounds: [usize; 4],
    texture: Option<egui::TextureHandle>,
    first: Color32,
    signature: Vec<LayerSignature>,
    /// The texture is a quick one (a pixel per `level` square) still to be
    /// replaced by the exact composite.
    rough: bool,
}

/// Uniform padded tiles sample identically from a single texel at every zoom
/// and rotation. Only full replacements may discard the other texels.
fn tile_image(local: [usize; 4], pixels: &[u8]) -> egui::ColorImage {
    if local == [0, 0, EXTENT, EXTENT] && pixels.chunks_exact(4).all(|pixel| pixel == &pixels[..4])
    {
        // Detect uniform straight RGBA before allocating and converting a full
        // ColorImage. Only one texel needs premultiplication and upload.
        egui::ColorImage::from_rgba_unmultiplied([1, 1], &pixels[..4])
    } else {
        egui::ColorImage::from_rgba_unmultiplied([local[2] - local[0], local[3] - local[1]], pixels)
    }
}

/// Downsample premultiplied pixels, keeping transparent colors from bleeding.
/// Powers of two preserve the padded tile's normalized UVs and shared gutters.
fn reduce_image(image: egui::ColorImage, level: usize) -> egui::ColorImage {
    if level == 1 || image.size == [1, 1] {
        return image;
    }
    reduce_image_ref(&image, level)
}
fn reduce_image_ref(image: &egui::ColorImage, level: usize) -> egui::ColorImage {
    if image.size == [1, 1] {
        return image.clone();
    }
    let [width, height] = image.size;
    let size = [width / level, height / level];
    let count = (level * level) as u32;
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let mut sums = [0u32; 4];
            for sy in y * level..(y + 1) * level {
                for pixel in &image.pixels[sy * width + x * level..sy * width + (x + 1) * level] {
                    for (sum, channel) in sums.iter_mut().zip(pixel.to_array()) {
                        *sum += u32::from(channel);
                    }
                }
            }
            let [r, g, b, a] = sums.map(|sum| ((sum + count / 2) / count) as u8);
            pixels.push(Color32::from_rgba_premultiplied(r, g, b, a));
        }
    }
    egui::ColorImage::new(size, pixels)
}

/// Around 256 document-wide samples keep thin detail stable in the navigator,
/// including renderers that do not generate mip levels for uploaded textures.
fn overview_level(doc: &Document) -> usize {
    doc.width
        .max(doc.height)
        .div_ceil(256)
        .next_power_of_two()
        .clamp(1, 16)
}

fn update_overview_options(
    overview: &mut Option<(egui::TextureHandle, Color32)>,
    options: egui::TextureOptions,
    performance: &mut Performance,
) {
    if let Some((texture, first)) = overview {
        texture.set_partial([0, 0], egui::ColorImage::filled([1, 1], *first), options);
        performance.display_queued_bytes += 4;
    }
}

/// A first edit to a compact tile reconstructs its constant background and
/// installs a complete texture. Later edits retain normal dirty subuploads.
fn prepare_tile_image(
    tile: &Tile,
    local: [usize; 4],
    image: egui::ColorImage,
) -> (egui::ColorImage, bool) {
    prepare_scaled_patch(
        tile.texture.as_ref().is_some_and(|t| t.size() == [1, 1]),
        tile.first,
        EXTENT,
        local,
        image,
    )
}
/// Expand a compact constant texture directly at its retained resolution.
fn prepare_scaled_patch(
    compact: bool,
    first: Color32,
    side: usize,
    local: [usize; 4],
    image: egui::ColorImage,
) -> (egui::ColorImage, bool) {
    if local == [0, 0, side, side] {
        return (image, true);
    }
    if !compact {
        return (image, false);
    }
    if image.pixels.iter().all(|pixel| *pixel == first) {
        return (egui::ColorImage::filled([1, 1], first), true);
    }
    let mut expanded = egui::ColorImage::filled([side, side], first);
    for y in 0..image.size[1] {
        let start = (local[1] + y) * side + local[0];
        expanded.pixels[start..start + image.size[0]]
            .copy_from_slice(&image.pixels[y * image.size[0]..(y + 1) * image.size[0]]);
    }
    (expanded, true)
}

struct PresentedTile {
    overview: Option<(egui::TextureHandle, Color32)>,
    bounds: [usize; 4],
    texture: egui::TextureHandle,
    first: Color32,
}
struct LayerSignature {
    rect: [usize; 4],
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
#[cfg(test)]
#[test]
fn mask_only_signatures_include_folder_coverage_and_preserve_unrelated_tiles() {
    let mut doc = Document::new(768, 256);
    doc.version = 2;
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[201, 17, 83, 255]);
    doc.layers[0].depth = 1;
    doc.layers
        .push(crate::document::Layer::new_folder("Masked group".into()));
    doc.layers[1].mask = Some(crate::layer_mask::LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage: crate::layer_mask::Mask::new(768, 256, 255).unwrap(),
    });
    let near = signature(&doc, [0, 0, 256, 256]);
    let distant = signature(&doc, [512, 0, 768, 256]);
    doc.layers[1]
        .mask
        .as_mut()
        .unwrap()
        .coverage
        .writer()
        .set(0, 0, 0);
    assert!(!signature_matches(&near, &doc));
    assert!(!metadata_matches(&near, &doc));
    assert!(signature_matches(&distant, &doc));
    doc.layers[1].mask.as_mut().unwrap().enabled = false;
    assert!(!signature_matches(&distant, &doc));
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
fn signature(doc: &Document, tile: [usize; 4]) -> Vec<LayerSignature> {
    let rect = [
        tile[0].saturating_sub(PAD),
        tile[1].saturating_sub(PAD),
        (tile[0] + SIDE + PAD).min(doc.width),
        (tile[1] + SIDE + PAD).min(doc.height),
    ];
    doc.layers
        .iter()
        .map(|layer| LayerSignature {
            rect,
            metadata: metadata(layer, rect),
            mask: crate::layer_mask::LayerStamp::new(layer.mask.as_ref(), rect),
            pixels: layer
                .pixels
                .region_stamp(if layer.folder { [0; 4] } else { rect }),
        })
        .collect()
}
fn unchanged(tile: &Tile, doc: &Document) -> bool {
    signature_matches(&tile.signature, doc)
}
/// Whether a layer, or any raster inside a folder, has visible coverage
/// under `rect`. Folder contents are the contiguous deeper entries directly
/// below it.
fn subtree_has_payload(doc: &Document, index: usize, rect: [usize; 4]) -> bool {
    let layer = &doc.layers[index];
    if layer.adjustment.is_some() {
        return true;
    }
    if !layer.folder {
        return layer.display_reference.as_ref().map_or_else(
            || layer.pixels.region_has_coverage(doc.width, rect) && !layer.mask_hides(rect),
            |r| r.intersects(rect),
        );
    }
    doc.layers[..index]
        .iter()
        .rev()
        .take_while(|inner| inner.depth > layer.depth)
        .any(|inner| !inner.folder && inner.has_display_payload(rect))
}
/// A tile still shows the right image when the only changes are visibility,
/// opacity or blend on layers with no pixels under it: their alpha there is
/// zero either way, and an empty clipping base keeps its group empty. Changes
/// to clipping, nesting or folder status regroup other layers, so they never
/// qualify. Toggling a sparse layer then recomposites only tiles it covers.
fn visually_unchanged(tile: &Tile, doc: &Document) -> bool {
    if tile.signature.len() != doc.layers.len() {
        return false;
    }
    let rect = [
        tile.bounds[0].saturating_sub(PAD),
        tile.bounds[1].saturating_sub(PAD),
        (tile.bounds[0] + SIDE + PAD).min(doc.width),
        (tile.bounds[1] + SIDE + PAD).min(doc.height),
    ];
    tile.signature
        .iter()
        .zip(&doc.layers)
        .enumerate()
        .all(|(index, (old, layer))| {
            let now = metadata(layer, rect);
            old.pixels.matches(&layer.pixels)
                && old.mask.matches(layer.mask.as_ref())
                && (old.metadata == now
                    || (old.metadata.7 == now.7
                        && ((old.metadata.3, old.metadata.4, old.metadata.5)
                            == (now.3, now.4, now.5)
                            && !subtree_has_payload(doc, index, rect))))
        })
}
fn metadata_matches(signature: &[LayerSignature], doc: &Document) -> bool {
    signature.len() == doc.layers.len()
        && signature.iter().zip(&doc.layers).all(|(old, layer)| {
            old.metadata == metadata(layer, old.rect) && old.mask.matches(layer.mask.as_ref())
        })
}
fn signature_matches(signature: &[LayerSignature], doc: &Document) -> bool {
    signature.len() == doc.layers.len()
        && signature.iter().zip(&doc.layers).all(|(old, layer)| {
            old.metadata == metadata(layer, old.rect)
                && old.pixels.matches(&layer.pixels)
                && old.mask.matches(layer.mask.as_ref())
        })
}
#[derive(Clone, Copy, PartialEq)]
struct ResolutionKey {
    dimensions: [usize; 2],
    viewport: Option<(Rect, View)>,
    pixels_per_point: f32,
}
#[derive(Default)]
pub struct CanvasTextures {
    prefix: crate::prefix_cache::PrefixCache,
    dimensions: [usize; 2],
    tiles: Vec<Tile>,
    presented: Vec<PresentedTile>,
    presented_dimensions: [usize; 2],
    /// Composite tiles with alpha (over a checkerboard) instead of on paper.
    transparent: bool,
    /// Mode of the presented frame; tiles of one frame always share it.
    presented_transparent: bool,
    checker: Option<egui::TextureHandle>,
    /// Show the checkerboard behind alpha tiles; otherwise white paper.
    show_checker: bool,
    generation: Option<Document>,
    refresh: bool,
    options: Option<egui::TextureOptions>,
    pending: Vec<Option<[usize; 4]>>,
    composite: Option<DisplayJob>,
    retired: Option<DisplayWorker>,
    viewport: Option<(Rect, View)>,
    /// Frozen resolution plan for the current generation (1, 2 or 4).
    levels: Vec<usize>,
    /// Desired plan is cached separately while a generation keeps its frozen plan.
    desired_levels: Vec<usize>,
    resolution_key: Option<ResolutionKey>,
    #[cfg(test)]
    resolution_builds: usize,
    worker_failed: bool,
    error: Option<String>,
    #[cfg(test)]
    test_units: Option<usize>,
}
struct DisplayJob {
    tile: usize,
    local: [usize; 4],
    source_rect: [usize; 4],
    preparation: Preparation,
    signature: Vec<LayerSignature>,
    /// 1 for the exact composite, else the side of the squares a quick job
    /// composites one pixel of.
    step: usize,
    /// Replaces quick tiles that already show the current document.
    refine: bool,
    /// Neighbouring tiles in the same row composited by the same job, each
    /// with its local rectangle and source signature.
    batch: Vec<(usize, [usize; 4], Vec<LayerSignature>)>,
}
/// Tiles one display job composites together. Each worker job already splits
/// its region across cores, so a wider region finishes several tiles in the
/// time one used to take, instead of one tile per frame.
const BATCH: usize = 4;
/// Document pixels a tile's local rectangle samples, clamped at canvas edges.
fn tile_source_rect(bounds: [usize; 4], local: [usize; 4], doc: &Document) -> [usize; 4] {
    let ox = bounds[0] as isize - PAD as isize;
    let oy = bounds[1] as isize - PAD as isize;
    let x = |value: usize| (ox + value as isize).clamp(0, doc.width as isize - 1) as usize;
    let y = |value: usize| (oy + value as isize).clamp(0, doc.height as isize - 1) as usize;
    [
        x(local[0]),
        y(local[1]),
        x(local[2] - 1) + 1,
        y(local[3] - 1) + 1,
    ]
}

enum Preparation {
    Cursor(Box<crate::composite_cursor::CompositeCursor>),
    Worker(DisplayWorker),
}
struct DisplayWorker {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<crate::display_workers::Job<DisplayResult>>,
    capacity: usize,
    snapshot: crate::snapshot_memory::Snapshot,
}
type DisplayResult = Result<(Vec<u8>, f64), String>;
impl DisplayWorker {
    fn start(
        doc: &Document,
        rect: [usize; 4],
        transparent: bool,
        step: usize,
    ) -> Result<Self, String> {
        let doc = doc.clone();
        let snapshot = crate::snapshot_memory::Snapshot::capture(&doc);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let handle = crate::display_workers::spawn(move || {
            let started = std::time::Instant::now();
            let pixels = if step > 1 {
                doc.composite_sampled_cancelable(rect, step, &worker_cancel, transparent)?
            } else {
                doc.composite_region_cancelable(rect, &worker_cancel, transparent)?
            };
            Ok((pixels, started.elapsed().as_secs_f64() * 1000.0))
        })
        .map_err(|error| error.to_string())?;
        Ok(Self {
            snapshot,
            cancel,
            handle: Some(handle),
            capacity: (rect[2] - rect[0]) * (rect[3] - rect[1]) * 4,
        })
    }
    fn ready(&self) -> bool {
        self.handle
            .as_ref()
            .is_none_or(|handle| handle.is_finished())
    }
    fn finish(mut self) -> Result<(Vec<u8>, f64), String> {
        self.handle
            .take()
            .unwrap()
            .join()
            .unwrap_or_else(|_| Err("Display worker panicked".into()))
    }
}
impl Drop for DisplayWorker {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
impl Preparation {
    fn snapshot(&self) -> Option<&crate::snapshot_memory::Snapshot> {
        match self {
            Self::Cursor(cursor) => Some(&cursor.snapshot),
            Self::Worker(worker) => (!worker.ready()).then_some(&worker.snapshot),
        }
    }
    fn capacity_bytes(&self) -> usize {
        match self {
            Self::Cursor(cursor) => cursor.capacity_bytes(),
            Self::Worker(worker) => worker.capacity,
        }
    }
    fn step(&mut self) -> bool {
        match self {
            Self::Cursor(cursor) => cursor.step(128),
            Self::Worker(worker) => worker.ready(),
        }
    }
    fn finish(self) -> Result<(Vec<u8>, f64), String> {
        match self {
            Self::Cursor(cursor) => Ok((cursor.finish(), 0.0)),
            Self::Worker(worker) => worker.finish(),
        }
    }
}
#[derive(Default, serde::Serialize)]
pub(crate) struct Memory {
    pub resident_tiles: usize,
    pub texture_base_rgba_bytes: usize,
    pub texture_with_mips_rgba_estimate_bytes: usize,
    pub metadata_heap_bytes: usize,
    pub prefix_heap_bytes: usize,
}
pub(crate) fn bounds(width: usize, height: usize) -> Vec<[usize; 4]> {
    let mut result = Vec::new();
    for top in (0..height).step_by(SIDE) {
        for left in (0..width).step_by(SIDE) {
            result.push([
                left,
                top,
                (left + SIDE).min(width),
                (top + SIDE).min(height),
            ]);
        }
    }
    result
}
fn dirty_rect(
    tile: [usize; 4],
    dirty: [usize; 4],
    width: usize,
    height: usize,
) -> Option<[usize; 4]> {
    let [left, top, right, bottom] = dirty;
    if left >= right || top >= bottom {
        return None;
    }
    let ox = tile[0] as isize - PAD as isize;
    let oy = tile[1] as isize - PAD as isize;
    let local = |coordinate: usize, origin: isize| {
        (coordinate as isize - origin).clamp(0, EXTENT as isize) as usize
    };
    let l = if left == 0 { 0 } else { local(left, ox) };
    let t = if top == 0 { 0 } else { local(top, oy) };
    let r = if right >= width {
        EXTENT
    } else {
        local(right, ox)
    };
    let b = if bottom >= height {
        EXTENT
    } else {
        local(bottom, oy)
    };
    (l < r && t < b).then_some([l, t, r, b])
}
pub(crate) fn rgba(doc: &Document, tile: [usize; 4], local: [usize; 4]) -> Vec<u8> {
    rgba_display(doc, tile, local, false)
}
fn rgba_display(doc: &Document, tile: [usize; 4], local: [usize; 4], transparent: bool) -> Vec<u8> {
    let [left, top, right, bottom] = local;
    let ox = tile[0] as isize - PAD as isize;
    let oy = tile[1] as isize - PAD as isize;
    let sx = |x: usize| (ox + x as isize).clamp(0, doc.width as isize - 1) as usize;
    let sy = |y: usize| (oy + y as isize).clamp(0, doc.height as isize - 1) as usize;
    let (x0, y0, x1, y1) = (sx(left), sy(top), sx(right - 1) + 1, sy(bottom - 1) + 1);
    let composite = doc.composite_display_region([x0, y0, x1, y1], transparent);
    if x1 - x0 == right - left && y1 - y0 == bottom - top {
        return composite;
    }
    sampled_rgba(doc, tile, local, [x0, y0, x1, y1], &composite)
}
fn sampled_rgba(
    doc: &Document,
    tile: [usize; 4],
    local: [usize; 4],
    source: [usize; 4],
    composite: &[u8],
) -> Vec<u8> {
    let [left, top, right, bottom] = local;
    let [x0, y0, x1, _] = source;
    let ox = tile[0] as isize - PAD as isize;
    let oy = tile[1] as isize - PAD as isize;
    let sx = |x: usize| (ox + x as isize).clamp(0, doc.width as isize - 1) as usize;
    let sy = |y: usize| (oy + y as isize).clamp(0, doc.height as isize - 1) as usize;
    let mut pixels = vec![0; (right - left) * (bottom - top) * 4];
    let core_left = (-ox).max(left as isize).min(right as isize) as usize;
    let core_right = (doc.width as isize - ox)
        .max(core_left as isize)
        .min(right as isize) as usize;
    for y in top..bottom {
        let source_row = (sy(y) - y0) * (x1 - x0) * 4;
        let target_row = (y - top) * (right - left) * 4;
        let row = &mut pixels[target_row..target_row + (right - left) * 4];
        let prefix = (core_left - left) * 4;
        let core_end = (core_right - left) * 4;
        let first = source_row + (sx(left) - x0) * 4;
        for pixel in row[..prefix].chunks_exact_mut(4) {
            pixel.copy_from_slice(&composite[first..first + 4]);
        }
        if core_left < core_right {
            let start = source_row + (sx(core_left) - x0) * 4;
            row[prefix..core_end].copy_from_slice(&composite[start..start + core_end - prefix]);
        }
        let last = source_row + (sx(right - 1) - x0) * 4;
        for pixel in row[core_end..].chunks_exact_mut(4) {
            pixel.copy_from_slice(&composite[last..last + 4]);
        }
    }
    pixels
}
/// Pack a shared composite directly into upload pixels. Clamp edge gutters
/// after conversion, so repeated border rows/pixels do not need conversion again.
/// Full uniform tiles remain allocation-free apart from their single texel.
fn sampled_image(
    doc: &Document,
    tile: [usize; 4],
    local: [usize; 4],
    source: [usize; 4],
    composite: &[u8],
) -> egui::ColorImage {
    let [left, top, right, bottom] = local;
    let [x0, y0, x1, _] = source;
    let ox = tile[0] as isize - PAD as isize;
    let oy = tile[1] as isize - PAD as isize;
    let sx = |x: usize| (ox + x as isize).clamp(0, doc.width as isize - 1) as usize;
    let sy = |y: usize| (oy + y as isize).clamp(0, doc.height as isize - 1) as usize;
    let stride = (x1 - x0) * 4;
    let start_x = (sx(left) - x0) * 4;
    let end_x = (sx(right - 1) - x0 + 1) * 4;
    let row = |y: usize| {
        let start = (y - y0) * stride;
        &composite[start + start_x..start + end_x]
    };
    let first = &row(sy(top))[..4];
    if local == [0, 0, EXTENT, EXTENT]
        && (sy(top)..=sy(bottom - 1)).all(|y| row(y).chunks_exact(4).all(|p| p == first))
    {
        return egui::ColorImage::from_rgba_unmultiplied([1, 1], first);
    }
    let width = right - left;
    let height = bottom - top;
    let prefix = (-ox - left as isize).clamp(0, width as isize - 1) as usize;
    let mut pixels = Vec::with_capacity(width * height);
    let mut previous_y = None;
    for y in top..bottom {
        let source_y = sy(y);
        if previous_y == Some(source_y) {
            let end = pixels.len();
            pixels.extend_from_within(end - width..end);
            continue;
        }
        let start = pixels.len();
        let source_row = row(source_y);
        let first = Color32::from_rgba_unmultiplied(
            source_row[0],
            source_row[1],
            source_row[2],
            source_row[3],
        );
        pixels.resize(start + prefix, first);
        pixels.extend(
            source_row
                .chunks_exact(4)
                .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])),
        );
        let last = *pixels.last().unwrap();
        pixels.resize(start + width, last);
        previous_y = Some(source_y);
    }
    egui::ColorImage::new([width, height], pixels)
}
impl CanvasTextures {
    fn publish(&mut self) {
        if self.presented_dimensions == self.dimensions
            && self.presented.len() == self.tiles.len()
            && self.presented.iter().zip(&self.tiles).all(|(shown, tile)| {
                shown.bounds == tile.bounds
                    && shown.overview.as_ref().map(|(t, c)| (t.id(), *c))
                        == tile.overview.as_ref().map(|(t, c)| (t.id(), *c))
                    && shown.first == tile.first
                    && tile
                        .texture
                        .as_ref()
                        .is_some_and(|texture| texture.id() == shown.texture.id())
            })
        {
            return;
        }
        self.presented = self
            .tiles
            .iter()
            .map(|tile| PresentedTile {
                overview: tile.overview.clone(),
                bounds: tile.bounds,
                texture: tile.texture.as_ref().unwrap().clone(),
                first: tile.first,
            })
            .collect();
        self.presented_dimensions = self.dimensions;
        self.presented_transparent = self.transparent;
    }
    #[cfg(test)]
    pub(crate) fn alpha(&self) -> bool {
        self.transparent
    }
    pub(crate) fn set_checker(&mut self, show: bool) {
        self.show_checker = show;
    }
    /// Switch between paper and checkerboard display. Every tile becomes stale
    /// and in-flight work is retired, so no tile composited in the old mode can
    /// publish; the current frame stays visible until the new one is complete.
    pub(crate) fn set_transparent(&mut self, transparent: bool) -> bool {
        if self.transparent == transparent {
            return false;
        }
        self.transparent = transparent;
        self.retire();
        for tile in &mut self.tiles {
            tile.signature.clear();
        }
        true
    }
    pub(crate) fn register_snapshots(&self, registry: &mut crate::snapshot_memory::Registry) {
        if let Some(source) = &self.generation {
            registry.add_document(source);
        }
        if let Some(snapshot) = self
            .composite
            .as_ref()
            .and_then(|job| job.preparation.snapshot())
        {
            registry.add_snapshot(snapshot);
        }
        if let Some(worker) = self.retired.as_ref().filter(|worker| !worker.ready()) {
            registry.add_snapshot(&worker.snapshot);
        }
    }
    pub(crate) fn snapshot_memory(&self, live: &Document) -> crate::snapshot_memory::Memory {
        let generation = self
            .generation
            .as_ref()
            .map(crate::snapshot_memory::Snapshot::capture);
        let active = self
            .composite
            .as_ref()
            .and_then(|job| job.preparation.snapshot());
        let retired = self
            .retired
            .as_ref()
            .filter(|worker| !worker.ready())
            .map(|worker| &worker.snapshot);
        crate::snapshot_memory::beyond_live(
            live,
            generation.as_ref().into_iter().chain(active).chain(retired),
        )
    }
    /// Keep a quarter-resolution overview outside a one-tile viewport margin.
    /// Visible pixels retain at least the screen's sampling density. A large or
    /// high-DPI viewport may therefore need more memory than a small one.
    fn resolution_plan(&self, pixels_per_point: f32) -> Vec<usize> {
        let Some((area, view)) = self
            .viewport
            .filter(|_| self.dimensions[0] * self.dimensions[1] >= 2048 * 2048)
        else {
            return vec![1; self.tiles.len()];
        };
        let scale = view.zoom * pixels_per_point;
        let visible_level = if scale <= 0.25 {
            4
        } else if scale <= 0.5 {
            2
        } else {
            1
        };
        let area = area.expand(SIDE as f32 * view.zoom);
        self.tiles
            .iter()
            .map(|tile| {
                let [l, t, r, b] = tile.bounds.map(|v| v as f32);
                let corners =
                    [pos2(l, t), pos2(r, t), pos2(r, b), pos2(l, b)].map(|p| view.to_screen(p));
                if Rect::from_points(&corners).intersects(area) {
                    visible_level
                } else {
                    4
                }
            })
            .collect()
    }
    pub(crate) fn sync_resolution_plan(&mut self, ctx: &egui::Context, doc: &Document) {
        self.resize(doc);
        let viewport = self
            .viewport
            .filter(|_| self.dimensions[0] * self.dimensions[1] >= 2048 * 2048);
        let key = ResolutionKey {
            dimensions: self.dimensions,
            viewport,
            pixels_per_point: if viewport.is_some() {
                ctx.pixels_per_point()
            } else {
                1.0
            },
        };
        if self.resolution_key != Some(key) {
            self.desired_levels = self.resolution_plan(key.pixels_per_point);
            self.resolution_key = Some(key);
            #[cfg(test)]
            {
                self.resolution_builds += 1;
            }
        }
        if self.generation.is_none()
            && self.composite.is_none()
            && self.retired.is_none()
            && self.pending.iter().all(Option::is_none)
        {
            if self.levels != self.desired_levels {
                self.levels.clone_from(&self.desired_levels);
            }
        } else if self.levels != self.desired_levels {
            self.refresh = true;
        }
    }
    fn resolution_stale(&self) -> bool {
        self.tiles
            .iter()
            .zip(&self.levels)
            .any(|(tile, level)| tile.level != *level)
    }
    fn full_resolution_damage(&self, doc: &Document, rect: [usize; 4]) -> bool {
        self.tiles.iter().zip(&self.levels).all(|(tile, level)| {
            *level == 1 || dirty_rect(tile.bounds, rect, doc.width, doc.height).is_none()
        })
    }
    pub(crate) fn set_viewport(&mut self, area: Rect, view: View) {
        self.viewport = (area.width() > 0.0 && area.height() > 0.0).then_some((area, view));
    }
    pub(crate) fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
    fn retire(&mut self) {
        if let Some(job) = self.composite.take()
            && let Preparation::Worker(worker) = job.preparation
        {
            worker
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            debug_assert!(self.retired.is_none());
            self.retired = Some(worker);
        }
    }
    fn waiting(&self) -> bool {
        self.retired.as_ref().is_some_and(|worker| !worker.ready())
            || self.composite.as_ref().is_some_and(
                |job| matches!(&job.preparation, Preparation::Worker(worker) if !worker.ready()),
            )
    }
    /// A quick tile already showing `doc`, nearest the view first.
    fn next_rough(&self, doc: &Document) -> Option<usize> {
        let visible = |index: usize| {
            self.viewport.is_none_or(|(area, view)| {
                let [left, top, right, bottom] = self.tiles[index].bounds;
                let corners = [
                    pos2(left as f32, top as f32),
                    pos2(right as f32, top as f32),
                    pos2(right as f32, bottom as f32),
                    pos2(left as f32, bottom as f32),
                ]
                .map(|point| view.to_screen(point));
                Rect::from_points(&corners).intersects(area)
            })
        };
        let candidates = || {
            self.tiles.iter().enumerate().filter(|(index, tile)| {
                tile.rough
                    && tile.level == self.levels[*index]
                    && self.pending[*index].is_none()
                    && unchanged(tile, doc)
            })
        };
        candidates()
            .find(|(index, _)| visible(*index))
            .or_else(|| candidates().next())
            .map(|(index, _)| index)
    }
    fn next_pending(&self) -> Option<usize> {
        self.pending
            .iter()
            .enumerate()
            .filter(|(_, pending)| pending.is_some())
            .min_by(|(a, _), (b, _)| {
                let priority = |index: usize| {
                    let Some((area, view)) = self.viewport else {
                        return (false, index as f32);
                    };
                    let [left, top, right, bottom] = self.tiles[index].bounds;
                    let corners = [
                        pos2(left as f32, top as f32),
                        pos2(right as f32, top as f32),
                        pos2(right as f32, bottom as f32),
                        pos2(left as f32, bottom as f32),
                    ]
                    .map(|point| view.to_screen(point));
                    let center = view.to_screen(pos2(
                        (left + right) as f32 / 2.0,
                        (top + bottom) as f32 / 2.0,
                    ));
                    (
                        !Rect::from_points(&corners).intersects(area),
                        center.distance_sq(area.center()),
                    )
                };
                let a = priority(*a);
                let b = priority(*b);
                a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1))
            })
            .map(|(index, _)| index)
    }
    fn resize(&mut self, doc: &Document) -> bool {
        if self.dimensions == [doc.width, doc.height] {
            return false;
        }
        self.retire();
        self.generation = None;
        self.refresh = false;
        // A differently sized frame cannot be painted. Release its handles now
        // instead of retaining invisible textures through the replacement batch.
        self.presented.clear();
        self.presented_dimensions = [0; 2];
        self.prefix = crate::prefix_cache::PrefixCache::default();
        self.dimensions = [doc.width, doc.height];
        self.tiles = bounds(doc.width, doc.height)
            .into_iter()
            .map(|bounds| Tile {
                overview: None,
                level: 1,
                bounds,
                texture: None,
                first: Color32::WHITE,
                signature: Vec::new(),
                rough: false,
            })
            .collect();
        self.pending = vec![None; self.tiles.len()];
        self.levels = vec![1; self.tiles.len()];
        true
    }
    /// Nothing is left but replacing quick tiles with exact ones.
    fn only_refining(&self) -> bool {
        self.composite.as_ref().is_none_or(|job| job.refine)
            && self.generation.is_none()
            && !self.resolution_stale()
            && !self.refresh
            && self.retired.is_none()
            && self.pending.iter().all(Option::is_none)
    }
    pub(crate) fn busy(&self) -> bool {
        self.generation.is_some()
            || self.resolution_stale()
            || self.refresh
            || self.retired.is_some()
            || self.composite.is_some()
            || self.pending.iter().any(Option::is_some)
            || (!self.worker_failed && self.tiles.iter().any(|tile| tile.rough))
    }
    /// Whether the cached path may update resident tiles in place. Refining
    /// quick tiles does not hold it up; the small path drops that job.
    fn cached_path_ready(&self, doc: &Document) -> bool {
        (!self.busy() || self.only_refining())
            && self.dimensions == [doc.width, doc.height]
            && self.tiles.iter().all(|tile| tile.texture.is_some())
            && self
                .tiles
                .iter()
                .all(|tile| metadata_matches(&tile.signature, doc))
    }
    /// Like `update_sliced`, but each damaged rectangle may take the small
    /// cached path on its own. Symmetry copies are far apart (diagonally once
    /// the axes are rotated); one box around them would cover mostly untouched
    /// canvas and force the sliced path to recomposite all of it.
    pub(crate) fn update_damage(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        full: bool,
        damage: crate::render_region::Damage,
        options: egui::TextureOptions,
        performance: &mut Performance,
    ) {
        self.sync_resolution_plan(ctx, doc);
        if self.show_checker && self.checker.is_none() {
            let [light, dark] = CHECKER_COLORS;
            self.checker = Some(ctx.load_texture(
                "transparency-checker",
                egui::ColorImage::new([2, 2], vec![light, dark, dark, light]),
                egui::TextureOptions {
                    magnification: egui::TextureFilter::Nearest,
                    minification: egui::TextureFilter::Nearest,
                    wrap_mode: egui::TextureWrapMode::Repeat,
                    mipmap_mode: None,
                },
            ));
        }
        let rects = damage.rects();
        let footprint: usize = rects.iter().map(|rect| cached_footprint(*rect, doc)).sum();
        if !full
            && rects.len() > 1
            && rects
                .iter()
                .all(|rect| cached_footprint(*rect, doc) <= SMALL_FOOTPRINT)
            && footprint <= SMALL_FOOTPRINT * 4
            && self.cached_path_ready(doc)
        {
            for rect in rects {
                self.update_small(ctx, doc, *rect, options, performance);
            }
            return;
        }
        self.update_sliced(ctx, doc, full, damage.bounds(), options, performance);
    }
    pub(crate) fn update_sliced(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        full: bool,
        dirty: Option<[usize; 4]>,
        options: egui::TextureOptions,
        performance: &mut Performance,
    ) {
        self.sync_resolution_plan(ctx, doc);
        // Keep the existing cached path for small, already-resident brush edits.
        // Bound the expanded prefix footprint as well as the requested rectangle.
        let small = dirty.is_some_and(|rect| cached_footprint(rect, doc) <= SMALL_FOOTPRINT);
        if !full && small && self.cached_path_ready(doc) {
            self.update_small(ctx, doc, dirty.unwrap(), options, performance);
            return;
        }
        self.resize(doc);
        if let Some(worker) = &self.retired {
            performance.display_preparation_peak_bytes = performance
                .display_preparation_peak_bytes
                .max(worker.capacity);
            if !worker.ready() {
                // Do not freeze a successor while retirement prevents any work.
                // Edits made during this wait will enter the eventual source.
                ctx.request_repaint_after(std::time::Duration::from_millis(8));
                return;
            }
            let _ = self.retired.take().unwrap().finish();
        }
        if self.generation.is_none() {
            // Retirement can finish after sync_resolution_plan returned. Start
            // the successor with the current viewport instead of a resize's
            // temporary full-detail default.
            self.levels.clone_from(&self.desired_levels);
            self.generation = Some(doc.clone());
            self.refresh = false;
        }
        // Move the one frozen source into this call without another document clone.
        // Later edits request a successor; they cannot prevent this frame finishing.
        let current = doc;
        let generation = self.generation.take().unwrap();
        let doc = &generation;
        self.refresh |= doc.width != current.width
            || doc.height != current.height
            || doc.layers.len() != current.layers.len()
            || !doc.layers.iter().zip(&current.layers).all(|(a, b)| {
                metadata(a, [0, 0, doc.width, doc.height])
                    == metadata(b, [0, 0, doc.width, doc.height])
                    && a.pixels.shares(&b.pixels)
                    && crate::layer_mask::same_snapshot(&a.mask, &b.mask)
            });
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let in_progress = self.composite.as_ref().is_some_and(|job| {
                (job.tile == index && signature_matches(&job.signature, doc))
                    || job.batch.iter().any(|(tile, _, signature)| {
                        *tile == index && signature_matches(signature, doc)
                    })
            });
            let resolution_changed = tile.level != self.levels[index];
            // A constant texel represents every resolution exactly.
            if resolution_changed && tile.texture.as_ref().is_some_and(|t| t.size() == [1, 1]) {
                tile.level = self.levels[index];
            }
            let resolution_changed = tile.level != self.levels[index];
            let stale = resolution_changed || tile.texture.is_none() || !unchanged(tile, doc);
            let local = if stale
                && !resolution_changed
                && !in_progress
                && tile.texture.is_some()
                && visually_unchanged(tile, doc)
            {
                // Same image under new properties: adopt them without compositing.
                tile.signature = signature(doc, tile.bounds);
                self.pending[index] = None;
                None
            } else if stale && !in_progress {
                Some([0, 0, EXTENT, EXTENT])
            } else {
                None
            };
            if let Some(rect) = local {
                self.pending[index] = Some(self.pending[index].map_or(rect, |old| {
                    [
                        old[0].min(rect[0]),
                        old[1].min(rect[1]),
                        old[2].max(rect[2]),
                        old[3].max(rect[3]),
                    ]
                }));
            }
            if self.options != Some(options)
                && let Some(texture) = &mut tile.texture
            {
                let started = std::time::Instant::now();
                texture.set_partial(
                    [0, 0],
                    egui::ColorImage::filled([1, 1], tile.first),
                    options,
                );
                performance.display_queued_bytes += 4;
                update_overview_options(&mut tile.overview, options, performance);
                performance.upload_ms += started.elapsed().as_secs_f64() * 1000.0;
                performance.display_preparation_peak_bytes =
                    performance.display_preparation_peak_bytes.max(
                        performance.display_queued_bytes
                            + self
                                .composite
                                .as_ref()
                                .map_or(0, |job| job.preparation.capacity_bytes()),
                    );
            }
        }
        if self.options != Some(options) {
            let shared: std::collections::HashSet<_> = self
                .tiles
                .iter()
                .filter_map(|tile| tile.texture.as_ref().map(egui::TextureHandle::id))
                .collect();
            for tile in &mut self.presented {
                if !shared.contains(&tile.texture.id()) {
                    let started = std::time::Instant::now();
                    tile.texture.set_partial(
                        [0, 0],
                        egui::ColorImage::filled([1, 1], tile.first),
                        options,
                    );
                    performance.display_queued_bytes += 4;
                    update_overview_options(&mut tile.overview, options, performance);
                    performance.upload_ms += started.elapsed().as_secs_f64() * 1000.0;
                }
            }
        }
        self.options = Some(options);
        performance.display_preparation_peak_bytes =
            performance.display_preparation_peak_bytes.max(
                performance.display_queued_bytes
                    + self.retired.as_ref().map_or(0, |worker| worker.capacity)
                    + self
                        .composite
                        .as_ref()
                        .map_or(0, |job| job.preparation.capacity_bytes()),
            );
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2);
        let units = 4096;
        #[cfg(test)]
        let units = self.test_units.unwrap_or(units);
        for _ in 0..units {
            if std::time::Instant::now() >= deadline {
                break;
            }
            if self.composite.is_none() {
                // Stale tiles first; once every tile shows the document, quick
                // tiles are replaced with exact ones.
                let use_worker = !self.worker_failed;
                #[cfg(test)]
                let use_worker = use_worker && self.test_units.is_none();
                let (index, refine) = match self.next_pending() {
                    Some(index) => (index, false),
                    None => match self.next_rough(doc) {
                        Some(index) if use_worker => (index, true),
                        _ => break,
                    },
                };
                let full = [0, 0, EXTENT, EXTENT];
                let local = if refine {
                    full
                } else {
                    self.pending[index].take().unwrap()
                };
                // Whole tiles shown reduced are first composited one pixel per
                // shown texel, so a large change appears quickly.
                let step = if !refine && use_worker && local == full {
                    self.levels[index]
                } else {
                    1
                };
                let tile = self.tiles[index].bounds;
                let mut source_rect = tile_source_rect(tile, local, doc);
                // Join neighbours on the same row needing the same work, left
                // then right.
                let mut batch = Vec::new();
                for direction in [-1isize, 1] {
                    let mut next = index as isize + direction;
                    // Quick jobs cost a fraction of an exact one; take more
                    // tiles each, as there is a frame's wait between jobs.
                    while batch.len() + 1 < BATCH * step
                        && let Some(neighbour) =
                            usize::try_from(next).ok().filter(|n| *n < self.tiles.len())
                        && self.tiles[neighbour].bounds[1] == tile[1]
                        && let Some(local) = if refine {
                            self.tiles[neighbour].rough.then_some(full).filter(|_| {
                                self.tiles[neighbour].level == self.levels[neighbour]
                                    && unchanged(&self.tiles[neighbour], doc)
                                    && self.pending[neighbour].is_none()
                            })
                        } else if step > 1 {
                            self.pending[neighbour]
                                .filter(|local| *local == full && self.levels[neighbour] == step)
                                .and_then(|_| self.pending[neighbour].take())
                        } else {
                            self.pending[neighbour].take()
                        }
                    {
                        let bounds = self.tiles[neighbour].bounds;
                        let rect = tile_source_rect(bounds, local, doc);
                        source_rect = [
                            source_rect[0].min(rect[0]),
                            source_rect[1].min(rect[1]),
                            source_rect[2].max(rect[2]),
                            source_rect[3].max(rect[3]),
                        ];
                        batch.push((neighbour, local, signature(doc, bounds)));
                        next += direction;
                    }
                }
                let preparation = if use_worker {
                    match DisplayWorker::start(doc, source_rect, self.transparent, step) {
                        Ok(worker) => Preparation::Worker(worker),
                        Err(error) => {
                            self.worker_failed = true;
                            self.error = Some(format!(
                                "Display worker unavailable; using sliced preparation: {error}"
                            ));
                            Preparation::Cursor(Box::new(
                                crate::composite_cursor::CompositeCursor::new(
                                    doc,
                                    source_rect,
                                    self.transparent,
                                ),
                            ))
                        }
                    }
                } else {
                    Preparation::Cursor(Box::new(crate::composite_cursor::CompositeCursor::new(
                        doc,
                        source_rect,
                        self.transparent,
                    )))
                };
                self.composite = Some(DisplayJob {
                    tile: index,
                    local,
                    source_rect,
                    preparation,
                    signature: signature(doc, tile),
                    step,
                    refine,
                    batch,
                });
            }
            let started = std::time::Instant::now();
            let job = self.composite.as_mut().unwrap();
            let complete = job.preparation.step();
            performance.composite_ms += started.elapsed().as_secs_f64() * 1000.0;
            performance.display_preparation_peak_bytes = performance
                .display_preparation_peak_bytes
                .max(performance.display_queued_bytes + job.preparation.capacity_bytes());
            if !complete {
                if matches!(job.preparation, Preparation::Worker(_)) {
                    break;
                }
                continue;
            }
            let DisplayJob {
                tile: first,
                local: first_local,
                source_rect,
                preparation,
                signature: first_signature,
                step,
                refine: _,
                batch,
            } = self.composite.take().unwrap();
            let (source, worker_ms) = match preparation.finish() {
                Ok(output) => output,
                Err(error) => {
                    self.worker_failed = true;
                    self.error = Some(format!(
                        "Display worker failed; using sliced preparation: {error}"
                    ));
                    self.pending[first] = Some([0, 0, EXTENT, EXTENT]);
                    for (index, _, _) in batch {
                        self.pending[index] = Some([0, 0, EXTENT, EXTENT]);
                    }
                    continue;
                }
            };
            performance.display_worker_ms += worker_ms;
            for (index, local, signature) in
                std::iter::once((first, first_local, first_signature)).chain(batch)
            {
                let tile = &mut self.tiles[index];
                let started = std::time::Instant::now();
                let full_image = sampled_image(doc, tile.bounds, local, source_rect, &source);
                let packing_bytes = full_image.pixels.capacity() * 4;
                tile.overview = if self.viewport.is_some()
                    && doc.width * doc.height >= 2048 * 2048
                    && full_image.size != [1, 1]
                {
                    let overview = reduce_image_ref(&full_image, overview_level(doc));
                    performance.display_queued_bytes += overview.pixels.len() * 4;
                    let first = overview.pixels[0];
                    Some((
                        ctx.load_texture(
                            format!("painting-overview-next-{index}"),
                            overview,
                            options,
                        ),
                        first,
                    ))
                } else {
                    None
                };
                let image = reduce_image(full_image, self.levels[index]);
                tile.level = self.levels[index];
                performance.display_preparation_peak_bytes =
                    performance.display_preparation_peak_bytes.max(
                        performance.display_queued_bytes
                            + source.capacity()
                            + packing_bytes
                            + image.pixels.capacity() * 4,
                    );
                performance.display_queued_bytes += image.pixels.len() * 4;
                if local[0] == 0 && local[1] == 0 {
                    tile.first = image.pixels[0];
                }
                // Never mutate a handle held by the presented generation.
                // Sliced jobs always produce a complete padded tile.
                tile.texture =
                    Some(ctx.load_texture(format!("painting-tile-next-{index}"), image, options));
                tile.signature = signature;
                // A sub-rectangle keeps the rest of a quick tile quick.
                tile.rough = step > 1 || (tile.rough && local != [0, 0, EXTENT, EXTENT]);
                performance.upload_ms += started.elapsed().as_secs_f64() * 1000.0;
            }
        }
        if self.retired.is_none()
            && self.composite.as_ref().is_none_or(|job| job.refine)
            && self.pending.iter().all(Option::is_none)
            && self.tiles.iter().enumerate().all(|(index, tile)| {
                tile.texture.is_some() && tile.level == self.levels[index] && unchanged(tile, doc)
            })
        {
            self.publish();
        } else {
            self.generation = Some(generation);
        }
        if self.busy() {
            if self.waiting() {
                ctx.request_repaint_after(std::time::Duration::from_millis(8));
            } else {
                ctx.request_repaint();
            }
        }
    }
    pub(crate) fn cache_stats(&self) -> crate::prefix_cache::Stats {
        self.prefix.stats
    }
    pub(crate) fn memory(&self) -> Memory {
        let mut memory = Memory {
            metadata_heap_bytes: (self.levels.capacity() + self.desired_levels.capacity())
                * std::mem::size_of::<usize>()
                + self.tiles.capacity() * std::mem::size_of::<Tile>()
                + self.presented.capacity() * std::mem::size_of::<PresentedTile>()
                + self.pending.capacity() * std::mem::size_of::<Option<[usize; 4]>>()
                + self.composite.as_ref().map_or(0, |job| {
                    job.signature.capacity() * std::mem::size_of::<LayerSignature>()
                        + job
                            .signature
                            .iter()
                            .map(|source| source.pixels.heap_bytes() + source.mask.heap_bytes())
                            .sum::<usize>()
                }),
            prefix_heap_bytes: self.prefix.retained_bytes(),
            ..Default::default()
        };
        memory.metadata_heap_bytes += self
            .retired
            .as_ref()
            .map_or(0, |worker| worker.snapshot.descriptor_bytes())
            + self
                .composite
                .as_ref()
                .map_or(0, |job| match &job.preparation {
                    Preparation::Cursor(cursor) => cursor.snapshot.descriptor_bytes(),
                    Preparation::Worker(worker) => worker.snapshot.descriptor_bytes(),
                });
        let mut seen = std::collections::HashSet::new();
        for tile in &self.tiles {
            memory.metadata_heap_bytes += tile.signature.capacity()
                * std::mem::size_of::<LayerSignature>()
                + tile
                    .signature
                    .iter()
                    .map(|source| source.pixels.heap_bytes() + source.mask.heap_bytes())
                    .sum::<usize>();
            if let Some(texture) = &tile.texture {
                seen.insert(texture.id());
                memory.resident_tiles += 1;
                let [mut width, mut height] = texture.size();
                memory.texture_base_rgba_bytes += width * height * 4;
                memory.texture_with_mips_rgba_estimate_bytes += width * height * 4;
                if self
                    .options
                    .is_some_and(|options| options.mipmap_mode.is_some())
                {
                    while width > 1 || height > 1 {
                        width = (width / 2).max(1);
                        height = (height / 2).max(1);
                        memory.texture_with_mips_rgba_estimate_bytes += width * height * 4;
                    }
                }
            }
        }
        for tile in &self.presented {
            if !seen.insert(tile.texture.id()) {
                continue;
            }
            memory.resident_tiles += 1;
            let [mut width, mut height] = tile.texture.size();
            memory.texture_base_rgba_bytes += width * height * 4;
            memory.texture_with_mips_rgba_estimate_bytes += width * height * 4;
            if self
                .options
                .is_some_and(|options| options.mipmap_mode.is_some())
            {
                while width > 1 || height > 1 {
                    width = (width / 2).max(1);
                    height = (height / 2).max(1);
                    memory.texture_with_mips_rgba_estimate_bytes += width * height * 4;
                }
            }
        }
        for (texture, _) in self
            .tiles
            .iter()
            .filter_map(|t| t.overview.as_ref())
            .chain(self.presented.iter().filter_map(|t| t.overview.as_ref()))
        {
            if !seen.insert(texture.id()) {
                continue;
            }
            memory.resident_tiles += 1;
            let [mut width, mut height] = texture.size();
            memory.texture_base_rgba_bytes += width * height * 4;
            memory.texture_with_mips_rgba_estimate_bytes += width * height * 4;
            if self.options.is_some_and(|o| o.mipmap_mode.is_some()) {
                while width > 1 || height > 1 {
                    width = (width / 2).max(1);
                    height = (height / 2).max(1);
                    memory.texture_with_mips_rgba_estimate_bytes += width * height * 4;
                }
            }
        }
        memory
    }
    /// Patch reduced textures on their own sampling grid. The 16px overview
    /// grid also contains every 1/2/4px primary grid, so one bounded composite
    /// supplies both images without filtering a previously reduced texture.
    fn update_small(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        dirty: [usize; 4],
        options: egui::TextureOptions,
        performance: &mut Performance,
    ) {
        if self.full_resolution_damage(doc, dirty) {
            self.update(ctx, doc, false, Some(dirty), options, performance);
            return;
        }
        debug_assert!(self.cached_path_ready(doc));
        // A refinement composited from an older document would overwrite
        // this edit; drop it, and its tiles stay quick until the next pass.
        if self.composite.as_ref().is_some_and(|job| job.refine) {
            self.composite = None;
        }
        let options_changed = self.options != Some(options);
        let overview_scale = overview_level(doc);
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let Some(local) = dirty_rect(tile.bounds, dirty, doc.width, doc.height) else {
                if options_changed {
                    tile.texture.as_mut().unwrap().set_partial(
                        [0, 0],
                        egui::ColorImage::filled([1, 1], tile.first),
                        options,
                    );
                    performance.display_queued_bytes += 4;
                    update_overview_options(&mut tile.overview, options, performance);
                }
                continue;
            };
            let started = std::time::Instant::now();
            let compact = tile.texture.as_ref().unwrap().size() == [1, 1];
            let first = tile.first;
            // A legacy/full-resolution source can predate navigator allocation.
            // Reconstruct it once rather than inventing its missing background.
            let local = if tile.overview.is_none() && !compact {
                [0, 0, EXTENT, EXTENT]
            } else {
                let grid = overview_scale.max(tile.level);
                [
                    local[0] / grid * grid,
                    local[1] / grid * grid,
                    local[2].div_ceil(grid) * grid,
                    local[3].div_ceil(grid) * grid,
                ]
            };
            let source = tile_source_rect(tile.bounds, local, doc);
            let pixels = if doc.active >= 4 {
                self.prefix.composite_region(doc, source, self.transparent)
            } else {
                doc.composite_display_region(source, self.transparent)
            };
            let patch = sampled_image(doc, tile.bounds, local, source, &pixels);
            performance.composite_ms += started.elapsed().as_secs_f64() * 1000.0;
            let started = std::time::Instant::now();
            let scratch = pixels.capacity() + patch.pixels.capacity() * 4;
            let nav_local = local.map(|v| v / overview_scale);
            let nav = reduce_image_ref(&patch, overview_scale);
            let (nav, nav_replace) = prepare_scaled_patch(
                tile.overview
                    .as_ref()
                    .is_none_or(|(texture, _)| texture.size() == [1, 1]),
                tile.overview.as_ref().map_or(first, |(_, color)| *color),
                EXTENT / overview_scale,
                nav_local,
                nav,
            );
            // Tiny marks can survive primary reduction but average to the
            // old navigator color. Retain even that constant overview instead
            // of falling back to differently sampled primary pixels.
            if tile.overview.is_some() || patch.pixels.iter().any(|p| *p != first) {
                performance.display_preparation_peak_bytes = performance
                    .display_preparation_peak_bytes
                    .max(performance.display_queued_bytes + scratch + nav.pixels.capacity() * 4);
                performance.display_queued_bytes += nav.pixels.len() * 4;
                if let Some((texture, nav_first)) = &mut tile.overview {
                    if nav_replace || nav_local[..2] == [0, 0] {
                        *nav_first = nav.pixels[0];
                    }
                    if nav_replace {
                        texture.set(nav, options);
                    } else {
                        texture.set_partial([nav_local[0], nav_local[1]], nav, options);
                    }
                } else {
                    let nav_first = nav.pixels[0];
                    tile.overview = Some((
                        ctx.load_texture(format!("painting-overview-{index}"), nav, options),
                        nav_first,
                    ));
                }
            }
            let scaled_local = local.map(|v| v / tile.level);
            let image = reduce_image(patch, tile.level);
            let (image, replace) =
                prepare_scaled_patch(compact, first, EXTENT / tile.level, scaled_local, image);
            performance.display_preparation_peak_bytes = performance
                .display_preparation_peak_bytes
                .max(performance.display_queued_bytes + scratch + image.pixels.capacity() * 4);
            performance.display_queued_bytes += image.pixels.len() * 4;
            if replace || scaled_local[..2] == [0, 0] {
                tile.first = image.pixels[0];
            }
            let texture = tile.texture.as_mut().unwrap();
            if replace {
                texture.set(image, options);
            } else {
                texture.set_partial([scaled_local[0], scaled_local[1]], image, options);
            }
            performance.upload_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        // Damage is complete; shared COW stamps may also change outside it.
        for tile in &mut self.tiles {
            tile.signature = signature(doc, tile.bounds);
        }
        self.options = Some(options);
        self.publish();
    }

    pub fn update(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        full: bool,
        dirty: Option<[usize; 4]>,
        options: egui::TextureOptions,
        performance: &mut Performance,
    ) {
        let full = full || self.busy();
        if full {
            self.levels.fill(1);
        }
        let resized = self.resize(doc);
        self.retire();
        self.generation = None;
        self.refresh = false;
        self.pending.fill(None);
        // Shared gutters may upload the same changed pixels to several tiles;
        // composite the dirty document rectangle only once for those copies.
        let shared = if !full && !resized {
            dirty.and_then(|[left, top, right, bottom]| {
                let rect = [
                    left.min(doc.width),
                    top.min(doc.height),
                    right.min(doc.width),
                    bottom.min(doc.height),
                ];
                if rect[0] >= rect[2] || rect[1] >= rect[3] {
                    return None;
                }
                let start = std::time::Instant::now();
                let pixels = if doc.active >= 4 {
                    self.prefix.composite_region(doc, rect, self.transparent)
                } else {
                    doc.composite_display_region(rect, self.transparent)
                };
                performance.composite_ms += start.elapsed().as_secs_f64() * 1000.0;
                Some((rect, pixels))
            })
        } else {
            None
        };
        let options_changed = self.options != Some(options);
        let complete_partial_damage = !full
            && dirty.is_some()
            && self
                .tiles
                .iter()
                .all(|tile| metadata_matches(&tile.signature, doc));
        // A bounded horizontal strip shares compositing across a row of tiles.
        // Avoid rebuilding overlapping gutters or allocating a full-canvas image.
        let mut row_composite = None;
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let local = if tile.texture.is_some() && full && unchanged(tile, doc) {
                None
            } else if full || tile.texture.is_none() {
                Some([0, 0, EXTENT, EXTENT])
            } else {
                dirty.and_then(|dirty| dirty_rect(tile.bounds, dirty, doc.width, doc.height))
            };
            if let Some(local) = local {
                let composite_start = std::time::Instant::now();
                let (image, scratch_bytes) = if full || tile.texture.is_none() {
                    if row_composite.as_ref().is_none_or(
                        |(top, _, _): &(usize, [usize; 4], Vec<u8>)| *top != tile.bounds[1],
                    ) {
                        let source = [
                            0,
                            tile.bounds[1].saturating_sub(PAD),
                            doc.width,
                            (tile.bounds[1] + SIDE + PAD).min(doc.height),
                        ];
                        row_composite = Some((
                            tile.bounds[1],
                            source,
                            doc.composite_display_region(source, self.transparent),
                        ));
                    }
                    let (_, source, composite) = row_composite.as_ref().unwrap();
                    (
                        sampled_image(doc, tile.bounds, local, *source, composite),
                        0,
                    )
                } else if tile.texture.is_some()
                    && let Some((source, composite)) = &shared
                {
                    (
                        sampled_image(doc, tile.bounds, local, *source, composite),
                        0,
                    )
                } else {
                    let pixels = rgba_display(doc, tile.bounds, local, self.transparent);
                    (tile_image(local, &pixels), pixels.capacity())
                };
                performance.composite_ms += composite_start.elapsed().as_secs_f64() * 1000.0;
                let upload_start = std::time::Instant::now();
                tile.level = 1;
                let patch_bytes = image.pixels.capacity() * 4;
                let (image, replace) = prepare_tile_image(tile, local, image);
                if self.viewport.is_some() && doc.width * doc.height >= 2048 * 2048 {
                    if replace {
                        tile.overview = if image.size == [1, 1] {
                            None
                        } else {
                            let overview = reduce_image_ref(&image, overview_level(doc));
                            performance.display_queued_bytes += overview.pixels.len() * 4;
                            let first = overview.pixels[0];
                            Some((
                                ctx.load_texture(
                                    format!("painting-overview-{index}"),
                                    overview,
                                    options,
                                ),
                                first,
                            ))
                        };
                    } else if let Some((texture, first)) = &mut tile.overview {
                        let level = overview_level(doc);
                        let aligned = [
                            local[0] / level * level,
                            local[1] / level * level,
                            local[2].div_ceil(level) * level,
                            local[3].div_ceil(level) * level,
                        ];
                        let source = tile_source_rect(tile.bounds, aligned, doc);
                        let pixels = if doc.active >= 4 {
                            self.prefix.composite_region(doc, source, self.transparent)
                        } else {
                            doc.composite_display_region(source, self.transparent)
                        };
                        let patch = sampled_image(doc, tile.bounds, aligned, source, &pixels);
                        let patch_bytes = patch.pixels.capacity() * 4;
                        let reduced = reduce_image(patch, level);
                        performance.display_preparation_peak_bytes =
                            performance.display_preparation_peak_bytes.max(
                                performance.display_queued_bytes
                                    + pixels.capacity()
                                    + patch_bytes
                                    + reduced.pixels.capacity() * 4,
                            );
                        if aligned[0] == 0 && aligned[1] == 0 {
                            *first = reduced.pixels[0];
                        }
                        performance.display_queued_bytes += reduced.pixels.len() * 4;
                        texture.set_partial(
                            [aligned[0] / level, aligned[1] / level],
                            reduced,
                            options,
                        );
                    }
                } else {
                    tile.overview = None;
                }
                let image_bytes = image.pixels.capacity() * std::mem::size_of::<Color32>();
                let shared_bytes = shared.as_ref().map_or(0, |(_, pixels)| pixels.capacity());
                let strip_bytes = row_composite
                    .as_ref()
                    .map_or(0, |(_, _, pixels)| pixels.capacity());
                performance.display_preparation_peak_bytes =
                    performance.display_preparation_peak_bytes.max(
                        performance.display_queued_bytes
                            + shared_bytes
                            + strip_bytes
                            + scratch_bytes
                            + image_bytes
                            + patch_bytes,
                    );
                performance.display_queued_bytes +=
                    image.pixels.len() * std::mem::size_of::<Color32>();
                if replace || (local[0] == 0 && local[1] == 0) {
                    tile.first = image.pixels[0];
                }
                if let Some(texture) = &mut tile.texture {
                    if replace {
                        texture.set(image, options);
                    } else {
                        texture.set_partial([local[0], local[1]], image, options);
                    }
                } else {
                    tile.texture =
                        Some(ctx.load_texture(format!("painting-tile-{index}"), image, options));
                }
                performance.upload_ms += upload_start.elapsed().as_secs_f64() * 1000.0;
                tile.signature = signature(doc, tile.bounds);
            } else if options_changed && let Some(texture) = &mut tile.texture {
                let start = std::time::Instant::now();
                texture.set_partial(
                    [0, 0],
                    egui::ColorImage::filled([1, 1], tile.first),
                    options,
                );
                performance.display_queued_bytes += std::mem::size_of::<Color32>();
                update_overview_options(&mut tile.overview, options, performance);
                performance.display_preparation_peak_bytes =
                    performance.display_preparation_peak_bytes.max(
                        performance.display_queued_bytes
                            + shared.as_ref().map_or(0, |(_, pixels)| pixels.capacity())
                            + row_composite
                                .as_ref()
                                .map_or(0, |(_, _, pixels)| pixels.capacity()),
                    );
                performance.upload_ms += start.elapsed().as_secs_f64() * 1000.0;
            }
        }
        self.options = Some(options);
        if complete_partial_damage {
            // The supplied rectangle describes all pixel changes. Coarse COW
            // identities can change for neighboring tiles whose actual sampled
            // pixels did not; acknowledge those unchanged regions too. Otherwise
            // the next idle update starts an unnecessary full-tile batch and holds
            // later brush damage behind it.
            for tile in &mut self.tiles {
                tile.signature = signature(doc, tile.bounds);
            }
        }
        // This synchronous path completes all damaged tiles within this call.
        self.publish();
    }
    #[cfg(test)]
    pub fn paint(&self, painter: &egui::Painter, area: Rect, view: View) {
        painter.extend(self.shapes(area, view));
    }
    #[cfg(test)]
    pub(crate) fn shapes(&self, area: Rect, view: View) -> Vec<egui::Shape> {
        let mut shapes = self.backdrop_shapes(area, view);
        shapes.extend(self.tile_shapes(area, view));
        shapes
    }
    /// Screen quads of the presented tiles that intersect `area`, with their
    /// document bounds.
    fn visible_tiles(
        &self,
        area: Rect,
        view: View,
    ) -> impl Iterator<Item = (&PresentedTile, [Pos2; 4])> {
        let presented = if self.presented_dimensions == self.dimensions {
            &self.presented[..]
        } else {
            &[]
        };
        presented.iter().filter_map(move |tile| {
            let [left, top, right, bottom] = tile.bounds;
            let corners = [
                pos2(left as f32, top as f32),
                pos2(right as f32, top as f32),
                pos2(right as f32, bottom as f32),
                pos2(left as f32, bottom as f32),
            ]
            .map(|point| view.to_screen(point));
            Rect::from_points(&corners)
                .intersects(area)
                .then_some((tile, corners))
        })
    }
    /// What shows through transparent artwork: the checkerboard, or white
    /// paper when alpha tiles are only needed for references below the art.
    /// Opaque paper tiles need no backdrop.
    pub(crate) fn backdrop_shapes(&self, area: Rect, view: View) -> Vec<egui::Shape> {
        if !self.presented_transparent {
            return Vec::new();
        }
        // Constant on-screen squares that rotate with the canvas.
        let Some(checker) = self.checker.as_ref().filter(|_| self.show_checker) else {
            // One paper quad avoids antialiased internal tile edges showing
            // gray seams through alpha artwork / below-artwork references.
            return if self.visible_tiles(area, view).next().is_some() {
                vec![egui::Shape::convex_polygon(
                    view.corners().to_vec(),
                    Color32::WHITE,
                    egui::Stroke::NONE,
                )]
            } else {
                Vec::new()
            };
        };
        let scale = (view.to_screen(pos2(1.0, 0.0)) - view.to_screen(pos2(0.0, 0.0))).length()
            / (2.0 * CHECKER_POINTS);
        self.visible_tiles(area, view)
            .map(|(tile, corners)| {
                let [left, top, right, bottom] = tile.bounds.map(|v| v as f32);
                let points = [
                    pos2(left, top),
                    pos2(right, top),
                    pos2(right, bottom),
                    pos2(left, bottom),
                ];
                let mut mesh = egui::Mesh::with_texture(checker.id());
                for (pos, point) in corners.into_iter().zip(points) {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos,
                        uv: pos2(point.x * scale, point.y * scale),
                        color: Color32::WHITE,
                    });
                }
                mesh.indices = vec![0, 1, 2, 0, 2, 3];
                egui::Shape::mesh(mesh)
            })
            .collect()
    }
    pub(crate) fn tile_shapes(&self, area: Rect, view: View) -> Vec<egui::Shape> {
        self.tile_shapes_for(area, view, false)
    }
    pub(crate) fn overview_shapes(&self, area: Rect, view: View) -> Vec<egui::Shape> {
        self.tile_shapes_for(area, view, true)
    }
    fn tile_shapes_for(&self, area: Rect, view: View, overview: bool) -> Vec<egui::Shape> {
        self.visible_tiles(area, view)
            .map(|(tile, corners)| {
                let [left, top, right, bottom] = tile.bounds;
                let uv0 = PAD as f32 / EXTENT as f32;
                let u1 = (PAD + right - left) as f32 / EXTENT as f32;
                let v1 = (PAD + bottom - top) as f32 / EXTENT as f32;
                let uvs = [pos2(uv0, uv0), pos2(u1, uv0), pos2(u1, v1), pos2(uv0, v1)];
                let texture = if overview {
                    tile.overview.as_ref().map_or(&tile.texture, |(t, _)| t)
                } else {
                    &tile.texture
                };
                let mut mesh = egui::Mesh::with_texture(texture.id());
                for (pos, uv) in corners.into_iter().zip(uvs) {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos,
                        uv,
                        color: Color32::WHITE,
                    });
                }
                mesh.indices = vec![0, 1, 2, 0, 2, 3];
                egui::Shape::mesh(mesh)
            })
            .collect()
    }
}

/// Side of one transparency checkerboard square, in screen points.
const CHECKER_POINTS: f32 = 8.0;
/// Light and dark squares, as in common painting applications.
const CHECKER_COLORS: [Color32; 2] = [Color32::from_gray(255), Color32::from_gray(204)];

/// Largest prefix-aligned area the cached display path updates in place.
const SMALL_FOOTPRINT: usize = 16_384;
/// Area a dirty rectangle covers once expanded to the 16px prefix grid;
/// invalid rectangles never qualify for the cached path.
fn cached_footprint([left, top, right, bottom]: [usize; 4], doc: &Document) -> usize {
    if left >= right || top >= bottom || right > doc.width || bottom > doc.height {
        return usize::MAX;
    }
    let width = right.div_ceil(16) * 16 - left / 16 * 16;
    let height = bottom.div_ceil(16) * 16 - top / 16 * 16;
    width * height
}

fn retain_images(
    retained: &mut std::collections::HashMap<egui::TextureId, egui::ColorImage>,
    delta: &egui::TexturesDelta,
) -> Result<usize, String> {
    let mut bytes = 0;
    for (id, delta) in &delta.set {
        if *id == egui::TextureId::Managed(0) {
            continue;
        }
        let egui::ImageData::Color(image) = &delta.image;
        bytes += image.pixels.len() * 4;
        if let Some([x, y]) = delta.pos {
            let old = retained
                .get_mut(id)
                .ok_or("Partial update lacks original texture")?;
            for row in 0..image.size[1] {
                let start = (y + row) * old.size[0] + x;
                old.pixels[start..start + image.size[0]]
                    .copy_from_slice(&image.pixels[row * image.size[0]..(row + 1) * image.size[0]]);
            }
        } else {
            // Retain logical padded pixels for the compositor oracle. A 1x1
            // full replacement is constant over the entire sampled tile.
            retained.insert(
                *id,
                if image.size == [1, 1] {
                    egui::ColorImage::filled([EXTENT, EXTENT], image.pixels[0])
                } else {
                    image.as_ref().clone()
                },
            );
        }
    }
    for id in &delta.free {
        retained.remove(id);
    }
    Ok(bytes)
}

pub(crate) fn benchmark_live(path: &std::path::Path) -> Result<(), String> {
    benchmark_live_inner(path, false)
}
pub(crate) fn benchmark_live_motion(path: &std::path::Path) -> Result<(), String> {
    benchmark_live_inner(path, true)
}
fn benchmark_live_inner(path: &std::path::Path, moving: bool) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        for tiled in [false, true] {
            for layers in [1usize, 8, 32] {
                for damage in [18usize, 80] {
                    let mut doc = Document::new(size, size);
                    doc.layers[0].pixels = if tiled {
                        crate::pixels::Pixels::read_rgba(
                            &mut [17, 83, 201, 128].repeat(size * size).as_slice(),
                            size,
                            size,
                        )
                        .map_err(|error| error.to_string())?
                    } else {
                        [17, 83, 201, 128].repeat(size * size).into()
                    };
                    for _ in 1..layers {
                        doc.add_layer();
                    }
                    let rect = [
                        192,
                        192,
                        192 + damage + if moving { 23 * 3 } else { 0 },
                        192 + damage,
                    ];
                    doc.layers[doc.active].pixels.writer().write_region(
                        size,
                        rect,
                        &[0, 83, 201, 255].repeat((rect[2] - rect[0]) * damage),
                    );
                    let ctx = egui::Context::default();
                    let mut textures = CanvasTextures::default();
                    let mut images = std::collections::HashMap::new();
                    let first = ctx.run(Default::default(), |ctx| {
                        textures.update(
                            ctx,
                            &doc,
                            true,
                            None,
                            egui::TextureOptions::LINEAR,
                            &mut Performance::default(),
                        );
                    });
                    retain_images(&mut images, &first.textures_delta)?;
                    let mut immediate = 0usize;
                    let mut lag_peak = 0usize;
                    let mut max_owner_ms = 0.0f64;
                    let mut emitted_bytes = 0usize;
                    let mut idle_batches = 0usize;
                    let mut owner_samples = Vec::new();
                    for frame in 1..=24usize {
                        let left = 192 + if moving { (frame - 1) * 3 } else { 0 };
                        let rect = [left, 192, left + damage, 192 + damage];
                        let point = [left + damage / 2, 192 + damage / 2];
                        doc.layers[doc.active].pixels.writer().write_region(
                            size,
                            rect,
                            &[frame as u8, 83, 201, 255].repeat(damage * damage),
                        );
                        for idle in [false, true] {
                            let mut performance = Performance::default();
                            let mut update_ms = 0.0;
                            let mut update_calls = 0usize;
                            let started = std::time::Instant::now();
                            let output = ctx.run(Default::default(), |ctx| {
                                let update_started = std::time::Instant::now();
                                textures.update_sliced(
                                    ctx,
                                    &doc,
                                    false,
                                    (!idle).then_some(rect),
                                    egui::TextureOptions::LINEAR,
                                    &mut performance,
                                );
                                update_ms += update_started.elapsed().as_secs_f64() * 1000.0;
                                update_calls += 1;
                            });
                            let owner_ms = started.elapsed().as_secs_f64() * 1000.0;
                            max_owner_ms = max_owner_ms.max(owner_ms);
                            let cache_bytes = textures.prefix.retained_bytes();
                            if cache_bytes > crate::prefix_cache::BUDGET_MIB * 1024 * 1024 {
                                return Err("Live display prefix cache exceeds byte budget".into());
                            }
                            owner_samples.push(serde_json::json!({"input_update":frame,
                                "phase":if idle { "idle" } else { "input" },
                                "owner_ms":owner_ms,"update_ms":update_ms,"update_calls":update_calls,
                                "composite_ms":performance.composite_ms,"upload_ms":performance.upload_ms,
                                "update_residual_ms":(update_ms-performance.composite_ms-performance.upload_ms).max(0.0),
                                "context_outside_update_ms":(owner_ms-update_ms).max(0.0),
                                "prefix_retained_bytes":cache_bytes,"queued_rgba_bytes":performance.display_queued_bytes,
                                "busy":textures.busy()}));
                            let bytes = retain_images(&mut images, &output.textures_delta)?;
                            if bytes != performance.display_queued_bytes {
                                return Err("Live display payload accounting mismatch".into());
                            }
                            emitted_bytes += bytes;
                            if idle {
                                idle_batches += usize::from(textures.busy());
                            } else {
                                let tile = textures
                                    .presented
                                    .iter()
                                    .find(|tile| {
                                        tile.bounds[0] <= point[0]
                                            && tile.bounds[1] <= point[1]
                                            && point[0] < tile.bounds[2]
                                            && point[1] < tile.bounds[3]
                                    })
                                    .ok_or("Missing live presented tile")?;
                                let index = (point[1] - tile.bounds[1] + PAD) * EXTENT + point[0]
                                    - tile.bounds[0]
                                    + PAD;
                                let shown = images
                                    .get(&tile.texture.id())
                                    .ok_or("Missing live image")?
                                    .pixels[index]
                                    .r() as usize;
                                if shown > frame {
                                    return Err("Display shows future input".into());
                                }
                                lag_peak = lag_peak.max(frame - shown);
                                immediate += usize::from(shown == frame);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(8));
                    }
                    let mut drained = false;
                    for _ in 0..3000 {
                        let output = ctx.run(Default::default(), |ctx| {
                            textures.update_sliced(
                                ctx,
                                &doc,
                                false,
                                None,
                                egui::TextureOptions::LINEAR,
                                &mut Performance::default(),
                            );
                        });
                        retain_images(&mut images, &output.textures_delta)?;
                        if !textures.busy() {
                            drained = true;
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    if !drained {
                        return Err("Live display failed to drain".into());
                    }
                    for tile in &textures.presented {
                        let expected = egui::ColorImage::from_rgba_unmultiplied(
                            [EXTENT, EXTENT],
                            &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
                        );
                        if images.get(&tile.texture.id()) != Some(&expected) {
                            return Err("Live display final pixels mismatch".into());
                        }
                    }
                    cases.push(
                        serde_json::json!({"size":size,"tiled":tiled,"layers":layers,
                        "damage_side":damage,"input_updates":24,"immediate_updates":immediate,
                        "display_age_input_updates_peak":lag_peak,"busy_idle_updates":idle_batches,
                        "max_owner_ms":max_owner_ms,"emitted_rgba_bytes":emitted_bytes,
                        "owner_samples":owner_samples}),
                    );
                }
            }
        }
    }
    std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"version":3,
        "motion_step_pixels":if moving { 3 } else { 0 },
        "profile_scope":"Each input/idle call records total Context::run wall, summed display callback wall, callback count, existing composite and upload wall timers, residual update wall, context wall outside the callback, retained prefix capacity and queued image bytes. Residual includes signature/metadata checks, snapshot controls, uninstrumented sampling, publication and cleanup, not an isolated cause. Timers include OS preemption; no CPU/GPU/pen latency attribution. Sampling/serialization/accounting/oracles are outside owner timing. Prefix retention must remain within its existing byte budget on every call.",
        "scope":"Supported 512/2048 documents, one dense flat/tiled base and sparse extra layers. 24 opaque region changes with one paint and one idle display update per change, followed by an 8 ms sleep. A center marker reports logical input-update age and same-call visibility; not physical tablet latency, UI frame rate or GPU rendering. Initial upload/setup, writes/COW, validation and sleep excluded from owner timings. Production workers are used. Final padded pixels and input-update payload accounting verified. One sequential run per case; no statistical speedup or hard deadline claim.","cases":cases}))
        .map_err(|error| error.to_string())?).map_err(|error| error.to_string())
}

pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        for layers in [1usize, 8, 32] {
            for sliced in [false, true] {
                let mut doc = Document::new(size, size);
                doc.layers[0].pixels = [17, 83, 201, 128].repeat(size * size).into();
                for index in 1..layers {
                    doc.add_layer();
                    doc.layers[index]
                        .pixels
                        .writer()
                        .rgba_mut(index * size + index)
                        .copy_from_slice(&[201, 83, 17, 127]);
                }
                doc.validate()?;
                let ctx = egui::Context::default();
                let mut textures = CanvasTextures::default();
                let mut retained =
                    std::collections::HashMap::<egui::TextureId, egui::ColorImage>::new();
                let options = egui::TextureOptions {
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                    ..egui::TextureOptions::LINEAR
                };
                for phase in ["cold", "unchanged", "dirty-prefix-pressure", "resize"] {
                    let previous_ids: Vec<_> = textures
                        .presented
                        .iter()
                        .map(|tile| tile.texture.id())
                        .collect();
                    let dirty = if phase == "dirty-prefix-pressure" {
                        doc.layers[doc.active].pixels.writer().write_region(
                            size,
                            [240, 240, 496, 496],
                            &[201, 17, 83, 255].repeat(256 * 256),
                        );
                        Some([240, 240, 496, 496])
                    } else {
                        None
                    };
                    if phase == "resize" {
                        doc = Document::new(257, 33);
                    }
                    let mut calls = 0usize;
                    let mut update_ms = 0.0f64;
                    let mut max_call_ms = 0.0f64;
                    let mut preparation_peak_bytes = 0usize;
                    let mut emitted_bytes = 0usize;
                    let mut completed_worker_ms = 0.0f64;
                    let mut snapshot_pixel_peak = 0usize;
                    let mut snapshot_metadata_peak = 0usize;
                    let mut resident_tiles_peak = 0usize;
                    let mut texture_base_peak = 0usize;
                    let mut texture_mips_peak = 0usize;
                    let mut coherent_publications = 0usize;
                    let phase_started = std::time::Instant::now();
                    loop {
                        let mut performance = Performance::default();
                        let started = std::time::Instant::now();
                        let output = ctx.run(Default::default(), |ctx| {
                            if sliced {
                                textures.update_sliced(
                                    ctx,
                                    &doc,
                                    calls == 0 && phase != "dirty-prefix-pressure",
                                    if calls == 0 { dirty } else { None },
                                    options,
                                    &mut performance,
                                );
                            } else {
                                textures.update(
                                    ctx,
                                    &doc,
                                    phase != "dirty-prefix-pressure",
                                    dirty,
                                    options,
                                    &mut performance,
                                );
                            }
                        });
                        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                        update_ms += elapsed;
                        max_call_ms = max_call_ms.max(elapsed);
                        preparation_peak_bytes =
                            preparation_peak_bytes.max(performance.display_preparation_peak_bytes);
                        completed_worker_ms += performance.display_worker_ms;
                        let snapshot = textures.snapshot_memory(&doc);
                        snapshot_pixel_peak =
                            snapshot_pixel_peak.max(snapshot.pixel_bytes_beyond_live);
                        snapshot_metadata_peak =
                            snapshot_metadata_peak.max(snapshot.metadata_bytes);
                        if let Some(error) = textures.take_error() {
                            return Err(error);
                        }
                        let bytes = retain_images(&mut retained, &output.textures_delta)?;
                        if bytes != performance.display_queued_bytes {
                            return Err(
                                "Display accounting differs from emitted texture payload".into()
                            );
                        }
                        emitted_bytes += bytes;
                        let memory = textures.memory();
                        resident_tiles_peak = resident_tiles_peak.max(memory.resident_tiles);
                        texture_base_peak = texture_base_peak.max(memory.texture_base_rgba_bytes);
                        texture_mips_peak =
                            texture_mips_peak.max(memory.texture_with_mips_rgba_estimate_bytes);
                        let shown: Vec<_> = textures
                            .presented
                            .iter()
                            .map(|tile| tile.texture.id())
                            .collect();
                        if shown.is_empty() && !previous_ids.is_empty() && phase != "resize" {
                            return Err("Display dropped a drawable previous frame".into());
                        }
                        if phase == "resize"
                            && calls == 0
                            && previous_ids.iter().any(|id| retained.contains_key(id))
                        {
                            return Err("Resize retained invisible display textures".into());
                        }
                        if !shown.is_empty() && shown != previous_ids {
                            if textures.presented_dimensions != [doc.width, doc.height]
                                || textures.presented.len() != textures.tiles.len()
                                || textures.presented.iter().zip(&textures.tiles).any(
                                    |(shown, tile)| {
                                        tile.texture.as_ref().map(egui::TextureHandle::id)
                                            != Some(shown.texture.id())
                                    },
                                )
                            {
                                return Err("Display published an incomplete generation".into());
                            }
                            coherent_publications += 1;
                        }
                        calls += 1;
                        if !textures.busy() {
                            break;
                        }
                        if textures.waiting() {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        if calls > 100_000 {
                            return Err("Display did not drain".into());
                        }
                    }
                    let drain_wall_ms = phase_started.elapsed().as_secs_f64() * 1000.0;
                    for tile in &textures.presented {
                        let expected = egui::ColorImage::from_rgba_unmultiplied(
                            [EXTENT, EXTENT],
                            &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
                        );
                        let id = tile.texture.id();
                        if retained.get(&id) != Some(&expected) {
                            return Err("Retained display pixels differ from compositor".into());
                        }
                    }
                    let memory = textures.memory();
                    if memory.prefix_heap_bytes > crate::prefix_cache::BUDGET_MIB * 1024 * 1024 {
                        return Err("Composite state cache exceeds byte budget".into());
                    }
                    cases.push(
                    serde_json::json!({"original_size":size, "layers":layers, "phase":phase, "sliced":sliced,
                    "calls":calls, "max_call_ms":max_call_ms,
                    "pipeline":if sliced { "background" } else { "synchronous" },
                    "drain_wall_ms":drain_wall_ms, "completed_worker_ms":completed_worker_ms,
                    "snapshot_pixel_bytes_beyond_live_peak":snapshot_pixel_peak,
                    "snapshot_metadata_bytes_peak":snapshot_metadata_peak,
                    "sampled_resident_tiles_peak":resident_tiles_peak,
                    "sampled_texture_base_rgba_bytes_peak":texture_base_peak,
                    "sampled_texture_with_mips_rgba_estimate_bytes_peak":texture_mips_peak,
                    "coherent_publication_checks":coherent_publications,
                    "update_ms":update_ms, "memory":memory, "emitted_rgba_bytes":emitted_bytes,
                    "preparation_peak_bytes":preparation_peak_bytes}),
                );
                }
            }
        }
    }
    let report = serde_json::json!({"version":3, "scope":"Supported 512/2048-square documents: one dense flat base plus sparse additional layers, 1/8/32 layers; eager padded display shared with navigator. Synchronous comparator and production background tile compositor using existing parallel pixel bands; sliced cursor is failure fallback. Owner 2 ms allowance checked between controls/publications, which can overrun. One sequential drain per case; 1 ms polling sleeps when awaiting worker, no UI frame waits. Owner update timings exclude worker execution; completed-worker milliseconds sum kernel elapsed wall time, not all-core CPU time. Drain wall includes polling and image reconstruction, excludes final pixel oracle. Every padded pixel and emitted-payload accounting checked outside call timing. Texture bytes are RGBA payload with theoretical complete mip chain, not measured GPU allocation. Metadata/prefix bytes count owned vector capacities, excluding weak allocation headers/allocator overhead. Preparation peak estimates worker output capacity plus imaging buffers and per-frame pending canvas image payload, excludes frozen source snapshots, resident cache/history/document and validation copies. This is not RSS, a total memory budget or dense 32-layer certification.", "prefix_budget_bytes":crate::prefix_cache::BUDGET_MIB * 1024 * 1024, "cases":cases});
    let mut report = report;
    report["version"] = serde_json::json!(8);
    report["publication_scope"] = serde_json::json!(
        "Sliced preparation holds one frozen document for the entire tile generation; continuous newer edits request a successor instead of restarting it. Canvas and navigator draw the same complete published handles. Static benchmark phases verify that drawable presentation stays on the previous complete set until all new handles publish together, then compare every presented padded pixel with the compositor. Initial and resized frames remain blank until complete; resize must free all incompatible old texture IDs on the first update. Both prepared and presented texture IDs count once when shared; sampled peaks include retained old handles during same-size replacement, but can miss a transient within one owner call. RGBA and complete mip-chain capacities are theoretical, not GPU/RSS measurements or a residency budget. Frozen generation and active/retired worker sources participate in snapshot deduplication; a canceled retiring worker prevents a new generation source from being captured until it exits. GUI/input latency and continuous-edit performance are covered only by separate functional tests, not these timings."
    );
    report["registry_scope"] = serde_json::json!(
        "Common allocation registry deduplicates live document, retained undo/redo, frozen display and save/recovery source storage, including shared patch/cooling input buffers. Probes hold a Full checkpoint, two frozen documents and two gated storage workers across one pixel edit. Save/recovery add only their independent document metadata; original pixels are checked after both file round trips, and finished workers must contribute zero before polling/join. Source write wall time includes concurrent project/recovery writes and 1 ms polling, excludes loading/pixel verification; it is not owner-call latency. Existing history budget policy is preserved. Recovery retirement counts source conservatively until off-thread join/path cleanup finishes. Excludes working buffers, worker output/scratch, descriptor/registry bookkeeping and allocator overhead; not RSS or a global memory cap."
    );
    report["snapshot_scope"] = serde_json::json!(
        "Non-owning allocation IDs/capacities describe pending immutable sources; handles already finished are excluded. Per-call source sampling is outside owner timing and may miss a short-lived source. Pixel bytes are deduplicated beyond the live document, and may also be shared with history/save. Metadata describes cloned document/name/layer capacities. Descriptor vectors are separate display metadata. Counts are conservative while a worker exits, not RSS or a global memory budget. Snapshot probes hold two real snapshots across one pixel edit, verify unchanged source RGBA and sharing, and compare tiled versus flat compatibility; extra layers are sparse."
    );
    report["snapshot_probes"] = serde_json::to_value(crate::snapshot_memory::probe()?)
        .map_err(|error| error.to_string())?;
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn uniform_tiles_expand_at_gutters_and_compact_after_clear_in_both_alpha_modes() {
        for transparent in [false, true] {
            let ctx = egui::Context::default();
            let mut doc = Document::new(517, 259);
            let background = [17, 83, 201, 96];
            doc.layers[0].pixels.writer().write_region(
                517,
                [0, 0, 517, 259],
                &background.repeat(517 * 259),
            );
            let mut textures = CanvasTextures::default();
            textures.set_transparent(transparent);
            let mut retained = std::collections::HashMap::new();
            for phase in 0..3 {
                if phase > 0 {
                    doc.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut(255 * 517 + 255)
                        .copy_from_slice(if phase == 1 {
                            &[201, 19, 81, 173]
                        } else {
                            &background
                        });
                }
                let output = ctx.run(Default::default(), |ctx| {
                    textures.update(
                        ctx,
                        &doc,
                        phase != 1,
                        (phase == 1).then_some([255, 255, 256, 256]),
                        egui::TextureOptions::LINEAR,
                        &mut Performance::default(),
                    );
                });
                for (id, delta) in &output.textures_delta.set {
                    if *id != egui::TextureId::Managed(0) {
                        // A compact-to-expanded or expanded-to-compact transition
                        // must resize the allocation, never issue an out-of-bounds patch.
                        assert!(delta.pos.is_none());
                        assert_eq!(
                            delta.image.size(),
                            if phase == 1 { [EXTENT, EXTENT] } else { [1, 1] }
                        );
                    }
                }
                retain_images(&mut retained, &output.textures_delta).unwrap();
                for tile in &textures.presented {
                    assert_eq!(
                        retained[&tile.texture.id()],
                        egui::ColorImage::from_rgba_unmultiplied(
                            [EXTENT, EXTENT],
                            &rgba_display(&doc, tile.bounds, [0, 0, EXTENT, EXTENT], transparent)
                        )
                    );
                }
                let expanded = usize::from(phase == 1) * 4;
                assert_eq!(
                    textures.memory().texture_base_rgba_bytes,
                    expanded * EXTENT * EXTENT * 4 + (6 - expanded) * 4
                );
            }
        }
    }

    fn drain_viewport(
        ctx: &egui::Context,
        textures: &mut CanvasTextures,
        doc: &Document,
        images: &mut std::collections::HashMap<egui::TextureId, egui::ColorImage>,
    ) -> (usize, f64) {
        let previous = shown_ids(textures);
        // Generous: debug builds composite full-resolution tiles slowly, and
        // a loaded machine took over 120 s with nothing stalled.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        let mut peak = 0;
        let mut owner_ms = 0.0;
        loop {
            let started = std::time::Instant::now();
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
            owner_ms += started.elapsed().as_secs_f64() * 1000.0;
            retain_images(images, &output.textures_delta).unwrap();
            peak = peak.max(textures.memory().texture_base_rgba_bytes);
            let shown = shown_ids(textures);
            if shown != previous {
                assert_eq!(
                    shown,
                    textures
                        .tiles
                        .iter()
                        .map(|t| t.texture.as_ref().unwrap().id())
                        .collect::<std::collections::HashSet<_>>()
                );
            }
            assert!(textures.take_error().is_none());
            if !textures.busy() {
                return (peak, owner_ms);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "viewport generation stalled"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn check_viewport_samples(
        textures: &CanvasTextures,
        doc: &Document,
        images: &std::collections::HashMap<egui::TextureId, egui::ColorImage>,
    ) {
        check_viewport_indices(
            textures,
            doc,
            images,
            [
                0,
                textures.tiles.len() / 4,
                textures.tiles.len() / 2,
                textures.tiles.len() / 2 + doc.width.div_ceil(SIDE) / 2,
                textures.tiles.len() - 1,
            ],
        );
    }
    fn check_viewport_indices(
        textures: &CanvasTextures,
        doc: &Document,
        images: &std::collections::HashMap<egui::TextureId, egui::ColorImage>,
        indices: impl IntoIterator<Item = usize>,
    ) {
        for index in indices {
            let tile = &textures.tiles[index];
            let full = egui::ColorImage::from_rgba_unmultiplied(
                [EXTENT, EXTENT],
                &rgba_display(
                    doc,
                    tile.bounds,
                    [0, 0, EXTENT, EXTENT],
                    textures.transparent,
                ),
            );
            // Independent image-library box reduction of premultiplied bytes.
            let raw = image::RgbaImage::from_raw(
                EXTENT as u32,
                EXTENT as u32,
                full.pixels.iter().flat_map(|p| p.to_array()).collect(),
            )
            .unwrap();
            let side = EXTENT / tile.level;
            let reduced = image::imageops::thumbnail(&raw, side as u32, side as u32);
            let expected =
                egui::ColorImage::from_rgba_premultiplied([side, side], reduced.as_raw());
            let texture = tile.texture.as_ref().unwrap();
            if texture.size() == [1, 1] {
                assert!(
                    expected
                        .pixels
                        .iter()
                        .all(|p| *p == images[&texture.id()].pixels[0])
                );
            } else {
                assert_eq!(images[&texture.id()], expected, "primary tile {index}");
            }
            if let Some((overview, _)) = &tile.overview {
                let side = EXTENT / overview_level(doc);
                let reduced = image::imageops::thumbnail(&raw, side as u32, side as u32);
                let expected =
                    egui::ColorImage::from_rgba_premultiplied([side, side], reduced.as_raw());
                if overview.size() == [1, 1] {
                    assert!(
                        expected
                            .pixels
                            .iter()
                            .all(|p| *p == images[&overview.id()].pixels[0])
                    );
                } else {
                    assert_eq!(images[&overview.id()], expected, "overview tile {index}");
                }
            }
        }
    }

    #[test]
    fn viewport_plan_cache_tracks_view_dpi_size_and_freezes_inflight_generations() {
        let ctx = egui::Context::default();
        let doc = Document::new(2048, 2048);
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0));
        let mut view = View {
            origin: area.center(),
            zoom: 0.25,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(1024.0, 1024.0),
        };
        let mut textures = CanvasTextures::default();
        textures.set_viewport(area, view);
        textures.sync_resolution_plan(&ctx, &doc);
        assert_eq!(textures.resolution_builds, 1);
        let allocation = textures.desired_levels.as_ptr();
        for _ in 0..100 {
            textures.set_viewport(area, view);
            textures.sync_resolution_plan(&ctx, &doc);
        }
        assert_eq!(textures.resolution_builds, 1);
        assert_eq!(textures.desired_levels.as_ptr(), allocation);
        let frozen = textures.levels.clone();
        textures.generation = Some(doc.clone());
        view.zoom = 1.0;
        textures.set_viewport(area, view);
        textures.sync_resolution_plan(&ctx, &doc);
        assert_eq!(textures.levels, frozen);
        assert_ne!(textures.desired_levels, frozen);
        assert!(textures.refresh);
        textures.generation = None;
        textures.refresh = false;
        textures.sync_resolution_plan(&ctx, &doc);
        assert_eq!(textures.levels, textures.desired_levels);
        assert_eq!(textures.resolution_builds, 2);
        for change in 0..4 {
            match change {
                0 => view.origin.x += 311.0,
                1 => view.rotation = 0.7,
                2 => view.mirror = true,
                _ => view.half.x += 1.0,
            }
            textures.set_viewport(area, view);
            textures.sync_resolution_plan(&ctx, &doc);
            assert_eq!(textures.resolution_builds, 3 + change);
            assert_eq!(
                textures.desired_levels,
                textures.resolution_plan(ctx.pixels_per_point())
            );
        }
        textures.set_viewport(area.expand(50.0), view);
        textures.sync_resolution_plan(&ctx, &doc);
        assert_eq!(textures.resolution_builds, 7);
        ctx.set_pixels_per_point(2.0);
        let _ = ctx.run(Default::default(), |ctx| {
            textures.sync_resolution_plan(ctx, &doc)
        });
        assert_eq!(textures.resolution_builds, 8);
        assert_eq!(textures.desired_levels, textures.resolution_plan(2.0));
        textures.sync_resolution_plan(&ctx, &Document::new(2051, 2053));
        assert_eq!(textures.resolution_builds, 9);
        textures.sync_resolution_plan(&ctx, &Document::new(512, 512));
        let count = textures.resolution_builds;
        textures.set_viewport(area, view);
        textures.sync_resolution_plan(&ctx, &Document::new(512, 512));
        assert_eq!(
            textures.resolution_builds, count,
            "small canvases ignore viewport changes"
        );
    }

    /// A large canvas zoomed out, with a layer that covers every tile.
    fn zoomed_out_canvas() -> (egui::Context, Document, CanvasTextures, Rect) {
        let ctx = egui::Context::default();
        let mut doc = Document::new(2048, 2048);
        doc.layers[0].pixels = (0..doc.width * doc.height)
            .flat_map(|i| [(i % 251) as u8, 71, (i / doc.width % 251) as u8, 255])
            .collect::<Vec<_>>()
            .into();
        doc.add_layer();
        doc.layers[1].pixels = [20, 120, 220, 128].repeat(doc.width * doc.height).into();
        let mut textures = CanvasTextures::default();
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0));
        textures.set_viewport(
            area,
            View {
                origin: area.center(),
                zoom: 0.2,
                rotation: 0.0,
                mirror: false,
                half: egui::vec2(1024.0, 1024.0),
            },
        );
        (ctx, doc, textures, area)
    }
    #[test]
    fn hiding_a_layer_shows_quick_tiles_first_then_exact_ones() {
        let (ctx, mut doc, mut textures, _) = zoomed_out_canvas();
        let mut images = std::collections::HashMap::new();
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        assert!(textures.levels.iter().all(|level| *level == 4));
        doc.layers[1].visible = false;
        let before = shown_ids(&textures);
        let mut quick_frame = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while shown_ids(&textures) == before {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            quick_frame = textures.tiles.iter().any(|tile| tile.rough);
            assert!(std::time::Instant::now() < deadline, "no frame was shown");
        }
        // Shown before every tile was composited exactly, and each quick tile
        // holds the true composite at its texels' middles.
        assert!(quick_frame);
        assert!(textures.tiles.iter().all(|tile| unchanged(tile, &doc)));
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        assert!(textures.tiles.iter().all(|tile| !tile.rough));
        check_viewport_samples(&textures, &doc, &images);
    }
    #[test]
    fn painting_while_quick_tiles_refine_keeps_the_stroke() {
        let (ctx, mut doc, mut textures, _) = zoomed_out_canvas();
        let mut images = std::collections::HashMap::new();
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        doc.layers[1].visible = false;
        while !textures.tiles.iter().all(|tile| unchanged(tile, &doc))
            || textures.composite.is_none()
        {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
        }
        assert!(textures.composite.as_ref().is_some_and(|job| job.refine));
        // A small stroke takes the fast path and drops the refinement, which
        // was composited without it.
        let rect = [1000, 1000, 1004, 1003];
        doc.layers[0]
            .pixels
            .writer()
            .write_region(doc.width, rect, &[255, 0, 0, 255].repeat(12));
        let mut damage = crate::render_region::Damage::default();
        damage.include(rect);
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_damage(
                ctx,
                &doc,
                false,
                damage.clone(),
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        assert!(textures.composite.is_none());
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        assert!(
            textures
                .tiles
                .iter()
                .all(|tile| !tile.rough && unchanged(tile, &doc))
        );
        check_viewport_samples(&textures, &doc, &images);
        let stroked = textures
            .tiles
            .iter()
            .position(|tile| dirty_rect(tile.bounds, rect, doc.width, doc.height).is_some())
            .unwrap();
        check_viewport_indices(&textures, &doc, &images, [stroked]);
    }

    #[test]
    fn reduced_dirty_patches_match_full_composites_at_edges_gutters_and_compact_tiles() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(2051, 2053);
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0));
        let mut view = View {
            origin: area.center(),
            zoom: 0.13,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(1025.5, 1026.5),
        };
        textures.set_viewport(area, view);
        textures.set_transparent(true);
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        for (stage, zoom, transparent) in [(0, 0.13, true), (1, 0.5, true), (2, 1.0, false)] {
            if stage == 1 {
                doc.layers[0].pixels = (0..doc.width * doc.height)
                    .flat_map(|i| [(i % 251) as u8, 71, (i / doc.width % 251) as u8, 127])
                    .collect::<Vec<_>>()
                    .into();
            }
            if stage == 2 {
                for _ in 0..4 {
                    doc.add_layer();
                }
                doc.version = 2;
                doc.layers[0].opacity = 0.63;
                doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
                    id: crate::document::LayerId::new(),
                    enabled: true,
                    linked: true,
                    coverage: crate::layer_mask::Mask::new(doc.width, doc.height, 127).unwrap(),
                });
            }
            view.zoom = zoom;
            view.rotation = 0.3;
            view.mirror = true;
            textures.set_viewport(area, view);
            textures.set_transparent(transparent);
            drain_viewport(&ctx, &mut textures, &doc, &mut images);
            let builds = textures.resolution_builds;
            // Tile crossings, narrow canvas-edge padding, distant simultaneous
            // symmetry damage and repeated edits to a formerly compact tile.
            for (step, rects) in [
                vec![[99, 99, 100, 100]],
                vec![[100, 99, 105, 105]],
                vec![[254, 254, 259, 259]],
                vec![[255, 255, 257, 258]],
                vec![[0, 0, 3, 5], [2048, 2050, 2051, 2053]],
                vec![[1023, 1023, 1041, 1041]],
            ]
            .into_iter()
            .enumerate()
            {
                let mut damage = crate::render_region::Damage::default();
                for &rect in &rects {
                    doc.layers[0].pixels.writer().write_region(
                        doc.width,
                        rect,
                        &[201, (17 + step) as u8, 83, 91]
                            .repeat((rect[2] - rect[0]) * (rect[3] - rect[1])),
                    );
                    damage.include(rect);
                }
                let ids = shown_ids(&textures);
                let mut performance = Performance::default();
                let output = ctx.run(Default::default(), |ctx| {
                    textures.update_damage(
                        ctx,
                        &doc,
                        false,
                        damage.clone(),
                        if step % 2 == 0 {
                            egui::TextureOptions::LINEAR
                        } else {
                            egui::TextureOptions::NEAREST
                        },
                        &mut performance,
                    )
                });
                let uploaded = retain_images(&mut images, &output.textures_delta).unwrap();
                // egui can coalesce an options patch with a later replacement
                // when two damage rectangles touch different compact tiles.
                assert!(uploaded <= performance.display_queued_bytes);
                if rects.len() == 1 {
                    assert_eq!(uploaded, performance.display_queued_bytes);
                }
                assert!(!textures.busy());
                assert_eq!(shown_ids(&textures), ids);
                assert_eq!(textures.resolution_builds, builds);
                let affected = textures.tiles.iter().enumerate().filter_map(|(i, tile)| {
                    rects
                        .iter()
                        .any(|r| dirty_rect(tile.bounds, *r, doc.width, doc.height).is_some())
                        .then_some(i)
                });
                check_viewport_indices(&textures, &doc, &images, affected);
            }
        }
    }

    #[test]
    fn viewport_resolution_preserves_pixels_and_atomic_pan_zoom_alpha_updates() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(2048, 2048);
        doc.layers[0].pixels = (0..2048 * 2048)
            .flat_map(|i| [(i % 251) as u8, 71, (i / 2048 % 251) as u8, 127])
            .collect::<Vec<_>>()
            .into();
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0));
        let mut view = View {
            origin: area.center(),
            zoom: 0.25,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(1024.0, 1024.0),
        };
        textures.set_viewport(area, view);
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        assert_eq!(
            textures.memory().texture_base_rgba_bytes,
            64 * (80 * 80 + 40 * 40) * 4
        );
        check_viewport_samples(&textures, &doc, &images);
        for (center, rotation, mirror) in [
            (pos2(512.0, 512.0), 0.7, true),
            (pos2(1800.0, 1800.0), -0.4, false),
        ] {
            view.zoom = 1.0;
            view.rotation = rotation;
            view.mirror = mirror;
            view.origin += area.center() - view.to_screen(center);
            textures.set_viewport(area, view);
            drain_viewport(&ctx, &mut textures, &doc, &mut images);
            assert!(textures.tiles.iter().any(|t| t.level == 1));
            assert!(textures.tiles.iter().any(|t| t.level == 4));
            check_viewport_samples(&textures, &doc, &images);
        }
        // Normal brush-sized damage in the detailed viewport remains immediate.
        let rect = [1796, 1796, 1800, 1800];
        doc.layers[0]
            .pixels
            .writer()
            .write_region(2048, rect, &[201, 17, 83, 91].repeat(16));
        let mut damage = crate::render_region::Damage::default();
        damage.include(rect);
        let ids = shown_ids(&textures);
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_damage(
                ctx,
                &doc,
                false,
                damage.clone(),
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        assert!(!textures.busy());
        assert_eq!(shown_ids(&textures), ids);
        check_viewport_samples(&textures, &doc, &images);
        doc.layers[0].pixels.writer().write_region(
            2048,
            [10, 10, 18, 18],
            &[201, 17, 83, 91].repeat(64),
        );
        // Offscreen damage uses the reduced grid's coordinates and filtering,
        // never full-scale partial coordinates on an 80-square texture.
        let mut damage = crate::render_region::Damage::default();
        damage.include([10, 10, 18, 18]);
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_damage(
                ctx,
                &doc,
                false,
                damage.clone(),
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        check_viewport_samples(&textures, &doc, &images);
        textures.set_transparent(true);
        drain_viewport(&ctx, &mut textures, &doc, &mut images);
        check_viewport_samples(&textures, &doc, &images);
        // The same viewport on a high-DPI screen asks for more detail.
        view.zoom = 0.25;
        view.origin = area.center();
        textures.set_viewport(area, view);
        let one = textures.resolution_plan(1.0);
        let two = textures.resolution_plan(2.0);
        assert!(one.iter().zip(two).any(|(a, b)| *a > b));
    }

    #[test]
    #[ignore = "release dense 4K viewport residency probe"]
    fn benchmark_viewport_residency() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(4096, 4096);
        doc.layers[0].pixels = (0..4096 * 4096)
            .flat_map(|i| [(i % 251) as u8, 71, (i / 4096 % 251) as u8, 255])
            .collect::<Vec<_>>()
            .into();
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut cases = Vec::new();
        for (phase, zoom, center, rotation, mirror) in [
            ("fit", 0.13, pos2(2048.0, 2048.0), 0.0, false),
            ("detail", 1.0, pos2(2048.0, 2048.0), 0.0, false),
            ("pan-rotate-mirror", 1.0, pos2(700.0, 700.0), 0.6, true),
            ("zoom-out", 0.13, pos2(2048.0, 2048.0), 0.0, false),
        ] {
            let mut view = View {
                origin: area.center(),
                zoom,
                rotation,
                mirror,
                half: egui::vec2(2048.0, 2048.0),
            };
            view.origin += area.center() - view.to_screen(center);
            textures.set_viewport(area, view);
            let (peak, owner_ms) = drain_viewport(&ctx, &mut textures, &doc, &mut images);
            check_viewport_samples(&textures, &doc, &images);
            cases.push(serde_json::json!({"phase":phase,"owner_total_ms":owner_ms,"memory":textures.memory(),
                "peak_including_previous_generation_rgba_bytes":peak,"full_detail_tiles":textures.tiles.iter().filter(|t|t.level==1).count()}));
            if phase == "detail" {
                let mut owner_samples = Vec::new();
                let mut queued_bytes = 0;
                for step in 0..24 {
                    let rect = [2050 + step, 2050, 2068 + step, 2068];
                    doc.layers[0].pixels.writer().write_region(
                        4096,
                        rect,
                        &[201, 17, 83, 91].repeat(18 * 18),
                    );
                    let mut damage = crate::render_region::Damage::default();
                    damage.include(rect);
                    let mut performance = Performance::default();
                    let started = std::time::Instant::now();
                    let output = ctx.run(Default::default(), |ctx| {
                        textures.update_damage(
                            ctx,
                            &doc,
                            false,
                            damage.clone(),
                            egui::TextureOptions::LINEAR,
                            &mut performance,
                        );
                    });
                    owner_samples.push(started.elapsed().as_secs_f64() * 1000.0);
                    queued_bytes += performance.display_queued_bytes;
                    retain_images(&mut images, &output.textures_delta).unwrap();
                    assert!(!textures.busy(), "visible brush edit must remain immediate");
                }
                check_viewport_samples(&textures, &doc, &images);
                cases.push(serde_json::json!({"phase":"visible-dirty-edits","updates":24,
                    "damage_side":18,"owner_samples_ms":owner_samples,"queued_rgba_bytes":queued_bytes,
                    "memory":textures.memory()}));
            }
        }
        std::fs::create_dir_all("artifacts").unwrap();
        std::fs::write("artifacts/viewport-residency.json",serde_json::to_vec_pretty(&serde_json::json!({
            "canvas":[4096,4096],"viewport":[800,600],"uncompressed_full_detail_rgba_bytes":256*EXTENT*EXTENT*4,
            "scope":"Dense patterned artwork, production workers, coherent publication checked on every update, sampled tiles compared with independent image-library reduction. Payload only, not physical VRAM; source/compositor memory excluded. Visible resolution follows screen density; no fixed global cap for arbitrarily large/high-DPI viewports. One run, not pen latency.","cases":cases
        })).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "release 4K viewport brush-patch latency probe"]
    fn benchmark_viewport_patches() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(4096, 4096);
        doc.layers[0].pixels = (0..4096 * 4096)
            .flat_map(|i| [(i % 251) as u8, 71, (i / 4096 % 251) as u8, 127])
            .collect::<Vec<_>>()
            .into();
        let mut textures = CanvasTextures::default();
        textures.set_transparent(true);
        let mut images = std::collections::HashMap::new();
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut cases = Vec::new();
        for zoom in [0.13, 0.5, 1.0] {
            textures.set_viewport(
                area,
                View {
                    origin: area.center(),
                    zoom,
                    rotation: 0.0,
                    mirror: false,
                    half: egui::vec2(2048.0, 2048.0),
                },
            );
            drain_viewport(&ctx, &mut textures, &doc, &mut images);
            for side in [18, 80] {
                let mut samples = Vec::new();
                for step in 0..24 {
                    let x = 2039 + step;
                    let rect = [x, 2043, x + side, 2043 + side];
                    doc.layers[0].pixels.writer().write_region(
                        4096,
                        rect,
                        &[201, (17 + step) as u8, 83, 91].repeat(side * side),
                    );
                    let mut damage = crate::render_region::Damage::default();
                    damage.include(rect);
                    let wall = std::time::Instant::now();
                    let mut owner_ms = 0.0;
                    let mut input_ms = 0.0;
                    let mut calls = 0;
                    let mut queued_bytes = 0;
                    let immediate;
                    loop {
                        let mut performance = Performance::default();
                        let started = std::time::Instant::now();
                        let output = ctx.run(Default::default(), |ctx| {
                            textures.update_damage(
                                ctx,
                                &doc,
                                false,
                                damage.clone(),
                                egui::TextureOptions::LINEAR,
                                &mut performance,
                            );
                        });
                        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                        if calls == 0 {
                            input_ms = elapsed;
                        }
                        owner_ms += elapsed;
                        queued_bytes += performance.display_queued_bytes;
                        calls += 1;
                        retain_images(&mut images, &output.textures_delta).unwrap();
                        if !textures.busy() {
                            immediate = calls == 1;
                            break;
                        }
                        assert!(wall.elapsed() < std::time::Duration::from_secs(5));
                        damage = Default::default();
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    samples.push(serde_json::json!({"input_owner_ms":input_ms,
                        "settled_owner_ms":owner_ms,"settled_wall_ms":wall.elapsed().as_secs_f64()*1000.0,
                        "calls":calls,"immediate":immediate,"queued_rgba_bytes":queued_bytes}));
                }
                check_viewport_samples(&textures, &doc, &images);
                cases.push(
                    serde_json::json!({"zoom":zoom,"damage_side":side,"samples":samples,
                    "memory":textures.memory()}),
                );
            }
        }
        let path = std::env::var("DUDEL_VIEWPORT_PATCH_REPORT")
            .unwrap_or_else(|_| "artifacts/viewport-patches.json".into());
        std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({
            "canvas":[4096,4096],"viewport":[800,600],"cases":cases,
            "scope":"24 sequential settled edits per size/zoom, dense partial-alpha 4K artwork. Production workers, retained pixels checked against independent image-library reductions. Owner timing includes Context::run only; writes, validation and 1ms waits excluded. Settled wall includes those waits and delta retention. Setup excluded. CPU/texture payload, not GPU or physical pen latency."
        })).unwrap()).unwrap();
    }

    #[test]
    fn direct_upload_packing_matches_padded_rgba_at_edges_and_partial_rects() {
        for (width, height) in [(1, 1), (1, 259), (513, 1), (17, 35), (513, 259)] {
            let doc = Document::new(width, height);
            for alpha in [0, 87, 255] {
                let composite: Vec<u8> = (0..width * height)
                    .flat_map(|i| [(i * 17) as u8, (i * 23) as u8, (i / width) as u8, alpha])
                    .collect();
                for tile in bounds(width, height) {
                    for local in [
                        [0, 0, EXTENT, EXTENT],
                        [0, 0, 2, 3],
                        [31, 31, 290, 289],
                        [318, 318, 320, 320],
                        [0, 250, 35, 320],
                    ] {
                        // Include a composite broader than this tile, as in batch jobs.
                        let source = [0, 0, width, height];
                        let expected =
                            tile_image(local, &sampled_rgba(&doc, tile, local, source, &composite));
                        assert_eq!(
                            sampled_image(&doc, tile, local, source, &composite),
                            expected,
                            "{width}x{height} alpha={alpha} tile={tile:?} local={local:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "release tile-packing probe; run without concurrent workloads"]
    fn benchmark_direct_upload_packing() {
        let doc = Document::new(4096, 4096);
        let mut cases = Vec::new();
        for (name, tile, local, alpha, uniform) in [
            (
                "dense-opaque",
                [256, 256, 512, 512],
                [0, 0, EXTENT, EXTENT],
                255,
                false,
            ),
            (
                "dense-alpha",
                [256, 256, 512, 512],
                [0, 0, EXTENT, EXTENT],
                87,
                false,
            ),
            (
                "canvas-edge",
                [0, 0, 256, 256],
                [0, 0, EXTENT, EXTENT],
                87,
                false,
            ),
            (
                "uniform",
                [0, 0, 256, 256],
                [0, 0, EXTENT, EXTENT],
                255,
                true,
            ),
            (
                "small-dirty",
                [256, 256, 512, 512],
                [40, 40, 58, 58],
                255,
                false,
            ),
        ] {
            let source = tile_source_rect(tile, [0, 0, EXTENT, EXTENT], &doc);
            let count = (source[2] - source[0]) * (source[3] - source[1]);
            let composite: Vec<u8> = (0..count)
                .flat_map(|i| {
                    if uniform {
                        [17, 83, 201, alpha]
                    } else {
                        [(i * 17) as u8, (i * 23) as u8, (i / 320) as u8, alpha]
                    }
                })
                .collect();
            let old = || {
                let pixels = sampled_rgba(&doc, tile, local, source, &composite);
                let image = tile_image(local, &pixels);
                let bytes = pixels.capacity() + image.pixels.capacity() * 4;
                (image, bytes)
            };
            let new = || {
                let image = sampled_image(&doc, tile, local, source, &composite);
                let bytes = image.pixels.capacity() * 4;
                (image, bytes)
            };
            let (expected, old_bytes) = old();
            let (actual, new_bytes) = new();
            assert_eq!(actual, expected);
            let mut old_ms = Vec::new();
            let mut new_ms = Vec::new();
            for trial in 0..5 {
                let mut measure = |direct| {
                    let started = std::time::Instant::now();
                    for _ in 0..100 {
                        std::hint::black_box(if direct { new() } else { old() });
                    }
                    let elapsed = started.elapsed().as_secs_f64() * 1000.0 / 100.0;
                    if direct {
                        new_ms.push(elapsed);
                    } else {
                        old_ms.push(elapsed);
                    }
                };
                measure(trial % 2 == 0);
                measure(trial % 2 != 0);
            }
            cases.push(serde_json::json!({"name":name,"before_ms_per_tile":old_ms,"after_ms_per_tile":new_ms,
                "before_tile_allocation_bytes":old_bytes,"after_tile_allocation_bytes":new_bytes,"exact_pixels":true}));
        }
        std::fs::create_dir_all("artifacts").unwrap();
        std::fs::write("artifacts/direct-upload-packing.json", serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"Five alternating-order pairs of 100 preparations per case, release CPU only. Shared composite, source document, upload queue and GPU allocation excluded. Exact image sizes/pixels checked outside timing. Old and new paths in the same test binary.","cases":cases
        })).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "release display memory probe; excludes real GPU allocation overhead"]
    fn benchmark_sparse_four_k_tiles() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(4096, 4096);
        let mut textures = CanvasTextures::default();
        let mut cases = Vec::new();
        for phase in ["blank", "stroke", "cleared"] {
            if phase == "stroke" {
                doc.dab(
                    2048.0,
                    2048.0,
                    1.0,
                    &crate::brush::Brush {
                        size: 300.0,
                        pressure_size: false,
                        ..Default::default()
                    },
                    [17, 83, 201],
                    false,
                );
            } else if phase == "cleared" {
                doc = Document::new(4096, 4096);
            }
            let mut performance = Performance::default();
            let started = std::time::Instant::now();
            let output = ctx.run(Default::default(), |ctx| {
                textures.update(
                    ctx,
                    &doc,
                    phase != "stroke",
                    (phase == "stroke").then_some([1896, 1896, 2200, 2200]),
                    egui::TextureOptions {
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..egui::TextureOptions::LINEAR
                    },
                    &mut performance,
                );
            });
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            let bytes: usize = output
                .textures_delta
                .set
                .iter()
                .filter(|(id, _)| *id != egui::TextureId::Managed(0))
                .map(|(_, delta)| {
                    delta.image.bytes_per_pixel()
                        * delta.image.size().into_iter().product::<usize>()
                })
                .sum();
            assert_eq!(bytes, performance.display_queued_bytes);
            cases.push(serde_json::json!({"phase":phase,"update_ms":elapsed,
                "memory":textures.memory(),"queued_rgba_bytes":bytes}));
        }
        std::fs::create_dir_all("artifacts").unwrap();
        std::fs::write("artifacts/compact-display.json", serde_json::to_vec_pretty(&serde_json::json!({
            "canvas":[4096,4096],"uncompressed_base_rgba_bytes":256*EXTENT*EXTENT*4,
            "scope":"CPU texture payload and theoretical mip bytes; excludes GPU/driver allocation overhead and source/compositor memory. One sequential run, not physical input latency.","cases":cases
        })).unwrap()).unwrap();
    }

    #[test]
    fn hiding_a_sparse_layer_recomposites_only_tiles_it_covers() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(1024, 768);
        doc.add_layer();
        let paint = |doc: &mut Document, layer: usize, rect: [usize; 4], rgba: [u8; 4]| {
            let count = (rect[2] - rect[0]) * (rect[3] - rect[1]);
            doc.layers[layer]
                .pixels
                .writer()
                .write_region(1024, rect, &rgba.repeat(count));
        };
        paint(&mut doc, 0, [0, 0, 1024, 768], [240, 240, 240, 255]);
        paint(&mut doc, 1, [300, 300, 340, 340], [200, 20, 20, 255]);
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        doc.layers[1].visible = false;
        textures.test_units = Some(1 << 20);
        let mut performance = Performance::default();
        // A tight fixed poll count can expire before Windows schedules the
        // worker, especially while the rest of the suite is running. The gate
        // here is correct pixels and bounded uploads, not worker scheduling.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            assert!(
                std::time::Instant::now() < deadline,
                "Texture worker did not settle"
            );
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut performance,
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
            std::thread::yield_now();
        }
        assert!(!textures.busy());
        check_presented(&textures, &images, &doc);
        // The 40px mark lies inside one 256px tile; with 32px gutters only that
        // tile is recomposited, never all twelve.
        let tile_bytes = EXTENT * EXTENT * 4;
        assert!(
            performance.display_queued_bytes <= tile_bytes,
            "queued {} bytes",
            performance.display_queued_bytes
        );
    }
    fn paint_ids(ctx: &egui::Context, textures: &CanvasTextures, width: usize, height: usize) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let area = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 800.0));
        for (origin, zoom) in [(pos2(350.0, 350.0), 1.0), (pos2(1100.0, 150.0), 0.3)] {
            textures.paint(
                &painter,
                area,
                View {
                    origin,
                    zoom,
                    rotation: 0.0,
                    mirror: false,
                    half: egui::vec2(width as f32 / 2.0, height as f32 / 2.0),
                },
            );
        }
    }
    fn shown_ids(textures: &CanvasTextures) -> std::collections::HashSet<egui::TextureId> {
        textures
            .presented
            .iter()
            .map(|tile| tile.texture.id())
            .collect()
    }
    #[test]
    fn bounded_brush_damage_is_visible_immediately_and_does_not_start_an_idle_batch() {
        for tiled in [false, true] {
            for layers in [1, 8] {
                for side in [18, 80] {
                    let mut doc = Document::new(517, 517);
                    let bytes = [17, 83, 201, 128].repeat(517 * 517);
                    doc.layers[0].pixels = if tiled {
                        crate::pixels::Pixels::read_rgba(&mut bytes.as_slice(), 517, 517).unwrap()
                    } else {
                        bytes.into()
                    };
                    for _ in 1..layers {
                        doc.add_layer();
                    }
                    doc.active = 0;
                    let ctx = egui::Context::default();
                    let mut textures = CanvasTextures::default();
                    let mut images = std::collections::HashMap::new();
                    let output = ctx.run(Default::default(), |ctx| {
                        textures.update(
                            ctx,
                            &doc,
                            true,
                            None,
                            egui::TextureOptions::LINEAR,
                            &mut Performance::default(),
                        )
                    });
                    retain_images(&mut images, &output.textures_delta).unwrap();
                    let rect = [192, 192, 192 + side, 192 + side];
                    for red in [31, 83, 201] {
                        doc.layers[0].pixels.writer().write_region(
                            517,
                            rect,
                            &[red, 17, 83, 255].repeat(side * side),
                        );
                        let output = ctx.run(Default::default(), |ctx| {
                            textures.update_sliced(
                                ctx,
                                &doc,
                                false,
                                Some(rect),
                                egui::TextureOptions::LINEAR,
                                &mut Performance::default(),
                            )
                        });
                        retain_images(&mut images, &output.textures_delta).unwrap();
                        assert!(!textures.busy());
                        let tile = &textures.presented[0];
                        assert_eq!(
                            images[&tile.texture.id()].pixels[(192 + PAD) * EXTENT + 192 + PAD],
                            Color32::from_rgb(red, 17, 83)
                        );
                        let idle = ctx.run(Default::default(), |ctx| {
                            textures.update_sliced(
                                ctx,
                                &doc,
                                false,
                                None,
                                egui::TextureOptions::LINEAR,
                                &mut Performance::default(),
                            )
                        });
                        assert!(
                            idle.textures_delta
                                .set
                                .iter()
                                .all(|(id, _)| *id == egui::TextureId::Managed(0))
                        );
                        assert!(!textures.busy());
                        assert_eq!(textures.snapshot_memory(&doc).snapshots, 0);
                    }
                    check_presented(&textures, &images, &doc);
                }
            }
        }
    }
    #[test]
    fn visual_metadata_change_cannot_use_the_bounded_pixel_only_path() {
        let mut doc = Document::new(257, 35);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
        doc.layers[0].opacity = 0.4;
        let ctx = egui::Context::default();
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let first = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &first.textures_delta).unwrap();
        let old = doc.clone();
        doc.layers[0].csp_byte_opacity = true;
        textures.test_units = Some(1);
        let first = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                false,
                Some([1, 1, 2, 2]),
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &first.textures_delta).unwrap();
        assert!(textures.busy());
        check_presented(&textures, &images, &old);
        textures.test_units = Some(64);
        for _ in 0..1000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
        }
        assert!(!textures.busy());
        check_presented(&textures, &images, &doc);
    }
    #[test]
    fn workspace_navigator_draws_generation_published_later_in_the_same_pass() {
        fn mesh_ids(shape: &egui::Shape, ids: &mut Vec<egui::TextureId>) {
            match shape {
                egui::Shape::Mesh(mesh) => ids.push(mesh.texture_id),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        mesh_ids(shape, ids);
                    }
                }
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut app = crate::Dudel {
            doc: Document::new(1, 1),
            inspector_override: Some(true),
            inspector_navigator: true, // Color is the right panel's default tab.
            ..Default::default()
        };
        let raw = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        let mut images = std::collections::HashMap::new();
        let first = ctx.run(raw(), |ctx| {
            app.texture.update(
                ctx,
                &app.doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        retain_images(&mut images, &first.textures_delta).unwrap();
        let old_ids = shown_ids(&app.texture);
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 128]);
        app.texture.test_units = Some(1); // One pixel finishes a padded tile this pass.
        let output = ctx.run(raw(), |ctx| {
            app.workspace(ctx); // Reserves navigator painting before the canvas.
            assert!(app.navigator_paint.is_some());
            assert_eq!(shown_ids(&app.texture), old_ids);
            app.canvas_after_tool_change(ctx, &[], crate::Tool::Brush);
            assert!(app.navigator_paint.is_none());
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        let new_ids = shown_ids(&app.texture);
        assert_eq!(new_ids.len(), 1);
        assert!(new_ids.is_disjoint(&old_ids));
        assert!(!app.texture.busy());
        check_presented(&app.texture, &images, &app.doc);
        let mut drawn = Vec::new();
        for shape in &output.shapes {
            mesh_ids(&shape.shape, &mut drawn);
        }
        assert!(!drawn.iter().any(|id| old_ids.contains(id)));
        assert_eq!(drawn.iter().filter(|id| new_ids.contains(id)).count(), 2);
        assert!(
            !ctx.tessellate(output.shapes, output.pixels_per_point)
                .is_empty()
        );
    }
    fn check_presented(
        textures: &CanvasTextures,
        images: &std::collections::HashMap<egui::TextureId, egui::ColorImage>,
        doc: &Document,
    ) {
        for tile in &textures.presented {
            assert_eq!(
                images[&tile.texture.id()],
                egui::ColorImage::from_rgba_unmultiplied(
                    [EXTENT, EXTENT],
                    &rgba(doc, tile.bounds, [0, 0, EXTENT, EXTENT])
                )
            );
        }
    }
    #[test]
    fn transparency_switch_keeps_old_frame_until_complete_alpha_tiles_publish_over_checker() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(517, 259);
        // Opaque, translucent and untouched (transparent) areas.
        {
            let mut writer = doc.layers[0].pixels.writer();
            for y in 0..259 {
                for x in 0..300 {
                    let alpha = if x < 150 { 255 } else { 96 };
                    writer
                        .rgba_mut(y * 517 + x)
                        .copy_from_slice(&[200, 40, 90, alpha]);
                }
            }
        }
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        check_presented(&textures, &images, &doc);
        let old_ids = shown_ids(&textures);
        let old_images = images.clone();
        assert!(textures.set_transparent(true));
        assert!(!textures.set_transparent(true));
        textures.set_checker(true);
        textures.test_units = Some(64);
        let view = View {
            origin: pos2(300.0, 200.0),
            zoom: 1.0,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(517.0 / 2.0, 259.0 / 2.0),
        };
        let area = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut published = false;
        for frame in 0..10000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_damage(
                    ctx,
                    &doc,
                    frame == 0,
                    Default::default(),
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if shown_ids(&textures) == old_ids {
                // The paper frame stays whole, without a checkerboard.
                assert!(!textures.presented_transparent);
                for id in &old_ids {
                    assert_eq!(images.get(id), old_images.get(id));
                }
                continue;
            }
            assert!(textures.presented_transparent);
            for tile in &textures.presented {
                assert_eq!(
                    images[&tile.texture.id()],
                    egui::ColorImage::from_rgba_unmultiplied(
                        [EXTENT, EXTENT],
                        &rgba_display(&doc, tile.bounds, [0, 0, EXTENT, EXTENT], true)
                    )
                );
            }
            published = true;
            break;
        }
        assert!(published && !textures.busy());
        // Transparent pixels really are transparent in the display tiles.
        let tile = &textures.presented[1];
        assert_eq!(
            images[&tile.texture.id()].pixels[EXTENT * PAD + EXTENT - PAD - 1].a(),
            0
        );
        // Each tile is drawn over its own checker quad.
        let checker = textures.checker.as_ref().unwrap().id();
        let shapes = textures.shapes(area, view);
        let meshes: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
                _ => None,
            })
            .collect();
        // Every checker quad is drawn before any tile, so content placed
        // between them (references below the artwork) shows beneath the art.
        let tiles = textures.presented.len();
        assert_eq!(meshes.len(), tiles * 2);
        assert!(meshes[..tiles].iter().all(|id| *id == checker));
        assert!(meshes[tiles..].iter().all(|id| *id != checker));
        // Without the checkerboard, alpha tiles sit on white paper instead.
        textures.set_checker(false);
        let paper = textures.shapes(area, view);
        assert_eq!(paper.len(), tiles + 1);
        assert!(matches!(&paper[0],egui::Shape::Path(path) if path.fill == Color32::WHITE));
        textures.set_checker(true);
        // Switching back restores paper tiles and drops the checkerboard.
        assert!(textures.set_transparent(false));
        for frame in 0..10000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_damage(
                    ctx,
                    &doc,
                    frame == 0,
                    Default::default(),
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if !textures.presented_transparent {
                break;
            }
        }
        assert!(!textures.presented_transparent);
        check_presented(&textures, &images, &doc);
        assert!(
            textures.shapes(area, view).iter().all(
                |shape| !matches!(shape, egui::Shape::Mesh(mesh) if mesh.texture_id == checker)
            )
        );
    }
    #[test]
    fn distant_symmetry_damage_updates_each_rectangle_in_place() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(1024, 768);
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        // Two mirrored marks on a diagonal: their bounding box would cover
        // most of the canvas, but each changed rectangle is tiny.
        let mut damage = crate::render_region::Damage::default();
        for [x, y] in [[100usize, 80usize], [900, 690]] {
            for dy in 0..6 {
                for dx in 0..6 {
                    doc.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut((y + dy) * 1024 + x + dx)
                        .copy_from_slice(&[200, 30, 40, 255]);
                }
            }
            damage.include([x, y, x + 6, y + 6]);
        }
        assert_eq!(damage.rects().len(), 2);
        let mut performance = Performance::default();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_damage(
                ctx,
                &doc,
                false,
                damage.clone(),
                egui::TextureOptions::LINEAR,
                &mut performance,
            )
        });
        let uploaded = retain_images(&mut images, &output.textures_delta).unwrap();
        // Updated in place this frame, without a sliced generation pass.
        assert!(!textures.busy());
        // Only the two newly painted tiles expand; distant uniform tiles
        // retain their compact storage. Subsequent marks use small patches.
        assert_eq!(uploaded, 2 * EXTENT * EXTENT * 4);
        check_presented(&textures, &images, &doc);
        for [x, y] in [[100usize, 80usize], [900, 690]] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(y * 1024 + x)
                .copy_from_slice(&[10, 90, 170, 255]);
        }
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_damage(
                ctx,
                &doc,
                false,
                damage.clone(),
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        let uploaded = retain_images(&mut images, &output.textures_delta).unwrap();
        assert!(uploaded < 64 * 1024, "uploaded {uploaded} bytes");
        check_presented(&textures, &images, &doc);
    }
    #[test]
    fn canvas_and_navigator_hold_complete_frame_until_all_replacement_tiles_publish() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(517, 259);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(517 * 259).into();
        let old = doc.clone();
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        let old_ids = shown_ids(&textures);
        assert_eq!(textures.memory().resident_tiles, 6); // Shared handles count once.
        check_presented(&textures, &images, &old);
        let old_images = images.clone();
        textures.test_units = Some(64);
        doc.layers[0].opacity = 0.4;
        let mut staged_seen = false;
        let mut committed = false;
        for frame in 0..10000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    frame == 0,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
                paint_ids(ctx, &textures, doc.width, doc.height);
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            let ids = shown_ids(&textures);
            let mesh_ids: std::collections::HashSet<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh.texture_id),
                    _ => None,
                })
                .collect();
            assert_eq!(mesh_ids, ids); // Both views draw the same presented set.
            if ids != old_ids {
                assert_eq!(ids.len(), 6);
                check_presented(&textures, &images, &doc);
                committed = true;
                break;
            }
            for id in &old_ids {
                assert_eq!(images.get(id), old_images.get(id));
            }
            if textures.memory().resident_tiles > 6 {
                staged_seen = true;
            }
        }
        assert!(staged_seen && committed && !textures.busy());
        assert_eq!(textures.memory().resident_tiles, 6);
    }
    #[test]
    fn continuous_newer_metadata_cannot_starve_a_frozen_display_generation() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(257, 35);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
        let mut textures = CanvasTextures::default();
        let mut images = std::collections::HashMap::new();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        let old_ids = shown_ids(&textures);
        textures.test_units = Some(64);
        doc.layers[0].opacity = 0.5;
        let frozen = doc.clone();
        let mut published = false;
        for frame in 0..1000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    true,
                    Some([0, 0, 257, 35]),
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if shown_ids(&textures) != old_ids {
                check_presented(&textures, &images, &frozen);
                assert!(textures.refresh && textures.busy());
                published = true;
                break;
            }
            doc.layers[0].opacity = 0.2 + (frame % 50) as f32 / 100.0;
        }
        assert!(published);
        for _ in 0..1000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
        }
        assert!(!textures.busy());
        check_presented(&textures, &images, &doc);
    }
    #[test]
    fn resize_draws_no_mismatched_frame_and_initial_generation_is_never_partial() {
        let ctx = egui::Context::default();
        let mut textures = CanvasTextures {
            test_units: Some(64),
            ..Default::default()
        };
        let doc = Document::new(257, 35);
        let mut images = std::collections::HashMap::new();
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        assert!(textures.presented.is_empty());
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            )
        });
        retain_images(&mut images, &output.textures_delta).unwrap();
        assert_eq!(textures.presented.len(), 2);
        let old_ids = shown_ids(&textures);
        let newer = Document::new(513, 257);
        let output = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &newer,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
            paint_ids(ctx, &textures, newer.width, newer.height);
        });
        assert!(
            output
                .shapes
                .iter()
                .all(|shape| !matches!(&shape.shape, egui::Shape::Mesh(_)))
        );
        retain_images(&mut images, &output.textures_delta).unwrap();
        assert!(old_ids.iter().all(|id| !images.contains_key(id)));
        assert!(textures.memory().resident_tiles <= 6);
        for _ in 0..10000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &newer,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                )
            });
            retain_images(&mut images, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
        }
        assert!(!textures.busy());
        assert_eq!(textures.presented.len(), 6);
        assert_eq!(textures.memory().resident_tiles, 6);
        check_presented(&textures, &images, &newer);
    }
    #[test]
    fn viewport_priority_tracks_pan_rotation_and_mirroring() {
        let doc = Document::new(1024, 1024);
        let mut textures = CanvasTextures::default();
        textures.resize(&doc);
        textures.pending.fill(Some([0, 0, EXTENT, EXTENT]));
        for rotation in [0.0, std::f32::consts::FRAC_PI_4] {
            for mirror in [false, true] {
                let mut view = View {
                    origin: pos2(0.0, 0.0),
                    zoom: 1.0,
                    rotation,
                    mirror,
                    half: egui::vec2(512.0, 512.0),
                };
                view.origin -= view.to_screen(pos2(900.0, 900.0)).to_vec2();
                let area = Rect::from_center_size(pos2(0.0, 0.0), egui::vec2(40.0, 40.0));
                textures.set_viewport(area, view);
                assert_eq!(textures.next_pending(), Some(15));
                view.origin = pos2(0.0, 0.0);
                view.origin -= view.to_screen(pos2(100.0, 100.0)).to_vec2();
                textures.set_viewport(area, view);
                assert_eq!(textures.next_pending(), Some(0));
            }
        }
    }

    #[test]
    fn newer_damage_and_metadata_during_display_worker_wait_drain_exactly() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(257, 35);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
        let frozen = doc.clone();
        let snapshot = crate::snapshot_memory::Snapshot::capture(&frozen);
        let mut textures = CanvasTextures::default();
        textures.resize(&doc);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (send, recv) = std::sync::mpsc::channel();
        let rect = [0, 0, 257, 35];
        let handle = crate::display_workers::spawn(move || {
            recv.recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            Ok((
                frozen.composite_region_cancelable(rect, &worker_cancel, false)?,
                0.0,
            ))
        })
        .unwrap();
        textures.composite = Some(DisplayJob {
            tile: 0,
            local: [0, 0, EXTENT, EXTENT],
            source_rect: rect,
            signature: signature(&doc, textures.tiles[0].bounds),
            preparation: Preparation::Worker(DisplayWorker {
                cancel,
                handle: Some(handle),
                capacity: 257 * 35 * 4,
                snapshot,
            }),
            step: 1,
            refine: false,
            batch: Vec::new(),
        });
        let first = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        let mut retained = std::collections::HashMap::new();
        retain_images(&mut retained, &first.textures_delta).unwrap();
        assert!(textures.waiting());
        assert_eq!(textures.snapshot_memory(&doc).pixel_bytes_beyond_live, 0);
        doc.layers[0].opacity = 0.4;
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(18 * 257 + 255)
            .copy_from_slice(&[201, 17, 83, 255]);
        let newer = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                Some([255, 18, 256, 19]),
                egui::TextureOptions::NEAREST,
                &mut Performance::default(),
            );
        });
        retain_images(&mut retained, &newer.textures_delta).unwrap();
        assert!(textures.refresh);
        assert_eq!(textures.generation.as_ref().unwrap().layers[0].opacity, 1.0);
        let held = textures.snapshot_memory(&doc);
        assert_eq!(held.snapshots, 2); // Batch source plus its still-running worker.
        assert!(held.pixel_bytes_beyond_live >= 257 * 35 * 4);
        send.send(()).unwrap();
        for calls in 0..1000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::NEAREST,
                    &mut Performance::default(),
                );
            });
            retain_images(&mut retained, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
            assert!(calls < 999);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!textures.worker_failed && textures.take_error().is_none());
        assert_eq!(textures.snapshot_memory(&doc).snapshots, 0);
        for tile in &textures.tiles {
            let expected = egui::ColorImage::from_rgba_unmultiplied(
                [EXTENT, EXTENT],
                &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
            );
            assert_eq!(retained[&tile.texture.as_ref().unwrap().id()], expected);
            assert!(unchanged(tile, &doc));
        }
    }

    #[test]
    fn display_worker_freezes_layer_pixels_and_styles() {
        let mut doc = Document::new(513, 257);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(513 * 257).into();
        doc.group_active().unwrap();
        doc.layers[doc.active].opacity = 0.6;
        doc.add_layer();
        doc.layers[doc.active].pixels = [201, 17, 83, 127].repeat(513 * 257).into();
        doc.layers[doc.active].clipped = true;
        doc.layers[doc.active].blend = crate::blend::BlendMode::Multiply;
        let rect = [240, 240, 513, 257];
        let expected = doc.composite_region(rect[0], rect[1], rect[2], rect[3]);
        let worker = DisplayWorker::start(&doc, rect, false, 1).unwrap();
        doc.layers[doc.active].opacity = 0.1;
        doc.layers[doc.active].pixels.writer().write_region(
            513,
            rect,
            &[0; 4].repeat((rect[2] - rect[0]) * (rect[3] - rect[1])),
        );
        let (actual, _) = worker.finish().unwrap();
        assert_eq!(actual, expected);
        assert_ne!(
            actual,
            doc.composite_region(rect[0], rect[1], rect[2], rect[3])
        );
    }

    #[test]
    fn geometry_replacement_waits_for_canceled_worker_without_blocking_update() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(513, 257);
        doc.layers[0].pixels = [17, 83, 201, 255].repeat(513 * 257).into();
        let frozen = doc.clone();
        let snapshot = crate::snapshot_memory::Snapshot::capture(&frozen);
        let mut textures = CanvasTextures::default();
        textures.resize(&doc);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (send, recv) = std::sync::mpsc::channel();
        let rect = [0, 0, 288, 257];
        let handle = crate::display_workers::spawn(move || {
            recv.recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|error| error.to_string())?;
            Ok((
                frozen.composite_region_cancelable(rect, &worker_cancel, false)?,
                0.0,
            ))
        })
        .unwrap();
        textures.composite = Some(DisplayJob {
            tile: 0,
            local: [0, 0, EXTENT, EXTENT],
            source_rect: rect,
            signature: signature(&doc, textures.tiles[0].bounds),
            preparation: Preparation::Worker(DisplayWorker {
                cancel: cancel.clone(),
                handle: Some(handle),
                capacity: 288 * 257 * 4,
                snapshot,
            }),
            step: 1,
            refine: false,
            batch: Vec::new(),
        });
        let first = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        assert!(textures.waiting());
        assert!(
            first
                .textures_delta
                .set
                .iter()
                .all(|(id, _)| *id == egui::TextureId::Managed(0))
        );
        doc = Document::new(1, 1);
        let mut canceled_performance = Performance::default();
        let resized = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut canceled_performance,
            );
        });
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(textures.retired.is_some() && textures.composite.is_none());
        assert!(textures.generation.is_none());
        assert_eq!(textures.snapshot_memory(&doc).snapshots, 1);
        assert!(textures.snapshot_memory(&doc).pixel_bytes_beyond_live >= 513 * 257 * 4);
        assert!(canceled_performance.display_preparation_peak_bytes >= 288 * 257 * 4);
        assert!(
            resized
                .textures_delta
                .set
                .iter()
                .all(|(id, _)| *id == egui::TextureId::Managed(0))
        );
        doc = Document::new(257, 35);
        for red in [17, 83, 201] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(18 * 257 + 255)
                .copy_from_slice(&[red, 17, 83, 255]);
            doc.layers[0].opacity = red as f32 / 255.0;
            let _ = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    true,
                    Some([255, 18, 256, 19]),
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
            assert!(textures.generation.is_none() && textures.composite.is_none());
            assert!(textures.busy() && textures.waiting());
            assert_eq!(textures.snapshot_memory(&doc).snapshots, 1);
            assert_eq!(textures.memory().resident_tiles, 0);
        }
        send.send(()).unwrap();
        assert!(textures.retired.take().unwrap().finish().is_err());
        let mut retained = std::collections::HashMap::new();
        for calls in 0..1000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
            retain_images(&mut retained, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
            assert!(calls < 999);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!textures.worker_failed && textures.take_error().is_none());
        assert_eq!(retained.len(), 2);
        check_presented(&textures, &retained, &doc);
    }

    #[test]
    fn failed_display_worker_uses_sliced_fallback_and_preserves_pixels() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(257, 35);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
        let mut textures = CanvasTextures::default();
        textures.resize(&doc);
        textures.composite = Some(DisplayJob {
            tile: 0,
            local: [0, 0, EXTENT, EXTENT],
            source_rect: [0, 0, 257, 35],
            signature: signature(&doc, textures.tiles[0].bounds),
            preparation: Preparation::Worker(DisplayWorker {
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                handle: Some(
                    crate::display_workers::spawn(|| Err("Synthetic failure".into())).unwrap(),
                ),
                capacity: 257 * 35 * 4,
                snapshot: crate::snapshot_memory::Snapshot::default(),
            }),
            step: 1,
            refine: false,
            batch: Vec::new(),
        });
        let mut retained = std::collections::HashMap::new();
        for calls in 0..1000 {
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
            retain_images(&mut retained, &output.textures_delta).unwrap();
            if !textures.busy() {
                break;
            }
            assert!(calls < 999);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(textures.worker_failed);
        assert!(textures.take_error().unwrap().contains("Synthetic failure"));
        for tile in &textures.tiles {
            let expected = egui::ColorImage::from_rgba_unmultiplied(
                [EXTENT, EXTENT],
                &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
            );
            assert_eq!(retained[&tile.texture.as_ref().unwrap().id()], expected);
        }
    }

    #[test]
    fn sliced_display_drains_edits_metadata_and_filter_changes_to_exact_pixels() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(257, 35);
        doc.layers[0].pixels = [17, 83, 201, 128].repeat(257 * 35).into();
        for _ in 0..4 {
            doc.add_layer();
        }
        let mut textures = CanvasTextures {
            test_units: Some(32),
            ..Default::default()
        };
        let mut retained = std::collections::HashMap::new();
        let mut calls = 0;
        loop {
            if calls == 1 {
                doc.layers[4]
                    .pixels
                    .writer()
                    .rgba_mut(16 * 257 + 255)
                    .copy_from_slice(&[201, 83, 17, 255]);
                doc.layers[0].opacity = 0.4;
            }
            let mut performance = Performance::default();
            let options = if calls >= 2 {
                egui::TextureOptions::NEAREST
            } else {
                egui::TextureOptions::LINEAR
            };
            let output = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    &doc,
                    calls <= 1,
                    (calls == 1).then_some([255, 16, 256, 17]),
                    options,
                    &mut performance,
                );
            });
            assert_eq!(
                retain_images(&mut retained, &output.textures_delta).unwrap(),
                performance.display_queued_bytes
            );
            if calls == 0 {
                assert!(textures.busy());
            }
            calls += 1;
            assert!(calls < 1000);
            if !textures.busy() {
                break;
            }
        }
        assert!(calls > 2);
        for tile in &textures.tiles {
            let expected = egui::ColorImage::from_rgba_unmultiplied(
                [EXTENT, EXTENT],
                &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
            );
            assert_eq!(retained[&tile.texture.as_ref().unwrap().id()], expected);
            assert!(unchanged(tile, &doc));
        }
        let idle = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::NEAREST,
                &mut Performance::default(),
            );
        });
        assert!(
            idle.textures_delta
                .set
                .iter()
                .all(|(id, _)| *id == egui::TextureId::Managed(0))
        );
        assert!(!textures.busy());
        doc.layers[4]
            .pixels
            .writer()
            .rgba_mut(16 * 257 + 255)
            .copy_from_slice(&[17, 201, 83, 255]);
        let small = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                false,
                Some([255, 16, 256, 17]),
                egui::TextureOptions::NEAREST,
                &mut Performance::default(),
            );
        });
        retain_images(&mut retained, &small.textures_delta).unwrap();
        assert!(!textures.busy());
        assert!(textures.memory().prefix_heap_bytes > 0);
        for tile in &textures.tiles {
            let expected = egui::ColorImage::from_rgba_unmultiplied(
                [EXTENT, EXTENT],
                &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
            );
            assert_eq!(retained[&tile.texture.as_ref().unwrap().id()], expected);
        }
    }

    #[test]
    fn resized_sliced_display_discards_unpublished_old_geometry() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(513, 257);
        doc.layers[0].pixels = [17, 83, 201, 255].repeat(513 * 257).into();
        let mut textures = CanvasTextures {
            test_units: Some(32),
            ..Default::default()
        };
        let initial = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        assert!(textures.busy());
        assert!(
            initial
                .textures_delta
                .set
                .iter()
                .all(|(id, _)| *id == egui::TextureId::Managed(0))
        );
        doc = Document::new(1, 1);
        let final_output = ctx.run(Default::default(), |ctx| {
            textures.update_sliced(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut Performance::default(),
            );
        });
        let mut retained = std::collections::HashMap::new();
        retain_images(&mut retained, &final_output.textures_delta).unwrap();
        assert!(!textures.busy());
        assert_eq!(textures.memory().resident_tiles, 1);
        assert_eq!(retained.len(), 1);
        assert!(
            retained
                .values()
                .next()
                .unwrap()
                .pixels
                .iter()
                .all(|pixel| *pixel == Color32::WHITE)
        );
    }

    #[test]
    fn display_accounting_matches_emitted_images_and_resize_releases_prefix_state() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(513, 257);
        for _ in 0..4 {
            doc.add_layer();
        }
        let mut textures = CanvasTextures::default();
        let options = egui::TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..egui::TextureOptions::LINEAR
        };
        let mut performance = Performance::default();
        let cold = ctx.run(Default::default(), |ctx| {
            textures.update(ctx, &doc, true, None, options, &mut performance);
        });
        let emitted: usize = cold
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| *id != egui::TextureId::Managed(0))
            .map(|(_, delta)| {
                let egui::ImageData::Color(image) = &delta.image;
                image.pixels.len() * std::mem::size_of::<Color32>()
            })
            .sum();
        assert_eq!(performance.display_queued_bytes, emitted);
        assert_eq!(textures.memory().texture_base_rgba_bytes, emitted);
        assert_eq!(textures.memory().resident_tiles, 6);
        assert_eq!(
            textures.memory().texture_with_mips_rgba_estimate_bytes,
            emitted
        );
        assert_eq!(emitted, 6 * 4);
        assert!(performance.display_preparation_peak_bytes > emitted);
        performance.begin_frame();
        let warm = ctx.run(Default::default(), |ctx| {
            textures.update(ctx, &doc, true, None, options, &mut performance);
        });
        assert!(
            warm.textures_delta
                .set
                .iter()
                .all(|(id, _)| *id == egui::TextureId::Managed(0))
        );
        assert_eq!(performance.display_queued_bytes, 0);
        assert_eq!(performance.display_preparation_peak_bytes, 0);
        doc.layers[4]
            .pixels
            .writer()
            .rgba_mut(10 * 513 + 10)
            .copy_from_slice(&[17, 83, 201, 255]);
        let dirty = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                false,
                Some([10, 10, 11, 11]),
                options,
                &mut performance,
            );
        });
        assert!(
            dirty
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| *id != egui::TextureId::Managed(0))
        );
        assert!(textures.memory().prefix_heap_bytes > 0);
        doc = Document::new(1, 1);
        performance.begin_frame();
        let resized = ctx.run(Default::default(), |ctx| {
            textures.update(ctx, &doc, true, None, options, &mut performance);
        });
        assert_eq!(textures.memory().prefix_heap_bytes, 0);
        assert_eq!(textures.memory().resident_tiles, 1);
        assert_eq!(resized.textures_delta.free.len(), 6);
    }

    #[test]
    fn cached_display_matches_compositor_across_blends_clipping_folders_and_replacements() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(257, 33);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(16 * 257 + 255)
            .copy_from_slice(&[140, 30, 90, 180]);
        doc.add_layer();
        doc.layers[1]
            .pixels
            .writer()
            .rgba_mut(16 * 257 + 255)
            .copy_from_slice(&[20, 180, 60, 210]);
        let mut textures = CanvasTextures::default();
        let mut performance = Performance::default();
        let mut retained = std::collections::HashMap::new();
        for step in 0..19 {
            match step {
                0..=11 => doc.layers[1].blend = crate::blend::BlendMode::ALL[step],
                12 => doc.layers[1].clipped = true,
                13 => doc.layers[0].visible = false,
                14 => {
                    doc.layers[0].visible = true;
                    doc.layers[1].opacity = 0.37;
                }
                15 => {
                    doc.layers[0].depth = 1;
                    doc.layers[1].depth = 1;
                    doc.layers
                        .push(crate::document::Layer::new_folder("Group".into()));
                }
                16 => doc.layers[2].opacity = 0.53,
                17 => doc.layers.swap(0, 1),
                18 => doc = Document::new(257, 33),
                _ => unreachable!(),
            }
            let output = ctx.run(Default::default(), |ctx| {
                textures.update(
                    ctx,
                    &doc,
                    true,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut performance,
                );
            });
            retain_images(&mut retained, &output.textures_delta).unwrap();
            for tile in &textures.tiles {
                let expected = egui::ColorImage::from_rgba_unmultiplied(
                    [EXTENT, EXTENT],
                    &rgba(&doc, tile.bounds, [0, 0, EXTENT, EXTENT]),
                );
                assert_eq!(
                    &retained[&tile.texture.as_ref().unwrap().id()],
                    &expected,
                    "step {step}"
                );
            }
        }
    }
    #[test]
    fn full_refresh_reuses_unchanged_tiles_and_invalidates_pixels_and_render_metadata() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(768, 512);
        let mut textures = CanvasTextures::default();
        let mut performance = Performance::default();
        let mut refresh = |doc: &Document| {
            ctx.run(Default::default(), |ctx| {
                textures.update(
                    ctx,
                    doc,
                    true,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut performance,
                );
            })
            .textures_delta
            .set
            .into_iter()
            .filter(|(id, _)| *id != egui::TextureId::Managed(0))
            .count()
        };
        assert_eq!(refresh(&doc), 6);
        assert_eq!(refresh(&doc), 0);
        doc.layers[0].name = "Renamed".into();
        doc.layers[0].locked = true;
        assert_eq!(refresh(&doc), 0);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(100 * 768 + 100)
            .copy_from_slice(&[90, 20, 10, 255]);
        // Allocation identity conservatively invalidates gutters overlapping
        // the changed storage tile, even when their exact pixels are unchanged.
        assert_eq!(refresh(&doc), 4);
        assert_eq!(refresh(&doc), 0);
        doc.layers[0].opacity = 0.5;
        assert_eq!(refresh(&doc), 6);
        doc.layers[0].blend = crate::blend::BlendMode::Multiply;
        assert_eq!(refresh(&doc), 6);
        doc.layers[0].visible = false;
        assert_eq!(refresh(&doc), 6);
        doc.layers[0].pixels = vec![0; 768 * 512 * 4].into();
        assert_eq!(refresh(&doc), 6);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(100 * 768 + 100)
            .copy_from_slice(&[1, 2, 3, 255]);
        assert_eq!(refresh(&doc), 6);
    }
    #[test]
    fn neighboring_tiles_share_identical_border_mips_through_minimum_canvas_zoom() {
        const {
            assert!(crate::view::MIN_ZOOM >= 1.0 / 32.0);
        }
        let mut doc = Document::new(512, 256);
        {
            let mut writer = doc.layers[0].pixels.writer();
            for y in 0..256 {
                for x in 0..512 {
                    writer.rgba_mut(y * 512 + x).copy_from_slice(&[
                        x as u8,
                        y as u8,
                        ((x / 8 + y / 8) % 2 * 255) as u8,
                        255,
                    ]);
                }
            }
        }
        let mut a = rgba(&doc, [0, 0, 256, 256], [0, 0, EXTENT, EXTENT]);
        let mut b = rgba(&doc, [256, 0, 512, 256], [0, 0, EXTENT, EXTENT]);
        let mut size = EXTENT;
        let mut scale = 1;
        for level in 0..=5 {
            for y in 0..size {
                for x in 0..PAD * 2 / scale {
                    let offset_a = (y * size + SIDE / scale + x) * 4;
                    let offset_b = (y * size + x) * 4;
                    assert_eq!(&a[offset_a..offset_a + 4], &b[offset_b..offset_b + 4]);
                }
            }
            if level == 5 {
                break;
            }
            let downsample = |pixels: &[u8]| {
                let mut reduced = vec![0; size / 2 * (size / 2) * 4];
                for y in 0..size / 2 {
                    for x in 0..size / 2 {
                        for channel in 0..4 {
                            let sum: u16 = [
                                ((y * 2) * size + x * 2) * 4,
                                ((y * 2) * size + x * 2 + 1) * 4,
                                ((y * 2 + 1) * size + x * 2) * 4,
                                ((y * 2 + 1) * size + x * 2 + 1) * 4,
                            ]
                            .into_iter()
                            .map(|offset| pixels[offset + channel] as u16)
                            .sum();
                            reduced[(y * (size / 2) + x) * 4 + channel] = (sum / 4) as u8;
                        }
                    }
                }
                reduced
            };
            a = downsample(&a);
            b = downsample(&b);
            size /= 2;
            scale *= 2;
        }
    }
    use super::*;
    #[test]
    fn padded_tile_pixels_and_dirty_updates_match_full_composite_at_boundaries() {
        let mut doc = Document::new(517, 259);
        doc.dab(
            255.5,
            255.5,
            1.0,
            &crate::brush::Brush::default(),
            [40, 90, 140],
            false,
        );
        doc.add_layer();
        doc.layers[1].blend = crate::blend::BlendMode::Multiply;
        let tiles = bounds(doc.width, doc.height);
        let mut cached: Vec<_> = tiles
            .iter()
            .map(|&tile| rgba(&doc, tile, [0, 0, EXTENT, EXTENT]))
            .collect();
        let ctx = egui::Context::default();
        let mut display = CanvasTextures::default();
        let mut performance = Performance::default();
        let output = ctx.run(Default::default(), |ctx| {
            display.update(
                ctx,
                &doc,
                true,
                None,
                egui::TextureOptions::LINEAR,
                &mut performance,
            );
        });
        let mut retained = std::collections::HashMap::new();
        retain_images(&mut retained, &output.textures_delta).unwrap();
        for (tile, expected) in display.tiles.iter().zip(&cached) {
            assert_eq!(
                &retained[&tile.texture.as_ref().unwrap().id()],
                &egui::ColorImage::from_rgba_unmultiplied([EXTENT, EXTENT], expected)
            );
        }
        for (x, y, dirty) in [
            (255.5, 255.5, [242, 242, 270, 259]),
            (516.5, 258.5, [503, 245, 517, 259]),
            (0.5, 0.5, [0, 0, 15, 15]),
        ] {
            doc.dab(
                x,
                y,
                1.0,
                &crate::brush::Brush::default(),
                [180, 60, 90],
                false,
            );
            let source = doc.composite_region(dirty[0], dirty[1], dirty[2], dirty[3]);
            for (tile, pixels) in tiles.iter().zip(&mut cached) {
                if let Some(local) = dirty_rect(*tile, dirty, doc.width, doc.height) {
                    let patch = sampled_rgba(&doc, *tile, local, dirty, &source);
                    let row_bytes = (local[2] - local[0]) * 4;
                    for (band, py) in (local[1]..local[3]).enumerate() {
                        let offset = (py * EXTENT + local[0]) * 4;
                        pixels[offset..offset + row_bytes]
                            .copy_from_slice(&patch[band * row_bytes..(band + 1) * row_bytes]);
                    }
                }
                assert_eq!(*pixels, rgba(&doc, *tile, [0, 0, EXTENT, EXTENT]));
            }
        }
        let full = doc.composite();
        for (tile, pixels) in tiles.iter().zip(&cached) {
            for y in 0..EXTENT {
                for x in 0..EXTENT {
                    let sx = (tile[0] as isize + x as isize - PAD as isize)
                        .clamp(0, doc.width as isize - 1) as usize;
                    let sy = (tile[1] as isize + y as isize - PAD as isize)
                        .clamp(0, doc.height as isize - 1) as usize;
                    assert_eq!(
                        &pixels[(y * EXTENT + x) * 4..][..4],
                        &full[(sy * doc.width + sx) * 4..][..4]
                    );
                }
            }
        }
    }
    #[test]
    fn texture_updates_are_bounded_partial_and_filter_changes_reach_every_tile() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(517, 259);
        let mut textures = CanvasTextures::default();
        let mut performance = Performance::default();
        let options = egui::TextureOptions::LINEAR;
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(ctx, &doc, false, None, options, &mut performance)
        });
        let ids: std::collections::HashSet<_> = textures
            .tiles
            .iter()
            .map(|tile| tile.texture.as_ref().unwrap().id())
            .collect();
        let deltas: Vec<_> = output
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .collect();
        assert_eq!(deltas.len(), 6);
        assert!(
            deltas
                .iter()
                .all(|(_, delta)| delta.pos.is_none() && delta.image.size() == [1, 1])
        );
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(100 * 517 + 100)
            .copy_from_slice(&[10, 20, 30, 255]);
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                false,
                Some([100, 100, 101, 101]),
                options,
                &mut performance,
            )
        });
        let deltas: Vec<_> = output
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .collect();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].1.image.size(), [EXTENT, EXTENT]);
        assert!(deltas[0].1.pos.is_none());
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[5, 6, 7, 255]);
        let _ = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                false,
                Some([0, 0, 1, 1]),
                options,
                &mut performance,
            )
        });
        assert_eq!(textures.tiles[0].first, Color32::from_rgb(5, 6, 7));
        let output = ctx.run(Default::default(), |ctx| {
            textures.update(
                ctx,
                &doc,
                false,
                None,
                egui::TextureOptions::NEAREST,
                &mut performance,
            )
        });
        let deltas: Vec<_> = output
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .collect();
        assert_eq!(deltas.len(), 6);
        assert!(
            deltas
                .iter()
                .all(|(_, delta)| delta.options == egui::TextureOptions::NEAREST)
        );
        doc = Document::new(1, 1);
        let _ = ctx.run(Default::default(), |ctx| {
            textures.update(ctx, &doc, true, None, options, &mut performance)
        });
        assert_eq!(textures.tiles.len(), 1);
    }
}

/// How long hiding and showing layers takes to appear on a real file:
/// `DUDEL_TOGGLE_FILE=FILE.clip cargo test --release toggle_latency -- --ignored --nocapture`.
#[cfg(test)]
mod toggle_latency {
    use super::*;
    #[test]
    #[ignore]
    fn toggle_latency_on_a_real_file() {
        let path = std::env::var("DUDEL_TOGGLE_FILE").unwrap();
        let mut doc = crate::clip_metadata::import_document(std::path::Path::new(&path), &|| false)
            .unwrap()
            .document;
        let ctx = egui::Context::default();
        let mut textures = CanvasTextures::default();
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(1400.0, 900.0));
        let zoom = 900.0 / doc.height as f32;
        textures.set_viewport(
            area,
            crate::view::View {
                origin: area.center(),
                zoom,
                rotation: 0.0,
                mirror: false,
                half: egui::vec2(doc.width as f32 / 2.0, doc.height as f32 / 2.0),
            },
        );
        let frame = |textures: &mut CanvasTextures, doc: &Document| {
            let _ = ctx.run(Default::default(), |ctx| {
                textures.update_sliced(
                    ctx,
                    doc,
                    false,
                    None,
                    egui::TextureOptions::LINEAR,
                    &mut Performance::default(),
                );
            });
        };
        let shown = |t: &CanvasTextures| {
            t.presented
                .iter()
                .map(|p| p.texture.id())
                .collect::<Vec<_>>()
        };
        let started = std::time::Instant::now();
        while textures.presented.is_empty() {
            frame(&mut textures, &doc);
        }
        println!(
            "open: first frame {:.0} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        while textures.busy() {
            frame(&mut textures, &doc);
        }
        println!(
            "open: exact {:.0} ms (zoom {zoom:.3}, levels {:?})",
            started.elapsed().as_secs_f64() * 1000.0,
            textures
                .levels
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
        for name in ["Layer 10", "Tone 4", "Tone 6", "Layer 2 2"] {
            for visible in [false, true] {
                let index = doc.layers.iter().position(|l| l.name == name).unwrap();
                doc.layers[index].visible = visible;
                let before = shown(&textures);
                let started = std::time::Instant::now();
                frame(&mut textures, &doc);
                while shown(&textures) == before && textures.busy() {
                    frame(&mut textures, &doc);
                }
                let first = started.elapsed().as_secs_f64() * 1000.0;
                while textures.busy() {
                    frame(&mut textures, &doc);
                }
                println!(
                    "{name:10} visible={visible}: shown {first:.0} ms, exact {:.0} ms",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
    }
}
