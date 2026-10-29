//! Photoshop layer effects ("layer styles"), read from a PSD's effect
//! descriptors and drawn from a layer's alpha. Import turns each effect into
//! its own layer, the way Photoshop's Layer > Layer Style > Create Layers
//! does; the renderers only need alpha so native effects can reuse them.

use crate::blend::BlendMode;
use crate::psd_vector::{Descriptor, Gradient, Value};

/// Where an effect's layer goes relative to the layer it belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// Under the layer: shadows, outer glows, outside strokes.
    Below,
    /// Inside the layer's pixels: overlays, inner effects, inside strokes.
    Inside,
    /// Over the layer, not limited to its pixels: centered strokes.
    Above,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Paint {
    Color([u8; 3]),
    Gradient(Gradient),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Position {
    Outside,
    Inside,
    Center,
}
#[derive(Clone, Copy, Debug, PartialEq)]
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
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Effect {
    pub kind: Kind,
    pub paint: Paint,
    pub blend: BlendMode,
    pub opacity: f32,
}
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
            _ => 0.0,
        };
        reach.ceil() as usize + 2
    }
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
        "ebbl" => return Err("bevel and emboss".into()),
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
    distance: Option<Vec<f32>>,
}
impl Alpha {
    pub(crate) fn new(rect: [usize; 4], values: Vec<f32>) -> Self {
        assert_eq!(values.len(), (rect[2] - rect[0]) * (rect[3] - rect[1]));
        Self {
            rect,
            values,
            distance: None,
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
    fn distance(&mut self, cancel: &impl Fn() -> bool) -> Result<&[f32], String> {
        if self.distance.is_none() {
            let (w, h) = (self.width(), self.height());
            let a = &self.values;
            let to_layer = nearest(w, h, false, |i| a[i] > 0.0, cancel)?;
            let to_gap = nearest(w, h, true, |i| a[i] < FULL, cancel)?;
            let at = |q: Option<usize>| q.map_or(0.0, |q| a[q]);
            let field = (0..w * h)
                .map(|i| {
                    if a[i] <= 0.0 {
                        let (d, q) = to_layer[i];
                        d - 0.5 + (1.0 - at(q))
                    } else if a[i] < FULL {
                        0.5 - a[i]
                    } else {
                        let (d, q) = to_gap[i];
                        0.5 - at(q) - d
                    }
                })
                .collect();
            self.distance = Some(field);
        }
        Ok(self.distance.as_deref().unwrap())
    }
}

/// Coverage treated as complete.
const FULL: f32 = 0.999;
const FAR: f64 = 1e12;

/// Euclidean distance from each pixel to the nearest pixel where `target`
/// holds, and which pixel that is. A one-pixel transparent border surrounds
/// the region (a target when `border`), reported as `None`.
/// Felzenszwalb and Huttenlocher's linear-time transform, keeping indices.
fn nearest(
    w: usize,
    h: usize,
    border: bool,
    target: impl Fn(usize) -> bool,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<(f32, Option<usize>)>, String> {
    let (pw, ph) = (w + 2, h + 2);
    let mut grid = vec![if border { 0.0 } else { FAR }; pw * ph];
    for y in 0..h {
        for x in 0..w {
            grid[(y + 1) * pw + x + 1] = if target(y * w + x) { 0.0 } else { FAR };
        }
    }
    // Column pass: squared distance and row of the nearest target.
    let mut rows = vec![0usize; pw * ph];
    let (mut line, mut out, mut owner) = (Vec::new(), Vec::new(), Vec::new());
    for x in 0..pw {
        if x % 64 == 0 && cancel() {
            return Err("Import canceled".into());
        }
        line.clear();
        line.extend((0..ph).map(|y| grid[y * pw + x]));
        transform(&line, &mut out, &mut owner);
        for y in 0..ph {
            grid[y * pw + x] = out[y];
            rows[y * pw + x] = owner[y];
        }
    }
    let mut result = vec![(0.0, None); w * h];
    for y in 0..h {
        if y % 64 == 0 && cancel() {
            return Err("Import canceled".into());
        }
        transform(&grid[(y + 1) * pw..(y + 2) * pw], &mut out, &mut owner);
        for x in 0..w {
            let (qx, qy) = (owner[x + 1], rows[(y + 1) * pw + owner[x + 1]]);
            let inside = (1..=w).contains(&qx) && (1..=h).contains(&qy);
            result[y * w + x] = (
                out[x + 1].sqrt() as f32,
                inside.then(|| (qy - 1) * w + qx - 1),
            );
        }
    }
    Ok(result)
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
/// blurs approximating a Gaussian. Edges repeat the border value.
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
    let mut line = Vec::new();
    for pass in 0..3 {
        if cancel() {
            return Err("Import canceled".into());
        }
        let radius = ((if pass < m { low } else { low + 2 }) - 1) as usize / 2;
        if radius == 0 {
            continue;
        }
        for y in 0..h {
            line.clear();
            line.extend_from_slice(&values[y * w..(y + 1) * w]);
            box_line(&line, &mut values[y * w..(y + 1) * w], radius);
        }
        for x in 0..w {
            line.clear();
            line.extend((0..h).map(|y| values[y * w + x]));
            let mut column = line.clone();
            box_line(&line, &mut column, radius);
            for (y, v) in column.into_iter().enumerate() {
                values[y * w + x] = v;
            }
        }
    }
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
            let values = alpha.values.clone();
            let distance = alpha.distance(cancel)?;
            let mut shape: Vec<f32> = values
                .iter()
                .zip(distance)
                .map(|(&a, &d)| {
                    let a = if invert { 1.0 - a } else { a };
                    let d = if invert { -d } else { d };
                    if grow > 0.0 {
                        a.max(band(d, f32::NEG_INFINITY, grow as f32))
                    } else {
                        a
                    }
                })
                .collect();
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
                .distance(cancel)?
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
                    .distance(cancel)?
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
                .distance(cancel)?
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
        let d = alpha.distance(&|| false).unwrap().to_vec();
        assert_eq!(d[10 * 20 + 4], 0.5);
        assert_eq!(d[10 * 20 + 2], 2.5);
        assert_eq!(d[10 * 20 + 5], -0.5);
        assert_eq!(d[10 * 20 + 7], -2.5);
        // Partial coverage moves the edge into the pixel.
        let mut soft = square(20, [5, 5, 15, 15]);
        soft.values[10 * 20 + 4] = 0.25;
        let d = soft.distance(&|| false).unwrap();
        assert!((d[10 * 20 + 4] - 0.25).abs() < 1e-6);
        // A layer filling its region still has an edge at the border.
        let mut full = square(4, [0, 0, 4, 4]);
        assert_eq!(full.distance(&|| false).unwrap()[0], -0.5);
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
                    "ebbl".into(),
                    object("ebbl", vec![("enab", Value::Bool(true))]),
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
        assert_eq!(effects.skipped, vec!["bevel and emboss".to_string()]);
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
