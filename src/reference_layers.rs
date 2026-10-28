//! Reference layers: images placed on the canvas to draw from. They are app
//! state saved with projects, never artwork: compositing, export, fill and
//! history do not see them, and undoing artwork never moves them.
use crate::{Dudel, egui, view::View};
use egui::{Color32, Pos2, Shape, Stroke, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const MAX_REFERENCES: usize = 8;
pub(crate) const MAX_IMAGES: usize = 12;
const IMPORT_BUDGET: usize = 128 * 1024 * 1024;
/// Imported images are reduced to this longest edge; references are for
/// viewing; new imports share a 128 MiB project-wide RGBA budget.
const IMPORT_SIDE: usize = 1536;
/// Largest stored side a project may carry.
const MAX_SIDE: usize = 4096;
const MAX_NAME_BYTES: usize = 256;
const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;
const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 64.0;
/// Project trailer holding reference images after the artwork records.
pub(crate) const TRAILER: &[u8; 5] = b"REFS\x01";
/// New placement/board data must not be silently discarded by older readers.
pub(crate) const STACK_TRAILER: &[u8; 5] = b"REFS\x02";
/// Screen distance of the rotation handle beyond the reference's top edge.
const ROTATE_REACH: f32 = 26.0;
const HANDLE_RADIUS: f32 = 10.0;

/// Straight (unpremultiplied) RGBA pixels.
#[derive(Default, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ReferenceLayer {
    /// Session identity for selection and textures; never saved.
    #[serde(skip)]
    pub id: u64,
    pub name: String,
    /// Image center in document pixels.
    pub center: [f32; 2],
    /// Document pixels per image pixel.
    pub scale: f32,
    /// Clockwise degrees.
    pub rotation: f32,
    pub opacity: f32,
    pub visible: bool,
    pub locked: bool,
    /// Drawn over the artwork rather than between paper and artwork.
    pub above: bool,
    #[serde(default)]
    pub flip: bool,
    #[serde(skip)]
    pub image: Arc<Image>,
    /// Additional placements on this reference board (the primary image above
    /// keeps existing project files compatible). No nested boards.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ReferenceLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<crate::reference_stack::Position>,
}

impl ReferenceLayer {
    pub(crate) fn placements(&self) -> impl Iterator<Item = &Self> {
        std::iter::once(self).chain(&self.images)
    }
    fn axes(&self) -> (Vec2, Vec2) {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        (vec2(cos, sin), vec2(-sin, cos))
    }
    /// Corners in document pixels: image top-left, top-right, bottom-right,
    /// bottom-left.
    pub(crate) fn corners(&self) -> [Pos2; 4] {
        let (x, y) = self.axes();
        let half = vec2(self.image.width as f32, self.image.height as f32) * self.scale / 2.0;
        let center = pos2(self.center[0], self.center[1]);
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(sx, sy)| center + x * half.x * sx + y * half.y * sy)
    }
    fn contains(&self, point: Pos2) -> bool {
        let (x, y) = self.axes();
        let offset = point - pos2(self.center[0], self.center[1]);
        let half = vec2(self.image.width as f32, self.image.height as f32) * self.scale / 2.0;
        offset.dot(x).abs() <= half.x && offset.dot(y).abs() <= half.y
    }
    /// Scale and center that fit the image inside a canvas with a margin.
    fn fit(&mut self, width: usize, height: usize) {
        let factor = (width as f32 / self.image.width as f32)
            .min(height as f32 / self.image.height as f32)
            * 0.9;
        self.scale = factor.clamp(MIN_SCALE, MAX_SCALE);
        self.center = [width as f32 / 2.0, height as f32 / 2.0];
        self.rotation = 0.0;
    }
}

/// Checks reference metadata, and the images too once they are attached.
pub(crate) fn validate(references: &[ReferenceLayer], images: bool) -> Result<(), String> {
    if references.len() > MAX_REFERENCES {
        return Err(format!(
            "At most {MAX_REFERENCES} reference layers are supported"
        ));
    }
    let mut total = 0usize;
    for board in references {
        if board.images.len() >= MAX_IMAGES
            || board
                .images
                .iter()
                .any(|i| !i.images.is_empty() || i.position.is_some())
        {
            return Err(format!(
                "Reference layers support at most {MAX_IMAGES} images, without nested boards"
            ));
        }
        if let Some(position) = &board.position {
            position.validate()?;
        }
        for reference in board.placements() {
            let finite = reference
                .center
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1.0e6)
                && reference.rotation.is_finite();
            if reference.name.len() > MAX_NAME_BYTES
                || !finite
                || !(MIN_SCALE..=MAX_SCALE).contains(&reference.scale)
                || !(0.0..=1.0).contains(&reference.opacity)
            {
                return Err("Invalid reference layer settings".into());
            }
            let image = &reference.image;
            total = total.saturating_add(image.rgba.len());
            if images
                && (image.width == 0
                    || image.height == 0
                    || image.width > MAX_SIDE
                    || image.height > MAX_SIDE
                    || image.rgba.len() != image.width * image.height * 4)
            {
                return Err("Invalid reference image".into());
            }
        }
    }
    // Retain compatibility with the old eight 4096-square image ceiling.
    if total > 512 * 1024 * 1024 {
        return Err("Reference images exceed the project memory limit".into());
    }
    Ok(())
}

