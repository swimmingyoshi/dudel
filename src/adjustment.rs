//! Adjustment layers: a color change applied to everything below them (or,
//! clipped, to their clipping group), faded by their opacity and mask. The
//! compositor calls `Adjustment::apply` on the backdrop's straight color.
//!
//! The curve shapes follow Photoshop as measured by psd-tools (MIT,
//! github.com/psd-tools/psd-tools, `composite/adjustments.py`): levels,
//! natural cubic curves, exposure in linear light with gamma 2.2, the
//! brightness and contrast curves, and hue/saturation in HSL.
//! Copyright (c) 2019 Kota Yamaguchi; see `LICENSE-psd-tools` for the
//! permission notice, which ships with source and binary packages.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One channel of a levels adjustment, in 0..=255 with gamma as a factor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LevelsChannel {
    pub input: [f32; 2],
    pub output: [f32; 2],
    pub gamma: f32,
}
/// What an adjustment layer does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Settings {
    Invert,
    Posterize {
        levels: u8,
    },
    Threshold {
        level: u8,
    },
    /// Index 0 applies to all channels after 1..=3 (red, green, blue).
    Levels {
        channels: Vec<Option<LevelsChannel>>,
    },
    /// (channel: 0 all after 1..=3 red, green, blue; [input, output] points).
    Curves {
        channels: Vec<(u8, Vec<[u8; 2]>)>,
    },
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
        legacy: bool,
    },
    /// Exposure acts in linear light; `encoding` is the document's tone
    /// curve gamma (2.2 for sRGB, 1.75 for Photoshop's default gray).
    Exposure {
        exposure: f32,
        offset: f32,
        gamma: f32,
        #[serde(default = "srgb_gamma")]
        encoding: f32,
    },
    /// Hue in degrees, saturation and lightness in -100..=100. `colorize`
    /// replaces hue and saturation; `ranges` are trapezoids of hue degrees.
    HueSaturation {
        colorize: Option<[f32; 3]>,
        master: [f32; 3],
        ranges: Vec<([f32; 4], [f32; 3])>,
    },
    /// Luminance mapped through color stops (location 0..=1, color
    /// 0..=255, midpoint 0..=1 of the segment ending at the stop).
    GradientMap {
        stops: Vec<(f32, [f32; 3], f32)>,
        reverse: bool,
    },
}

fn srgb_gamma() -> f32 {
    2.2
}
/// Settings plus the lookup tables drawn from them, built once.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "Settings", into = "Settings")]
pub(crate) struct Adjustment(Arc<Prepared>);
struct Prepared {
    settings: Settings,
    key: usize,
    /// Per channel, 256 samples over 0..=255, for curve-like adjustments.
    tables: Option<[[f32; 256]; 3]>,
    /// Color by luminance step, for gradient maps.
    map: Option<Box<[[f32; 3]; 256]>>,
}
impl TryFrom<Settings> for Adjustment {
    type Error = String;

    fn try_from(settings: Settings) -> Result<Self, Self::Error> {
        settings.validate()?;
        Ok(Self::new(settings))
    }
}
impl From<Adjustment> for Settings {
    fn from(adjustment: Adjustment) -> Self {
        adjustment.0.settings.clone()
    }
}
impl PartialEq for Adjustment {
    fn eq(&self, other: &Self) -> bool {
        self.0.settings == other.0.settings
    }
}
impl std::fmt::Debug for Adjustment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.settings.fmt(f)
    }
}

impl Adjustment {
    pub(crate) fn new(settings: Settings) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{settings:?}").hash(&mut hasher);
        // Never 0, which display keys use for "no adjustment".
        let key = (hasher.finish() as usize) | 1;
        let tables = settings.curve().map(|curve| {
            let mut tables = [[0.0; 256]; 3];
            for (channel, table) in tables.iter_mut().enumerate() {
                for (i, value) in table.iter_mut().enumerate() {
                    *value = (curve(channel, i as f32 / 255.0) * 255.0).clamp(0.0, 255.0);
                }
            }
            tables
        });
        let map = match &settings {
            Settings::GradientMap { stops, reverse } => {
                let mut map = Box::new([[0.0; 3]; 256]);
                for (i, color) in map.iter_mut().enumerate() {
                    let t = i as f32 / 255.0;
                    *color = ramp(stops, if *reverse { 1.0 - t } else { t });
                }
                Some(map)
            }
            _ => None,
        };
        Self(Arc::new(Prepared {
            settings,
            key,
            tables,
            map,
        }))
    }
    /// Changes whenever the settings do; never 0.
    pub(crate) fn key(&self) -> usize {
        self.0.key
    }
    pub(crate) fn name(&self) -> &'static str {
        match self.0.settings {
            Settings::Invert => "Invert",
            Settings::Posterize { .. } => "Posterize",
            Settings::Threshold { .. } => "Threshold",
            Settings::Levels { .. } => "Levels",
            Settings::Curves { .. } => "Curves",
            Settings::BrightnessContrast { .. } => "Brightness/Contrast",
            Settings::Exposure { .. } => "Exposure",
            Settings::HueSaturation { .. } => "Hue/Saturation",
            Settings::GradientMap { .. } => "Gradient Map",
        }
    }
    /// The adjusted straight color, channels in 0..=255.
    pub(crate) fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        if let Some(tables) = &self.0.tables {
            return std::array::from_fn(|i| lookup(&tables[i], c[i]));
        }
        if let Some(map) = &self.0.map {
            let luminance = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
            return map[luminance.round().clamp(0.0, 255.0) as usize];
        }
        match &self.0.settings {
            Settings::Invert => c.map(|v| 255.0 - v),
            Settings::Posterize { levels } => {
                let levels = f32::from((*levels).max(2));
                c.map(|v| {
                    ((v.round() / 256.0 * levels).floor() / (levels - 1.0) * 255.0).min(255.0)
                })
            }
            Settings::Threshold { level } => {
                let luminance = (0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]).round();
                [if luminance >= f32::from(*level) {
                    255.0
                } else {
                    0.0
                }; 3]
            }
            Settings::HueSaturation {
                colorize,
                master,
                ranges,
            } => hue_saturation(c, *colorize, *master, ranges),
            _ => c,
        }
    }
    /// Reject settings a damaged project could hold.
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.0.settings.validate()
    }
}

