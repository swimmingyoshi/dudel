//! Filters that look at neighboring pixels or position: Gaussian blur,
//! sharpen (unsharp mask) and add noise. They run on the active layer inside
//! the selection, like the color adjustments, through the same preview.
use crate::{document::Layer, plugins::FilterControl, selection::Selection, selection_mask::Mask};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Filter {
    /// Gaussian blur with this radius (standard deviation) in pixels.
    Blur { radius: f32 },
    /// Unsharp mask: push each pixel away from its blurred surroundings.
    Sharpen { amount: f32, radius: f32 },
    /// Grain: `amount` is the largest change, as a share of full brightness.
    Noise { amount: f32, monochrome: bool },
}

/// Fixed so the preview and the applied result match exactly.
const NOISE_SEED: u32 = 0x5EED_D0D1;

fn canceled(control: &FilterControl) -> bool {
    control.cancel.load(Ordering::Relaxed)
}

/// A deterministic value in -1..=1 for this pixel and channel.
fn noise(x: usize, y: usize, channel: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ channel.wrapping_mul(0xC2B2_AE3D)
        ^ NOISE_SEED;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// Below this sigma an exact Gaussian kernel is used; above it, three
/// extended box passes (same variance, smooth in sigma, cost independent of
/// the radius).
const EXACT_SIGMA: f32 = 3.0;

/// How one line is blurred.
enum Line {
    /// Half a normalized Gaussian kernel: centre weight first.
    Kernel(Vec<f32>),
    /// A box of radius `r` with `edge` weight on the next pixel each side,
    /// applied three times (Gwosdek et al., extended box filtering).
    Boxes { r: usize, edge: f64 },
}

impl Line {
    fn new(sigma: f32) -> Self {
        if sigma < EXACT_SIGMA {
            let reach = (sigma * 3.0).ceil() as usize;
            let mut half: Vec<f32> = (0..=reach)
                .map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp())
                .collect();
            let total = half[0] + 2.0 * half[1..].iter().sum::<f32>();
            half.iter_mut().for_each(|k| *k /= total);
            return Self::Kernel(half);
        }
        // Each pass carries a third of the variance. Pick the largest whole
        // box not exceeding it, then the fractional edge weight that makes
        // up the rest exactly.
        let t = f64::from(sigma * sigma) / 3.0;
        let mut r = ((3.0 * t + 0.25).sqrt() - 0.5).floor().max(0.0) as usize;
        while ((r + 1) * (r + 2)) as f64 / 3.0 <= t {
            r += 1;
        }
        let rf = r as f64;
        let edge =
            (2.0 * rf + 1.0) * (t - rf * (rf + 1.0) / 3.0) / (2.0 * ((rf + 1.0).powi(2) - t));
        Self::Boxes {
            r,
            edge: edge.clamp(0.0, 1.0),
        }
    }
    /// How far a blurred pixel reads, in pixels.
    fn reach(&self) -> usize {
        match self {
            Self::Kernel(half) => half.len() - 1,
            Self::Boxes { r, .. } => 3 * (r + 1),
        }
    }
    /// Blur `line` in place; `scratch` is as long. Edges repeat.
    fn apply(&self, line: &mut [f32], scratch: &mut [f32]) {
        let n = line.len();
        if n == 0 {
            return;
        }
        let last = n as isize - 1;
        match self {
            Self::Kernel(half) => {
                scratch.copy_from_slice(line);
                let src = &*scratch;
                let at = |i: isize| src[i.clamp(0, last) as usize];
                let reach = half.len() - 1;
                for (i, out) in line.iter_mut().enumerate() {
                    let mut sum = half[0] * src[i];
                    if i >= reach && i + reach < n {
                        // Inside: no edge to repeat.
                        for (k, weight) in half.iter().enumerate().skip(1) {
                            sum += weight * (src[i - k] + src[i + k]);
                        }
                    } else {
                        let i = i as isize;
                        for (k, weight) in half.iter().enumerate().skip(1) {
                            let k = k as isize;
                            sum += weight * (at(i - k) + at(i + k));
                        }
                    }
                    *out = sum;
                }
            }
            Self::Boxes { r, edge } => {
                let r = *r as isize;
                let scale = 1.0 / (2.0 * r as f64 + 1.0 + 2.0 * edge);
                for _ in 0..3 {
                    scratch.copy_from_slice(line);
                    let src = &*scratch;
                    let at = |i: isize| f64::from(src[i.clamp(0, last) as usize]);
                    // A running sum over [i - r, i + r], in f64 so long lines
                    // do not drift.
                    let mut sum: f64 = (-r..=r).map(at).sum();
                    for (i, out) in line.iter_mut().enumerate() {
                        let i = i as isize;
                        *out = ((sum + edge * (at(i - r - 1) + at(i + r + 1))) * scale) as f32;
                        sum += at(i + r + 1) - at(i - r);
                    }
                }
            }
        }
    }
}

