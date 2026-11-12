//! Pigment mixing: colors that mix like paint, so blue and yellow make
//! green rather than gray, red and blue make purple, and white tints.
//!
//! Each color becomes a reflectance spectrum (16 bands) built from seven
//! primaries (white, cyan, magenta, yellow, red, green, blue) whose spectra
//! overlap the way real pigments do. Spectra mix by Kubelka–Munk: absorption
//! over scattering (K/S) adds by concentration, then turns back into
//! reflectance and color. Concentrations are weighted by a low power of each
//! color's luminance, so a little black still darkens strongly and
//! white tints rather than washing out. Whatever the model gets wrong about a
//! single color is added back in proportion (the residual), so mixing a color
//! with itself, or taking all of one, gives exactly that color.
//!
//! Material layers (watercolor and thick paint) mix this way wherever paint
//! meets paint: wet-on-wet, paint landing on wet paint, smudging and
//! blending.
use std::sync::OnceLock;

const BANDS: usize = 16;
/// Concentrations are weighted by luminance to this power: light paints
/// tint strongly (white makes pink of red), dark ones still darken.
const TINT: f32 = 0.35;

struct Model {
    /// Spectra of white, cyan, magenta, yellow, red, green, blue.
    primaries: [[f32; BANDS]; 7],
    /// Red, green and blue sensor weights per band.
    sensors: [[f32; BANDS]; 3],
    /// Undoes the sensors' overlap, so a pure band reads as its own channel.
    unmix: [[f32; 3]; 3],
}

fn model() -> &'static Model {
    static MODEL: OnceLock<Model> = OnceLock::new();
    MODEL.get_or_init(|| {
        let at = |i: usize| i as f32 / (BANDS - 1) as f32;
        let bump = |center: f32, width: f32, x: f32| (-((x - center) / width).powi(2)).exp();
        // Three bands that sum to one everywhere: short (blue), middle
        // (green) and long (red) wavelengths.
        let bands: [[f32; BANDS]; 3] = std::array::from_fn(|c| {
            std::array::from_fn(|i| {
                let x = at(i);
                let raw = [bump(0.9, 0.2, x), bump(0.5, 0.2, x), bump(0.1, 0.2, x)];
                raw[c] / raw.iter().sum::<f32>()
            })
        });
        let [red, green, blue] = bands;
        let combine = |weights: [f32; 3]| -> [f32; BANDS] {
            std::array::from_fn(|i| {
                weights[0] * red[i] + weights[1] * green[i] + weights[2] * blue[i]
            })
        };
        // Reds reflect a little blue and blues some green, as real pigments
        // do: that is what makes blue and yellow green and red and blue purple.
        let primaries = [
            [1.0; BANDS],
            combine([0.0, 1.0, 1.0]),
            combine([1.0, 0.0, 1.0]),
            combine([1.0, 1.0, 0.0]),
            combine([1.0, 0.0, 0.1]),
            combine([0.0, 1.0, 0.0]),
            combine([0.0, 0.25, 1.0]),
        ];
        let sensors = [0.9, 0.5, 0.1].map(|center| {
            let mut sensor: [f32; BANDS] = std::array::from_fn(|i| bump(center, 0.13, at(i)));
            let total: f32 = sensor.iter().sum();
            sensor.iter_mut().for_each(|v| *v /= total);
            sensor
        });
        let mut overlap = [[0.0f32; 3]; 3];
        for (row, sensor) in sensors.iter().enumerate() {
            for (column, band) in bands.iter().enumerate() {
                overlap[row][column] = (0..BANDS).map(|i| sensor[i] * band[i]).sum();
            }
        }
        Model {
            primaries,
            sensors,
            unmix: invert(overlap),
        }
    })
}

fn invert(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [
        [
            (e * i - f * h) / det,
            (c * h - b * i) / det,
            (b * f - c * e) / det,
        ],
        [
            (f * g - d * i) / det,
            (a * i - c * g) / det,
            (c * d - a * f) / det,
        ],
        [
            (d * h - e * g) / det,
            (b * g - a * h) / det,
            (a * e - b * d) / det,
        ],
    ]
}

/// sRGB byte to linear light.
fn linear(byte: u8) -> f32 {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let v = i as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    })[usize::from(byte)]
}

/// Linear light to an sRGB byte.
fn encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

fn spectrum(model: &Model, [r, g, b]: [f32; 3]) -> [f32; BANDS] {
    let white = r.min(g).min(b);
    let (r, g, b) = (r - white, g - white, b - white);
    let amounts = [
        white,
        g.min(b),
        r.min(b),
        r.min(g),
        (r - b).max(0.0).min((r - g).max(0.0)),
        (g - b).max(0.0).min((g - r).max(0.0)),
        (b - g).max(0.0).min((b - r).max(0.0)),
    ];
    let mut out = [0.0f32; BANDS];
    for (amount, primary) in amounts.iter().zip(&model.primaries) {
        if *amount > 0.0 {
            for (value, reflects) in out.iter_mut().zip(primary) {
                *value += amount * reflects;
            }
        }
    }
    out.map(|v| v.clamp(1e-4, 1.0))
}

fn project(model: &Model, reflectance: &[f32; BANDS]) -> [f32; 3] {
    let seen: [f32; 3] = std::array::from_fn(|c| {
        (0..BANDS)
            .map(|i| model.sensors[c][i] * reflectance[i])
            .sum()
    });
    std::array::from_fn(|row| (0..3).map(|c| model.unmix[row][c] * seen[c]).sum())
}