impl Settings {
    /// Check structural bounds before building lookup tables from file data.
    fn validate(&self) -> Result<(), String> {
        let finite = |values: &[f32]| values.iter().all(|v| v.is_finite());
        let ok = match self {
            Settings::Invert | Settings::Posterize { .. } | Settings::Threshold { .. } => true,
            Settings::Levels { channels } => {
                channels.len() <= 4
                    && channels.iter().flatten().all(|c| {
                        finite(&[c.input[0], c.input[1], c.output[0], c.output[1], c.gamma])
                            && c.input
                                .iter()
                                .chain(&c.output)
                                .all(|v| (0.0..=255.0).contains(v))
                            && (0.01..=9.99).contains(&c.gamma)
                    })
            }
            Settings::Curves { channels } => {
                channels.len() <= 4
                    && channels.iter().all(|(channel, points)| {
                        *channel <= 3
                            && (2..=64).contains(&points.len())
                            && points.windows(2).all(|p| p[0][0] < p[1][0])
                    })
            }
            Settings::BrightnessContrast {
                brightness,
                contrast,
                ..
            } => {
                finite(&[*brightness, *contrast])
                    && (-150.0..=150.0).contains(brightness)
                    && (-100.0..=100.0).contains(contrast)
            }
            Settings::Exposure {
                exposure,
                offset,
                gamma,
                encoding,
            } => {
                finite(&[*exposure, *offset, *gamma, *encoding])
                    && (1.0..=3.0).contains(encoding)
                    && (-20.0..=20.0).contains(exposure)
                    && (-0.5..=0.5).contains(offset)
                    && (0.01..=9.99).contains(gamma)
            }
            Settings::HueSaturation {
                colorize,
                master,
                ranges,
            } => {
                let hsl = |v: &[f32; 3]| {
                    finite(v)
                        && (-180.0..=360.0).contains(&v[0])
                        && v[1..].iter().all(|v| (-100.0..=100.0).contains(v))
                };
                colorize.as_ref().is_none_or(hsl)
                    && hsl(master)
                    && ranges.len() <= 6
                    && ranges.iter().all(|(r, v)| finite(r) && hsl(v))
            }
            Settings::GradientMap { stops, .. } => {
                let mut last = 0.0;
                !stops.is_empty()
                    && stops.len() <= 256
                    && stops.iter().all(|(at, color, midpoint)| {
                        let ok = finite(color)
                            && color.iter().all(|v| (0.0..=255.0).contains(v))
                            && (last..=1.0).contains(at)
                            && (0.05..=0.95).contains(midpoint);
                        last = *at;
                        ok
                    })
            }
        };
        if ok {
            Ok(())
        } else {
            Err("Invalid adjustment settings".into())
        }
    }
}

/// Color at `t` between stops, bending each segment toward its midpoint.
fn ramp(stops: &[(f32, [f32; 3], f32)], t: f32) -> [f32; 3] {
    let first = stops[0];
    if t <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.0 {
            let span = b.0 - a.0;
            let mut u = if span > 0.0 { (t - a.0) / span } else { 1.0 };
            if (b.2 - 0.5).abs() > 1e-6 {
                u = u.powf(0.5f32.ln() / b.2.ln());
            }
            return std::array::from_fn(|c| a.1[c] + (b.1[c] - a.1[c]) * u);
        }
    }
    stops[stops.len() - 1].1
}
fn lookup(table: &[f32; 256], v: f32) -> f32 {
    let x = v.clamp(0.0, 255.0);
    let i = (x as usize).min(254);
    table[i] + (table[i + 1] - table[i]) * (x - i as f32)
}