/// Blur premultiplied planes (each `w` x `h`) with a Gaussian of standard
/// deviation `sigma`: rows, then columns in narrow strips, every plane's
/// pieces shared over the worker pool.
fn gaussian_planes(
    planes: &mut [Vec<f32>],
    w: usize,
    h: usize,
    sigma: f32,
    cancel: &(impl Fn() -> bool + Sync),
) -> Result<(), String> {
    if sigma < 0.3 || w == 0 || h == 0 {
        return Ok(());
    }
    const CHUNK: usize = 32;
    let line = Line::new(sigma);
    let collect = |results: Vec<std::thread::Result<Option<Vec<f32>>>>| {
        results
            .into_iter()
            .map(|r| match r {
                Ok(Some(piece)) => Ok(piece),
                Ok(None) => Err("Filter canceled".to_owned()),
                Err(_) => Err("Blur failed".to_owned()),
            })
            .collect::<Result<Vec<_>, String>>()
    };
    // Rows: each job blurs CHUNK rows of one plane.
    let row_chunks = h.div_ceil(CHUNK);
    let pieces = {
        let source = &*planes;
        collect(crate::parallel::run(source.len() * row_chunks, |job| {
            if cancel() {
                return None;
            }
            let (plane, chunk) = (job / row_chunks, job % row_chunks);
            let (first, end) = (chunk * CHUNK, ((chunk + 1) * CHUNK).min(h));
            let mut out = source[plane][first * w..end * w].to_vec();
            let mut scratch = vec![0.0f32; w];
            for row in out.chunks_mut(w) {
                line.apply(row, &mut scratch);
            }
            Some(out)
        }))?
    };
    for (job, piece) in pieces.into_iter().enumerate() {
        let (plane, chunk) = (job / row_chunks, job % row_chunks);
        let first = chunk * CHUNK * w;
        planes[plane][first..first + piece.len()].copy_from_slice(&piece);
    }
    // Columns: each job copies a strip of CHUNK columns into lines, blurs
    // them, and hands the lines back.
    let column_chunks = w.div_ceil(CHUNK);
    let pieces = {
        let source = &*planes;
        collect(crate::parallel::run(source.len() * column_chunks, |job| {
            if cancel() {
                return None;
            }
            let (plane, chunk) = (job / column_chunks, job % column_chunks);
            let (first, end) = (chunk * CHUNK, ((chunk + 1) * CHUNK).min(w));
            let strip = end - first;
            let data = &source[plane];
            let mut lines = vec![0.0f32; strip * h];
            for y in 0..h {
                for (c, value) in data[y * w + first..y * w + end].iter().enumerate() {
                    lines[c * h + y] = *value;
                }
            }
            let mut scratch = vec![0.0f32; h];
            for column in lines.chunks_mut(h) {
                line.apply(column, &mut scratch);
            }
            Some(lines)
        }))?
    };
    for (job, lines) in pieces.into_iter().enumerate() {
        let (plane, chunk) = (job / column_chunks, job % column_chunks);
        let first = chunk * CHUNK;
        let strip = lines.len() / h;
        let data = &mut planes[plane];
        for y in 0..h {
            let row = &mut data[y * w + first..y * w + first + strip];
            for (c, value) in row.iter_mut().enumerate() {
                *value = lines[c * h + y];
            }
        }
    }
    Ok(())
}

