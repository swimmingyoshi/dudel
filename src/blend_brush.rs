//! Smudge and blur dabs: brushes that move or soften the paint already on
//! the layer instead of laying down a color.
//!
//! Each dab first copies the area it can read into a premultiplied scratch
//! (so pixels it changes are not read back within the same dab), then mixes
//! every covered pixel toward its source by the dab's delivery:
//!
//! - **Smudge** reads the layer where the previous dab was, so paint is
//!   carried along the stroke.
//! - **Blur** reads a box-blurred copy of the area around the dab.
//!
//! The stroke buffer records each pixel's original before it first changes,
//! so cancel and undo work as for painting.
use crate::brush::Blend;

/// Premultiplied RGBA in 0..=1 for one dab's readable area.
#[derive(Default)]
pub(crate) struct Scratch {
    /// Left, top, right, bottom (exclusive) in canvas pixels.
    rect: [usize; 4],
    values: Vec<[f32; 4]>,
    /// One row or column of sums while blurring.
    line: Vec<[f32; 4]>,
}

impl Scratch {
    pub(crate) fn capacity_bytes(&self) -> usize {
        (self.values.capacity() + self.line.capacity()) * std::mem::size_of::<[f32; 4]>()
    }
}

/// Paint vertical stripes of `color` across columns `left..right`, so
/// previews of smudge and blur brushes have something to move.
pub(crate) fn preview_stripes(
    doc: &mut crate::document::Document,
    color: [u8; 3],
    [left, right]: [usize; 2],
) {
    let (width, height) = (doc.width, doc.height);
    let mut writer = doc.layers[0].pixels.writer();
    for y in 0..height {
        for x in left.min(width)..right.min(width) {
            if (x - left) / 10 % 2 == 0 {
                writer
                    .rgba_mut(y * width + x)
                    .copy_from_slice(&[color[0], color[1], color[2], 255]);
            }
        }
    }
}

