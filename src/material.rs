//! Material layers: layers that know what kind of paint they hold. This is
//! stage four of the canvas material plan, built on the paper (stage one),
//! granulation (two) and wet paint (three).
//!
//! - Watercolor: while a wash dries, water flows toward its drying edges
//!   and carries pigment there, so washes dry with darker rims; fresh water
//!   dropped into a drying wash pushes the paint aside into blooms.
//! - Dry media: every brush catches on the paper's tooth, as pastel and
//!   charcoal do; firmer pressure fills the valleys. With no paper on the
//!   canvas the layer brings a fine tooth of its own.
//! - Thick paint: brushes leave paint that stands up from the canvas. A
//!   relief channel (one byte per pixel, saved with the project) records
//!   how high, with bristle streaks along each stroke, and a light shades
//!   it. The shading is drawn as a layer clipped to the paint whenever the
//!   document is composited, and redrawn only where the relief changed.
use crate::{
    Dudel,
    document::{Document, Layer, LayerId},
    egui,
    layer_mask::Mask,
    pixels::Pixels,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// What a material layer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    Watercolor,
    Dry,
    Paint,
}
impl Kind {
    pub const ALL: [Kind; 3] = [Self::Watercolor, Self::Dry, Self::Paint];
    pub fn name(self) -> &'static str {
        match self {
            Self::Watercolor => "Watercolor",
            Self::Dry => "Dry media",
            Self::Paint => "Thick paint",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Self::Watercolor => {
                "Wet washes dry with darker edges, and fresh water pushes drying paint into blooms. \
                 Paint with a Wet brush (Brush settings › Blending)."
            }
            Self::Dry => {
                "Every brush catches on the paper's tooth like pastel or charcoal; press harder to \
                 fill the valleys. Brings a fine tooth of its own when the canvas has no paper."
            }
            Self::Paint => {
                "Brushes leave paint that stands up, with bristle streaks, lit by a light you \
                 choose. Each brush's Thickness setting says how thick (Brush settings › Texture)."
            }
        }
    }
}

fn default_edges() -> f32 {
    0.6
}
fn default_blooms() -> f32 {
    0.5
}
fn default_tooth() -> f32 {
    0.7
}
fn default_light() -> f32 {
    135.0
}
fn default_depth() -> f32 {
    0.5
}
fn default_shine() -> f32 {
    0.35
}

/// A material layer's settings. Every kind keeps all of them, so switching
/// kinds and back loses nothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub kind: Kind,
    /// Watercolor: how much drying paint gathers at the edges of a wash.
    #[serde(default = "default_edges")]
    pub edges: f32,
    /// Watercolor: how far fresh water pushes drying paint aside.
    #[serde(default = "default_blooms")]
    pub blooms: f32,
    /// Dry media: how much the paper's tooth catches every stroke.
    #[serde(default = "default_tooth")]
    pub tooth: f32,
    /// Thick paint: where the light comes from, in degrees counterclockwise
    /// from the right (135 is the upper left).
    #[serde(default = "default_light")]
    pub light: f32,
    /// Thick paint: how deep the relief looks.
    #[serde(default = "default_depth")]
    pub depth: f32,
    /// Thick paint: how glossy the ridges are.
    #[serde(default = "default_shine")]
    pub shine: f32,
}

impl Settings {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            edges: default_edges(),
            blooms: default_blooms(),
            tooth: default_tooth(),
            light: default_light(),
            depth: default_depth(),
            shine: default_shine(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        if unit(self.edges)
            && unit(self.blooms)
            && unit(self.tooth)
            && unit(self.depth)
            && unit(self.shine)
            && self.light.is_finite()
            && (-180.0..=180.0).contains(&self.light)
        {
            Ok(())
        } else {
            Err("Material setting outside supported range".into())
        }
    }
    /// Lighting depends on these alone.
    fn look(&self) -> [f32; 3] {
        [self.light, self.depth, self.shine]
    }
}

/// A layer's material: its settings, and for thick paint the relief.
#[derive(Clone, Serialize, Deserialize)]
pub struct Material {
    #[serde(flatten)]
    pub settings: Settings,
    /// Thick paint: how high the paint stands at each pixel (0 flat, 255 the
    /// thickest). Saved in the project's relief trailer, not its metadata.
    #[serde(skip)]
    pub relief: Option<Mask>,
    #[serde(skip)]
    cache: Cache,
}

impl Material {
    /// Boxed: most layers have none, and layers stay small.
    pub fn new(settings: Settings) -> Box<Self> {
        Box::new(Self {
            settings,
            relief: None,
            cache: Cache::default(),
        })
    }
    pub fn kind(&self) -> Kind {
        self.settings.kind
    }
    /// Thick paint with some relief: drawn with its light.
    pub(crate) fn lit(&self) -> bool {
        self.settings.kind == Kind::Paint
            && self
                .relief
                .as_ref()
                .is_some_and(|r| r.painted_bounds().is_some())
    }
}

