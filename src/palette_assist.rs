//! Palette assistance: colors extracted from the artwork and suggestions
//! related to the current color. The math runs in Oklab, where distances and
//! hue steps track perception; results stay 8-bit sRGB like every swatch.
use crate::{Dudel, color_wheel, selection::Selection};
use eframe::egui;
use std::f64::consts::{PI, TAU};

/// Most document pixels one extraction reads; larger regions use a grid.
const MAX_SAMPLES: usize = 1 << 16;
/// Clusters closer than this Oklab distance read as one swatch.
const MERGE_DISTANCE: f64 = 0.03;
/// Clusters below this share of the sampled coverage are noise, not accents.
const MIN_SHARE: f64 = 0.0025;

fn to_linear(channel: u8) -> f64 {
    let c = channel as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f64) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

/// sRGB to Oklab (Björn Ottosson, 2020).
pub(crate) fn oklab(rgb: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// Unclamped linear sRGB, so callers can tell whether a color is in gamut.
fn linear_rgb([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    [
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_,
    ]
}

pub(crate) fn srgb(lab: [f64; 3]) -> [u8; 3] {
    linear_rgb(lab).map(from_linear)
}

fn in_gamut(lab: [f64; 3]) -> bool {
    linear_rgb(lab)
        .iter()
        .all(|c| (-1e-6..=1.0 + 1e-6).contains(c))
}

fn distance2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// Weighted sRGB samples grouped into 32 levels per channel, so clustering
/// cost depends on the colors present rather than on the region size.
pub(crate) struct Samples {
    weight: Vec<u64>,
    sum: Vec<[u64; 3]>,
}

impl Default for Samples {
    fn default() -> Self {
        Self {
            weight: vec![0; 1 << 15],
            sum: vec![[0; 3]; 1 << 15],
        }
    }
}

impl Samples {
    /// `weight` is coverage, e.g. alpha times selection coverage.
    pub(crate) fn add(&mut self, rgb: [u8; 3], weight: u32) {
        if weight == 0 {
            return;
        }
        let [r, g, b] = rgb.map(|c| c as usize >> 3);
        let bin = r << 10 | g << 5 | b;
        self.weight[bin] += weight as u64;
        for (sum, channel) in self.sum[bin].iter_mut().zip(rgb) {
            *sum += channel as u64 * weight as u64;
        }
    }

    /// Up to `count` distinct colors, most covered first. Fewer return when
    /// the artwork has fewer distinguishable colors.
    pub(crate) fn palette(&self, count: usize) -> Vec<[u8; 3]> {
        let points: Vec<_> = self
            .weight
            .iter()
            .zip(&self.sum)
            .filter(|(weight, _)| **weight > 0)
            .map(|(&weight, sum)| {
                let mean = sum.map(|s| (s as f64 / weight as f64).round() as u8);
                (oklab(mean), weight as f64)
            })
            .collect();
        let total: f64 = points.iter().map(|point| point.1).sum();
        let mut clusters = merge(kmeans(&points, count));
        clusters.retain(|cluster| cluster.1 >= total * MIN_SHARE);
        let mut colors = Vec::with_capacity(clusters.len());
        for (lab, _) in clusters {
            let color = srgb(lab);
            if !colors.contains(&color) {
                colors.push(color);
            }
        }
        colors
    }
}

/// Weighted Lloyd iterations from a deterministic farthest-point start, so
/// the same artwork always yields the same palette.
fn kmeans(points: &[([f64; 3], f64)], count: usize) -> Vec<([f64; 3], f64)> {
    let Some(heaviest) = points.iter().max_by(|a, b| a.1.total_cmp(&b.1)) else {
        return vec![];
    };
    let mut centers = vec![heaviest.0];
    let mut nearest: Vec<f64> = points.iter().map(|p| distance2(p.0, heaviest.0)).collect();
    while centers.len() < count {
        let (index, score) = points
            .iter()
            .zip(&nearest)
            .map(|(point, distance)| point.1 * distance)
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if score <= 0.0 {
            break;
        }
        let center = points[index].0;
        centers.push(center);
        for (distance, point) in nearest.iter_mut().zip(points) {
            *distance = distance.min(distance2(point.0, center));
        }
    }
    let mut assignment = vec![usize::MAX; points.len()];
    let mut weights = vec![0.0; centers.len()];
    for _ in 0..32 {
        let mut moved = false;
        for (slot, point) in assignment.iter_mut().zip(points) {
            let best = (0..centers.len())
                .min_by(|&a, &b| {
                    distance2(point.0, centers[a]).total_cmp(&distance2(point.0, centers[b]))
                })
                .unwrap();
            moved |= *slot != best;
            *slot = best;
        }
        let mut sums = vec![[0.0; 3]; centers.len()];
        weights.fill(0.0);
        for (&cluster, point) in assignment.iter().zip(points) {
            weights[cluster] += point.1;
            for (sum, value) in sums[cluster].iter_mut().zip(point.0) {
                *sum += value * point.1;
            }
        }
        for ((center, sum), &weight) in centers.iter_mut().zip(&sums).zip(&weights) {
            if weight > 0.0 {
                *center = sum.map(|s| s / weight);
            }
        }
        if !moved {
            break;
        }
    }
    centers.into_iter().zip(weights).collect()
}

/// Joins clusters that would read as one swatch, largest first.
fn merge(mut clusters: Vec<([f64; 3], f64)>) -> Vec<([f64; 3], f64)> {
    clusters.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut merged: Vec<([f64; 3], f64)> = Vec::with_capacity(clusters.len());
    for (lab, weight) in clusters {
        if weight <= 0.0 {
            continue;
        }
        match merged
            .iter_mut()
            .find(|kept| distance2(kept.0, lab) < MERGE_DISTANCE * MERGE_DISTANCE)
        {
            Some(kept) => {
                let total = kept.1 + weight;
                kept.0 = std::array::from_fn(|i| (kept.0[i] * kept.1 + lab[i] * weight) / total);
                kept.1 = total;
            }
            None => merged.push((lab, weight)),
        }
    }
    merged
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scheme {
    #[default]
    Shading,
    Analogous,
    Complementary,
    Split,
    Triadic,
    Blend,
}

impl Scheme {
    pub(crate) const ALL: [Self; 6] = [
        Self::Shading,
        Self::Analogous,
        Self::Complementary,
        Self::Split,
        Self::Triadic,
        Self::Blend,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Shading => "Shade",
            Self::Analogous => "Analogous",
            Self::Complementary => "Complement",
            Self::Split => "Split",
            Self::Triadic => "Triad",
            Self::Blend => "Blend",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Shading => "Shadows shift cool and highlights warm around the current color",
            Self::Analogous => "Neighboring hues at the same lightness and chroma",
            Self::Complementary => "From the current color through neutral to its complement",
            Self::Split => "The two hues either side of the complement",
            Self::Triadic => "Three hues evenly spaced around the wheel",
            Self::Blend => "Steps from the current color to the secondary color",
        }
    }
}

fn lch([l, a, b]: [f64; 3]) -> [f64; 3] {
    [l, a.hypot(b), b.atan2(a)]
}

fn from_lch([l, c, h]: [f64; 3]) -> [f64; 3] {
    [l, c * h.cos(), c * h.sin()]
}

/// This lightness and hue at the most chroma, up to `c`, that sRGB can show.
fn fit([l, c, h]: [f64; 3]) -> [u8; 3] {
    let l = l.clamp(0.0, 1.0);
    if in_gamut(from_lch([l, c, h])) {
        return srgb(from_lch([l, c, h]));
    }
    let (mut low, mut high) = (0.0, c);
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if in_gamut(from_lch([l, middle, h])) {
            low = middle;
        } else {
            high = middle;
        }
    }
    srgb(from_lch([l, low, h]))
}

/// Shadows darken toward blue and highlights lighten toward yellow, the hue
/// shift painters use instead of mixing in black and white. Grays only
/// change value.
fn shade([l, c, h]: [f64; 3], amount: f64) -> [u8; 3] {
    const COOL: f64 = 265.0 * PI / 180.0;
    const WARM: f64 = 85.0 * PI / 180.0;
    const MAX_SHIFT: f64 = 20.0 * PI / 180.0;
    let t = amount.abs();
    let (target, hue, chroma) = if amount < 0.0 {
        (l.min(0.12), COOL, 1.0 - 0.1 * t)
    } else {
        (l.max(0.97), WARM, 1.0 - 0.5 * t)
    };
    let hue = if c < 0.02 {
        h
    } else {
        let delta = (hue - h + PI).rem_euclid(TAU) - PI;
        h + delta.clamp(-MAX_SHIFT, MAX_SHIFT) * t
    };
    fit([l + (target - l) * t * 0.85, c * chroma, hue])
}

/// Evenly spaced Oklab mixes from `from` to `to`, both ends exact.
fn blend(from: [u8; 3], to: [u8; 3], steps: usize) -> Vec<[u8; 3]> {
    let (a, b) = (oklab(from), oklab(to));
    (0..steps)
        .map(|step| match step {
            0 => from,
            _ if step + 1 == steps => to,
            _ => {
                let t = step as f64 / (steps - 1) as f64;
                srgb(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t))
            }
        })
        .collect()
}

