//! The 3D model window: a model to draw from, a live preview of the canvas
//! as its texture, its texture layout over the canvas, and its view as a
//! layer or a perspective ruler. Models are loaded and drawn by the engine
//! (`mesh`, `mesh_render`) on background workers; nothing here waits for one.
use crate::{
    Dudel,
    background::{self, Handle},
    clipboard::Raster,
    document::Document,
    egui,
    mesh::{self, Mesh, Texture},
    mesh_render::{self, Camera, Look, Paint, Shading},
    rulers::Ruler,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// Longest side of the picture in the window, in pixels.
const MAX_PICTURE: f32 = 1600.0;
/// Longest side of the canvas as a texture while painting, and when idle.
const QUICK: usize = 512;
const FINE: usize = 2048;
/// The canvas counts as being painted for this long after a change.
const SETTLE: Duration = Duration::from_millis(350);
/// Least time between textures while painting, so dabs keep the cores.
const INTERVAL: Duration = Duration::from_millis(90);
/// More layout lines than this are drawn once as a picture instead.
const MAX_LINES: usize = 3000;
/// The layout's color until one is chosen, and the quick choices.
const LAYOUT_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 143, 191, 190);
const LAYOUT_COLORS: [(&str, [u8; 3]); 5] = [
    ("Blue", [0, 190, 255]),
    ("Magenta", [255, 40, 200]),
    ("Yellow", [255, 220, 0]),
    ("Black", [0, 0, 0]),
    ("White", [255, 255, 255]),
];

/// What dragging on the model does.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Drag {
    #[default]
    Turn,
    Move,
    Light,
}

/// Which texture is shown: the canvas's change count and sampling step.
type TextureKey = (u64, usize);

#[derive(Clone, Copy, PartialEq)]
struct ViewKey {
    camera: Camera,
    shading: Shading,
    light: [f32; 2],
    sharp: bool,
    clay: bool,
    /// The material dimmed around, while its folder is being painted.
    focus: Option<u16>,
    /// The canvas as a texture, on this material or on all.
    paint: Option<(TextureKey, Option<u16>)>,
    size: [usize; 2],
    samples: usize,
}

struct Drawn {
    key: ViewKey,
    rgba: Vec<u8>,
    texture: Option<(TextureKey, Arc<Painted>)>,
}

/// The canvas as the model's textures: one for every part, and one for each
/// part with a folder of its own.
struct Painted {
    base: Arc<Texture>,
    parts: Vec<(u16, Arc<Texture>)>,
}

/// A material's name for its folder: its own, or its number.
fn part_name(mesh: &Mesh, material: u16) -> String {
    let name = mesh.materials[material as usize].name.trim();
    if name.is_empty() {
        format!("Part {}", material + 1)
    } else {
        name.to_string()
    }
}

/// Each material with a folder of its name, and the folder's index; the
/// topmost folder when several share it.
fn part_folders(doc: &Document, mesh: &Mesh) -> Vec<(u16, usize)> {
    (0..mesh.materials.len() as u16)
        .filter_map(|material| {
            let name = part_name(mesh, material);
            doc.layers
                .iter()
                .rposition(|l| l.folder && l.name.trim().eq_ignore_ascii_case(&name))
                .map(|index| (material, index))
        })
        .collect()
}

/// The folders in `links`, each once.
fn folders(links: &[(u16, usize)]) -> Vec<usize> {
    let mut folders: Vec<usize> = links.iter().map(|(_, folder)| *folder).collect();
    folders.sort_unstable();
    folders.dedup();
    folders
}

type Edges = Vec<[[f32; 2]; 2]>;
type LayoutKey = (Option<u16>, [usize; 2]);

struct Layout {
    key: LayoutKey,
    edges: Edges,
    /// The lines as a picture, when there are too many to draw each time.
    picture: Option<([usize; 2], Vec<u8>)>,
    texture: Option<egui::TextureHandle>,
}

struct Model {
    name: String,
    mesh: Arc<Mesh>,
}

#[derive(Default)]
pub(crate) struct ModelView {
    pub open: bool,
    model: Option<Model>,
    camera: Camera,
    look: Look,
    drag: Drag,
    /// Show the canvas as the model's texture, on one material or on all.
    paint: bool,
    material: Option<u16>,
    /// Keep whole pixels; by the canvas size until chosen.
    sharp: Option<bool>,
    show_layout: bool,
    /// The layout lines' color, when chosen.
    layout_color: Option<egui::Color32>,
    /// Keep a perspective ruler matched to the view.
    follow_ruler: bool,
    /// Counts canvas changes, for the texture.
    stamp: u64,
    changed: Option<Instant>,
    textured: Option<Instant>,
    texture: Option<(TextureKey, Arc<Painted>)>,
    picture: Option<(ViewKey, egui::TextureHandle)>,
    /// Materials with folders of their own, and what they were made from.
    links: Vec<(u16, usize)>,
    links_hash: u64,
    /// The material whose folder holds the active layer.
    focus: Option<u16>,
    /// Do not dim the other parts while one is painted.
    no_highlight: bool,
    load: Option<(String, Handle<Result<Mesh, String>>)>,
    draw: Option<Handle<Result<Drawn, String>>>,
    layout: Option<Layout>,
    layout_job: Option<Handle<Layout>>,
    place: Option<Handle<Result<Placed, String>>>,
    ruler: Option<Ruler>,
    error: Option<String>,
    /// Screenshots wait for the workers.
    wait: bool,
}

/// The model as it goes onto a layer, cut to its bounds.
struct Placed {
    raster: Raster,
    center: [f32; 2],
    /// The canvas it was drawn for.
    canvas: [usize; 2],
}

fn joined<T>(result: std::thread::Result<Result<T, String>>) -> Result<T, String> {
    result.unwrap_or_else(|_| Err("The task stopped unexpectedly".into()))
}

