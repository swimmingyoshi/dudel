//! CSP vector layers. CSP saves the strokes, not their pixels, and redraws
//! them when it opens a file; Dudel draws them as round, anti-aliased brush
//! lines into an ordinary raster layer.
//!
//! The layout was read from the owner's files (every observed vector layer
//! parses to its last byte): the external body is a big-endian length and
//! then strokes, each an 88-byte header followed by 88-byte points.
//!
//! Stroke header: sizes (88, 72, 88, 88), point count, flags 0x2081, the
//! stroke's integer bounds, main color and sub color (three 32-bit channels
//! each), opacity (f64), brush style id, brush width (f64) and a stroke tag.
//! Point: x and y (f64), integer dab bounds, a flag word, then pressure (f32)
//! and ten further values Dudel does not use. A point's radius is width x
//! pressure, which matched CSP's saved previews best of the models tried.
//! The brush's anti-aliasing level (0 none to 3 strong) moves the edge's
//! one-pixel ramp inward and widens it a little: at 0 it is centered on the
//! radius, at 3 it ends there and spans 1.5 px, which makes light strokes
//! thinner and fainter as in CSP (fitted on previews saved at full size).
use crate::pixels::Pixels;

const HEADER: usize = 88;
const POINT: usize = 88;
const FLAGS: u32 = 0x2081;
/// Bounds that keep a damaged file from asking for unbounded work.
const MAX_STROKES: usize = 200_000;
const MAX_POINTS: usize = 4_000_000;
const MAX_WIDTH: f64 = 10_000.0;
const MAX_COORDINATE: f64 = 1e6;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Stroke {
    pub color: [u8; 3],
    pub opacity: f32,
    pub width: f64,
    /// `BrushStyle.MainId` the stroke was drawn with.
    pub style: u32,
    /// The brush style's anti-aliasing level, 0 (none) to 3 (strong).
    pub antialias: u8,
    pub points: Vec<Point>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}
fn f64_at(data: &[u8], at: usize) -> f64 {
    f64::from_be_bytes(data[at..at + 8].try_into().unwrap())
}
fn f32_at(data: &[u8], at: usize) -> f32 {
    f32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}

/// Strokes of one vector layer's external body (after its identifier).
pub(crate) fn parse(body: &[u8]) -> Result<Vec<Stroke>, String> {
    if body.len() < 8 {
        return Err("Truncated vector data".into());
    }
    let length = u64::from_be_bytes(body[..8].try_into().unwrap());
    let data = &body[8..];
    if length != data.len() as u64 {
        return Err("Vector data length disagrees with its chunk".into());
    }
    let mut strokes = Vec::new();
    let mut points_total = 0usize;
    let mut at = 0;
    while at < data.len() {
        if data.len() - at < HEADER || strokes.len() >= MAX_STROKES {
            return Err("Vector stroke list exceeds its data or limits".into());
        }
        let h = &data[at..at + HEADER];
        let words: [u32; 6] = std::array::from_fn(|i| u32_at(h, i * 4));
        let count = words[4] as usize;
        if words[..4] != [HEADER as u32, 72, HEADER as u32, POINT as u32]
            || words[5] != FLAGS
            || count == 0
        {
            return Err("Unrecognized vector stroke header".into());
        }
        points_total += count;
        let end = count
            .checked_mul(POINT)
            .and_then(|bytes| bytes.checked_add(at + HEADER))
            .filter(|end| *end <= data.len() && points_total <= MAX_POINTS)
            .ok_or("Vector stroke points exceed their data or limits")?;
        let channel = |i: usize| {
            (f64::from(u32_at(h, 40 + i * 4)) / f64::from(u32::MAX) * 255.0).round() as u8
        };
        let opacity = f64_at(h, 64);
        let width = f64_at(h, 76);
        if !(0.0..=1.0).contains(&opacity) || !(width > 0.0 && width <= MAX_WIDTH) {
            return Err("Vector stroke opacity or width out of range".into());
        }
        let mut points = Vec::with_capacity(count);
        for p in data[at + HEADER..end].chunks_exact(POINT) {
            let (x, y) = (f64_at(p, 0), f64_at(p, 8));
            let pressure = f64::from(f32_at(p, 36));
            if !(x.abs() <= MAX_COORDINATE && y.abs() <= MAX_COORDINATE)
                || !(0.0..=1.0).contains(&pressure)
            {
                return Err("Vector point out of range".into());
            }
            points.push(Point { x, y, pressure });
        }
        strokes.push(Stroke {
            color: [channel(0), channel(1), channel(2)],
            opacity: opacity as f32,
            width,
            style: u32_at(h, 72),
            antialias: 0,
            points,
        });
        at = end;
    }
    Ok(strokes)
}