/// Colors related to `color`; the current color itself is always included.
pub(crate) fn suggest(scheme: Scheme, color: [u8; 3], secondary: [u8; 3]) -> Vec<[u8; 3]> {
    let base = lch(oklab(color));
    let turn = |degrees: f64| fit([base[0], base[1], base[2] + degrees.to_radians()]);
    let mut colors = match scheme {
        Scheme::Shading => (-2..=2)
            .map(|step| match step {
                0 => color,
                _ => shade(base, step as f64 / 2.0),
            })
            .collect(),
        Scheme::Analogous => vec![turn(-60.0), turn(-30.0), color, turn(30.0), turn(60.0)],
        Scheme::Complementary => blend(color, turn(180.0), 5),
        Scheme::Split => vec![turn(150.0), color, turn(210.0)],
        Scheme::Triadic => vec![turn(-120.0), color, turn(120.0)],
        Scheme::Blend => blend(color, secondary, 5),
    };
    colors.dedup();
    colors
}

pub(crate) struct Assist {
    pub(crate) scheme: Scheme,
    /// Colors one extraction asks for.
    pub(crate) count: usize,
    /// The last extraction, kept apart from the swatches until saved.
    pub(crate) extracted: Vec<[u8; 3]>,
    /// The color suggestions are built around, and the suggestion picked
    /// from them. Picking keeps the row in place; any other color change
    /// rebuilds it around the new color.
    anchor: Option<([u8; 3], [u8; 3])>,
}