/// Whether two materials are the same snapshot: same settings and the same
/// relief storage.
pub(crate) fn same(a: &Option<Box<Material>>, b: &Option<Box<Material>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.settings == b.settings
                && match (&a.relief, &b.relief) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.shares(b),
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Check a layer's material against the document size.
pub(crate) fn validate(material: &Material, size: [usize; 2], folder: bool) -> Result<(), String> {
    material.settings.validate()?;
    if folder {
        return Err("Folders have no material".into());
    }
    if let Some(relief) = &material.relief {
        if material.settings.kind != Kind::Paint {
            return Err("Only thick paint keeps a relief".into());
        }
        if relief.dimensions() != size || relief.default_value() != 0 {
            return Err("Paint relief differs from the document".into());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Painting

/// The surface dry media feels: the canvas's paper (or, with none, a
/// fine tooth of its own), its relief deepened by `tooth` and drawing in
/// little: dry pigment sits on the peaks until pressure pushes it down.
pub(crate) fn dry_paper(paper: Option<crate::paper::Paper>, tooth: f32) -> crate::paper::Paper {
    let paper = paper.unwrap_or(crate::paper::Paper::presets()[5].1);
    crate::paper::Paper {
        roughness: paper.roughness.max(0.35 + 0.6 * tooth),
        absorbency: paper.absorbency * (1.0 - tooth),
        ..paper
    }
}

/// Bristle streaks: how much paint a bristle at `across` pixels from the
/// middle of the tip carries, 0.5 to 1. Bristles keep their place across
/// the tip as it moves, so streaks run along the stroke.
pub(crate) fn bristle(across: f32, radius: f32) -> f32 {
    let spacing = (radius * 0.16).clamp(1.2, 6.0);
    let hash = |n: f32, seed: i64| {
        let n = (n as i64).wrapping_add(seed).wrapping_mul(0x2545_F491) as u64;
        let n = (n ^ (n >> 15)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        ((n >> 40) & 0xFFFF) as f32 / 65535.0
    };
    // Smooth value noise across the tip: wide clumps of bristles, and
    // single hairs within them, so the ridges are uneven like a real brush.
    let noise = |t: f32, seed: i64| {
        let (i, f) = (t.floor(), t - t.floor());
        let s = f * f * (3.0 - 2.0 * f);
        hash(i, seed) * (1.0 - s) + hash(i + 1.0, seed) * s
    };
    let clumps = noise(across / (spacing * 2.6), 7_919);
    let hairs = noise(across / spacing, 0);
    0.5 + 0.5 * (0.55 * clumps + 0.45 * hairs)
}

/// Thick paint left by one dab at a pixel: the relief there moves toward
/// `height` (0 to 1) by `share`, and never sinks under new paint.
#[inline]
pub(crate) fn build(
    writer: &mut crate::layer_mask::Writer<'_>,
    x: usize,
    y: usize,
    height: f32,
    share: f32,
) {
    let old = f32::from(writer.at(x, y));
    let target = (height * 255.0).clamp(0.0, 255.0);
    if target <= old || share <= 0.0 {
        return;
    }
    let new = old + (target - old) * share.min(1.0);
    writer.set(x, y, new.round() as u8);
}

/// An eraser takes the paint's height away with its color.
#[inline]
pub(crate) fn wear(writer: &mut crate::layer_mask::Writer<'_>, x: usize, y: usize, share: f32) {
    writer.paint(x, y, 0, share);
}

// ---------------------------------------------------------------------------
// Light

/// Elevation of the light above the canvas.
const ELEVATION: f32 = 0.70;
/// Relief height of a full byte at depth 1, in pixels.
const HEIGHT: f32 = 9.0;

/// Shading over `rect`: black where the paint faces away from the light,
/// white on ridges facing it, straight RGBA. `alpha` (the owner's
/// coverage) scales it when given.
fn shade(relief: &Mask, settings: &Settings, rect: [usize; 4], alpha: Option<&Pixels>) -> Vec<u8> {
    let [width, height] = relief.dimensions();
    let [left, top, right, bottom] = rect;
    // The relief with a one-pixel border, edges repeated.
    let (bl, bt) = (left.saturating_sub(1), top.saturating_sub(1));
    let (br, bb) = ((right + 1).min(width), (bottom + 1).min(height));
    let bw = br - bl;
    let mut heights = Vec::with_capacity(bw * (bb - bt));
    for y in bt..bb {
        for x in bl..br {
            heights.push(f32::from(relief.at(x, y)));
        }
    }
    let at = |x: usize, y: usize| {
        let x = x.clamp(bl, br - 1);
        let y = y.clamp(bt, bb - 1);
        heights[(y - bt) * bw + (x - bl)]
    };
    let scale = settings.depth * HEIGHT / 255.0;
    let (sin, cos) = settings.light.to_radians().sin_cos();
    let (ce, se) = (ELEVATION.cos(), ELEVATION.sin());
    // Toward the light: x right, y down, z out of the canvas.
    let light = [cos * ce, -sin * ce, se];
    let owner = alpha.map(|pixels| pixels.region(width, rect));
    let mut out = Vec::with_capacity((right - left) * (bottom - top) * 4);
    let mut i = 0;
    for y in top..bottom {
        for x in left..right {
            let (xm, xp) = (x.saturating_sub(1), x + 1);
            let (ym, yp) = (y.saturating_sub(1), y + 1);
            // Sobel slopes: smooth over the bristle streaks' byte steps.
            let gx = (at(xp, ym) + 2.0 * at(xp, y) + at(xp, yp)
                - at(xm, ym)
                - 2.0 * at(xm, y)
                - at(xm, yp))
                / 8.0
                * scale;
            let gy = (at(xm, yp) + 2.0 * at(x, yp) + at(xp, yp)
                - at(xm, ym)
                - 2.0 * at(x, ym)
                - at(xp, ym))
                / 8.0
                * scale;
            let length = (gx * gx + gy * gy + 1.0).sqrt();
            let normal = [-gx / length, -gy / length, 1.0 / length];
            let facing = normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2];
            let delta = facing - se;
            let mut pixel = if delta < 0.0 {
                [0.0, 0.0, 0.0, (-delta * 2.4).min(0.85)]
            } else {
                // Gloss: the light mirrored about the normal, seen from above.
                let mirror = 2.0 * facing * normal[2] - light[2];
                let gloss = mirror.max(0.0).powi(28) * settings.shine * 1.6;
                [255.0, 255.0, 255.0, (delta * 1.6 + gloss).min(0.9)]
            };
            if let Some(owner) = &owner {
                pixel[3] *= f32::from(owner[i * 4 + 3]) / 255.0;
            }
            let a = (pixel[3] * 255.0).round() as u8;
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.extend_from_slice(&[pixel[0] as u8, pixel[1] as u8, pixel[2] as u8, a]);
            }
            i += 1;
        }
    }
    out
}

/// The last lighting drawn for a layer, to redraw only what changed.
struct Lit {
    relief: Mask,
    look: [f32; 3],
    /// The owner's pixels, when its coverage was applied (a clipped owner).
    owner: Option<Pixels>,
    shading: Pixels,
    id: LayerId,
}

/// Lighting drawn for one layer. Clones share it, so snapshots and the
/// display find the same drawing.
#[derive(Clone, Default)]
pub(crate) struct Cache(Arc<Mutex<Option<Lit>>>);

/// Grow `rect` by `by` pixels inside `size`.
fn grow(rect: [usize; 4], by: usize, [w, h]: [usize; 2]) -> [usize; 4] {
    [
        rect[0].saturating_sub(by),
        rect[1].saturating_sub(by),
        (rect[2] + by).min(w),
        (rect[3] + by).min(h),
    ]
}

/// Draw `rects` of the shading into `pixels`, in parallel bands.
fn draw_into(
    pixels: &mut Pixels,
    relief: &Mask,
    settings: &Settings,
    rects: &[[usize; 4]],
    owner: Option<&Pixels>,
) {
    let width = relief.dimensions()[0];
    let drawn: Vec<Vec<u8>> = if rects.len() > 4 {
        crate::parallel::run(rects.len(), |i| shade(relief, settings, rects[i], owner))
            .into_iter()
            .map(|r| r.expect("a shading band"))
            .collect()
    } else {
        rects
            .iter()
            .map(|rect| shade(relief, settings, *rect, owner))
            .collect()
    };
    let mut writer = pixels.writer();
    for (rect, bytes) in rects.iter().zip(drawn) {
        writer.write_region(width, *rect, &bytes);
        writer.reclaim_region(width, *rect);
    }
}

/// The shading layer for a thick paint `owner`, clipped to it; None when
/// it has no relief.
pub(crate) fn light_layer(owner: &Layer, size: [usize; 2]) -> Option<Layer> {
    let material = owner.material.as_ref()?;
    if !material.lit() {
        return None;
    }
    let relief = material.relief.as_ref()?;
    let settings = &material.settings;
    let look = settings.look();
    // A clipped owner cannot have layers clipped to it, so the shading
    // carries its coverage instead.
    let owner_pixels = owner.clipped.then(|| owner.pixels.clone());
    let mut cache = material.cache.0.lock().unwrap_or_else(|e| e.into_inner());
    let reusable = cache.as_ref().filter(|lit| {
        lit.look == look
            && lit.relief.dimensions() == size
            && match (&lit.owner, &owner_pixels) {
                (None, None) => true,
                (Some(a), Some(b)) => a.shares(b),
                _ => false,
            }
    });
    let lit = match reusable {
        Some(lit) if lit.relief.shares(relief) => cache.as_ref().unwrap(),
        Some(_) => {
            let lit = cache.as_mut().unwrap();
            // Redraw around the relief tiles that changed: slopes reach one
            // pixel past them.
            let rects: Vec<[usize; 4]> = lit
                .relief
                .changed_tiles(relief)
                .into_iter()
                .map(|rect| grow(rect, 2, size))
                .collect();
            draw_into(
                &mut lit.shading,
                relief,
                settings,
                &rects,
                owner_pixels.as_ref(),
            );
            lit.relief = relief.clone();
            lit
        }
        None => {
            let mut shading = Pixels::sparse(size[0], size[1]);
            let rects: Vec<[usize; 4]> = relief
                .painted_tiles()
                .into_iter()
                .map(|rect| grow(rect, 2, size))
                .collect();
            draw_into(
                &mut shading,
                relief,
                settings,
                &rects,
                owner_pixels.as_ref(),
            );
            let id = cache.as_ref().map_or_else(LayerId::new, |lit| lit.id);
            *cache = Some(Lit {
                relief: relief.clone(),
                look,
                owner: owner_pixels,
                shading,
                id,
            });
            cache.as_ref().unwrap()
        }
    };
    let mut layer = Layer::raster(format!("{}'s light", owner.name), size[0], size[1]);
    layer.id = lit.id;
    layer.pixels = lit.shading.clone();
    layer.visible = owner.visible;
    layer.clipped = true;
    layer.depth = owner.depth;
    Some(layer)
}

// ---------------------------------------------------------------------------
// Saving

/// Trailer of a project with thick paint: each relief, in layer order.
pub(crate) const TRAILER: &[u8; 5] = b"RELF\x01";

/// Write the reliefs of `doc`'s layers, if any.
pub(crate) fn encode_trailer(
    doc: &Document,
    writer: &mut impl std::io::Write,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    let reliefs: Vec<(usize, &Mask)> = doc
        .layers
        .iter()
        .enumerate()
        .filter_map(|(i, l)| Some((i, l.material.as_ref()?.relief.as_ref()?)))
        .collect();
    if reliefs.is_empty() {
        return Ok(());
    }
    writer.write_all(TRAILER).map_err(|e| e.to_string())?;
    writer
        .write_all(&(reliefs.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    for (index, relief) in reliefs {
        writer
            .write_all(&(index as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        relief.encode(&mut *writer, cancel)?;
    }
    Ok(())
}

/// Read the reliefs written by `encode_trailer` (after its tag).
pub(crate) fn decode_trailer(
    doc: &mut Document,
    reader: &mut impl std::io::Read,
    payload_budget: usize,
) -> Result<usize, String> {
    let mut word = [0u8; 4];
    let mut number = |reader: &mut dyn std::io::Read| -> Result<usize, String> {
        reader
            .read_exact(&mut word)
            .map_err(|e| format!("Damaged paint relief: {e}"))?;
        Ok(u32::from_le_bytes(word) as usize)
    };
    let count = number(reader)?;
    if count == 0 || count > doc.layers.len() {
        return Err("Invalid paint relief count".into());
    }
    let mut used = 0usize;
    let mut previous = None;
    for _ in 0..count {
        let index = number(reader)?;
        if index >= doc.layers.len() || previous.is_some_and(|p| index <= p) {
            return Err("Invalid or unordered paint relief".into());
        }
        previous = Some(index);
        let size = [doc.width, doc.height];
        let material = doc.layers[index]
            .material
            .as_mut()
            .filter(|m| m.settings.kind == Kind::Paint)
            .ok_or("Paint relief for a layer without thick paint")?;
        let relief = Mask::decode(
            &mut *reader,
            size,
            payload_budget.saturating_sub(used),
            None,
        )?;
        if relief.default_value() != 0 {
            return Err("Invalid paint relief".into());
        }
        used += relief.payload_bytes();
        material.relief = Some(relief);
    }
    Ok(used)
}

// ---------------------------------------------------------------------------
// Following the pixels

/// Carry the active layer's relief through a whole-layer move of its pixels
/// (`apply` maps a relief to its moved copy). Nothing happens when the layer
/// has no relief.
pub(crate) fn follow(
    layer: &mut Layer,
    apply: impl FnOnce(&Mask) -> Result<Mask, String>,
) -> Result<(), String> {
    if let Some(material) = &mut layer.material
        && let Some(relief) = &material.relief
    {
        material.relief = Some(apply(relief)?);
    }
    Ok(())
}

/// How much of the pixel at (x, y) an edit limited to `doc`'s selection
/// takes along: 0 to 1, and everything without a selection.
pub(crate) fn selected(doc: &Document) -> impl Fn(usize, usize) -> f32 + '_ {
    move |x, y| {
        if let Some(mask) = &doc.selection_mask {
            f32::from(mask.at(x, y)) / 255.0
        } else if let Some(s) = doc.selection {
            f32::from(u8::from(
                x >= s.left && x < s.right && y >= s.top && y < s.bottom,
            ))
        } else {
            1.0
        }
    }
}

/// The relief after selected paint moved: what was not taken along stays,
/// and what was arrives where `source` says the paint at each pixel came
/// from. Where both meet, the higher paint shows.
pub(crate) fn moved_relief(
    relief: &Mask,
    selected: impl Fn(usize, usize) -> f32,
    source: impl Fn(usize, usize) -> Option<[usize; 2]>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Mask, String> {
    let [w, h] = relief.dimensions();
    Mask::generate(w, h, 0, cancel, |x, y| {
        let stays = f32::from(relief.at(x, y)) * (1.0 - selected(x, y));
        let arrives = source(x, y)
            .filter(|[sx, sy]| *sx < w && *sy < h)
            .map_or(0.0, |[sx, sy]| {
                f32::from(relief.at(sx, sy)) * selected(sx, sy)
            });
        Ok(stays.max(arrives).round() as u8)
    })
}

// ---------------------------------------------------------------------------
// Dialog

const PREVIEW: [usize; 2] = [348, 110];

pub(crate) struct MaterialForm {
    target: LayerId,
    layer_name: String,
    /// None is a plain layer.
    settings: Option<Settings>,
    /// Remembered per kind while the dialog is open, so trying another kind
    /// and coming back keeps the changes.
    kept: Settings,
    paper: Option<crate::paper::Paper>,
    preview: Option<(Option<Settings>, egui::TextureHandle)>,
}

/// A sample stroke on the material, painted and dried by the real engine.
fn preview_image(
    settings: Option<Settings>,
    paper: Option<crate::paper::Paper>,
) -> egui::ColorImage {
    let [w, h] = PREVIEW;
    let mut doc = Document::new(w, h);
    doc.paper = paper;
    doc.layers[0].material = settings.map(Material::new);
    let kind = settings.map(|s| s.kind);
    let brush = match kind {
        Some(Kind::Watercolor) => crate::brush::Brush {
            size: 34.0,
            opacity: 0.9,
            flow: 0.1,
            hardness: 0.7,
            spacing: 0.06,
            pressure_size: false,
            blend: crate::brush::Blend::Wet,
            wetness: 0.9,
            paper_response: 0.2,
            ..Default::default()
        },
        Some(Kind::Paint) => crate::brush::Brush {
            size: 30.0,
            hardness: 0.9,
            spacing: 0.05,
            thickness: 0.8,
            pressure_size: true,
            ..Default::default()
        },
        _ => crate::brush::Brush {
            size: 26.0,
            hardness: 0.7,
            flow: 0.6,
            spacing: 0.05,
            paper_response: 0.0,
            pressure_size: false,
            pressure_opacity: true,
            ..Default::default()
        },
    };
    let color = match kind {
        Some(Kind::Watercolor) => [40, 90, 170],
        Some(Kind::Paint) => [196, 84, 40],
        _ => [60, 52, 64],
    };
    let settings = crate::stroke_buffer::Settings {
        target: crate::paint_target::Target::Raster,
        brush: brush.clone(),
        color,
        secondary: color,
        erase: false,
        layer: 0,
        symmetry: Default::default(),
    };
    let mut stroke = crate::stroke::Stroke::default();
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    let steps = 40;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let sample = crate::stroke::Sample {
            x: 30.0 + t * (w as f32 - 60.0),
            y: h as f32 / 2.0 + (t * std::f32::consts::TAU).sin() * 18.0,
            pressure: 0.35 + 0.65 * (t * std::f32::consts::PI).sin(),
            pressure_known: true,
            ..Default::default()
        };
        let dabs = if i == steps {
            stroke.finish(sample, &brush)
        } else {
            stroke.sample(sample, &brush)
        };
        for dab in dabs {
            buffer.paint(&mut doc, dab, &settings);
        }
    }
    buffer.finish_tail(&mut doc, &settings);
    crate::wet::settle(&mut doc);
    let mut rgba = paper_backdrop(paper);
    let painted = crate::layer_effects::expand(&doc, false, &|| false)
        .map(|doc| doc.composite_transparent())
        .unwrap_or_default();
    for (i, p) in painted.chunks_exact(4).enumerate() {
        let a = f32::from(p[3]) / 255.0;
        for c in 0..3 {
            let under = f32::from(rgba[i * 4 + c]);
            rgba[i * 4 + c] = (f32::from(p[c]) * a + under * (1.0 - a)).round() as u8;
        }
    }
    egui::ColorImage::from_rgba_unmultiplied(PREVIEW, &rgba)
}

/// The paper the preview is painted on, lightly shaded by its height.
fn paper_backdrop(paper: Option<crate::paper::Paper>) -> Vec<u8> {
    let [w, h] = PREVIEW;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let tone = paper.map_or(250.0, |p| {
                236.0 + 16.0 * p.height(x, y) * p.roughness.max(0.3)
            });
            let tone = tone.min(255.0) as u8;
            rgba.extend_from_slice(&[tone, tone, tone.saturating_sub(4), 255]);
        }
    }
    rgba
}

impl Dudel {
    /// Layer › Material…
    pub(crate) fn open_material_dialog(&mut self) {
        let Some(layer) = self.doc.layers.get(self.doc.active) else {
            return;
        };
        if !Self::takes_material(layer) {
            self.status = "Materials belong to raster layers".into();
            return;
        }
        let settings = layer.material.as_ref().map(|m| m.settings);
        self.material_form = Some(MaterialForm {
            target: layer.id,
            layer_name: layer.name.clone(),
            settings,
            kept: settings.unwrap_or(Settings::new(Kind::Watercolor)),
            paper: self.doc.paper,
            preview: None,
        });
    }

    /// Screenshot fixture: a watercolor wash, dry media and thick paint on
    /// cold-pressed paper, with the thick paint layer's material open.
    pub(crate) fn material_capture_fixture(&mut self) {
        let (w, h) = (1400usize, 900usize);
        let mut doc = Document::new(w, h);
        doc.paper = Some(crate::paper::Paper::presets()[1].1);
        doc.layers[0].name = "Paper tone".into();
        let stroke = |doc: &mut Document,
                      brush: &crate::brush::Brush,
                      color: [u8; 3],
                      from: [f32; 2],
                      to: [f32; 2],
                      wave: f32| {
            let settings = crate::stroke_buffer::Settings {
                target: crate::paint_target::Target::Raster,
                brush: brush.clone(),
                color,
                secondary: color,
                erase: false,
                layer: doc.active,
                symmetry: Default::default(),
            };
            let mut path = crate::stroke::Stroke::default();
            let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
            let steps = 90;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let sample = crate::stroke::Sample {
                    x: from[0] + (to[0] - from[0]) * t,
                    y: from[1] + (to[1] - from[1]) * t + (t * std::f32::consts::TAU).sin() * wave,
                    pressure: 0.45 + 0.55 * (t * std::f32::consts::PI).sin(),
                    pressure_known: true,
                    ..Default::default()
                };
                let dabs = if i == steps {
                    path.finish(sample, brush)
                } else {
                    path.sample(sample, brush)
                };
                for dab in dabs {
                    buffer.paint(doc, dab, &settings);
                }
            }
            buffer.finish_tail(doc, &settings);
        };
        // Watercolor washes.
        doc.add_layer();
        doc.layers[1].name = "Watercolor 1".into();
        doc.layers[1].material = Some(Material::new(Settings::new(Kind::Watercolor)));
        let wash = crate::brush::Brush {
            size: 90.0,
            opacity: 0.9,
            flow: 0.07,
            hardness: 0.7,
            spacing: 0.06,
            pressure_size: false,
            blend: crate::brush::Blend::Wet,
            wetness: 0.9,
            granulation: 0.4,
            paper_response: 0.2,
            ..Default::default()
        };
        for (i, color) in [[46, 98, 170], [52, 120, 150], [120, 70, 150]]
            .into_iter()
            .enumerate()
        {
            let y = 140.0 + i as f32 * 70.0;
            stroke(&mut doc, &wash, color, [90.0, y], [620.0, y + 20.0], 16.0);
            crate::wet::settle(&mut doc);
        }
        // Yellow into wet blue: the pigments make green.
        let mixing = crate::brush::Brush {
            size: 60.0,
            flow: 0.18,
            ..wash.clone()
        };
        stroke(
            &mut doc,
            &mixing,
            [24, 50, 190],
            [110.0, 395.0],
            [600.0, 395.0],
            0.0,
        );
        stroke(
            &mut doc,
            &mixing,
            [250, 215, 30],
            [110.0, 425.0],
            [600.0, 395.0],
            0.0,
        );
        crate::wet::settle(&mut doc);
        // Dry media.
        doc.add_layer();
        doc.layers[2].name = "Dry media 1".into();
        doc.layers[2].material = Some(Material::new(Settings::new(Kind::Dry)));
        let chalk = crate::brush::Brush {
            size: 34.0,
            hardness: 0.6,
            flow: 0.5,
            spacing: 0.05,
            paper_response: 0.0,
            pressure_size: false,
            pressure_opacity: true,
            ..Default::default()
        };
        for (i, color) in [[40, 36, 44], [180, 60, 60]].into_iter().enumerate() {
            let y = 560.0 + i as f32 * 90.0;
            stroke(&mut doc, &chalk, color, [90.0, y], [620.0, y - 30.0], 22.0);
        }
        // Thick paint.
        doc.add_layer();
        doc.layers[3].name = "Thick paint 1".into();
        doc.layers[3].material = Some(Material::new(Settings::new(Kind::Paint)));
        let knife = crate::brush::Brush {
            size: 70.0,
            hardness: 0.92,
            spacing: 0.04,
            thickness: 0.85,
            pressure_size: true,
            ..Default::default()
        };
        for (i, color) in [
            [214, 92, 44],
            [236, 186, 60],
            [64, 128, 104],
            [230, 226, 214],
        ]
        .into_iter()
        .enumerate()
        {
            let y = 170.0 + i as f32 * 150.0;
            stroke(
                &mut doc,
                &knife,
                color,
                [760.0, y],
                [1300.0, y + 60.0],
                30.0,
            );
        }
        // A knife dragged down through the paint carries it and its height.
        let knife = crate::brush::Brush {
            size: 50.0,
            hardness: 0.9,
            opacity: 0.9,
            spacing: 0.04,
            pressure_size: false,
            blend: crate::brush::Blend::Smudge,
            ..Default::default()
        };
        stroke(
            &mut doc,
            &knife,
            [0, 0, 0],
            [1180.0, 150.0],
            [1180.0, 760.0],
            0.0,
        );
        doc.active = 3;
        self.doc = doc;
        self.changed();
        self.open_material_dialog();
    }

    /// "New material layer" with one entry per kind.
    pub(crate) fn new_material_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("New material layer", |ui| {
            for kind in Kind::ALL {
                if ui
                    .button(kind.name())
                    .on_hover_text(kind.describe())
                    .clicked()
                {
                    self.new_material_layer(kind);
                    ui.close();
                }
            }
        });
    }

    /// Raster layers can hold a material; folders, vector, text and
    /// adjustment layers cannot.
    pub(crate) fn takes_material(layer: &Layer) -> bool {
        !layer.folder
            && layer.vector.is_none()
            && layer.text.is_none()
            && layer.adjustment.is_none()
    }

    /// A new raster layer holding `kind`, above the selected entry and
    /// selected afterwards.
    pub(crate) fn new_material_layer(&mut self, kind: Kind) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(active) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let id = LayerId::new();
        let edit = crate::document_edits::Edit::NewMaterial {
            id,
            settings: Settings::new(kind),
        };
        if !self.render_queue.busy() {
            self.dispatch_document_edit(edit, "New material layer", "No layer created");
            return;
        }
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_new_layer(active, id) {
            Ok(projected) => projected,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(edit, "New material layer", "No layer created") {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(id);
        }
    }

    pub(crate) fn material_dialog(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.material_form else {
            return;
        };
        let (mut apply, mut cancel) = (false, false);
        let canvas = self.canvas_area;
        let mut modal = egui::Modal::new(egui::Id::new("material dialog"))
            .backdrop_color(egui::Color32::TRANSPARENT);
        if canvas.width() >= 640.0 && canvas.height() >= 360.0 {
            modal = modal.area(
                egui::Modal::default_area(egui::Id::new("material dialog")).anchor(
                    egui::Align2::RIGHT_TOP,
                    canvas.right_top().to_vec2() - ctx.content_rect().right_top().to_vec2()
                        + egui::vec2(-12.0, 12.0),
                ),
            );
        }
        let response = modal.show(ctx, |ui| {
            ui.set_width(PREVIEW[0] as f32 + 16.0);
            cancel |= crate::dialogs::dialog_title(ui, "Layer material");
            ui.label(egui::RichText::new(&form.layer_name).weak());
            if form
                .preview
                .as_ref()
                .is_none_or(|(shown, _)| *shown != form.settings)
            {
                let image = preview_image(form.settings, form.paper);
                match &mut form.preview {
                    Some((shown, texture)) => {
                        texture.set(image, egui::TextureOptions::LINEAR);
                        *shown = form.settings;
                    }
                    None => {
                        form.preview = Some((
                            form.settings,
                            ctx.load_texture("material preview", image, egui::TextureOptions::LINEAR),
                        ));
                    }
                }
            }
            if let Some((_, texture)) = &form.preview {
                ui.image((texture.id(), egui::vec2(PREVIEW[0] as f32, PREVIEW[1] as f32)));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(form.settings.is_none(), "Plain")
                    .on_hover_text("An ordinary layer: paint is only color")
                    .clicked()
                    && let Some(settings) = form.settings.take()
                {
                    form.kept = settings;
                }
                for kind in Kind::ALL {
                    let chosen = form.settings.is_some_and(|s| s.kind == kind);
                    if ui
                        .selectable_label(chosen, kind.name())
                        .on_hover_text(kind.describe())
                        .clicked()
                        && !chosen
                    {
                        let mut settings = form.settings.unwrap_or(form.kept);
                        settings.kind = kind;
                        form.settings = Some(settings);
                    }
                }
            });
            if let Some(settings) = &mut form.settings {
                ui.label(egui::RichText::new(settings.kind.describe()).small());
                ui.add_space(4.0);
                let slider = |ui: &mut egui::Ui, label: &str, value: &mut f32, hint: &str| {
                    crate::workspace::labeled_slider(ui, label, egui::Slider::new(value, 0.0..=1.0))
                        .on_hover_text(hint);
                };
                match settings.kind {
                    Kind::Watercolor => {
                        slider(ui, "Edge darkening", &mut settings.edges, "How much paint gathers at the edges of a wash as it dries");
                        slider(ui, "Blooms", &mut settings.blooms, "How far fresh water pushes paint that is drying");
                    }
                    Kind::Dry => {
                        slider(ui, "Tooth", &mut settings.tooth, "How much the paper's tooth catches every stroke on this layer");
                    }
                    Kind::Paint => {
                        crate::workspace::labeled_slider(
                            ui,
                            "Light from",
                            egui::Slider::new(&mut settings.light, -180.0..=180.0)
                                .suffix("°")
                                .integer(),
                        )
                        .on_hover_text("Where the light comes from: 0° the right, 90° the top, 135° the upper left");
                        slider(ui, "Depth", &mut settings.depth, "How deep the paint looks");
                        slider(ui, "Shine", &mut settings.shine, "How glossy the ridges are");
                    }
                }
            }
            ui.small(
                "The material is saved with the layer. Watercolor and dry media change how new \
                 strokes behave; thick paint's light can be changed at any time.",
            );
            crate::dialogs::dialog_buttons(ui, |ui| {
                apply = ui.add(crate::workspace::primary_button(ui, "Apply")).clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if apply {
            let Some(form) = self.material_form.take() else {
                return;
            };
            match self.doc.layers.iter().position(|l| l.id == form.target) {
                Some(index) => {
                    if index == self.doc.active || self.select_layer_target(index) {
                        self.dispatch_document_edit(
                            crate::document_edits::Edit::Material(form.settings),
                            "Material changed",
                            "The layer already has this material",
                        );
                    }
                }
                None => self.status = "The layer is gone".into(),
            }
        } else if cancel || (response.should_close() && escape) {
            self.material_form = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(doc: &mut Document, brush: &crate::brush::Brush, color: [u8; 3], y: f32, erase: bool) {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color,
            secondary: color,
            erase,
            layer: doc.active,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=20 {
            let sample = crate::stroke::Sample {
                x: 20.0 + i as f32 * 6.0,
                y,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 20 {
                path.finish(sample, brush)
            } else {
                path.sample(sample, brush)
            };
            for dab in dabs {
                buffer.paint(doc, dab, &settings);
            }
        }
        buffer.finish_tail(doc, &settings);
    }

    fn thick() -> crate::brush::Brush {
        crate::brush::Brush {
            size: 24.0,
            hardness: 0.9,
            spacing: 0.08,
            thickness: 0.8,
            pressure_size: false,
            ..Default::default()
        }
    }

    #[test]
    fn thick_paint_builds_relief_with_streaks_and_the_light_shades_it() {
        let mut doc = Document::new(160, 60);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut doc, &thick(), [200, 80, 40], 30.0, false);
        let relief = doc.layers[0]
            .material
            .as_ref()
            .unwrap()
            .relief
            .clone()
            .unwrap();
        // Across the stroke, the bristles leave ridges of different heights.
        let column: Vec<u8> = (22..39).map(|y| relief.at(80, y)).collect();
        let (low, high) = column
            .iter()
            .fold((255, 0), |(l, h), v| (l.min(*v), h.max(*v)));
        assert!(high > 120, "thick paint stands up: {column:?}");
        assert!(high - low > 30, "bristle streaks vary: {column:?}");
        // Along the stroke, a bristle's ridge stays put.
        let along: Vec<u8> = (60..100).step_by(5).map(|x| relief.at(x, 30)).collect();
        let spread = along.iter().max().unwrap() - along.iter().min().unwrap();
        assert!(spread < 40, "streaks run along the stroke: {along:?}");
        assert_eq!(relief.at(80, 5), 0, "no paint, no relief");
        assert!(doc.needs_expansion());
        let light = light_layer(&doc.layers[0], [160, 60]).unwrap();
        assert!(light.clipped);
        // The upper edge faces the upper-left light, the lower edge turns away.
        let column = |y: usize| light.pixels.rgba(y * 160 + 80);
        let lit = (18..30).map(column).find(|p| p[0] == 255 && p[3] > 40);
        let dark = (30..43).map(column).find(|p| p[0] == 0 && p[3] > 40);
        assert!(lit.is_some() && dark.is_some(), "{lit:?} {dark:?}");
        // The composite shows the shading.
        let flat = {
            let mut plain = doc.clone();
            plain.layers[0].material = None;
            plain.composite_transparent()
        };
        assert_ne!(doc.composite_transparent(), flat);
    }

    #[test]
    fn the_light_redraws_only_where_the_relief_changed() {
        let mut doc = Document::new(400, 200);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut doc, &thick(), [200, 80, 40], 40.0, false);
        let first = light_layer(&doc.layers[0], [400, 200]).unwrap();
        // A second stroke far below leaves the first stroke's tiles alone.
        paint(&mut doc, &thick(), [200, 80, 40], 170.0, false);
        let second = light_layer(&doc.layers[0], [400, 200]).unwrap();
        assert_eq!(first.id, second.id, "the display sees the same layer");
        let changed = first.pixels.changed_regions(&second.pixels, 400, 200);
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|r| r[1] >= 64), "{changed:?}");
        // Drawn incrementally equals drawn from scratch.
        let mut fresh = doc.layers[0].clone();
        fresh.material.as_mut().unwrap().cache = Cache::default();
        let scratch = light_layer(&fresh, [400, 200]).unwrap();
        assert_eq!(
            scratch.pixels.region(400, [0, 0, 400, 200]),
            second.pixels.region(400, [0, 0, 400, 200])
        );
        // Turning the light redraws everything.
        doc.layers[0].material.as_mut().unwrap().settings.light = -45.0;
        let turned = light_layer(&doc.layers[0], [400, 200]).unwrap();
        assert_ne!(
            turned.pixels.region(400, [0, 0, 400, 200]),
            second.pixels.region(400, [0, 0, 400, 200])
        );
    }

    #[test]
    fn erasing_thick_paint_flattens_it() {
        let mut doc = Document::new(160, 60);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut doc, &thick(), [200, 80, 40], 30.0, false);
        let eraser = crate::brush::Brush {
            size: 40.0,
            ..thick()
        };
        paint(&mut doc, &eraser, [0, 0, 0], 30.0, true);
        let relief = doc.layers[0]
            .material
            .as_ref()
            .unwrap()
            .relief
            .clone()
            .unwrap();
        assert!(
            (40..120).all(|x| relief.at(x, 30) < 30),
            "{}",
            relief.at(80, 30)
        );
    }

    #[test]
    fn dry_media_catches_on_the_tooth_and_pressure_fills_it() {
        let soft = crate::brush::Brush {
            size: 16.0,
            hardness: 0.8,
            flow: 0.35,
            spacing: 0.1,
            paper_response: 0.0,
            pressure_size: false,
            ..Default::default()
        };
        let covered = |kind: Option<Kind>, brush: &crate::brush::Brush| {
            let mut doc = Document::new(160, 60);
            doc.layers[0].material = kind.map(|k| Material::new(Settings::new(k)));
            paint(&mut doc, brush, [0, 0, 0], 30.0, false);
            (26..34)
                .flat_map(|y| (30..130).map(move |x| (x, y)))
                .filter(|(x, y)| doc.layers[0].pixels.rgba(y * 160 + x)[3] > 0)
                .count()
        };
        let plain = covered(None, &soft);
        let dry = covered(Some(Kind::Dry), &soft);
        assert_eq!(plain, 800);
        assert!(
            dry < plain * 9 / 10,
            "the tooth breaks the stroke up: {dry}"
        );
        let firm = crate::brush::Brush { flow: 1.0, ..soft };
        assert!(
            covered(Some(Kind::Dry), &firm) > dry,
            "firm strokes fill more"
        );
    }

    #[test]
    fn watercolor_washes_dry_with_darker_edges() {
        let wash = crate::brush::Brush {
            size: 30.0,
            hardness: 0.9,
            flow: 0.08,
            spacing: 0.1,
            pressure_size: false,
            blend: crate::brush::Blend::Wet,
            wetness: 1.0,
            paper_response: 0.0,
            ..Default::default()
        };
        let dried = |kind: Option<Kind>| {
            let mut doc = Document::new(160, 70);
            doc.layers[0].material = kind.map(|k| Material::new(Settings::new(k)));
            // A broad wash: overlapping strokes with an even middle.
            for y in [22.0, 27.0, 32.0, 37.0] {
                paint(&mut doc, &wash, [30, 60, 160], y, false);
            }
            crate::wet::settle(&mut doc);
            let alpha = |y: usize| {
                (60..100)
                    .map(|x| f32::from(doc.layers[0].pixels.rgba(y * 160 + x)[3]))
                    .sum::<f32>()
                    / 40.0
            };
            // Walking in from the top edge: how far the paint falls back
            // after the darkest point so far, which is a rim.
            let mut darkest = 0.0f32;
            let mut rim = 0.0f32;
            for y in 5..30 {
                let a = alpha(y);
                darkest = darkest.max(a);
                rim = rim.max(darkest - a);
            }
            rim
        };
        assert!(dried(None) < 1.0, "a plain wash rises evenly to its middle");
        let rim = dried(Some(Kind::Watercolor));
        assert!(
            rim > 10.0,
            "a watercolor wash dries with a darker rim: {rim}"
        );
    }

    fn wet(color_flow: f32, pigment: f32) -> crate::brush::Brush {
        crate::brush::Brush {
            size: 30.0,
            hardness: 0.9,
            flow: color_flow,
            spacing: 0.1,
            pressure_size: false,
            blend: crate::brush::Blend::Wet,
            wetness: 1.0,
            pigment,
            paper_response: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn wet_blue_and_yellow_make_green_on_watercolor_and_gray_on_plain() {
        let mixed = |kind: Option<Kind>| {
            let mut doc = Document::new(160, 60);
            doc.layers[0].material = kind.map(|k| Material::new(Settings::new(k)));
            paint(&mut doc, &wet(0.5, 1.0), [20, 40, 200], 30.0, false);
            // Into the wet blue, before it dries.
            paint(&mut doc, &wet(0.12, 1.0), [250, 220, 20], 30.0, false);
            doc.layers[0].pixels.rgba(30 * 160 + 80)
        };
        let green = |[r, g, b, _]: [u8; 4]| {
            let [r, g, b] = [r, g, b].map(i32::from);
            g > r + 30 && g > b + 30
        };
        let watercolor = mixed(Some(Kind::Watercolor));
        assert!(green(watercolor), "green on watercolor: {watercolor:?}");
        let plain = mixed(None);
        assert!(!green(plain), "plain mixes in RGB: {plain:?}");
    }

    #[test]
    fn wet_paint_running_together_mixes_as_pigment_while_it_dries() {
        let seam = |kind: Option<Kind>| {
            let mut doc = Document::new(64, 16);
            doc.layers[0].material = kind.map(|k| Material::new(Settings::new(k)));
            {
                let mut writer = doc.layers[0].pixels.writer();
                for y in 0..16 {
                    for x in 0..64 {
                        let color = if x < 32 {
                            [20, 40, 200]
                        } else {
                            [250, 220, 20]
                        };
                        writer
                            .rgba_mut(y * 64 + x)
                            .copy_from_slice(&[color[0], color[1], color[2], 255]);
                    }
                }
            }
            let id = doc.layers[0].id;
            for y in 0..16 {
                doc.wet.add_row(id, y, 0, &[1.0; 64]);
            }
            crate::wet::settle(&mut doc);
            doc.layers[0].pixels.rgba(8 * 64 + 32)
        };
        let green = |[r, g, b, _]: [u8; 4]| {
            let [r, g, b] = [r, g, b].map(i32::from);
            g > r + 25 && g > b + 25
        };
        let watercolor = seam(Some(Kind::Watercolor));
        assert!(
            green(watercolor),
            "blue and yellow ran into green: {watercolor:?}"
        );
        let plain = seam(None);
        assert!(!green(plain), "a plain layer mixes in RGB: {plain:?}");
    }

    #[test]
    fn clear_water_rewets_dried_paint_and_lifts_some() {
        let mut doc = Document::new(160, 60);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Watercolor)));
        paint(&mut doc, &wet(0.4, 1.0), [30, 60, 160], 30.0, false);
        crate::wet::settle(&mut doc);
        assert!(doc.wet.is_empty());
        let dried = doc.layers[0].pixels.rgba(30 * 160 + 80);
        // Water across the dried wash: wet again, a little lighter, color kept.
        paint(&mut doc, &wet(0.4, 0.0), [255, 0, 0], 30.0, false);
        assert!(doc.wet.at(80, 30) > 0.3, "the paint is wet again");
        let wetted = doc.layers[0].pixels.rgba(30 * 160 + 80);
        assert!(
            wetted[3] < dried[3],
            "water lifts paint: {dried:?} -> {wetted:?}"
        );
        assert!(wetted[0] < 120, "clear water adds no color: {wetted:?}");
        // And the paint runs again as it dries.
        let before = doc.layers[0].pixels.region(160, [0, 0, 160, 60]);
        crate::wet::settle(&mut doc);
        assert_ne!(doc.layers[0].pixels.region(160, [0, 0, 160, 60]), before);
    }

    #[test]
    fn smudging_thick_paint_carries_its_relief() {
        let mut doc = Document::new(160, 100);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut doc, &thick(), [200, 80, 40], 30.0, false);
        let relief = |doc: &Document, y: usize| {
            let r = doc.layers[0]
                .material
                .as_ref()
                .unwrap()
                .relief
                .clone()
                .unwrap();
            (60..100).map(|x| u32::from(r.at(x, y))).sum::<u32>()
        };
        assert_eq!(relief(&doc, 50), 0);
        // A knife dragged down across the stroke pulls the paint's height along.
        let knife = crate::brush::Brush {
            size: 30.0,
            hardness: 0.9,
            opacity: 0.9,
            spacing: 0.05,
            pressure_size: false,
            blend: crate::brush::Blend::Smudge,
            ..Default::default()
        };
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: knife.clone(),
            color: [0, 0, 0],
            secondary: [0, 0, 0],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=20 {
            let sample = crate::stroke::Sample {
                x: 80.0,
                y: 20.0 + i as f32 * 2.0,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 20 {
                path.finish(sample, &knife)
            } else {
                path.sample(sample, &knife)
            };
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
        buffer.finish_tail(&mut doc, &settings);
        assert!(
            relief(&doc, 50) > 200,
            "relief dragged below: {}",
            relief(&doc, 50)
        );
        assert!(
            doc.layers[0].pixels.rgba(50 * 160 + 80)[3] > 0,
            "and the paint came with it"
        );
    }

    #[test]
    fn relief_moves_and_flips_with_the_selected_paint_only() {
        let mut doc = Document::new(200, 80);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut doc, &thick(), [200, 80, 40], 30.0, false);
        let relief = |doc: &Document| {
            doc.layers[0]
                .material
                .as_ref()
                .unwrap()
                .relief
                .clone()
                .unwrap()
        };
        let before = relief(&doc);
        // Select the left part of the stroke and move it down 30 px.
        doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 60,
            bottom: 80,
        });
        crate::move_preview::apply(&mut doc, [0, 30], None).unwrap();
        let after = relief(&doc);
        assert!(
            after.at(40, 60) > 60,
            "the selected paint's height moved down"
        );
        assert_eq!(after.at(40, 30), 0, "and left its old place");
        assert_eq!(after.at(120, 30), before.at(120, 30), "the rest stayed");
        // Flipping a selection turns the relief inside it only.
        let moved = relief(&doc);
        doc.selection = Some(crate::selection::Selection {
            left: 100,
            top: 0,
            right: 200,
            bottom: 80,
        });
        doc.transform_active(crate::transform::Transform::FlipHorizontal)
            .unwrap();
        let flipped = relief(&doc);
        assert_eq!(flipped.at(150, 30), moved.at(149, 30));
        assert_eq!(
            flipped.at(40, 60),
            moved.at(40, 60),
            "outside the selection unchanged"
        );
        doc.validate().unwrap();
    }

    #[test]
    fn reliefs_survive_saving_and_canvas_changes() {
        let mut doc = Document::new(160, 60);
        doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut doc, &thick(), [200, 80, 40], 30.0, false);
        doc.add_layer();
        doc.layers[1].material = Some(Material::new(Settings {
            edges: 0.25,
            ..Settings::new(Kind::Watercolor)
        }));
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let relief = |doc: &Document| {
            doc.layers[0]
                .material
                .as_ref()
                .unwrap()
                .relief
                .clone()
                .unwrap()
        };
        assert!(relief(&loaded).equivalent(&relief(&doc), None).unwrap());
        assert_eq!(
            loaded.layers[1].material.as_ref().unwrap().settings.edges,
            0.25
        );
        assert!(loaded.layers[1].material.as_ref().unwrap().relief.is_none());
        // Rotating the canvas turns the relief with the paint.
        let mut turned = doc.clone();
        turned.rotate_canvas_clockwise();
        let r = relief(&turned);
        assert_eq!(r.dimensions(), [60, 160]);
        assert_eq!(r.at(60 - 1 - 30, 80), relief(&doc).at(80, 30));
        turned.validate().unwrap();
    }
}

