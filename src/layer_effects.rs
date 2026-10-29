//! Layer effects ("layer styles"): read from a PSD's effect descriptors and
//! drawn from a layer's alpha. Effects drawn for a layer become extra layers
//! around it, the way Photoshop's Layer > Layer Style > Create Layers does:
//! PSD import can bake them, and native effects are expanded the same way
//! whenever the document is composited (see `expand`).

use crate::blend::BlendMode;
use crate::document::{Document, Layer};
use crate::psd_vector::{Descriptor, Gradient, Value};
use serde::{Deserialize, Serialize};

/// Where an effect's layer goes relative to the layer it belongs to.
/// Indexes the importer's [below, inside, above] lists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// Under the layer: shadows, outer glows, outside strokes.
    Below,
    /// Inside the layer's pixels: overlays, inner effects, inside strokes.
    Inside,
    /// Over the layer, not limited to its pixels: centered strokes.
    Above,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Paint {
    Color([u8; 3]),
    Gradient(Gradient),
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Position {
    Outside,
    Inside,
    Center,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Kind {
    DropShadow {
        offset: [f64; 2],
        spread: f64,
        size: f64,
        knockout: bool,
    },
    OuterGlow {
        spread: f64,
        size: f64,
        precise: bool,
        /// The share of the glow's falloff its contour spans.
        range: f64,
    },
    GradientOverlay,
    ColorOverlay,
    InnerGlow {
        choke: f64,
        size: f64,
        center: bool,
        precise: bool,
        range: f64,
    },
    InnerShadow {
        offset: [f64; 2],
        choke: f64,
        size: f64,
    },
    Stroke {
        size: f64,
        position: Position,
    },
    /// One half of a bevel: its lit (`highlight`) or its shaded side.
    Bevel {
        highlight: bool,
        style: BevelStyle,
        technique: Technique,
        /// Slope steepness (Photoshop's depth, 1.0 = 100%).
        depth: f64,
        size: f64,
        soften: f64,
        /// Unit vector toward the light: x right, y down, z out of the canvas.
        light: [f64; 3],
        /// Raised (true) or pressed in.
        up: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum BevelStyle {
    Inner,
    Outer,
    Emboss,
    Pillow,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Technique {
    Smooth,
    ChiselHard,
    ChiselSoft,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Effect {
    pub kind: Kind,
    pub paint: Paint,
    pub blend: BlendMode,
    pub opacity: f32,
}
/// An effect on a layer, which can be switched off without losing it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LayerEffect {
    pub enabled: bool,
    pub effect: Effect,
}
impl LayerEffect {
    pub(crate) fn new(effect: Effect) -> Self {
        Self {
            enabled: true,
            effect,
        }
    }
    /// Reject settings a damaged project could hold.
    pub(crate) fn validate(&self) -> Result<(), String> {
        let effect = &self.effect;
        let size = |v: f64| v.is_finite() && (0.0..=MAX_SIZE).contains(&v);
        let unit = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        let ok = match effect.kind {
            Kind::DropShadow {
                offset,
                spread,
                size: s,
                ..
            }
            | Kind::InnerShadow {
                offset,
                choke: spread,
                size: s,
            } => {
                offset.iter().all(|v| v.is_finite() && v.abs() <= MAX_SIZE)
                    && unit(spread)
                    && size(s)
            }
            Kind::OuterGlow {
                spread,
                size: s,
                range,
                ..
            }
            | Kind::InnerGlow {
                choke: spread,
                size: s,
                range,
                ..
            } => unit(spread) && size(s) && range.is_finite() && (0.01..=1.0).contains(&range),
            Kind::GradientOverlay | Kind::ColorOverlay => true,
            Kind::Stroke { size: s, .. } => size(s),
            Kind::Bevel {
                depth,
                size: s,
                soften,
                light,
                ..
            } => {
                depth.is_finite()
                    && (0.01..=10.0).contains(&depth)
                    && size(s)
                    && size(soften)
                    && light.iter().all(|v| v.is_finite() && v.abs() <= 1.0)
            }
        };
        if !ok || !(0.0..=1.0).contains(&effect.opacity) {
            return Err("Invalid layer effect settings".into());
        }
        if let Paint::Gradient(gradient) = &effect.paint {
            gradient.validate()?;
        }
        Ok(())
    }
}
/// The light for shadows and bevels from Photoshop-style angle and altitude
/// in degrees.
pub(crate) fn light(angle: f64, altitude: f64) -> [f64; 3] {
    let (angle, altitude) = (angle.to_radians(), altitude.to_radians());
    [
        altitude.cos() * angle.cos(),
        -altitude.cos() * angle.sin(),
        altitude.sin(),
    ]
}
/// Shadow offset for a light from `angle` degrees at `distance` pixels.
pub(crate) fn offset(angle: f64, distance: f64) -> [f64; 2] {
    let (sin, cos) = angle.to_radians().sin_cos();
    [-cos * distance, sin * distance]
}
impl Effect {
    /// A new effect of `kind` with Photoshop's default settings.
    pub(crate) fn preset(kind: &str) -> Option<Self> {
        let black = Paint::Color([0; 3]);
        let (kind, paint, blend, opacity) = match kind {
            "Drop Shadow" => (
                Kind::DropShadow {
                    offset: offset(120.0, 5.0),
                    spread: 0.0,
                    size: 5.0,
                    knockout: true,
                },
                black,
                BlendMode::Multiply,
                0.75,
            ),
            "Inner Shadow" => (
                Kind::InnerShadow {
                    offset: offset(120.0, 5.0),
                    choke: 0.0,
                    size: 5.0,
                },
                black,
                BlendMode::Multiply,
                0.75,
            ),
            "Outer Glow" => (
                Kind::OuterGlow {
                    spread: 0.0,
                    size: 5.0,
                    precise: false,
                    range: 0.5,
                },
                Paint::Color([255, 255, 190]),
                BlendMode::Screen,
                0.75,
            ),
            "Inner Glow" => (
                Kind::InnerGlow {
                    choke: 0.0,
                    size: 5.0,
                    center: false,
                    precise: false,
                    range: 0.5,
                },
                Paint::Color([255, 255, 190]),
                BlendMode::Screen,
                0.75,
            ),
            "Color Overlay" => (
                Kind::ColorOverlay,
                Paint::Color([255, 0, 0]),
                BlendMode::Normal,
                1.0,
            ),
            "Gradient Overlay" => (
                Kind::GradientOverlay,
                Paint::Gradient(Gradient::between([0; 3], [255; 3], 90.0)),
                BlendMode::Normal,
                1.0,
            ),
            "Stroke" => (
                Kind::Stroke {
                    size: 3.0,
                    position: Position::Outside,
                },
                Paint::Color([255, 0, 0]),
                BlendMode::Normal,
                1.0,
            ),
            _ => return None,
        };
        Some(Self {
            kind,
            paint,
            blend,
            opacity,
        })
    }
    /// The two halves of a default inner bevel.
    pub(crate) fn bevel_presets() -> [Self; 2] {
        [false, true].map(|highlight| Self {
            kind: Kind::Bevel {
                highlight,
                style: BevelStyle::Inner,
                technique: Technique::Smooth,
                depth: 1.0,
                size: 5.0,
                soften: 0.0,
                light: light(120.0, 30.0),
                up: true,
            },
            paint: Paint::Color(if highlight { [255; 3] } else { [0; 3] }),
            blend: if highlight {
                BlendMode::Screen
            } else {
                BlendMode::Multiply
            },
            opacity: 0.75,
        })
    }
}
/// Names offered when adding an effect.
pub(crate) const PRESETS: [&str; 8] = [
    "Drop Shadow",
    "Outer Glow",
    "Stroke",
    "Color Overlay",
    "Gradient Overlay",
    "Inner Shadow",
    "Inner Glow",
    "Bevel and Emboss",
];
/// A layer's effects, bottom to top, and those that could not be drawn.
#[derive(Debug, Default)]
pub(crate) struct Effects {
    pub list: Vec<Effect>,
    pub skipped: Vec<String>,
    /// Drawn, but without a setting Dudel cannot reproduce yet.
    pub approximated: Vec<String>,
}

impl Effect {
    pub(crate) fn name(&self) -> &'static str {
        match self.kind {
            Kind::DropShadow { .. } => "Drop Shadow",
            Kind::OuterGlow { .. } => "Outer Glow",
            Kind::GradientOverlay => "Gradient Overlay",
            Kind::ColorOverlay => "Color Overlay",
            Kind::InnerGlow { .. } => "Inner Glow",
            Kind::InnerShadow { .. } => "Inner Shadow",
            Kind::Stroke { .. } => "Stroke",
            Kind::Bevel {
                highlight: true, ..
            } => "Bevel Highlights",
            Kind::Bevel { .. } => "Bevel Shadows",
        }
    }
    pub(crate) fn place(&self) -> Place {
        match self.kind {
            Kind::DropShadow { .. } | Kind::OuterGlow { .. } => Place::Below,
            Kind::Stroke {
                position: Position::Outside,
                ..
            } => Place::Below,
            Kind::Stroke {
                position: Position::Center,
                ..
            } => Place::Above,
            Kind::Bevel { style, .. } => match style {
                BevelStyle::Inner => Place::Inside,
                BevelStyle::Outer => Place::Below,
                BevelStyle::Emboss | BevelStyle::Pillow => Place::Above,
            },
            _ => Place::Inside,
        }
    }
    /// Photoshop's stacking order, bottom to top.
    fn rank(&self) -> u8 {
        match self.kind {
            Kind::DropShadow { .. } => 0,
            Kind::OuterGlow { .. } => 1,
            Kind::GradientOverlay => 2,
            Kind::ColorOverlay => 3,
            Kind::InnerGlow { .. } => 4,
            Kind::InnerShadow { .. } => 5,
            Kind::Stroke { .. } => 6,
            Kind::Bevel { .. } => 7,
        }
    }
    /// How far beyond the layer's pixels the effect can reach.
    pub(crate) fn reach(&self) -> usize {
        let reach = match self.kind {
            Kind::DropShadow { offset, size, .. } => {
                2.0 * size + offset[0].abs().max(offset[1].abs())
            }
            Kind::OuterGlow { size, .. } => 2.0 * size,
            Kind::Stroke { size, .. } => size,
            Kind::Bevel { size, soften, .. } => size + soften,
            _ => 0.0,
        };
        reach.ceil() as usize + 2
    }
}

/// Draw effects as layers (Photoshop's Create Layers): those under `owner`,
/// those clipped to it and those over its clip group. They follow `source`
/// when given (a shape's fill), else `owner`'s alpha, taken from `content`
/// when given (a folder's composite, one value per canvas pixel), else from
/// its pixels; `owner`'s mask applies either way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    effects: &[Effect],
    owner: &Layer,
    opacity: f32,
    fill: u8,
    content: Option<&[f32]>,
    source: Option<&[f32]>,
    [width, height]: [usize; 2],
    budget: &mut usize,
    cancel: &impl Fn() -> bool,
) -> Result<[Vec<Layer>; 3], String> {
    let mut placed: [Vec<Layer>; 3] = Default::default();
    let union = |a: [usize; 4], b: [usize; 4]| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    };
    let covered = |plane: &[f32]| {
        plane
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0.0)
            .map(|(i, _)| [i % width, i / width, i % width + 1, i / width + 1])
            .reduce(union)
    };
    let bounds = match (source, content) {
        (Some(plane), _) | (None, Some(plane)) => covered(plane),
        (None, None) => owner
            .pixels
            .nonzero_tiles(width, height)
            .into_iter()
            .reduce(union),
    };
    let Some(bounds) = bounds else {
        return Ok(placed);
    };
    let reach = effects.iter().map(|e| e.reach()).max().unwrap_or(0);
    let rect = [
        bounds[0].saturating_sub(reach),
        bounds[1].saturating_sub(reach),
        (bounds[2] + reach).min(width),
        (bounds[3] + reach).min(height),
    ];
    let w = rect[2] - rect[0];
    let at = |i: usize| (rect[1] + i / w) * width + rect[0] + i % w;
    let mask = |i: usize| owner.mask_coverage(at(i), width);
    let content: Vec<f32> = match content {
        Some(plane) => (0..w * (rect[3] - rect[1]))
            .map(|i| plane[at(i)] * mask(i))
            .collect(),
        None => owner
            .pixels
            .region(width, rect)
            .chunks_exact(4)
            .enumerate()
            .map(|(i, p)| f32::from(p[3]) / 255.0 * mask(i))
            .collect(),
    };
    let visible: Vec<f32> = content
        .iter()
        .map(|a| a * f32::from(fill) / 255.0)
        .collect();
    let values: Vec<f32> = match source {
        Some(source) => (0..content.len())
            .map(|i| source[at(i)] * mask(i))
            .collect(),
        None => content.clone(),
    };
    // Gradients span the layer's visible pixels.
    let mut span = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (i, _) in values.iter().enumerate().filter(|(_, a)| **a > 0.0) {
        let (x, y) = ((rect[0] + i % w) as f64, (rect[1] + i / w) as f64);
        span = [
            span[0].min(x),
            span[1].min(y),
            span[2].max(x + 1.0),
            span[3].max(y + 1.0),
        ];
    }
    if span[0] > span[2] {
        return Ok(placed);
    }
    let mut alpha = Alpha::new(rect, values);
    // Inner effects clip to the layer, unless that would apply its fill
    // opacity, the layer is clipped itself (clipping does not nest), or
    // its effects follow a shape rather than its pixels; then they carry
    // that alpha.
    let clip_inside = !owner.clipped && fill == 255 && source.is_none();
    for effect in effects {
        if cancel() {
            return Err("Canceled".into());
        }
        *budget = budget
            .checked_add(w * (rect[3] - rect[1]))
            .filter(|b| *b <= crate::document::MAX_PAYLOAD)
            .ok_or("Layer effects exceed the memory budget")?;
        let mut coverage = coverage(effect, &mut alpha, cancel)?;
        let mut place = effect.place();
        if place == Place::Below {
            effect.under(&mut coverage, &alpha.values, &visible);
        }
        let mut out = Layer::raster(format!("{}'s {}", owner.name, effect.name()), width, height);
        out.visible = owner.visible;
        out.blend = effect.blend;
        out.clipped = owner.clipped;
        out.opacity = effect.opacity * opacity;
        if place == Place::Inside {
            for (c, a) in coverage.iter_mut().zip(&alpha.values) {
                *c *= if clip_inside {
                    f32::from(u8::from(*a > 0.0))
                } else {
                    *a
                };
            }
            if clip_inside {
                out.clipped = true;
                out.opacity = effect.opacity;
            } else if !owner.clipped {
                place = Place::Above;
            }
        }
        let pixels = paint(effect, rect, &coverage, span, [width, height]);
        out.pixels.writer().write_region(width, rect, &pixels);
        placed[place as usize].push(out);
    }
    Ok(placed)
}

/// Effect layers drawn for one layer, kept until the layer or its effects
/// change. Clones share the cache, which holds a few layers by identity.
#[derive(Clone, Default)]
pub(crate) struct Cache(std::sync::Arc<std::sync::Mutex<Vec<Cached>>>);
struct Cached {
    id: crate::document::LayerId,
    pixels: crate::pixels::Pixels,
    mask: Option<crate::layer_mask::LayerMask>,
    effects: Vec<Effect>,
    look: (String, f32, bool, bool),
    size: [usize; 2],
    layers: [Vec<Layer>; 3],
}
const CACHED_LAYERS: usize = 4;
impl Cache {
    /// The effect layers of `layer`, drawn now unless cached. `stale`
    /// accepts a cached drawing that no longer matches (while painting).
    fn layers(
        &self,
        layer: &Layer,
        effects: &[Effect],
        size: [usize; 2],
        stale: bool,
        cancel: &dyn Fn() -> bool,
    ) -> Result<([Vec<Layer>; 3], bool), String> {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let look = (
            layer.name.clone(),
            layer.opacity,
            layer.visible,
            layer.clipped,
        );
        let fresh = |c: &Cached| {
            c.pixels.shares(&layer.pixels)
                && crate::layer_mask::same_snapshot(&c.mask, &layer.mask)
                && c.effects == effects
                && c.look == look
                && c.size == size
        };
        if let Some(cached) = entries
            .iter()
            .find(|c| c.id == layer.id && (fresh(c) || (stale && c.size == size)))
        {
            return Ok((cached.layers.clone(), false));
        }
        let mut budget = 0;
        let layers = draw(
            effects,
            layer,
            layer.opacity,
            255,
            None,
            None,
            size,
            &mut budget,
            &cancel,
        )?;
        entries.retain(|c| c.id != layer.id);
        if entries.len() >= CACHED_LAYERS {
            entries.remove(0);
        }
        entries.push(Cached {
            id: layer.id,
            pixels: layer.pixels.clone(),
            mask: layer.mask.clone(),
            effects: effects.to_vec(),
            look,
            size,
            layers: layers.clone(),
        });
        Ok((layers, true))
    }
}
/// The document with each layer's enabled effects drawn as layers around
/// it, as Photoshop's Create Layers would: what the compositor draws.
/// Effects on folders are not drawn. `stale` reuses drawings made before
/// the layer last changed, for a fast preview while painting.
pub(crate) fn expand(
    doc: &Document,
    stale: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Document, String> {
    expand_tracked(doc, stale, cancel).map(|(doc, _)| doc)
}
/// `expand`, also telling whether any effect was drawn anew (rather than
/// taken from the cache): then it may have changed anywhere it reaches.
pub(crate) fn expand_tracked(
    doc: &Document,
    stale: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(Document, bool), String> {
    let mut drew = false;
    let mut out = doc.clone();
    out.layers = Vec::with_capacity(doc.layers.len() * 2);
    // Effects drawn over a layer wait until the layers clipped to it end.
    let mut pending: Vec<Layer> = Vec::new();
    let mut pending_depth = 0;
    for (index, layer) in doc.layers.iter().enumerate() {
        if !(layer.clipped && layer.depth == pending_depth) {
            out.layers.append(&mut pending);
        }
        let effects: Vec<Effect> = layer
            .effects
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.effect.clone())
            .collect();
        let [below, inside, above] = if layer.folder || effects.is_empty() {
            Default::default()
        } else {
            let (layers, fresh) = layer.effect_cache.layers(
                layer,
                &effects,
                [doc.width, doc.height],
                stale,
                cancel,
            )?;
            drew |= fresh;
            layers
        };
        let depth = layer.depth;
        let at_depth = move |mut l: Layer| {
            l.depth = depth;
            l
        };
        out.layers.extend(below.into_iter().map(at_depth));
        if index == doc.active {
            out.active = out.layers.len();
        }
        let mut owner = layer.clone();
        owner.effects.clear();
        out.layers.push(owner);
        out.layers.extend(inside.into_iter().map(at_depth));
        if !above.is_empty() {
            pending.extend(above.into_iter().map(at_depth));
            pending_depth = depth;
        }
    }
    out.layers.append(&mut pending);
    Ok((out, drew))
}

/// Settings beyond these are rejected as damaged.
const MAX_SIZE: f64 = 1000.0;

/// `lfx2`/`lfxs`/`lmfx`: a version word, then a versioned descriptor.
/// `global_angle` is the document's light angle in degrees.
pub(crate) fn parse(data: &[u8], global_angle: f64) -> Result<Effects, String> {
    let settings = crate::psd_vector::versioned(data.get(4..).ok_or("Truncated PSD effects")?)?;
    let mut effects = Effects::default();
    if settings.flag("masterFXSwitch") == Some(false) {
        return Ok(effects);
    }
    // "Scl " is not applied: it records a Scale Effects factor already
    // in the stored sizes (a 208% file draws its 1 px strokes at 1 px).
    for (key, value) in &settings.items {
        let list: Vec<&Descriptor> = match value {
            Value::Object(effect) => vec![effect],
            // Several of one kind: the first is drawn on top.
            Value::List(items) if key.ends_with("Multi") => items
                .iter()
                .rev()
                .filter_map(|v| match v {
                    Value::Object(effect) => Some(effect),
                    _ => None,
                })
                .collect(),
            _ => continue,
        };
        for effect in list {
            if effect.flag("enab") != Some(true) {
                continue;
            }
            if effect.class == "ebbl" {
                match bevel(effect, global_angle, &mut effects.approximated) {
                    Ok(halves) => effects.list.extend(halves),
                    Err(skipped) => effects.skipped.push(skipped),
                }
                continue;
            }
            match read(effect, global_angle, &mut effects.approximated) {
                Ok(Some(effect)) => effects.list.push(effect),
                Ok(None) => {}
                Err(skipped) => effects.skipped.push(skipped),
            }
        }
    }
    effects.list.sort_by_key(Effect::rank);
    Ok(effects)
}

/// One enabled effect. `Ok(None)` for keys that are not effects; `Err`
/// names an effect that cannot be drawn.
fn read(
    effect: &Descriptor,
    global_angle: f64,
    approximated: &mut Vec<String>,
) -> Result<Option<Effect>, String> {
    let pixels = |key: &str| {
        effect
            .number(key)
            .filter(|v| v.is_finite() && (0.0..=MAX_SIZE).contains(v))
    };
    let percent = |key: &str| effect.number(key).map(|v| (v / 100.0).clamp(0.0, 1.0));
    let paint = || -> Result<Paint, String> {
        effect
            .object("Clr ")
            .map(crate::psd_vector::color)
            .transpose()
            .map_err(|e| e.to_string())?
            .map(Paint::Color)
            .ok_or_else(|| "no color".to_string())
    };
    let offset = || {
        let angle = if effect.flag("uglg") == Some(true) {
            global_angle
        } else {
            effect.number("lagl").unwrap_or(global_angle)
        };
        let distance = pixels("Dstn").unwrap_or(0.0);
        let (sin, cos) = angle.to_radians().sin_cos();
        [-cos * distance, sin * distance]
    };
    let name = match effect.class.as_str() {
        "DrSh" => "drop shadow",
        "IrSh" => "inner shadow",
        "OrGl" => "outer glow",
        "IrGl" => "inner glow",
        "SoFi" => "color overlay",
        "GrFl" => "gradient overlay",
        "FrFX" => "stroke",
        "ChFX" => return Err("satin".into()),
        "patternFill" => return Err("pattern overlay".into()),
        _ => return Ok(None),
    };
    let blend = blend(effect.enumerated("Md  ").unwrap_or("Nrml")).ok_or(format!(
        "{name} with blend mode {}",
        effect.enumerated("Md  ").unwrap_or_default()
    ))?;
    let opacity = percent("Opct").unwrap_or(1.0) as f32;
    if matches!(
        name,
        "drop shadow" | "inner shadow" | "outer glow" | "inner glow"
    ) {
        if !linear(effect.object("TrnS")) {
            approximated.push(format!("{name} contour"));
        }
        if percent("Nose").unwrap_or(0.0) > 0.0 {
            approximated.push(format!("{name} noise"));
        }
    }
    let size = pixels("blur").unwrap_or(0.0);
    // Spread and choke are percentages despite their pixel unit.
    let spread = percent("Ckmt").unwrap_or(0.0);
    let kind = match name {
        "drop shadow" => Kind::DropShadow {
            offset: offset(),
            spread,
            size,
            knockout: effect.flag("layerConceals").unwrap_or(true),
        },
        "inner shadow" => Kind::InnerShadow {
            offset: offset(),
            choke: spread,
            size,
        },
        "outer glow" => Kind::OuterGlow {
            spread,
            size,
            precise: effect.enumerated("GlwT") == Some("PrBL"),
            range: percent("Inpr").unwrap_or(0.5).max(0.01),
        },
        "inner glow" => Kind::InnerGlow {
            choke: spread,
            size,
            center: effect.enumerated("glwS") == Some("SrcC"),
            precise: effect.enumerated("GlwT") == Some("PrBL"),
            range: percent("Inpr").unwrap_or(0.5).max(0.01),
        },
        "color overlay" => Kind::ColorOverlay,
        "gradient overlay" => Kind::GradientOverlay,
        _ => Kind::Stroke {
            size: pixels("Sz  ").ok_or("stroke without a size")?,
            position: match effect.enumerated("Styl") {
                Some("InsF") => Position::Inside,
                Some("CtrF") => Position::Center,
                _ => Position::Outside,
            },
        },
    };
    let paint = match (name, effect.enumerated("PntT")) {
        ("gradient overlay", _) | ("stroke", Some("GrFl")) => Paint::Gradient(
            crate::psd_vector::gradient(effect).map_err(|e| format!("{name}: {e}"))?,
        ),
        ("stroke", Some("Ptrn")) => return Err("pattern stroke".into()),
        _ => paint().map_err(|e| format!("{name}: {e}"))?,
    };
    Ok(Some(Effect {
        kind,
        paint,
        blend,
        opacity,
    }))
}
/// Bevel and emboss: a highlight and a shadow, each with its own color,
/// blend mode and opacity.
fn bevel(
    effect: &Descriptor,
    global_angle: f64,
    approximated: &mut Vec<String>,
) -> Result<Vec<Effect>, String> {
    let style = match effect.enumerated("bvlS") {
        Some("InrB") | None => BevelStyle::Inner,
        Some("OtrB") => BevelStyle::Outer,
        Some("Embs") => BevelStyle::Emboss,
        Some("PlEb") => BevelStyle::Pillow,
        Some(_) => return Err("stroke emboss".into()),
    };
    let technique = match effect.enumerated("bvlT") {
        Some("PrBL") => Technique::ChiselHard,
        Some("Slmt") => Technique::ChiselSoft,
        _ => Technique::Smooth,
    };
    let pixels = |key: &str| {
        effect
            .number(key)
            .filter(|v| v.is_finite() && (0.0..=MAX_SIZE).contains(v))
            .unwrap_or(0.0)
    };
    let angle = if effect.flag("uglg") == Some(true) {
        global_angle
    } else {
        effect.number("lagl").unwrap_or(global_angle)
    }
    .to_radians();
    let altitude = effect
        .number("Lald")
        .unwrap_or(30.0)
        .clamp(0.0, 90.0)
        .to_radians();
    let light = [
        altitude.cos() * angle.cos(),
        -altitude.cos() * angle.sin(),
        altitude.sin(),
    ];
    if !linear(effect.object("TrnS")) {
        approximated.push("bevel gloss contour".into());
    }
    if effect.flag("useShape") == Some(true) {
        approximated.push("bevel contour".into());
    }
    if effect.flag("useTexture") == Some(true) {
        approximated.push("bevel texture".into());
    }
    let half = |highlight: bool| -> Result<Effect, String> {
        let [mode, color, opacity] = if highlight {
            ["hglM", "hglC", "hglO"]
        } else {
            ["sdwM", "sdwC", "sdwO"]
        };
        let mode = effect.enumerated(mode).unwrap_or("Nrml");
        Ok(Effect {
            kind: Kind::Bevel {
                highlight,
                style,
                technique,
                depth: effect
                    .number("srgR")
                    .map_or(1.0, |v| (v / 100.0).clamp(0.01, 10.0)),
                size: pixels("blur"),
                soften: pixels("Sftn"),
                light,
                up: effect.enumerated("bvlD") != Some("Out "),
            },
            paint: Paint::Color(match effect.object(color) {
                Some(color) => crate::psd_vector::color(color)?,
                None if highlight => [255; 3],
                None => [0; 3],
            }),
            blend: blend(mode).ok_or(format!("bevel with blend mode {mode}"))?,
            opacity: effect
                .number(opacity)
                .map_or(0.75, |v| (v / 100.0).clamp(0.0, 1.0)) as f32,
        })
    };
    // Shadow first: Photoshop draws the highlight over it.
    Ok(vec![half(false)?, half(true)?])
}
/// A contour that leaves the effect's falloff unchanged.
fn linear(contour: Option<&Descriptor>) -> bool {
    let Some(contour) = contour else {
        return true;
    };
    match contour.get("Crv ") {
        Some(Value::List(points)) => {
            let points: Vec<_> = points
                .iter()
                .filter_map(|p| match p {
                    Value::Object(p) => Some((p.number("Hrzn")?, p.number("Vrtc")?)),
                    _ => None,
                })
                .collect();
            points.len() == points.iter().filter(|(x, y)| x == y).count()
        }
        _ => true,
    }
}
/// Effect blend modes are named by Photoshop's descriptor IDs.
fn blend(name: &str) -> Option<BlendMode> {
    use BlendMode::*;
    Some(match name.trim_end() {
        "Nrml" | "normal" => Normal,
        "Dslv" | "dissolve" => Dissolve,
        "Drkn" | "darken" => Darken,
        "Mltp" | "multiply" => Multiply,
        "CBrn" | "colorBurn" => ColorBurn,
        "linearBurn" => LinearBurn,
        "darkerColor" => DarkerColor,
        "Lghn" | "lighten" => Lighten,
        "Scrn" | "screen" => Screen,
        "CDdg" | "colorDodge" => ColorDodge,
        "linearDodge" => LinearDodge,
        "lighterColor" => LighterColor,
        "Ovrl" | "overlay" => Overlay,
        "SftL" | "softLight" => SoftLight,
        "HrdL" | "hardLight" => HardLight,
        "vividLight" => VividLight,
        "linearLight" => LinearLight,
        "pinLight" => PinLight,
        "hardMix" => HardMix,
        "Dfrn" | "difference" => Difference,
        "Xclu" | "exclusion" => Exclusion,
        "blendSubtraction" => Subtract,
        "blendDivide" => Divide,
        "H" | "hue" => Hue,
        "Strt" | "saturation" => Saturation,
        "Clr" | "color" => Color,
        "Lmns" | "luminosity" => Luminosity,
        _ => return None,
    })
}

/// A layer's alpha over a canvas region, with its signed distance field
/// computed on first use.
pub(crate) struct Alpha {
    pub rect: [usize; 4],
    pub values: Vec<f32>,
    /// The field and which sides of the edge it measures.
    distance: Option<(Vec<f32>, [bool; 2])>,
    /// A bevel's lit surface (shared by its highlight and shadow halves),
    /// keyed by the bevel with `highlight` cleared.
    lit: Option<(Kind, Vec<f32>)>,
}
impl Alpha {
    pub(crate) fn new(rect: [usize; 4], values: Vec<f32>) -> Self {
        assert_eq!(values.len(), (rect[2] - rect[0]) * (rect[3] - rect[1]));
        Self {
            rect,
            values,
            distance: None,
            lit: None,
        }
    }
    fn width(&self) -> usize {
        self.rect[2] - self.rect[0]
    }
    fn height(&self) -> usize {
        self.rect[3] - self.rect[1]
    }
    /// Signed distance from each pixel center to the layer's edge: positive
    /// outside, negative inside. Outside the region counts as transparent.
    ///
    /// Like Photoshop, any coverage counts as part of the layer and a partly
    /// covered pixel holds the edge at depth `alpha` from its outer side; a
    /// pixel without coverage (or with full coverage) measures to the
    /// nearest pixel that has some (or lacks some), then to the edge in it.
    ///
    /// Only the sides asked for (`outside`, `inside`) are measured; pixels on
    /// the other side read as far away.
    fn distance(
        &mut self,
        cancel: &impl Fn() -> bool,
        outside: bool,
        inside: bool,
    ) -> Result<&[f32], String> {
        let known = self
            .distance
            .as_ref()
            .is_some_and(|(_, [o, i])| (*o || !outside) && (*i || !inside));
        if !known {
            let (w, h) = (self.width(), self.height());
            let a = &self.values;
            const AWAY: f32 = 1e6;
            let to_layer = if outside {
                nearest(
                    a,
                    w,
                    h,
                    false,
                    |v| v > 0.0,
                    |d, q| d - 0.5 + (1.0 - q),
                    cancel,
                )?
            } else {
                Vec::new()
            };
            let to_gap = if inside {
                nearest(a, w, h, true, |v| v < FULL, |d, q| 0.5 - q - d, cancel)?
            } else {
                Vec::new()
            };
            let field = (0..w * h)
                .map(|i| {
                    if a[i] <= 0.0 {
                        to_layer.get(i).copied().unwrap_or(AWAY)
                    } else if a[i] < FULL {
                        0.5 - a[i]
                    } else {
                        to_gap.get(i).copied().unwrap_or(-AWAY)
                    }
                })
                .collect();
            self.distance = Some((field, [outside, inside]));
        }
        Ok(&self.distance.as_ref().unwrap().0)
    }
}

/// Coverage treated as complete.
const FULL: f32 = 0.999;
const FAR: f64 = 1e12;

/// For each pixel, `edge(distance, alpha)` of the nearest pixel whose alpha
/// satisfies `target`: its Euclidean distance and its alpha. A one-pixel
/// transparent border surrounds the region (a target when `border`).
/// Felzenszwalb and Huttenlocher's linear-time transform: a column pass, then
/// a row pass, each over contiguous lines in parallel.
#[allow(clippy::too_many_arguments)]
fn nearest(
    alpha: &[f32],
    w: usize,
    h: usize,
    border: bool,
    target: impl Fn(f32) -> bool + Sync,
    edge: impl Fn(f32, f32) -> f32 + Sync,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<f32>, String> {
    let (pw, ph) = (w + 2, h + 2);
    let at = |x: usize, y: usize| {
        ((1..=w).contains(&x) && (1..=h).contains(&y)).then(|| alpha[(y - 1) * w + x - 1])
    };
    // Column pass, one column per line: squared vertical distance to the
    // nearest target in the column, and that target's alpha.
    let mut columns = vec![[0f32; 2]; pw * ph];
    bands(&mut columns, ph, |first, band| {
        let (mut f, mut out, mut owner) = (vec![0.0; ph], Vec::new(), Vec::new());
        for (k, column) in band.chunks_mut(ph).enumerate() {
            let x = first + k;
            for (y, value) in f.iter_mut().enumerate() {
                *value = if at(x, y).map_or(border, &target) {
                    0.0
                } else {
                    FAR
                };
            }
            transform(&f, &mut out, &mut owner);
            for (y, cell) in column.iter_mut().enumerate() {
                *cell = [out[y] as f32, at(x, owner[y]).unwrap_or(0.0)];
            }
        }
    });
    check_cancel(cancel)?;
    let rows = transpose(&columns, ph, pw);
    drop(columns);
    check_cancel(cancel)?;
    let mut result = vec![0f32; w * h];
    bands(&mut result, w, |first, band| {
        let (mut f, mut out, mut owner) = (vec![0.0; pw], Vec::new(), Vec::new());
        for (k, line) in band.chunks_mut(w).enumerate() {
            let row = &rows[(first + k + 1) * pw..(first + k + 2) * pw];
            for (value, cell) in f.iter_mut().zip(row) {
                *value = f64::from(cell[0]);
            }
            transform(&f, &mut out, &mut owner);
            for (x, value) in line.iter_mut().enumerate() {
                *value = edge(out[x + 1].sqrt() as f32, row[owner[x + 1]][1]);
            }
        }
    });
    check_cancel(cancel)?;
    Ok(result)
}
fn check_cancel(cancel: &impl Fn() -> bool) -> Result<(), String> {
    if cancel() {
        Err("Canceled".into())
    } else {
        Ok(())
    }
}
/// Run `job(first_row, rows)` over bands of rows of `data` (rows of `row`
/// values) on the shared worker pool.
fn bands<T: Send>(data: &mut [T], row: usize, job: impl Fn(usize, &mut [T]) + Sync) {
    const ROWS: usize = 32;
    let chunks: Vec<std::sync::Mutex<&mut [T]>> = data
        .chunks_mut(ROWS * row.max(1))
        .map(std::sync::Mutex::new)
        .collect();
    let results = crate::parallel::run(chunks.len(), |i| {
        let mut band = chunks[i].lock().unwrap_or_else(|e| e.into_inner());
        job(i * ROWS, &mut band);
    });
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
/// `src` holds `h` rows of `w`; the result holds `w` rows of `h`.
fn transpose<T: Copy + Default + Send + Sync>(src: &[T], w: usize, h: usize) -> Vec<T> {
    let mut out = vec![T::default(); w * h];
    bands(&mut out, h, |first, band| {
        let lines = band.len() / h.max(1);
        // Read a short run of each source row, write down the band.
        for y in 0..h {
            let run = &src[y * w + first..y * w + first + lines];
            for (k, value) in run.iter().enumerate() {
                band[k * h + y] = *value;
            }
        }
    });
    out
}
/// Squared distance transform of one line (lower envelope of parabolas),
/// with the index of the nearest target for each position.
fn transform(f: &[f64], out: &mut Vec<f64>, owner: &mut Vec<usize>) {
    let n = f.len();
    out.clear();
    out.resize(n, 0.0);
    owner.clear();
    owner.resize(n, 0);
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let meet = |q: usize, p: usize| {
        ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q - p) as f64)
    };
    for q in 1..n {
        let mut s = meet(q, v[k]);
        // z[0] is -inf, so this stops at the first parabola.
        while s <= z[k] {
            k -= 1;
            s = meet(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        out[q] = (q as f64 - p as f64).powi(2) + f[p];
        owner[q] = p;
    }
}

/// Blur standing in for Photoshop's effect blur of `size` pixels: three box
/// blurs approximating a Gaussian. Edges repeat the border value. Box blurs
/// are separable, so all three run along rows, then along columns.
fn blur(
    values: &mut [f32],
    w: usize,
    h: usize,
    size: f64,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    let sigma = size * BLUR_SIGMA;
    if sigma < 0.3 || w == 0 || h == 0 {
        return Ok(());
    }
    // Box widths whose combined variance matches sigma (Kuckir's method).
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut low = ideal.floor() as i64;
    if low % 2 == 0 {
        low -= 1;
    }
    let low = low.max(1);
    let m = ((12.0 * sigma * sigma - (3 * low * low) as f64 - (12 * low) as f64 - 9.0)
        / (-4 * low - 4) as f64)
        .round() as i64;
    let radii: Vec<usize> = (0..3)
        .map(|pass| ((if pass < m { low } else { low + 2 }) - 1) as usize / 2)
        .filter(|r| *r > 0)
        .collect();
    if radii.is_empty() {
        return Ok(());
    }
    let along_rows = |data: &mut [f32], row: usize| {
        bands(data, row, |_, band| {
            let mut line = vec![0.0; row];
            for values in band.chunks_mut(row) {
                for &radius in &radii {
                    line.copy_from_slice(values);
                    box_line(&line, values, radius);
                }
            }
        });
    };
    along_rows(values, w);
    check_cancel(cancel)?;
    let mut columns = transpose(values, w, h);
    along_rows(&mut columns, h);
    check_cancel(cancel)?;
    values.copy_from_slice(&transpose(&columns, h, w));
    Ok(())
}
/// Sigma of the Gaussian matching an effect size of one pixel, fitted to
/// Photoshop's saved images of shadows and glows (ag-psd effects test).
const BLUR_SIGMA: f64 = 0.4;
fn box_line(src: &[f32], dst: &mut [f32], radius: usize) {
    let n = src.len();
    let at = |i: isize| src[i.clamp(0, n as isize - 1) as usize];
    let r = radius as isize;
    let scale = 1.0 / (2 * radius + 1) as f32;
    let mut sum: f32 = (-r..=r).map(at).sum();
    for (i, out) in dst.iter_mut().enumerate() {
        *out = sum * scale;
        let i = i as isize;
        sum += at(i + r + 1) - at(i - r);
    }
}
/// Move a plane by whole pixels, filling with `fill`.
fn shift(values: &[f32], w: usize, h: usize, offset: [f64; 2], fill: f32) -> Vec<f32> {
    let [dx, dy] = offset.map(|v| v.round() as isize);
    let mut out = vec![fill; w * h];
    for y in 0..h as isize {
        let sy = y - dy;
        if !(0..h as isize).contains(&sy) {
            continue;
        }
        for x in 0..w as isize {
            let sx = x - dx;
            if (0..w as isize).contains(&sx) {
                out[(y * w as isize + x) as usize] = values[(sy * w as isize + sx) as usize];
            }
        }
    }
    out
}
/// A glow's contour spans `range` of its falloff, so the falloff reaches
/// full strength at `range` (50% doubles a soft glow, capped at full).
fn ranged(mut glow: Vec<f32>, range: f64) -> Vec<f32> {
    for g in &mut glow {
        *g = (*g / range as f32).min(1.0);
    }
    glow
}
/// How brightly a light along `light` falls on a bevel's surface: the cosine
/// to its normal, so a flat surface gets `light[2]`.
#[allow(clippy::too_many_arguments)]
fn lit_surface(
    alpha: &mut Alpha,
    style: BevelStyle,
    technique: Technique,
    depth: f64,
    size: f64,
    light: [f64; 3],
    up: bool,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<f32>, String> {
    let (w, h) = (alpha.width(), alpha.height());
    let size = size.max(1.0);
    let s = size as f32;
    let sides = match style {
        BevelStyle::Inner => (false, true),
        BevelStyle::Outer => (true, false),
        BevelStyle::Emboss | BevelStyle::Pillow => (true, true),
    };
    // Height over the slope, 0 at its foot and 1 at its top.
    let mut height: Vec<f32> = alpha
        .distance(cancel, sides.0, sides.1)?
        .iter()
        .map(|&d| {
            let t = match style {
                BevelStyle::Inner => -d / s,
                BevelStyle::Outer => 1.0 - d / s,
                BevelStyle::Emboss => 0.5 - d / s,
                BevelStyle::Pillow => 2.0 * d.abs() / s,
            }
            .clamp(0.0, 1.0);
            if up { t } else { 1.0 - t }
        })
        .collect();
    let round = match technique {
        Technique::Smooth => 0.5,
        Technique::ChiselSoft => 0.15,
        Technique::ChiselHard => 0.0,
    };
    blur(&mut height, w, h, size * round, cancel)?;
    let k = (size * depth) as f32;
    let at = |x: usize, y: usize| height[y * w + x];
    let mut lit = vec![0.0; w * h];
    bands(&mut lit, w, |first, band| {
        for (k_row, row) in band.chunks_mut(w).enumerate() {
            let y = first + k_row;
            for (x, value) in row.iter_mut().enumerate() {
                let dx = (at((x + 1).min(w - 1), y) - at(x.saturating_sub(1), y)) / 2.0;
                let dy = (at(x, (y + 1).min(h - 1)) - at(x, y.saturating_sub(1))) / 2.0;
                let n = [-dx * k, -dy * k];
                let length = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
                *value =
                    (n[0] * light[0] as f32 + n[1] * light[1] as f32 + light[2] as f32) / length;
            }
        }
    });
    Ok(lit)
}
/// Coverage of the band of signed distances [lo, hi] over a pixel.
fn band(distance: f32, lo: f32, hi: f32) -> f32 {
    ((distance + 0.5).min(hi) - (distance - 0.5).max(lo)).clamp(0.0, 1.0)
}

/// An effect's coverage over `alpha`'s region, 0..=1 per pixel. Inside
/// effects are not limited to the layer's alpha here.
pub(crate) fn coverage(
    effect: &Effect,
    alpha: &mut Alpha,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<f32>, String> {
    let (w, h) = (alpha.width(), alpha.height());
    // Grow the shape (its outside, for inner effects) by `grow` pixels, then
    // blur by the rest of `size`.
    let soft =
        |alpha: &mut Alpha, grow: f64, size: f64, invert: bool| -> Result<Vec<f32>, String> {
            let flip = |a: f32| if invert { 1.0 - a } else { a };
            // Only spread or choke needs distances.
            let mut shape: Vec<f32> = if grow > 0.0 {
                let values = alpha.values.clone();
                values
                    .iter()
                    .zip(alpha.distance(cancel, !invert, invert)?)
                    .map(|(&a, &d)| {
                        let d = if invert { -d } else { d };
                        flip(a).max(band(d, f32::NEG_INFINITY, grow as f32))
                    })
                    .collect()
            } else {
                alpha.values.iter().map(|&a| flip(a)).collect()
            };
            blur(&mut shape, w, h, size - grow, cancel)?;
            Ok(shape)
        };
    Ok(match effect.kind {
        Kind::DropShadow {
            offset,
            spread,
            size,
            ..
        } => {
            let shadow = soft(alpha, size * spread, size, false)?;
            shift(&shadow, w, h, offset, 0.0)
        }
        Kind::OuterGlow {
            spread,
            size,
            precise: false,
            range,
        } => ranged(soft(alpha, size * spread, size, false)?, range),
        Kind::OuterGlow {
            spread,
            size,
            precise: true,
            ..
        } => {
            let grow = (size * spread) as f32;
            let fade = (size as f32 - grow).max(1.0);
            alpha
                .distance(cancel, true, false)?
                .iter()
                .map(|&d| (1.0 - (d - grow) / fade).clamp(0.0, 1.0))
                .collect()
        }
        Kind::InnerShadow {
            offset,
            choke,
            size,
        } => {
            let shadow = soft(alpha, size * choke, size, true)?;
            shift(&shadow, w, h, offset, 1.0)
        }
        Kind::InnerGlow {
            choke,
            size,
            center,
            precise,
            range,
        } => {
            let glow = if precise {
                let grow = (size * choke) as f32;
                let fade = (size as f32 - grow).max(1.0);
                alpha
                    .distance(cancel, false, true)?
                    .iter()
                    .map(|&d| (1.0 - (-d - grow) / fade).clamp(0.0, 1.0))
                    .collect()
            } else {
                ranged(soft(alpha, size * choke, size, true)?, range)
            };
            if center {
                glow.into_iter().map(|g| 1.0 - g).collect()
            } else {
                glow
            }
        }
        Kind::ColorOverlay | Kind::GradientOverlay => vec![1.0; w * h],
        Kind::Bevel {
            highlight,
            style,
            technique,
            depth,
            size,
            soften,
            light,
            up,
        } => {
            let surface = Kind::Bevel {
                highlight: false,
                style,
                technique,
                depth,
                size,
                soften,
                light,
                up,
            };
            if alpha.lit.as_ref().is_none_or(|(kind, _)| *kind != surface) {
                let lit = lit_surface(alpha, style, technique, depth, size, light, up, cancel)?;
                alpha.lit = Some((surface, lit));
            }
            let flat = light[2] as f32;
            let lit = &alpha.lit.as_ref().unwrap().1;
            let mut shade: Vec<f32> = lit
                .iter()
                .map(|&lit| {
                    if highlight {
                        ((lit - flat) / (1.0 - flat).max(1e-3)).max(0.0)
                    } else {
                        ((flat - lit) / flat.max(1e-3)).max(0.0)
                    }
                    .min(1.0)
                })
                .collect();
            blur(&mut shade, w, h, soften, cancel)?;
            shade
        }
        Kind::Stroke { size, position } => {
            let size = size as f32;
            // Outside strokes lie under the layer and fill beneath its
            // partly transparent pixels; inside strokes are limited to it.
            let (lo, hi) = match position {
                Position::Outside => (f32::NEG_INFINITY, size),
                Position::Inside => (-size, f32::INFINITY),
                Position::Center => (-size / 2.0, size / 2.0),
            };
            alpha
                .distance(
                    cancel,
                    position != Position::Inside,
                    position != Position::Outside,
                )?
                .iter()
                .map(|&d| band(d, lo, hi))
                .collect()
        }
    })
}

impl Effect {
    /// Adjust the coverage of an effect drawn under its layer. Photoshop
    /// adds such effects to the layer rather than layering them, so where
    /// the layer partly covers them they show only in what it leaves
    /// uncovered. `source` is the alpha the effect follows and `visible`
    /// how much the layer actually covers (less with fill opacity).
    pub(crate) fn under(&self, coverage: &mut [f32], source: &[f32], visible: &[f32]) {
        let gap = |v: f32| if v < 1.0 { 1.0 / (1.0 - v) } else { 0.0 };
        match self.kind {
            // The stroke also covers the inside, where the layer is drawn.
            Kind::Stroke { .. } => {
                for ((c, s), v) in coverage.iter_mut().zip(source).zip(visible) {
                    *c = ((*c - s) * gap(*v)).clamp(0.0, 1.0);
                }
            }
            Kind::DropShadow { knockout: true, .. } => {
                for ((c, s), v) in coverage.iter_mut().zip(source).zip(visible) {
                    *c = (*c * (1.0 - s) * gap(*v)).clamp(0.0, 1.0);
                }
            }
            _ => {}
        }
    }
}
/// Straight RGBA for an effect: its paint with `coverage` as alpha.
/// `bounds` is the box a gradient spans (the layer's pixels), in canvas pixels.
pub(crate) fn paint(
    effect: &Effect,
    rect: [usize; 4],
    coverage: &[f32],
    bounds: [f64; 4],
    canvas: [usize; 2],
) -> Vec<u8> {
    let w = rect[2] - rect[0];
    let mut pixels = vec![0; coverage.len() * 4];
    let gradient = match &effect.paint {
        Paint::Gradient(g) => Some(g.painter(bounds, canvas)),
        Paint::Color(_) => None,
    };
    for (i, (&c, out)) in coverage.iter().zip(pixels.chunks_exact_mut(4)).enumerate() {
        if c <= 0.0 {
            continue;
        }
        let [r, g, b, a] = match (&effect.paint, &gradient) {
            (_, Some(painter)) => painter(rect[0] + i % w, rect[1] + i / w),
            (Paint::Color(color), _) => [color[0], color[1], color[2], 255],
            _ => unreachable!(),
        };
        let alpha = (c.min(1.0) * f32::from(a)).round() as u8;
        if alpha > 0 {
            out.copy_from_slice(&[r, g, b, alpha]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: usize, inner: [usize; 4]) -> Alpha {
        let mut values = vec![0.0; size * size];
        for y in inner[1]..inner[3] {
            for x in inner[0]..inner[2] {
                values[y * size + x] = 1.0;
            }
        }
        Alpha::new([0, 0, size, size], values)
    }
    fn effect(kind: Kind) -> Effect {
        Effect {
            kind,
            paint: Paint::Color([255, 0, 0]),
            blend: BlendMode::Normal,
            opacity: 1.0,
        }
    }

    #[test]
    fn distance_field_measures_from_the_edge_on_both_sides() {
        let mut alpha = square(20, [5, 5, 15, 15]);
        let d = alpha.distance(&|| false, true, true).unwrap().to_vec();
        assert_eq!(d[10 * 20 + 4], 0.5);
        assert_eq!(d[10 * 20 + 2], 2.5);
        assert_eq!(d[10 * 20 + 5], -0.5);
        assert_eq!(d[10 * 20 + 7], -2.5);
        // Partial coverage moves the edge into the pixel.
        let mut soft = square(20, [5, 5, 15, 15]);
        soft.values[10 * 20 + 4] = 0.25;
        let d = soft.distance(&|| false, true, true).unwrap();
        assert!((d[10 * 20 + 4] - 0.25).abs() < 1e-6);
        // A layer filling its region still has an edge at the border.
        let mut full = square(4, [0, 0, 4, 4]);
        assert_eq!(full.distance(&|| false, true, true).unwrap()[0], -0.5);
    }

    #[test]
    fn strokes_cover_their_band_around_the_edge() {
        let stroke = |position| {
            let mut alpha = square(20, [5, 5, 15, 15]);
            coverage(
                &effect(Kind::Stroke {
                    size: 2.0,
                    position,
                }),
                &mut alpha,
                &|| false,
            )
            .unwrap()
        };
        let row = |c: &[f32]| (0..20).map(|x| c[10 * 20 + x]).collect::<Vec<_>>();
        let outside = row(&stroke(Position::Outside));
        // Outside strokes go under the layer and cover its inside too.
        assert_eq!(&outside[1..8], &[0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        // Inside strokes are clipped to the layer, so only the inner edge counts.
        let inside = row(&stroke(Position::Inside));
        assert_eq!(&inside[5..9], &[1.0, 1.0, 0.0, 0.0]);
        let center = row(&stroke(Position::Center));
        assert_eq!(&center[2..8], &[0.0, 0.0, 1.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn shadows_move_away_from_the_light_and_knock_out_the_layer() {
        let mut alpha = square(30, [10, 10, 20, 20]);
        let shadow = coverage(
            &effect(Kind::DropShadow {
                offset: [3.0, 4.0],
                spread: 0.0,
                size: 0.0,
                knockout: false,
            }),
            &mut alpha,
            &|| false,
        )
        .unwrap();
        assert_eq!(shadow[14 * 30 + 13], 1.0);
        assert_eq!(shadow[13 * 30 + 12], 0.0);
        assert_eq!(shadow[23 * 30 + 22], 1.0);
        let soft = effect(Kind::DropShadow {
            offset: [3.0, 4.0],
            spread: 0.0,
            size: 6.0,
            knockout: true,
        });
        let mut knocked = coverage(&soft, &mut alpha, &|| false).unwrap();
        let edge = knocked[19 * 30 + 23];
        assert!(edge > 0.3 && edge < 0.6, "{edge}");
        // Under an opaque layer the shadow is hidden; under a partly
        // covering one it adds up with the layer (here the layer is fully
        // drawn, so nothing changes), and under a layer whose fill opacity
        // hides it the shadow stays knocked out.
        let mut source = alpha.values.clone();
        source[19 * 30 + 23] = 0.5;
        let mut visible = source.clone();
        let before = knocked[15 * 30 + 15];
        assert!(before > 0.5);
        soft.under(&mut knocked, &source, &visible);
        assert_eq!(knocked[15 * 30 + 15], 0.0);
        assert!((knocked[19 * 30 + 23] - edge).abs() < 1e-6);
        visible[19 * 30 + 23] = 0.0;
        let mut hidden = vec![edge; source.len()];
        soft.under(&mut hidden, &source, &visible);
        assert!((hidden[19 * 30 + 23] - edge * 0.5).abs() < 1e-6);
    }

    #[test]
    fn outside_strokes_fill_what_the_layer_leaves_uncovered() {
        let stroke = effect(Kind::Stroke {
            size: 2.0,
            position: Position::Outside,
        });
        // Inside a raster layer drawn at alpha 0.4 the stroke fills the
        // other 0.6; outside it covers fully.
        let mut c = vec![1.0, 1.0];
        stroke.under(&mut c, &[0.4, 0.0], &[0.4, 0.0]);
        assert_eq!(c, vec![1.0, 1.0]);
        // A shape's fill covers its inside even where its paint is
        // transparent, so the stroke does not show there.
        let mut c = vec![1.0];
        stroke.under(&mut c, &[1.0], &[0.3]);
        assert_eq!(c, vec![0.0]);
    }

    #[test]
    fn bevels_light_the_slope_facing_the_light_and_leave_flat_parts_alone() {
        let bevel = |highlight, style| {
            let mut alpha = square(40, [10, 10, 30, 30]);
            coverage(
                &effect(Kind::Bevel {
                    highlight,
                    style,
                    technique: Technique::ChiselHard,
                    depth: 1.0,
                    size: 4.0,
                    soften: 0.0,
                    // From straight above the top edge, 30 degrees up.
                    light: [0.0, -(30f64.to_radians().cos()), 30f64.to_radians().sin()],
                    up: true,
                }),
                &mut alpha,
                &|| false,
            )
            .unwrap()
        };
        let (light, dark) = (
            bevel(true, BevelStyle::Inner),
            bevel(false, BevelStyle::Inner),
        );
        let at = |plane: &[f32], x: usize, y: usize| plane[y * 40 + x];
        // Top slope lit, bottom slope shaded, the plateau and outside untouched.
        assert!(at(&light, 20, 11) > 0.5 && at(&dark, 20, 11) == 0.0);
        assert!(at(&dark, 20, 28) > 0.5 && at(&light, 20, 28) == 0.0);
        assert_eq!((at(&light, 20, 20), at(&dark, 20, 20)), (0.0, 0.0));
        assert_eq!((at(&light, 20, 5), at(&dark, 20, 5)), (0.0, 0.0));
        // An outer bevel slopes outside the layer instead.
        let outer = bevel(true, BevelStyle::Outer);
        assert!(at(&outer, 20, 8) > 0.5 && at(&outer, 20, 12) == 0.0);
    }

    /// A 4 x 1 document with an opaque red pixel at x = 1.
    fn doc_with(effects: Vec<LayerEffect>) -> Document {
        let mut doc = Document::new(4, 1);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[255, 0, 0, 255]);
        doc.layers[0].effects = effects;
        doc
    }
    fn overlay() -> LayerEffect {
        let mut effect = Effect::preset("Color Overlay").unwrap();
        effect.paint = Paint::Color([0, 0, 255]);
        LayerEffect::new(effect)
    }

    #[test]
    fn compositing_draws_enabled_effects_and_caches_them() {
        let stroke = |position| {
            let mut effect = Effect::preset("Stroke").unwrap();
            effect.kind = Kind::Stroke {
                size: 1.0,
                position,
            };
            effect.paint = Paint::Color([0, 255, 0]);
            LayerEffect::new(effect)
        };
        let mut doc = doc_with(vec![overlay(), stroke(Position::Outside)]);
        assert!(doc.has_effects());
        let pixels = doc.composite_transparent();
        // Overlay recolors the layer; the outside stroke rings it.
        assert_eq!(&pixels[4..8], &[0, 0, 255, 255]);
        assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
        assert_eq!(&pixels[8..12], &[0, 255, 0, 255]);
        assert_eq!(&pixels[12..16], &[0, 0, 0, 0]);
        // Switched off, an effect is kept but not drawn.
        doc.layers[0].effects[1].enabled = false;
        let pixels = doc.composite_transparent();
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&pixels[4..8], &[0, 0, 255, 255]);
        // Expanding again reuses the drawing until the layer changes.
        let first = expand(&doc, false, &|| false).unwrap();
        let again = expand(&doc, false, &|| false).unwrap();
        assert!(first.layers[1].pixels.shares(&again.layers[1].pixels));
        assert_eq!(first.layers[0].name, doc.layers[0].name);
        assert!(first.layers.iter().all(|l| l.effects.is_empty()));
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[255, 0, 0, 255]);
        let changed = expand(&doc, false, &|| false).unwrap();
        assert!(!changed.layers[1].pixels.shares(&first.layers[1].pixels));
        assert_eq!(changed.layers[1].pixels.rgba(2), [0, 0, 255, 255]);
        // A stale preview keeps the old drawing.
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(3)
            .copy_from_slice(&[255, 0, 0, 255]);
        let stale = expand(&doc, true, &|| false).unwrap();
        assert!(stale.layers[1].pixels.shares(&changed.layers[1].pixels));
    }

    #[test]
    fn effects_survive_saving_and_damaged_settings_are_rejected() {
        let mut shadow = Effect::preset("Drop Shadow").unwrap();
        shadow.blend = BlendMode::Screen;
        let mut bevel = Effect::bevel_presets().map(LayerEffect::new).to_vec();
        bevel[0].enabled = false;
        let mut effects = vec![LayerEffect::new(shadow), overlay()];
        effects.append(&mut bevel);
        effects.push(LayerEffect::new(
            Effect::preset("Gradient Overlay").unwrap(),
        ));
        let doc = doc_with(effects);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        let loaded =
            crate::tile_project::decode(&bytes[crate::tile_project::MAGIC.len()..]).unwrap();
        assert_eq!(loaded.layers[0].effects, doc.layers[0].effects);
        assert_eq!(loaded.composite_transparent(), doc.composite_transparent());
        // Out-of-range settings from a damaged file fail validation.
        let mut bad = doc.clone();
        if let Kind::DropShadow { size, .. } = &mut bad.layers[0].effects[0].effect.kind {
            *size = f64::NAN;
        }
        assert!(bad.validate().is_err());
        let mut folder = doc.clone();
        folder.layers[0].folder = true;
        assert!(folder.validate_metadata().is_err());
        let mut many = doc;
        many.layers[0].effects = vec![overlay(); crate::document::MAX_EFFECTS + 1];
        assert!(many.validate().is_err());
    }

    #[test]
    fn merging_down_bakes_effects_once() {
        let mut doc = doc_with(vec![overlay()]);
        doc.add_layer();
        doc.layers[1].effects = vec![overlay()];
        doc.merge_down().unwrap();
        assert!(doc.layers[0].effects.is_empty());
        assert_eq!(doc.layers[0].pixels.rgba(1), [0, 0, 255, 255]);
        assert_eq!(&doc.composite_transparent()[4..8], &[0, 0, 255, 255]);
    }

    #[test]
    fn effect_edits_undo_and_reject_folders() {
        use crate::document_edits::{Edit, Property, Target};
        let mut app = crate::Dudel {
            doc: doc_with(Vec::new()),
            ..Default::default()
        };
        let target = app.doc.layers[0].id;
        assert!(app.set_canvas_property(target, Property::Effects(vec![overlay()])));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].effects, vec![overlay()]);
        assert!(app.history.can_undo());
        app.undo();
        app.flush_render_queue();
        assert!(app.doc.layers[0].effects.is_empty());
        let edit = Edit::Property {
            target: Target::Identity(target),
            property: Property::Effects(vec![overlay(); crate::document::MAX_EFFECTS + 1]),
        };
        assert!(edit.validate_arguments().is_err());
    }

    #[test]
    fn glows_reach_full_strength_at_their_range() {
        assert_eq!(
            ranged(vec![0.1, 0.25, 0.5, 0.9], 0.5),
            vec![0.2, 0.5, 1.0, 1.0]
        );
    }

    #[test]
    fn blur_preserves_mass_and_spreads_symmetrically() {
        let mut values = vec![0.0; 41 * 41];
        values[20 * 41 + 20] = 1.0;
        blur(&mut values, 41, 41, 8.0, &|| false).unwrap();
        let total: f32 = values.iter().sum();
        assert!((total - 1.0).abs() < 1e-4, "{total}");
        assert!((values[20 * 41 + 16] - values[20 * 41 + 24]).abs() < 1e-6);
        assert!((values[16 * 41 + 20] - values[20 * 41 + 16]).abs() < 1e-6);
        // Variance close to the requested sigma squared.
        let variance: f32 = (0..41)
            .map(|x| {
                values[20 * 41 + x] / values[20 * 41..21 * 41].iter().sum::<f32>()
                    * (x as f32 - 20.0).powi(2)
            })
            .sum();
        let sigma = (8.0 * BLUR_SIGMA) as f32;
        assert!((variance.sqrt() - sigma).abs() < 0.6, "{}", variance.sqrt());
    }

    #[test]
    fn inner_shadow_enters_from_the_far_side_of_the_light() {
        let mut alpha = square(30, [10, 10, 20, 20]);
        let shadow = coverage(
            &effect(Kind::InnerShadow {
                offset: [3.0, 0.0],
                choke: 0.0,
                size: 0.0,
            }),
            &mut alpha,
            &|| false,
        )
        .unwrap();
        // Shifted right by 3: the left inner edge is shadowed.
        assert_eq!(
            &(10..15).map(|x| shadow[15 * 30 + x]).collect::<Vec<_>>(),
            &[1.0, 1.0, 1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn effect_descriptors_parse_with_order_and_skips() {
        use crate::psd_vector::{Descriptor, Value};
        let unit = |unit: &str, v: f64| Value::Unit(unit.into(), v);
        let object = |class: &str, items: Vec<(&str, Value)>| {
            Value::Object(Descriptor {
                class: class.into(),
                items: items.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            })
        };
        let red = object(
            "RGBC",
            vec![
                ("Rd  ", Value::Number(255.0)),
                ("Grn ", Value::Number(0.0)),
                ("Bl  ", Value::Number(0.0)),
            ],
        );
        let settings = Descriptor {
            class: "null".into(),
            items: vec![
                ("Scl ".into(), unit("#Prc", 200.0)),
                (
                    "FrFX".into(),
                    object(
                        "FrFX",
                        vec![
                            ("enab", Value::Bool(true)),
                            ("Styl", Value::Enum("FStl".into(), "InsF".into())),
                            ("PntT", Value::Enum("FrFl".into(), "SClr".into())),
                            ("Md  ", Value::Enum("BlnM".into(), "Nrml".into())),
                            ("Opct", unit("#Prc", 50.0)),
                            ("Sz  ", unit("#Pxl", 3.0)),
                            ("Clr ", red.clone()),
                        ],
                    ),
                ),
                (
                    "DrSh".into(),
                    object(
                        "DrSh",
                        vec![
                            ("enab", Value::Bool(true)),
                            ("Md  ", Value::Enum("BlnM".into(), "Mltp".into())),
                            ("Clr ", red.clone()),
                            ("uglg", Value::Bool(true)),
                            ("Dstn", unit("#Pxl", 5.0)),
                            ("blur", unit("#Pxl", 4.0)),
                        ],
                    ),
                ),
                (
                    "ChFX".into(),
                    object("ChFX", vec![("enab", Value::Bool(true))]),
                ),
                (
                    "SoFi".into(),
                    object("SoFi", vec![("enab", Value::Bool(false)), ("Clr ", red)]),
                ),
            ],
        };
        let mut data = vec![0, 0, 0, 0, 0, 0, 0, 16];
        data.extend(crate::psd_vector::encode(&settings));
        let effects = parse(&data, 90.0).unwrap();
        assert_eq!(effects.skipped, vec!["satin".to_string()]);
        assert_eq!(effects.list.len(), 2);
        assert_eq!(effects.list[0].name(), "Drop Shadow");
        assert_eq!(effects.list[0].blend, BlendMode::Multiply);
        let Kind::DropShadow { offset, size, .. } = effects.list[0].kind else {
            panic!()
        };
        // Global light from 90 degrees casts the shadow straight down; the
        // 200% effect scale is already in the stored sizes.
        assert!(offset[0].abs() < 1e-9 && (offset[1] - 5.0).abs() < 1e-9);
        assert_eq!(size, 4.0);
        assert_eq!(effects.list[1].opacity, 0.5);
        assert_eq!(
            effects.list[1].kind,
            Kind::Stroke {
                size: 3.0,
                position: Position::Inside
            }
        );
        assert_eq!(effects.list[1].place(), Place::Inside);
    }
}