impl Default for Assist {
    fn default() -> Self {
        Self {
            scheme: Scheme::default(),
            count: 6,
            extracted: vec![],
            anchor: None,
        }
    }
}

impl Dudel {
    /// Colors of the active layer, or of the visible canvas, within the
    /// selection when there is one. Transparent pixels contribute nothing.
    pub(crate) fn extract_palette(&mut self, layer_only: bool) {
        let doc = &self.doc;
        let layer = &doc.layers[doc.active];
        if layer_only && layer.folder {
            self.status = "Select a layer inside the folder to extract its colors".into();
            return;
        }
        let mask = doc.selection_mask.as_ref();
        let area = doc
            .selection
            .or_else(|| mask.and_then(|mask| mask.bounds()))
            .unwrap_or(Selection {
                left: 0,
                top: 0,
                right: doc.width,
                bottom: doc.height,
            });
        let [left, right] = [area.left, area.right].map(|x| x.min(doc.width));
        let [top, bottom] = [area.top, area.bottom].map(|y| y.min(doc.height));
        let pixels = right.saturating_sub(left) * bottom.saturating_sub(top);
        let step = ((pixels as f64 / MAX_SAMPLES as f64).sqrt().ceil() as usize).max(1);
        let mut samples = Samples::default();
        for y in (top..bottom).step_by(step) {
            for x in (left..right).step_by(step) {
                let coverage = mask.map_or(255, |mask| mask.at(x, y));
                if coverage == 0 {
                    continue;
                }
                let index = y * doc.width + x;
                let [r, g, b, a] = if layer_only {
                    layer.pixels.rgba(index)
                } else {
                    doc.composite_display_pixel(index, true)
                };
                samples.add([r, g, b], a as u32 * coverage as u32);
            }
        }
        let source = match (layer_only, doc.selection.is_some() || mask.is_some()) {
            (true, true) => "selected part of the layer",
            (true, false) => "layer",
            (false, true) => "selection",
            (false, false) => "canvas",
        };
        let colors = samples.palette(self.palette_assist.count);
        self.status = if colors.is_empty() {
            format!("No painted pixels in the {source}")
        } else {
            format!("Extracted {} colors from the {source}", colors.len())
        };
        self.palette_assist.extracted = colors;
    }