/// Append the reference images as PNG records after the artwork payload.
pub(crate) fn encode_trailer(
    references: &[ReferenceLayer],
    writer: &mut impl std::io::Write,
    cancel: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    use image::ImageEncoder;
    if references.is_empty() {
        return Ok(());
    }
    validate(references, true)?;
    let tag = if references
        .iter()
        .any(|b| b.position.is_some() || !b.images.is_empty())
    {
        STACK_TRAILER
    } else {
        TRAILER
    };
    writer.write_all(tag).map_err(|e| e.to_string())?;
    writer
        .write_all(
            &(references.iter().map(|r| 1 + r.images.len()).sum::<usize>() as u32).to_le_bytes(),
        )
        .map_err(|e| e.to_string())?;
    for reference in references.iter().flat_map(ReferenceLayer::placements) {
        cancel()?;
        let image = &reference.image;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(
            &image.rgba,
            image.width as u32,
            image.height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("Reference image encoding failed: {e}"))?;
        writer
            .write_all(&(png.len() as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        writer.write_all(&png).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Read the images for metadata-declared references, after the trailer tag.
pub(crate) fn decode_trailer(
    references: &mut [ReferenceLayer],
    reader: &mut impl std::io::Read,
) -> Result<(), String> {
    let mut word = [0u8; 4];
    let mut read_u32 = |reader: &mut dyn std::io::Read| -> Result<usize, String> {
        reader
            .read_exact(&mut word)
            .map_err(|e| format!("Damaged reference images: {e}"))?;
        Ok(u32::from_le_bytes(word) as usize)
    };
    if read_u32(reader)? != references.iter().map(|r| 1 + r.images.len()).sum::<usize>()
        || references.is_empty()
    {
        return Err("Reference images do not match project metadata".into());
    }
    let mut total = 0usize;
    for reference in references.iter_mut().flat_map(|r| {
        std::iter::once(&mut r.image).chain(r.images.iter_mut().map(|i| &mut i.image))
    }) {
        let length = read_u32(reader)?;
        if length == 0 || length > MAX_PNG_BYTES {
            return Err("Invalid reference image record".into());
        }
        let mut png = vec![0; length];
        reader
            .read_exact(&mut png)
            .map_err(|e| format!("Damaged reference images: {e}"))?;
        let mut decoder =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_SIDE as u32);
        limits.max_image_height = Some(MAX_SIDE as u32);
        limits.max_alloc = Some((MAX_SIDE * MAX_SIDE * 4) as u64);
        decoder.limits(limits);
        let rgba = decoder
            .decode()
            .map_err(|e| format!("Damaged reference image: {e}"))?
            .into_rgba8();
        total += rgba.len();
        if total > 512 * 1024 * 1024 {
            return Err("Reference images exceed the project memory limit".into());
        }
        *reference = Arc::new(Image {
            width: rgba.width() as usize,
            height: rgba.height() as usize,
            rgba: rgba.into_raw(),
        });
    }
    validate(references, true)
}

#[derive(Clone, Copy)]
enum Drag {
    Move { grab: Pos2, center: [f32; 2] },
    Scale { distance: f32, scale: f32 },
    Rotate { angle: f32, rotation: f32 },
}

type ImportedImages = Result<Vec<(String, Image)>, String>;
struct ImportJob {
    target: u64,
    worker: Option<std::thread::JoinHandle<ImportedImages>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for ImportJob {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Live reference layers and their editing state, owned by the app.
#[derive(Default)]
pub struct State {
    pub(crate) layers: Vec<ReferenceLayer>,
    /// Selected reference; while set, canvas drags edit it instead of painting.
    pub(crate) active: Option<u64>,
    pub(crate) active_image: usize,
    next_id: u64,
    revision: u64,
    saved_revision: u64,
    pub(crate) saving_revision: Option<u64>,
    textures: Vec<(u64, usize, egui::TextureHandle)>,
    drag: Option<Drag>,
    pub(crate) scene_cache: Vec<crate::reference_stack::CachedBoard>,
    pub(crate) blocked: Vec<u64>,
    pub(crate) hidden: Vec<u64>,
    pub(crate) hit_order: Vec<u64>,
    import_job: Option<ImportJob>,
}

impl State {
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        registry.add_owned(self.layers.capacity() * std::mem::size_of::<ReferenceLayer>());
        for board in &self.layers {
            registry.add_owned(board.images.capacity() * std::mem::size_of::<ReferenceLayer>());
            for reference in board.placements() {
                registry.add_owned(reference.name.capacity());
                registry.add_buffer(
                    reference.image.rgba.as_ptr() as usize,
                    reference.image.rgba.capacity(),
                );
            }
        }
        for board in &self.scene_cache {
            board.register_allocations(registry);
        }
    }

    pub(crate) fn visit_textures(&self, mut visit: impl FnMut(&egui::TextureHandle)) {
        for (_, _, texture) in &self.textures {
            visit(texture);
        }
    }

    pub(crate) fn importing_target(&self) -> Option<u64> {
        self.import_job.as_ref().map(|job| job.target)
    }
    fn assign_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    pub(crate) fn changed(&self) -> bool {
        self.revision != self.saved_revision
    }
    /// A batch of reference images is decoding in the background.
    pub(crate) fn importing(&self) -> bool {
        self.import_job.is_some()
    }
    pub(crate) fn begin_save(&mut self) -> Vec<ReferenceLayer> {
        self.saving_revision = Some(self.revision);
        self.layers.clone()
    }
    pub(crate) fn finish_save(&mut self, saved: bool) {
        if let Some(revision) = self.saving_revision.take()
            && saved
        {
            self.saved_revision = revision;
        }
    }
    /// Install the references of a newly loaded or created document.
    pub(crate) fn install(&mut self, mut layers: Vec<ReferenceLayer>) {
        if let Some(job) = &self.import_job {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        for layer in &mut layers {
            layer.id = self.assign_id();
            for image in &mut layer.images {
                image.id = self.assign_id();
            }
        }
        self.layers = layers;
        self.active = None;
        self.drag = None;
        self.active_image = 0;
        self.scene_cache.clear();
        self.saving_revision = None;
        self.revision = self.revision.wrapping_add(1);
        self.saved_revision = self.revision;
    }
    pub(crate) fn index(&self, id: Option<u64>) -> Option<usize> {
        id.and_then(|id| self.layers.iter().position(|layer| layer.id == id))
    }
    fn selected(&self) -> Option<&ReferenceLayer> {
        let board = &self.layers[self.index(self.active)?];
        if self.active_image == 0 {
            Some(board)
        } else {
            board.images.get(self.active_image - 1)
        }
    }
    fn selected_mut(&mut self) -> Option<&mut ReferenceLayer> {
        let index = self.index(self.active)?;
        if self.active_image == 0 {
            Some(&mut self.layers[index])
        } else {
            self.layers[index].images.get_mut(self.active_image - 1)
        }
    }
    pub(crate) fn below_visible(&self) -> bool {
        self.layers
            .iter()
            .any(|layer| layer.visible && !layer.above)
    }
    /// What a press at screen `pos` takes hold of: the selected reference's
    /// handles first, then the topmost visible, unlocked reference under it.
    fn grab(&self, view: View, pos: Pos2) -> Option<(u64, usize, Drag)> {
        let at = view.to_doc(pos);
        if let Some(index) = self.index(self.active) {
            let board = &self.layers[index];
            if !board.locked
                && board.visible
                && !self.blocked.contains(&board.id)
                && let Some(layer) = self.selected()
            {
                let h = handles(layer, view);
                let center = pos2(layer.center[0], layer.center[1]);
                if (pos - h.rotate).length() <= HANDLE_RADIUS {
                    return Some((
                        board.id,
                        self.active_image,
                        Drag::Rotate {
                            angle: angle(h.center, pos),
                            rotation: layer.rotation,
                        },
                    ));
                }
                if h.corners
                    .iter()
                    .any(|c| (pos - *c).length() <= HANDLE_RADIUS)
                {
                    return Some((
                        board.id,
                        self.active_image,
                        Drag::Scale {
                            distance: (at - center).length().max(1e-3),
                            scale: layer.scale,
                        },
                    ));
                }
            }
        }
        let mut boards: Vec<_> = self.layers.iter().enumerate().collect();
        boards.sort_by_key(|(index, b)| {
            self.hit_order
                .iter()
                .position(|id| *id == b.id)
                .unwrap_or(if b.above {
                    self.layers.len() + *index
                } else {
                    *index
                })
        });
        for (_, board) in boards
            .into_iter()
            .rev()
            .filter(|(_, b)| b.visible && !b.locked && !self.blocked.contains(&b.id))
        {
            for (image, layer) in board
                .placements()
                .enumerate()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                if (image == 0 || layer.visible) && layer.contains(at) {
                    return Some((
                        board.id,
                        image,
                        Drag::Move {
                            grab: at,
                            center: layer.center,
                        },
                    ));
                }
            }
        }
        None
    }
    /// Keep one texture per reference image, uploading new images once.
    fn sync_textures(&mut self, ctx: &egui::Context, size: [usize; 2]) {
        let stacked = self.layers.iter().any(|b| b.position.is_some());
        let bounds = egui::Rect::from_min_max(Pos2::ZERO, pos2(size[0] as f32, size[1] as f32));
        let images: Vec<_> = self
            .layers
            .iter()
            .flat_map(|b| {
                let thumbnail = stacked
                    && b.placements()
                        .all(|p| bounds.contains_rect(egui::Rect::from_points(&p.corners())));
                b.placements().map(move |p| {
                    (
                        p.id,
                        Arc::as_ptr(&p.image) as usize | usize::from(thumbnail),
                        p.image.clone(),
                        thumbnail,
                    )
                })
            })
            .collect();
        self.textures
            .retain(|(id, key, _)| images.iter().any(|(i, k, _, _)| i == id && k == key));
        for (id, key, image, thumbnail) in images {
            if self.textures.iter().any(|(i, _, _)| *i == id) {
                continue;
            }
            let pixels = if thumbnail {
                let source = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                    image.width as u32,
                    image.height as u32,
                    image.rgba.as_slice(),
                )
                .unwrap();
                let thumb = image::imageops::thumbnail(&source, 128, 128);
                egui::ColorImage::from_rgba_unmultiplied(
                    [thumb.width() as usize, thumb.height() as usize],
                    thumb.as_raw(),
                )
            } else {
                egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba)
            };
            let texture = ctx.load_texture(
                format!("reference-{id}"),
                pixels,
                egui::TextureOptions {
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                    ..egui::TextureOptions::LINEAR
                },
            );
            self.textures.push((id, key, texture));
        }
    }
    pub(crate) fn texture(&self, id: u64) -> Option<&egui::TextureHandle> {
        self.textures
            .iter()
            .find(|(texture, _, _)| *texture == id)
            .map(|(_, _, texture)| texture)
    }
    /// Textured quads for the visible references on one side of the artwork.
    pub(crate) fn shapes(&self, view: View, above: bool) -> Vec<Shape> {
        if self.layers.iter().any(|b| b.position.is_some()) {
            return Vec::new();
        }
        self.layers
            .iter()
            .filter(|board| {
                board.position.is_none()
                    && board.visible
                    && board.above == above
                    && board.opacity > 0.0
            })
            .flat_map(|board| {
                board
                    .placements()
                    .enumerate()
                    .filter(|(i, p)| *i == 0 || p.visible)
                    .map(move |(i, p)| (p, board.opacity * if i == 0 { 1.0 } else { p.opacity }))
            })
            .filter_map(|(layer, opacity)| {
                let texture = self.texture(layer.id)?;
                let mut mesh = egui::Mesh::with_texture(texture.id());
                let uvs = if layer.flip {
                    [
                        pos2(1.0, 0.0),
                        pos2(0.0, 0.0),
                        pos2(0.0, 1.0),
                        pos2(1.0, 1.0),
                    ]
                } else {
                    [
                        pos2(0.0, 0.0),
                        pos2(1.0, 0.0),
                        pos2(1.0, 1.0),
                        pos2(0.0, 1.0),
                    ]
                };
                let color = Color32::WHITE.gamma_multiply(opacity);
                for (corner, uv) in layer.corners().into_iter().zip(uvs) {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: view.to_screen(corner),
                        uv,
                        color,
                    });
                }
                mesh.indices = vec![0, 1, 2, 0, 2, 3];
                Some(Shape::mesh(mesh))
            })
            .collect()
    }
}