impl Settings {
    /// For adjustments that act on each channel alone, the curve for
    /// `channel` (0 red, 1 green, 2 blue) over 0..=1.
    #[allow(clippy::type_complexity)]
    fn curve(&self) -> Option<Box<dyn Fn(usize, f32) -> f32 + '_>> {
        match self {
            Settings::Levels { channels } => {
                let level = |c: Option<&Option<LevelsChannel>>, v: f32| match c {
                    Some(Some(c)) => levels(c, v),
                    _ => v,
                };
                Some(Box::new(move |i, v| {
                    level(channels.first(), level(channels.get(i + 1), v))
                }))
            }
            Settings::Curves { channels } => {
                let splines: Vec<(u8, Spline)> = channels
                    .iter()
                    .map(|(c, points)| (*c, Spline::new(points)))
                    .collect();
                Some(Box::new(move |i, v| {
                    let apply = |channel: u8, v: f32| {
                        splines
                            .iter()
                            .find(|(c, _)| *c == channel)
                            .map_or(v, |(_, s)| s.at(v))
                    };
                    apply(0, apply(i as u8 + 1, v))
                }))
            }
            Settings::BrightnessContrast {
                brightness,
                contrast,
                legacy: true,
            } => {
                // Photoshop CS2 and before: a straight shift, then a line
                // through mid gray.
                let (b, c) = (*brightness / 255.0, *contrast / 100.0);
                let slope = if c >= 0.0 {
                    1.0 / (1.0 - c * 0.99)
                } else {
                    1.0 + c
                };
                Some(Box::new(move |_, v| {
                    ((v + b - 0.5) * slope + 0.5).clamp(0.0, 1.0)
                }))
            }
            Settings::BrightnessContrast {
                brightness,
                contrast,
                legacy: false,
            } => Some(Box::new(brightness_contrast(*brightness, *contrast))),
            Settings::Exposure {
                exposure,
                offset,
                gamma,
                encoding,
            } => {
                let (scale, offset, gamma) = (exposure.exp2(), *offset, gamma.clamp(0.01, 9.99));
                let encoding = encoding.clamp(1.0, 3.0);
                Some(Box::new(move |_, v| {
                    let linear = (v.powf(encoding) * scale + offset).clamp(0.0, 1.0);
                    linear.powf(1.0 / gamma).powf(1.0 / encoding)
                }))
            }
            _ => None,
        }
    }
}

fn levels(c: &LevelsChannel, v: f32) -> f32 {
    let [black, white] = c.input.map(|x| x / 255.0);
    let span = if (white - black).abs() > 1e-6 {
        white - black
    } else {
        1.0
    };
    let t = ((v - black) / span)
        .clamp(0.0, 1.0)
        .powf(1.0 / c.gamma.clamp(0.01, 9.99));
    let [low, high] = c.output.map(|x| x / 255.0);
    (low + t * (high - low)).clamp(0.0, 1.0)
}

/// A natural cubic spline through curve points, flat beyond the ends.
struct Spline {
    x: Vec<f32>,
    y: Vec<f32>,
    /// Second derivatives at the points.
    m: Vec<f32>,
}
impl Spline {
    fn new(points: &[[u8; 2]]) -> Self {
        let x: Vec<f32> = points.iter().map(|p| f32::from(p[0]) / 255.0).collect();
        let y: Vec<f32> = points.iter().map(|p| f32::from(p[1]) / 255.0).collect();
        Self::through(x, y)
    }
    fn through(x: Vec<f32>, y: Vec<f32>) -> Self {
        let n = x.len();
        let mut m = vec![0.0; n];
        if n > 2 {
            // Tridiagonal system for the inner second derivatives.
            let mut c = vec![0.0; n];
            let mut d = vec![0.0; n];
            for i in 1..n - 1 {
                let (h0, h1) = (x[i] - x[i - 1], x[i + 1] - x[i]);
                let a = h0 / 6.0;
                let b = (h0 + h1) / 3.0;
                let rhs = (y[i + 1] - y[i]) / h1 - (y[i] - y[i - 1]) / h0;
                let denominator = b - a * c[i - 1];
                c[i] = (h1 / 6.0) / denominator;
                d[i] = (rhs - a * d[i - 1]) / denominator;
            }
            for i in (1..n - 1).rev() {
                m[i] = d[i] - c[i] * m[i + 1];
            }
        }
        Self { x, y, m }
    }
    fn at(&self, v: f32) -> f32 {
        let n = self.x.len();
        let v = v.clamp(self.x[0], self.x[n - 1]);
        let i = self.x.windows(2).position(|w| v <= w[1]).unwrap_or(n - 2);
        let h = self.x[i + 1] - self.x[i];
        if h <= 0.0 {
            return self.y[i];
        }
        let (a, b) = ((self.x[i + 1] - v) / h, (v - self.x[i]) / h);
        (a * self.y[i]
            + b * self.y[i + 1]
            + ((a * a * a - a) * self.m[i] + (b * b * b - b) * self.m[i + 1]) * h * h / 6.0)
            .clamp(0.0, 1.0)
    }
}