    /// Saves colors that are not swatches yet, as many as the palette holds.
    pub(crate) fn save_swatches(&mut self, colors: &[[u8; 3]]) {
        let mut new = Vec::new();
        for &color in colors {
            if !self.palette.contains(&color) && !new.contains(&color) {
                new.push(color);
            }
        }
        let saved = new.len().min(64 - self.palette.len().min(64));
        self.palette.extend(&new[..saved]);
        self.status = if saved < new.len() {
            format!("Palette is full; saved {saved} of {} colors", new.len())
        } else {
            format!("Saved {saved} swatches")
        };
    }

    pub(crate) fn palette_extract_menu_ui(&mut self, ui: &mut egui::Ui) {
        let scope = if self.doc.selection.is_some() || self.doc.selection_mask.is_some() {
            "Only the selection is read"
        } else {
            "Select an area first to read only that part"
        };
        if ui
            .button("Extract from canvas")
            .on_hover_text(format!("Colors of all visible layers. {scope}"))
            .clicked()
        {
            ui.close();
            self.extract_palette(false);
        }
        if ui
            .button("Extract from layer")
            .on_hover_text(format!("Colors of the active layer. {scope}"))
            .clicked()
        {
            ui.close();
            self.extract_palette(true);
        }
        ui.add(egui::Slider::new(&mut self.palette_assist.count, 3..=12).text("colors"));
    }