/// Screen positions of the active reference's handles.
struct Handles {
    corners: [Pos2; 4],
    top: Pos2,
    rotate: Pos2,
    center: Pos2,
}
fn handles(layer: &ReferenceLayer, view: View) -> Handles {
    let corners = layer.corners().map(|p| view.to_screen(p));
    let center = view.to_screen(pos2(layer.center[0], layer.center[1]));
    let top = corners[0] + (corners[1] - corners[0]) / 2.0;
    let outward = (top - center).normalized();
    Handles {
        corners,
        top,
        rotate: top + outward * ROTATE_REACH,
        center,
    }
}

fn angle(center: Pos2, point: Pos2) -> f32 {
    (point.y - center.y).atan2(point.x - center.x).to_degrees()
}

impl Dudel {
    pub(crate) fn references_edited(&mut self) {
        self.reference_layers.revision = self.reference_layers.revision.wrapping_add(1);
        self.metadata_changed();
    }
    pub(crate) fn references_changed(&self) -> bool {
        self.reference_layers.changed()
    }
    /// Per-frame upkeep: textures, and display tiles with alpha whenever a
    /// reference must show between the paper and the artwork.
    pub(crate) fn prepare_reference_layers(&mut self, ctx: &egui::Context) {
        self.poll_reference_import(ctx);
        self.reference_layers.bind_positions(&self.doc);
        self.reference_layers
            .sync_textures(ctx, [self.doc.width, self.doc.height]);
        let stacked = self
            .reference_layers
            .layers
            .iter()
            .any(|b| b.position.is_some());
        let alpha = self.show_transparency || (!stacked && self.reference_layers.below_visible());
        if self.texture.set_transparent(alpha) {
            self.dirty = true;
        }
        self.texture.set_checker(self.show_transparency);
        if self.reference_layers.active.is_some()
            && self
                .reference_layers
                .index(self.reference_layers.active)
                .is_none()
        {
            self.reference_layers.active = None;
        }
        if self.reference_layers.active.is_some()
            && !ctx.wants_keyboard_input()
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.deselect_reference();
        }
    }
    /// Paper or checkerboard, references below, artwork, references above.
    pub(crate) fn layered_canvas_shapes(
        &self,
        area: egui::Rect,
        view: View,
        overview: bool,
    ) -> Vec<Shape> {
        let mut shapes = self.texture.backdrop_shapes(area, view);
        shapes.extend(self.reference_layers.shapes(view, false));
        shapes.extend(if overview {
            self.texture.overview_shapes(area, view)
        } else {
            self.texture.tile_shapes(area, view)
        });
        shapes.extend(self.reference_layers.shapes(view, true));
        shapes.extend(self.reference_margin_shapes(view));
        shapes
    }
    pub(crate) fn deselect_reference(&mut self) {
        if self.reference_layers.active.take().is_some() {
            self.reference_layers.drag = None;
            self.reference_layers.active_image = 0;
            self.status = "Reference deselected · drawing on the artwork".into();
        }
    }
    /// Select a reference for editing with the Pointer tool (O).
    pub(crate) fn select_reference(&mut self, id: u64) {
        self.finish_painted_stroke();
        if self.tool != crate::tools::Tool::Pointer {
            self.select_tool(crate::tools::Tool::Pointer);
        }
        if self.reference_layers.active != Some(id) {
            self.reference_layers.active_image = 0;
        }
        self.reference_layers.active = Some(id);
        self.reference_layers.drag = None;
        self.status = "Pointer (O): drag the reference to move it, a corner to scale, the top handle to rotate · Esc deselects".into();
    }
    pub(crate) fn add_reference_layer(&mut self) {
        if self.reference_layers.layers.len() >= MAX_REFERENCES {
            self.status = format!("At most {MAX_REFERENCES} reference layers per project");
            return;
        }
        let dialog =
            rfd::AsyncFileDialog::new().add_filter("Images", &crate::image_paste::EXTENSIONS);
        self.ask_open_file(dialog, Self::add_reference_layer_from);
    }
    pub(crate) fn add_reference_layer_from(&mut self, path: std::path::PathBuf) {
        match import(&path) {
            Ok(image) => {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.add_reference_image_layer(name, image);
            }
            Err(error) => self.status = format!("Reference: {error}"),
        }
    }
    /// A new reference layer above the current artwork layer.
    pub(crate) fn add_reference_image_layer(&mut self, name: String, image: Image) {
        let before = self.reference_layers.layers.len();
        self.insert_reference(name, image);
        if self.reference_layers.layers.len() > before {
            let id = self.reference_layers.layers.last().unwrap().id;
            let active = self.doc.active;
            let layer = &self.doc.layers[active];
            let (gap, depth) =
                if layer.folder && !layer.collapsed && layer.depth < crate::document::MAX_DEPTH {
                    (active, layer.depth + 1)
                } else {
                    (active + 1, layer.depth)
                };
            self.place_reference(id, gap, depth);
        }
    }
    pub(crate) fn insert_reference(&mut self, name: String, image: Image) {
        if let Err(error) = validate_new_image(&image) {
            self.status = error;
            return;
        }
        if self.reference_layers.layers.len() >= MAX_REFERENCES {
            self.status = format!("At most {MAX_REFERENCES} reference layers per project");
            return;
        }
        if self
            .reference_image_bytes()
            .saturating_add(image.rgba.len())
            > IMPORT_BUDGET
        {
            self.status =
                "Reference image memory budget reached (128 MiB); remove an image first".into();
            return;
        }
        let mut name = name;
        while name.len() > MAX_NAME_BYTES {
            name.pop();
        }
        let mut layer = ReferenceLayer {
            id: self.reference_layers.assign_id(),
            name: if name.is_empty() {
                "Reference".into()
            } else {
                name
            },
            center: [0.0; 2],
            scale: 1.0,
            rotation: 0.0,
            opacity: 0.6,
            visible: true,
            locked: false,
            above: true,
            flip: false,
            image: Arc::new(image),
            images: Vec::new(),
            position: None,
        };
        layer.fit(self.doc.width, self.doc.height);
        let id = layer.id;
        self.reference_layers.layers.push(layer);
        self.references_edited();
        self.select_reference(id);
    }
    fn reference_image_bytes(&self) -> usize {
        self.reference_layers
            .layers
            .iter()
            .flat_map(ReferenceLayer::placements)
            .map(|p| p.image.rgba.len())
            .sum()
    }
    pub(crate) fn insert_reference_image(
        &mut self,
        id: u64,
        name: String,
        image: Image,
    ) -> Result<(), String> {
        validate_new_image(&image)?;
        let mut name = name;
        while name.len() > MAX_NAME_BYTES {
            name.pop();
        }
        let index = self
            .reference_layers
            .index(Some(id))
            .ok_or("Reference layer no longer exists")?;
        if self.reference_layers.layers[index].images.len() + 1 >= MAX_IMAGES {
            return Err(format!("At most {MAX_IMAGES} images per reference layer"));
        }
        if self
            .reference_image_bytes()
            .saturating_add(image.rgba.len())
            > IMPORT_BUDGET
        {
            return Err("Reference image memory budget reached (128 MiB)".into());
        }
        let mut placement = self.reference_layers.layers[index].clone();
        placement.images.clear();
        placement.position = None;
        placement.id = self.reference_layers.assign_id();
        placement.name = name;
        placement.image = Arc::new(image);
        placement.opacity = 1.0;
        placement.visible = true;
        placement.locked = false;
        placement.fit(self.doc.width, self.doc.height);
        self.reference_layers.layers[index].images.push(placement);
        if self.reference_layers.active == Some(id) {
            self.reference_layers.active_image = self.reference_layers.layers[index].images.len();
        }
        self.references_edited();
        Ok(())
    }
    fn add_reference_images(&mut self, id: u64) {
        let Some(index) = self.reference_layers.index(Some(id)) else {
            return;
        };
        if self.reference_layers.layers[index].images.len() + 1 >= MAX_IMAGES {
            self.status = format!("This reference layer already holds {MAX_IMAGES} images");
            return;
        }
        let dialog =
            rfd::AsyncFileDialog::new().add_filter("Images", &crate::image_paste::EXTENSIONS);
        self.ask_open_files(dialog, move |app, paths| {
            // The board may have changed while the dialog was open.
            let Some(index) = app.reference_layers.index(Some(id)) else {
                return;
            };
            let available =
                (MAX_IMAGES - 1).saturating_sub(app.reference_layers.layers[index].images.len());
            if paths.len() > available {
                app.status =
                    format!("Choose at most {available} more images for this reference layer");
                return;
            }
            app.queue_reference_import(id, paths);
        });
    }
    fn queue_reference_import(&mut self, id: u64, paths: Vec<std::path::PathBuf>) {
        if self.reference_layers.import_job.is_some() {
            self.status = "A reference import is already running".into();
            return;
        }
        let Some(index) = self.reference_layers.index(Some(id)) else {
            return;
        };
        if paths.is_empty()
            || paths.len() > MAX_IMAGES - 1 - self.reference_layers.layers[index].images.len()
        {
            return;
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancel.clone();
        let mut bytes = self.reference_image_bytes();
        let worker = std::thread::Builder::new()
            .name("dudel-reference-import".into())
            .spawn(move || {
                let mut pending = Vec::new();
                for path in paths {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err("Reference import canceled".into());
                    }
                    let image = import(&path)?;
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err("Reference import canceled".into());
                    }
                    bytes = bytes.saturating_add(image.rgba.len());
                    if bytes > IMPORT_BUDGET {
                        return Err("Reference image memory budget reached (128 MiB)".into());
                    }
                    pending.push((
                        path.file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        image,
                    ));
                }
                Ok(pending)
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                self.status = format!("Reference import could not start: {error}");
                return;
            }
        };
        self.reference_layers.import_job = Some(ImportJob {
            target: id,
            worker: Some(worker),
            cancel,
        });
        self.status = "Importing reference images...".into();
    }
    fn poll_reference_import(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.reference_layers.import_job else {
            return;
        };
        if self.reference_layers.index(Some(job.target)).is_none() {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if !job.worker.as_ref().unwrap().is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
            return;
        }
        let mut job = self.reference_layers.import_job.take().unwrap();
        let result = job
            .worker
            .take()
            .unwrap()
            .join()
            .unwrap_or_else(|_| Err("Reference import worker failed".into()));
        if job.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            self.status = "Reference import canceled".into();
            return;
        }
        let Some(index) = self.reference_layers.index(Some(job.target)) else {
            self.status = "Reference layer was removed; import canceled".into();
            return;
        };
        let pending = match result {
            Ok(images) => images,
            Err(e) => {
                self.status = format!("Reference: {e}");
                return;
            }
        };
        let mut locked = self.reference_layers.layers[index].locked;
        let mut parent = self
            .reference_layers
            .location(&self.reference_layers.layers[index], &self.doc)
            .2;
        while let Some(index) = parent {
            locked |= self.doc.layers[index].locked;
            parent = self.doc.parent(index);
        }
        let bytes = pending.iter().map(|(_, p)| p.rgba.len()).sum::<usize>();
        if locked
            || pending.len() + 1 + self.reference_layers.layers[index].images.len() > MAX_IMAGES
            || self.reference_image_bytes().saturating_add(bytes) > IMPORT_BUDGET
        {
            self.status =
                "Reference layer locked, changed or image budget reached; import canceled".into();
            return;
        }
        for (name, image) in pending {
            // All images and budgets were validated before publishing the batch.
            if let Err(error) = self.insert_reference_image(job.target, name, image) {
                self.status = error;
                return;
            }
        }
        self.arrange_reference_images(job.target);
        self.status = "Reference images added".into();
        ctx.request_repaint();
    }
    pub(crate) fn arrange_reference_images(&mut self, id: u64) {
        let Some(index) = self.reference_layers.index(Some(id)) else {
            return;
        };
        let board = &mut self.reference_layers.layers[index];
        let count = board.images.len() + 1;
        let columns = (count as f32).sqrt().ceil() as usize;
        let rows = count.div_ceil(columns);
        let cell = [
            self.doc.width as f32 / columns as f32,
            self.doc.height as f32 / rows as f32,
        ];
        let place = |i: usize, p: &mut ReferenceLayer| {
            p.center = [
                (i % columns) as f32 * cell[0] + cell[0] / 2.0,
                (i / columns) as f32 * cell[1] + cell[1] / 2.0,
            ];
            p.scale = (cell[0] / p.image.width as f32).min(cell[1] / p.image.height as f32) * 0.85;
            p.scale = p.scale.clamp(MIN_SCALE, MAX_SCALE);
            p.rotation = 0.0;
        };
        // Borrow the primary placement separately from its extra placements.
        let mut extra = std::mem::take(&mut board.images);
        place(0, board);
        for (i, p) in extra.iter_mut().enumerate() {
            place(i + 1, p);
        }
        board.images = extra;
        self.references_edited();
    }
    /// Follow a canvas crop, resize anchor or rotation like the guides do.
    pub(crate) fn map_references(
        &mut self,
        before: crate::canvas_size::Frame,
        after: crate::canvas_size::Frame,
    ) {
        for layer in self.reference_layers.layers.iter_mut().flat_map(|b| {
            std::iter::once((&mut b.center, &mut b.rotation)).chain(
                b.images
                    .iter_mut()
                    .map(|p| (&mut p.center, &mut p.rotation)),
            )
        }) {
            *layer.0 = after.point_from(before, *layer.0);
            *layer.1 += after.angle_from(before);
        }
    }
    /// Whether a Pointer press at `pos` would grab a reference or a handle,
    /// so the Pointer does not also pan.
    pub(crate) fn reference_grabs(&self, view: View, pos: Pos2) -> bool {
        self.reference_layers.drag.is_some() || self.reference_layers.grab(view, pos).is_some()
    }
    /// With the Pointer tool (O), presses on a reference select and move it,
    /// and the selected reference's handles scale and rotate it. Other tools
    /// never move references. Returns whether the packet was consumed.
    pub(crate) fn reference_packet(
        &mut self,
        view: View,
        pos: Pos2,
        start: bool,
        end: bool,
        on_area: bool,
        shift: bool,
    ) -> bool {
        if self.tool != crate::tools::Tool::Pointer {
            self.reference_layers.drag = None;
            return false;
        }
        if start {
            self.reference_layers.drag = None;
            if !on_area {
                return false;
            }
            let Some((id, image, drag)) = self.reference_layers.grab(view, pos) else {
                // Empty canvas: deselect and let the Pointer pan or pick guides.
                self.deselect_reference();
                return false;
            };
            if self.reference_layers.active != Some(id) {
                self.select_reference(id);
            }
            self.reference_layers.active_image = image;
            self.reference_layers.drag = Some(drag);
            return true;
        }
        let Some(drag) = self.reference_layers.drag else {
            return false;
        };
        let Some(index) = self.reference_layers.index(self.reference_layers.active) else {
            self.reference_layers.drag = None;
            return false;
        };
        let at = view.to_doc(pos);
        let _ = index;
        let layer = self.reference_layers.selected_mut().unwrap();
        match drag {
            Drag::Move { grab, center } => {
                layer.center = [center[0] + at.x - grab.x, center[1] + at.y - grab.y];
            }
            Drag::Scale { distance, scale } => {
                let now = (at - pos2(layer.center[0], layer.center[1])).length();
                layer.scale = (scale * now / distance).clamp(MIN_SCALE, MAX_SCALE);
            }
            Drag::Rotate {
                angle: from,
                rotation,
            } => {
                let center = view.to_screen(pos2(layer.center[0], layer.center[1]));
                let turned = angle(center, pos) - from;
                // The view may be mirrored; screen angles then run backward.
                let turned = if view.mirror { -turned } else { turned };
                let mut value = rotation + turned;
                if shift {
                    value = (value / 15.0).round() * 15.0;
                }
                layer.rotation = (value + 180.0).rem_euclid(360.0) - 180.0;
            }
        }
        if end {
            self.reference_layers.drag = None;
            self.references_edited();
        }
        true
    }
    pub(crate) fn cancel_reference_drag(&mut self) {
        self.reference_layers.drag = None;
    }
    pub(crate) fn paint_reference_handles(&self, painter: &egui::Painter, view: View) {
        let Some(index) = self.reference_layers.index(self.reference_layers.active) else {
            return;
        };
        if !self.reference_layers.layers[index].visible
            || self
                .reference_layers
                .hidden
                .contains(&self.reference_layers.layers[index].id)
        {
            return;
        }
        let Some(layer) = self.reference_layers.selected() else {
            return;
        };
        let h = handles(layer, view);
        let accent = if layer.locked {
            Color32::from_gray(150)
        } else {
            Color32::from_rgb(240, 160, 60)
        };
        let stroke = Stroke::new(1.5_f32, accent);
        let mut outline = h.corners.to_vec();
        outline.push(h.corners[0]);
        painter.add(Shape::line(
            outline.clone(),
            Stroke::new(3.0_f32, Color32::from_black_alpha(90)),
        ));
        painter.add(Shape::line(outline, stroke));
        if layer.locked
            || self.reference_layers.layers[index].locked
            || self
                .reference_layers
                .blocked
                .contains(&self.reference_layers.layers[index].id)
        {
            return;
        }
        painter.line_segment([h.top, h.rotate], stroke);
        for corner in h.corners {
            painter.circle(corner, 5.0, Color32::WHITE, stroke);
        }
        painter.circle(h.rotate, 6.0, Color32::WHITE, stroke);
    }

    /// Step through the unified artwork/reference order.
    pub(crate) fn step_reference(&mut self, id: u64, up: bool) {
        let Some(index) = self.reference_layers.index(Some(id)) else {
            return;
        };
        let (gap, _, _) = self
            .reference_layers
            .location(&self.reference_layers.layers[index], &self.doc);
        let other = if up {
            (index + 1..self.reference_layers.layers.len()).find(|&i| {
                self.reference_layers
                    .location(&self.reference_layers.layers[i], &self.doc)
                    .0
                    == gap
            })
        } else {
            (0..index).rev().find(|&i| {
                self.reference_layers
                    .location(&self.reference_layers.layers[i], &self.doc)
                    .0
                    == gap
            })
        };
        if let Some(other) = other {
            let target = self.reference_layers.layers[other].id;
            self.reorder_reference_board(id, target, up);
            return;
        }
        if up && gap < self.doc.layers.len() {
            self.place_reference(id, gap + 1, self.doc.layers[gap].depth);
        } else if !up && gap > 0 {
            self.place_reference(id, gap - 1, self.doc.layers[gap - 1].depth);
        }
    }

    /// Compact fixed-width rows: text never pushes controls outside the panel.
    pub(crate) fn reference_rows_for_ids(&mut self, ui: &mut egui::Ui, ids: Vec<u64>) {
        let mut select = None;
        let mut reorder = None;
        let mut edited = false;
        let mut placement = None;
        let mut board_drop = None;
        let mut artwork_drop = None;
        let folders: Vec<_> = self
            .doc
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.folder && l.depth < crate::document::MAX_DEPTH)
            .map(|(i, l)| (i, l.depth + 1, l.name.clone()))
            .collect();
        for id in ids {
            let texture = self.reference_layers.texture(id).cloned();
            let active = self.reference_layers.active == Some(id);
            let (gap, depth, _) = self.reference_layers.location(
                self.reference_layers
                    .layers
                    .iter()
                    .find(|l| l.id == id)
                    .unwrap(),
                &self.doc,
            );
            let layer = self
                .reference_layers
                .layers
                .iter_mut()
                .find(|l| l.id == id)
                .unwrap();
            let (mut rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 54.0), egui::Sense::hover());
            rect.min.x += depth as f32 * 14.0;
            let response = ui.interact(
                rect,
                egui::Id::new(("reference row", id)),
                egui::Sense::click_and_drag(),
            );
            if response.drag_started() {
                response.dnd_set_drag_payload(crate::reference_stack::ReferenceDrag(id));
            }
            if response
                .dnd_hover_payload::<crate::reference_stack::ReferenceDrag>()
                .is_some()
                || response.dnd_hover_payload::<usize>().is_some()
            {
                ui.painter().rect_stroke(
                    rect.shrink(1.0),
                    4.0,
                    Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
                    egui::StrokeKind::Inside,
                );
                let above = ui
                    .ctx()
                    .pointer_interact_pos()
                    .is_none_or(|p| p.y < rect.center().y);
                if let Some(from) =
                    response.dnd_release_payload::<crate::reference_stack::ReferenceDrag>()
                    && from.0 != id
                {
                    board_drop = Some((from.0, id, above));
                }
                if let Some(from) = response.dnd_release_payload::<usize>() {
                    artwork_drop = Some((*from, gap, depth));
                }
            }
            if active {
                let key = (
                    id,
                    [
                        ui.clip_rect().width().round() as i32,
                        ui.clip_rect().height().round() as i32,
                    ],
                );
                let state = egui::Id::new("revealed reference");
                if ui.ctx().data(|d| d.get_temp::<(u64, [i32; 2])>(state)) != Some(key) {
                    response.scroll_to_me(Some(egui::Align::Max));
                    ui.ctx().data_mut(|d| d.insert_temp(state, key));
                }
            }
            if !ui.is_rect_visible(rect) {
                continue;
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    active,
                    &layer.name,
                )
            });
            let visuals = ui.visuals();
            let fill = if active {
                visuals.selection.bg_fill
            } else if response.hovered() {
                visuals.widgets.hovered.weak_bg_fill
            } else {
                visuals.faint_bg_color
            };
            let text = if active {
                visuals.selection.stroke.color
            } else {
                visuals.text_color()
            };
            ui.painter().rect_filled(rect.shrink(1.0), 4.0, fill);
            if active {
                ui.painter().rect_stroke(
                    rect.shrink(1.0),
                    4.0,
                    Stroke::new(1.0_f32, text),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(texture) = texture {
                let bounds = egui::Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(40.0, 40.0));
                let aspect = layer.image.width as f32 / layer.image.height as f32;
                let size = if aspect >= 1.0 {
                    vec2(40.0, 40.0 / aspect)
                } else {
                    vec2(40.0 * aspect, 40.0)
                };
                ui.painter().image(
                    texture.id(),
                    egui::Rect::from_center_size(bounds.center(), size),
                    egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            let width = (rect.width() - 122.0).max(1.0);
            let summary = format!(
                "{} / {MAX_IMAGES} images{}",
                layer.images.len() + 1,
                if layer.locked { " - locked" } else { "" }
            );
            for (label, y, font) in [(&*layer.name, 8.0, 13.0), (&*summary, 29.0, 11.0)] {
                let galley = egui::WidgetText::from(label).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Truncate),
                    width,
                    egui::FontId::proportional(font),
                );
                ui.painter().galley(rect.min + vec2(52.0, y), galley, text);
            }
            let controls =
                egui::Rect::from_min_max(pos2(rect.right() - 62.0, rect.top() + 12.0), rect.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal(|ui| {
                    let eye = if layer.visible {
                        crate::icons::Icon::Eye
                    } else {
                        crate::icons::Icon::EyeOff
                    };
                    if crate::icons::button(ui, eye, 26.0, false, true)
                        .on_hover_text("Show or hide reference")
                        .clicked()
                    {
                        layer.visible = !layer.visible;
                        edited = true;
                    }
                    if crate::icons::button(ui, crate::icons::Icon::Lock, 26.0, layer.locked, true)
                        .on_hover_text("Lock reference position")
                        .clicked()
                    {
                        layer.locked = !layer.locked;
                        edited = true;
                    }
                });
            });
            if response.clicked()
                && response
                    .interact_pointer_pos()
                    .is_none_or(|p| p.x < controls.left())
            {
                select = Some(id);
            }
            response.on_hover_text(&layer.name).context_menu(|ui| {
                if ui.button("Select reference").clicked() {
                    select = Some(id);
                    ui.close();
                }
                ui.menu_button("Move into folder", |ui| {
                    for (gap, depth, name) in &folders {
                        if ui.button(name).clicked() {
                            placement = Some((id, *gap, *depth));
                            ui.close();
                        }
                    }
                    if ui.button("Document root (top)").clicked() {
                        placement = Some((id, self.doc.layers.len(), 0));
                        ui.close();
                    }
                });
                if ui.button("Move up").clicked() {
                    reorder = Some((id, true));
                    ui.close();
                }
                if ui.button("Move down").clicked() {
                    reorder = Some((id, false));
                    ui.close();
                }
            });
        }
        if let Some((id, gap, depth)) = placement {
            self.place_reference(id, gap, depth);
        }
        if let Some((from, to, above)) = board_drop {
            self.reorder_reference_board(from, to, above);
        }
        if let Some((from, gap, depth)) = artwork_drop {
            self.reorder_canvas_layer(from, gap, depth);
        }
        if edited {
            self.references_edited();
        }
        if let Some(id) = select {
            self.select_reference(id);
        }
        if let Some((id, up)) = reorder {
            self.step_reference(id, up);
        }
    }

    /// Properties of the selected reference, shown in place of layer controls.
    pub(crate) fn reference_properties_ui(&mut self, ui: &mut egui::Ui) {
        let Some(index) = self.reference_layers.index(self.reference_layers.active) else {
            return;
        };
        let importing = self.reference_layers.import_job.is_some();
        let mut cancel_import = false;
        let (width, height) = (self.doc.width, self.doc.height);
        let inherited_locked = self
            .reference_layers
            .blocked
            .contains(&self.reference_layers.layers[index].id);
        let mut edited = false;
        let mut remove = false;
        let mut done = false;
        let mut step = None;
        let mut add_images = false;
        let mut arrange = false;
        let mut remove_image = false;
        let mut active_image = self.reference_layers.active_image;
        let names: Vec<_> = self.reference_layers.layers[index]
            .placements()
            .map(|p| p.name.clone())
            .collect();
        let textures: Vec<_> = self.reference_layers.layers[index]
            .placements()
            .map(|p| self.reference_layers.texture(p.id).cloned())
            .collect();
        let layer = &mut self.reference_layers.layers[index];
        let id = layer.id;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("REFERENCE").small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Done")
                    .on_hover_text("Return to drawing · Esc")
                    .clicked()
                {
                    done = true;
                }
            });
        });
        let name = ui.add(egui::TextEdit::singleline(&mut layer.name).char_limit(64));
        if name.changed() {
            while layer.name.len() > MAX_NAME_BYTES {
                layer.name.pop();
            }
            edited = true;
        }
        ui.horizontal(|ui| {
            ui.menu_button(
                format!("Images {}/{MAX_IMAGES}", layer.images.len() + 1),
                |ui| {
                    ui.small(format!("Editing image {}", active_image + 1));
                    if ui
                        .add_enabled(
                            !layer.locked && !inherited_locked,
                            egui::Button::new("Arrange images"),
                        )
                        .clicked()
                    {
                        arrange = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !layer.locked && !inherited_locked && !layer.images.is_empty(),
                            egui::Button::new("Remove image"),
                        )
                        .clicked()
                    {
                        remove_image = true;
                        ui.close();
                    }
                },
            );
            add_images = ui
                .add_enabled(
                    layer.images.len() + 1 < MAX_IMAGES
                        && !layer.locked
                        && !inherited_locked
                        && !importing,
                    egui::Button::new("+ Add images..."),
                )
                .clicked();
        });
        if importing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.small("Importing images...");
                cancel_import = ui.small_button("Cancel import").clicked();
            });
        }
        egui::ScrollArea::horizontal()
            .id_salt(("reference images", id))
            .max_height(46.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().button_padding = vec2(3.0, 3.0);
                    for (image, texture) in textures.iter().enumerate() {
                        if let Some(texture) = texture {
                            let response = ui
                                .push_id(("reference image", id, image), |ui| {
                                    ui.add(
                                        egui::Button::image(
                                            egui::Image::new(texture)
                                                .fit_to_exact_size(vec2(36.0, 36.0)),
                                        )
                                        .selected(active_image == image),
                                    )
                                    .on_hover_text(format!(
                                        "Image {}: {}",
                                        image + 1,
                                        names[image]
                                    ))
                                })
                                .inner;
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(
                                    egui::Id::new(("reference image button", id, image)),
                                    response.rect,
                                )
                            });
                            if response.clicked() {
                                active_image = image;
                            }
                        } else if ui
                            .selectable_label(active_image == image, format!("{}", image + 1))
                            .clicked()
                        {
                            active_image = image;
                        }
                    }
                });
            });
        active_image = active_image.min(layer.images.len());
        egui::CollapsingHeader::new("Reference settings")
            .id_salt("reference settings")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Opacity");
                    let mut percent = layer.opacity * 100.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut percent, 0.0..=100.0)
                                .suffix("%")
                                .max_decimals(0),
                        )
                        .changed()
                    {
                        edited |= set(&mut layer.opacity, (percent / 100.0).clamp(0.0, 1.0));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Placement");
                    for (above, label) in [(true, "Above artwork"), (false, "Below artwork")] {
                        if ui
                            .selectable_label(
                                layer.position.is_none() && layer.above == above,
                                label,
                            )
                            .clicked()
                        {
                            layer.above = above;
                            layer.position = None;
                            edited = true;
                        }
                    }
                    if layer.position.is_some() {
                        ui.small("In layer stack (drag row to reorder)");
                    }
                });
                egui::CollapsingHeader::new("Transform reference")
                    .id_salt("reference transform")
                    .show(ui, |ui| {
                        let locked = layer.locked || inherited_locked;
                        let layer = if active_image == 0 {
                            &mut *layer
                        } else {
                            &mut layer.images[active_image - 1]
                        };
                        ui.add_enabled_ui(!locked, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Scale");
                                let mut percent = layer.scale * 100.0;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut percent)
                                            .range(MIN_SCALE * 100.0..=MAX_SCALE * 100.0)
                                            .suffix("%")
                                            .speed(1.0),
                                    )
                                    .changed()
                                {
                                    edited |= set(
                                        &mut layer.scale,
                                        (percent / 100.0).clamp(MIN_SCALE, MAX_SCALE),
                                    );
                                }
                                ui.label("Rotate");
                                let mut rotation = layer.rotation;
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut rotation)
                                            .range(-180.0..=180.0)
                                            .suffix("°")
                                            .speed(0.5),
                                    )
                                    .changed()
                                {
                                    edited |= set(&mut layer.rotation, rotation);
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .button("Flip")
                                    .on_hover_text("Mirror horizontally")
                                    .clicked()
                                {
                                    layer.flip = !layer.flip;
                                    edited = true;
                                }
                                if ui.button("Fit canvas").clicked() {
                                    layer.fit(width, height);
                                    edited = true;
                                }
                                if ui
                                    .button("Actual size")
                                    .on_hover_text("One image pixel per canvas pixel")
                                    .clicked()
                                {
                                    layer.scale = 1.0;
                                    edited = true;
                                }
                            });
                        });
                    });
                ui.horizontal(|ui| {
                    if ui.small_button("Move up").clicked() {
                        step = Some(true);
                    }
                    if ui.small_button("Move down").clicked() {
                        step = Some(false);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut layer.locked, "Locked").changed() {
                        edited = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            remove = true;
                        }
                    });
                });
            });
        if cancel_import && let Some(job) = &self.reference_layers.import_job {
            job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.reference_layers.active_image = active_image;
        if remove_image {
            let board = &mut self.reference_layers.layers[index];
            if active_image > 0 {
                board.images.remove(active_image - 1);
            } else {
                let image = board.images.remove(0);
                board.image = image.image;
                board.center = image.center;
                board.scale = image.scale;
                board.rotation = image.rotation;
                board.flip = image.flip;
            }
            self.reference_layers.active_image = active_image.min(board.images.len());
            edited = true;
        }
        if add_images {
            self.add_reference_images(id);
        }
        if arrange {
            self.arrange_reference_images(id);
        }
        if let Some(up) = step {
            self.step_reference(id, up);
        }
        if remove {
            self.reference_layers.layers.remove(index);
            self.reference_layers.active = None;
            self.reference_layers.drag = None;
            self.status = "Reference removed · artwork unchanged".into();
            edited = true;
        }
        if edited {
            self.references_edited();
        }
        if done {
            self.deselect_reference();
        }
    }
}