/// The sampling step that brings a canvas within `most` pixels a side.
fn step(canvas: [usize; 2], most: usize) -> usize {
    canvas[0].max(canvas[1]).div_ceil(most).max(1)
}

/// Draw the model, after making the canvas into a texture when asked.
fn draw(
    mesh: &Mesh,
    key: ViewKey,
    canvas: Option<(Document, Vec<(u16, usize)>)>,
    mut texture: Option<(TextureKey, Arc<Painted>)>,
) -> Result<Drawn, String> {
    if let (Some((doc, links)), Some((wanted, _))) = (canvas, key.paint) {
        // The canvas without these folders.
        let without = |hidden: &[usize]| -> Result<Arc<Texture>, String> {
            let mut doc = doc.clone();
            for &index in hidden {
                if let Some(layer) = doc.layers.get_mut(index) {
                    layer.visible = false;
                }
            }
            let (rgba, size) = doc.composite_reduced(wanted.1, false, || Ok(()))?;
            let made = Texture::new(size, rgba).ok_or("The canvas could not be shown")?;
            Ok(Arc::new(made))
        };
        // Every part wears the layers outside the parts' folders; a part
        // with a folder wears its own folder too, and no other.
        let all = folders(&links);
        let base = without(&all)?;
        let mut parts = Vec::new();
        for &folder in &all {
            let others: Vec<usize> = all.iter().copied().filter(|f| *f != folder).collect();
            let worn = without(&others)?;
            for (material, _) in links.iter().filter(|(_, f)| *f == folder) {
                parts.push((*material, worn.clone()));
            }
        }
        texture = Some((wanted, Arc::new(Painted { base, parts })));
    }
    let look = Look {
        shading: key.shading,
        light: key.light,
        sharp: key.sharp,
        clay: key.clay,
        focus: key.focus,
        paint: key.paint.and_then(|(_, material)| {
            let (_, painted) = texture.as_ref()?;
            Some(Paint {
                texture: painted.base.clone(),
                material,
                parts: painted.parts.clone(),
            })
        }),
    };
    let rgba = mesh_render::render(mesh, &key.camera, &look, key.size, key.samples, &|| Ok(()))?;
    Ok(Drawn { key, rgba, texture })
}

/// Cut a picture to what is not transparent; None when nothing is.
fn cut(rgba: &[u8], size: [usize; 2]) -> Option<Placed> {
    let (mut low, mut high) = (size, [0, 0]);
    for (at, pixel) in rgba.chunks_exact(4).enumerate() {
        if pixel[3] > 0 {
            let at = [at % size[0], at / size[0]];
            for axis in 0..2 {
                low[axis] = low[axis].min(at[axis]);
                high[axis] = high[axis].max(at[axis] + 1);
            }
        }
    }
    if high == [0, 0] {
        return None;
    }
    let (width, height) = (high[0] - low[0], high[1] - low[1]);
    let mut out = Vec::with_capacity(width * height * 4);
    for y in low[1]..high[1] {
        let row = (y * size[0] + low[0]) * 4;
        out.extend_from_slice(&rgba[row..row + width * 4]);
    }
    Some(Placed {
        raster: Raster {
            width,
            height,
            rgba: out,
        },
        center: [0, 1].map(|i| (low[i] + high[i]) as f32 / 2.0),
        canvas: size,
    })
}

fn make_layout(mesh: &Mesh, key: LayoutKey) -> Layout {
    let edges = mesh.uv_edges(key.0);
    let picture = (edges.len() > MAX_LINES).then(|| {
        let step = step(key.1, FINE);
        let size = key.1.map(|side| side.div_ceil(step).max(1));
        // White, tinted to the chosen color when drawn.
        (size, mesh_render::draw_layout(&edges, size, [255; 4]))
    });
    Layout {
        key,
        edges: if picture.is_some() { Vec::new() } else { edges },
        picture,
        texture: None,
    }
}

impl ModelView {
    pub(crate) fn visit_textures(&self, mut visit: impl FnMut(&egui::TextureHandle)) {
        if let Some((_, texture)) = &self.picture {
            visit(texture);
        }
        if let Some(texture) = self.layout.as_ref().and_then(|l| l.texture.as_ref()) {
            visit(texture);
        }
    }
    /// The canvas changed: a texture of it is out of date.
    pub(crate) fn note_damage(&mut self, full: bool, damage: &crate::render_region::Damage) {
        if full || !damage.is_empty() {
            self.stamp = self.stamp.wrapping_add(1);
            self.changed = Some(Instant::now());
        }
    }
    fn show(&mut self, name: String, mesh: Mesh) {
        *self = Self {
            open: true,
            paint: self.paint && mesh.has_uv,
            show_layout: self.show_layout && mesh.has_uv,
            look: Look {
                paint: None,
                ..self.look.clone()
            },
            sharp: self.sharp,
            stamp: self.stamp,
            ruler: self.ruler,
            wait: self.wait,
            model: Some(Model {
                name,
                mesh: Arc::new(mesh),
            }),
            ..Self::default()
        };
    }
    fn sharp(&self, canvas: [usize; 2]) -> bool {
        self.sharp.unwrap_or(canvas[0].max(canvas[1]) <= 256)
    }
    /// Changes to the canvas or to which folders belong to which parts.
    fn texture_stamp(&self) -> u64 {
        self.stamp ^ self.links_hash.rotate_left(17)
    }
    /// Longest side of the textures when idle: smaller as there are more,
    /// so they take about the memory one would.
    fn fine(&self) -> usize {
        let textures = folders(&self.links).len() + 1;
        ((FINE as f32 / (textures as f32).sqrt()) as usize).max(QUICK)
    }
    fn focus(&self) -> Option<u16> {
        self.focus.filter(|_| !self.no_highlight)
    }
    fn key(&self, canvas: [usize; 2], size: [usize; 2], moving: bool) -> ViewKey {
        let busy = self.changed.is_some_and(|at| at.elapsed() < SETTLE);
        let paint = self.paint.then(|| {
            let side = if busy { QUICK } else { self.fine() };
            let wanted = (self.texture_stamp(), step(canvas, side));
            let recent = self.textured.is_some_and(|at| at.elapsed() < INTERVAL);
            let shown = match &self.texture {
                Some((shown, _)) if recent || (shown.0 == wanted.0 && shown.1 <= wanted.1) => {
                    *shown
                }
                _ => wanted,
            };
            (shown, self.material)
        });
        ViewKey {
            camera: self.camera,
            shading: self.look.shading,
            light: self.look.light,
            sharp: self.sharp(canvas),
            clay: self.look.clay,
            focus: self.focus(),
            paint,
            size,
            samples: if moving || (busy && self.paint) { 1 } else { 2 },
        }
    }
}