    pub(crate) fn palette_assist_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Color ideas")
            .default_open(true)
            .show(ui, |ui| {
                if !self.palette_assist.extracted.is_empty() {
                    let extracted = self.palette_assist.extracted.clone();
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Extracted").small().weak());
                        if ui.small_button("Save all").clicked() {
                            self.save_swatches(&extracted);
                        }
                        if ui.small_button("Clear").clicked() {
                            self.palette_assist.extracted.clear();
                        }
                    });
                    self.idea_swatches_ui(ui, &extracted, None);
                }
                let base = match self.palette_assist.anchor {
                    Some((base, picked)) if picked == self.color => base,
                    _ => self.color,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for scheme in Scheme::ALL {
                        ui.selectable_value(
                            &mut self.palette_assist.scheme,
                            scheme,
                            scheme.label(),
                        )
                        .on_hover_text(scheme.hint());
                    }
                });
                let ideas = suggest(self.palette_assist.scheme, base, self.secondary);
                self.idea_swatches_ui(ui, &ideas, Some(base));
            });
    }

    /// Click to paint with a color; right-click to save it as a swatch.
    /// `base` keeps a suggestion row in place while its colors are picked.
    fn idea_swatches_ui(&mut self, ui: &mut egui::Ui, colors: &[[u8; 3]], base: Option<[u8; 3]>) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for &color in colors {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 3.0, color_wheel::color32(color));
                if self.color == color {
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
                let response = response.on_hover_text(format!(
                    "#{} · right-click to save",
                    color_wheel::hex(color)
                ));
                if response.clicked() {
                    self.palette_assist.anchor = base.map(|base| (base, color));
                    self.color = color;
                }
                response.context_menu(|ui| {
                    if ui.button("Save as swatch").clicked() {
                        self.save_swatches(&[color]);
                        ui.close();
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hue_degrees(color: [u8; 3]) -> f64 {
        lch(oklab(color))[2].to_degrees().rem_euclid(360.0)
    }

    #[test]
    fn oklab_round_trips_8_bit_colors_and_matches_reference_white() {
        let white = oklab([255, 255, 255]);
        assert!((white[0] - 1.0).abs() < 1e-4 && white[1].abs() < 1e-4 && white[2].abs() < 1e-4);
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(85) {
                    let rgb = [r as u8, g as u8, b as u8];
                    assert_eq!(srgb(oklab(rgb)), rgb);
                }
            }
        }
    }

    #[test]
    fn extraction_orders_by_coverage_keeps_accents_and_ignores_transparency() {
        let mut samples = Samples::default();
        let mut add = |rgb: [u8; 3], alpha: u32, count: usize| {
            for _ in 0..count {
                samples.add(rgb, alpha * 255);
            }
        };
        add([40, 70, 160], 255, 700);
        add([42, 72, 158], 255, 50);
        add([230, 150, 60], 255, 250);
        add([220, 30, 40], 255, 20);
        // Hidden RGB under full transparency must not become a swatch.
        add([0, 255, 0], 0, 5000);
        let colors = samples.palette(6);
        assert_eq!(colors.len(), 3, "{colors:?}");
        let near = |a: [u8; 3], b: [u8; 3]| distance2(oklab(a), oklab(b)) < 0.02 * 0.02;
        assert!(near(colors[0], [40, 70, 160]), "{colors:?}");
        assert!(near(colors[1], [230, 150, 60]), "{colors:?}");
        assert!(near(colors[2], [220, 30, 40]), "{colors:?}");
        assert_eq!(samples.palette(6), colors, "extraction must be repeatable");
        assert!(Samples::default().palette(6).is_empty());
    }

    #[test]
    fn extraction_reads_only_selected_painted_pixels() {
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 4),
            ..Default::default()
        };
        {
            let mut writer = app.doc.layers[0].pixels.writer();
            for y in 0..4 {
                for x in 0..8 {
                    let rgba = if x < 4 {
                        [200, 40, 40, 255]
                    } else {
                        [40, 40, 200, 255]
                    };
                    writer.rgba_mut(y * 8 + x).copy_from_slice(&rgba);
                }
            }
        }
        app.extract_palette(false);
        assert_eq!(app.palette_assist.extracted.len(), 2);
        app.doc.selection = Some(Selection {
            left: 4,
            top: 0,
            right: 8,
            bottom: 4,
        });
        app.extract_palette(true);
        assert_eq!(app.palette_assist.extracted, vec![[40, 40, 200]]);
        app.palette = (0..63).map(|i| [i, 0, 0]).collect();
        app.save_swatches(&[[1, 0, 0], [9, 9, 9], [8, 8, 8]]);
        assert_eq!(app.palette.len(), 64);
        assert_eq!(app.palette[63], [9, 9, 9]);
    }

    #[test]
    fn suggestions_include_the_color_and_follow_their_scheme() {
        let color = [70, 130, 180];
        let shades = suggest(Scheme::Shading, color, [0, 0, 0]);
        assert_eq!(shades.len(), 5);
        assert_eq!(shades[2], color);
        let lightness: Vec<f64> = shades.iter().map(|&c| oklab(c)[0]).collect();
        assert!(lightness.windows(2).all(|w| w[0] < w[1]), "{lightness:?}");
        assert!(
            hue_degrees(shades[4]) < hue_degrees(color),
            "highlights shift warm"
        );
        let gray = suggest(Scheme::Shading, [128, 128, 128], [0, 0, 0]);
        assert!(
            gray.iter()
                .all(|c| c[0].abs_diff(c[1]) <= 1 && c[1].abs_diff(c[2]) <= 1)
        );

        let complement = suggest(Scheme::Complementary, color, [0, 0, 0]);
        assert_eq!(complement[0], color);
        let apart = (hue_degrees(complement[4]) - hue_degrees(color)).rem_euclid(360.0);
        assert!((apart - 180.0).abs() < 3.0, "{apart}");

        let triad = suggest(Scheme::Triadic, color, [0, 0, 0]);
        assert_eq!(triad[1], color);
        assert_eq!(
            suggest(Scheme::Blend, color, [250, 240, 200]),
            blend(color, [250, 240, 200], 5)
        );
        assert_eq!(suggest(Scheme::Blend, color, color), vec![color]);
    }
}