/// Photoshop's brightness/contrast (CS3 and later), as fitted by psd-tools:
/// contrast bends a natural spline through (0,0), (63,63±25c), (191,191∓25c)
/// and (255,255); brightness adds a bump, applied along the diagonal.
fn brightness_contrast(brightness: f32, contrast: f32) -> impl Fn(usize, f32) -> f32 {
    let (b, c) = (brightness / 150.0, contrast / 100.0);
    let delta = 25.0 / 255.0;
    let contrast = Spline::through(
        vec![0.0, 63.0 / 255.0, 191.0 / 255.0, 1.0],
        vec![
            0.0,
            63.0 / 255.0 - c * delta,
            191.0 / 255.0 + c * delta,
            1.0,
        ],
    );
    let bump = move |t: f32| {
        let p = |a: f32, r: f32| a * t.powf(r);
        let magnitude = b.abs();
        let h = 0.5
            * (magnitude * (p(1.65, 0.35) + p(-1.0, 10.0))
                + (1.0 - magnitude) * (p(1.96, 0.4) + p(1.0, 4.0))
                + p(1.0, 1.25));
        b * t * (1.0 - t) * h
    };
    // The bump is added at 45°: sample the rotated curve and interpolate.
    const SAMPLES: usize = 512;
    let curve: Vec<(f32, f32)> = (0..=SAMPLES)
        .map(|k| {
            let t = k as f32 / SAMPLES as f32;
            (t - bump(t), contrast.at(t) + bump(t))
        })
        .collect();
    move |_, v: f32| {
        let i = curve.partition_point(|(x, _)| *x < v);
        if i == 0 {
            return curve[0].1.clamp(0.0, 1.0);
        }
        if i > SAMPLES {
            return curve[SAMPLES].1.clamp(0.0, 1.0);
        }
        let ((x0, y0), (x1, y1)) = (curve[i - 1], curve[i]);
        let t = if x1 > x0 { (v - x0) / (x1 - x0) } else { 0.0 };
        (y0 + (y1 - y0) * t).clamp(0.0, 1.0)
    }
}

fn hue_saturation(
    c: [f32; 3],
    colorize: Option<[f32; 3]>,
    master: [f32; 3],
    ranges: &[([f32; 4], [f32; 3])],
) -> [f32; 3] {
    let lighten = |v: [f32; 3], l: f32| {
        if l > 0.0 {
            v.map(|x| x * (1.0 - l) + l)
        } else {
            v.map(|x| x * (1.0 + l))
        }
    };
    let saturate = |s: f32, by: f32| {
        if by > 0.0 {
            s / (1.0 - by + 0.01)
        } else {
            s * (1.0 + by)
        }
        .clamp(0.0, 1.0)
    };
    let rgb = c.map(|v| v / 255.0);
    if let Some([hue, saturation, lightness]) = colorize {
        let [_, _, l] = to_hsl(rgb);
        let l = lighten([l; 3], lightness / 100.0)[0];
        return from_hsl([(hue / 360.0).rem_euclid(1.0), saturation / 100.0, l]).map(|v| v * 255.0);
    }
    let rgb = lighten(rgb, master[2] / 100.0);
    let [h, s, l] = to_hsl(rgb);
    // Per-range changes weighted by how far inside each hue range this is.
    let (mut hue, mut saturation, mut lightness) = (0.0f32, 0.0f32, 0.0f32);
    for (range, change) in ranges {
        let weight = range_weight(h * 360.0, *range);
        hue += change[0] / 360.0 * weight;
        lightness = (lightness + change[2] / 100.0 * weight).clamp(-1.0, 1.0);
        let by = change[1] / 100.0;
        let share = if by > 0.0 {
            1.0 - (1.0 - weight).powf(1.5 / (1.0 - by.powi(4) + 0.05))
        } else {
            weight
        };
        saturation = (saturation + by * share).clamp(-1.0, 1.0);
    }
    // Range lightness acts in HSV and scales with saturation, so grays,
    // which have no hue, keep their value.
    let [_, sv, v] = hsl_to_hsv([h, s, l]);
    let (sv, v) = if lightness >= 0.0 {
        (sv * (1.0 - lightness), v)
    } else {
        let inverse = 1.0 / (sv + 1e-3);
        (
            1.0 + (inverse - 1.0) / (lightness.abs() - inverse + 1e-3),
            v * (1.0 + lightness * sv),
        )
    };
    let [_, s, l] = hsv_to_hsl([h, sv.clamp(0.0, 1.0), v.clamp(0.0, 1.0)]);
    let h = (h + hue + master[0] / 360.0).rem_euclid(1.0);
    let s = saturate(saturate(s, master[1] / 100.0), saturation);
    from_hsl([h, s, l]).map(|v| (v * 255.0).clamp(0.0, 255.0))
}
fn hsl_to_hsv([h, s, l]: [f32; 3]) -> [f32; 3] {
    let v = l + s * l.min(1.0 - l);
    [h, if v > 0.0 { 2.0 * (1.0 - l / v) } else { 0.0 }, v]
}
fn hsv_to_hsl([h, s, v]: [f32; 3]) -> [f32; 3] {
    let l = v * (1.0 - s / 2.0);
    let m = l.min(1.0 - l);
    [h, if m > 0.0 { (v - l) / m } else { 0.0 }, l]
}
/// Trapezoid: 0 outside [a, d], rising over [a, b], 1 over [b, c], falling
/// over [c, d], in degrees around the hue circle.
fn range_weight(hue: f32, [a, b, c, d]: [f32; 4]) -> f32 {
    let around = |v: f32| (v - a).rem_euclid(360.0);
    let (h, b, c, d) = (around(hue), around(b), around(c), around(d));
    if h <= b {
        if b > 0.0 { h / b } else { 1.0 }
    } else if h <= c {
        1.0
    } else if h <= d && d > c {
        (d - h) / (d - c)
    } else {
        0.0
    }
}
fn to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= 1e-6 {
        return [0.0, 0.0, l];
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h / 6.0, s, l]
}
fn from_hsl([h, s, l]: [f32; 3]) -> [f32; 3] {
    if s <= 0.0 {
        return [l; 3];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0)]
}

