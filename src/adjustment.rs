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
//!
//! Color balance, vibrance, black & white, photo filter, channel mixer and
//! selective color are Dudel's own approximations of Photoshop's results:
//! no reference implementation measures them.

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
        /// Clip Studio Paint's curves: the points between the ends pull the
        /// curve towards them (a quadratic B-spline) instead of lying on it.
        #[serde(default)]
        bezier: bool,
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
    /// Clip Studio Paint's Hue/Saturation/Luminosity: whole-number HSV with
    /// hue in degrees, saturation and luminosity in percent.
    HueSaturationLuminosity {
        hue: i32,
        saturation: i32,
        luminosity: i32,
    },
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
    /// Cyan–red, magenta–green and yellow–blue shifts (-100..=100) for the
    /// shadows, midtones and highlights.
    ColorBalance {
        tones: [[f32; 3]; 3],
        preserve_luminosity: bool,
    },
    /// Both in -100..=100; vibrance changes muted colors most.
    Vibrance {
        vibrance: f32,
        saturation: f32,
    },
    /// Gray from how much reds, yellows, greens, cyans, blues and magentas
    /// count (percent, -200..=300), optionally tinted with a color.
    BlackWhite {
        weights: [f32; 6],
        tint: Option<[f32; 3]>,
    },
    /// A colored lens over the image: color 0..=255, density 0..=100.
    PhotoFilter {
        color: [f32; 3],
        density: f32,
        preserve_luminosity: bool,
    },
    /// Each output channel (red, green, blue) as percentages of the input
    /// red, green and blue plus a constant, all -200..=200. Monochrome uses
    /// the first row for all three.
    ChannelMixer {
        rows: [[f32; 4]; 3],
        monochrome: bool,
    },
    /// Cyan, magenta, yellow and black changes (-100..=100) for reds,
    /// yellows, greens, cyans, blues, magentas, whites, neutrals and blacks.
    SelectiveColor {
        colors: [[f32; 4]; 9],
        absolute: bool,
    },
}