impl crate::Dudel {
    /// Screenshot scene: color bands with a smudge wave and a blur stroke
    /// across them, and the Smudge preset in the tool panel.
    pub(crate) fn smudge_capture_fixture(&mut self) {
        self.tool_panel_override = Some(true);
        self.select_tool(crate::tools::Tool::Brush);
        if let Some(index) = self.brushes.iter().position(|b| b.name == "Smudge") {
            self.brush = self.brushes[index].clone();
            self.selected_brush = index;
        }
        let (width, height) = (self.doc.width, self.doc.height);
        let bands = [[226, 94, 62], [246, 196, 70], [70, 150, 120], [60, 92, 170]];
        {
            let mut writer = self.doc.layers[self.doc.active].pixels.writer();
            for y in 0..height {
                for x in 0..width {
                    let c = bands[x / 48 % bands.len()];
                    writer
                        .rgba_mut(y * width + x)
                        .copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
        let run =
            |doc: &mut crate::document::Document, brush: crate::brush::Brush, y: f32, wave: f32| {
                let settings = crate::stroke_buffer::Settings {
                    target: crate::paint_target::Target::Raster,
                    brush,
                    color: [0, 0, 0],
                    secondary: [0, 0, 0],
                    erase: false,
                    layer: doc.active,
                    symmetry: Default::default(),
                };
                let mut stroke = crate::stroke::Stroke::default();
                let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
                let steps = 120;
                for i in 0..=steps {
                    let t = i as f32 / steps as f32;
                    let sample = crate::stroke::Sample {
                        x: width as f32 * (0.08 + 0.84 * t),
                        y: y + (t * std::f32::consts::TAU * 1.5).sin() * wave,
                        pressure: 0.9,
                        pressure_known: true,
                        ..Default::default()
                    };
                    let dabs = if i == steps {
                        stroke.finish(sample, &settings.brush)
                    } else {
                        stroke.sample(sample, &settings.brush)
                    };
                    for dab in dabs {
                        buffer.paint(doc, dab, &settings);
                    }
                }
            };
        let smudge = self.brush.clone();
        run(
            &mut self.doc,
            smudge,
            height as f32 * 0.3,
            height as f32 * 0.08,
        );
        if let Some(blur) = self.brushes.iter().find(|b| b.name == "Blur").cloned() {
            run(&mut self.doc, blur, height as f32 * 0.7, 0.0);
        }
        self.changed();
    }
}

/// How far a blur dab reaches, in pixels, for a tip of this radius.
pub(crate) fn blur_radius(radius: f32) -> usize {
    (radius * 0.25).round().clamp(1.0, 24.0) as usize
}

fn premultiply(p: [u8; 4]) -> [f32; 4] {
    let a = f32::from(p[3]) / 255.0;
    [
        f32::from(p[0]) / 255.0 * a,
        f32::from(p[1]) / 255.0 * a,
        f32::from(p[2]) / 255.0 * a,
        a,
    ]
}

/// Copy the area a dab can read: its bounds, widened by how far it reaches.
/// Blur then softens the copy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture(
    scratch: &mut Scratch,
    read: impl Fn(usize, usize) -> [u8; 4],
    [width, height]: [usize; 2],
    bounds: [i32; 4],
    blend: Blend,
    smear: [f32; 2],
    radius: f32,
) {
    let reach = match blend {
        Blend::Smudge => smear[0].abs().max(smear[1].abs()).ceil() as i32 + 2,
        Blend::Blur | Blend::Wet => blur_radius(radius) as i32,
        Blend::Paint => 0,
    };
    let clamp = |v: i32, limit: usize| v.clamp(0, limit as i32) as usize;
    let rect = [
        clamp(bounds[0] - reach, width),
        clamp(bounds[1] - reach, height),
        clamp(bounds[2] + 1 + reach, width),
        clamp(bounds[3] + 1 + reach, height),
    ];
    scratch.rect = rect;
    scratch.values.clear();
    for y in rect[1]..rect[3] {
        for x in rect[0]..rect[2] {
            scratch.values.push(premultiply(read(x, y)));
        }
    }
    if matches!(blend, Blend::Blur | Blend::Wet) {
        let r = blur_radius(radius);
        // Two box passes approximate a smooth (tent) blur.
        for _ in 0..2 {
            box_blur(scratch, r);
        }
    }
}

/// Separable box blur of the scratch, averaging only pixels inside it so
/// canvas edges do not darken.
fn box_blur(scratch: &mut Scratch, r: usize) {
    let w = scratch.rect[2] - scratch.rect[0];
    let h = scratch.rect[3] - scratch.rect[1];
    if w == 0 || h == 0 {
        return;
    }
    let Scratch { values, line, .. } = scratch;
    let mut pass = |count: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
        for l in 0..lines {
            line.clear();
            line.extend((0..count).map(|i| values[at(l, i)]));
            let mut sum = [0.0f32; 4];
            for v in line.iter().take(r.min(count - 1) + 1) {
                for c in 0..4 {
                    sum[c] += v[c];
                }
            }
            for i in 0..count {
                // Window [i - r, i + r] clipped to the line.
                let (low, high) = (i.saturating_sub(r), (i + r).min(count - 1));
                let n = (high - low + 1) as f32;
                let out = &mut values[at(l, i)];
                for c in 0..4 {
                    out[c] = sum[c] / n;
                }
                if i + r + 1 < count {
                    let v = line[i + r + 1];
                    for c in 0..4 {
                        sum[c] += v[c];
                    }
                }
                if i >= r {
                    let v = line[i - r];
                    for c in 0..4 {
                        sum[c] -= v[c];
                    }
                }
            }
        }
    };
    pass(w, h, &|row, i| row * w + i);
    pass(h, w, &|column, i| i * w + column);
}

impl Scratch {
    fn at(&self, x: usize, y: usize) -> [f32; 4] {
        let w = self.rect[2] - self.rect[0];
        self.values[(y - self.rect[1]) * w + (x - self.rect[0])]
    }
    /// What pixel (`x`, `y`) moves toward.
    pub(crate) fn source(&self, blend: Blend, smear: [f32; 2], x: usize, y: usize) -> [f32; 4] {
        if blend != Blend::Smudge {
            return self.at(x, y);
        }
        // Bilinear read where this pixel was one dab ago, held inside the copy.
        let fx = (x as f32 - smear[0]).clamp(self.rect[0] as f32, (self.rect[2] - 1) as f32);
        let fy = (y as f32 - smear[1]).clamp(self.rect[1] as f32, (self.rect[3] - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = (
            (x0 + 1).min(self.rect[2] - 1),
            (y0 + 1).min(self.rect[3] - 1),
        );
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let (a, b, c, d) = (
            self.at(x0, y0),
            self.at(x1, y0),
            self.at(x0, y1),
            self.at(x1, y1),
        );
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bottom = c[i] + (d[i] - c[i]) * tx;
            out[i] = top + (bottom - top) * ty;
        }
        out
    }
}

/// Move `pixel` toward premultiplied `source` by `amount`. Lock transparency
/// keeps the pixel's alpha and changes only its color. With `pigment`, the
/// colors mix as paint does (`pigment::mix`).
pub(crate) fn mix(
    pixel: &mut [u8],
    source: [f32; 4],
    amount: f32,
    alpha_locked: bool,
    pigment: bool,
) {
    let amount = amount.clamp(0.0, 1.0);
    let current = premultiply([pixel[0], pixel[1], pixel[2], pixel[3]]);
    let mut out = [0.0; 4];
    for c in 0..4 {
        out[c] = current[c] + (source[c] - current[c]) * amount;
    }
    if pigment && current[3] > 0.0 && source[3] > 0.0 && out[3] > 0.0 {
        // The share of the result that comes from the source, as paint.
        let share = (source[3] * amount / out[3]).clamp(0.0, 1.0);
        let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        let from = [pixel[0], pixel[1], pixel[2]];
        let to = [0, 1, 2].map(|c| byte(source[c] / source[3]));
        let mixed = crate::pigment::mix(from, to, share);
        for c in 0..3 {
            out[c] = f32::from(mixed[c]) / 255.0 * out[3];
        }
    }
    let alpha = if alpha_locked {
        f32::from(pixel[3]) / 255.0
    } else {
        out[3]
    };
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    if out[3] * 255.0 < 0.5 {
        // Fully clear: keep the hidden color, as erasing does.
        if !alpha_locked {
            pixel[3] = 0;
        }
        return;
    }
    for c in 0..3 {
        pixel[c] = byte(out[c] / out[3]);
    }
    pixel[3] = byte(alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_is_premultiplied_and_respects_lock_transparency() {
        // Half way from opaque red to clear stays red, at half alpha.
        let mut pixel = [255, 0, 0, 255];
        mix(&mut pixel, [0.0; 4], 0.5, false, false);
        assert_eq!(pixel, [255, 0, 0, 128]);
        // Locked: alpha kept, color moves toward the opaque source.
        let mut pixel = [255, 0, 0, 255];
        mix(&mut pixel, [0.0, 0.0, 1.0, 1.0], 0.5, true, false);
        assert_eq!(pixel, [128, 0, 128, 255]);
        // Fully clearing keeps the hidden color.
        let mut pixel = [10, 20, 30, 255];
        mix(&mut pixel, [0.0; 4], 1.0, false, false);
        assert_eq!(pixel, [10, 20, 30, 0]);
    }

    use crate::document::Document;
    use crate::stroke_buffer::{Settings, StrokeBuffer};

    /// Stripes 10 px wide across a 120 x 60 layer.
    fn striped() -> Document {
        let mut doc = Document::new(120, 60);
        preview_stripes(&mut doc, [200, 40, 40], [0, 120]);
        doc
    }
    fn settings(blend: Blend) -> Settings {
        Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 16.0,
                opacity: 0.9,
                hardness: 0.6,
                spacing: 0.05,
                pressure_size: false,
                taper_out: 20.0,
                blend,
                ..Default::default()
            },
            color: [0, 0, 255],
            secondary: [0, 0, 255],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        }
    }
    /// Dabs of a straight stroke left to right along y = 30.
    fn dabs(settings: &Settings) -> Vec<crate::stroke::Sample> {
        let mut stroke = crate::stroke::Stroke::default();
        let mut dabs = Vec::new();
        for i in 0..=10 {
            let sample = crate::stroke::Sample {
                x: 15.0 + i as f32 * 9.0,
                y: 30.0,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            dabs.extend(if i == 10 {
                stroke.finish(sample, &settings.brush)
            } else {
                stroke.sample(sample, &settings.brush)
            });
        }
        dabs
    }
    fn paint(doc: &mut Document, settings: &Settings) -> StrokeBuffer {
        let mut buffer = StrokeBuffer::default();
        for dab in dabs(settings) {
            buffer.paint(doc, dab, settings);
        }
        buffer.finish_tail(doc, settings);
        buffer
    }

    #[test]
    fn smudge_drags_paint_forward_and_cancel_restores_it() {
        let original = striped();
        let settings = settings(Blend::Smudge);
        let mut doc = original.clone();
        let buffer = paint(&mut doc, &settings);
        // The gap at x 10..20 fills with red dragged from the stripe behind it;
        // the row far from the stroke is untouched. Nothing turns blue.
        let gap = doc.layers[0].pixels.rgba(30 * 120 + 14);
        assert!(gap[3] > 60, "{gap:?}");
        assert!(gap[0] > 150 && gap[2] < 60, "{gap:?}");
        assert_eq!(
            doc.layers[0].pixels.rgba(2 * 120 + 14),
            original.layers[0].pixels.rgba(2 * 120 + 14)
        );
        assert!(buffer.has_visible_changes(&doc));
        buffer.restore(&mut doc);
        assert!(doc.layers[0].pixels == original.layers[0].pixels);
    }

    #[test]
    fn blur_softens_edges_and_live_slices_match_whole_dabs() {
        for blend in [Blend::Blur, Blend::Smudge] {
            let settings = settings(blend);
            let mut whole = striped();
            paint(&mut whole, &settings);
            // The live path paints each dab in resumable slices.
            let mut sliced = striped();
            let mut buffer = StrokeBuffer::default();
            let shared = std::sync::Arc::new(settings.clone());
            for dab in dabs(&settings) {
                let mut cursor = buffer.begin_dab(&sliced, dab, shared.clone()).unwrap();
                while !cursor.step(&mut buffer, &mut sliced, 1).unwrap().complete {}
            }
            assert!(
                sliced.layers[0].pixels == whole.layers[0].pixels,
                "{blend:?} slices differ"
            );
            if blend == Blend::Blur {
                // The stripe edge at x = 10 is now a ramp.
                let edge = whole.layers[0].pixels.rgba(30 * 120 + 10);
                assert!(edge[3] > 20 && edge[3] < 235, "{edge:?}");
            }
        }
    }

    #[test]
    fn lock_transparency_and_the_eraser_end() {
        let mut settings = settings(Blend::Smudge);
        let mut locked = striped();
        locked.layers[0].alpha_locked = true;
        paint(&mut locked, &settings);
        let original = striped();
        for i in 0..120 * 60 {
            assert_eq!(
                locked.layers[0].pixels.rgba(i)[3],
                original.layers[0].pixels.rgba(i)[3]
            );
        }
        // The eraser end erases as with any brush.
        settings.erase = true;
        let mut erased = striped();
        paint(&mut erased, &settings);
        // Opacity 0.9 is the stroke ceiling, so a tenth remains.
        assert!(erased.layers[0].pixels.rgba(30 * 120 + 24)[3] <= 26);
    }

    #[test]
    fn blur_averages_inside_the_copy_and_smudge_reads_behind() {
        // A 9 x 1 strip: one white pixel in the middle of black.
        let read = |x: usize, _y: usize| {
            if x == 4 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        };
        let mut scratch = Scratch::default();
        capture(
            &mut scratch,
            read,
            [9, 1],
            [4, 0, 4, 0],
            Blend::Blur,
            [0.0; 2],
            4.0,
        );
        let center = scratch.source(Blend::Blur, [0.0; 2], 4, 0);
        assert!(center[0] < 0.6 && center[0] > 0.1, "{center:?}");
        assert!((center[3] - 1.0).abs() < 1e-6, "edges do not darken alpha");
        let mut scratch = Scratch::default();
        capture(
            &mut scratch,
            read,
            [9, 1],
            [4, 0, 6, 0],
            Blend::Smudge,
            [2.0, 0.0],
            4.0,
        );
        // Pixel 6 reads from pixel 4, where the tip was before.
        assert_eq!(scratch.source(Blend::Smudge, [2.0, 0.0], 6, 0)[0], 1.0);
        let half = scratch.source(Blend::Smudge, [1.5, 0.0], 6, 0)[0];
        assert!((half - 0.5).abs() < 1e-6, "{half}");
    }
}