impl Dudel {
    pub(crate) fn open_model(&mut self, path: PathBuf) {
        self.model_view.open = true;
        let name = crate::image_paste::file_name(&path);
        self.model_view.error = None;
        self.model_view.load = Some((name, background::spawn(move || Mesh::load(&path))));
    }

    fn ask_for_model(&mut self) {
        let dialog = self.dialog_in(
            crate::locations::Kind::Projects,
            rfd::AsyncFileDialog::new().add_filter("3D models", &mesh::EXTENSIONS),
        );
        self.ask_open_file(dialog, |app, path| app.open_model(path));
    }

    /// Take over what the workers have finished.
    fn poll_model(&mut self, ctx: &egui::Context) {
        let view = &mut self.model_view;
        let wait = view.wait;
        if view
            .load
            .as_ref()
            .is_some_and(|(_, j)| wait || j.is_finished())
            && let Some((name, job)) = view.load.take()
        {
            match joined(job.join()) {
                Ok(mesh) => view.show(name, mesh),
                Err(error) => view.error = Some(error),
            }
        }
        if view.draw.as_ref().is_some_and(|j| wait || j.is_finished())
            && let Some(job) = view.draw.take()
        {
            match joined(job.join()) {
                Ok(drawn) => {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied(drawn.key.size, &drawn.rgba);
                    match &mut view.picture {
                        Some((key, texture)) => {
                            *key = drawn.key;
                            texture.set(image, egui::TextureOptions::LINEAR);
                        }
                        None => {
                            let options = egui::TextureOptions::LINEAR;
                            let texture = ctx.load_texture("3D model", image, options);
                            view.picture = Some((drawn.key, texture));
                        }
                    }
                    if drawn.texture.as_ref().map(|t| t.0) != view.texture.as_ref().map(|t| t.0) {
                        view.textured = Some(Instant::now());
                    }
                    view.texture = drawn.texture;
                }
                Err(error) => view.error = Some(error),
            }
        }
        if view
            .layout_job
            .as_ref()
            .is_some_and(|j| wait || j.is_finished())
            && let Some(job) = view.layout_job.take()
            && let Ok(mut layout) = job.join()
        {
            if let Some((size, rgba)) = layout.picture.take() {
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                let options = egui::TextureOptions::LINEAR;
                layout.texture = Some(ctx.load_texture("texture layout", image, options));
            }
            view.layout = Some(layout);
        }
        if view.place.as_ref().is_some_and(|j| wait || j.is_finished())
            && let Some(job) = view.place.take()
        {
            let name = view.model.as_ref().map(|m| m.name.clone());
            let name = name.unwrap_or_else(|| "3D model".into());
            let placed = joined(job.join()).and_then(|placed| {
                if placed.canvas != [self.doc.width, self.doc.height] {
                    return Err("The canvas size changed; place the model again".into());
                }
                crate::image_paste::validate_incoming(&placed.raster)?;
                self.queue_paste_layer(
                    crate::clipboard::PasteSource::Raster(placed.raster, name),
                    Some(placed.center),
                )
            });
            if let Err(error) = placed {
                self.status = format!("3D model: {error}");
            }
        }
    }

    /// Start what is wanted and not being made yet.
    /// Which folders belong to which parts, and which part is being painted.
    fn update_part_folders(&mut self) {
        let Some(model) = &self.model_view.model else {
            return;
        };
        let links = part_folders(&self.doc, &model.mesh);
        let mut hasher = std::hash::DefaultHasher::new();
        for (material, folder) in &links {
            std::hash::Hash::hash(
                &(material, folder, self.doc.layers[*folder].id),
                &mut hasher,
            );
        }
        let mut at = (self.doc.active < self.doc.layers.len()).then_some(self.doc.active);
        let focus = loop {
            let Some(index) = at else { break None };
            if let Some((material, _)) = links.iter().find(|(_, folder)| *folder == index) {
                break Some(*material);
            }
            at = self.doc.parent(index);
        };
        let view = &mut self.model_view;
        view.links_hash = std::hash::Hasher::finish(&hasher);
        view.links = links;
        view.focus = focus;
    }