fn luminance([r, g, b]: [f32; 3]) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Absorption over scattering for a reflectance.
fn ks(r: f32) -> f32 {
    (1.0 - r) * (1.0 - r) / (2.0 * r)
}
fn reflectance(ks: f32) -> f32 {
    1.0 + ks - (ks * ks + 2.0 * ks).sqrt()
}

/// Steps the mixing amount is rounded to; 256 keeps every step below what
/// a byte of color can show.
const STEPS: f32 = 256.0;

/// Mix `t` (0 to 1) of paint `b` into paint `a`, as pigments. Smudging
/// mixes the same few colors over and over, so recent results are kept in
/// a small table per thread.
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let step = (t.clamp(0.0, 1.0) * STEPS).round() as u32;
    if a == b || step == 0 {
        return a;
    }
    if step >= STEPS as u32 {
        return b;
    }
    const SLOTS: usize = 4096;
    thread_local! {
        static RECENT: std::cell::RefCell<Vec<(u64, [u8; 3])>> =
            std::cell::RefCell::new(vec![(u64::MAX, [0; 3]); SLOTS]);
    }
    let key = u64::from(a[0]) << 56
        | u64::from(a[1]) << 48
        | u64::from(a[2]) << 40
        | u64::from(b[0]) << 32
        | u64::from(b[1]) << 24
        | u64::from(b[2]) << 16
        | u64::from(step);
    let slot = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 52) as usize % SLOTS;
    if let Some(hit) = RECENT.with(|recent| {
        let entry = recent.borrow()[slot];
        (entry.0 == key).then_some(entry.1)
    }) {
        return hit;
    }
    let mixed = compute(a, b, step as f32 / STEPS);
    RECENT.with(|recent| recent.borrow_mut()[slot] = (key, mixed));
    mixed
}

fn compute(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let model = model();
    let la = a.map(linear);
    let lb = b.map(linear);
    let wa = luminance(la).powf(TINT) * (1.0 - t);
    let wb = luminance(lb).powf(TINT) * t;
    let share = if wa + wb > 0.0 { wb / (wa + wb) } else { t };
    let (ra, rb) = (spectrum(model, la), spectrum(model, lb));
    let mixed: [f32; BANDS] =
        std::array::from_fn(|i| reflectance(ks(ra[i]) * (1.0 - share) + ks(rb[i]) * share));
    let out = project(model, &mixed);
    // What the model gets wrong about each color alone, added back.
    let (pa, pb) = (project(model, &ra), project(model, &rb));
    std::array::from_fn(|c| encode(out[c] + (1.0 - t) * (la[c] - pa[c]) + t * (lb[c] - pb[c])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hue([r, g, b]: [u8; 3]) -> f32 {
        let [r, g, b] = [r, g, b].map(|v| f32::from(v) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        if d == 0.0 {
            return 0.0;
        }
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h * 60.0
    }

    #[test]
    fn paints_mix_like_paint() {
        let blue = [20, 40, 200];
        let yellow = [250, 220, 20];
        let red = [220, 30, 30];
        let white = [255, 255, 255];
        let black = [10, 10, 10];
        let green = mix(blue, yellow, 0.5);
        assert!(
            (90.0..160.0).contains(&hue(green)),
            "blue + yellow {green:?}"
        );
        let orange = mix(red, yellow, 0.5);
        assert!(
            (5.0..45.0).contains(&hue(orange)),
            "red + yellow {orange:?}"
        );
        let purple = mix(red, blue, 0.5);
        assert!(
            (260.0..340.0).contains(&hue(purple)),
            "red + blue {purple:?}"
        );
        let pink = mix(white, red, 0.5);
        assert!(
            pink[1] > 60 && pink[0] > 200 && pink[2] > 80,
            "white + red {pink:?}"
        );
        let dark = mix(black, yellow, 0.5);
        assert!(dark.iter().all(|v| *v < 90), "black + yellow {dark:?}");
        // Linear mixing would make gray; pigment keeps color.
        let [r, g, b] = green;
        assert!(g > r + 40 && g > b + 40, "{green:?}");
    }

    #[test]
    fn a_color_alone_is_unchanged() {
        for color in [
            [120, 80, 40],
            [0, 0, 0],
            [255, 255, 255],
            [20, 200, 90],
            [7, 7, 250],
        ] {
            assert_eq!(mix(color, color, 0.5), color);
            assert_eq!(mix(color, [200, 10, 10], 0.0), color);
            assert_eq!(mix([200, 10, 10], color, 1.0), color);
            // Through the model, a color mixed with itself reads back.
            let near = mix(color, [color[0], color[1], color[2].saturating_add(1)], 0.0);
            assert_eq!(near, color);
        }
    }

    #[test]
    fn mixing_moves_smoothly_from_one_paint_to_the_other() {
        let (a, b) = ([20, 40, 200], [250, 220, 20]);
        let steps: Vec<[u8; 3]> = (0..=20).map(|i| mix(a, b, i as f32 / 20.0)).collect();
        for pair in steps.windows(2) {
            let jump = (0..3)
                .map(|c| (i32::from(pair[0][c]) - i32::from(pair[1][c])).abs())
                .max()
                .unwrap();
            // Paint is steep where a little dark pigment meets a light
            // one, but never jumps.
            assert!(jump < 120, "{pair:?}");
        }
    }
}