impl Stroke {
    /// The canvas pixels the stroke can cover, if any.
    pub(crate) fn rect(&self, [width, height]: [usize; 2]) -> Option<[usize; 4]> {
        let mut bounds = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for p in &self.points {
            let r = self.width * p.pressure + 1.0;
            bounds = [
                bounds[0].min(p.x - r),
                bounds[1].min(p.y - r),
                bounds[2].max(p.x + r),
                bounds[3].max(p.y + r),
            ];
        }
        let clip = |v: f64, limit: usize| v.floor().clamp(0.0, limit as f64) as usize;
        let rect = [
            clip(bounds[0], width),
            clip(bounds[1], height),
            clip(bounds[2] + 1.0, width),
            clip(bounds[3] + 1.0, height),
        ];
        (rect[0] < rect[2] && rect[1] < rect[3]).then_some(rect)
    }
    /// Paint the stroke source-over onto `rgba`, straight-alpha pixels of
    /// the canvas rectangle `area`. Whole-canvas and partial painting give
    /// the same bytes where they overlap.
    pub(crate) fn paint(
        &self,
        rgba: &mut [u8],
        area: [usize; 4],
        canvas: [usize; 2],
        coverage: &mut Vec<f32>,
    ) {
        let Some(rect) = self.rect(canvas) else {
            return;
        };
        let [left, top] = [rect[0].max(area[0]), rect[1].max(area[1])];
        let [right, bottom] = [rect[2].min(area[2]), rect[3].min(area[3])];
        if left >= right || top >= bottom {
            return;
        }
        let [width, height] = canvas;
        let clip = |v: f64, limit: usize| v.floor().clamp(0.0, limit as f64) as usize;
        let radius = |p: &Point| self.width * p.pressure;
        let level = f64::from(self.antialias.min(3)) / 3.0;
        let (inset, soft) = (0.5 * (1.0 - level), 1.0 + 0.5 * level);
        let span = right - left;
        coverage.clear();
        coverage.resize(span * (bottom - top), 0f32);
        // Each segment is a capsule whose radius runs from one point's to the
        // next; a pixel takes the largest coverage of any segment.
        let points = &self.points;
        let segments = points.len().saturating_sub(1).max(1);
        for i in 0..segments {
            let a = points[i];
            let b = points[(i + 1).min(points.len() - 1)];
            let (ra, rb) = (radius(&a), radius(&b));
            let reach = ra.max(rb) + 1.0;
            let x0 = clip(a.x.min(b.x) - reach, width).max(left);
            let y0 = clip(a.y.min(b.y) - reach, height).max(top);
            let x1 = clip(a.x.max(b.x) + reach + 1.0, width).min(right);
            let y1 = clip(a.y.max(b.y) + reach + 1.0, height).min(bottom);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let length2 = dx * dx + dy * dy;
            for y in y0..y1 {
                let py = y as f64 + 0.5;
                let row = (y - top) * span;
                for x in x0..x1 {
                    let px = x as f64 + 0.5;
                    let t = if length2 > 0.0 {
                        (((px - a.x) * dx + (py - a.y) * dy) / length2).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let (qx, qy) = (a.x + dx * t, a.y + dy * t);
                    let r = ra + (rb - ra) * t;
                    let distance = ((px - qx).powi(2) + (py - qy).powi(2)).sqrt();
                    // A line thinner than a pixel covers it only by its width.
                    let c = ((r - distance + inset) / soft).clamp(0.0, 1.0).min(2.0 * r) as f32;
                    let cell = &mut coverage[row + x - left];
                    if c > *cell {
                        *cell = c;
                    }
                }
            }
        }
        // Source-over in straight alpha.
        let stride = area[2] - area[0];
        for y in top..bottom {
            for x in left..right {
                let source = coverage[(y - top) * span + x - left] * self.opacity;
                if source <= 0.0 {
                    continue;
                }
                let i = ((y - area[1]) * stride + x - area[0]) * 4;
                let below = f32::from(rgba[i + 3]) / 255.0;
                let alpha = source + below * (1.0 - source);
                for c in 0..3 {
                    let value = (f32::from(self.color[c]) * source
                        + f32::from(rgba[i + c]) * below * (1.0 - source))
                        / alpha;
                    rgba[i + c] = value.round().clamp(0.0, 255.0) as u8;
                }
                rgba[i + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    /// Paint the stroke onto a layer's pixels, as `render` would.
    pub(crate) fn paint_onto(&self, pixels: &mut Pixels, canvas: [usize; 2]) {
        let Some(rect) = self.rect(canvas) else {
            return;
        };
        let mut rgba = pixels.region(canvas[0], rect);
        self.paint(&mut rgba, rect, canvas, &mut Vec::new());
        pixels.writer().write_region(canvas[0], rect, &rgba);
    }
}

/// Draw strokes in order onto a transparent `canvas`-sized layer.
pub(crate) fn render(
    strokes: &[Stroke],
    canvas: [usize; 2],
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let [width, height] = canvas;
    let mut pixels = Pixels::sparse(width, height);
    if strokes.is_empty() {
        return Ok(pixels);
    }
    let mut rgba = vec![0u8; width * height * 4];
    let mut coverage = Vec::new();
    // Rows any stroke touched; the rest stay sparse.
    let mut rows = [height, 0];
    for stroke in strokes {
        if cancelled() {
            return Err("Vector drawing canceled".into());
        }
        if let Some(rect) = stroke.rect(canvas) {
            rows = [rows[0].min(rect[1]), rows[1].max(rect[3])];
            stroke.paint(&mut rgba, [0, 0, width, height], canvas, &mut coverage);
        }
    }
    {
        let mut writer = pixels.writer();
        for y in rows[0]..rows[1] {
            writer.write_region(
                width,
                [0, y, width, y + 1],
                &rgba[y * width * 4..(y + 1) * width * 4],
            );
        }
    }
    Ok(pixels)
}

#[cfg(test)]
pub(crate) fn encode(strokes: &[Stroke]) -> Vec<u8> {
    let mut data = Vec::new();
    for stroke in strokes {
        for word in [
            HEADER as u32,
            72,
            HEADER as u32,
            POINT as u32,
            stroke.points.len() as u32,
            FLAGS,
        ] {
            data.extend(word.to_be_bytes());
        }
        data.extend([0u8; 16]);
        for c in stroke.color {
            data.extend((u32::from(c) * 0x0101_0101).to_be_bytes());
        }
        data.extend([0xff; 12]);
        data.extend(f64::from(stroke.opacity).to_be_bytes());
        data.extend(stroke.style.to_be_bytes());
        data.extend(stroke.width.to_be_bytes());
        data.extend([0u8; 4]);
        for p in &stroke.points {
            data.extend(p.x.to_be_bytes());
            data.extend(p.y.to_be_bytes());
            data.extend([0u8; 20]);
            data.extend((p.pressure as f32).to_be_bytes());
            data.extend([0u8; 48]);
        }
    }
    let mut body = (data.len() as u64).to_be_bytes().to_vec();
    body.extend(data);
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(color: [u8; 3], y: f64) -> Stroke {
        Stroke {
            color,
            opacity: 1.0,
            width: 2.0,
            style: 1,
            antialias: 0,
            points: vec![
                Point {
                    x: 2.0,
                    y,
                    pressure: 1.0,
                },
                Point {
                    x: 18.0,
                    y,
                    pressure: 1.0,
                },
            ],
        }
    }

    #[test]
    fn strokes_round_trip_and_damage_is_rejected() {
        let strokes = vec![line([0, 0, 0], 5.0), line([255, 0, 0], 6.0)];
        let body = encode(&strokes);
        assert_eq!(parse(&body).unwrap(), strokes);
        assert!(parse(&body[..body.len() - 1]).is_err());
        let mut bad = body.clone();
        bad[8 + 20..8 + 24].copy_from_slice(&0u32.to_be_bytes());
        assert!(parse(&bad).is_err());
        let mut huge = body.clone();
        huge[8 + 16..8 + 20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse(&huge).is_err());
        let mut nan = body;
        nan[8 + 88..8 + 96].copy_from_slice(&f64::NAN.to_be_bytes());
        assert!(parse(&nan).is_err());
    }

    #[test]
    fn strokes_draw_round_lines_in_order() {
        let pixels = render(
            &[line([0, 0, 0], 5.0), line([255, 0, 0], 6.0)],
            [20, 12],
            &|| false,
        )
        .unwrap();
        let at = |x: usize, y: usize| pixels.rgba(y * 20 + x);
        // Radius 2 around y = 6: the red line covers the black one there.
        assert_eq!(at(10, 6), [255, 0, 0, 255]);
        assert_eq!(at(10, 3), [0, 0, 0, 255]);
        assert_eq!(at(10, 10)[3], 0);
        // Round caps reach past the end points by the radius.
        assert!(at(19, 6)[3] > 0 && at(0, 6)[3] > 0);
        // Strong anti-aliasing fades the edge inside the radius.
        let soft = Stroke {
            antialias: 3,
            ..line([0, 0, 0], 6.0)
        };
        let soft = render(&[soft], [20, 12], &|| false).unwrap();
        let hard = render(&[line([0, 0, 0], 6.0)], [20, 12], &|| false).unwrap();
        let edge = |p: &Pixels| p.rgba(4 * 20 + 10)[3];
        assert!(edge(&soft) < edge(&hard));
        assert_eq!(soft.rgba(6 * 20 + 10)[3], 255);
        assert!(render(&[line([0; 3], 5.0)], [20, 12], &|| true).is_err());
    }
}