/// Read a PSD adjustment block (`key`), for an RGB (`gray` false) or
/// grayscale document. `Ok(None)` for keys that are not adjustments; `Err`
/// names an adjustment Dudel cannot apply.
pub(crate) fn from_psd(key: &[u8], data: &[u8], gray: bool) -> Result<Option<Settings>, String> {
    let mut r = Reader(data);
    let damaged = || "Damaged PSD adjustment".to_string();
    Ok(Some(match key {
        b"nvrt" => Settings::Invert,
        b"post" => Settings::Posterize {
            levels: r.u16().ok_or_else(damaged)?.clamp(2, 255) as u8,
        },
        b"thrs" => Settings::Threshold {
            level: r.u16().ok_or_else(damaged)?.clamp(1, 255) as u8,
        },
        b"levl" => {
            r.u16().ok_or_else(damaged)?;
            let mut records = Vec::new();
            for _ in 0..4 {
                let v: Vec<u16> = (0..5)
                    .map(|_| r.u16())
                    .collect::<Option<_>>()
                    .ok_or_else(damaged)?;
                let channel = LevelsChannel {
                    input: [f32::from(v[0].min(255)), f32::from(v[1].min(255))],
                    output: [f32::from(v[2].min(255)), f32::from(v[3].min(255))],
                    gamma: (f32::from(v[4]) / 100.0).clamp(0.01, 9.99),
                };
                let identity = channel.input == [0.0, 255.0]
                    && channel.output == [0.0, 255.0]
                    && (channel.gamma - 1.0).abs() < 1e-6;
                records.push((!identity).then_some(channel));
            }
            Settings::Levels {
                // Grayscale documents keep their only channel in record 1.
                channels: if gray { vec![records[1]] } else { records },
            }
        }
        b"curv" => Settings::Curves {
            channels: curves(data, gray).ok_or_else(damaged)?,
        },
        b"brit" => {
            let brightness = r.u16().ok_or_else(damaged)? as i16;
            let contrast = r.u16().ok_or_else(damaged)? as i16;
            Settings::BrightnessContrast {
                brightness: f32::from(brightness).clamp(-150.0, 150.0),
                contrast: f32::from(contrast).clamp(-100.0, 100.0),
                legacy: true,
            }
        }
        b"expA" => {
            r.u16().ok_or_else(damaged)?;
            let mut f = || r.u32().map(f32::from_bits).filter(|v| v.is_finite());
            let (exposure, offset, gamma) = (
                f().ok_or_else(damaged)?,
                f().ok_or_else(damaged)?,
                f().ok_or_else(damaged)?,
            );
            Settings::Exposure {
                exposure: exposure.clamp(-20.0, 20.0),
                offset: offset.clamp(-0.5, 0.5),
                gamma: gamma.clamp(0.01, 9.99),
                encoding: if gray { 1.75 } else { 2.2 },
            }
        }
        b"hue2" => {
            if gray {
                return Err("hue/saturation on grayscale".into());
            }
            r.u16().ok_or_else(damaged)?;
            let colorize = r.u8().ok_or_else(damaged)? != 0;
            r.u8().ok_or_else(damaged)?;
            let mut triple =
                || -> Option<[f32; 3]> { Some([r.i16()?, r.i16()?, r.i16()?].map(f32::from)) };
            let colorization = triple().ok_or_else(damaged)?;
            let master = triple().ok_or_else(damaged)?;
            let mut ranges = Vec::new();
            for _ in 0..6 {
                let range = [r.i16(), r.i16(), r.i16(), r.i16()];
                let range: Vec<f32> = range
                    .into_iter()
                    .map(|v| v.map(f32::from))
                    .collect::<Option<_>>()
                    .ok_or_else(damaged)?;
                let change = [r.i16(), r.i16(), r.i16()];
                let change: Vec<f32> = change
                    .into_iter()
                    .map(|v| v.map(f32::from))
                    .collect::<Option<_>>()
                    .ok_or_else(damaged)?;
                if change.iter().any(|v| *v != 0.0) {
                    ranges.push((
                        [range[0], range[1], range[2], range[3]],
                        [change[0], change[1], change[2]],
                    ));
                }
            }
            let clamp = |v: [f32; 3]| {
                [
                    v[0].clamp(-180.0, 360.0),
                    v[1].clamp(-100.0, 100.0),
                    v[2].clamp(-100.0, 100.0),
                ]
            };
            Settings::HueSaturation {
                colorize: colorize.then_some(clamp(colorization)),
                master: clamp(master),
                ranges: ranges.into_iter().map(|(r, c)| (r, clamp(c))).collect(),
            }
        }
        b"blnc" => return Err("color balance".into()),
        b"vibA" => return Err("vibrance".into()),
        b"mixr" => return Err("channel mixer".into()),
        b"selc" => return Err("selective color".into()),
        b"grdm" => gradient_map(data).ok_or_else(damaged)??,
        b"phfl" => return Err("photo filter".into()),
        b"blwh" => return Err("black & white".into()),
        b"clrL" => return Err("color lookup".into()),
        _ => return Ok(None),
    }))
}
/// `grdm`: version (3 adds an interpolation method), reverse and dither
/// flags, a name, color stops, then transparency stops (unused by a map).
/// `Some(Err)` names stop colors Dudel cannot read.
fn gradient_map(data: &[u8]) -> Option<Result<Settings, String>> {
    let mut r = Reader(data);
    let version = r.u16()?;
    let reverse = r.u8()? != 0;
    r.u8()?;
    if version >= 3 {
        r.take(4)?;
    }
    let name = r.u32()? as usize;
    r.take(name.checked_mul(2)?)?;
    let count = usize::from(r.u16()?);
    if count == 0 || count > 256 {
        return None;
    }
    let mut stops = Vec::with_capacity(count);
    for _ in 0..count {
        let location = (r.u32()? as f32 / 4096.0).clamp(0.0, 1.0);
        let midpoint = (r.u32()? as f32 / 100.0).clamp(0.05, 0.95);
        let mode = r.u16()?;
        let values = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        r.u16()?;
        let color = match mode {
            0 => [0, 1, 2].map(|i| f32::from(values[i]) / 257.0),
            8 => [255.0 * (1.0 - f32::from(values[0]) / 10000.0); 3],
            _ => return Some(Err("gradient map with non-RGB stop colors".into())),
        };
        stops.push((location, color, midpoint));
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(Ok(Settings::GradientMap { stops, reverse }))
}
/// Brightness/contrast saved by CS3 and later (`CgEd`), which replaces the
/// legacy `brit` values.
pub(crate) fn brightness_contrast_descriptor(settings: &crate::psd_vector::Descriptor) -> Settings {
    Settings::BrightnessContrast {
        brightness: settings.number("Brgh").unwrap_or(0.0).clamp(-150.0, 150.0) as f32,
        contrast: settings.number("Cntr").unwrap_or(0.0).clamp(-100.0, 100.0) as f32,
        legacy: settings.flag("useLegacy").unwrap_or(false),
    }
}
/// `curv`: the version 1 table, then optionally a `Crv ` block listing each
/// channel by ID (which wins).
fn curves(data: &[u8], gray: bool) -> Option<Vec<(u8, Vec<[u8; 2]>)>> {
    let mut r = Reader(data);
    r.u8()?;
    r.u16()?;
    let bitmap = r.u32()?;
    let read_points = |r: &mut Reader| -> Option<Vec<[u8; 2]>> {
        let count = usize::from(r.u16()?);
        if !(2..=64).contains(&count) {
            return None;
        }
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            let output = r.u16()?.min(255) as u8;
            let input = r.u16()?.min(255) as u8;
            points.push([input, output]);
        }
        points.sort_by_key(|p| p[0]);
        points.dedup_by_key(|p| p[0]);
        (points.len() >= 2).then_some(points)
    };
    let mut channels = Vec::new();
    for bit in 0..32 {
        if bitmap & (1 << bit) != 0 {
            let points = read_points(&mut r)?;
            channels.push((bit as u8, points));
        }
    }
    if r.0.len() >= 4 && &r.0[..4] == b"Crv " {
        r.take(4)?;
        r.u16()?;
        let count = r.u32()?;
        let mut extra = Vec::new();
        for _ in 0..count.min(32) {
            let id = r.u16()?;
            extra.push((id as u8, read_points(&mut r)?));
        }
        channels = extra;
    }
    // Grayscale documents keep their only channel as 1; Dudel applies it to
    // all three, like the RGB composite channel 0.
    Some(
        channels
            .into_iter()
            .filter_map(|(id, points)| {
                let id = if gray {
                    if id == 1 { 0 } else { return None }
                } else {
                    id
                };
                let identity = points == [[0, 0], [255, 255]];
                (id <= 3 && !identity).then_some((id, points))
            })
            .collect(),
    )
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.0.len() {
            return None;
        }
        let (data, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(data)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }
    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(settings: Settings, c: [f32; 3]) -> [f32; 3] {
        Adjustment::new(settings).apply(c).map(|v| v.round())
    }

    #[test]
    fn simple_adjustments_map_colors() {
        assert_eq!(
            apply(Settings::Invert, [0.0, 100.0, 255.0]),
            [255.0, 155.0, 0.0]
        );
        assert_eq!(
            apply(Settings::Posterize { levels: 2 }, [10.0, 127.0, 200.0]),
            [0.0, 0.0, 255.0]
        );
        assert_eq!(
            apply(Settings::Threshold { level: 128 }, [200.0, 200.0, 200.0]),
            [255.0; 3]
        );
        assert_eq!(
            apply(Settings::Threshold { level: 128 }, [255.0, 0.0, 0.0]),
            [0.0; 3]
        );
        let levels = Settings::Levels {
            channels: vec![Some(LevelsChannel {
                input: [50.0, 200.0],
                output: [0.0, 255.0],
                gamma: 1.0,
            })],
        };
        assert_eq!(apply(levels, [50.0, 140.0, 200.0]), [0.0, 153.0, 255.0]);
        // Curves through their points; a straight line stays straight.
        let curves = Settings::Curves {
            channels: vec![
                (0, vec![[0, 0], [128, 200], [255, 255]]),
                (1, vec![[0, 255], [255, 0]]),
            ],
        };
        let out = apply(curves, [128.0, 128.0, 0.0]);
        assert_eq!(out[1], 200.0);
        // Red is inverted first (channel 1), then the composite curve.
        assert!(
            (out[0]
                - apply(
                    Settings::Curves {
                        channels: vec![(0, vec![[0, 0], [128, 200], [255, 255]])]
                    },
                    [127.0; 3]
                )[0])
                .abs()
                <= 1.0
        );
        assert_eq!(out[2], 0.0);
        // Exposure +1 stop doubles linear light.
        let brighter = apply(
            Settings::Exposure {
                exposure: 1.0,
                offset: 0.0,
                gamma: 1.0,
                encoding: 2.2,
            },
            [100.0; 3],
        );
        assert!(
            (brighter[0] - 255.0 * (2.0f32 * (100.0f32 / 255.0).powf(2.2)).powf(1.0 / 2.2)).abs()
                <= 1.0
        );
        // Hue +120 turns red green; colorize sets one hue.
        let hue = apply(
            Settings::HueSaturation {
                colorize: None,
                master: [120.0, 0.0, 0.0],
                ranges: Vec::new(),
            },
            [255.0, 0.0, 0.0],
        );
        assert_eq!(hue, [0.0, 255.0, 0.0]);
        let tinted = apply(
            Settings::HueSaturation {
                colorize: Some([240.0, 100.0, 0.0]),
                master: [0.0; 3],
                ranges: Vec::new(),
            },
            [128.0, 128.0, 128.0],
        );
        assert!(tinted[2] > 200.0 && tinted[0] < 10.0, "{tinted:?}");
        // A gradient map colors by luminance: black to red, white to blue.
        let map = Settings::GradientMap {
            stops: vec![(0.0, [255.0, 0.0, 0.0], 0.5), (1.0, [0.0, 0.0, 255.0], 0.5)],
            reverse: false,
        };
        assert_eq!(apply(map.clone(), [0.0; 3]), [255.0, 0.0, 0.0]);
        assert_eq!(apply(map.clone(), [255.0; 3]), [0.0, 0.0, 255.0]);
        let mid = apply(map, [128.0; 3]);
        assert!((mid[0] - 127.0).abs() <= 1.0 && (mid[2] - 128.0).abs() <= 1.0);
        // Zero brightness and contrast change nothing.
        let same = apply(
            Settings::BrightnessContrast {
                brightness: 0.0,
                contrast: 0.0,
                legacy: false,
            },
            [30.0, 128.0, 220.0],
        );
        assert_eq!(same, [30.0, 128.0, 220.0]);
    }

    /// Three pixels of one layer: red, half-transparent green, empty.
    fn base() -> crate::document::Document {
        let mut doc = crate::document::Document::new(3, 1);
        {
            let mut w = doc.layers[0].pixels.writer();
            w.rgba_mut(0).copy_from_slice(&[255, 0, 0, 255]);
            w.rgba_mut(1).copy_from_slice(&[0, 255, 0, 128]);
        }
        doc
    }
    fn adjustment_layer(settings: Settings) -> crate::document::Layer {
        let mut layer = crate::document::Layer::raster("Adjust".into(), 3, 1);
        layer.adjustment = Some(Adjustment::new(settings));
        layer
    }
    fn pixels(doc: &crate::document::Document) -> Vec<u8> {
        doc.composite_transparent()
    }

    #[test]
    fn adjustment_layers_recolor_what_lies_below() {
        let mut doc = base();
        doc.layers.push(adjustment_layer(Settings::Invert));
        // Colors change, coverage does not.
        assert_eq!(
            pixels(&doc),
            vec![0, 255, 255, 255, 255, 0, 255, 128, 0, 0, 0, 0]
        );
        // Opacity fades the change in.
        doc.layers[1].opacity = 0.5;
        assert_eq!(&pixels(&doc)[..4], &[128, 128, 128, 255]);
        doc.layers[1].opacity = 1.0;
        assert!(!doc.has_effects() && doc.layers[1].has_display_payload([2, 0, 3, 1]));
        // Clipped, it only recolors its base.
        let mut clipped = crate::document::Document::new(3, 1);
        clipped.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[0, 0, 255, 255]);
        clipped.add_layer();
        clipped.layers[1]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[255, 0, 0, 255]);
        let mut invert = adjustment_layer(Settings::Invert);
        invert.clipped = true;
        clipped.layers.push(invert);
        let out = pixels(&clipped);
        assert_eq!(&out[..4], &[0, 255, 255, 255]);
        assert_eq!(&out[4..8], &[0, 0, 255, 255]);
        // A layer clipped to an adjustment layer acts through its mask.
        let mut through = base();
        let mut masked = adjustment_layer(Settings::Invert);
        masked.mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(3, 1, 0).unwrap(),
        });
        masked
            .mask
            .as_mut()
            .unwrap()
            .coverage
            .writer()
            .set(0, 0, 255);
        through.layers.push(masked);
        let mut second = adjustment_layer(Settings::Invert);
        second.clipped = true;
        through.layers.push(second);
        let out = pixels(&through);
        // Pixel 0: inverted twice; pixel 1: masked out, untouched.
        assert_eq!(&out[..8], &[255, 0, 0, 255, 0, 255, 0, 128]);
    }

    #[test]
    fn damaged_adjustments_are_rejected_before_preparing_lookup_tables() {
        for settings in [
            r#"{"GradientMap":{"stops":[],"reverse":false}}"#,
            r#"{"Curves":{"channels":[[0,[]]]}}"#,
            r#"{"Curves":{"channels":[[0,[[0,0]]]]}}"#,
            r#"{"Curves":{"channels":[[0,[[255,255],[0,0]]]]}}"#,
        ] {
            let parsed = std::panic::catch_unwind(|| serde_json::from_str::<Adjustment>(settings));
            assert!(parsed.is_ok(), "damaged settings panicked: {settings}");
            assert!(
                parsed.unwrap().is_err(),
                "accepted damaged settings: {settings}"
            );
        }
    }

    #[test]
    fn folders_scope_adjustments() {
        // In a pass-through folder an adjustment reaches below the folder;
        // in an isolated folder it only changes the folder's contents.
        for (blend, expected) in [
            (crate::blend::BlendMode::PassThrough, [0, 255, 255, 255]),
            (crate::blend::BlendMode::Normal, [255, 0, 0, 255]),
        ] {
            let mut doc = base();
            let mut inside = adjustment_layer(Settings::Invert);
            inside.depth = 1;
            doc.layers.push(inside);
            let mut folder = crate::document::Layer::new_folder("Folder".into());
            folder.blend = blend;
            doc.layers.push(folder);
            doc.validate().unwrap();
            assert_eq!(&pixels(&doc)[..4], &expected, "{blend:?}");
        }
    }

    #[test]
    fn adjustment_layers_save_refuse_paint_and_merge() {
        let mut doc = base();
        doc.layers.push(adjustment_layer(Settings::Levels {
            channels: vec![Some(LevelsChannel {
                input: [0.0, 200.0],
                output: [0.0, 255.0],
                gamma: 1.2,
            })],
        }));
        doc.active = 1;
        assert!(doc.paint_blocker().is_some());
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
        assert_eq!(loaded.layers[1].adjustment, doc.layers[1].adjustment);
        assert_eq!(pixels(&loaded), pixels(&doc));
        // Merging the adjustment down applies it to the layer below once.
        let before = pixels(&doc);
        let mut merged = doc.clone();
        merged.merge_down().unwrap();
        assert_eq!(merged.layers.len(), 1);
        assert_eq!(pixels(&merged), before);
        // Nothing merges into an adjustment layer.
        let mut onto = doc.clone();
        onto.add_layer();
        assert!(onto.merge_down().is_err());
        // Folders cannot be adjustment layers.
        let mut bad = doc;
        bad.layers[1].folder = true;
        assert!(bad.validate_metadata().is_err());
    }

    #[test]
    fn psd_blocks_parse_and_damage_is_rejected() {
        let mut levl = vec![0, 2];
        for record in 0..29u16 {
            let values: [u16; 5] = if record == 0 {
                [10, 240, 0, 255, 150]
            } else {
                [0, 255, 0, 255, 100]
            };
            values.iter().for_each(|v| levl.extend(v.to_be_bytes()));
        }
        let Some(Settings::Levels { channels }) = from_psd(b"levl", &levl, false).unwrap() else {
            panic!()
        };
        assert_eq!(channels[0].unwrap().gamma, 1.5);
        assert!(channels[1..].iter().all(Option::is_none));
        assert!(from_psd(b"levl", &levl[..20], false).is_err());
        // Curves: one composite channel, three points.
        let mut curv = vec![0, 0, 1, 0, 0, 0, 1, 0, 3];
        for (output, input) in [(0u16, 0u16), (200, 128), (255, 255)] {
            curv.extend(output.to_be_bytes());
            curv.extend(input.to_be_bytes());
        }
        let Some(Settings::Curves { channels }) = from_psd(b"curv", &curv, false).unwrap() else {
            panic!()
        };
        assert_eq!(channels, vec![(0, vec![[0, 0], [128, 200], [255, 255]])]);
        assert!(from_psd(b"curv", &curv[..12], false).is_err());
        let mut exposure = vec![0, 1];
        for v in [1.0f32, 0.0, 1.0] {
            exposure.extend(v.to_bits().to_be_bytes());
        }
        assert!(matches!(
            from_psd(b"expA", &exposure, false),
            Ok(Some(Settings::Exposure { .. }))
        ));
        assert!(from_psd(b"blnc", &[], false).is_err());
        // Gradient map v3: method, empty name, two RGB stops.
        let mut grdm = vec![0, 3, 1, 0];
        grdm.extend(b"Perc");
        grdm.extend(0u32.to_be_bytes());
        grdm.extend(2u16.to_be_bytes());
        for (at, rgb) in [(0u32, [0u16, 0, 0]), (4096, [65535, 65535, 65535])] {
            grdm.extend(at.to_be_bytes());
            grdm.extend(50u32.to_be_bytes());
            grdm.extend(0u16.to_be_bytes());
            rgb.iter()
                .chain(&[0, 0])
                .for_each(|v| grdm.extend(v.to_be_bytes()));
        }
        let Some(Settings::GradientMap { stops, reverse }) =
            from_psd(b"grdm", &grdm, false).unwrap()
        else {
            panic!()
        };
        assert!(reverse);
        assert_eq!(stops[1], (1.0, [255.0; 3], 0.5));
        assert!(from_psd(b"grdm", &grdm[..20], false).is_err());
        assert!(from_psd(b"luni", &[], false).unwrap().is_none());
        // Validation rejects out-of-range settings from a damaged project.
        let bad = Adjustment::new(Settings::Exposure {
            exposure: f32::NAN,
            offset: 0.0,
            gamma: 1.0,
            encoding: 2.2,
        });
        assert!(bad.validate().is_err());
        assert!(Adjustment::new(Settings::Invert).validate().is_ok());
    }
}