fn srgb_gamma() -> f32 {
    2.2
}
/// The kinds offered when adding an adjustment layer, with neutral settings.
pub(crate) fn presets() -> [Settings; 16] {
    let identity = LevelsChannel {
        input: [0.0, 255.0],
        output: [0.0, 255.0],
        gamma: 1.0,
    };
    [
        Settings::Levels {
            channels: vec![Some(identity), None, None, None],
        },
        Settings::Curves {
            channels: vec![(0, vec![[0, 0], [255, 255]])],
            bezier: false,
        },
        Settings::BrightnessContrast {
            brightness: 0.0,
            contrast: 0.0,
            legacy: false,
        },
        Settings::Exposure {
            exposure: 0.0,
            offset: 0.0,
            gamma: 1.0,
            encoding: 2.2,
        },
        Settings::HueSaturation {
            colorize: None,
            master: [0.0; 3],
            ranges: Vec::new(),
        },
        Settings::HueSaturationLuminosity {
            hue: 0,
            saturation: 0,
            luminosity: 0,
        },
        Settings::GradientMap {
            stops: vec![(0.0, [0.0; 3], 0.5), (1.0, [255.0; 3], 0.5)],
            reverse: false,
        },
        Settings::Vibrance {
            vibrance: 0.0,
            saturation: 0.0,
        },
        Settings::ColorBalance {
            tones: [[0.0; 3]; 3],
            preserve_luminosity: true,
        },
        Settings::BlackWhite {
            weights: BLACK_WHITE_DEFAULT,
            tint: None,
        },
        Settings::PhotoFilter {
            color: [236.0, 138.0, 0.0],
            density: 25.0,
            preserve_luminosity: true,
        },
        Settings::ChannelMixer {
            rows: [
                [100.0, 0.0, 0.0, 0.0],
                [0.0, 100.0, 0.0, 0.0],
                [0.0, 0.0, 100.0, 0.0],
            ],
            monochrome: false,
        },
        Settings::SelectiveColor {
            colors: [[0.0; 4]; 9],
            absolute: false,
        },
        Settings::Invert,
        Settings::Posterize { levels: 4 },
        Settings::Threshold { level: 128 },
    ]
}
/// Photoshop's default black & white weights.
pub(crate) const BLACK_WHITE_DEFAULT: [f32; 6] = [40.0, 60.0, 40.0, 60.0, 20.0, 80.0];
/// A curve through `points` ([input, output] in 0..=255), sampled at every
/// input level, as the curves editor draws it.
pub(crate) fn sample_curve(points: &[[u8; 2]], bezier: bool) -> [f32; 256] {
    let curve = Curve::new(points, bezier);
    std::array::from_fn(|i| curve.at(i as f32 / 255.0) * 255.0)
}
/// A curves channel: a spline through its points (Photoshop) or one its
/// inner points pull on (Clip Studio Paint).
enum Curve {
    Spline(Spline),
    Bezier(Vec<[f32; 2]>),
}
impl Curve {
    fn new(points: &[[u8; 2]], bezier: bool) -> Self {
        if bezier {
            Self::Bezier(
                points
                    .iter()
                    .map(|p| p.map(|v| f32::from(v) / 255.0))
                    .collect(),
            )
        } else {
            Self::Spline(Spline::new(points))
        }
    }
    fn at(&self, v: f32) -> f32 {
        match self {
            Self::Spline(spline) => spline.at(v),
            Self::Bezier(points) => bezier_at(points, v),
        }
    }
}
/// A clamped uniform quadratic B-spline over `points` (x ascending): from the
/// first point to the last, a quadratic Bezier between each pair of midpoints
/// of the inner points. Three points make one Bezier, which matched CSP's
/// tone curve on the owner's samples within half a level.
fn bezier_at(points: &[[f32; 2]], v: f32) -> f32 {
    let n = points.len();
    let v = v.clamp(points[0][0], points[n - 1][0]);
    if n == 2 {
        let [[x0, y0], [x1, y1]] = [points[0], points[1]];
        return if x1 > x0 {
            (y0 + (y1 - y0) * (v - x0) / (x1 - x0)).clamp(0.0, 1.0)
        } else {
            y0
        };
    }
    let mid = |i: usize| -> [f32; 2] {
        if i == 0 {
            points[0]
        } else if i == n - 2 {
            points[n - 1]
        } else {
            [
                (points[i][0] + points[i + 1][0]) / 2.0,
                (points[i][1] + points[i + 1][1]) / 2.0,
            ]
        }
    };
    // Segment k runs from mid(k) to mid(k + 1), pulled by points[k + 1].
    let k = (0..n - 2).find(|&k| v <= mid(k + 1)[0]).unwrap_or(n - 3);
    let ([x0, y0], [cx, cy], [x2, y2]) = (mid(k), points[k + 1], mid(k + 1));
    // x(t) = (1-t)^2 x0 + 2t(1-t) cx + t^2 x2: solve for t in 0..=1.
    let a = x0 - 2.0 * cx + x2;
    let b = 2.0 * (cx - x0);
    let c = x0 - v;
    let t = if a.abs() < 1e-7 {
        if b.abs() < 1e-7 { 0.0 } else { -c / b }
    } else {
        (-b + (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a)
    }
    .clamp(0.0, 1.0);
    ((1.0 - t) * (1.0 - t) * y0 + 2.0 * t * (1.0 - t) * cy + t * t * y2).clamp(0.0, 1.0)
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
    pub(crate) fn settings(&self) -> &Settings {
        &self.0.settings
    }
    pub(crate) fn name(&self) -> &'static str {
        self.0.settings.name()
    }
}
impl Settings {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Settings::Invert => "Invert",
            Settings::Posterize { .. } => "Posterize",
            Settings::Threshold { .. } => "Threshold",
            Settings::Levels { .. } => "Levels",
            Settings::Curves { .. } => "Curves",
            Settings::BrightnessContrast { .. } => "Brightness/Contrast",
            Settings::Exposure { .. } => "Exposure",
            Settings::HueSaturation { .. } => "Hue/Saturation",
            Settings::HueSaturationLuminosity { .. } => "Hue/Saturation/Luminosity",
            Settings::GradientMap { .. } => "Gradient Map",
            Settings::ColorBalance { .. } => "Color Balance",
            Settings::Vibrance { .. } => "Vibrance",
            Settings::BlackWhite { .. } => "Black & White",
            Settings::PhotoFilter { .. } => "Photo Filter",
            Settings::ChannelMixer { .. } => "Channel Mixer",
            Settings::SelectiveColor { .. } => "Selective Color",
        }
    }
}
impl Adjustment {
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
            Settings::ColorBalance {
                tones,
                preserve_luminosity,
            } => color_balance(c, tones, *preserve_luminosity),
            Settings::HueSaturationLuminosity {
                hue,
                saturation,
                luminosity,
            } => csp_hsv(c, *hue, *saturation, *luminosity),
            Settings::Vibrance {
                vibrance,
                saturation,
            } => {
                let [h, s, l] = to_hsl(c.map(|v| v / 255.0));
                // Muted colors gain (or lose) the most; grays stay gray.
                let s = saturate(s, vibrance / 100.0 * (1.0 - s));
                let s = saturate(s, saturation / 100.0);
                from_hsl([h, s, l]).map(|v| (v * 255.0).clamp(0.0, 255.0))
            }
            Settings::BlackWhite { weights, tint } => black_white(c, weights, *tint),
            Settings::PhotoFilter {
                color,
                density,
                preserve_luminosity,
            } => {
                let rgb = c.map(|v| v / 255.0);
                let d = density / 100.0;
                // Multiply by the filter color, faded by density.
                let filtered: [f32; 3] =
                    std::array::from_fn(|i| rgb[i] * (1.0 - d + d * color[i] / 255.0));
                let out = if *preserve_luminosity {
                    crate::blend::set_lum(filtered, crate::blend::lum(rgb))
                } else {
                    filtered
                };
                out.map(|v| (v * 255.0).clamp(0.0, 255.0))
            }
            Settings::ChannelMixer { rows, monochrome } => {
                let mix = |row: &[f32; 4]| {
                    ((row[0] * c[0] + row[1] * c[1] + row[2] * c[2]) / 100.0 + row[3] * 2.55)
                        .clamp(0.0, 255.0)
                };
                if *monochrome {
                    [mix(&rows[0]); 3]
                } else {
                    [mix(&rows[0]), mix(&rows[1]), mix(&rows[2])]
                }
            }
            Settings::SelectiveColor { colors, absolute } => selective_color(c, colors, *absolute),
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
            Settings::Curves { channels, .. } => {
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
            Settings::HueSaturationLuminosity {
                hue,
                saturation,
                luminosity,
            } => {
                (-180..=180).contains(hue)
                    && (-100..=100).contains(saturation)
                    && (-100..=100).contains(luminosity)
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
            Settings::ColorBalance { tones, .. } => {
                tones.iter().flatten().all(|v| (-100.0..=100.0).contains(v))
            }
            Settings::Vibrance {
                vibrance,
                saturation,
            } => [vibrance, saturation]
                .iter()
                .all(|v| (-100.0..=100.0).contains(*v)),
            Settings::BlackWhite { weights, tint } => {
                weights.iter().all(|v| (-200.0..=300.0).contains(v))
                    && tint.iter().flatten().all(|v| (0.0..=255.0).contains(v))
            }
            Settings::PhotoFilter { color, density, .. } => {
                color.iter().all(|v| (0.0..=255.0).contains(v)) && (0.0..=100.0).contains(density)
            }
            Settings::ChannelMixer { rows, .. } => {
                rows.iter().flatten().all(|v| (-200.0..=200.0).contains(v))
            }
            Settings::SelectiveColor { colors, .. } => colors
                .iter()
                .flatten()
                .all(|v| (-100.0..=100.0).contains(v)),
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
            Settings::Curves { channels, bezier } => {
                let splines: Vec<(u8, Curve)> = channels
                    .iter()
                    .map(|(c, points)| (*c, Curve::new(points, *bezier)))
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

/// Clip Studio Paint's hue/saturation/luminosity: whole-number HSV on 8-bit
/// color, with hue in 1/0x30000 of a turn, saturation in 1/0x10000 and
/// value in 0..=255, settings in 1/0x8000 (truncated). Luminosity moves
/// value toward white or black (lightening also drains saturation), then
/// saturation, which lowering also darkens a little. Matched CSP's saved
/// preview within its rounding on the owner's +60 / -40 / +20 sample.
fn csp_hsv(c: [f32; 3], hue: i32, saturation: i32, luminosity: i32) -> [f32; 3] {
    // The whole-number round trip would move some colors by a level.
    if hue % 360 == 0 && saturation == 0 && luminosity == 0 {
        return c;
    }
    let [r, g, b] = c.map(|v| i64::from(v.round().clamp(0.0, 255.0) as u8));
    let turn = 0x30000i64;
    let hue = i64::from(hue).rem_euclid(360) * turn / 360;
    let saturation = i64::from(saturation) * 0x8000 / 100;
    let luminosity = i64::from(luminosity) * 0x8000 / 100;
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    let mut s = if max == 0 { 0 } else { delta * 0x10000 / max };
    let mut h = 0;
    if s != 0 {
        let part = |high: i64, low: i64| (high - low) * 0x8000 / delta;
        h = if max == r {
            if g < b { turn - part(b, g) } else { part(g, b) }
        } else if max == g {
            if b < r {
                0x10000 - part(r, b)
            } else {
                0x10000 + part(b, r)
            }
        } else if r < g {
            0x20000 - part(g, r)
        } else {
            0x20000 + part(r, g)
        };
        if h >= turn {
            h -= turn;
        }
    }
    let mut v = max;
    if hue != 0 {
        h += hue;
        if h >= turn {
            h -= turn;
        }
    }
    if luminosity < 0 {
        v = (v - ((v * -luminosity) >> 15)).max(0);
    } else if luminosity > 0 {
        v = ((((255 - v) * luminosity) >> 15) + v).min(255);
        s = (s - ((s * luminosity) >> 15)).max(0);
    }
    if saturation != 0 && s > 0 && v > 0 {
        if saturation < 0 {
            let lowered = (s - ((s * -saturation) >> 15)).max(0);
            v = (v - (((s - lowered) * v) >> 17)).max(0);
            s = lowered;
        } else {
            let mut gain = ((0x10000 - s) * saturation) >> 15;
            let added = (gain * v) >> 16;
            if v + added > 255 {
                gain = gain * (255 - v) / added;
                v = 255;
            } else {
                v += added;
            }
            s = (s + gain).min(0x10000);
        }
    }
    let f = h & 0x7fff;
    let low = ((0x10000 - s) * v) >> 16;
    let falling = ((0x10000 - ((f * s) >> 15)) * v) >> 16;
    let rising = ((0x10000 - (((0x8000 - f) * s) >> 15)) * v) >> 16;
    let [r, g, b] = match h >> 15 {
        0 => [v, rising, low],
        1 => [falling, v, low],
        2 => [low, v, rising],
        3 => [low, falling, v],
        4 => [rising, low, v],
        5 => [v, low, falling],
        _ => [v; 3],
    };
    [r, g, b].map(|v| v as f32)
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

/// Saturation `s` (0..=1) changed by `by` (-1..=1), as hue/saturation does.
fn saturate(s: f32, by: f32) -> f32 {
    if by > 0.0 {
        s / (1.0 - by + 0.01)
    } else {
        s * (1.0 + by)
    }
    .clamp(0.0, 1.0)
}

/// Each tone's shifts weighted by how far the pixel's lightness lies in the
/// shadows, midtones or highlights; the three weights always sum to 1.
fn color_balance(c: [f32; 3], tones: &[[f32; 3]; 3], preserve_luminosity: bool) -> [f32; 3] {
    let rgb = c.map(|v| v / 255.0);
    let [_, _, l] = to_hsl(rgb);
    let shadows = ((1.0 / 3.0 - l) / 0.25 + 0.5).clamp(0.0, 1.0);
    let highlights = ((l - 2.0 / 3.0) / 0.25 + 0.5).clamp(0.0, 1.0);
    let weights = [shadows, 1.0 - shadows - highlights, highlights];
    let mut out = rgb;
    for (tone, weight) in tones.iter().zip(weights) {
        for (v, shift) in out.iter_mut().zip(tone) {
            *v += shift / 100.0 * weight * 0.7;
        }
    }
    let out = out.map(|v| v.clamp(0.0, 1.0));
    let out = if preserve_luminosity {
        crate::blend::set_lum(out, crate::blend::lum(rgb))
    } else {
        out
    };
    out.map(|v| (v * 255.0).clamp(0.0, 255.0))
}

/// Gray as the darkest channel plus the step to the middle one weighted by
/// the two-channel color (yellow, cyan or magenta) and the step to the
/// brightest weighted by its primary, so grays keep their value.
fn black_white(c: [f32; 3], weights: &[f32; 6], tint: Option<[f32; 3]>) -> [f32; 3] {
    let [r, g, b] = c.map(|v| v / 255.0);
    const RED: usize = 0;
    const YELLOW: usize = 1;
    const GREEN: usize = 2;
    const CYAN: usize = 3;
    const BLUE: usize = 4;
    const MAGENTA: usize = 5;
    let (max, mid, min, primary, secondary) = if r >= g && g >= b {
        (r, g, b, RED, YELLOW)
    } else if r >= b && b >= g {
        (r, b, g, RED, MAGENTA)
    } else if g >= r && r >= b {
        (g, r, b, GREEN, YELLOW)
    } else if g >= b && b >= r {
        (g, b, r, GREEN, CYAN)
    } else if b >= r && r >= g {
        (b, r, g, BLUE, MAGENTA)
    } else {
        (b, g, r, BLUE, CYAN)
    };
    let gray =
        (min + (mid - min) * weights[secondary] / 100.0 + (max - mid) * weights[primary] / 100.0)
            .clamp(0.0, 1.0);
    let out = match tint {
        // The tint's hue and saturation at the gray's luminosity.
        Some(tint) => crate::blend::set_lum(tint.map(|v| v / 255.0), gray),
        None => [gray; 3],
    };
    out.map(|v| (v * 255.0).clamp(0.0, 255.0))
}

/// Each color group's ink changes, weighted by how strongly the pixel
/// belongs to the group. Ink is 1 - channel (cyan from red and so on);
/// relative changes scale the ink already there, absolute ones add to it.
fn selective_color(c: [f32; 3], colors: &[[f32; 4]; 9], absolute: bool) -> [f32; 3] {
    let rgb = c.map(|v| v / 255.0);
    let [r, g, b] = rgb;
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let weights = [
        r - g.max(b),
        r.min(g) - b,
        g - r.max(b),
        g.min(b) - r,
        b - r.max(g),
        r.min(b) - g,
        (min - 0.5) * 2.0,
        1.0 - ((max - 0.5).abs() + (min - 0.5).abs()),
        (0.5 - max) * 2.0,
    ];
    let black = 1.0 - max;
    let mut out = rgb;
    for (change, weight) in colors.iter().zip(weights) {
        if weight <= 0.0 {
            continue;
        }
        let k = change[3] / 100.0;
        for i in 0..3 {
            let ink = 1.0 - rgb[i];
            let p = change[i] / 100.0;
            let delta = if absolute { p + k } else { ink * p + black * k };
            out[i] -= ((ink + delta).clamp(0.0, 1.0) - ink) * weight;
        }
    }
    out.map(|v| (v * 255.0).clamp(0.0, 255.0))
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
            bezier: false,
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
        // Photoshop offers these only for color documents.
        b"blnc" | b"vibA" | b"mixr" | b"selc" | b"phfl" | b"blwh" if gray => {
            return Err("color adjustments on grayscale".into());
        }
        b"blnc" => {
            let mut tones = [[0.0; 3]; 3];
            for v in tones.iter_mut().flatten() {
                *v = f32::from(r.i16().ok_or_else(damaged)?).clamp(-100.0, 100.0);
            }
            Settings::ColorBalance {
                tones,
                preserve_luminosity: r.u8().ok_or_else(damaged)? != 0,
            }
        }
        b"vibA" => {
            let d = crate::psd_vector::versioned(data).map_err(|_| damaged())?;
            let value = |key| d.number(key).unwrap_or(0.0).clamp(-100.0, 100.0) as f32;
            Settings::Vibrance {
                vibrance: value("vibrance"),
                saturation: value("Strt"),
            }
        }
        b"blwh" => {
            let d = crate::psd_vector::versioned(data).map_err(|_| damaged())?;
            let keys = ["Rd  ", "Yllw", "Grn ", "Cyn ", "Bl  ", "Mgnt"];
            let weights = std::array::from_fn(|i| {
                d.number(keys[i])
                    .map_or(BLACK_WHITE_DEFAULT[i], |v| v.clamp(-200.0, 300.0) as f32)
            });
            let tint = match d.object("tintColor") {
                Some(color) if d.flag("useTint") == Some(true) => {
                    Some(descriptor_rgb(color).ok_or("black & white with a non-RGB tint")?)
                }
                _ => None,
            };
            Settings::BlackWhite { weights, tint }
        }
        b"phfl" => {
            let version = r.u16().ok_or_else(damaged)?;
            let color = match version {
                // XYZ against D50 with 32768 as 1, as ag-psd (MIT) found.
                3 => {
                    let mut xyz = [0.0; 3];
                    for v in &mut xyz {
                        *v = (r.u32().ok_or_else(damaged)? as i32 as f32 / 32768.0).clamp(0.0, 2.0);
                    }
                    xyz_d50_to_srgb(xyz)
                }
                2 => {
                    let space = r.u16().ok_or_else(damaged)?;
                    let mut v = [0; 4];
                    for v in &mut v {
                        *v = r.u16().ok_or_else(damaged)?;
                    }
                    psd_color(space, [v[0], v[1], v[2]])
                        .ok_or("photo filter with an unsupported color")?
                }
                _ => return Err("photo filter version".into()),
            };
            let density = (r.u32().ok_or_else(damaged)?).min(100) as f32;
            Settings::PhotoFilter {
                color,
                density,
                preserve_luminosity: r.u8().ok_or_else(damaged)? != 0,
            }
        }
        b"mixr" => {
            r.u16().ok_or_else(damaged)?;
            let monochrome = r.u16().ok_or_else(damaged)? != 0;
            let mut rows = [[0.0; 4]; 3];
            // Rows hold red, green, blue, black (unused in RGB) and the
            // constant; monochrome files may stop after the first.
            for (i, row) in rows.iter_mut().enumerate() {
                let values: Option<Vec<i16>> = (0..5).map(|_| r.i16()).collect();
                let values = match values {
                    Some(values) => values,
                    None if monochrome && i > 0 => vec![0; 5],
                    None => return Err(damaged()),
                };
                *row = [values[0], values[1], values[2], values[4]]
                    .map(|v| f32::from(v).clamp(-200.0, 200.0));
            }
            Settings::ChannelMixer { rows, monochrome }
        }
        b"selc" => {
            r.u16().ok_or_else(damaged)?;
            let absolute = r.u16().ok_or_else(damaged)? != 0;
            // The first record is unused.
            r.take(8).ok_or_else(damaged)?;
            let mut colors = [[0.0; 4]; 9];
            for v in colors.iter_mut().flatten() {
                *v = f32::from(r.i16().ok_or_else(damaged)?).clamp(-100.0, 100.0);
            }
            Settings::SelectiveColor { colors, absolute }
        }
        b"grdm" => gradient_map(data).ok_or_else(damaged)??,
        b"clrL" => return Err("color lookup".into()),
        _ => return Ok(None),
    }))
}
/// Whether Photoshop has no exact form for these settings, so `to_psd`
/// writes the closest Photoshop adjustment instead.
pub(crate) fn psd_approximated(settings: &Settings) -> bool {
    matches!(
        settings,
        Settings::HueSaturationLuminosity { .. } | Settings::Curves { bezier: true, .. }
    )
}
/// Photoshop's tagged blocks for an adjustment layer: the inverse of
/// `from_psd`, laid out as Photoshop writes them (as read by psd-tools).
pub(crate) fn to_psd(settings: &Settings) -> Vec<([u8; 4], Vec<u8>)> {
    fn short(out: &mut Vec<u8>, v: f32) {
        out.extend((v.round().clamp(-32768.0, 65535.0) as i32 as u16).to_be_bytes());
    }
    fn pad(mut out: Vec<u8>) -> Vec<u8> {
        out.resize(out.len().next_multiple_of(4), 0);
        out
    }
    fn descriptor(items: Vec<(&str, crate::psd_vector::Value)>) -> Vec<u8> {
        let mut out = 16u32.to_be_bytes().to_vec();
        out.extend(crate::psd_vector::encode(&crate::psd_vector::Descriptor {
            class: "null".into(),
            items: items.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }));
        pad(out)
    }
    use crate::psd_vector::Value;
    let long = |v: f32| Value::Integer(v.round() as i64);
    let mut out = Vec::new();
    let key = match settings {
        Settings::Invert => b"nvrt",
        Settings::Posterize { levels } => {
            short(&mut out, f32::from(*levels));
            short(&mut out, 0.0);
            b"post"
        }
        Settings::Threshold { level } => {
            short(&mut out, f32::from(*level));
            short(&mut out, 0.0);
            b"thrs"
        }
        Settings::Levels { channels } => {
            short(&mut out, 2.0);
            let identity = LevelsChannel {
                input: [0.0, 255.0],
                output: [0.0, 255.0],
                gamma: 1.0,
            };
            for i in 0..29 {
                let c = channels.get(i).copied().flatten().unwrap_or(identity);
                for v in [c.input[0], c.input[1], c.output[0], c.output[1]] {
                    short(&mut out, v.clamp(0.0, 255.0));
                }
                short(&mut out, (c.gamma * 100.0).clamp(1.0, 999.0));
            }
            b"levl"
        }
        Settings::Curves { channels, bezier } => {
            let mut channels: Vec<(u8, Vec<[u8; 2]>)> = channels
                .iter()
                .filter(|(id, _)| *id <= 3)
                .map(|(id, points)| {
                    // Photoshop curves are splines of at most 16 points; a
                    // CSP curve is sampled for the spline to follow it.
                    if !*bezier && points.len() <= 16 {
                        return (*id, points.clone());
                    }
                    let table = sample_curve(points, *bezier);
                    let samples = (0..=8).map(|i| (i * 32).min(255));
                    let points = samples
                        .map(|x| [x as u8, table[x].round().clamp(0.0, 255.0) as u8])
                        .collect();
                    (*id, points)
                })
                .collect();
            channels.sort_by_key(|(id, _)| *id);
            out.push(0);
            short(&mut out, 1.0);
            let bitmap = channels.iter().fold(0u32, |b, (id, _)| b | 1 << id);
            out.extend(bitmap.to_be_bytes());
            let points = |out: &mut Vec<u8>, points: &[[u8; 2]]| {
                short(out, points.len() as f32);
                for [input, output] in points {
                    short(out, f32::from(*output));
                    short(out, f32::from(*input));
                }
            };
            for (_, list) in &channels {
                points(&mut out, list);
            }
            out.extend(b"Crv ");
            short(&mut out, 4.0);
            out.extend((channels.len() as u32).to_be_bytes());
            for (id, list) in &channels {
                short(&mut out, f32::from(*id));
                points(&mut out, list);
            }
            b"curv"
        }
        Settings::BrightnessContrast {
            brightness,
            contrast,
            legacy,
        } => {
            // Photoshop keeps the values in `CgEd`; `brit` holds them only
            // for legacy mode.
            let mut brit = Vec::new();
            let legacy_values = if *legacy {
                [*brightness, *contrast]
            } else {
                [0.0; 2]
            };
            legacy_values.iter().for_each(|v| short(&mut brit, *v));
            short(&mut brit, if *legacy { 127.0 } else { 0.0 });
            brit.extend([0, 0]);
            let cged = descriptor(vec![
                ("Vrsn", Value::Integer(1)),
                ("Brgh", long(*brightness)),
                ("Cntr", long(*contrast)),
                ("means", Value::Integer(127)),
                ("Lab ", Value::Bool(false)),
                ("useLegacy", Value::Bool(*legacy)),
                ("Auto", Value::Bool(false)),
            ]);
            return vec![(*b"brit", brit), (*b"CgEd", cged)];
        }
        Settings::Exposure {
            exposure,
            offset,
            gamma,
            ..
        } => {
            short(&mut out, 1.0);
            for v in [exposure, offset, gamma] {
                out.extend(v.to_bits().to_be_bytes());
            }
            b"expA"
        }
        Settings::HueSaturation {
            colorize,
            master,
            ranges,
        } => write_hue_saturation(&mut out, *colorize, *master, ranges),
        // Clip Studio Paint's HSV change, as Photoshop's HSL master.
        Settings::HueSaturationLuminosity {
            hue,
            saturation,
            luminosity,
        } => write_hue_saturation(
            &mut out,
            None,
            [*hue, *saturation, *luminosity].map(|v| v as f32),
            &[],
        ),
        Settings::GradientMap { stops, reverse } => {
            short(&mut out, 1.0);
            out.extend([u8::from(*reverse), 0]);
            let name: Vec<u16> = "Custom\0".encode_utf16().collect();
            out.extend((name.len() as u32).to_be_bytes());
            name.iter().for_each(|u| out.extend(u.to_be_bytes()));
            short(&mut out, stops.len() as f32);
            for (location, color, midpoint) in stops {
                out.extend(((location * 4096.0).round() as u32).to_be_bytes());
                out.extend(((midpoint * 100.0).round() as u32).to_be_bytes());
                short(&mut out, 0.0);
                color.iter().for_each(|c| short(&mut out, c * 257.0));
                out.extend([0; 4]);
            }
            // Fully opaque from end to end.
            short(&mut out, 2.0);
            for location in [0u32, 4096] {
                out.extend(location.to_be_bytes());
                out.extend(50u32.to_be_bytes());
                short(&mut out, 255.0);
            }
            // Expansion, interpolation, length, mode, random seed, show
            // transparency, vector color, roughness, color model, minimum
            // and maximum color: Photoshop's values for a solid gradient.
            for v in [2u16, 4096, 32, 0] {
                out.extend(v.to_be_bytes());
            }
            out.extend(0u32.to_be_bytes());
            out.extend([0, 0, 0, 1]);
            out.extend(2048u32.to_be_bytes());
            out.extend(3u16.to_be_bytes());
            out.extend([0; 8]);
            out.extend([0x80, 0].repeat(4));
            out.extend([0; 2]);
            b"grdm"
        }
        Settings::ColorBalance {
            tones,
            preserve_luminosity,
        } => {
            tones.iter().flatten().for_each(|v| short(&mut out, *v));
            out.push(u8::from(*preserve_luminosity));
            b"blnc"
        }
        Settings::Vibrance {
            vibrance,
            saturation,
        } => {
            let mut items = Vec::new();
            if vibrance.round() != 0.0 {
                items.push(("vibrance", long(*vibrance)));
            }
            if saturation.round() != 0.0 {
                items.push(("Strt", long(*saturation)));
            }
            return vec![(*b"vibA", descriptor(items))];
        }
        Settings::BlackWhite { weights, tint } => {
            let keys = ["Rd  ", "Yllw", "Grn ", "Cyn ", "Bl  ", "Mgnt"];
            let mut items: Vec<_> = keys
                .iter()
                .zip(weights)
                .map(|(k, w)| (*k, long(*w)))
                .collect();
            // Photoshop's default tint when none is used.
            let color = tint.unwrap_or([225.0, 211.0, 179.0]);
            let rgb = crate::psd_vector::Descriptor {
                class: "RGBC".into(),
                items: ["Rd  ", "Grn ", "Bl  "]
                    .iter()
                    .zip(color)
                    .map(|(k, v)| ((*k).into(), Value::Number(f64::from(v))))
                    .collect(),
            };
            items.push(("useTint", Value::Bool(tint.is_some())));
            items.push(("tintColor", Value::Object(rgb)));
            items.push(("bwPresetKind", Value::Integer(1)));
            items.push(("blackAndWhitePresetFileName", Value::Text("\0".into())));
            return vec![(*b"blwh", descriptor(items))];
        }
        Settings::PhotoFilter {
            color,
            density,
            preserve_luminosity,
        } => {
            // Version 2 with an RGB color (space 0).
            short(&mut out, 2.0);
            short(&mut out, 0.0);
            color.iter().for_each(|c| short(&mut out, c * 257.0));
            short(&mut out, 0.0);
            out.extend((density.round().clamp(0.0, 100.0) as u32).to_be_bytes());
            out.push(u8::from(*preserve_luminosity));
            b"phfl"
        }
        Settings::ChannelMixer { rows, monochrome } => {
            short(&mut out, 1.0);
            short(&mut out, f32::from(u8::from(*monochrome)));
            for [r, g, b, constant] in rows {
                for v in [*r, *g, *b, 0.0, *constant] {
                    short(&mut out, v);
                }
            }
            // The black output row, unused in RGB.
            for v in [0.0, 0.0, 0.0, 100.0, 0.0] {
                short(&mut out, v);
            }
            b"mixr"
        }
        Settings::SelectiveColor { colors, absolute } => {
            short(&mut out, 1.0);
            short(&mut out, f32::from(u8::from(*absolute)));
            out.extend([0; 8]);
            colors.iter().flatten().for_each(|v| short(&mut out, *v));
            b"selc"
        }
    };
    vec![(*key, pad(out))]
}
/// `hue2`: version 2, the colorize flag, colorization and master (hue,
/// saturation, lightness), then six ranges (reds, yellows, greens, cyans,
/// blues, magentas): the given ranges first, then Photoshop's defaults.
fn write_hue_saturation(
    out: &mut Vec<u8>,
    colorize: Option<[f32; 3]>,
    master: [f32; 3],
    ranges: &[([f32; 4], [f32; 3])],
) -> &'static [u8; 4] {
    let short = |out: &mut Vec<u8>, v: f32| out.extend((v.round() as i16).to_be_bytes());
    out.extend([0, 2, u8::from(colorize.is_some()), 0]);
    for v in colorize
        .unwrap_or([0.0, 25.0, 0.0])
        .into_iter()
        .chain(master)
    {
        short(out, v);
    }
    // Photoshop's default ranges, centered every 60 degrees from red.
    let mut slots: [([f32; 4], [f32; 3]); 6] = [
        [315.0, 345.0, 15.0, 45.0],
        [15.0, 45.0, 75.0, 105.0],
        [75.0, 105.0, 135.0, 165.0],
        [135.0, 165.0, 195.0, 225.0],
        [195.0, 225.0, 255.0, 285.0],
        [255.0, 285.0, 315.0, 345.0],
    ]
    .map(|range| (range, [0.0; 3]));
    // Overlapping ranges apply in order, so they keep theirs.
    for (slot, range) in slots.iter_mut().zip(ranges) {
        *slot = *range;
    }
    for (range, change) in slots {
        range.into_iter().chain(change).for_each(|v| short(out, v));
    }
    b"hue2"
}
/// A Photoshop color record (space, then components) as sRGB 0..=255:
/// RGB, Lab or grayscale.
fn psd_color(space: u16, [a, b, c]: [u16; 3]) -> Option<[f32; 3]> {
    match space {
        0 => Some([a, b, c].map(|v| f32::from(v) / 257.0)),
        // L in 0..=10000, a and b in hundredths.
        7 => Some(lab_d50_to_srgb([
            f32::from(a) / 100.0,
            f32::from(b as i16) / 100.0,
            f32::from(c as i16) / 100.0,
        ])),
        8 => Some([255.0 * (1.0 - f32::from(a.min(10000)) / 10000.0); 3]),
        _ => None,
    }
}
/// `Rd  `, `Grn `, `Bl  ` of an RGB color descriptor.
fn descriptor_rgb(color: &crate::psd_vector::Descriptor) -> Option<[f32; 3]> {
    if color.class != "RGBC" {
        return None;
    }
    let v = |key| color.number(key).map(|v| v.clamp(0.0, 255.0) as f32);
    Some([v("Rd  ")?, v("Grn ")?, v("Bl  ")?])
}
fn lab_d50_to_srgb([l, a, b]: [f32; 3]) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let f = |t: f32| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    xyz_d50_to_srgb([
        0.9642 * f(fy + a / 500.0),
        f(fy),
        0.8249 * f(fy - b / 200.0),
    ])
}
/// D50 XYZ (white Y 1) through the Bradford adaptation to sRGB 0..=255.
fn xyz_d50_to_srgb([x, y, z]: [f32; 3]) -> [f32; 3] {
    let d65 = [
        0.955_576_6 * x - 0.023_039_3 * y + 0.063_163_6 * z,
        -0.028_289_5 * x + 1.009_941_6 * y + 0.021_007_7 * z,
        0.012_298_2 * x - 0.020_483 * y + 1.329_909_8 * z,
    ];
    let [x, y, z] = d65;
    let linear = [
        3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z,
        -0.969_266 * x + 1.876_010_8 * y + 0.041_556 * z,
        0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z,
    ];
    linear.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let encoded = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).clamp(0.0, 255.0)
    })
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
    fn csp_curves_bend_towards_their_inner_points() {
        let csp = |points: Vec<[u8; 2]>, v: f32| {
            apply(
                Settings::Curves {
                    channels: vec![(0, points)],
                    bezier: true,
                },
                [v; 3],
            )[0]
        };
        // CSP's saved preview: (0.5, 0.667) takes 114 to 135, where a curve
        // through the point would give 155.
        assert_eq!(csp(vec![[0, 0], [128, 170], [255, 255]], 114.0), 135.0);
        assert_eq!(csp(vec![[0, 0], [99, 190], [255, 255]], 100.0), 145.0);
        assert_eq!(csp(vec![[0, 0], [255, 255]], 77.0), 77.0);
        // More inner points join quadratic pieces at their midpoints: the
        // curve stays inside 0..=255 and rises with its points.
        let points = vec![[0, 0], [60, 120], [140, 150], [200, 230], [255, 255]];
        let values: Vec<f32> = (0..=255).map(|v| csp(points.clone(), v as f32)).collect();
        assert!(values.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!((values[0], values[255]), (0.0, 255.0));
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
            bezier: false,
        };
        let out = apply(curves, [128.0, 128.0, 0.0]);
        assert_eq!(out[1], 200.0);
        // Red is inverted first (channel 1), then the composite curve.
        assert!(
            (out[0]
                - apply(
                    Settings::Curves {
                        channels: vec![(0, vec![[0, 0], [128, 200], [255, 255]])],
                        bezier: false,
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

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1.0)
    }

    #[test]
    fn color_adjustments_keep_neutral_settings_and_map_colors() {
        let colors = [[30.0, 120.0, 220.0], [200.0, 60.0, 90.0], [128.0; 3]];
        for (settings, neutral) in presets().into_iter().zip([true; 15]) {
            if matches!(
                settings,
                Settings::BlackWhite { .. }
                    | Settings::PhotoFilter { .. }
                    | Settings::Invert
                    | Settings::Posterize { .. }
                    | Settings::Threshold { .. }
                    | Settings::GradientMap { .. }
            ) {
                continue;
            }
            for c in colors {
                let out = apply(settings.clone(), c);
                assert_eq!(close(out, c), neutral, "{} {c:?} {out:?}", settings.name());
            }
        }
        // Black & white: grays keep their value; primaries and mixes take
        // their weight.
        let bw = |c| {
            apply(
                Settings::BlackWhite {
                    weights: BLACK_WHITE_DEFAULT,
                    tint: None,
                },
                c,
            )
        };
        assert!(close(bw([77.0; 3]), [77.0; 3]));
        assert!(close(bw([255.0, 0.0, 0.0]), [102.0; 3]));
        assert!(close(bw([255.0, 255.0, 0.0]), [153.0; 3]));
        let tinted = apply(
            Settings::BlackWhite {
                weights: BLACK_WHITE_DEFAULT,
                tint: Some([225.0, 211.0, 179.0]),
            },
            [128.0; 3],
        );
        assert!(tinted[0] > tinted[2]);
        assert!((crate::blend::lum(tinted.map(|v| v / 255.0)) * 255.0 - 128.0).abs() < 1.0);
        // Channel mixer: monochrome from green only.
        let mono = Settings::ChannelMixer {
            rows: [[0.0, 100.0, 0.0, 0.0], [0.0; 4], [0.0; 4]],
            monochrome: true,
        };
        assert_eq!(apply(mono, [10.0, 90.0, 200.0]), [90.0; 3]);
        // Photo filter: a warm filter warms gray but keeps its luminosity.
        let warm = apply(
            Settings::PhotoFilter {
                color: [236.0, 138.0, 0.0],
                density: 25.0,
                preserve_luminosity: true,
            },
            [128.0; 3],
        );
        assert!(warm[0] > warm[1] && warm[1] > warm[2]);
        assert!((crate::blend::lum(warm.map(|v| v / 255.0)) * 255.0 - 128.0).abs() < 1.0);
        // Color balance: red in the midtones reddens gray, not black.
        let mut tones = [[0.0; 3]; 3];
        tones[1][0] = 100.0;
        let balance = Settings::ColorBalance {
            tones,
            preserve_luminosity: false,
        };
        assert!(apply(balance.clone(), [128.0; 3])[0] > 200.0);
        assert_eq!(apply(balance, [0.0; 3]), [0.0; 3]);
        // Selective color: absolute cyan in reds empties red's red, leaves
        // blue alone.
        let mut colors = [[0.0; 4]; 9];
        colors[0][0] = 100.0;
        let selective = |absolute, c| apply(Settings::SelectiveColor { colors, absolute }, c);
        assert_eq!(selective(true, [255.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(selective(true, [0.0, 0.0, 255.0]), [0.0, 0.0, 255.0]);
        // Relative changes need ink already there: pure red has no cyan.
        assert_eq!(selective(false, [255.0, 0.0, 0.0]), [255.0, 0.0, 0.0]);
        // Vibrance lifts a muted color more than a vivid one.
        let vibrance = |c: [f32; 3]| {
            let out = apply(
                Settings::Vibrance {
                    vibrance: 80.0,
                    saturation: 0.0,
                },
                c,
            );
            to_hsl(out.map(|v| v / 255.0))[1] - to_hsl(c.map(|v| v / 255.0))[1]
        };
        assert!(vibrance([140.0, 120.0, 110.0]) > vibrance([250.0, 20.0, 10.0]));
    }

    #[test]
    fn color_adjustment_blocks_parse() {
        use crate::psd_vector::{Descriptor, Value, encode};
        let versioned = |d: &Descriptor| {
            let mut data = 16u32.to_be_bytes().to_vec();
            data.extend(encode(d));
            data
        };
        // Color balance: shadows, midtones, highlights, preserve luminosity.
        let mut blnc = Vec::new();
        for v in [-4i16, 2, -5, 10, 4, -9, 1, -9, -3] {
            blnc.extend(v.to_be_bytes());
        }
        blnc.extend([1, 0, 0, 0]);
        assert_eq!(
            from_psd(b"blnc", &blnc, false).unwrap(),
            Some(Settings::ColorBalance {
                tones: [[-4.0, 2.0, -5.0], [10.0, 4.0, -9.0], [1.0, -9.0, -3.0]],
                preserve_luminosity: true,
            })
        );
        assert!(from_psd(b"blnc", &blnc[..10], false).is_err());
        assert!(from_psd(b"blnc", &blnc, true).is_err());
        // Photoshop's warming filter (85), saved as Lab.
        let phfl = [
            0x00, 0x02, 0x00, 0x07, 0x1a, 0x32, 0x0c, 0x80, 0x2e, 0xe0, 0, 0, 0, 0, 0, 0x19, 1, 0,
            0, 0,
        ];
        let Some(Settings::PhotoFilter {
            color,
            density,
            preserve_luminosity,
        }) = from_psd(b"phfl", &phfl, false).unwrap()
        else {
            panic!()
        };
        assert!(close(color, [236.0, 138.0, 0.0]), "{color:?}");
        assert_eq!((density, preserve_luminosity), (25.0, true));
        // The same color as D50 XYZ (version 3).
        let mut xyz = vec![0, 3];
        for v in [0.4576f32, 0.3671, 0.0] {
            xyz.extend(((v * 32768.0) as u32).to_be_bytes());
        }
        xyz.extend([0, 0, 0, 25, 1]);
        let Some(Settings::PhotoFilter { color, .. }) = from_psd(b"phfl", &xyz, false).unwrap()
        else {
            panic!()
        };
        assert!(
            close(color.map(|v| v / 3.0), [236.0 / 3.0, 46.0, 0.0]),
            "{color:?}"
        );
        // Channel mixer, monochrome from green.
        let mut mixr = vec![0, 1, 0, 1];
        for v in [0i16, 100, 0, 0, 0] {
            mixr.extend(v.to_be_bytes());
        }
        assert_eq!(
            from_psd(b"mixr", &mixr, false).unwrap(),
            Some(Settings::ChannelMixer {
                rows: [[0.0, 100.0, 0.0, 0.0], [0.0; 4], [0.0; 4]],
                monochrome: true,
            })
        );
        assert!(from_psd(b"mixr", &mixr[..6], false).is_err());
        // Selective color, absolute, reds changed.
        let mut selc = vec![0, 1, 0, 1];
        selc.extend([0; 8]);
        for v in [15i16, -14, 33, 6] {
            selc.extend(v.to_be_bytes());
        }
        selc.extend([0; 64]);
        let Some(Settings::SelectiveColor { colors, absolute }) =
            from_psd(b"selc", &selc, false).unwrap()
        else {
            panic!()
        };
        assert!(absolute);
        assert_eq!(colors[0], [15.0, -14.0, 33.0, 6.0]);
        assert!(from_psd(b"selc", &selc[..40], false).is_err());
        // Descriptor-based: vibrance and black & white.
        let vib = Descriptor {
            class: "null".into(),
            items: vec![
                ("vibrance".into(), Value::Integer(21)),
                ("Strt".into(), Value::Integer(-4)),
            ],
        };
        assert_eq!(
            from_psd(b"vibA", &versioned(&vib), false).unwrap(),
            Some(Settings::Vibrance {
                vibrance: 21.0,
                saturation: -4.0,
            })
        );
        let tint = Descriptor {
            class: "RGBC".into(),
            items: vec![
                ("Rd  ".into(), Value::Number(55.6)),
                ("Grn ".into(), Value::Number(156.9)),
                ("Bl  ".into(), Value::Number(215.0)),
            ],
        };
        let bw = Descriptor {
            class: "null".into(),
            items: vec![
                ("Rd  ".into(), Value::Integer(-40)),
                ("Yllw".into(), Value::Integer(235)),
                ("useTint".into(), Value::Bool(true)),
                ("tintColor".into(), Value::Object(tint)),
            ],
        };
        let Some(Settings::BlackWhite { weights, tint }) =
            from_psd(b"blwh", &versioned(&bw), false).unwrap()
        else {
            panic!()
        };
        assert_eq!(weights[..3], [-40.0, 235.0, 40.0]);
        assert!(close(tint.unwrap(), [55.6, 156.9, 215.0]));
        assert!(from_psd(b"blwh", &[0, 0, 0, 16], false).is_err());
        // Damaged projects cannot smuggle out-of-range values.
        assert!(
            Adjustment::try_from(Settings::ChannelMixer {
                rows: [[f32::NAN; 4]; 3],
                monochrome: false,
            })
            .is_err()
        );
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
        let Some(Settings::Curves { channels, .. }) = from_psd(b"curv", &curv, false).unwrap()
        else {
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
        assert!(from_psd(b"clrL", &[], false).is_err());
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