#[cfg(test)]
mod look {
    use super::*;

    #[test]
    #[ignore = "prints drying timings for review"]
    fn drying_cost() {
        // 2000 x 1000 px wet at once, in 40 px stripes of two colors: a
        // seam every 40 px is far more mixing than real washes have.
        for kind in [None, Some(Kind::Watercolor)] {
            let (w, h) = (2000, 1000);
            let mut doc = Document::new(w, h);
            doc.layers[0].material = kind.map(|k| Material::new(Settings::new(k)));
            {
                let mut writer = doc.layers[0].pixels.writer();
                for y in 0..h {
                    for x in 0..w {
                        let c = if x / 40 % 2 == 0 {
                            [20, 40, 200]
                        } else {
                            [250, 220, 20]
                        };
                        writer
                            .rgba_mut(y * w + x)
                            .copy_from_slice(&[c[0], c[1], c[2], 200]);
                    }
                }
            }
            let id = doc.layers[0].id;
            let row = vec![1.0; w];
            for y in 0..h {
                doc.wet.add_row(id, y, 0, &row);
            }
            let start = std::time::Instant::now();
            for _ in 0..10 {
                crate::wet::dry_step(&mut doc);
            }
            eprintln!(
                "{kind:?}: {:.1} ms per step",
                start.elapsed().as_secs_f64() * 100.0
            );
        }
    }
    #[test]
    #[ignore = "writes a picture for a person to look at"]
    fn knife_picture() {
        let mut app = crate::Dudel::default();
        app.material_capture_fixture();
        let doc = &app.doc;
        let expanded = crate::layer_effects::expand(doc, false, &|| false).unwrap();
        let rgba = expanded.composite_display_region([700, 100, 1400, 900], false);
        let path = std::env::var("DUDEL_LOOK").unwrap();
        image::save_buffer(path, &rgba, 700, 800, image::ExtendedColorType::Rgba8).unwrap();
    }
}