    /// Folders at the top for the parts that have none.
    fn make_part_folders(&mut self) {
        let Some(model) = &self.model_view.model else {
            return;
        };
        let mesh = model.mesh.clone();
        let linked = part_folders(&self.doc, &mesh);
        let mut names: Vec<String> = (0..mesh.materials.len() as u16)
            .filter(|m| !linked.iter().any(|(linked, _)| linked == m))
            .map(|m| part_name(&mesh, m))
            .collect();
        // Parts sharing a name share a folder.
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| seen.insert(name.to_lowercase()));
        // The first part ends on top.
        names.reverse();
        if names.is_empty() {
            self.status = "Every part has a folder".into();
            return;
        }
        if self.render_queue.busy() {
            self.status = "Wait for the canvas to finish, then make the folders".into();
            return;
        }
        self.dispatch_document_edit(
            crate::document_edits::Edit::NewFolders(names),
            "Folders made for the model's parts",
            "No folders made",
        );
    }

    fn pump_model(&mut self, ctx: &egui::Context, size: [usize; 2], moving: bool) {
        self.update_part_folders();
        let canvas = [self.doc.width, self.doc.height];
        let view = &mut self.model_view;
        let Some(model) = &view.model else {
            return;
        };
        let key = view.key(canvas, size, moving);
        let shown = view.picture.as_ref().map(|(key, _)| *key);
        if view.draw.is_none() && view.error.is_none() && shown != Some(key) {
            let mesh = model.mesh.clone();
            let texture = view.texture.clone();
            let fresh = key.paint.is_some_and(|(wanted, _)| {
                texture.as_ref().is_none_or(|(have, _)| *have != wanted)
            });
            let doc = fresh.then(|| (self.doc.clone(), view.links.clone()));
            let repaint = ctx.clone();
            view.draw = Some(background::spawn(move || {
                let drawn = draw(&mesh, key, doc, texture);
                repaint.request_repaint();
                drawn
            }));
        }
        // Painting leaves a quick texture: come back for the fine one.
        if view.paint && view.changed.is_some_and(|at| at.elapsed() < SETTLE) {
            ctx.request_repaint_after(INTERVAL);
        }
    }

    fn pump_layout(&mut self, ctx: &egui::Context) {
        let part = self.model_view.focus().or(self.model_view.material);
        let key = (part, [self.doc.width, self.doc.height]);
        let view = &mut self.model_view;
        if view.show_layout
            && view.layout_job.is_none()
            && view.layout.as_ref().is_none_or(|l| l.key != key)
            && let Some(model) = &view.model
        {
            let (mesh, repaint) = (model.mesh.clone(), ctx.clone());
            view.layout_job = Some(background::spawn(move || {
                let layout = make_layout(&mesh, key);
                repaint.request_repaint();
                layout
            }));
        }
    }

    /// The texture layout over the canvas.
    pub(crate) fn paint_model_layout(&self, painter: &egui::Painter, view: crate::view::View) {
        let model = &self.model_view;
        if !model.open || !model.show_layout {
            return;
        }
        let Some(layout) = &model.layout else {
            return;
        };
        let color = model.layout_color.unwrap_or(LAYOUT_COLOR);
        if let Some(texture) = &layout.texture {
            crate::animation_cache::paint_tinted(painter, view, texture, color);
            return;
        }
        let stroke = egui::Stroke::new(1.0_f32, color);
        let size = [self.doc.width as f32, self.doc.height as f32];
        let clip = painter.clip_rect();
        for edge in &layout.edges {
            let line = edge.map(|p| view.to_screen(egui::pos2(p[0] * size[0], p[1] * size[1])));
            if egui::Rect::from_two_pos(line[0], line[1])
                .expand(1.0)
                .intersects(clip)
            {
                painter.line_segment(line, stroke);
            }
        }
    }

    fn place_model(&mut self) {
        let canvas = [self.doc.width, self.doc.height];
        let view = &mut self.model_view;
        let Some(model) = &view.model else {
            return;
        };
        let mesh = model.mesh.clone();
        let mut key = view.key(canvas, canvas, false);
        // Twice the samples costs four times the memory on a huge canvas.
        key.samples = if canvas[0] * canvas[1] <= 40_000_000 {
            2
        } else {
            1
        };
        if let Some((wanted, _)) = &mut key.paint {
            *wanted = (view.texture_stamp(), step(canvas, view.fine()));
        }
        // Placed as it looks, without one part dimmed.
        key.focus = None;
        let doc = key.paint.map(|_| (self.doc.clone(), view.links.clone()));
        view.place = Some(background::spawn(move || {
            let drawn = draw(&mesh, key, doc, None)?;
            cut(&drawn.rgba, canvas).ok_or_else(|| "The model is out of view".into())
        }));
        self.status = "Drawing the model…".into();
    }

    /// Where the ruler made from the model is, while it is unchanged.
    fn model_ruler_index(&self) -> Option<usize> {
        let last = self.model_view.ruler?;
        self.rulers.iter().position(|r| *r == last)
    }

    /// Turns the ruler that follows the model's view on or off.
    fn set_model_ruler(&mut self, on: bool) {
        if on {
            if self.rulers.len() >= crate::rulers::MAX_RULERS {
                self.status = format!("At most {} rulers", crate::rulers::MAX_RULERS);
                return;
            }
            self.model_view.follow_ruler = true;
            self.model_view.ruler = None;
            self.follow_model_ruler();
            if self.model_view.ruler.is_some() {
                self.snapping = true;
                self.status = "Strokes now follow the model's perspective".into();
            }
        } else {
            if let Some(index) = self.model_ruler_index() {
                self.remove_ruler(index);
            }
            self.model_view.follow_ruler = false;
            self.model_view.ruler = None;
            self.status = "Perspective ruler removed".into();
        }
    }

    /// Keeps the model's ruler matched to its view while it follows it.
    fn follow_model_ruler(&mut self) {
        if !self.model_view.follow_ruler {
            return;
        }
        let made = self.model_view.ruler.is_some();
        let index = self.model_ruler_index();
        if made && index.is_none() {
            // Moved by hand or removed: it is the artist's ruler now.
            self.model_view.follow_ruler = false;
            self.model_view.ruler = None;
            return;
        }
        let Some(ruler) = self
            .model_view
            .camera
            .ruler([self.doc.width, self.doc.height])
            .filter(Ruler::usable)
        else {
            if !made {
                self.model_view.follow_ruler = false;
                self.status = "This view has no vanishing points".into();
            }
            return;
        };
        match index {
            Some(index) if self.rulers[index] != ruler => self.rulers[index] = ruler,
            Some(_) => return,
            None => self.push_ruler(ruler),
        }
        self.model_view.ruler = Some(ruler);
    }

    pub(crate) fn model_view_ui(&mut self, ctx: &egui::Context) {
        self.poll_model(ctx);
        if !self.model_view.open {
            return;
        }
        let mut open = true;
        let mut ask = false;
        let mut place = false;
        let mut ruler = None;
        let mut make_folders = false;
        let mut go_to = None;
        let canvas = [self.doc.width, self.doc.height];
        let mut picture = None;
        egui::Window::new("3D model")
            .open(&mut open)
            .default_width(430.0)
            .default_height(620.0)
            .min_width(300.0)
            .show(ctx, |ui| {
                let view = &mut self.model_view;
                ui.horizontal_wrapped(|ui| {
                    ask = ui
                        .button("Open model…")
                        .on_hover_text("A .glb or .obj file")
                        .clicked();
                    if let Some((name, _)) = &view.load {
                        ui.spinner();
                        ui.label(format!("Opening {name}…"));
                    } else if let Some(model) = &view.model {
                        ui.add(egui::Label::new(&model.name).truncate());
                    }
                });
                if let Some(error) = &view.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                let Some(model) = &view.model else {
                    ui.add_space(4.0);
                    ui.label("Open a 3D model to draw from it, or to see this canvas on it as its texture while you paint.");
                    ui.weak("Models stay separate from the project. You can also drop a .glb or .obj file on the window.");
                    return;
                };
                let mesh = model.mesh.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.label("Dragging");
                    for (drag, label, hint) in [
                        (Drag::Turn, "turns the model", "Dragging on the picture turns the model around"),
                        (Drag::Move, "moves the model", "Dragging on the picture slides the model; the middle button always does"),
                        (Drag::Light, "moves the light", "Dragging on the picture moves where the light comes from; the right button always does"),
                    ] {
                        ui.selectable_value(&mut view.drag, drag, label).on_hover_text(hint);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("View from");
                    let quarter = std::f32::consts::FRAC_PI_2;
                    for (label, yaw, pitch) in [
                        ("Front", 0.0, 0.0),
                        ("Side", quarter, 0.0),
                        ("Top", 0.0, 1.55),
                    ] {
                        if ui.button(label).clicked() {
                            view.camera.yaw = yaw;
                            view.camera.pitch = pitch;
                        }
                    }
                    if ui.button("Reset").on_hover_text("The view as it was at first").clicked() {
                        view.camera = Camera {
                            flat: view.camera.flat,
                            ..Camera::default()
                        };
                    }
                });
                // A slider for pens, which have no wheel.
                let mut near = 3.6 / view.camera.distance;
                let zoom = egui::Slider::new(&mut near, 0.1..=20.0).logarithmic(true);
                if ui.add(zoom.show_value(false).text("Zoom")).changed() {
                    view.camera.distance = (3.6 / near).clamp(0.05, 40.0);
                }
                egui::CollapsingHeader::new("Perspective and shading").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("model shading")
                        .selected_text(view.look.shading.name())
                        .show_ui(ui, |ui| {
                            for shading in Shading::ALL {
                                ui.selectable_value(&mut view.look.shading, shading, shading.name());
                            }
                        });
                    ui.checkbox(&mut view.look.clay, "Plain gray")
                        .on_hover_text("Without the model's own colors and textures");
                    ui.checkbox(&mut view.camera.flat, "No perspective");
                });
                if !view.camera.flat {
                    let mut degrees = view.camera.field.to_degrees();
                    let field = egui::Slider::new(&mut degrees, 10.0..=100.0).integer().suffix("°");
                    if ui
                        .add(field.text("Field of view"))
                        .on_hover_text("Wide exaggerates perspective, narrow flattens it")
                        .changed()
                    {
                        // The model keeps its size while the perspective changes.
                        let field = degrees.to_radians();
                        let ratio = (view.camera.field / 2.0).tan() / (field / 2.0).tan();
                        view.camera.distance = (view.camera.distance * ratio).clamp(0.05, 40.0);
                        view.camera.field = field;
                    }
                }
                });
                ui.add_enabled_ui(mesh.has_uv, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut view.paint, "Show this canvas on the model")
                            .on_hover_text("The canvas as the model's texture, updated while you paint")
                            .on_disabled_hover_text("This model has no texture coordinates");
                        if view.paint {
                            let mut sharp = view.sharp(canvas);
                            if ui.checkbox(&mut sharp, "Sharp pixels").changed() {
                                view.sharp = Some(sharp);
                            }
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut view.show_layout, "Show the layout on the canvas")
                            .on_hover_text("The model's UV layout as lines over the canvas; never part of the artwork");
                        if view.show_layout {
                            let current = view.layout_color.unwrap_or(LAYOUT_COLOR);
                            for (name, [r, g, b]) in LAYOUT_COLORS {
                                let color = egui::Color32::from_rgba_unmultiplied(r, g, b, current.a());
                                let chosen = current == color;
                                let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                                ui.painter().rect_filled(rect.shrink(2.0), 3.0, egui::Color32::from_rgb(r, g, b));
                                let outline = if chosen { ui.visuals().selection.stroke } else { ui.visuals().widgets.inactive.bg_stroke };
                                ui.painter().rect_stroke(rect.shrink(1.0), 4.0, outline, egui::StrokeKind::Inside);
                                if response.on_hover_text(format!("{name} lines")).clicked() {
                                    view.layout_color = Some(color);
                                }
                            }
                            let mut color = current;
                            if ui.color_edit_button_srgba(&mut color).on_hover_text("Any color and how strong the lines are").changed() {
                                view.layout_color = Some(color);
                            }
                        }
                        if mesh.materials.len() > 1 && (view.paint || view.show_layout) {
                            let name = |m: Option<u16>| {
                                m.and_then(|m| mesh.materials.get(m as usize))
                                    .map_or("All materials", |m| m.name.as_str())
                            };
                            egui::ComboBox::from_id_salt("model material")
                                .selected_text(name(view.material))
                                .truncate()
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut view.material, None, "All materials");
                                    for index in 0..mesh.materials.len() as u16 {
                                        ui.selectable_value(&mut view.material, Some(index), name(Some(index)));
                                    }
                                });
                        }
                    });
                });
                if mesh.has_uv && mesh.materials.len() > 1 {
                    egui::CollapsingHeader::new(format!("Parts ({})", mesh.materials.len()))
                        .id_salt("model parts")
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "A folder named after a part paints only that part; layers outside \
                                     the parts' folders paint them all.",
                                )
                                .small()
                                .weak(),
                            );
                            for material in 0..mesh.materials.len() as u16 {
                                let folder = view.links.iter().find(|(m, _)| *m == material).map(|(_, f)| *f);
                                ui.horizontal(|ui| {
                                    let name = part_name(&mesh, material);
                                    let painting = view.focus == Some(material);
                                    let text = egui::RichText::new(&name);
                                    ui.label(if painting { text.strong() } else { text });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        match folder {
                                            Some(_) if painting => {
                                                ui.weak("Painting");
                                            }
                                            Some(folder) => {
                                                if ui
                                                    .small_button("Paint it")
                                                    .on_hover_text("Select its folder in the Layers panel")
                                                    .clicked()
                                                {
                                                    go_to = Some(folder);
                                                }
                                            }
                                            None => {
                                                ui.weak("No folder");
                                            }
                                        }
                                    });
                                });
                            }
                            ui.horizontal_wrapped(|ui| {
                                let missing = view.links.len() < mesh.materials.len();
                                make_folders = ui
                                    .add_enabled(missing, egui::Button::new("Make folders for the parts"))
                                    .on_hover_text(
                                        "A folder for each part without one, at the top of the layers, \
                                         each with a layer to paint on",
                                    )
                                    .clicked();
                                let mut highlight = !view.no_highlight;
                                if ui
                                    .checkbox(&mut highlight, "Dim the other parts")
                                    .on_hover_text(
                                        "While a part's folder is selected, the other parts are dimmed \
                                         and only its layout is shown",
                                    )
                                    .changed()
                                {
                                    view.no_highlight = !highlight;
                                }
                            });
                        });
                }
                ui.horizontal_wrapped(|ui| {
                    place = ui
                        .add_enabled(view.place.is_none(), egui::Button::new("Place as layer"))
                        .on_hover_text("Draws this view on a new layer, framed as here")
                        .clicked();
                    let mut follow = view.follow_ruler;
                    if ui
                        .add_enabled(!view.camera.flat, egui::Checkbox::new(&mut follow, "Draw in its perspective"))
                        .on_hover_text(
                            "A perspective ruler that matches this view while it is on, so lines \
                             you draw around the model (a floor, a room, a building) share its \
                             vanishing points. Turn it off to remove the ruler.",
                        )
                        .on_disabled_hover_text("Needs perspective: turn off No perspective")
                        .changed()
                    {
                        ruler = Some(follow);
                    }
                });
                // The picture has the canvas's shape: it shows exactly what
                // Place as layer draws.
                let area = ui.available_size().max(egui::vec2(120.0, 160.0));
                let shape = egui::vec2(canvas[0] as f32, canvas[1] as f32);
                let fit = (area.x / shape.x).min(area.y / shape.y);
                let (whole, response) = ui.allocate_exact_size(area, egui::Sense::click_and_drag());
                let rect = egui::Rect::from_center_size(whole.center(), (shape * fit).max(egui::vec2(1.0, 1.0)));
                ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
                let pixels = rect.size() * ui.ctx().pixels_per_point();
                let scale = (MAX_PICTURE / pixels.max_elem()).min(1.0);
                let size = [pixels.x, pixels.y].map(|v| ((v * scale).round() as usize).max(1));
                if let Some((_, texture)) = &view.picture {
                    let all = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                    ui.painter().image(texture.id(), rect, all, egui::Color32::WHITE);
                }
                let by = response.drag_delta();
                if by != egui::Vec2::ZERO {
                    let drag = if response.dragged_by(egui::PointerButton::Middle) {
                        Drag::Move
                    } else if response.dragged_by(egui::PointerButton::Secondary) {
                        Drag::Light
                    } else {
                        view.drag
                    };
                    // Across the picture is half a turn.
                    let turn = std::f32::consts::PI / rect.size().min_elem().max(1.0);
                    match drag {
                        Drag::Turn => view.camera.orbit([by.x * turn, by.y * turn]),
                        Drag::Move => view.camera.pan(
                            [by.x, by.y],
                            [rect.width().max(1.0) as usize, rect.height().max(1.0) as usize],
                        ),
                        Drag::Light => {
                            let [yaw, pitch] = &mut view.look.light;
                            *yaw = (*yaw + by.x * turn).rem_euclid(std::f32::consts::TAU);
                            *pitch = (*pitch - by.y * turn).clamp(-1.55, 1.55);
                        }
                    }
                }
                if response.hovered() {
                    let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
                    let factor = (scroll * 0.004).exp() * pinch;
                    if factor != 1.0 && factor.is_finite() && factor > 0.0 {
                        view.camera.zoom(factor);
                    }
                }
                ui.painter().text(
                    rect.left_bottom() + egui::vec2(6.0, -6.0),
                    egui::Align2::LEFT_BOTTOM,
                    "Middle button: move · right button: light · scroll or pinch: zoom",
                    egui::FontId::proportional(11.0),
                    ui.visuals().weak_text_color(),
                );
                picture = Some((size, response.dragged()));
            });
        self.model_view.open = open;
        if let Some((size, moving)) = picture {
            self.pump_model(ctx, size, moving);
            self.pump_layout(ctx);
        }
        if ask {
            self.ask_for_model();
        }
        if place {
            self.place_model();
        }
        if make_folders {
            self.make_part_folders();
        }
        if let Some(folder) = go_to {
            self.select_layer_target(folder);
        }
        if let Some(on) = ruler {
            self.set_model_ruler(on);
        } else if self.model_view.open {
            self.follow_model_ruler();
        }
    }

    /// A ball wearing the canvas, for screenshots.
    pub(crate) fn model_capture_fixture(&mut self) {
        self.model_view.wait = true;
        self.model_view.show("ball.glb".into(), mesh::sphere(24));
        self.model_view.paint = true;
        self.model_view.show_layout = true;
        self.model_view.camera.yaw = std::f32::consts::PI;
        self.set_model_ruler(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(app: &mut Dudel, ctx: &egui::Context) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 900.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            crate::workspace::theme(ctx);
            app.model_view_ui(ctx);
        });
    }

    fn with_ball() -> Dudel {
        let mut app = Dudel::default();
        app.model_view.wait = true;
        app.model_view.show("ball.glb".into(), mesh::sphere(12));
        app
    }

    #[test]
    fn the_canvas_shows_on_the_model_and_follows_changes() {
        let mut app = with_ball();
        app.doc = Document::new(64, 64);
        app.model_view.paint = true;
        app.model_view.look.shading = Shading::Colors;
        let ctx = egui::Context::default();
        for _ in 0..6 {
            frame(&mut app, &ctx);
        }
        let shown = |app: &Dudel| {
            let (_, texture) = app.model_view.texture.as_ref().unwrap();
            texture.base.size()
        };
        assert_eq!(shown(&app), [64, 64]);
        let before = app.model_view.texture.clone().unwrap();
        assert!(app.model_view.sharp([64, 64]), "small canvases stay sharp");
        // Paint, and let the change through.
        {
            let mut pixels = app.doc.layers[0].pixels.writer();
            for index in 0..64 * 64 {
                pixels.rgba_mut(index).copy_from_slice(&[200, 10, 10, 255]);
            }
        }
        app.model_view.note_damage(true, &Default::default());
        app.model_view.changed = Some(Instant::now() - SETTLE);
        app.model_view.textured = Some(Instant::now() - INTERVAL);
        for _ in 0..6 {
            frame(&mut app, &ctx);
        }
        let after = app.model_view.texture.clone().unwrap();
        assert!(before.0 != after.0 && !Arc::ptr_eq(&before.1, &after.1));
        // With nothing to do, nothing is drawn again.
        assert!(app.model_view.draw.is_none());
    }

    /// A ball in two parts: the left half "Body", the right "Eyes".
    fn with_parts() -> Dudel {
        let mut app = Dudel::default();
        app.model_view.wait = true;
        let mut mesh = mesh::sphere(12);
        mesh.materials[0].name = "Body".into();
        mesh.materials.push(mesh::Material::plain(""));
        for (index, triangle) in mesh.triangles.clone().iter().enumerate() {
            if mesh.vertices[triangle[0] as usize].at[0] > 0.0 {
                mesh.material[index] = 1;
            }
        }
        app.model_view.show("ball.glb".into(), mesh);
        app.doc = Document::new(32, 32);
        app
    }

    #[test]
    fn each_part_wears_its_own_folder_and_every_part_the_rest() {
        let mut app = with_parts();
        let ctx = egui::Context::default();
        app.make_part_folders();
        let names: Vec<_> = app
            .doc
            .layers
            .iter()
            .filter(|l| l.folder)
            .map(|l| l.name.clone())
            .collect();
        assert_eq!(names, ["Part 2", "Body"], "the first part on top");
        // Making them again makes none.
        let layers = app.doc.layers.len();
        app.make_part_folders();
        assert_eq!(app.doc.layers.len(), layers);
        // Red in Body's folder, blue in the unnamed part's, green outside.
        let fill = |app: &mut Dudel, index: usize, color: [u8; 4]| {
            let mut pixels = app.doc.layers[index].pixels.writer();
            for at in 0..32 * 16 {
                pixels.rgba_mut(at).copy_from_slice(&color);
            }
        };
        let folder =
            |app: &Dudel, name: &str| app.doc.layers.iter().position(|l| l.name == name).unwrap();
        let body = folder(&app, "Body");
        let other = folder(&app, "Part 2");
        fill(&mut app, body - 1, [255, 0, 0, 255]);
        fill(&mut app, other - 1, [0, 0, 255, 255]);
        fill(&mut app, 0, [0, 0, 0, 0]);
        {
            let mut writer = app.doc.layers[0].pixels.writer();
            for at in 32 * 16..32 * 32 {
                writer.rgba_mut(at).copy_from_slice(&[0, 255, 0, 255]);
            }
        }
        app.model_view.paint = true;
        app.model_view.note_damage(true, &Default::default());
        for _ in 0..6 {
            frame(&mut app, &ctx);
        }
        let (_, painted) = app.model_view.texture.clone().unwrap();
        let wears = |material: u16| {
            painted
                .parts
                .iter()
                .find(|(m, _)| *m == material)
                .map(|(_, t)| [t.texel(4, 4), t.texel(4, 28)])
        };
        let shared = [painted.base.texel(4, 4), painted.base.texel(4, 28)];
        assert_eq!(
            shared,
            [[255, 255, 255, 255], [0, 255, 0, 255]],
            "no folder's paint"
        );
        assert_eq!(wears(0), Some([[255, 0, 0, 255], [0, 255, 0, 255]]));
        assert_eq!(wears(1), Some([[0, 0, 255, 255], [0, 255, 0, 255]]));
        // Painting in a part's folder dims the others and shows its layout.
        assert_eq!(
            app.model_view.focus,
            Some(0),
            "the top folder's layer is active"
        );
        assert_eq!(app.model_view.key([32, 32], [10, 10], false).focus, Some(0));
        app.select_layer_target(0);
        frame(&mut app, &ctx);
        assert_eq!(app.model_view.focus, None);
        app.select_layer_target(other - 1);
        frame(&mut app, &ctx);
        assert_eq!(app.model_view.focus, Some(1));
        app.model_view.no_highlight = true;
        assert_eq!(app.model_view.key([32, 32], [10, 10], false).focus, None);
        // Making the folders undoes in one step.
        app.undo();
        assert!(
            app.doc.layers.iter().all(|l| !l.folder),
            "{:?}",
            app.doc.layers.len()
        );
    }

    #[test]
    fn painting_uses_a_quick_texture_and_resting_the_fine_one() {
        let mut app = with_ball();
        app.model_view.paint = true;
        let canvas = [4096, 2048];
        app.model_view.note_damage(true, &Default::default());
        let busy = app.model_view.key(canvas, [100, 100], false);
        assert_eq!(busy.paint.unwrap().0.1, 8);
        assert_eq!(busy.samples, 1);
        app.model_view.changed = Some(Instant::now() - SETTLE);
        let idle = app.model_view.key(canvas, [100, 100], false);
        assert_eq!(idle.paint.unwrap().0.1, 2);
        assert_eq!(idle.samples, 2);
        // A fine texture is not replaced by a coarser one of the same canvas.
        let texture = Texture::new([1, 1], vec![255; 4]).unwrap();
        let painted = Painted {
            base: Arc::new(texture),
            parts: Vec::new(),
        };
        app.model_view.texture = Some(((app.model_view.stamp, 2), Arc::new(painted)));
        app.model_view.changed = Some(Instant::now());
        let again = app.model_view.key(canvas, [100, 100], false);
        assert_eq!(again.paint.unwrap().0.1, 2);
    }

    #[test]
    fn the_view_becomes_one_layer_where_it_is_shown_and_undoes() {
        let mut app = with_ball();
        app.doc = Document::new(200, 120);
        app.model_view.camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        };
        let layers = app.doc.layers.len();
        app.place_model();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.doc.layers.len() == layers || app.render_queue.busy() {
            assert!(Instant::now() < deadline, "{}", app.status);
            let _ = ctx.run(Default::default(), |ctx| {
                app.pump_render_queue(ctx);
                app.poll_paste(ctx);
            });
            std::thread::yield_now();
        }
        assert_eq!(app.doc.layers.len(), layers + 1, "{}", app.status);
        let layer = &app.doc.layers[app.doc.active];
        assert_eq!(layer.name, "ball.glb");
        let alpha = |x: usize, y: usize| app.doc.layers[app.doc.active].pixels.rgba(y * 200 + x)[3];
        assert_eq!(alpha(100, 60), 255);
        assert_eq!((alpha(5, 5), alpha(195, 115)), (0, 0));
        // Centered: as much to the left as to the right.
        let row: Vec<usize> = (0..200).filter(|x| alpha(*x, 60) > 127).collect();
        assert!((row[0] as i32 - (199 - *row.last().unwrap()) as i32).abs() <= 1);
        app.undo();
        assert_eq!(app.doc.layers.len(), layers);
    }

    #[test]
    fn the_ruler_follows_the_view_and_switches_off() {
        let mut app = with_ball();
        app.set_model_ruler(true);
        assert_eq!(app.rulers.len(), 1);
        assert!(app.snapping && app.model_view.follow_ruler);
        let first = app.rulers[0];
        app.model_view.camera.yaw += 0.3;
        app.follow_model_ruler();
        assert_eq!(app.rulers.len(), 1);
        assert!(app.rulers[0] != first, "it follows the view");
        // Off removes it.
        app.set_model_ruler(false);
        assert!(app.rulers.is_empty() && !app.model_view.follow_ruler);
        // One the artist moved is theirs now: it stays, and stops following.
        app.set_model_ruler(true);
        app.rulers[0].set_handle(0, [7.0, 7.0]);
        let moved = app.rulers[0];
        app.model_view.camera.yaw += 0.3;
        app.follow_model_ruler();
        assert_eq!(app.rulers, vec![moved]);
        assert!(!app.model_view.follow_ruler);
        // A view without perspective has no ruler to make.
        app.model_view.camera.flat = true;
        app.set_model_ruler(true);
        assert_eq!(app.rulers.len(), 1);
        assert!(!app.model_view.follow_ruler);
    }

    #[test]
    fn a_file_that_is_not_a_model_says_so_and_keeps_the_one_shown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.glb");
        std::fs::write(&path, "nothing").unwrap();
        let mut app = with_ball();
        app.open_model(path);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        let error = app.model_view.error.clone().unwrap();
        assert!(error.contains("broken.glb"), "{error}");
        assert!(app.model_view.model.is_some());
        // Dropped models are recognized.
        assert_eq!(
            crate::image_paste::classify("a.GLB".into()),
            crate::image_paste::Dropped::Model("a.GLB".into())
        );
    }

    #[test]
    fn large_layouts_become_one_picture() {
        let few = make_layout(&mesh::sphere(8), (None, [300, 200]));
        assert!(few.picture.is_none() && !few.edges.is_empty());
        let many = make_layout(&mesh::sphere(64), (None, [5000, 2500]));
        assert!(many.edges.is_empty());
        assert_eq!(many.picture.unwrap().0, [1667, 834]);
        assert!(cut(&[0; 16], [2, 2]).is_none());
    }
}