/// How far the blurs read around what they change, in pixels.
fn blur_reach(sigma: f32) -> usize {
    if sigma < 0.3 {
        0
    } else {
        Line::new(sigma).reach() + 1
    }
}

/// Apply `filter` to `layer` inside the selection. Soft selection edges blend
/// the result in; Lock transparency keeps each pixel's alpha.
pub(crate) fn apply(
    layer: &mut Layer,
    selection: Option<Selection>,
    mask: Option<&Mask>,
    [width, height]: [usize; 2],
    filter: Filter,
    control: &FilterControl,
) -> Result<(), String> {
    let cancel = || canceled(control);
    if cancel() {
        return Err("Filter canceled".into());
    }
    let mut rect = selection.map_or([0, 0, width, height], |s| {
        [
            s.left.min(width),
            s.top.min(height),
            s.right.min(width),
            s.bottom.min(height),
        ]
    });
    if let Some(bounds) = mask.and_then(|m| m.bounds()) {
        rect = [
            rect[0].max(bounds.left),
            rect[1].max(bounds.top),
            rect[2].min(bounds.right),
            rect[3].min(bounds.bottom),
        ];
    }
    let [left, top, right, bottom] = rect;
    if left >= right || top >= bottom {
        return Ok(());
    }
    // Blurs read pixels around the selection too.
    let reach = match filter {
        Filter::Blur { radius } | Filter::Sharpen { radius, .. } => blur_reach(radius),
        Filter::Noise { .. } => 0,
    };
    let source = [
        left.saturating_sub(reach),
        top.saturating_sub(reach),
        (right + reach).min(width),
        (bottom + reach).min(height),
    ];
    let (sw, sh) = (source[2] - source[0], source[3] - source[1]);
    // One read of everything the filter looks at.
    let bytes = layer.pixels.region(width, source);
    // Premultiplied, blurred planes for the blurs: alpha, then red, green, blue.
    let blurred: Option<[Vec<f32>; 4]> = match filter {
        Filter::Blur { radius } | Filter::Sharpen { radius, .. } => {
            // Split into premultiplied planes.
            let mut planes: [Vec<f32>; 4] = std::array::from_fn(|_| Vec::with_capacity(sw * sh));
            for p in bytes.chunks_exact(4) {
                let alpha = f32::from(p[3]) / 255.0;
                planes[0].push(alpha);
                planes[1].push(f32::from(p[0]) * alpha);
                planes[2].push(f32::from(p[1]) * alpha);
                planes[3].push(f32::from(p[2]) * alpha);
            }
            if cancel() {
                return Err("Filter canceled".into());
            }
            gaussian_planes(&mut planes, sw, sh, radius, &cancel)?;
            Some(planes)
        }
        Filter::Noise { .. } => None,
    };
    let alpha_locked = layer.alpha_locked;
    // What the filter makes of one pixel.
    let filtered = |x: usize, y: usize, before: [u8; 4]| -> [u8; 4] {
        if selection.is_some_and(|s| !s.contains(x, y)) {
            return before;
        }
        let coverage = mask.map_or(255, |m| m.at(x, y));
        if coverage == 0 {
            return before;
        }
        let mut after = match (filter, &blurred) {
            (Filter::Blur { .. }, Some([a, r, g, b])) => {
                let i = (y - source[1]) * sw + (x - source[0]);
                let alpha = a[i];
                if alpha * 255.0 < 0.5 {
                    // Blurred to nothing: clear, keeping the hidden color.
                    [before[0], before[1], before[2], 0]
                } else {
                    let c = |v: f32| (v / alpha).round().clamp(0.0, 255.0) as u8;
                    [c(r[i]), c(g[i]), c(b[i]), (alpha * 255.0).round() as u8]
                }
            }
            (Filter::Sharpen { amount, .. }, Some([a, r, g, b])) => {
                if before[3] == 0 {
                    return before;
                }
                let i = (y - source[1]) * sw + (x - source[0]);
                let alpha = a[i].max(1.0 / 512.0);
                let around = [r[i] / alpha, g[i] / alpha, b[i] / alpha];
                let mut p = before;
                for c in 0..3 {
                    let v = f32::from(before[c]);
                    p[c] = (v + amount * (v - around[c])).round().clamp(0.0, 255.0) as u8;
                }
                p
            }
            (Filter::Noise { amount, monochrome }, _) => {
                if before[3] == 0 {
                    return before;
                }
                let mut p = before;
                for c in 0..3 {
                    let n = noise(x, y, if monochrome { 0 } else { c as u32 });
                    let v = f32::from(before[c]) + n * amount * 255.0;
                    p[c] = v.round().clamp(0.0, 255.0) as u8;
                }
                p
            }
            _ => unreachable!("blurred planes exist for blur and sharpen"),
        };
        if alpha_locked {
            after[3] = before[3];
        }
        crate::selection_mask::mix_rgba(before, after, coverage)
    };
    // Each output row depends only on what was read, so rows are computed
    // in parallel and written back in one go.
    let (rw, rh) = (right - left, bottom - top);
    let before_at = |x: usize, y: usize| -> [u8; 4] {
        let i = ((y - source[1]) * sw + (x - source[0])) * 4;
        [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
    };
    const ROWS: usize = 16;
    let results = crate::parallel::run(rh.div_ceil(ROWS), |chunk| {
        if cancel() {
            return None;
        }
        let (first, end) = (top + chunk * ROWS, (top + (chunk + 1) * ROWS).min(bottom));
        let mut out = Vec::with_capacity((end - first) * rw * 4);
        let mut changed = false;
        for y in first..end {
            for x in left..right {
                let before = before_at(x, y);
                let after = filtered(x, y, before);
                changed |= after != before;
                out.extend_from_slice(&after);
            }
        }
        Some((out, changed))
    });
    let mut out = Vec::with_capacity(rw * rh * 4);
    let mut changed = false;
    for result in results {
        match result {
            Ok(Some((rows, any))) => {
                out.extend(rows);
                changed |= any;
            }
            Ok(None) => return Err("Filter canceled".into()),
            Err(_) => return Err("Filter failed".into()),
        }
    }
    let mut writer = layer.pixels.writer();
    if changed {
        writer.write_region(width, rect, &out);
        writer.reclaim_region(width, rect);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn control() -> FilterControl {
        FilterControl::default()
    }
    /// A 21 x 21 layer with one opaque red pixel in the middle.
    fn dot() -> Document {
        let mut doc = Document::new(21, 21);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(10 * 21 + 10)
            .copy_from_slice(&[255, 0, 0, 255]);
        doc
    }

    #[test]
    fn blur_spreads_alpha_without_darkening_the_color() {
        let mut doc = dot();
        let [w, h] = [doc.width, doc.height];
        apply(
            &mut doc.layers[0],
            None,
            None,
            [w, h],
            Filter::Blur { radius: 2.0 },
            &control(),
        )
        .unwrap();
        let center = doc.layers[0].pixels.rgba(10 * 21 + 10);
        let near = doc.layers[0].pixels.rgba(10 * 21 + 12);
        assert!(center[3] < 255 && near[3] > 0, "{center:?} {near:?}");
        // Premultiplied blurring keeps the spread paint red, not dark.
        assert!(near[0] > 240 && near[1] == 0, "{near:?}");
        let far = doc.layers[0].pixels.rgba(0);
        assert_eq!(far[3], 0);
    }

    #[test]
    fn blur_follows_the_radius_smoothly_and_clears_what_it_spreads() {
        // The centre of a blurred dot fades steadily as the radius grows:
        // every radius gives its own result, the smallest ones included.
        let center = |radius: f32| {
            let mut doc = dot();
            let [w, h] = [doc.width, doc.height];
            apply(
                &mut doc.layers[0],
                None,
                None,
                [w, h],
                Filter::Blur { radius },
                &control(),
            )
            .unwrap();
            doc.layers[0].pixels.rgba(10 * 21 + 10)[3]
        };
        let fades: Vec<u8> = [0.4, 0.5, 0.6, 0.8, 1.0, 1.2, 1.4].map(center).into();
        assert!(fades[0] < 255, "even 0.4 px blurs: {fades:?}");
        assert!(fades.windows(2).all(|p| p[0] > p[1]), "{fades:?}");
        // A wide blur spreads a lone dot to nothing: it does not survive sharp.
        assert_eq!(center(10.0), 0);
    }

    #[test]
    fn blur_keeps_an_opaque_layer_opaque_to_its_edges() {
        let mut doc = Document::new(40, 30);
        {
            let mut writer = doc.layers[0].pixels.writer();
            for i in 0..40 * 30 {
                let v = if i % 40 < 20 { 30 } else { 220 };
                writer.rgba_mut(i).copy_from_slice(&[v, v, v, 255]);
            }
        }
        apply(
            &mut doc.layers[0],
            None,
            None,
            [40, 30],
            Filter::Blur { radius: 6.0 },
            &control(),
        )
        .unwrap();
        for i in [0, 39, 40 * 29, 40 * 30 - 1, 15 * 40 + 20] {
            assert_eq!(doc.layers[0].pixels.rgba(i)[3], 255, "pixel {i}");
        }
        // The hard edge in the middle became a ramp; the far sides kept
        // their shade (edges repeat rather than fading in black or white).
        let shade = |x: usize| doc.layers[0].pixels.rgba(15 * 40 + x)[0];
        assert!(shade(19) > 60 && shade(19) < 190, "{}", shade(19));
        assert!(
            shade(0) < 35 && shade(39) > 215,
            "{} {}",
            shade(0),
            shade(39)
        );
    }

    #[test]
    fn the_line_blur_matches_a_true_gaussian() {
        for sigma in [0.5f32, 1.0, 2.0, 2.9, 3.0, 6.0, 20.0] {
            let n = 301;
            let mut line = vec![0.0f32; n];
            line[150] = 1.0;
            let mut scratch = vec![0.0f32; n];
            Line::new(sigma).apply(&mut line, &mut scratch);
            let s = f64::from(sigma);
            let g =
                |d: f64| (-d * d / (2.0 * s * s)).exp() / ((2.0 * std::f64::consts::PI).sqrt() * s);
            let error = (0..n)
                .map(|i| (f64::from(line[i]) - g(i as f64 - 150.0)).abs())
                .fold(0.0, f64::max)
                / g(0.0);
            assert!(error < 0.07, "sigma {sigma}: {error}");
            assert!(
                (line.iter().sum::<f32>() - 1.0).abs() < 1e-4,
                "sigma {sigma}"
            );
        }
    }

    #[test]
    fn sharpen_raises_edge_contrast_and_noise_is_repeatable() {
        let mut doc = Document::new(8, 1);
        for x in 0..8 {
            let v = if x < 4 { 100 } else { 150 };
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(x)
                .copy_from_slice(&[v, v, v, 255]);
        }
        let [w, h] = [doc.width, doc.height];
        let sharpen = Filter::Sharpen {
            amount: 1.0,
            radius: 1.0,
        };
        apply(&mut doc.layers[0], None, None, [w, h], sharpen, &control()).unwrap();
        assert!(doc.layers[0].pixels.rgba(3)[0] < 100);
        assert!(doc.layers[0].pixels.rgba(4)[0] > 150);
        let grain = Filter::Noise {
            amount: 0.2,
            monochrome: true,
        };
        let (mut a, mut b) = (doc.clone(), doc.clone());
        apply(&mut a.layers[0], None, None, [w, h], grain, &control()).unwrap();
        apply(&mut b.layers[0], None, None, [w, h], grain, &control()).unwrap();
        assert!((0..8).all(|i| a.layers[0].pixels.rgba(i) == b.layers[0].pixels.rgba(i)));
        assert!((0..8).any(|i| a.layers[0].pixels.rgba(i) != doc.layers[0].pixels.rgba(i)));
        let p = a.layers[0].pixels.rgba(2);
        assert!(
            p[0] == p[1] && p[1] == p[2],
            "monochrome grain keeps gray gray"
        );
    }

    #[test]
    fn selection_and_alpha_lock_limit_the_change() {
        let mut doc = dot();
        let [w, h] = [doc.width, doc.height];
        let selection = Some(Selection {
            left: 0,
            top: 0,
            right: 11,
            bottom: 21,
        });
        apply(
            &mut doc.layers[0],
            selection,
            None,
            [w, h],
            Filter::Blur { radius: 2.0 },
            &control(),
        )
        .unwrap();
        assert!(doc.layers[0].pixels.rgba(10 * 21 + 9)[3] > 0);
        assert_eq!(doc.layers[0].pixels.rgba(10 * 21 + 12)[3], 0);
        let mut locked = dot();
        locked.layers[0].alpha_locked = true;
        apply(
            &mut locked.layers[0],
            None,
            None,
            [w, h],
            Filter::Blur { radius: 2.0 },
            &control(),
        )
        .unwrap();
        assert_eq!(locked.layers[0].pixels.rgba(10 * 21 + 10)[3], 255);
        assert_eq!(locked.layers[0].pixels.rgba(10 * 21 + 12)[3], 0);
    }

    #[test]
    fn canceling_leaves_the_layer_untouched() {
        let mut doc = dot();
        let [w, h] = [doc.width, doc.height];
        let control = control();
        control.cancel.store(true, Ordering::Relaxed);
        assert!(
            apply(
                &mut doc.layers[0],
                None,
                None,
                [w, h],
                Filter::Blur { radius: 2.0 },
                &control
            )
            .is_err()
        );
        assert_eq!(doc.layers[0].pixels.rgba(10 * 21 + 12)[3], 0);
    }
}

#[cfg(test)]
#[test]
#[ignore = "measurement"]
fn blur_measure() {
    // A unit impulse blurred in floating point, against the true Gaussian.
    for sigma in [0.5f32, 1.0, 1.5, 2.0, 2.9, 3.0, 4.0, 6.0, 10.0, 30.0] {
        let n = 401;
        let mut plane = vec![0.0f32; n];
        plane[200] = 1.0;
        let mut scratch = vec![0.0f32; n];
        Line::new(sigma).apply(&mut plane, &mut scratch);
        let s = f64::from(sigma);
        let g = |d: f64| (-d * d / (2.0 * s * s)).exp() / ((2.0 * std::f64::consts::PI).sqrt() * s);
        let err = (0..n)
            .map(|i| (f64::from(plane[i]) - g(i as f64 - 200.0)).abs())
            .fold(0.0, f64::max)
            / g(0.0);
        let sum: f32 = plane.iter().sum();
        println!(
            "sigma {sigma:5}: max error {:.1}% of peak, total {sum:.5}",
            err * 100.0
        );
    }
    let ctl = FilterControl::default();
    for (side, radius) in [(2048usize, 2.0f32), (2048, 4.0), (2048, 40.0), (4096, 40.0)] {
        let mut doc = crate::document::Document::new(side, side);
        {
            let mut w = doc.layers[0].pixels.writer();
            for i in (0..side * side).step_by(7) {
                w.rgba_mut(i).copy_from_slice(&[200, 100, 50, 255]);
            }
        }
        let t = std::time::Instant::now();
        apply(
            &mut doc.layers[0],
            None,
            None,
            [side, side],
            Filter::Blur { radius },
            &ctl,
        )
        .unwrap();
        println!(
            "{side}² r{radius}: {:.0} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
}
