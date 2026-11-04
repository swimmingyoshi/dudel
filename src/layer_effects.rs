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
    Pattern(PatternPaint),
}
/// A pattern repeated from the canvas origin moved by `phase`, at `scale`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PatternPaint {
    pub pattern: crate::pattern::Pattern,
    pub scale: f64,
    pub phase: [f64; 2],
}
impl PatternPaint {
    fn new(pattern: crate::pattern::Pattern) -> Self {
        Self {
            pattern,
            scale: 1.0,
            phase: [0.0; 2],
        }
    }
    /// The color at canvas pixel (`x`, `y`).
    pub(crate) fn at(&self, x: usize, y: usize) -> [u8; 4] {
        self.pattern.sample(
            (x as f64 + 0.5 - self.phase[0]) / self.scale,
            (y as f64 + 0.5 - self.phase[1]) / self.scale,
        )
    }
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
    PatternOverlay,
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
    /// Satin: shading inside the layer where two copies of its shape, moved
    /// apart along `offset` and blurred by `size`, differ.
    Satin {
        /// Where one copy moves; the other moves the opposite way.
        offset: [f64; 2],
        size: f64,
        /// Shade where the copies agree instead (Photoshop's default).
        invert: bool,
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
        /// Shared by the two halves of one bevel (unique within a layer);
        /// 0 in projects saved before pairing, which pair by adjacency.
        #[serde(default)]
        pair: u32,
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
            Kind::GradientOverlay | Kind::ColorOverlay | Kind::PatternOverlay => true,
            Kind::Stroke { size: s, .. } => size(s),
            Kind::Satin {
                offset, size: s, ..
            } => offset.iter().all(|v| v.is_finite() && v.abs() <= MAX_SIZE) && size(s),
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
        match &effect.paint {
            Paint::Gradient(gradient) => gradient.validate()?,
            Paint::Pattern(paint) => {
                paint.pattern.validate()?;
                if !(0.01..=10.0).contains(&paint.scale)
                    || paint.phase.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
                {
                    return Err("Invalid layer effect settings".into());
                }
            }
            Paint::Color(_) => {}
        }
        if matches!(effect.kind, Kind::PatternOverlay) != matches!(effect.paint, Paint::Pattern(_))
            && !matches!(effect.kind, Kind::Stroke { .. })
        {
            return Err("Invalid layer effect settings".into());
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
            "Pattern Overlay" => (
                Kind::PatternOverlay,
                Paint::Pattern(PatternPaint::new(crate::pattern::Pattern::default_dots())),
                BlendMode::Normal,
                1.0,
            ),
            "Satin" => (
                Kind::Satin {
                    offset: offset(19.0, 11.0),
                    size: 14.0,
                    invert: true,
                },
                black,
                BlendMode::Multiply,
                0.5,
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
    pub(crate) fn bevel_presets(pair: u32) -> [Self; 2] {
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
                pair,
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
pub(crate) const PRESETS: [&str; 10] = [
    "Drop Shadow",
    "Outer Glow",
    "Stroke",
    "Color Overlay",
    "Gradient Overlay",
    "Pattern Overlay",
    "Inner Shadow",
    "Inner Glow",
    "Satin",
    "Bevel and Emboss",
];
/// A layer's effects, bottom to top, and those that could not be drawn.
#[derive(Debug, Default)]
pub(crate) struct Effects {
    pub list: Vec<Effect>,
    pub skipped: Vec<String>,
    /// Drawn, but without a setting Dudel cannot reproduce yet.
    pub approximated: Vec<String>,
    /// Switched off in the file (or by its master switch): not drawn, but
    /// kept on layers whose effects stay editable.
    pub disabled: Vec<Effect>,
}

impl Effects {
    /// The effects as a layer keeps them, bottom to top: switched-off ones
    /// included when they fit and are valid, else only those drawn. `None`
    /// when even those cannot stay editable.
    pub(crate) fn editable(&self) -> Option<Vec<LayerEffect>> {
        let usable = |effects: &[LayerEffect]| {
            effects.len() <= crate::document::MAX_EFFECTS
                && effects.iter().all(|e| e.validate().is_ok())
        };
        let mut all: Vec<LayerEffect> = self
            .list
            .iter()
            .cloned()
            .map(LayerEffect::new)
            .chain(self.disabled.iter().cloned().map(|effect| LayerEffect {
                enabled: false,
                ..LayerEffect::new(effect)
            }))
            .collect();
        all.sort_by_key(|e| e.effect.rank());
        if usable(&all) {
            return Some(all);
        }
        all.retain(|e| e.enabled);
        usable(&all).then_some(all)
    }
}

impl Effect {
    pub(crate) fn name(&self) -> &'static str {
        match self.kind {
            Kind::DropShadow { .. } => "Drop Shadow",
            Kind::OuterGlow { .. } => "Outer Glow",
            Kind::GradientOverlay => "Gradient Overlay",
            Kind::ColorOverlay => "Color Overlay",
            Kind::PatternOverlay => "Pattern Overlay",
            Kind::InnerGlow { .. } => "Inner Glow",
            Kind::InnerShadow { .. } => "Inner Shadow",
            Kind::Stroke { .. } => "Stroke",
            Kind::Satin { .. } => "Satin",
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
    pub(crate) fn rank(&self) -> u8 {
        match self.kind {
            Kind::DropShadow { .. } => 0,
            Kind::OuterGlow { .. } => 1,
            Kind::PatternOverlay => 2,
            Kind::GradientOverlay => 3,
            Kind::ColorOverlay => 4,
            Kind::Satin { .. } => 5,
            Kind::InnerGlow { .. } => 6,
            Kind::InnerShadow { .. } => 7,
            Kind::Stroke { .. } => 8,
            Kind::Bevel { .. } => 9,
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
    /// How far a change to the layer's pixels can change this effect's
    /// drawing: its reach, and inside the layer the distances and blurs it
    /// measures (a blur of `size` spreads about 1.2 times that).
    fn influence(&self) -> usize {
        let influence = match self.kind {
            Kind::DropShadow { offset, size, .. }
            | Kind::InnerShadow { offset, size, .. }
            | Kind::Satin { offset, size, .. } => 3.0 * size + offset[0].abs().max(offset[1].abs()),
            Kind::OuterGlow { size, .. } | Kind::InnerGlow { size, .. } => 3.0 * size,
            Kind::Stroke { size, .. } => size,
            Kind::Bevel { size, soften, .. } => 3.0 * size + 2.0 * soften,
            Kind::ColorOverlay | Kind::GradientOverlay | Kind::PatternOverlay => 0.0,
        };
        (influence.ceil() as usize + 4).max(self.reach())
    }
}

/// Whether two effects draw the same pixels: blending and opacity are only
/// applied when compositing.
fn same_drawing(a: &Effect, b: &Effect) -> bool {
    a.kind == b.kind && a.paint == b.paint
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
    size: [usize; 2],
    budget: &mut usize,
    cancel: &impl Fn() -> bool,
) -> Result<[Vec<Layer>; 3], String> {
    let mut placed: [Vec<Layer>; 3] = Default::default();
    let parts = draw_parts(
        effects, owner, fill, content, source, size, false, budget, cancel,
    )?;
    for (effect, part) in effects.iter().zip(parts) {
        if let Some(mut layer) = part.layer {
            dress(&mut layer, effect, owner, opacity, part.clip);
            placed[part.place as usize].push(layer);
        }
    }
    Ok(placed)
}

/// One effect's drawing, before compositing settings are applied.
#[derive(Clone)]
struct Part {
    effect: Effect,
    place: Place,
    /// Clipped to the layer's own pixels.
    clip: bool,
    /// None when the effect draws nothing.
    layer: Option<Layer>,
}

/// Name, visibility, blending and opacity of an effect's layer.
fn dress(out: &mut Layer, effect: &Effect, owner: &Layer, opacity: f32, clip: bool) {
    out.name = format!("{}'s {}", owner.name, effect.name());
    out.visible = owner.visible;
    out.blend = effect.blend;
    out.clipped = clip || owner.clipped;
    out.opacity = if clip {
        effect.opacity
    } else {
        effect.opacity * opacity
    };
}

/// The region effects reaching `reach` pixels around `owner`'s pixels (or
/// around `plane`'s coverage) can draw in, if any.
fn extent(
    owner: &Layer,
    plane: Option<&[f32]>,
    [width, height]: [usize; 2],
    reach: usize,
) -> Option<[usize; 4]> {
    let bounds = match plane {
        Some(plane) => plane
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0.0)
            .map(|(i, _)| [i % width, i / width, i % width + 1, i / width + 1])
            .reduce(union),
        None => owner
            .pixels
            .nonzero_tiles(width, height)
            .into_iter()
            .reduce(union),
    }?;
    Some([
        bounds[0].saturating_sub(reach),
        bounds[1].saturating_sub(reach),
        (bounds[2] + reach).min(width),
        (bounds[3] + reach).min(height),
    ])
}
fn union(a: [usize; 4], b: [usize; 4]) -> [usize; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}
fn area(rect: [usize; 4]) -> usize {
    (rect[2] - rect[0]) * (rect[3] - rect[1])
}

/// Each effect drawn whole, in order.
#[allow(clippy::too_many_arguments)]
fn draw_parts(
    effects: &[Effect],
    owner: &Layer,
    fill: u8,
    content: Option<&[f32]>,
    source: Option<&[f32]>,
    size: [usize; 2],
    local_inside: bool,
    budget: &mut usize,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<Part>, String> {
    let empty = || {
        effects
            .iter()
            .map(|effect| Part {
                effect: effect.clone(),
                place: effect.place(),
                clip: false,
                layer: None,
            })
            .collect()
    };
    let reach = effects.iter().map(|e| e.reach()).max().unwrap_or(0);
    let Some(rect) = extent(owner, source.or(content), size, reach) else {
        return Ok(empty());
    };
    let drawn = draw_rect(
        effects,
        owner,
        fill,
        content,
        source,
        size,
        rect,
        None,
        local_inside,
        budget,
        cancel,
    )?;
    let Some(drawn) = drawn else {
        return Ok(empty());
    };
    Ok(effects
        .iter()
        .zip(drawn)
        .map(|(effect, (place, clip, pixels))| {
            let mut layer = Layer::raster(String::new(), size[0], size[1]);
            layer.pixels.writer().write_region(size[0], rect, &pixels);
            Part {
                effect: effect.clone(),
                place,
                clip,
                layer: Some(layer),
            }
        })
        .collect())
}

/// An effect's RGBA over a region, where it goes and whether it is
/// clipped to the layer.
type Drawn = (Place, bool, Vec<u8>);
/// Each effect's RGBA over `rect`, where it goes and whether it is clipped
/// to the layer. `span` is the box gradients span; without it, it is
/// measured, and nothing is drawn for a layer without coverage.
#[allow(clippy::too_many_arguments)]
fn draw_rect(
    effects: &[Effect],
    owner: &Layer,
    fill: u8,
    content: Option<&[f32]>,
    source: Option<&[f32]>,
    [width, height]: [usize; 2],
    rect: [usize; 4],
    span: Option<[f64; 4]>,
    local_inside: bool,
    budget: &mut usize,
    cancel: &impl Fn() -> bool,
) -> Result<Option<Vec<Drawn>>, String> {
    let w = rect[2] - rect[0];
    let at = |i: usize| (rect[1] + i / w) * width + rect[0] + i % w;
    let mask = |i: usize| owner.mask_coverage(at(i), width);
    let content: Vec<f32> = match content {
        Some(plane) => (0..area(rect)).map(|i| plane[at(i)] * mask(i)).collect(),
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
    let span = match span {
        Some(span) => span,
        None => {
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
                return Ok(None);
            }
            span
        }
    };
    let mut alpha = Alpha::new(rect, values);
    // Inner effects clip to the layer, unless that would apply its fill
    // opacity, the layer is clipped itself (clipping does not nest), or
    // its effects follow a shape rather than its pixels; then they carry
    // that alpha.
    let clip_inside = !owner.clipped && fill == 255 && source.is_none();
    let mut drawn = Vec::with_capacity(effects.len());
    for effect in effects {
        if cancel() {
            return Err("Canceled".into());
        }
        *budget = budget
            .checked_add(area(rect) * 4)
            .filter(|b| *b <= crate::document::MAX_PAYLOAD)
            .ok_or("Layer effects exceed the memory budget")?;
        let mut coverage = coverage(effect, &mut alpha, cancel)?;
        let mut place = effect.place();
        if place == Place::Below {
            effect.under(&mut coverage, &alpha.values, &visible);
        }
        let mut clip = false;
        if place == Place::Inside {
            for (c, a) in coverage.iter_mut().zip(&alpha.values) {
                *c *= if clip_inside || local_inside {
                    f32::from(u8::from(*a > 0.0))
                } else {
                    *a
                };
            }
            if clip_inside {
                clip = true;
            } else if !owner.clipped && !local_inside {
                place = Place::Above;
            }
        }
        let pixels = paint(effect, rect, &coverage, span, [width, height]);
        drawn.push((place, clip, pixels));
    }
    Ok(Some(drawn))
}

/// The rows of `pixels` (RGBA over `from`) inside `to`.
fn crop(pixels: &[u8], from: [usize; 4], to: [usize; 4]) -> Vec<u8> {
    let w = from[2] - from[0];
    let row = (to[2] - to[0]) * 4;
    let mut out = Vec::with_capacity(row * (to[3] - to[1]));
    for y in to[1]..to[3] {
        let start = ((y - from[1]) * w + to[0] - from[0]) * 4;
        out.extend_from_slice(&pixels[start..start + row]);
    }
    out
}

/// Where effects reaching `influence` pixels must be redrawn after the
/// pixels in `changed` did: pairs of the region to update and the larger
/// window to measure it in. Overlapping windows merge when that costs no
/// more than measuring both.
fn redraw_windows(
    changed: &[[usize; 4]],
    influence: usize,
    [width, height]: [usize; 2],
) -> Vec<([usize; 4], [usize; 4])> {
    let grow = |r: [usize; 4], by: usize| {
        [
            r[0].saturating_sub(by),
            r[1].saturating_sub(by),
            (r[2] + by).min(width),
            (r[3] + by).min(height),
        ]
    };
    let overlap =
        |a: [usize; 4], b: [usize; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    let mut dirty: Vec<[usize; 4]> = changed.to_vec();
    loop {
        let mut merged = false;
        'pairs: for i in 0..dirty.len() {
            for j in i + 1..dirty.len() {
                let (a, b) = (grow(dirty[i], 2 * influence), grow(dirty[j], 2 * influence));
                if overlap(a, b) && area(union(a, b)) <= area(a) + area(b) {
                    dirty[i] = union(dirty[i], dirty[j]);
                    dirty.swap_remove(j);
                    merged = true;
                    break 'pairs;
                }
            }
        }
        if !merged {
            break;
        }
    }
    dirty
        .into_iter()
        .map(|d| (grow(d, influence), grow(d, 2 * influence)))
        .collect()
}

/// Effect layers drawn for one layer, kept until the layer or its effects
/// change. Clones share the cache, which holds a few layers by identity and
/// a few recent states of each, so undo finds its drawing ready. All caches
/// share one byte budget (`RETAINED_LIMIT`): the least recently used
/// drawings anywhere are dropped first. A folder's cache also keeps its
/// composited contents, which its effects follow.
#[derive(Clone, Default)]
pub(crate) struct Cache(Entries, Contents);
type Entries = std::sync::Arc<std::sync::Mutex<Vec<Cached>>>;
type Contents = std::sync::Arc<std::sync::Mutex<Vec<Content>>>;
/// A folder's contents composited (straight RGBA): what its effects follow,
/// as a layer's effects follow its pixels. The same contents keep the same
/// pixels, so drawings of them are found again and redrawn only around what
/// changed.
struct Content {
    /// The layers composited, depths relative to the folder, without caches.
    children: Vec<Layer>,
    pixels: crate::pixels::Pixels,
    used: u64,
}
struct Cached {
    /// Unique per drawing as shown, so a display can tell it has not shown
    /// it yet; it changes when the same parts are shown differently.
    generation: u64,
    /// The effects and look the drawing was last shown with.
    shown: (Vec<Effect>, Look),
    /// Pixel bytes held by the drawing.
    bytes: usize,
    used: u64,
    id: crate::document::LayerId,
    pixels: crate::pixels::Pixels,
    mask: Option<crate::layer_mask::LayerMask>,
    clipped: bool,
    /// Exact fill opacity; the combined owner must distinguish changes even
    /// when they round to the same byte in the effect coverage renderer.
    fill: f32,
    size: [usize; 2],
    parts: Vec<Part>,
    /// Reduced-fill owner and its inside effects, with coverage applied once.
    combined: Option<Combined>,
}

#[derive(Clone)]
struct Combined {
    layer: Layer,
    effects: Vec<Effect>,
}
const CACHED_LAYERS: usize = 8;
/// Drawings kept of one layer's recent states.
const STATES_PER_LAYER: usize = 3;
/// Effect drawings kept across every open document, in pixel bytes.
pub(crate) const RETAINED_LIMIT: usize = 1 << 30;
static TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static CACHES: std::sync::Mutex<Vec<std::sync::Weak<std::sync::Mutex<Vec<Cached>>>>> =
    std::sync::Mutex::new(Vec::new());
fn tick() -> u64 {
    TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
/// Pixel bytes of all retained effect drawings (busy caches are skipped).
pub(crate) fn retained_bytes() -> usize {
    let caches = CACHES.lock().unwrap_or_else(|e| e.into_inner());
    caches
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .filter_map(|cache| {
            cache
                .try_lock()
                .ok()
                .map(|entries| entries.iter().map(|c| c.bytes).sum::<usize>())
        })
        .sum()
}
/// Drop the least recently used drawings anywhere until all fit the budget,
/// sparing `keep` (the drawing just made). Only `try_lock`s caches, so it
/// never waits on one while holding the registry.
fn enforce_budget(keep: u64) {
    let mut caches = CACHES.lock().unwrap_or_else(|e| e.into_inner());
    caches.retain(|weak| weak.strong_count() > 0);
    let live: Vec<Entries> = caches.iter().filter_map(std::sync::Weak::upgrade).collect();
    drop(caches);
    evict(&live, keep, RETAINED_LIMIT);
}
/// Drop the least recently used drawings in `live` until they total at most
/// `limit` bytes, sparing the drawing `keep`.
fn evict(live: &[Entries], keep: u64, limit: usize) {
    loop {
        let mut total = 0;
        let mut oldest: Option<(u64, usize, u64)> = None;
        for (index, cache) in live.iter().enumerate() {
            let Ok(entries) = cache.try_lock() else {
                continue;
            };
            for cached in entries.iter() {
                total += cached.bytes;
                if cached.generation != keep && oldest.is_none_or(|(used, ..)| cached.used < used) {
                    oldest = Some((cached.used, index, cached.generation));
                }
            }
        }
        let Some((_, index, generation)) = oldest.filter(|_| total > limit) else {
            return;
        };
        let Ok(mut entries) = live[index].try_lock() else {
            return;
        };
        entries.retain(|c| c.generation != generation);
    }
}

/// A layer's fill opacity as the byte effects are drawn with.
fn fill_byte(layer: &Layer) -> u8 {
    (layer.fill.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// What besides pixels, mask and effects shows in a layer's drawing.
type Look = (String, f32, bool, BlendMode, bool);
fn look(layer: &Layer) -> Look {
    (
        layer.name.clone(),
        layer.opacity,
        layer.visible,
        layer.blend,
        layer.csp_byte_opacity,
    )
}

/// Whether two folder contents composite alike wherever their pixels do:
/// the same layers, settings and masks (depths may be shifted alike).
fn same_children(a: &[Layer], b: &[Layer]) -> bool {
    let shift = |l: &[Layer]| l.iter().map(|l| l.depth).min().unwrap_or(0);
    let (sa, sb) = (shift(a), shift(b));
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.depth - sa == b.depth - sb
                && a.visible == b.visible
                && a.opacity == b.opacity
                && a.csp_byte_opacity == b.csp_byte_opacity
                && a.fill == b.fill
                && a.blend == b.blend
                && a.clipped == b.clipped
                && a.folder == b.folder
                && a.adjustment == b.adjustment
                && a.display_key() == b.display_key()
                && crate::layer_mask::same_snapshot(&a.mask, &b.mask)
        })
}

/// A folder as the owner of its effects: a layer whose pixels are its
/// composited `children`.
fn with_content(folder: &Layer, pixels: crate::pixels::Pixels) -> Layer {
    let mut owner = folder.clone();
    owner.pixels = pixels;
    owner
}

/// `effects` as layers from drawn `parts`, matched by what they draw.
/// `loose` stands in an effect's earlier drawing at the same position (or
/// nothing) when none matches, for a preview; otherwise that fails.
fn assemble(
    parts: &[Part],
    effects: &[Effect],
    layer: &Layer,
    loose: bool,
    combined: Option<&Combined>,
) -> Option<[Vec<Layer>; 3]> {
    let mut used = vec![false; parts.len()];
    let mut placed: [Vec<Layer>; 3] = Default::default();
    if let Some(combined) = combined {
        if !loose && combined.effects != effects {
            return None;
        }
        let mut owner = combined.layer.clone();
        owner.name = layer.name.clone();
        owner.opacity = layer.opacity;
        owner.csp_byte_opacity = layer.csp_byte_opacity;
        owner.visible = layer.visible;
        owner.blend = layer.blend;
        owner.clipped = layer.clipped;
        owner.depth = layer.depth;
        owner.locked = layer.locked;
        owner.alpha_locked = layer.alpha_locked;
        placed[Place::Inside as usize].push(owner);
    }
    for (index, effect) in effects.iter().enumerate() {
        if combined.is_some() && effect.place() == Place::Inside {
            continue;
        }
        let found = (0..parts.len())
            .find(|&k| !used[k] && same_drawing(&parts[k].effect, effect))
            .or_else(|| (loose && index < parts.len() && !used[index]).then_some(index));
        let Some(k) = found else {
            if loose {
                continue;
            }
            return None;
        };
        used[k] = true;
        if let Some(drawn) = &parts[k].layer {
            let mut drawn = drawn.clone();
            dress(&mut drawn, effect, layer, layer.opacity, parts[k].clip);
            placed[parts[k].place as usize].push(drawn);
        }
    }
    Some(placed)
}

impl Cached {
    /// Drawn from `layer`'s current pixels, mask and clipping.
    fn matches(&self, layer: &Layer, size: [usize; 2]) -> bool {
        self.id == layer.id
            && self.size == size
            && self.pixels.shares(&layer.pixels)
            && crate::layer_mask::same_snapshot(&self.mask, &layer.mask)
            && self.clipped == layer.clipped
            && self.fill == layer.fill
    }
    /// Its layers as `effects` on `layer`, with the generation identifying
    /// them: new whenever they are shown differently than last time.
    fn show(
        &mut self,
        layer: &Layer,
        effects: &[Effect],
        loose: bool,
    ) -> Option<([Vec<Layer>; 3], u64)> {
        let layers = assemble(&self.parts, effects, layer, loose, self.combined.as_ref())?;
        let shown = (effects.to_vec(), look(layer));
        if self.shown != shown {
            self.shown = shown;
            self.generation = tick();
        }
        self.used = tick();
        Some((layers, self.generation))
    }
}

impl Cache {
    /// Visit the pixel allocations of every drawing held, for memory totals.
    pub(crate) fn visit_allocations(
        &self,
        mut visit: impl FnMut(crate::pixels::AllocationId, usize),
    ) {
        let entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for layer in entries
            .iter()
            .flat_map(|c| &c.parts)
            .filter_map(|p| p.layer.as_ref())
        {
            layer.pixels.visit_allocations(&mut visit);
        }
        for combined in entries.iter().filter_map(|c| c.combined.as_ref()) {
            combined.layer.pixels.visit_allocations(&mut visit);
        }
        let contents = self.1.lock().unwrap_or_else(|e| e.into_inner());
        for content in contents.iter() {
            content.pixels.visit_allocations(&mut visit);
        }
    }
    /// Forget `id`'s drawing, once its effects are gone or switched off.
    fn forget(&self, id: crate::document::LayerId) {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|c| c.id != id);
        drop(entries);
        self.1.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    /// The composited contents of a folder holding `children`, if kept.
    fn content_ready(&self, children: &[Layer]) -> Option<crate::pixels::Pixels> {
        let mut contents = self.1.lock().unwrap_or_else(|e| e.into_inner());
        let found = contents.iter_mut().find(|c| {
            same_children(&c.children, children)
                && c.children
                    .iter()
                    .zip(children)
                    .all(|(a, b)| a.pixels.shares(&b.pixels))
        })?;
        found.used = tick();
        Some(found.pixels.clone())
    }
    /// The composited contents of a folder holding `children` (depths as in
    /// the document, inside a folder at `depth`): kept, updated from the
    /// last kept where only pixels changed, or composited whole.
    fn content(
        &self,
        children: &[Layer],
        depth: u8,
        [width, height]: [usize; 2],
    ) -> crate::pixels::Pixels {
        if let Some(pixels) = self.content_ready(children) {
            return pixels;
        }
        let base = {
            let contents = self.1.lock().unwrap_or_else(|e| e.into_inner());
            contents
                .iter()
                .filter(|c| same_children(&c.children, children))
                .max_by_key(|c| c.used)
                .map(|c| (c.children.clone(), c.pixels.clone()))
        };
        let mut stack = Document::new(1, 1);
        (stack.width, stack.height) = (width, height);
        stack.selection = None;
        stack.layers = children
            .iter()
            .map(|child| {
                let mut child = child.clone();
                child.depth -= depth + 1;
                child.effect_cache = Cache::default();
                child
            })
            .collect();
        stack.active = 0;
        let pixels = match base {
            Some((old, mut pixels)) => {
                let changed: Vec<[usize; 4]> = old
                    .iter()
                    .zip(children)
                    .flat_map(|(a, b)| a.pixels.changed_regions(&b.pixels, width, height))
                    .collect();
                for (rect, _) in redraw_windows(&changed, 0, [width, height]) {
                    let rgba = stack.composite_display_region(rect, true);
                    pixels.writer().write_region(width, rect, &rgba);
                }
                pixels
            }
            None => {
                let mut pixels = crate::pixels::Pixels::sparse(width, height);
                if !stack.layers.is_empty() {
                    pixels.writer().write_region(
                        width,
                        [0, 0, width, height],
                        &stack.composite_transparent(),
                    );
                }
                pixels
            }
        };
        let mut contents = self.1.lock().unwrap_or_else(|e| e.into_inner());
        while contents.len() >= STATES_PER_LAYER {
            let oldest = (0..contents.len())
                .min_by_key(|&i| contents[i].used)
                .unwrap();
            contents.remove(oldest);
        }
        contents.push(Content {
            children: stack.layers,
            pixels: pixels.clone(),
            used: tick(),
        });
        pixels
    }
    /// The cached drawing of `layer` if it matches, or with `stale` its
    /// most recent drawing at this size, standing in (while painting).
    fn lookup(
        &self,
        layer: &Layer,
        effects: &[Effect],
        size: [usize; 2],
        stale: bool,
    ) -> Option<([Vec<Layer>; 3], u64)> {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if stale {
            entries
                .iter_mut()
                .filter(|c| c.id == layer.id && c.size == size)
                .max_by_key(|c| c.used)?
                .show(layer, effects, true)
        } else {
            entries
                .iter_mut()
                .filter(|c| c.matches(layer, size))
                .find_map(|c| c.show(layer, effects, false))
        }
    }
    /// The effect layers of `layer`, drawn now unless cached. `stale`
    /// accepts a cached drawing that no longer matches (while painting).
    /// Drawing happens without holding the cache, so a canvas reading it
    /// meanwhile never waits for a background drawing.
    fn layers(
        &self,
        layer: &Layer,
        effects: &[Effect],
        size: [usize; 2],
        stale: bool,
        cancel: &dyn Fn() -> bool,
    ) -> Result<([Vec<Layer>; 3], u64), String> {
        if let Some(found) = self.lookup(layer, effects, size, false) {
            return Ok(found);
        }
        if stale && let Some(found) = self.lookup(layer, effects, size, true) {
            return Ok(found);
        }
        let parts = self.redraw(layer, effects, size, cancel)?;
        let combined = self.combine_inside(layer, effects, &parts, size, cancel)?;
        let generation = tick();
        let layers = assemble(&parts, effects, layer, false, combined.as_ref())
            .expect("every effect was drawn");
        let bytes: usize = parts
            .iter()
            .filter_map(|p| p.layer.as_ref())
            .map(|l| l.pixels.payload_bound())
            .sum::<usize>()
            + combined
                .as_ref()
                .map_or(0, |c| c.layer.pixels.payload_bound());
        // A drawing larger than the whole budget is used once, not kept.
        if bytes > RETAINED_LIMIT {
            return Ok((layers, generation));
        }
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|c| !c.matches(layer, size));
        // Keep a few recent states of this layer, and a few layers.
        let mut states: Vec<u64> = entries
            .iter()
            .filter(|c| c.id == layer.id)
            .map(|c| c.used)
            .collect();
        states.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(&oldest_kept) = states.get(STATES_PER_LAYER - 2) {
            entries.retain(|c| c.id != layer.id || c.used >= oldest_kept);
        }
        while entries.len() >= CACHED_LAYERS {
            let oldest = (0..entries.len()).min_by_key(|&i| entries[i].used).unwrap();
            entries.remove(oldest);
        }
        entries.push(Cached {
            generation,
            shown: (effects.to_vec(), look(layer)),
            bytes,
            used: generation,
            id: layer.id,
            pixels: layer.pixels.clone(),
            mask: layer.mask.clone(),
            clipped: layer.clipped,
            fill: layer.fill,
            size,
            parts,
            combined,
        });
        drop(entries);
        {
            let mut caches = CACHES.lock().unwrap_or_else(|e| e.into_inner());
            if !caches
                .iter()
                .any(|weak| weak.as_ptr() == std::sync::Arc::as_ptr(&self.0))
            {
                caches.push(std::sync::Arc::downgrade(&self.0));
            }
        }
        enforce_budget(generation);
        Ok((layers, generation))
    }
    /// Combine reduced fill and inside effects in layer space. Coverage and
    /// mask apply only after blending there, so each soft edge is applied once.
    fn combine_inside(
        &self,
        layer: &Layer,
        effects: &[Effect],
        parts: &[Part],
        size: [usize; 2],
        cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Combined>, String> {
        if layer.folder || layer.fill == 1.0 || !effects.iter().any(|e| e.place() == Place::Inside)
        {
            return Ok(None);
        }
        // The result cannot occupy more source tiles than the owner. Reserve
        // that upper bound before allocating output, alongside the raw effects.
        let part_bytes = parts
            .iter()
            .filter_map(|p| p.layer.as_ref())
            .map(|l| l.pixels.payload_bound())
            .sum::<usize>();
        if part_bytes.saturating_add(layer.pixels.payload_bound()) > crate::document::MAX_PAYLOAD {
            return Err("Layer effects exceed the memory budget".into());
        }
        let base = {
            let entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .iter()
                .filter(|c| {
                    c.id == layer.id
                        && c.size == size
                        && c.fill == layer.fill
                        && c.clipped == layer.clipped
                        && crate::layer_mask::same_snapshot(&c.mask, &layer.mask)
                        && c.combined.as_ref().is_some_and(|c| c.effects == effects)
                })
                .max_by_key(|c| c.used)
                .map(|c| {
                    (
                        c.pixels.clone(),
                        c.parts.clone(),
                        c.combined.as_ref().unwrap().layer.clone(),
                    )
                })
        };
        let (mut joined, mut regions) = match base {
            Some((pixels, previous, joined)) => {
                let mut regions = pixels.changed_regions(&layer.pixels, size[0], size[1]);
                for (old, new) in previous
                    .iter()
                    .zip(parts)
                    .filter(|(_, p)| p.place == Place::Inside)
                {
                    match (&old.layer, &new.layer) {
                        (Some(a), Some(b)) => {
                            regions.extend(a.pixels.changed_regions(&b.pixels, size[0], size[1]))
                        }
                        (Some(a), None) => regions.extend(a.pixels.nonzero_tiles(size[0], size[1])),
                        (None, Some(b)) => regions.extend(b.pixels.nonzero_tiles(size[0], size[1])),
                        (None, None) => {}
                    }
                }
                (joined, regions)
            }
            None => {
                let mut joined = Layer::raster(layer.name.clone(), size[0], size[1]);
                // Expansion recognizes this as a replacement for the owner.
                joined.id = layer.id;
                (joined, layer.pixels.nonzero_tiles(size[0], size[1]))
            }
        };
        regions.sort_unstable();
        regions.dedup();
        for rect in regions {
            if cancel() {
                return Err("Canceled".into());
            }
            let source = layer.pixels.region(size[0], rect);
            let inside: Vec<_> = effects
                .iter()
                .zip(parts)
                .filter(|(e, _)| e.place() == Place::Inside)
                .filter_map(|(e, p)| {
                    p.layer
                        .as_ref()
                        .map(|p| (e, p.pixels.region(size[0], rect)))
                })
                .collect();
            let mut output = vec![0; source.len()];
            let width = rect[2] - rect[0];
            for (i, (src, dst)) in source
                .chunks_exact(4)
                .zip(output.chunks_exact_mut(4))
                .enumerate()
            {
                if i % (width * 32) == 0 && cancel() {
                    return Err("Canceled".into());
                }
                let at = (rect[1] + i / width) * size[0] + rect[0] + i % width;
                let coverage = f32::from(src[3]) / 255.0 * layer.mask_coverage(at, size[0]);
                if coverage == 0.0 {
                    continue;
                }
                let mut alpha = layer.fill;
                let mut color = [0, 1, 2].map(|c| f32::from(src[c]) / 255.0 * alpha);
                for (effect, pixels) in &inside {
                    let pixel = &pixels[i * 4..i * 4 + 4];
                    let a = f32::from(pixel[3]) / 255.0 * effect.opacity;
                    let top = [0, 1, 2].map(|c| f32::from(pixel[c]) / 255.0);
                    let bottom = if alpha > 0.0 {
                        color.map(|c| c / alpha)
                    } else {
                        [0.0; 3]
                    };
                    if let Some(glow) = effect.blend.glow_base() {
                        let weighted = top.map(|c| c * a);
                        let blended = glow.pixel(bottom, weighted);
                        color = [0, 1, 2].map(|c| blended[c] * alpha + weighted[c] * (1.0 - alpha));
                    } else {
                        let blended = effect.blend.pixel(bottom, top);
                        color = [0, 1, 2].map(|c| {
                            color[c] * (1.0 - a)
                                + top[c] * a * (1.0 - alpha)
                                + blended[c] * a * alpha
                        });
                    }
                    alpha = a + alpha * (1.0 - a);
                }
                if alpha > 0.0 {
                    for c in 0..3 {
                        dst[c] = (color[c] / alpha * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                    dst[3] = (alpha * coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
            joined.pixels.writer().write_region(size[0], rect, &output);
        }
        if part_bytes.saturating_add(joined.pixels.payload_bound()) > crate::document::MAX_PAYLOAD {
            return Err("Layer effects exceed the memory budget".into());
        }
        Ok(Some(Combined {
            layer: joined,
            effects: effects.to_vec(),
        }))
    }

    /// Draw `effects` for `layer`, reusing its most recent drawing: effects
    /// drawn the same way are kept, and where only some pixels changed they
    /// are redrawn just around those.
    fn redraw(
        &self,
        layer: &Layer,
        effects: &[Effect],
        size: [usize; 2],
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<Part>, String> {
        let mut budget = 0;
        let base = {
            let entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
            entries
                .iter()
                .filter(|c| {
                    c.id == layer.id
                        && c.size == size
                        && c.clipped == layer.clipped
                        && c.fill == layer.fill
                        && crate::layer_mask::same_snapshot(&c.mask, &layer.mask)
                })
                .max_by_key(|c| c.used)
                .map(|c| (c.pixels.clone(), c.parts.clone()))
        };
        let mut parts: Vec<Option<Part>> = vec![None; effects.len()];
        if let Some((pixels, base)) = base {
            let changed = pixels.changed_regions(&layer.pixels, size[0], size[1]);
            let mut used = vec![false; base.len()];
            let mut update = Vec::new();
            for (index, effect) in effects.iter().enumerate() {
                let Some(k) =
                    (0..base.len()).find(|&k| !used[k] && same_drawing(&base[k].effect, effect))
                else {
                    continue;
                };
                // Gradients span the whole layer, so they are redrawn whole.
                let partial =
                    base[k].layer.is_some() && !matches!(effect.paint, Paint::Gradient(_));
                if changed.is_empty() || partial {
                    used[k] = true;
                    parts[index] = Some(base[k].clone());
                    if !changed.is_empty() {
                        update.push(index);
                    }
                }
            }
            if !update.is_empty() {
                let subset: Vec<Effect> = update.iter().map(|&i| effects[i].clone()).collect();
                let influence = subset.iter().map(Effect::influence).max().unwrap_or(0);
                let windows = redraw_windows(&changed, influence, size);
                let whole = extent(layer, None, size, influence).map_or(0, area);
                if windows
                    .iter()
                    .map(|(_, window)| area(*window))
                    .sum::<usize>()
                    * 2
                    > whole
                {
                    // Changed nearly everywhere: drawing whole is cheaper.
                    for &index in &update {
                        parts[index] = None;
                    }
                } else {
                    for (output, window) in windows {
                        // The span only places gradients, which redraw whole.
                        let drawn = draw_rect(
                            &subset,
                            layer,
                            fill_byte(layer),
                            None,
                            None,
                            size,
                            window,
                            Some([0.0; 4]),
                            layer.fill < 1.0 && !layer.folder,
                            &mut budget,
                            &cancel,
                        )?
                        .expect("drawing with a span always draws");
                        for (&index, (.., pixels)) in update.iter().zip(drawn) {
                            let part = parts[index].as_mut().expect("kept above");
                            part.layer
                                .as_mut()
                                .expect("kept above")
                                .pixels
                                .writer()
                                .write_region(size[0], output, &crop(&pixels, window, output));
                        }
                    }
                }
            }
        }
        let missing: Vec<usize> = (0..effects.len()).filter(|&i| parts[i].is_none()).collect();
        if !missing.is_empty() {
            let subset: Vec<Effect> = missing.iter().map(|&i| effects[i].clone()).collect();
            let drawn = draw_parts(
                &subset,
                layer,
                fill_byte(layer),
                None,
                None,
                size,
                layer.fill < 1.0 && !layer.folder,
                &mut budget,
                &cancel,
            )?;
            for (index, part) in missing.into_iter().zip(drawn) {
                parts[index] = Some(part);
            }
        }
        Ok(parts.into_iter().map(|p| p.expect("drawn above")).collect())
    }
}

/// A drawing being made on a background worker, so each layer has at most
/// one: a newer state waits for it and is drawn next, from it.
struct InFlight {
    id: crate::document::LayerId,
    pixels: crate::pixels::Pixels,
}
static IN_FLIGHT: std::sync::Mutex<Vec<InFlight>> = std::sync::Mutex::new(Vec::new());

/// Called when a background drawing finishes, to show it.
pub(crate) type Wake = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Draw `layer`'s effects on a background worker unless one is already
/// drawing this layer; it wakes the display when done, which then asks
/// for the newest state. Keeping one at a time lets each drawing build on
/// the last, so effects follow a stroke while it is painted.
fn draw_in_background(
    layer: &Layer,
    effects: &[Effect],
    size: [usize; 2],
    children: Option<&[Layer]>,
    wake: &Wake,
) {
    let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    if in_flight.iter().any(|f| f.id == layer.id) {
        return;
    }
    let pixels = layer.pixels.clone();
    in_flight.push(InFlight {
        id: layer.id,
        pixels: pixels.clone(),
    });
    drop(in_flight);
    let (layer, effects, wake) = (layer.clone(), effects.to_vec(), wake.clone());
    let children = children.map(<[Layer]>::to_vec);
    // The handle is not kept: the drawing lands in the layer's shared cache.
    let _ = crate::background::spawn(move || {
        let owner = children.map(|children| {
            let content = layer.effect_cache.content(&children, layer.depth, size);
            with_content(&layer, content)
        });
        let owner = owner.as_ref().unwrap_or(&layer);
        let _ = owner
            .effect_cache
            .layers(owner, &effects, size, false, &|| false);
        IN_FLIGHT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|f| !(f.id == layer.id && f.pixels.shares(&pixels)));
        wake();
    });
}

/// The document with each layer's enabled effects drawn as layers around
/// it, as Photoshop's Create Layers would: what the compositor draws. A
/// folder's effects follow its composited contents; those under it go
/// beneath its contents. `stale` reuses drawings made before the layer
/// last changed, for a fast preview while painting.
pub(crate) fn expand(
    doc: &Document,
    stale: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<Document, String> {
    expand_tracked(doc, stale, cancel).map(|(doc, _)| doc)
}
/// `expand`, also identifying the effect drawings it used: when that changes
/// (whoever drew them), effects may have changed anywhere they reach.
pub(crate) fn expand_tracked(
    doc: &Document,
    stale: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(Document, Vec<u64>), String> {
    expand_with(doc, |layer, effects, size, children| {
        let owner = children.map(|children| {
            let content = layer.effect_cache.content(children, layer.depth, size);
            with_content(layer, content)
        });
        let owner = owner.as_ref().unwrap_or(layer);
        owner
            .effect_cache
            .layers(owner, effects, size, stale, cancel)
            .map(Some)
    })
}
/// For the canvas: `expand_tracked` without waiting. A layer whose current
/// drawing is not ready shows its previous drawing (or none yet) while a
/// background worker draws it; `wake` runs when it is ready. The flag is
/// true while any drawing is still on its way.
pub(crate) fn expand_for_display(doc: &Document, wake: &Wake) -> (Document, Vec<u64>, bool) {
    let mut waiting = false;
    let expanded = expand_with(doc, |layer, effects, size, children| {
        let cache = &layer.effect_cache;
        let owner = match children {
            Some(children) => cache
                .content_ready(children)
                .map(|content| with_content(layer, content)),
            None => Some(layer.clone()),
        };
        if let Some(owner) = &owner
            && let Some(found) = cache.lookup(owner, effects, size, false)
        {
            return Ok(Some(found));
        }
        draw_in_background(layer, effects, size, children, wake);
        waiting = true;
        Ok(cache.lookup(layer, effects, size, true))
    });
    let (doc, drawings) = expanded.expect("display expansion never draws, so never fails");
    (doc, drawings, waiting)
}
fn expand_with(
    doc: &Document,
    mut drawing: impl FnMut(
        &Layer,
        &[Effect],
        [usize; 2],
        Option<&[Layer]>,
    ) -> Result<Option<([Vec<Layer>; 3], u64)>, String>,
) -> Result<(Document, Vec<u64>), String> {
    let mut drawings = Vec::new();
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
        // A folder's contents are the deeper entries just before it.
        let start = layer.folder.then(|| {
            let mut start = out.layers.len();
            while start > 0 && out.layers[start - 1].depth > layer.depth {
                start -= 1;
            }
            start
        });
        let [below, mut inside, above] = if effects.is_empty() {
            layer.effect_cache.forget(layer.id);
            Default::default()
        } else if layer.folder && !layer.visible {
            // Nothing shows; skip compositing the contents.
            Default::default()
        } else {
            let children = start.map(|start| &out.layers[start..]);
            match drawing(layer, &effects, [doc.width, doc.height], children)? {
                Some((layers, generation)) => {
                    drawings.push(generation);
                    layers
                }
                None => Default::default(),
            }
        };
        let depth = layer.depth;
        let at_depth = move |mut l: Layer| {
            l.depth = depth;
            l
        };
        let below: Vec<Layer> = below.into_iter().map(at_depth).collect();
        match start {
            Some(start) => {
                if doc.active < index && out.active >= start {
                    out.active += below.len();
                }
                out.layers.splice(start..start, below);
            }
            None => out.layers.extend(below),
        }
        if index == doc.active {
            out.active = out.layers.len();
        }
        let mut owner = if let Some(position) = inside.iter().position(|l| l.id == layer.id) {
            inside.remove(position)
        } else {
            layer.clone()
        };
        owner.effects.clear();
        // Fill fades the layer's own pixels, not its effects.
        owner.opacity *= owner.fill;
        owner.fill = 1.0;
        out.layers.push(owner);
        out.layers.extend(inside.into_iter().map(at_depth));
        if !above.is_empty() {
            pending.extend(above.into_iter().map(at_depth));
            pending_depth = depth;
        }
    }
    out.layers.append(&mut pending);
    Ok((out, drawings))
}

/// Settings beyond these are rejected as damaged.
const MAX_SIZE: f64 = 1000.0;

/// `lfx2`/`lfxs`/`lmfx`: a version word, then a versioned descriptor.
/// `global_angle` is the document's light angle in degrees.
pub(crate) fn parse(
    data: &[u8],
    global_angle: f64,
    patterns: &[crate::pattern::Pattern],
) -> Result<Effects, String> {
    let settings = crate::psd_vector::versioned(data.get(4..).ok_or("Truncated PSD effects")?)?;
    let mut effects = Effects::default();
    // With the master switch off nothing is drawn, but every effect is kept.
    let master = settings.flag("masterFXSwitch") != Some(false);
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
            // Photoshop lists effects never added as switched off, without
            // `present`; switched-off ones the user added say `present`.
            let switched_on = effect.flag("enab") == Some(true);
            if !switched_on && effect.flag("present") != Some(true) {
                continue;
            }
            let drawn = switched_on && master;
            // Notes about switched-off effects would describe nothing shown.
            let mut notes = Vec::new();
            let approximated = if drawn {
                &mut effects.approximated
            } else {
                &mut notes
            };
            let parsed = if effect.class == "ebbl" {
                let pair = effects
                    .list
                    .iter()
                    .chain(&effects.disabled)
                    .filter(|e| matches!(e.kind, Kind::Bevel { .. }))
                    .count() as u32
                    / 2
                    + 1;
                bevel(effect, global_angle, pair, approximated)
            } else {
                read(effect, global_angle, approximated, patterns).map(Vec::from_iter)
            };
            match (parsed, drawn) {
                (Ok(parsed), true) => effects.list.extend(parsed),
                (Ok(parsed), false) => effects.disabled.extend(parsed),
                (Err(skipped), true) => effects.skipped.push(skipped),
                (Err(_), false) => {}
            }
        }
    }
    effects.list.sort_by_key(Effect::rank);
    effects.disabled.sort_by_key(Effect::rank);
    Ok(effects)
}

/// Photoshop's descriptor name for an effect blend mode; `None` for the
/// modes it lacks.
fn blend_name(mode: BlendMode) -> Option<&'static str> {
    use BlendMode::*;
    Some(match mode {
        Normal => "normal",
        Dissolve => "dissolve",
        Darken => "darken",
        Multiply => "multiply",
        ColorBurn => "colorBurn",
        LinearBurn => "linearBurn",
        DarkerColor => "darkerColor",
        Lighten => "lighten",
        Screen => "screen",
        ColorDodge => "colorDodge",
        LinearDodge => "linearDodge",
        LighterColor => "lighterColor",
        Overlay => "overlay",
        SoftLight => "softLight",
        HardLight => "hardLight",
        VividLight => "vividLight",
        LinearLight => "linearLight",
        PinLight => "pinLight",
        HardMix => "hardMix",
        Difference => "difference",
        Exclusion => "exclusion",
        Subtract => "blendSubtraction",
        Divide => "blendDivide",
        Hue => "hue",
        Saturation => "saturation",
        Color => "color",
        Luminosity => "luminosity",
        GlowDodge | AddGlow | PassThrough => return None,
    })
}

/// `lfx2` for a layer's effects, the inverse of `parse`, with the keys
/// Photoshop writes for each effect. `None` when Photoshop cannot show them
/// all: blend modes it lacks, gradient glows, or more than one glow or bevel
/// of a kind (Photoshop has one of each); those layers keep drawn effects.
pub(crate) fn to_psd(effects: &[LayerEffect]) -> Option<Vec<u8>> {
    let unit = |unit: &str, v: f64| Value::Unit(unit.into(), v);
    let pixels = |v: f64| unit("#Pxl", v);
    let percent = |v: f64| unit("#Prc", v * 100.0);
    let enumerated = |kind: &str, value: &str| Value::Enum(kind.into(), value.into());
    let rgb = |[r, g, b]: [u8; 3]| {
        Value::Object(Descriptor {
            class: "RGBC".into(),
            items: [("Rd  ", r), ("Grn ", g), ("Bl  ", b)]
                .map(|(k, v)| (k.into(), Value::Number(f64::from(v))))
                .into(),
        })
    };
    let linear = || {
        let point = |v: f64| {
            Value::Object(Descriptor {
                class: "CrPt".into(),
                items: vec![
                    ("Hrzn".into(), Value::Number(v)),
                    ("Vrtc".into(), Value::Number(v)),
                ],
            })
        };
        Value::Object(Descriptor {
            class: "ShpC".into(),
            items: vec![
                ("Nm  ".into(), Value::Text("Linear\0".into())),
                ("Crv ".into(), Value::List(vec![point(0.0), point(255.0)])),
            ],
        })
    };
    // [x, y] offset back to Photoshop's angle (degrees) and distance.
    let polar = |[x, y]: [f64; 2]| (y.atan2(-x).to_degrees(), x.hypot(y));
    let mut slots: Vec<(&str, Vec<Descriptor>)> = Vec::new();
    let mut add =
        |key: &'static str, effect: Descriptor| match slots.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(effect),
            None => slots.push((key, vec![effect])),
        };
    let mut bevels: Vec<&Effect> = Vec::new();
    for layer_effect in effects {
        let effect = &layer_effect.effect;
        let mut items: Vec<(&str, Value)> = vec![
            ("enab", Value::Bool(layer_effect.enabled)),
            ("present", Value::Bool(true)),
            ("showInDialog", Value::Bool(true)),
        ];
        if let Kind::Bevel { .. } = effect.kind {
            bevels.push(effect);
            continue;
        }
        items.push(("Md  ", enumerated("BlnM", blend_name(effect.blend)?)));
        let color = match &effect.paint {
            Paint::Color(c) => Some(*c),
            Paint::Gradient(_) | Paint::Pattern(_) => None,
        };
        let opacity = ("Opct", percent(f64::from(effect.opacity)));
        let (key, class) = match effect.kind {
            Kind::DropShadow {
                offset,
                spread,
                size,
                ..
            }
            | Kind::InnerShadow {
                offset,
                choke: spread,
                size,
            } if color.is_some() => {
                let (angle, distance) = polar(offset);
                items.extend([
                    ("Clr ", rgb(color?)),
                    opacity,
                    ("uglg", Value::Bool(false)),
                    ("lagl", unit("#Ang", angle)),
                    ("Dstn", pixels(distance)),
                    ("Ckmt", pixels(spread * 100.0)),
                    ("blur", pixels(size)),
                    ("Nose", percent(0.0)),
                    ("AntA", Value::Bool(false)),
                    ("TrnS", linear()),
                ]);
                if let Kind::DropShadow { knockout, .. } = effect.kind {
                    items.push(("layerConceals", Value::Bool(knockout)));
                    ("dropShadowMulti", "DrSh")
                } else {
                    ("innerShadowMulti", "IrSh")
                }
            }
            Kind::OuterGlow {
                spread,
                size,
                precise,
                range,
            }
            | Kind::InnerGlow {
                choke: spread,
                size,
                precise,
                range,
                ..
            } if color.is_some() => {
                items.extend([
                    ("Clr ", rgb(color?)),
                    opacity,
                    (
                        "GlwT",
                        enumerated("BETE", if precise { "PrBL" } else { "SfBL" }),
                    ),
                    ("Ckmt", pixels(spread * 100.0)),
                    ("blur", pixels(size)),
                    ("Nose", percent(0.0)),
                    ("ShdN", percent(0.0)),
                    ("AntA", Value::Bool(false)),
                    ("TrnS", linear()),
                    ("Inpr", percent(range)),
                ]);
                if let Kind::InnerGlow { center, .. } = effect.kind {
                    let source = if center { "SrcC" } else { "SrcE" };
                    items.push(("glwS", enumerated("IGSr", source)));
                    ("IrGl", "IrGl")
                } else {
                    ("OrGl", "OrGl")
                }
            }
            Kind::ColorOverlay if color.is_some() => {
                items.extend([("Clr ", rgb(color?)), opacity]);
                ("solidFillMulti", "SoFi")
            }
            Kind::Satin {
                offset,
                size,
                invert,
            } if color.is_some() => {
                let (angle, distance) = polar(offset);
                items.extend([
                    ("Clr ", rgb(color?)),
                    ("AntA", Value::Bool(false)),
                    ("Invr", Value::Bool(invert)),
                    opacity,
                    ("lagl", unit("#Ang", angle)),
                    ("Dstn", pixels(distance)),
                    ("blur", pixels(size)),
                    ("MpgS", linear()),
                ]);
                ("ChFX", "ChFX")
            }
            Kind::PatternOverlay => {
                let Paint::Pattern(paint) = &effect.paint else {
                    return None;
                };
                items.push(opacity);
                items.extend(pattern_items(paint));
                add("patternFill", descriptor("patternFill", items));
                continue;
            }
            Kind::GradientOverlay => {
                let Paint::Gradient(gradient) = &effect.paint else {
                    return None;
                };
                items.push(opacity);
                let rest: Vec<_> = gradient.psd_items();
                let mut descriptor = descriptor("GrFl", items);
                descriptor.items.extend(rest);
                add("gradientFillMulti", descriptor);
                continue;
            }
            Kind::Stroke { size, position } => {
                let style = match position {
                    Position::Outside => "OutF",
                    Position::Inside => "InsF",
                    Position::Center => "CtrF",
                };
                items.extend([
                    ("Styl", enumerated("FStl", style)),
                    (
                        "PntT",
                        enumerated(
                            "FrFl",
                            match effect.paint {
                                Paint::Color(_) => "SClr",
                                Paint::Gradient(_) => "GrFl",
                                Paint::Pattern(_) => "Ptrn",
                            },
                        ),
                    ),
                    opacity,
                    ("Sz  ", pixels(size)),
                    ("overprint", Value::Bool(false)),
                ]);
                let mut descriptor = descriptor("FrFX", items);
                match &effect.paint {
                    Paint::Color(c) => descriptor.items.push(("Clr ".into(), rgb(*c))),
                    Paint::Gradient(gradient) => descriptor.items.extend(gradient.psd_items()),
                    Paint::Pattern(paint) => descriptor
                        .items
                        .extend(pattern_items(paint).into_iter().map(|(k, v)| (k.into(), v))),
                }
                add("frameFXMulti", descriptor);
                continue;
            }
            // Glows, shadows and color overlays painted with a gradient.
            _ => return None,
        };
        add(key, descriptor(class, items));
    }
    // A bevel's halves share a pair number (0 in older projects, which
    // pair by adjacency); Photoshop keeps one bevel per layer.
    let pairs: Vec<u32> = bevels
        .iter()
        .filter_map(|e| match e.kind {
            Kind::Bevel { pair, .. } => Some(pair),
            _ => None,
        })
        .collect();
    if bevels.len() > 2 || (bevels.len() == 2 && pairs[0] != pairs[1]) {
        return None;
    }
    if let Some(first) = bevels.first() {
        let half = |highlight: bool| {
            bevels
                .iter()
                .find(|e| matches!(e.kind, Kind::Bevel { highlight: h, .. } if h == highlight))
        };
        let Kind::Bevel {
            style,
            technique,
            depth,
            size,
            soften,
            light,
            up,
            ..
        } = first.kind
        else {
            unreachable!()
        };
        let enabled = effects
            .iter()
            .filter(|e| matches!(e.effect.kind, Kind::Bevel { .. }))
            .any(|e| e.enabled);
        let mut items: Vec<(&str, Value)> = vec![
            ("enab", Value::Bool(enabled)),
            ("present", Value::Bool(true)),
            ("showInDialog", Value::Bool(true)),
        ];
        // A missing half is written invisible, as Photoshop needs both.
        for (highlight, [mode, color, opacity]) in [
            (true, ["hglM", "hglC", "hglO"]),
            (false, ["sdwM", "sdwC", "sdwO"]),
        ] {
            let (blend, paint, alpha) = match half(highlight) {
                Some(e) => (e.blend, &e.paint, f64::from(e.opacity)),
                None => (BlendMode::Normal, &Paint::Color([0; 3]), 0.0),
            };
            let Paint::Color(c) = paint else {
                return None;
            };
            items.extend([
                (mode, enumerated("BlnM", blend_name(blend)?)),
                (color, rgb(*c)),
                (opacity, percent(alpha)),
            ]);
        }
        let altitude = light[2].clamp(-1.0, 1.0).asin().to_degrees();
        let angle = (-light[1]).atan2(light[0]).to_degrees();
        items.extend([
            (
                "bvlT",
                enumerated(
                    "bvlT",
                    match technique {
                        Technique::Smooth => "SfBL",
                        Technique::ChiselHard => "PrBL",
                        Technique::ChiselSoft => "Slmt",
                    },
                ),
            ),
            (
                "bvlS",
                enumerated(
                    "BESl",
                    match style {
                        BevelStyle::Inner => "InrB",
                        BevelStyle::Outer => "OtrB",
                        BevelStyle::Emboss => "Embs",
                        BevelStyle::Pillow => "PlEb",
                    },
                ),
            ),
            ("uglg", Value::Bool(false)),
            ("lagl", unit("#Ang", angle)),
            ("Lald", unit("#Ang", altitude)),
            ("srgR", percent(depth)),
            ("blur", pixels(size)),
            ("bvlD", enumerated("BESs", if up { "In  " } else { "Out " })),
            ("TrnS", linear()),
            ("antialiasGloss", Value::Bool(false)),
            ("Sftn", pixels(soften)),
            ("useShape", Value::Bool(false)),
            ("useTexture", Value::Bool(false)),
        ]);
        add("ebbl", descriptor("ebbl", items));
    }
    let mut items: Vec<(String, Value)> = vec![
        ("Scl ".into(), percent(1.0)),
        ("masterFXSwitch".into(), Value::Bool(true)),
    ];
    for (key, mut list) in slots {
        // One glow or bevel of each kind; a single effect of the others
        // under its own class key, several in a list drawn first on top.
        if key.ends_with("Multi") && list.len() > 1 {
            list.reverse();
            items.push((
                key.into(),
                Value::List(list.into_iter().map(Value::Object).collect()),
            ));
        } else if list.len() == 1 {
            let effect = list.pop().unwrap();
            items.push((effect.class.clone(), Value::Object(effect)));
        } else {
            return None;
        }
    }
    let mut out = 0u32.to_be_bytes().to_vec();
    out.extend(16u32.to_be_bytes());
    out.extend(crate::psd_vector::encode(&Descriptor {
        class: "null".into(),
        items,
    }));
    Some(out)
}
/// A pattern effect's pattern (named by id, written with the file's
/// patterns), scale and phase.
fn pattern_items(paint: &PatternPaint) -> Vec<(&'static str, Value)> {
    let number = |v: f64| Value::Number(v);
    vec![
        (
            "Ptrn",
            Value::Object(descriptor(
                "Ptrn",
                vec![
                    ("Nm  ", Value::Text(format!("{}\0", paint.pattern.name))),
                    ("Idnt", Value::Text(format!("{}\0", paint.pattern.id))),
                ],
            )),
        ),
        ("Scl ", Value::Unit("#Prc".into(), paint.scale * 100.0)),
        ("Algn", Value::Bool(true)),
        (
            "phase",
            Value::Object(descriptor(
                "Pnt ",
                vec![
                    ("Hrzn", number(paint.phase[0])),
                    ("Vrtc", number(paint.phase[1])),
                ],
            )),
        ),
    ]
}
/// The patterns `layers`' effects paint with, once each.
pub(crate) fn patterns_in<'a>(
    layers: impl IntoIterator<Item = &'a Layer>,
) -> Vec<&'a crate::pattern::Pattern> {
    let mut patterns: Vec<&crate::pattern::Pattern> = Vec::new();
    for effect in layers.into_iter().flat_map(|l| &l.effects) {
        if let Paint::Pattern(paint) = &effect.effect.paint
            && !patterns.iter().any(|p| p.id == paint.pattern.id)
        {
            patterns.push(&paint.pattern);
        }
    }
    patterns
}
fn descriptor(class: &str, items: Vec<(&str, Value)>) -> Descriptor {
    Descriptor {
        class: class.into(),
        items: items.into_iter().map(|(k, v)| (k.into(), v)).collect(),
    }
}

/// One enabled effect. `Ok(None)` for keys that are not effects; `Err`
/// names an effect that cannot be drawn.
/// A pattern effect's (or pattern fill layer's) paint: the file's pattern
/// it names, with its scale and phase.
pub(crate) fn pattern_paint(
    effect: &Descriptor,
    patterns: &[crate::pattern::Pattern],
) -> Result<PatternPaint, String> {
    let named = effect.object("Ptrn").ok_or("no pattern")?;
    let id = named.text("Idnt").ok_or("no pattern id")?;
    let pattern = patterns
        .iter()
        .find(|p| p.id == id)
        .ok_or("its pattern is not in the file")?;
    let phase = effect
        .object("phase")
        .map(|p| [p.number("Hrzn"), p.number("Vrtc")].map(|v| v.unwrap_or(0.0)))
        .unwrap_or_default();
    let paint = PatternPaint {
        pattern: pattern.clone(),
        scale: effect
            .number("Scl ")
            .map_or(1.0, |v| v / 100.0)
            .clamp(0.01, 10.0),
        phase: phase.map(|v| {
            if v.is_finite() {
                v.clamp(-1e6, 1e6)
            } else {
                0.0
            }
        }),
    };
    Ok(paint)
}
fn read(
    effect: &Descriptor,
    global_angle: f64,
    approximated: &mut Vec<String>,
    patterns: &[crate::pattern::Pattern],
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
        "ChFX" => "satin",
        "patternFill" => "pattern overlay",
        _ => return Ok(None),
    };
    let blend = blend(effect.enumerated("Md  ").unwrap_or("Nrml")).ok_or(format!(
        "{name} with blend mode {}",
        effect.enumerated("Md  ").unwrap_or_default()
    ))?;
    let opacity = percent("Opct").unwrap_or(1.0) as f32;
    if name == "satin" && !linear(effect.object("MpgS")) {
        approximated.push("satin contour".into());
    }
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
        "pattern overlay" => Kind::PatternOverlay,
        "satin" => Kind::Satin {
            offset: offset(),
            size,
            invert: effect.flag("Invr").unwrap_or(true),
        },
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
        ("pattern overlay", _) | ("stroke", Some("Ptrn")) => {
            Paint::Pattern(pattern_paint(effect, patterns).map_err(|e| format!("{name}: {e}"))?)
        }
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
    pair: u32,
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
                pair,
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
/// Gaussian-like blur with standard deviation `sigma` pixels.
pub(crate) fn gaussian(
    values: &mut [f32],
    w: usize,
    h: usize,
    sigma: f64,
    cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    blur(values, w, h, sigma / BLUR_SIGMA, cancel)
}
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
        Kind::ColorOverlay | Kind::GradientOverlay | Kind::PatternOverlay => vec![1.0; w * h],
        Kind::Satin {
            offset,
            size,
            invert,
        } => {
            let mut a = shift(&alpha.values, w, h, offset, 0.0);
            let mut b = shift(&alpha.values, w, h, offset.map(|v| -v), 0.0);
            blur(&mut a, w, h, size, cancel)?;
            blur(&mut b, w, h, size, cancel)?;
            a.iter()
                .zip(&b)
                .map(|(a, b)| {
                    let differ = (a - b).abs().min(1.0);
                    if invert { 1.0 - differ } else { differ }
                })
                .collect()
        }
        Kind::Bevel {
            highlight,
            style,
            technique,
            depth,
            size,
            soften,
            light,
            up,
            ..
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
                pair: 0,
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
        Paint::Color(_) | Paint::Pattern(_) => None,
    };
    for (i, (&c, out)) in coverage.iter().zip(pixels.chunks_exact_mut(4)).enumerate() {
        if c <= 0.0 {
            continue;
        }
        let (x, y) = (rect[0] + i % w, rect[1] + i / w);
        let [r, g, b, a] = match (&effect.paint, &gradient) {
            (_, Some(painter)) => painter(x, y),
            (Paint::Color(color), _) => [color[0], color[1], color[2], 255],
            (Paint::Pattern(pattern), _) => pattern.at(x, y),
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
                    pair: 0,
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
        assert!(doc.needs_expansion());
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
    #[ignore = "timing probe; run with --release --ignored --nocapture"]
    fn probe_stroke_end_time_with_a_drop_shadow() {
        let side = 4096;
        let mut doc = Document::new(side, side);
        {
            let mut writer = doc.layers[0].pixels.writer();
            for i in 0..side * side {
                if (i % side) % 7 != 0 {
                    writer.rgba_mut(i).copy_from_slice(&[200, 40, 40, 255]);
                }
            }
        }
        doc.layers[0].effects = vec![LayerEffect::new(Effect::preset("Drop Shadow").unwrap())];
        let wake: Wake = std::sync::Arc::new(|| {});
        expand(&doc, false, &|| false).unwrap();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(5)
            .copy_from_slice(&[0, 0, 0, 255]);
        let start = std::time::Instant::now();
        expand(&doc.clone(), false, &|| false).unwrap();
        let blocking = start.elapsed();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(6)
            .copy_from_slice(&[0, 0, 0, 255]);
        let start = std::time::Instant::now();
        let (_, _, waiting) = expand_for_display(&doc, &wake);
        let background = start.elapsed();
        eprintln!(
            "stroke end on {side}x{side}: blocking {blocking:?}, background {background:?} (waiting {waiting})"
        );
    }

    #[test]
    fn the_canvas_shows_ready_drawings_and_draws_the_rest_in_the_background() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let woken = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        let wake: Wake = std::sync::Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let settle = |doc: &Document| {
            for _ in 0..500 {
                let (shown, drawings, waiting) = expand_for_display(doc, &wake);
                if !waiting {
                    return (shown, drawings);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            panic!("background drawing never finished");
        };
        let mut doc = doc_with(vec![overlay()]);
        // Nothing drawn yet: the layer shows without effects until ready.
        // (A worker fast enough to finish before the call returns is shown
        // at once.)
        let (first, _, waiting) = expand_for_display(&doc, &wake);
        assert!(waiting);
        assert!(
            [1, 2].contains(&first.layers.len()),
            "{}",
            first.layers.len()
        );
        let (ready, drawn) = settle(&doc);
        if first.layers.len() == 2 {
            assert!(first.layers[1].pixels.shares(&ready.layers[1].pixels));
        }
        // The drawing is stored just before the worker wakes the display.
        for _ in 0..500 {
            if woken.load(Ordering::Relaxed) >= 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(woken.load(Ordering::Relaxed) >= 1);
        let direct = expand(&doc, false, &|| false).unwrap();
        assert_eq!(ready.layers.len(), direct.layers.len());
        assert!(ready.layers[1].pixels.shares(&direct.layers[1].pixels));
        // A changed layer keeps its previous drawing on screen meanwhile.
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[255, 0, 0, 255]);
        let (meanwhile, previous, waiting) = expand_for_display(&doc, &wake);
        if waiting {
            assert_eq!(previous, drawn);
            assert!(meanwhile.layers[1].pixels.shares(&ready.layers[1].pixels));
        }
        let (updated, redrawn) = settle(&doc);
        assert_ne!(redrawn, drawn);
        assert_eq!(updated.layers[1].pixels.rgba(2), [0, 0, 255, 255]);
        // Opacity and blending reuse the drawing: no wait.
        doc.layers[0].effects[0].effect.opacity = 0.5;
        let (faded, _, waiting) = expand_for_display(&doc, &wake);
        assert!(!waiting);
        assert!(faded.layers[1].pixels.shares(&updated.layers[1].pixels));
        assert_eq!(faded.layers[1].opacity, 0.5);
    }

    /// Effects redrawn around changed pixels match drawing them whole.
    #[test]
    fn redrawing_around_changes_matches_drawing_whole() {
        let side = 1000;
        let mut doc = Document::new(side, side);
        let paint = |doc: &mut Document, rect: [usize; 4], rgba: [u8; 4]| {
            let mut writer = doc.layers[0].pixels.writer();
            for y in rect[1]..rect[3] {
                for x in rect[0]..rect[2] {
                    writer.rgba_mut(y * side + x).copy_from_slice(&rgba);
                }
            }
        };
        paint(&mut doc, [20, 20, 120, 90], [200, 40, 40, 255]);
        paint(&mut doc, [700, 700, 900, 900], [200, 40, 40, 160]);
        let mut shadow = Effect::preset("Drop Shadow").unwrap();
        shadow.kind = Kind::DropShadow {
            offset: [6.0, 4.0],
            spread: 0.2,
            size: 9.0,
            knockout: true,
        };
        let mut stroke = Effect::preset("Stroke").unwrap();
        stroke.kind = Kind::Stroke {
            size: 3.0,
            position: Position::Center,
        };
        let mut glow = Effect::preset("Inner Glow").unwrap();
        if let Kind::InnerGlow { size, .. } = &mut glow.kind {
            *size = 7.0;
        }
        let mut bevel = Effect::bevel_presets(1);
        if let Kind::Bevel { size, soften, .. } = &mut bevel[0].kind {
            (*size, *soften) = (6.0, 2.0);
        }
        doc.layers[0].effects = [shadow, stroke, glow, bevel[0].clone()]
            .into_iter()
            .map(LayerEffect::new)
            .collect();
        let pixels = |doc: &Document| -> Vec<Vec<u8>> {
            let layers = expand(doc, false, &|| false).unwrap().layers;
            layers
                .iter()
                .map(|l| l.pixels.region(side, [0, 0, side, side]))
                .collect()
        };
        let ids = |doc: &Document| -> Vec<_> {
            let layers = expand(doc, false, &|| false).unwrap().layers;
            layers.iter().map(|l| l.id).collect()
        };
        let drawn = ids(&doc);
        // A stroke across one shape and an erased corner of the other.
        paint(&mut doc, [100, 60, 170, 75], [10, 200, 10, 255]);
        paint(&mut doc, [700, 700, 730, 730], [0, 0, 0, 0]);
        let partial = pixels(&doc);
        // Updated in place rather than drawn anew.
        assert_eq!(ids(&doc), drawn);
        doc.layers[0].effect_cache = Cache::default();
        let whole = pixels(&doc);
        assert_eq!(partial.len(), whole.len());
        for (a, b) in partial.iter().zip(&whole) {
            let worst = a
                .iter()
                .zip(b)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(worst <= 2, "differs by {worst}");
        }
    }

    #[test]
    fn undo_finds_the_previous_drawing_ready() {
        let mut doc = doc_with(vec![overlay()]);
        let before = doc.clone();
        let (_, first) = expand_tracked(&doc, false, &|| false).unwrap();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[255, 0, 0, 255]);
        expand_tracked(&doc, false, &|| false).unwrap();
        let wake: Wake = std::sync::Arc::new(|| {});
        let (_, shown, waiting) = expand_for_display(&before, &wake);
        assert!(!waiting);
        assert_eq!(shown, first);
    }

    #[test]
    fn project_memory_counts_effect_drawings_once() {
        let doc = doc_with(vec![overlay()]);
        let bare = crate::snapshot_memory::Registry::new(&doc).live_bytes();
        expand(&doc, false, &|| false).unwrap();
        let drawn = crate::snapshot_memory::Registry::new(&doc).live_bytes();
        let mut bytes = 0;
        doc.layers[0]
            .effect_cache
            .visit_allocations(|_, size| bytes += size);
        assert!(bytes > 0);
        assert_eq!(drawn - bare, bytes);
        // A clone shares the cache, so the drawing is not counted twice.
        let mut registry = crate::snapshot_memory::Registry::new(&doc);
        registry.add_document(&doc.clone());
        assert_eq!(registry.live_bytes(), drawn);
    }

    #[test]
    fn effect_drawings_share_one_budget_and_are_released() {
        let overlay_doc = || doc_with(vec![overlay()]);
        let (a, b) = (overlay_doc(), overlay_doc());
        let (_, first) = expand_tracked(&a, false, &|| false).unwrap();
        let (_, second) = expand_tracked(&b, false, &|| false).unwrap();
        let held = |doc: &Document| doc.layers[0].effect_cache.0.lock().unwrap().len();
        assert_eq!((held(&a), held(&b)), (1, 1));
        let bytes = a.layers[0].effect_cache.0.lock().unwrap()[0].bytes;
        assert!(bytes > 0);
        // Over budget, the least recently used drawing goes first.
        let live = [
            a.layers[0].effect_cache.0.clone(),
            b.layers[0].effect_cache.0.clone(),
        ];
        evict(&live, second[0], bytes);
        assert_eq!((held(&a), held(&b)), (0, 1));
        assert!(first != second);
        // Switching effects off releases the drawing.
        let mut b = b;
        b.layers[0].effects[0].enabled = false;
        expand(&b, false, &|| false).unwrap();
        assert_eq!(held(&b), 0);
    }

    #[test]
    fn displays_see_drawings_made_elsewhere() {
        let mut doc = doc_with(vec![overlay()]);
        let (_, shown) = expand_tracked(&doc, false, &|| false).unwrap();
        let (_, again) = expand_tracked(&doc, false, &|| false).unwrap();
        assert_eq!(shown, again);
        // An export (or preview, or color pick) redraws after an edit; the
        // canvas then gets the cached drawing but must still notice it is new.
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[255, 0, 0, 255]);
        doc.composite_transparent();
        let (_, now) = expand_tracked(&doc, false, &|| false).unwrap();
        assert_ne!(now, shown);
    }

    #[test]
    fn effects_survive_saving_and_damaged_settings_are_rejected() {
        let mut shadow = Effect::preset("Drop Shadow").unwrap();
        shadow.blend = BlendMode::Screen;
        let mut bevel = Effect::bevel_presets(1).map(LayerEffect::new).to_vec();
        bevel[0].enabled = false;
        let mut effects = vec![LayerEffect::new(shadow), overlay()];
        effects.append(&mut bevel);
        effects.push(LayerEffect::new(
            Effect::preset("Gradient Overlay").unwrap(),
        ));
        let doc = doc_with(effects);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        // Saved under the header older builds refuse, so they cannot drop
        // the styles on resave; the project loader reads it back.
        assert!(bytes.starts_with(crate::tile_project::STYLE_MAGIC));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        assert!(
            crate::tile_project::decode(&bytes[crate::tile_project::MAGIC.len()..]).is_ok()
                == (doc.version == 1)
        );
        assert_eq!(loaded.layers[0].effects, doc.layers[0].effects);
        assert_eq!(loaded.composite_transparent(), doc.composite_transparent());
        // Out-of-range settings from a damaged file fail validation.
        let mut bad = doc.clone();
        if let Kind::DropShadow { size, .. } = &mut bad.layers[0].effects[0].effect.kind {
            *size = f64::NAN;
        }
        assert!(bad.validate().is_err());
        let mut many = doc;
        many.layers[0].effects = vec![overlay(); crate::document::MAX_EFFECTS + 1];
        assert!(many.validate().is_err());
    }

    #[test]
    fn pattern_overlay_repeats_its_pattern_inside_the_layer() {
        let pattern = crate::pattern::Pattern::new(
            "Two".into(),
            "two".into(),
            2,
            1,
            vec![255, 0, 0, 255, 0, 0, 255, 255],
        )
        .unwrap();
        let mut effect = Effect::preset("Pattern Overlay").unwrap();
        effect.paint = Paint::Pattern(PatternPaint::new(pattern));
        let doc = doc_with(vec![LayerEffect::new(effect)]);
        let pixels = doc.composite_transparent();
        // The layer covers pixel 1, where the pattern (from the canvas
        // origin) is blue; nothing is drawn beside it.
        assert_eq!(&pixels[4..8], &[0, 0, 255, 255]);
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        assert_eq!(loaded.layers[0].effects, doc.layers[0].effects);
    }

    #[test]
    fn gradient_stops_edit_in_location_order_and_survive_psd() {
        let mut gradient = Gradient::between([0; 3], [255; 3], 90.0);
        gradient.set_color_stops(&[(1.0, [255; 3]), (0.0, [0; 3]), (0.5, [255, 0, 0])]);
        assert_eq!(gradient.color_stops()[1], (0.5, [255, 0, 0]));
        assert_eq!(gradient.sample(0.5), [255, 0, 0, 255]);
        gradient.set_opacity_stops(&[(1.0, 0.0), (0.0, 1.0)]);
        assert_eq!(gradient.sample(1.0)[3], 0);
        gradient.set_style(1);
        gradient.set_reverse(true);
        gradient.set_scale(0.5);
        gradient.validate().unwrap();
        let back = crate::psd_vector::gradient(&Descriptor {
            class: "GrFl".into(),
            items: gradient.psd_items(),
        })
        .unwrap();
        assert_eq!(back.color_stops(), gradient.color_stops());
        assert_eq!(back.opacity_stops(), gradient.opacity_stops());
        assert_eq!((back.style(), back.reverse(), back.scale()), (1, true, 0.5));
    }

    #[test]
    fn satin_shades_where_moved_copies_differ() {
        let mut effect = Effect::preset("Satin").unwrap();
        effect.kind = Kind::Satin {
            offset: [8.0, 0.0],
            size: 0.0,
            invert: false,
        };
        let mut alpha = square(64, [16, 16, 48, 48]);
        let plain = coverage(&effect, &mut alpha, &|| false).unwrap();
        // Copies moved 8 px apart sideways agree in the middle, differ
        // within 8 px of the left and right edges, and not above or below.
        let at = |c: &[f32], x: usize, y: usize| c[y * 64 + x];
        assert_eq!(at(&plain, 32, 32), 0.0);
        assert_eq!(at(&plain, 18, 32), 1.0);
        assert_eq!(at(&plain, 45, 32), 1.0);
        assert_eq!(at(&plain, 32, 18), 0.0);
        effect.kind = Kind::Satin {
            offset: [8.0, 0.0],
            size: 0.0,
            invert: true,
        };
        let inverted = coverage(&effect, &mut alpha, &|| false).unwrap();
        assert_eq!(at(&inverted, 32, 32), 1.0);
        assert_eq!(at(&inverted, 18, 32), 0.0);
        // Drawn inside the layer, clipped to it.
        assert_eq!(effect.place(), Place::Inside);
    }

    /// Two layers in a folder carrying a drop shadow and an overlay.
    fn folder_doc() -> Document {
        let (w, h) = (600, 200);
        let mut doc = Document::new(w, h);
        let square = |name: &str, x0: usize, color: [u8; 4]| {
            let mut layer = Layer::raster(name.into(), w, h);
            layer.depth = 1;
            let mut writer = layer.pixels.writer();
            for y in 40..90 {
                for x in x0..x0 + 50 {
                    writer.rgba_mut(y * w + x).copy_from_slice(&color);
                }
            }
            layer
        };
        let mut folder = Layer::new_folder("F".into());
        folder.effects = vec![
            LayerEffect::new(Effect::preset("Drop Shadow").unwrap()),
            overlay(),
        ];
        doc.layers = vec![
            square("A", 40, [255, 0, 0, 255]),
            square("B", 400, [0, 255, 0, 128]),
            folder,
        ];
        doc.active = 0;
        doc.validate().unwrap();
        doc
    }
    fn fresh(doc: &Document) -> Document {
        let mut fresh = doc.clone();
        for layer in &mut fresh.layers {
            layer.effect_cache = Cache::default();
        }
        expand(&fresh, false, &|| false).unwrap()
    }
    fn close(a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 2)
    }

    #[test]
    fn folder_effects_follow_contents_and_update_around_changes() {
        let mut doc = folder_doc();
        let expanded = expand(&doc, false, &|| false).unwrap();
        let stack: Vec<_> = expanded
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.depth, l.clipped))
            .collect();
        assert_eq!(
            stack,
            [
                ("F's Drop Shadow", 0, false),
                ("A", 1, false),
                ("B", 1, false),
                ("F", 0, false),
                ("F's Color Overlay", 0, true),
            ]
        );
        assert_eq!(expanded.layers[expanded.active].name, "A");
        let rendered = expanded.composite_transparent();
        let at = |x: usize, y: usize| &rendered[(y * 600 + x) * 4..][..4];
        // The overlay recolors both layers; the shadow falls beside them.
        assert_eq!(at(60, 60), [0, 0, 255, 255]);
        // B is half transparent: blue, over the shadow beneath it.
        assert!(at(420, 60)[..2] == [0, 0] && at(420, 60)[2] > 0);
        assert!(at(92, 93)[3] > 0 && at(20, 20)[3] == 0);
        // A change inside the folder updates its contents around the change
        // only, and matches drawing everything afresh.
        let cache = doc.layers[2].effect_cache.clone();
        let content = || cache.1.lock().unwrap().last().unwrap().pixels.clone();
        let before = content();
        doc.layers[1]
            .pixels
            .writer()
            .rgba_mut(150 * 600 + 500)
            .copy_from_slice(&[0, 0, 0, 255]);
        let updated = expand(&doc, false, &|| false).unwrap();
        let after = content();
        assert!(after.shares_tile(&before, 0) && !after.shares_tile(&before, 1));
        assert!(close(
            &updated.composite_transparent(),
            &fresh(&doc).composite_transparent()
        ));
        // A hidden folder draws nothing, and showing it again is ready.
        doc.layers[2].visible = false;
        assert_eq!(expand(&doc, false, &|| false).unwrap().layers.len(), 3);
    }

    #[test]
    fn folder_effects_draw_in_the_background_on_the_canvas() {
        let wake: Wake = std::sync::Arc::new(|| {});
        let settle = |doc: &Document| {
            for _ in 0..1000 {
                let (shown, _, waiting) = expand_for_display(doc, &wake);
                if !waiting {
                    return shown;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            panic!("background drawing never finished");
        };
        let mut doc = folder_doc();
        let (first, _, waiting) = expand_for_display(&doc, &wake);
        assert!(waiting);
        assert_eq!(first.layers.len(), 3);
        let shown = settle(&doc);
        assert!(close(
            &shown.composite_transparent(),
            &fresh(&doc).composite_transparent()
        ));
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(150 * 600 + 100)
            .copy_from_slice(&[0, 0, 0, 255]);
        let shown = settle(&doc);
        assert!(close(
            &shown.composite_transparent(),
            &fresh(&doc).composite_transparent()
        ));
    }

    #[test]
    fn merging_a_folder_keeps_its_effects_and_look() {
        let mut doc = folder_doc();
        let before = doc.composite_transparent();
        doc.active = 2;
        doc.merge_folder().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].effects.len(), 2);
        assert!(close(&doc.composite_transparent(), &before));
    }

    #[test]
    fn folder_effects_survive_saving() {
        let doc = folder_doc();
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        assert_eq!(loaded.layers[2].effects, doc.layers[2].effects);
        assert_eq!(loaded.composite_transparent(), doc.composite_transparent());
    }

    #[test]
    fn reduced_fill_inside_effects_apply_soft_coverage_once() {
        // Independent normal-blend oracle: local red fill, then green and
        // blue overlays, finally the source alpha, mask and layer opacity.
        for fill in [0.0f32, 0.25, 0.5, 0.9999, 1.0] {
            for green in [0.0f32, 0.5, 1.0] {
                for blue in [0.0f32, 0.5, 1.0] {
                    let mut doc = Document::new(1, 1);
                    let layer = &mut doc.layers[0];
                    layer.pixels = vec![255, 0, 0, 128].into();
                    layer.fill = fill;
                    layer.opacity = 0.5;
                    layer.mask = Some(crate::layer_mask::LayerMask {
                        id: crate::document::LayerId::new(),
                        enabled: true,
                        linked: true,
                        coverage: crate::layer_mask::Mask::new(1, 1, 200).unwrap(),
                    });
                    layer.effects = [(green, [0, 255, 0]), (blue, [0, 0, 255])]
                        .into_iter()
                        .map(|(opacity, color)| {
                            LayerEffect::new(Effect {
                                kind: Kind::ColorOverlay,
                                paint: Paint::Color(color),
                                blend: BlendMode::Normal,
                                opacity,
                            })
                        })
                        .collect();
                    let weights = [
                        fill * (1.0 - green) * (1.0 - blue),
                        green * (1.0 - blue),
                        blue,
                    ];
                    let alpha: f32 = weights.iter().sum();
                    let mut expected = [0u8; 4];
                    if alpha > 0.0 {
                        for c in 0..3 {
                            expected[c] = (weights[c] / alpha * 255.0).round() as u8;
                        }
                        expected[3] = (alpha * 128.0 * (200.0 / 255.0) * 0.5).round() as u8;
                    }
                    let actual = doc.composite_transparent();
                    for c in 0..4 {
                        assert!(
                            actual[c].abs_diff(expected[c]) <= 1,
                            "fill={fill}, green={green}, blue={blue}: {actual:?} != {expected:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn reduced_fill_combination_tracks_cache_edits_masks_clipping_and_exports() {
        let mut doc = Document::new(513, 2);
        doc.layers[0].fill = 0.5;
        doc.layers[0].effects = vec![overlay()];
        for i in [1, 512] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(i)
                .copy_from_slice(&[255, 0, 0, 128]);
        }
        let first = expand(&doc, false, &|| false).unwrap();
        assert_eq!(first.layers[0].pixels.rgba(1), [0, 0, 255, 128]);
        let repeat = expand(&doc, false, &|| false).unwrap();
        assert!(first.layers[0].pixels.shares(&repeat.layers[0].pixels));
        let old = doc.clone();
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[0; 4]);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[255, 0, 0, 64]);
        let updated = expand(&doc, false, &|| false).unwrap();
        let mut fresh = doc.clone();
        fresh.layers[0].effect_cache = Cache::default();
        assert_eq!(
            updated.composite_transparent(),
            fresh.composite_transparent()
        );
        assert_eq!(old.composite_transparent(), first.composite_transparent());
        // Same geometry with a new effect opacity must recombine, not merely
        // re-dress a cached layer; mask changes must invalidate it too.
        doc.layers[0].effects[0].effect.opacity = 0.5;
        doc.version = 2;
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(513, 2, 128).unwrap(),
        });
        let expected = doc.composite_transparent();
        let dir = tempfile::tempdir().unwrap();
        let native = dir.path().join("soft.dudel");
        crate::storage::save_project(&native, &doc).unwrap();
        assert_eq!(
            crate::storage::load_project(&native)
                .unwrap()
                .composite_transparent(),
            expected
        );
        for mode in [
            crate::psd_export::Effects::Editable,
            crate::psd_export::Effects::AsLayers,
        ] {
            let (written, _) = crate::psd_export::written(&doc, mode, &|| false).unwrap();
            assert_eq!(written.composite_transparent(), expected);
            let path = dir.path().join("soft.psd");
            let bytes =
                crate::psd_export::encode(&doc, mode, &std::sync::atomic::AtomicBool::new(false))
                    .unwrap();
            std::fs::write(&path, bytes).unwrap();
            let loaded = crate::psd_import::import(&path, &|| false)
                .unwrap()
                .document;
            for (a, b) in loaded.composite_transparent().iter().zip(&expected) {
                assert!(a.abs_diff(*b) <= 2, "PSD {mode:?}: {a} != {b}");
            }
        }
        // A combined owner still clips to its base and retains its folder depth.
        let mut base = Layer::raster("base".into(), 513, 2);
        base.pixels
            .writer()
            .rgba_mut(2)
            .copy_from_slice(&[255, 255, 255, 255]);
        doc.layers[0].clipped = true;
        doc.layers.insert(0, base);
        doc.active = 1;
        let clipped = doc.composite_transparent();
        assert_eq!(&clipped[512 * 4..512 * 4 + 4], &[0; 4]);
        doc.group_active().unwrap();
        let expanded = expand(&doc, false, &|| false).unwrap();
        assert_eq!(
            expanded.layers[expanded.active].depth,
            doc.layers[doc.active].depth
        );
    }

    #[test]
    fn fill_fades_the_layer_but_not_its_effects() {
        // At no fill only the overlay shows, at full strength.
        let mut doc = doc_with(vec![overlay()]);
        doc.layers[0].fill = 0.0;
        assert!(doc.needs_expansion());
        assert_eq!(&doc.composite_transparent()[4..8], &[0, 0, 255, 255]);
        // Without effects fill works like opacity.
        let mut plain = doc_with(Vec::new());
        plain.layers[0].fill = 0.5;
        let alpha = plain.composite_transparent()[7];
        assert!((127..=128).contains(&alpha), "{alpha}");
        // Saved and reopened.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&doc, &mut bytes).unwrap();
        assert!(bytes.starts_with(crate::tile_project::STYLE_MAGIC));
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        assert_eq!(loaded.layers[0].fill, 0.0);
        // Edited as a layer property; folders have none, nor out-of-range values.
        use crate::document_edits::{Edit, Property, Target};
        let fill = |doc: &Document, index: usize, value: f32| {
            Edit::Property {
                target: Target::Identity(doc.layers[index].id),
                property: Property::Fill(value),
            }
            .prepare(doc)
            .map(|(updated, _)| updated.layers[index].fill)
        };
        assert_eq!(fill(&doc, 0, 0.25), Ok(0.25));
        assert!(fill(&doc, 0, 1.5).is_err());
        let folders = folder_doc();
        assert!(fill(&folders, 2, 0.5).is_err());
        let mut bad = doc.clone();
        bad.layers[0].fill = f32::NAN;
        assert!(bad.validate().is_err());
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
                    object(
                        "ChFX",
                        vec![
                            ("enab", Value::Bool(true)),
                            ("Clr ", red.clone()),
                            ("Invr", Value::Bool(false)),
                            ("lagl", unit("#Ang", 0.0)),
                            ("Dstn", unit("#Pxl", 6.0)),
                            ("blur", unit("#Pxl", 2.0)),
                        ],
                    ),
                ),
                (
                    "patternFill".into(),
                    object("patternFill", vec![("enab", Value::Bool(true))]),
                ),
                (
                    "SoFi".into(),
                    object("SoFi", vec![("enab", Value::Bool(false)), ("Clr ", red)]),
                ),
            ],
        };
        let mut data = vec![0, 0, 0, 0, 0, 0, 0, 16];
        data.extend(crate::psd_vector::encode(&settings));
        let mut effects = parse(&data, 90.0, &[]).unwrap();
        // A pattern overlay whose pattern the file lacks cannot be drawn.
        assert_eq!(
            effects.skipped,
            vec!["pattern overlay: no pattern".to_string()]
        );
        assert_eq!(effects.list.len(), 3);
        let Kind::Satin {
            offset,
            size,
            invert,
        } = effects.list[1].kind
        else {
            panic!("satin is read")
        };
        assert!((offset[0] + 6.0).abs() < 1e-9 && offset[1].abs() < 1e-9);
        assert_eq!((size, invert), (2.0, false));
        effects.list.remove(1);
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
        assert!(effects.disabled.is_empty(), "never added, not present");
        // With the master switch off, every effect is kept switched off,
        // with no notes about how it would draw.
        let mut settings = settings;
        settings
            .items
            .push(("masterFXSwitch".into(), Value::Bool(false)));
        let mut data = vec![0, 0, 0, 0, 0, 0, 0, 16];
        data.extend(crate::psd_vector::encode(&settings));
        let effects = parse(&data, 90.0, &[]).unwrap();
        assert!(effects.list.is_empty() && effects.skipped.is_empty());
        let names: Vec<_> = effects.disabled.iter().map(Effect::name).collect();
        assert_eq!(names, ["Drop Shadow", "Satin", "Stroke"]);
        let editable = effects.editable().unwrap();
        assert!(editable.iter().all(|e| !e.enabled));
    }
}