/// Store `value`, reporting whether it differs. Widgets can report a change
/// when they only round or clamp what they display; that is not an edit.
fn set(field: &mut f32, value: f32) -> bool {
    let changed = *field != value;
    *field = value;
    changed
}

fn validate_new_image(image: &Image) -> Result<(), String> {
    if image.width == 0
        || image.height == 0
        || image.width > MAX_SIDE
        || image.height > MAX_SIDE
        || image
            .width
            .checked_mul(image.height)
            .and_then(|p| p.checked_mul(4))
            != Some(image.rgba.len())
    {
        Err("Invalid reference image".into())
    } else {
        Ok(())
    }
}

/// Decode a PNG with the bounded artwork importer, then keep a view-sized copy.
fn import(path: &std::path::Path) -> Result<Image, String> {
    let raster = crate::image_paste::decode(path)?;
    downscale(Image {
        width: raster.width,
        height: raster.height,
        rgba: raster.rgba,
    })
}

pub(crate) fn downscale(image: Image) -> Result<Image, String> {
    let longest = image.width.max(image.height);
    if longest <= IMPORT_SIDE {
        return Ok(image);
    }
    let factor = IMPORT_SIDE as f32 / longest as f32;
    let source = image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.rgba)
        .ok_or("Invalid reference pixels")?;
    let resized = image::imageops::resize(
        &source,
        ((image.width as f32 * factor).round() as u32).max(1),
        ((image.height as f32 * factor).round() as u32).max(1),
        image::imageops::FilterType::Triangle,
    );
    Ok(Image {
        width: resized.width() as usize,
        height: resized.height() as usize,
        rgba: resized.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn reference_batch_import_publishes_atomically_and_cancels_without_partial_images() {
        fn drain(app: &mut Dudel) {
            let ctx = egui::Context::default();
            let start = std::time::Instant::now();
            while app.reference_layers.import_job.is_some() {
                app.poll_reference_import(&ctx);
                assert!(start.elapsed() < std::time::Duration::from_secs(5));
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("hair.png");
        let bad = dir.path().join("broken.png");
        image::save_buffer_with_format(
            &good,
            &[90, 120, 180, 255].repeat(16),
            4,
            4,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        std::fs::write(&bad, b"broken PNG").unwrap();
        let mut app = Dudel {
            doc: Document::new(16, 16),
            ..Default::default()
        };
        app.insert_reference("Hair".into(), image(4, 4, 1));
        let id = app.reference_layers.active.unwrap();
        let artwork = app.doc.clone();
        app.queue_reference_import(id, vec![good.clone(), bad]);
        assert!(app.reference_layers.layers[0].images.is_empty());
        drain(&mut app);
        assert!(app.reference_layers.layers[0].images.is_empty());
        app.queue_reference_import(id, vec![good.clone(), good.clone()]);
        drain(&mut app);
        assert_eq!(app.reference_layers.layers[0].images.len(), 2);
        let revision = app.revision;
        app.queue_reference_import(id, vec![good.clone()]);
        app.reference_layers
            .import_job
            .as_ref()
            .unwrap()
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        drain(&mut app);
        assert_eq!(app.reference_layers.layers[0].images.len(), 2);
        assert_eq!(app.revision, revision);
        app.queue_reference_import(id, vec![good]);
        app.reference_layers.install(Vec::new());
        drain(&mut app);
        assert!(app.reference_layers.layers.is_empty());
        assert!(crate::document::same_snapshot(&artwork, &app.doc));
    }

    fn image(width: usize, height: usize, seed: u8) -> Image {
        Image {
            width,
            height,
            rgba: (0..width * height * 4)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
                .collect(),
        }
    }
    #[test]
    fn twelve_image_board_uploads_once_and_keeps_image_allocations_shared_while_transforming() {
        let mut app = Dudel::default();
        app.insert_reference("Hair".into(), image(64, 48, 1));
        let id = app.reference_layers.active.unwrap();
        for n in 1..MAX_IMAGES {
            app.insert_reference_image(id, format!("Photo {n}"), image(64, 48, n as u8))
                .unwrap();
        }
        app.arrange_reference_images(id);
        app.place_reference(id, 0, 0);
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        let ids: Vec<_> = app.reference_layers.layers[0]
            .placements()
            .map(|p| app.reference_layers.texture(p.id).unwrap().id())
            .collect();
        let snapshot = app.reference_layers.display_document(&app.doc).unwrap();
        app.reference_layers.layers[0].images[3].center[0] += 10.0;
        let output = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        assert!(
            !output
                .textures_delta
                .set
                .iter()
                .any(|(id, _)| ids.contains(id)),
            "unchanged source images must not upload again after a transform"
        );
        let next = app.reference_layers.display_document(&app.doc).unwrap();
        assert_ne!(
            snapshot.layers[0].display_key(),
            next.layers[0].display_key()
        );
        assert_eq!(app.reference_layers.textures.len(), MAX_IMAGES);
        assert_eq!(app.reference_image_bytes(), 64 * 48 * 4 * MAX_IMAGES);
    }
    fn reference(seed: u8, above: bool) -> ReferenceLayer {
        ReferenceLayer {
            id: 0,
            name: format!("Pose {seed}"),
            center: [40.5, 22.0],
            scale: 0.75,
            rotation: -12.5,
            opacity: 0.4,
            visible: true,
            locked: seed.is_multiple_of(2),
            above,
            flip: true,
            image: Arc::new(image(13 + seed as usize, 7, seed)),
            images: Vec::new(),
            position: None,
        }
    }
    fn same(a: &ReferenceLayer, b: &ReferenceLayer) -> bool {
        a.name == b.name
            && a.center == b.center
            && a.scale == b.scale
            && a.rotation == b.rotation
            && a.opacity == b.opacity
            && (a.visible, a.locked, a.above, a.flip) == (b.visible, b.locked, b.above, b.flip)
            && a.image == b.image
    }

    #[test]
    fn projects_round_trip_reference_layers_beside_selection_masks() {
        let directory = tempfile::tempdir().unwrap();
        for masked in [false, true] {
            let mut doc = Document::new(64, 48);
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(5)
                .copy_from_slice(&[9, 8, 7, 255]);
            if masked {
                let mask = crate::selection_mask::Mask::from_coverage(
                    64,
                    48,
                    (0..64 * 48).map(|i| (i % 256) as u8).collect(),
                )
                .unwrap();
                doc.selection = mask.bounds();
                doc.selection_mask = Some(mask);
            }
            doc.references = vec![reference(1, true), reference(2, false)];
            let path = directory.path().join(format!("refs-{masked}.dudel"));
            crate::storage::save_project(&path, &doc).unwrap();
            let loaded = crate::storage::load_project(&path).unwrap();
            assert_eq!(loaded.references.len(), 2);
            for (a, b) in loaded.references.iter().zip(&doc.references) {
                assert!(same(a, b));
            }
            assert_eq!(loaded.selection_mask, doc.selection_mask);
            assert_eq!(loaded.composite(), doc.composite());
        }
        // Documents without references keep their earlier bytes.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&Document::new(32, 32), &mut bytes).unwrap();
        let mut payload = Vec::new();
        use std::io::Read;
        flate2::read::GzDecoder::new(&bytes[crate::tile_project::MAGIC.len()..])
            .read_to_end(&mut payload)
            .unwrap();
        assert!(!payload.windows(4).any(|w| w == b"REFS"));
    }

    #[test]
    fn damaged_or_mismatched_reference_records_are_rejected() {
        let mut doc = Document::new(16, 16);
        doc.references = vec![reference(3, true)];
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        let header = crate::tile_project::MAGIC.len();
        assert!(crate::tile_project::decode(&bytes[header..]).is_ok());
        // Metadata that names a reference without its image trailer.
        let mut missing = Vec::new();
        {
            use std::io::Write;
            let mut gzip = flate2::write::GzEncoder::new(&mut missing, flate2::Compression::fast());
            let metadata = crate::binary_project::metadata(&doc).unwrap();
            gzip.write_all(&(metadata.len() as u32).to_le_bytes())
                .unwrap();
            gzip.write_all(&metadata).unwrap();
            gzip.write_all(&0u32.to_le_bytes()).unwrap();
            gzip.finish().unwrap();
        }
        assert!(crate::tile_project::decode(&missing[..]).is_err());
        // A record count that disagrees with the metadata.
        let mut images = Vec::new();
        encode_trailer(&doc.references, &mut images, &|| Ok(())).unwrap();
        images[5] = 2;
        let mut references = vec![reference(3, true)];
        assert!(decode_trailer(&mut references, &mut &images[5..]).is_err());
        // Invalid settings fail validation; legacy v3 refuses references.
        let mut bad = reference(4, true);
        bad.scale = f32::NAN;
        assert!(validate(&[bad], false).is_err());
        assert!(validate(&vec![reference(5, true); MAX_REFERENCES + 1], false).is_err());
        assert!(crate::binary_project::encode(&doc, Vec::new()).is_err());
    }

    fn app_with_reference() -> (Dudel, View) {
        let mut app = Dudel::default();
        app.insert_reference("Pose".into(), image(40, 20, 1));
        let view = View {
            origin: pos2(500.0, 400.0),
            zoom: 2.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(app.doc.width as f32 / 2.0, app.doc.height as f32 / 2.0),
        };
        (app, view)
    }

    #[test]
    fn selected_reference_is_revealed_instead_of_the_artwork_row() {
        let (mut app, _) = app_with_reference();
        let first = app.reference_layers.layers[0].id;
        for i in 0..7 {
            app.insert_reference(format!("Reference {i}"), image(8, 8, i));
        }
        app.select_reference(first);
        let ctx = egui::Context::default();
        for frame in 0..5 {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(frame as f64 * 0.2),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(300.0, 560.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    crate::themes::apply(ctx, &app.theme);
                    egui::CentralPanel::default().show(ctx, |ui| app.layers_ui(ui, true));
                },
            );
        }
        let row = ctx
            .read_response(egui::Id::new(("reference row", first)))
            .unwrap()
            .rect;
        assert!(
            row.min.y > 200.0 && row.max.y < 560.0,
            "selected reference is outside the visible layer list: {row:?}"
        );
        assert!(
            row.max.x <= 300.0,
            "reference controls exceeded the panel width"
        );
    }

    #[test]
    fn reference_reorder_stays_in_group_and_preserves_selection_and_artwork() {
        let (mut app, _) = app_with_reference();
        let first = app.reference_layers.layers[0].id;
        app.insert_reference("Below".into(), image(8, 8, 2));
        app.reference_layers.layers[1].above = false;
        app.insert_reference("Top".into(), image(8, 8, 3));
        let top = app.reference_layers.layers[2].id;
        let artwork = app.doc.clone();
        app.select_reference(first);
        app.step_reference(first, true);
        assert_eq!(app.reference_layers.layers[2].id, first);
        assert!(
            app.reference_layers
                .layers
                .iter()
                .any(|r| r.id == top && r.above)
        );
        assert!(app.reference_layers.layers.iter().any(|r| !r.above));
        assert_eq!(app.reference_layers.active, Some(first));
        assert!(crate::document::same_snapshot(&artwork, &app.doc));
        app.step_reference(first, true);
        assert_eq!(app.reference_layers.layers[2].id, first);
    }

    #[test]
    fn selected_reference_moves_scales_and_rotates_without_touching_artwork() {
        let (mut app, view) = app_with_reference();
        let artwork = app.doc.composite();
        assert!(app.references_changed() && app.modified);
        let id = app.reference_layers.active.unwrap();
        let layer = app.reference_layers.layers[0].clone();
        let center = view.to_screen(pos2(layer.center[0], layer.center[1]));
        // Drag inside: 30 screen pixels at 200% move it 15 document pixels.
        assert!(app.reference_packet(view, center, true, false, true, false));
        assert!(app.reference_packet(view, center + vec2(30.0, 0.0), false, true, true, false));
        let moved = app.reference_layers.layers[0].center;
        assert!((moved[0] - layer.center[0] - 15.0).abs() < 1e-3);
        // Dragging a corner to twice its distance doubles the scale.
        let corner = view.to_screen(app.reference_layers.layers[0].corners()[2]);
        let middle = view.to_screen(pos2(moved[0], moved[1]));
        assert!(app.reference_packet(view, corner, true, false, true, false));
        app.reference_packet(
            view,
            middle + (corner - middle) * 2.0,
            false,
            true,
            true,
            false,
        );
        let scaled = app.reference_layers.layers[0].scale;
        assert!((scaled / layer.scale - 2.0).abs() < 1e-3, "{scaled}");
        // The top handle turns it around its center; Shift snaps to 15 degrees.
        let h = handles(&app.reference_layers.layers[0], view);
        let swing = h.center + vec2(1.0, -0.3).normalized() * (h.rotate - h.center).length();
        assert!(app.reference_packet(view, h.rotate, true, false, true, false));
        app.reference_packet(view, swing, false, true, true, true);
        let rotation = app.reference_layers.layers[0].rotation;
        assert!(
            rotation != 0.0 && (rotation / 15.0).fract().abs() < 1e-4,
            "{rotation}"
        );
        // A locked reference cannot be grabbed; the Pointer pans instead.
        app.reference_layers.layers[0].locked = true;
        let before = app.reference_layers.layers[0].center;
        assert!(!app.reference_packet(view, middle, true, false, true, false));
        assert!(!app.reference_packet(view, middle + vec2(50.0, 0.0), false, true, true, false));
        assert_eq!(app.reference_layers.layers[0].center, before);
        app.reference_layers.layers[0].locked = false;
        // Other tools never move references.
        app.select_tool(crate::tools::Tool::Brush);
        assert!(app.reference_layers.active.is_none());
        assert!(!app.reference_packet(view, middle, true, false, true, false));
        assert_eq!(app.reference_layers.layers[0].center, before);
        // With the Pointer, a press on an unselected reference selects and
        // moves it; a press on empty canvas deselects and is left to panning.
        app.select_tool(crate::tools::Tool::Pointer);
        assert!(app.reference_layers.active.is_none());
        assert!(app.reference_packet(view, middle, true, false, true, false));
        assert_eq!(app.reference_layers.active, Some(id));
        assert!(app.reference_grabs(view, middle));
        assert!(app.reference_packet(view, middle + vec2(20.0, 0.0), false, true, true, false));
        assert!((app.reference_layers.layers[0].center[0] - before[0] - 10.0).abs() < 1e-3);
        let empty = view.to_screen(pos2(-4000.0, -4000.0));
        assert!(!app.reference_grabs(view, empty));
        assert!(!app.reference_packet(view, empty, true, false, true, false));
        assert!(app.reference_layers.active.is_none());
        assert_eq!(app.doc.composite(), artwork);
        assert_eq!(app.reference_layers.layers[0].id, id);
    }

    #[test]
    fn pointer_picks_the_topmost_reference_and_selecting_a_row_switches_to_pointer() {
        let mut app = Dudel::default();
        app.select_tool(crate::tools::Tool::Brush);
        app.insert_reference("Below".into(), image(40, 20, 1));
        app.reference_layers.layers[0].above = false;
        app.insert_reference("Above".into(), image(40, 20, 2));
        assert!(app.tool == crate::tools::Tool::Pointer);
        let (below, above) = (
            app.reference_layers.layers[0].id,
            app.reference_layers.layers[1].id,
        );
        app.deselect_reference();
        let view = View {
            origin: pos2(500.0, 400.0),
            zoom: 1.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(app.doc.width as f32 / 2.0, app.doc.height as f32 / 2.0),
        };
        let center = app.reference_layers.layers[0].center;
        let at = view.to_screen(pos2(center[0], center[1]));
        // Both are fitted to the canvas center: the one above wins.
        assert!(app.reference_packet(view, at, true, false, true, false));
        assert_eq!(app.reference_layers.active, Some(above));
        app.reference_packet(view, at, false, true, true, false);
        // Hidden or locked references are skipped.
        app.reference_layers.layers[1].visible = false;
        assert!(app.reference_packet(view, at, true, false, true, false));
        assert_eq!(app.reference_layers.active, Some(below));
        app.reference_packet(view, at, false, true, true, false);
        app.reference_layers.layers[0].locked = true;
        assert!(!app.reference_packet(view, at, true, false, true, false));
        // Selecting a reference while drawing switches to the Pointer tool.
        app.select_tool(crate::tools::Tool::Brush);
        app.select_reference(below);
        assert!(app.tool == crate::tools::Tool::Pointer);
        assert_eq!(app.reference_layers.active, Some(below));
    }

    #[test]
    fn save_and_load_track_reference_changes_and_canvas_rotation_carries_them() {
        let (mut app, _) = app_with_reference();
        let saved = app.reference_layers.begin_save();
        app.reference_layers.finish_save(true);
        assert!(!app.references_changed());
        // A failed save leaves later edits unsaved.
        app.reference_layers.layers[0].opacity = 0.9;
        app.references_edited();
        app.reference_layers.begin_save();
        app.reference_layers.finish_save(false);
        assert!(app.references_changed());
        // Loading installs a clean saved state with nothing selected.
        app.reference_layers.install(saved);
        assert!(!app.references_changed());
        assert!(app.reference_layers.active.is_none());
        assert_eq!(app.reference_layers.layers[0].opacity, 0.6);
        // A quarter turn of the canvas carries the reference along.
        let before = crate::canvas_size::Frame::default();
        let after = before.rotated(app.doc.height).unwrap();
        let center = app.reference_layers.layers[0].center;
        app.map_references(before, after);
        let layer = &app.reference_layers.layers[0];
        assert_eq!(layer.center, after.point_from(before, center));
        assert_eq!(layer.rotation, 90.0);
    }

    #[test]
    fn references_below_the_artwork_switch_display_tiles_to_alpha() {
        let ctx = egui::Context::default();
        let (mut app, _) = app_with_reference();
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        assert!(!app.texture.alpha());
        app.reference_layers.layers[0].above = false;
        app.dirty = false;
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        assert!(app.texture.alpha() && app.dirty);
        app.reference_layers.layers[0].visible = false;
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        assert!(!app.texture.alpha());
        app.show_transparency = true;
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        assert!(app.texture.alpha());
    }

    #[test]
    fn showing_the_reference_panel_is_not_an_edit() {
        // Widgets may report a change when they only round a displayed value
        // such as 60.000004%; that must not mark the project unsaved.
        let ctx = egui::Context::default();
        let (mut app, _) = app_with_reference();
        app.reference_layers.layers[0].opacity = 0.6;
        app.reference_layers.layers[0].scale = 0.333_333_34;
        app.reference_layers.layers[0].rotation = 12.000_001;
        let saved = app.reference_layers.begin_save();
        app.reference_layers.finish_save(true);
        let revision = app.revision;
        for _ in 0..4 {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.reference_properties_ui(ui));
            });
        }
        assert_eq!(app.revision, revision);
        assert!(!app.references_changed());
        assert!(same(&app.reference_layers.layers[0], &saved[0]));
    }
}
