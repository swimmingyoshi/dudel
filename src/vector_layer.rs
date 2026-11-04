//! Vector layers: the layer keeps every brush stroke drawn on it as a
//! recording, and its pixels are those strokes painted in order.
//!
//! A stroke is recorded as the commands that drove the native stroke sampler
//! (pen samples in canvas pixels with the view zoom, stabilizer catch-ups and
//! resets), together with the brush, color and symmetry it was drawn with.
//! Replaying them through the same sampler and stroke buffer reproduces the
//! stroke exactly, so the layer can be redrawn after strokes are removed and
//! saved without its pixels changing. The layer's pixels stay an ordinary
//! raster, so compositing, effects, masks and export need nothing new.
use crate::{
    brush::Brush,
    document::Document,
    pixels::Pixels,
    stroke::Sample,
    stroke_buffer::{Settings, StrokeBuffer},
    symmetry::Symmetry,
};
use std::sync::{Arc, OnceLock};

/// Strokes a vector layer may hold, and commands one stroke may hold.
pub(crate) const MAX_STROKES: usize = 20_000;
const MAX_COMMANDS: usize = 200_000;

/// What drove the stroke sampler, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Command {
    /// A pen sample in canvas pixels; `lift` ends the stroke there.
    Sample {
        sample: Sample,
        zoom: f32,
        lift: bool,
    },
    /// Shift+click's anchor: primes the sampler without drawing.
    Seed { sample: Sample, zoom: f32 },
    /// A resting pen pulled the stabilized line this fraction of the way.
    CatchUp(f32),
    /// Sampling restarted (the pen left the canvas).
    Reset,
}

/// One recorded stroke.
#[derive(Clone)]
pub(crate) struct Stroke {
    pub brush: Arc<Brush>,
    pub color: [u8; 3],
    pub symmetry: Symmetry,
    pub commands: Vec<Command>,
    /// A line from a Clip Studio vector layer, drawn as CSP draws it (its
    /// width, pressure and anti-aliasing); `brush` and `commands` then go
    /// unused and `color` mirrors its color.
    pub csp: Option<Arc<crate::clip_vector::Stroke>>,
    /// A CSP line's start and end tapers, in canvas pixels along it (a
    /// stroke's are its brush's).
    pub csp_taper: [f32; 2],
    /// Where its dabs land, worked out on first use.
    footprint: OnceLock<Footprint>,
}
impl PartialEq for Stroke {
    fn eq(&self, other: &Self) -> bool {
        self.brush == other.brush
            && self.color == other.color
            && self.symmetry == other.symmetry
            && self.commands == other.commands
            && self.csp == other.csp
            && self.csp_taper == other.csp_taper
    }
}
impl std::fmt::Debug for Stroke {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stroke")
            .field("brush", &self.brush.name)
            .field("color", &self.color)
            .field("commands", &self.commands.len())
            .finish()
    }
}

/// The line's circles (center and drawn radius) and the bounds of all it
/// can paint.
#[derive(Clone, Default)]
struct Footprint {
    circles: Vec<[f32; 3]>,
    bounds: Option<[f32; 4]>,
    /// The line's middle as drawn (without symmetry copies).
    path: Vec<[f32; 2]>,
}

impl Stroke {
    pub(crate) fn new(brush: Brush, color: [u8; 3], symmetry: Symmetry) -> Self {
        Self {
            brush: Arc::new(brush),
            color,
            symmetry,
            commands: Vec::new(),
            csp: None,
            csp_taper: [0.0; 2],
            footprint: OnceLock::new(),
        }
    }
    /// A Clip Studio vector line, kept as CSP drew it.
    pub(crate) fn from_csp(line: crate::clip_vector::Stroke) -> Self {
        Self {
            brush: Arc::new(Brush::default()),
            color: line.color,
            symmetry: Symmetry::default(),
            commands: Vec::new(),
            csp: Some(Arc::new(line)),
            csp_taper: [0.0; 2],
            footprint: OnceLock::new(),
        }
    }
    /// The line as circles of its drawn radius, about a pixel apart along
    /// a CSP line, and at the brush's dabs otherwise.
    fn line(&self) -> Vec<(Sample, f32)> {
        let Some(line) = self.drawn_csp() else {
            let radius = self.brush.size * 0.5;
            return self.dabs().into_iter().map(|d| (d, radius)).collect();
        };
        let at = |p: &crate::clip_vector::Point| Sample {
            x: p.x as f32,
            y: p.y as f32,
            pressure: p.pressure as f32,
            pressure_known: true,
            ..Default::default()
        };
        let radius = |pressure: f32| (line.width as f32 * pressure).max(0.5);
        let mut out = Vec::new();
        for pair in line.points.windows(2) {
            let (a, b) = (at(&pair[0]), at(&pair[1]));
            let steps = ((b.x - a.x).hypot(b.y - a.y).ceil() as usize).clamp(1, 4096);
            for i in 0..steps {
                let t = i as f32 / steps as f32;
                let s = Sample {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    pressure: a.pressure + (b.pressure - a.pressure) * t,
                    ..a
                };
                out.push((s, radius(s.pressure)));
            }
        }
        if let Some(last) = line.points.last() {
            let s = at(last);
            out.push((s, radius(s.pressure)));
        }
        out
    }
    fn settings(&self, layer: usize) -> Settings {
        Settings {
            target: crate::paint_target::Target::Raster,
            brush: (*self.brush).clone(),
            color: self.color,
            erase: false,
            layer,
            symmetry: self.symmetry,
        }
    }
    /// The sampler's dabs, as the canvas produced them.
    fn dabs(&self) -> Vec<Sample> {
        let brush = &self.brush;
        let mut sampler = crate::stroke::Stroke::default();
        let mut dabs = Vec::new();
        for command in &self.commands {
            match *command {
                Command::Sample { sample, zoom, lift } => dabs.extend(if lift {
                    sampler.finish_at_zoom(sample, brush, zoom)
                } else {
                    sampler.sample_at_zoom(sample, brush, zoom)
                }),
                Command::Seed { sample, zoom } => {
                    let _ = sampler.sample_at_zoom(sample, brush, zoom);
                }
                Command::CatchUp(fraction) => dabs.extend(sampler.catch_up(fraction, brush)),
                Command::Reset => sampler.reset(),
            }
        }
        dabs
    }
    fn footprint(&self, size: [usize; 2]) -> &Footprint {
        self.footprint.get_or_init(|| {
            // Generous: pressure, dynamics and scatter never draw a dab
            // farther out than this.
            let reach = match &self.csp {
                Some(line) => line.width as f32 + 2.0,
                None => self.brush.size * (1.0 + self.brush.scatter) + 2.0,
            };
            let mut footprint = Footprint::default();
            for (dab, radius) in self.line() {
                footprint.path.push([dab.x, dab.y]);
                for ([x, y], _) in self.symmetry.copies(dab.x, dab.y, size[0], size[1]) {
                    footprint.circles.push([x, y, radius]);
                    let b = footprint.bounds.get_or_insert([x, y, x, y]);
                    *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                }
            }
            if let Some(b) = &mut footprint.bounds {
                *b = [b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach];
            }
            footprint
        })
    }
    /// The canvas rectangle the stroke can have painted, if any.
    pub(crate) fn bounds(&self, size: [usize; 2]) -> Option<[usize; 4]> {
        let b = self.footprint(size).bounds?;
        let clamp = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
        let rect = [
            clamp(b[0].floor(), size[0]),
            clamp(b[1].floor(), size[1]),
            clamp(b[2].ceil() + 1.0, size[0]),
            clamp(b[3].ceil() + 1.0, size[1]),
        ];
        (rect[0] < rect[2] && rect[1] < rect[3]).then_some(rect)
    }
    /// The middle of the line as drawn, in canvas pixels.
    pub(crate) fn path(&self, size: [usize; 2]) -> &[[f32; 2]] {
        &self.footprint(size).path
    }
    /// Whether a circle at (`x`, `y`) of `radius` touches the stroke's line.
    pub(crate) fn touches(&self, x: f32, y: f32, radius: f32, size: [usize; 2]) -> bool {
        let footprint = self.footprint(size);
        let Some(b) = footprint.bounds else {
            return false;
        };
        if x + radius < b[0] || x - radius > b[2] || y + radius < b[1] || y - radius > b[3] {
            return false;
        }
        // The line itself, not the generous reach used for redraw bounds.
        footprint.circles.iter().any(|&[cx, cy, line]| {
            let r = radius + line;
            (cx - x) * (cx - x) + (cy - y) * (cy - y) <= r * r
        })
    }
    /// Paint the stroke onto layer `layer` of `doc`, as it was drawn.
    pub(crate) fn paint(&self, doc: &mut Document, layer: usize) {
        if let Some(line) = self.drawn_csp() {
            let size = [doc.width, doc.height];
            line.paint_onto(&mut doc.layers[layer].pixels, size);
            return;
        }
        let settings = self.settings(layer);
        let mut buffer = StrokeBuffer::default();
        for dab in self.dabs() {
            buffer.paint(doc, dab, &settings);
        }
        buffer.finish_tail(doc, &settings);
        buffer.reclaim_empty_tiles(doc);
    }
}

/// Whether two layers hold the very same strokes (cheap: shared recordings).
pub(crate) fn same(a: &Option<VectorLayer>, b: &Option<VectorLayer>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.strokes.len() == b.strokes.len()
                && a.strokes
                    .iter()
                    .zip(&b.strokes)
                    .all(|(a, b)| Arc::ptr_eq(a, b))
        }
        _ => false,
    }
}

/// A vector layer's strokes, bottom to top.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VectorLayer {
    pub strokes: Vec<Arc<Stroke>>,
}

impl VectorLayer {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.strokes.len() > MAX_STROKES {
            return Err("A vector layer holds too many strokes".into());
        }
        for stroke in &self.strokes {
            if let Some(line) = &stroke.csp {
                let ok = !line.points.is_empty()
                    && line.points.len() <= MAX_COMMANDS
                    && line.width > 0.0
                    && line.width <= MAX_CSP_WIDTH
                    && (0.0..=1.0).contains(&line.opacity)
                    && line.points.iter().all(|p| {
                        p.x.abs() < 1e7 && p.y.abs() < 1e7 && (0.0..=1.0).contains(&p.pressure)
                    });
                let taper = stroke
                    .csp_taper
                    .iter()
                    .all(|t| t.is_finite() && (0.0..=MAX_TAPER).contains(t));
                if !ok || !taper {
                    return Err("A Clip Studio vector line has damaged points".into());
                }
                continue;
            }
            stroke.brush.validate()?;
            if stroke.commands.len() > MAX_COMMANDS {
                return Err("A vector stroke has too many points".into());
            }
            let finite = |s: &Sample| {
                s.x.is_finite()
                    && s.y.is_finite()
                    && s.pressure.is_finite()
                    && s.x.abs() < 1e7
                    && s.y.abs() < 1e7
            };
            let ok = stroke.commands.iter().all(|c| match c {
                Command::Sample { sample, zoom, .. } | Command::Seed { sample, zoom } => {
                    finite(sample) && zoom.is_finite() && *zoom > 0.0
                }
                Command::CatchUp(f) => f.is_finite() && (0.0..=1.0).contains(f),
                Command::Reset => true,
            });
            if !ok {
                return Err("A vector stroke has damaged points".into());
            }
        }
        Ok(())
    }

    /// The layer's pixels: every stroke painted in order onto a clear layer.
    pub(crate) fn render(
        &self,
        size: [usize; 2],
        cancel: &dyn Fn() -> bool,
    ) -> Result<Pixels, String> {
        render(self.strokes.iter(), size, cancel)
    }

    /// Redraw `pixels` inside `rect` from the strokes that reach it (the
    /// rest of the layer cannot have changed).
    pub(crate) fn redraw(
        &self,
        pixels: &mut Pixels,
        rect: [usize; 4],
        size: [usize; 2],
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let overlaps = |s: &&Arc<Stroke>| {
            s.bounds(size).is_some_and(|b| {
                b[0] < rect[2] && rect[0] < b[2] && b[1] < rect[3] && rect[1] < b[3]
            })
        };
        let drawn = render(self.strokes.iter().filter(overlaps), size, cancel)?;
        let mut writer = pixels.writer();
        for y in rect[1]..rect[3] {
            for x in rect[0]..rect[2] {
                let i = y * size[0] + x;
                writer.rgba_mut(i).copy_from_slice(&drawn.rgba(i));
            }
        }
        Ok(())
    }
}

impl Stroke {
    /// The stroke with every point moved by `map` (canvas pixels to canvas
    /// pixels): its tip scales with the map and its symmetry follows it.
    /// `size` is the canvas the stroke was drawn on.
    pub(crate) fn mapped(&self, map: &dyn Fn(f32, f32) -> [f32; 2], size: [usize; 2]) -> Self {
        let mut stroke = Stroke {
            brush: self.brush.clone(),
            color: self.color,
            symmetry: self.symmetry,
            commands: self.commands.clone(),
            csp: self.csp.clone(),
            csp_taper: self.csp_taper,
            footprint: OnceLock::new(),
        };
        for command in &mut stroke.commands {
            if let Command::Sample { sample, .. } | Command::Seed { sample, .. } = command {
                [sample.x, sample.y] = map(sample.x, sample.y);
            }
        }
        // The map's local stretch and turn where the stroke is.
        let [cx, cy] = self
            .footprint(size)
            .bounds
            .map_or([0.0; 2], |b| [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]);
        let origin = map(cx, cy);
        let across = map(cx + 1.0, cy);
        let down = map(cx, cy + 1.0);
        let (ux, uy) = (across[0] - origin[0], across[1] - origin[1]);
        let (vx, vy) = (down[0] - origin[0], down[1] - origin[1]);
        let scale = (ux * vy - uy * vx).abs().sqrt();
        let turn = uy.atan2(ux).to_degrees();
        if let Some(line) = &self.csp {
            let mut line = (**line).clone();
            for p in &mut line.points {
                let [x, y] = map(p.x as f32, p.y as f32);
                (p.x, p.y) = (f64::from(x), f64::from(y));
            }
            if scale.is_finite() && scale > 0.0 {
                line.width = (line.width * f64::from(scale)).clamp(0.01, MAX_CSP_WIDTH);
                stroke.csp_taper = self.csp_taper.map(|t| (t * scale).min(MAX_TAPER));
            }
            stroke.csp = Some(Arc::new(line));
            return stroke;
        }
        if scale.is_finite() && (scale - 1.0).abs() > 1e-4 || turn.abs() > 1e-3 {
            let mut brush = (*self.brush).clone();
            if scale.is_finite() && scale > 0.0 {
                brush.size = (brush.size * scale).clamp(1.0, 300.0);
            }
            brush.angle = (brush.angle + turn + 180.0).rem_euclid(360.0) - 180.0;
            stroke.brush = Arc::new(brush);
        }
        if stroke.symmetry.active() {
            let [x, y] = self.symmetry.center(size[0], size[1]);
            stroke.symmetry.center = Some(map(x, y));
            stroke.symmetry.angle += turn;
        }
        stroke
    }
}

/// What the vector eraser takes away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum EraseMode {
    /// The part of a line under the eraser; the rest stays as new strokes.
    #[default]
    Touched,
    /// Every line the eraser touches, whole.
    Whole,
    /// The touched part of a line up to the lines crossing it, or up to its
    /// ends where nothing crosses it.
    Crossings,
}

impl Stroke {
    /// The stroke with the parts under `circles` (x, y, radius) taken out,
    /// as the pieces that remain (none when it is all covered). Each piece
    /// follows the line as drawn: its recorded points are the line's dabs,
    /// drawn without the stabilizer, which already shaped them. `None` when
    /// the stroke cannot be cut (scattered or mirrored dabs are not a line).
    fn cut(&self, circles: &[[f32; 3]]) -> Option<Vec<Stroke>> {
        let points = self.cut_points()?;
        let removed: Vec<bool> = points
            .iter()
            .map(|(s, r)| covered(circles, s, *r))
            .collect();
        Some(self.rebuild(&points, &removed))
    }

    /// `cut`, taking the touched part out up to the nearest lines of
    /// `others` that cross this one on either side (or up to its ends).
    fn cut_to_crossings(
        &self,
        circles: &[[f32; 3]],
        others: &[Arc<Stroke>],
        size: [usize; 2],
    ) -> Option<Vec<Stroke>> {
        let points = self.cut_points()?;
        let crossings = crossings(&points, self, others, size);
        // Where each crossing cuts the line: its point nearest the middle of
        // the line that crosses there.
        let mut cuts = Vec::new();
        let mut run: Option<(usize, f32)> = None;
        for (i, depth) in crossings.iter().enumerate() {
            match (depth, &mut run) {
                (Some(d), Some((at, best))) => {
                    if d < best {
                        (*at, *best) = (i, *d);
                    }
                }
                (Some(d), None) => run = Some((i, *d)),
                (None, Some((at, _))) => {
                    cuts.push(*at);
                    run = None;
                }
                (None, None) => {}
            }
        }
        if let Some((at, _)) = run {
            cuts.push(at);
        }
        // The stretches between cuts that the eraser touches go; the cut
        // points stay as the ends of what remains.
        let mut removed = vec![false; points.len()];
        let mut from = 0;
        for end in cuts.iter().copied().chain([points.len()]) {
            let stretch = from..end;
            if stretch
                .clone()
                .any(|i| covered(circles, &points[i].0, points[i].1))
            {
                removed[stretch].fill(true);
            }
            from = end + 1;
        }
        Some(self.rebuild(&points, &removed))
    }

    /// The points `cut` works on, with the line's radius at each: a CSP
    /// line's dense points, or a stroke's dabs. `None` for strokes whose
    /// dabs are not one line (scattered or mirrored).
    fn cut_points(&self) -> Option<Vec<(Sample, f32)>> {
        if self.csp.is_some() {
            return Some(self.line());
        }
        if self.brush.scatter > 0.0 || self.symmetry.active() || self.cut_zoom().is_none() {
            return None;
        }
        let half = self.brush.size * 0.5;
        let pressure_size = self.brush.pressure_size;
        Some(
            self.dabs()
                .into_iter()
                .map(|d| {
                    let scale = if pressure_size {
                        d.pressure.max(0.05)
                    } else {
                        1.0
                    };
                    (d, half * scale)
                })
                .collect(),
        )
    }
    fn cut_zoom(&self) -> Option<f32> {
        self.commands.iter().find_map(|c| match c {
            Command::Sample { zoom, .. } => Some(*zoom),
            _ => None,
        })
    }

    /// The runs of `points` not `removed`, as strokes like this one.
    fn rebuild(&self, points: &[(Sample, f32)], removed: &[bool]) -> Vec<Stroke> {
        if let Some(line) = &self.csp {
            return rebuild_csp(line, points, removed);
        }
        let Some(zoom) = self.cut_zoom() else {
            return Vec::new();
        };
        let mut pieces = Vec::new();
        let mut run: Vec<Sample> = Vec::new();
        let mut first_run = true;
        let last = points.len();
        let mut flush = |run: &mut Vec<Sample>, starts: bool, ends: bool| {
            if run.len() >= 2 {
                let mut brush = (*self.brush).clone();
                brush.stabilization = 0.0;
                // Only the stroke's own ends keep their tapers.
                if !starts {
                    brush.taper_in = 0.0;
                }
                if !ends {
                    brush.taper_out = 0.0;
                }
                let count = run.len();
                let commands = run
                    .iter()
                    .enumerate()
                    .map(|(i, d)| Command::Sample {
                        sample: Sample {
                            x: d.x,
                            y: d.y,
                            pressure: d.pressure,
                            time: d.time,
                            tilt: d.tilt,
                            twist: d.twist,
                            pressure_known: true,
                            ..Default::default()
                        },
                        zoom,
                        lift: i + 1 == count,
                    })
                    .collect();
                pieces.push(Stroke {
                    brush: Arc::new(brush),
                    color: self.color,
                    symmetry: self.symmetry,
                    commands,
                    csp: None,
                    csp_taper: [0.0; 2],
                    footprint: OnceLock::new(),
                });
            }
            run.clear();
        };
        for (i, (dab, _)) in points.iter().enumerate() {
            if removed[i] {
                flush(&mut run, first_run, false);
                first_run = false;
            } else {
                run.push(*dab);
            }
            if i + 1 == last {
                flush(&mut run, first_run, true);
            }
        }
        pieces
    }
}

/// Whether one of `circles` covers the line at `s`, where it is `r` wide.
fn covered(circles: &[[f32; 3]], s: &Sample, r: f32) -> bool {
    circles.iter().any(|&[x, y, radius]| {
        let reach = radius + r;
        (s.x - x) * (s.x - x) + (s.y - y) * (s.y - y) <= reach * reach
    })
}

/// For each of `stroke`'s `points`, how deep inside another line of
/// `others` it lies (distance to that line's middle over its radius, below
/// 1), or `None` where no other line covers it.
fn crossings(
    points: &[(Sample, f32)],
    stroke: &Stroke,
    others: &[Arc<Stroke>],
    size: [usize; 2],
) -> Vec<Option<f32>> {
    let Some(b) = stroke.footprint(size).bounds else {
        return vec![None; points.len()];
    };
    let overlaps = |o: &[f32; 4]| o[0] <= b[2] && b[0] <= o[2] && o[1] <= b[3] && b[1] <= o[3];
    // The other lines' circles over this one, bucketed in a grid of at
    // most 256 x 256 cells.
    let side = ((b[2] - b[0]).max(b[3] - b[1]) / 256.0).max(16.0);
    let columns = ((b[2] - b[0]) / side) as usize + 1;
    let rows = ((b[3] - b[1]) / side) as usize + 1;
    let cell = |x: f32, y: f32| {
        [
            (((x - b[0]) / side).max(0.0) as usize).min(columns - 1),
            (((y - b[1]) / side).max(0.0) as usize).min(rows - 1),
        ]
    };
    let mut grid: Vec<Vec<[f32; 3]>> = vec![Vec::new(); columns * rows];
    for other in others.iter().filter(|o| !std::ptr::eq(o.as_ref(), stroke)) {
        let footprint = other.footprint(size);
        if !footprint.bounds.as_ref().is_some_and(overlaps) {
            continue;
        }
        for &[x, y, r] in &footprint.circles {
            if x + r < b[0] || x - r > b[2] || y + r < b[1] || y - r > b[3] {
                continue;
            }
            let [x0, y0] = cell(x - r, y - r);
            let [x1, y1] = cell(x + r, y + r);
            for row in y0..=y1 {
                for column in x0..=x1 {
                    grid[row * columns + column].push([x, y, r]);
                }
            }
        }
    }
    points
        .iter()
        .map(|(s, _)| {
            let [column, row] = cell(s.x, s.y);
            let mut deepest: Option<f32> = None;
            for &[x, y, r] in &grid[row * columns + column] {
                let d2 = (s.x - x) * (s.x - x) + (s.y - y) * (s.y - y);
                if d2 <= r * r {
                    let depth = d2.sqrt() / r.max(1e-3);
                    deepest = Some(deepest.map_or(depth, |d: f32| d.min(depth)));
                }
            }
            deepest
        })
        .collect()
}

/// Widest CSP line kept after scaling.
const MAX_CSP_WIDTH: f64 = 10_000.0;

/// What remains of a CSP line (`dense`: its points about a pixel apart with
/// their radius) once the `removed` points are taken out, as CSP lines.
fn rebuild_csp(
    line: &crate::clip_vector::Stroke,
    dense: &[(Sample, f32)],
    removed: &[bool],
) -> Vec<Stroke> {
    let mut pieces = Vec::new();
    let mut run: Vec<crate::clip_vector::Point> = Vec::new();
    let mut flush = |run: &mut Vec<crate::clip_vector::Point>| {
        if run.len() >= 2 {
            pieces.push(Stroke::from_csp(crate::clip_vector::Stroke {
                points: std::mem::take(run),
                ..line.clone()
            }));
        }
        run.clear();
    };
    for ((s, _), removed) in dense.iter().zip(removed) {
        if *removed {
            flush(&mut run);
        } else {
            run.push(crate::clip_vector::Point {
                x: f64::from(s.x),
                y: f64::from(s.y),
                pressure: f64::from(s.pressure),
            });
        }
    }
    flush(&mut run);
    pieces
}

/// Whether `stroke` lies in `doc`'s selection (its middle does), or there
/// is no selection.
fn picked(doc: &Document, stroke: &Stroke) -> bool {
    let size = [doc.width, doc.height];
    let Some(b) = stroke.bounds(size) else {
        return false;
    };
    let (x, y) = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2);
    if let Some(mask) = &doc.selection_mask {
        return mask.at(x.min(doc.width - 1), y.min(doc.height - 1)) > 127;
    }
    doc.selection.is_none_or(|s| s.contains(x, y))
}

/// After an operation turned `before`'s active layer into `doc`'s, redraw
/// it from its strokes moved by `map` instead: the strokes `before`'s
/// selection picks (all without one) move, the rest stay. Not a vector
/// layer: nothing to do.
pub(crate) fn follow(
    before: &Document,
    doc: &mut Document,
    map: &dyn Fn(f32, f32) -> [f32; 2],
) -> Result<(), String> {
    let index = before.active;
    if let Some(vector) = &before.layers[index].vector {
        let size = [before.width, before.height];
        let strokes = vector
            .strokes
            .iter()
            .map(|s| {
                if picked(before, s) {
                    Arc::new(s.mapped(map, size))
                } else {
                    s.clone()
                }
            })
            .collect();
        redraw_layer(doc, index, VectorLayer { strokes })?;
    }
    Ok(())
}

/// `follow` for a Free Transform's settings: its warp or affine map, over
/// the same frame the pixels used. A mirrored pair keeps the transformed
/// pixels and drops the strokes (reflected copies are not strokes yet).
pub(crate) fn follow_settings(
    before: &Document,
    doc: &mut Document,
    settings: &crate::affine::Settings,
) -> Result<(), String> {
    if before.layers[before.active].vector.is_none() {
        return Ok(());
    }
    if settings.mirror.is_some() {
        doc.layers[before.active].vector = None;
        return Ok(());
    }
    let mapping = settings.mapping(before)?;
    let frame = crate::affine::Settings::bounds(before);
    let map = |x: f32, y: f32| {
        mapping
            .forward(frame, f64::from(x), f64::from(y))
            .map(|v| v as f32)
    };
    follow(before, doc, &map)
}

/// Every vector layer of `doc` redrawn with all its strokes moved by `map`
/// (canvas size changes, crops and turns; `before` is the old canvas).
pub(crate) fn follow_canvas(
    before: &Document,
    doc: &mut Document,
    map: &dyn Fn(f32, f32) -> [f32; 2],
) -> Result<(), String> {
    let size = [before.width, before.height];
    for index in 0..doc.layers.len() {
        let Some(vector) = &before.layers[index].vector else {
            continue;
        };
        let strokes = vector
            .strokes
            .iter()
            .map(|s| Arc::new(s.mapped(map, size)))
            .collect();
        redraw_layer(doc, index, VectorLayer { strokes })?;
    }
    Ok(())
}

/// Recolor and/or re-brush layer `index`'s strokes in `doc`'s selection
/// (all without one). Whether anything changed.
pub(crate) fn restyle(
    doc: &mut Document,
    index: usize,
    color: Option<[u8; 3]>,
    brush: Option<&Brush>,
) -> Result<bool, String> {
    let vector = doc.layers[index]
        .vector
        .as_ref()
        .ok_or("Select a vector layer to restyle its strokes")?;
    let brush = brush.map(|b| Arc::new(b.clone()));
    let mut changed = false;
    let strokes = vector
        .strokes
        .iter()
        .map(|stroke| {
            if !picked(doc, stroke) {
                return stroke.clone();
            }
            let mut restyled = (**stroke).clone().fresh();
            if let Some(color) = color {
                restyled = restyled.recolored(color);
            }
            if let Some(brush) = &brush {
                restyled = restyled.with_brush(brush.clone());
            }
            if restyled == **stroke {
                stroke.clone()
            } else {
                changed = true;
                Arc::new(restyled)
            }
        })
        .collect();
    if changed {
        redraw_layer(doc, index, VectorLayer { strokes })?;
    }
    Ok(changed)
}

impl Stroke {
    /// The stroke without its worked-out footprint, for a changed copy.
    pub(crate) fn fresh(self) -> Self {
        Self {
            footprint: OnceLock::new(),
            ..self
        }
    }
    /// The stroke in `color`.
    pub(crate) fn recolored(&self, color: [u8; 3]) -> Self {
        let mut out = self.clone();
        out.color = color;
        if let Some(line) = &self.csp {
            out.csp = Some(Arc::new(crate::clip_vector::Stroke {
                color,
                ..(**line).clone()
            }));
        }
        out
    }
    /// The stroke drawn with `brush`; a CSP line becomes a stroke through
    /// its points (the brush's stabilizer is not applied again).
    fn with_brush(mut self, brush: Arc<Brush>) -> Self {
        let Some(line) = self.csp.take() else {
            self.brush = brush;
            return self;
        };
        let mut steady = (*brush).clone();
        steady.stabilization = 0.0;
        self.brush = Arc::new(steady);
        let count = line.points.len();
        self.commands = line
            .points
            .iter()
            .enumerate()
            .map(|(i, p)| Command::Sample {
                sample: Sample {
                    x: p.x as f32,
                    y: p.y as f32,
                    pressure: p.pressure as f32,
                    pressure_known: true,
                    ..Default::default()
                },
                zoom: 1.0,
                lift: i + 1 == count,
            })
            .collect();
        self
    }
}

/// Longest taper a line keeps, in canvas pixels.
pub(crate) const MAX_TAPER: f32 = 1000.0;

impl Stroke {
    /// A CSP line as it is drawn: with its tapers, when it has any.
    fn drawn_csp(&self) -> Option<std::borrow::Cow<'_, crate::clip_vector::Stroke>> {
        let line = self.csp.as_deref()?;
        if self.csp_taper == [0.0; 2] {
            return Some(std::borrow::Cow::Borrowed(line));
        }
        // Points at most 2 px apart, so the taper narrows smoothly.
        let mut points: Vec<crate::clip_vector::Point> = Vec::new();
        for pair in line.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let steps = ((b.x - a.x).hypot(b.y - a.y) / 2.0)
                .ceil()
                .clamp(1.0, 4096.0) as usize;
            for i in 0..steps {
                let t = i as f64 / steps as f64;
                points.push(crate::clip_vector::Point {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                    pressure: a.pressure + (b.pressure - a.pressure) * t,
                });
            }
        }
        points.extend(line.points.last().copied());
        let mut along = vec![0.0; points.len()];
        for i in 1..points.len() {
            let (a, b) = (points[i - 1], points[i]);
            along[i] = along[i - 1] + (b.x - a.x).hypot(b.y - a.y);
        }
        let total = along.last().copied().unwrap_or(0.0);
        // As a brush tapers: from 5% of the width over the taper's length.
        let factor = |distance: f64, taper: f32| {
            if taper > 0.0 {
                0.05 + 0.95 * (distance / f64::from(taper)).min(1.0)
            } else {
                1.0
            }
        };
        for (p, s) in points.iter_mut().zip(&along) {
            p.pressure *= factor(*s, self.csp_taper[0]).min(factor(total - s, self.csp_taper[1]));
        }
        Some(std::borrow::Cow::Owned(crate::clip_vector::Stroke {
            points,
            ..line.clone()
        }))
    }
    /// The line's start and end tapers, in canvas pixels.
    pub(crate) fn taper(&self) -> [f32; 2] {
        if self.csp.is_some() {
            self.csp_taper
        } else {
            [self.brush.taper_in, self.brush.taper_out]
        }
    }
    /// The stroke with these tapers (`None` keeps that end's).
    pub(crate) fn with_taper(&self, start: Option<f32>, end: Option<f32>) -> Self {
        let [old_start, old_end] = self.taper();
        let taper = [
            start.unwrap_or(old_start).clamp(0.0, MAX_TAPER),
            end.unwrap_or(old_end).clamp(0.0, MAX_TAPER),
        ];
        let mut out = self.clone().fresh();
        if out.csp.is_some() {
            out.csp_taper = taper;
        } else {
            let mut brush = (*self.brush).clone();
            // Brush settings allow up to 300 px.
            [brush.taper_in, brush.taper_out] = taper.map(|t| t.min(300.0));
            out.brush = Arc::new(brush);
        }
        out
    }
    /// The stroke with its path evened out over `radius` canvas pixels
    /// along it; its ends stay where they are.
    pub(crate) fn smoothed(&self, radius: f32) -> Self {
        let mut out = self.clone().fresh();
        let radius = f64::from(radius);
        if let Some(line) = &self.csp {
            let path: Vec<[f64; 2]> = line.points.iter().map(|p| [p.x, p.y]).collect();
            let mut line = (**line).clone();
            for (p, [x, y]) in line.points.iter_mut().zip(smooth_path(&path, radius)) {
                (p.x, p.y) = (x, y);
            }
            out.csp = Some(Arc::new(line));
            return out;
        }
        // The pen's points, run by run (a reset starts a new run).
        let mut start = 0;
        let commands = &mut out.commands;
        while start < commands.len() {
            let end = commands[start..]
                .iter()
                .position(|c| matches!(c, Command::Reset))
                .map_or(commands.len(), |i| start + i);
            let run: Vec<usize> = (start..end)
                .filter(|&i| matches!(commands[i], Command::Sample { .. } | Command::Seed { .. }))
                .collect();
            let path: Vec<[f64; 2]> = run
                .iter()
                .map(|&i| match commands[i] {
                    Command::Sample { sample, .. } | Command::Seed { sample, .. } => {
                        [f64::from(sample.x), f64::from(sample.y)]
                    }
                    _ => unreachable!(),
                })
                .collect();
            for (&i, [x, y]) in run.iter().zip(smooth_path(&path, radius)) {
                if let Command::Sample { sample, .. } | Command::Seed { sample, .. } =
                    &mut commands[i]
                {
                    (sample.x, sample.y) = (x as f32, y as f32);
                }
            }
            start = end + 1;
        }
        out
    }
}

impl Stroke {
    /// The points that shape the line, in order: a CSP line's points, or
    /// the pen samples of a stroke.
    pub(crate) fn control_points(&self) -> Vec<[f32; 2]> {
        if let Some(line) = &self.csp {
            return line
                .points
                .iter()
                .map(|p| [p.x as f32, p.y as f32])
                .collect();
        }
        self.commands
            .iter()
            .filter_map(|c| match c {
                Command::Sample { sample, .. } | Command::Seed { sample, .. } => {
                    Some([sample.x, sample.y])
                }
                _ => None,
            })
            .collect()
    }
    /// The stroke with points added along it so that no two shaping points
    /// are more than `spacing` apart; it draws the same line.
    pub(crate) fn densified(&self, spacing: f32) -> Self {
        let spacing = spacing.max(0.5);
        let steps = |dx: f32, dy: f32| (dx.hypot(dy) / spacing).ceil().clamp(1.0, 4096.0) as usize;
        let mut out = self.clone().fresh();
        if let Some(line) = &self.csp {
            let mut dense = (**line).clone();
            dense.points.clear();
            for pair in line.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let n = steps((b.x - a.x) as f32, (b.y - a.y) as f32);
                for i in 0..n {
                    let t = i as f64 / n as f64;
                    dense.points.push(crate::clip_vector::Point {
                        x: a.x + (b.x - a.x) * t,
                        y: a.y + (b.y - a.y) * t,
                        pressure: a.pressure + (b.pressure - a.pressure) * t,
                    });
                }
            }
            dense.points.extend(line.points.last().copied());
            if dense.points.len() <= MAX_COMMANDS {
                out.csp = Some(Arc::new(dense));
            }
            return out;
        }
        let mut commands = Vec::with_capacity(self.commands.len());
        let mut previous: Option<Sample> = None;
        for command in &self.commands {
            match *command {
                Command::Sample { sample, zoom, .. } => {
                    if let Some(a) = previous {
                        let n = steps(sample.x - a.x, sample.y - a.y);
                        for i in 1..n {
                            let t = i as f32 / n as f32;
                            let time = match (a.time, sample.time) {
                                (Some(ta), Some(tb)) => Some(ta + (tb - ta) * f64::from(t)),
                                _ => sample.time,
                            };
                            commands.push(Command::Sample {
                                sample: Sample {
                                    x: a.x + (sample.x - a.x) * t,
                                    y: a.y + (sample.y - a.y) * t,
                                    pressure: a.pressure + (sample.pressure - a.pressure) * t,
                                    time,
                                    ..a
                                },
                                zoom,
                                lift: false,
                            });
                        }
                    }
                    previous = Some(sample);
                }
                Command::Seed { sample, .. } => previous = Some(sample),
                Command::Reset => previous = None,
                Command::CatchUp(_) => {}
            }
            commands.push(*command);
        }
        if commands.len() <= MAX_COMMANDS {
            out.commands = commands;
        }
        out
    }
    /// The stroke with each shaping point `i` moved by `offset` times
    /// `weights[i]` (points past the weights stay).
    pub(crate) fn displaced(&self, weights: &[f32], offset: [f32; 2]) -> Self {
        let mut out = self.clone().fresh();
        let [dx, dy] = offset;
        if let Some(line) = &self.csp {
            let mut line = (**line).clone();
            for (p, w) in line.points.iter_mut().zip(weights) {
                p.x += f64::from(dx * w);
                p.y += f64::from(dy * w);
            }
            out.csp = Some(Arc::new(line));
            return out;
        }
        let shaping = out.commands.iter_mut().filter_map(|c| match c {
            Command::Sample { sample, .. } | Command::Seed { sample, .. } => Some(sample),
            _ => None,
        });
        for (sample, w) in shaping.zip(weights) {
            sample.x += dx * w;
            sample.y += dy * w;
        }
        out
    }
}

/// `points` averaged over `radius` along the path (triangle weights); the
/// window narrows toward the ends, which stay put.
fn smooth_path(points: &[[f64; 2]], radius: f64) -> Vec<[f64; 2]> {
    let n = points.len();
    if n < 3 || radius <= 0.0 {
        return points.to_vec();
    }
    let mut along = vec![0.0; n];
    for i in 1..n {
        let (a, b) = (points[i - 1], points[i]);
        along[i] = along[i - 1] + (b[0] - a[0]).hypot(b[1] - a[1]);
    }
    let total = along[n - 1];
    let mut first = 0;
    (0..n)
        .map(|i| {
            let window = radius.min(along[i]).min(total - along[i]);
            if window <= 1e-9 {
                return points[i];
            }
            // The window's start only moves forward along the path.
            while along[first] < along[i] - window {
                first += 1;
            }
            let (mut sum, mut weight) = ([0.0; 2], 0.0);
            for j in first..n {
                let offset = (along[j] - along[i]).abs();
                if along[j] > along[i] + window {
                    break;
                }
                let k = 1.0 - offset / window;
                sum = [sum[0] + points[j][0] * k, sum[1] + points[j][1] * k];
                weight += k;
            }
            if weight > 0.0 {
                [sum[0] / weight, sum[1] / weight]
            } else {
                points[i]
            }
        })
        .collect()
}

/// Give layer `index` these strokes and the pixels they paint.
pub(crate) fn redraw_layer(
    doc: &mut Document,
    index: usize,
    vector: VectorLayer,
) -> Result<(), String> {
    let pixels = vector.render([doc.width, doc.height], &|| false)?;
    let layer = &mut doc.layers[index];
    layer.pixels = pixels;
    layer.vector = Some(vector);
    Ok(())
}

fn render<'a>(
    strokes: impl Iterator<Item = &'a Arc<Stroke>>,
    [width, height]: [usize; 2],
    cancel: &dyn Fn() -> bool,
) -> Result<Pixels, String> {
    let mut doc = Document::new(width, height);
    for stroke in strokes {
        if cancel() {
            return Err("Canceled".into());
        }
        stroke.paint(&mut doc, 0);
    }
    Ok(doc.layers.swap_remove(0).pixels)
}

// Saved form: each distinct brush once, and each stroke's commands packed
// little-endian in base64 (a sample is 37 bytes).
mod saved {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    #[derive(Serialize, Deserialize)]
    struct Layer {
        brushes: Vec<Brush>,
        strokes: Vec<SavedStroke>,
    }
    #[derive(Serialize, Deserialize)]
    struct SavedStroke {
        #[serde(default)]
        brush: usize,
        color: [u8; 3],
        #[serde(default)]
        symmetry: Symmetry,
        #[serde(default)]
        commands: String,
        /// A Clip Studio line; brush and commands are then unused.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        csp: Option<SavedLine>,
        /// The CSP line's tapers.
        #[serde(default, skip_serializing_if = "untapered")]
        taper: [f32; 2],
    }
    fn untapered(taper: &[f32; 2]) -> bool {
        *taper == [0.0; 2]
    }
    #[derive(Serialize, Deserialize)]
    struct SavedLine {
        opacity: f32,
        width: f64,
        style: u32,
        antialias: u8,
        /// x, y and pressure (f64) per point, little-endian base64.
        points: String,
    }
    impl SavedLine {
        fn new(line: &crate::clip_vector::Stroke) -> Self {
            let mut bytes = Vec::with_capacity(line.points.len() * 24);
            for p in &line.points {
                bytes.extend(p.x.to_le_bytes());
                bytes.extend(p.y.to_le_bytes());
                bytes.extend(p.pressure.to_le_bytes());
            }
            Self {
                opacity: line.opacity,
                width: line.width,
                style: line.style,
                antialias: line.antialias,
                points: crate::pattern::base64(&bytes),
            }
        }
        fn line(self, color: [u8; 3]) -> Option<crate::clip_vector::Stroke> {
            let bytes = crate::pattern::unbase64(&self.points)?;
            if bytes.len() % 24 != 0 || bytes.len() / 24 > MAX_COMMANDS {
                return None;
            }
            let points = bytes
                .chunks_exact(24)
                .map(|b| crate::clip_vector::Point {
                    x: f64::from_le_bytes(b[0..8].try_into().unwrap()),
                    y: f64::from_le_bytes(b[8..16].try_into().unwrap()),
                    pressure: f64::from_le_bytes(b[16..24].try_into().unwrap()),
                })
                .collect();
            Some(crate::clip_vector::Stroke {
                color,
                opacity: self.opacity,
                width: self.width,
                style: self.style,
                antialias: self.antialias,
                points,
            })
        }
    }

    const SAMPLE: u8 = 0;
    const LIFT: u8 = 1;
    const SEED: u8 = 2;
    const CATCH_UP: u8 = 3;
    const RESET: u8 = 4;

    fn pack(commands: &[Command]) -> Vec<u8> {
        let mut out = Vec::with_capacity(commands.len() * 37);
        let sample = |out: &mut Vec<u8>, s: &Sample, zoom: f32| {
            for v in [s.x, s.y, s.pressure, zoom] {
                out.extend(v.to_le_bytes());
            }
            out.extend(s.time.unwrap_or(f64::NAN).to_le_bytes());
            let [tx, ty] = s.tilt.unwrap_or([f32::NAN; 2]);
            for v in [tx, ty, s.twist.unwrap_or(f32::NAN)] {
                out.extend(v.to_le_bytes());
            }
            out.push(
                u8::from(s.eraser) | u8::from(s.barrel) << 1 | u8::from(s.pressure_known) << 2,
            );
        };
        for command in commands {
            match command {
                Command::Sample {
                    sample: s,
                    zoom,
                    lift,
                } => {
                    out.push(if *lift { LIFT } else { SAMPLE });
                    sample(&mut out, s, *zoom);
                }
                Command::Seed { sample: s, zoom } => {
                    out.push(SEED);
                    sample(&mut out, s, *zoom);
                }
                Command::CatchUp(f) => {
                    out.push(CATCH_UP);
                    out.extend(f.to_le_bytes());
                }
                Command::Reset => out.push(RESET),
            }
        }
        out
    }
    fn unpack(mut data: &[u8]) -> Option<Vec<Command>> {
        let mut take = |n: usize| -> Option<&[u8]> {
            let (head, rest) = data.split_at_checked(n)?;
            data = rest;
            Some(head)
        };
        let mut commands = Vec::new();
        while let Some(tag) = take(1) {
            let f32_at = |b: &[u8], i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap());
            let some = |v: f32| (!v.is_nan()).then_some(v);
            let read_sample = |b: &[u8]| -> (Sample, f32) {
                let time = f64::from_le_bytes(b[16..24].try_into().unwrap());
                let (tx, ty) = (f32_at(b, 24), f32_at(b, 28));
                let flags = b[36];
                (
                    Sample {
                        x: f32_at(b, 0),
                        y: f32_at(b, 4),
                        pressure: f32_at(b, 8),
                        time: (!time.is_nan()).then_some(time),
                        tilt: (!tx.is_nan() && !ty.is_nan()).then_some([tx, ty]),
                        twist: some(f32_at(b, 32)),
                        eraser: flags & 1 != 0,
                        barrel: flags & 2 != 0,
                        pressure_known: flags & 4 != 0,
                        ..Default::default()
                    },
                    f32_at(b, 12),
                )
            };
            commands.push(match tag[0] {
                SAMPLE | LIFT => {
                    let (sample, zoom) = read_sample(take(37)?);
                    Command::Sample {
                        sample,
                        zoom,
                        lift: tag[0] == LIFT,
                    }
                }
                SEED => {
                    let (sample, zoom) = read_sample(take(37)?);
                    Command::Seed { sample, zoom }
                }
                CATCH_UP => Command::CatchUp(f32_at(take(4)?, 0)),
                RESET => Command::Reset,
                _ => return None,
            });
            if commands.len() > MAX_COMMANDS {
                return None;
            }
        }
        Some(commands)
    }

    impl Serialize for VectorLayer {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut brushes: Vec<Brush> = Vec::new();
            let strokes = self
                .strokes
                .iter()
                .map(|stroke| {
                    if let Some(line) = &stroke.csp {
                        return SavedStroke {
                            brush: 0,
                            color: line.color,
                            symmetry: stroke.symmetry,
                            commands: String::new(),
                            csp: Some(SavedLine::new(line)),
                            taper: stroke.csp_taper,
                        };
                    }
                    let brush = brushes
                        .iter()
                        .position(|b| *b == *stroke.brush)
                        .unwrap_or_else(|| {
                            brushes.push((*stroke.brush).clone());
                            brushes.len() - 1
                        });
                    SavedStroke {
                        brush,
                        color: stroke.color,
                        symmetry: stroke.symmetry,
                        commands: crate::pattern::base64(&pack(&stroke.commands)),
                        csp: None,
                        taper: [0.0; 2],
                    }
                })
                .collect();
            Layer { brushes, strokes }.serialize(serializer)
        }
    }
    impl<'de> Deserialize<'de> for VectorLayer {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let saved = Layer::deserialize(deserializer)?;
            let brushes: Vec<Arc<Brush>> = saved.brushes.into_iter().map(Arc::new).collect();
            if saved.strokes.len() > MAX_STROKES {
                return Err(D::Error::custom("A vector layer holds too many strokes"));
            }
            let strokes = saved
                .strokes
                .into_iter()
                .map(|s| {
                    if let Some(saved) = s.csp {
                        let line = saved
                            .line(s.color)
                            .ok_or_else(|| D::Error::custom("Damaged vector line"))?;
                        return Ok(Arc::new(Stroke {
                            csp_taper: s.taper,
                            ..Stroke::from_csp(line)
                        }));
                    }
                    let brush = brushes
                        .get(s.brush)
                        .cloned()
                        .ok_or_else(|| D::Error::custom("Vector stroke names a missing brush"))?;
                    let commands = crate::pattern::unbase64(&s.commands)
                        .and_then(|bytes| unpack(&bytes))
                        .ok_or_else(|| D::Error::custom("Damaged vector stroke"))?;
                    Ok(Arc::new(Stroke {
                        brush,
                        color: s.color,
                        symmetry: s.symmetry,
                        commands,
                        csp: None,
                        csp_taper: [0.0; 2],
                        footprint: OnceLock::new(),
                    }))
                })
                .collect::<Result<_, D::Error>>()?;
            let layer = VectorLayer { strokes };
            layer.validate().map_err(D::Error::custom)?;
            Ok(layer)
        }
    }
}

/// Add a command to the stroke being recorded, if any.
pub(crate) fn record(record: &mut Option<Stroke>, command: Command) {
    if let Some(record) = record
        && record.commands.len() < MAX_COMMANDS
    {
        record.commands.push(command);
    }
}

/// The vector eraser while it is dragged: strokes it touches are removed at
/// once, all in one undo step.
pub(crate) struct Erase {
    layer: crate::document::LayerId,
    brush: Brush,
    sampler: crate::stroke::Stroke,
    checkpointed: bool,
    modified: bool,
    removed: usize,
    mode: EraseMode,
}

impl crate::Dudel {
    /// The active layer's index when it is a vector layer drawn on directly
    /// (not its mask).
    pub(crate) fn vector_target(&self) -> Option<usize> {
        if !self.input_stroke_target().is_raster() || self.pending_layers.is_some() {
            return None;
        }
        let index = self.input_layer()?;
        self.doc.layers.get(index)?.vector.as_ref().map(|_| index)
    }
    /// Why the stroke about to start cannot go on this vector layer.
    pub(crate) fn vector_stroke_blocker(&self) -> Option<&'static str> {
        self.vector_target()?;
        if self.doc.selection.is_some() || self.doc.selection_mask.is_some() {
            Some("Vector layers draw without selections — deselect to draw here")
        } else if self.scripted_tool.is_some() {
            Some(
                "Scripted tools draw on raster layers — rasterize this vector layer or pick another",
            )
        } else if self.replay.enabled {
            Some("Stroke recording does not support vector layers yet")
        } else {
            None
        }
    }
    /// Start recording the stroke that just began, if it is on a vector layer.
    pub(crate) fn begin_vector_record(&mut self) {
        self.vector_record = None;
        if self.vector_target().is_none() {
            return;
        }
        if let Some(settings) = &self.stroke_settings
            && !settings.erase
        {
            self.vector_record = Some(Stroke::new(
                settings.brush.clone(),
                settings.color,
                settings.symmetry,
            ));
        }
    }

    /// Begin the vector eraser on the active vector layer.
    pub(crate) fn begin_vector_erase(&mut self, brush: Brush) {
        let Some(index) = self.vector_target() else {
            return;
        };
        if let Some(blocker) = self.doc.layers[index]
            .locked
            .then_some("Unlock the layer before erasing")
            .or_else(|| {
                (!self.doc.shown(index)).then_some("Show the layer and its folder before erasing")
            })
        {
            self.status = blocker.into();
            return;
        }
        // Strokes still rendering must land before strokes are removed.
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.flush_render_queue();
        self.vector_erase = Some(Erase {
            layer: self.doc.layers[index].id,
            brush,
            sampler: crate::stroke::Stroke::default(),
            checkpointed: false,
            modified: self.modified,
            removed: 0,
            mode: self.vector_erase_mode,
        });
    }
    /// Feed the vector eraser one pen sample in canvas pixels.
    pub(crate) fn vector_erase_sample(&mut self, sample: Sample, lift: bool) {
        let Some(erase) = &mut self.vector_erase else {
            return;
        };
        let zoom = self.zoom;
        let dabs = if lift {
            erase.sampler.finish_at_zoom(sample, &erase.brush, zoom)
        } else {
            erase.sampler.sample_at_zoom(sample, &erase.brush, zoom)
        };
        let Some(index) = self.doc.layers.iter().position(|l| l.id == erase.layer) else {
            self.vector_erase = None;
            return;
        };
        let size = [self.doc.width, self.doc.height];
        let Some(vector) = &self.doc.layers[index].vector else {
            self.vector_erase = None;
            return;
        };
        let pressure_size = erase.brush.pressure_size;
        let circles: Vec<[f32; 3]> = dabs
            .iter()
            .map(|dab| {
                let scale = if pressure_size {
                    dab.pressure.max(0.1)
                } else {
                    1.0
                };
                [dab.x, dab.y, erase.brush.size * 0.5 * scale]
            })
            .collect();
        let hit: Vec<Arc<Stroke>> = vector
            .strokes
            .iter()
            .filter(|stroke| {
                circles
                    .iter()
                    .any(|&[x, y, r]| stroke.touches(x, y, r, size))
            })
            .cloned()
            .collect();
        if hit.is_empty() {
            return;
        }
        if !erase.checkpointed {
            erase.checkpointed = true;
            self.history.begin_gesture_checkpoint(&self.doc);
        }
        erase.removed += hit.len();
        let rect = hit.iter().filter_map(|s| s.bounds(size)).reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        });
        let mode = erase.mode;
        let layer = &mut self.doc.layers[index];
        let vector = layer.vector.as_mut().unwrap();
        let all = vector.strokes.clone();
        // Each touched line is removed, or replaced by what is left of it.
        vector.strokes = std::mem::take(&mut vector.strokes)
            .into_iter()
            .flat_map(|stroke| {
                if !hit.iter().any(|h| Arc::ptr_eq(h, &stroke)) {
                    return vec![stroke];
                }
                match mode {
                    EraseMode::Touched => stroke
                        .cut(&circles)
                        .unwrap_or_default()
                        .into_iter()
                        .map(Arc::new)
                        .collect(),
                    EraseMode::Whole => Vec::new(),
                    EraseMode::Crossings => stroke
                        .cut_to_crossings(&circles, &all, size)
                        .unwrap_or_default()
                        .into_iter()
                        .map(Arc::new)
                        .collect(),
                }
            })
            .collect();
        if let Some(rect) = rect {
            let vector = vector.clone();
            if let Err(error) = vector.redraw(&mut layer.pixels, rect, size, &|| false) {
                self.status = error;
            }
            self.dirty_region.include(rect);
        }
        self.changed();
    }
    /// Lift the vector eraser: one undo step for everything it removed.
    pub(crate) fn finish_vector_erase(&mut self) {
        let Some(erase) = self.vector_erase.take() else {
            return;
        };
        if erase.checkpointed {
            self.history.settle(&self.doc);
            self.status = format!(
                "Erased {} {} vector stroke{}",
                if erase.mode == EraseMode::Whole {
                    "from"
                } else {
                    "parts of"
                },
                erase.removed,
                if erase.removed == 1 { "" } else { "s" }
            );
        } else {
            self.modified = erase.modified;
        }
    }
    /// Give the active vector layer's strokes (those in the selection, or
    /// all) the current color, or the current brush.
    pub(crate) fn restyle_vector(&mut self, use_color: bool) {
        let Some(target) = self.input_target() else {
            return;
        };
        if !use_color && self.tool != crate::tools::Tool::Brush {
            self.status = "Pick the brush to apply first (Brush tool)".into();
            return;
        }
        self.close_input_gestures();
        let edit = crate::document_edits::Edit::VectorStyle {
            target,
            color: use_color.then_some(self.color),
            brush: (!use_color).then(|| Box::new(self.brush.clone())),
        };
        self.dispatch_document_edit(
            edit,
            if use_color {
                "Recolored the vector strokes"
            } else {
                "Redrew the vector strokes with the current brush"
            },
            "The strokes already look like that",
        );
    }
    /// Turn the active vector layer into an ordinary raster layer.
    pub(crate) fn rasterize_layer(&mut self) {
        let Some(target) = self.input_target() else {
            return;
        };
        self.close_input_gestures();
        self.dispatch_document_edit(
            crate::document_edits::Edit::Rasterize(target),
            "Rasterized the layer",
            "The layer is not a vector layer",
        );
    }
    /// Add an empty vector layer above the selected one.
    pub(crate) fn create_vector_layer(&mut self) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let id = crate::document::LayerId::new();
        self.flush_render_queue();
        self.dispatch_document_edit(
            crate::document_edits::Edit::NewVector(id),
            "New vector layer: strokes stay editable — the eraser removes whole strokes",
            "No layer created",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dudel, egui, input::Packet, tools::Tool};

    fn app() -> Dudel {
        let mut app = Dudel {
            doc: Document::new(96, 64),
            brush: Brush {
                size: 6.0,
                hardness: 0.7,
                stabilization: 4.0,
                taper_out: 6.0,
                scatter: 0.2,
                scatter_seed: 7,
                ..Brush::default()
            },
            zoom: 4.0,
            auto_fit: false,
            ..Default::default()
        };
        app.create_vector_layer();
        app.flush_render_queue();
        app
    }
    /// A pen stroke through real input, from `from` to `to` in canvas pixels.
    fn stroke(app: &mut Dudel, from: [f32; 2], to: [f32; 2]) {
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let n = 16;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let p = egui::pos2(
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t + 6.0 * (t * 6.0).sin(),
            );
            let screen = app.view(app.canvas_area).to_screen(p);
            let sample = Sample {
                x: screen.x,
                y: screen.y,
                pressure: 0.3 + 0.6 * t,
                pressure_known: true,
                time: Some(i as f64 * 0.012),
                ..Default::default()
            };
            let packet = match i {
                0 => Packet::Start(sample),
                i if i == n => Packet::End(sample),
                _ => Packet::Move(sample),
            };
            let mut input = raw();
            input.events.push(egui::Event::PointerMoved(screen));
            let _ = ctx.run(input, |ctx| app.canvas(ctx, &[packet]));
        }
        app.flush_render_queue();
    }
    fn layer(app: &Dudel) -> &crate::document::Layer {
        &app.doc.layers[app.doc.active]
    }
    fn rendered(app: &Dudel) -> Pixels {
        let doc = &app.doc;
        layer(app)
            .vector
            .as_ref()
            .unwrap()
            .render([doc.width, doc.height], &|| false)
            .unwrap()
    }
    fn same_pixels(a: &Pixels, b: &Pixels, count: usize) -> bool {
        (0..count).all(|i| a.rgba(i) == b.rgba(i))
    }

    #[test]
    fn strokes_on_a_vector_layer_are_kept_and_redraw_exactly() {
        let mut app = app();
        assert_eq!(layer(&app).name, "Vector 1");
        stroke(&mut app, [8.0, 16.0], [88.0, 20.0]);
        app.color = [200, 40, 30];
        stroke(&mut app, [10.0, 50.0], [80.0, 12.0]);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes.len(), 2);
        assert_eq!(vector.strokes[1].color, [200, 40, 30]);
        let count = 96 * 64;
        assert!((0..count).any(|i| layer(&app).pixels.rgba(i)[3] > 0));
        // Replaying the recordings gives the very pixels drawn live.
        assert!(same_pixels(&rendered(&app), &layer(&app).pixels, count));
        // Undo takes back a stroke and its recording together.
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(same_pixels(&rendered(&app), &layer(&app).pixels, count));
        app.redo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
    }

    #[test]
    fn the_eraser_removes_whole_strokes_in_one_undo_step() {
        let mut app = app();
        stroke(&mut app, [8.0, 14.0], [88.0, 14.0]);
        stroke(&mut app, [8.0, 50.0], [88.0, 50.0]);
        let before = layer(&app).pixels.clone();
        app.select_tool(Tool::Eraser);
        app.vector_erase_mode = EraseMode::Whole;
        app.brush.size = 4.0;
        // Across the top stroke only.
        stroke(&mut app, [48.0, 2.0], [48.0, 24.0]);
        assert!(app.vector_erase.is_none());
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes.len(), 1, "{}", app.status);
        let count = 96 * 64;
        assert!(same_pixels(&rendered(&app), &layer(&app).pixels, count));
        // The top line is gone, the bottom one untouched.
        assert!((0..96).all(|x| layer(&app).pixels.rgba(14 * 96 + x)[3] == 0));
        assert!((0..96).any(|x| layer(&app).pixels.rgba(50 * 96 + x)[3] > 0));
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
        assert!(same_pixels(&before, &layer(&app).pixels, count));
    }

    #[test]
    fn vector_layers_refuse_pixel_edits_and_drawing_through_selections() {
        let mut app = app();
        stroke(&mut app, [8.0, 14.0], [88.0, 14.0]);
        assert!(app.doc.paint_blocker().unwrap().contains("vector layer"));
        // A selection would clip strokes their recording cannot repeat.
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 40,
            bottom: 40,
        });
        stroke(&mut app, [8.0, 30.0], [88.0, 30.0]);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(app.status.contains("deselect"), "{}", app.status);
        app.doc.selection = None;
        // Clearing the layer removes every stroke.
        let edit = crate::document_edits::Edit::Clear { whole_layer: true };
        let (cleared, outcome) = edit.prepare(&app.doc).unwrap();
        assert!(outcome.changed);
        let layer = &cleared.layers[cleared.active];
        assert!(layer.vector.as_ref().unwrap().strokes.is_empty());
        assert!((0..96 * 64).all(|i| layer.pixels.rgba(i)[3] == 0));
        // Rasterizing keeps the pixels as an ordinary layer.
        app.rasterize_layer();
        app.flush_render_queue();
        assert!(app.doc.layers[app.doc.active].vector.is_none());
        assert!(app.doc.paint_blocker().is_none());
    }

    #[test]
    fn vector_layers_survive_saving_and_reopening() {
        let mut app = app();
        stroke(&mut app, [8.0, 16.0], [88.0, 20.0]);
        stroke(&mut app, [10.0, 50.0], [80.0, 12.0]);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let (a, b) = (
            &app.doc.layers[app.doc.active],
            &loaded.layers[loaded.active],
        );
        let (va, vb) = (a.vector.as_ref().unwrap(), b.vector.as_ref().unwrap());
        assert_eq!(va.strokes.len(), vb.strokes.len());
        for (x, y) in va.strokes.iter().zip(&vb.strokes) {
            assert_eq!(**x, **y);
        }
        let count = 96 * 64;
        assert!(same_pixels(&a.pixels, &b.pixels, count));
        let redrawn = vb.render([96, 64], &|| false).unwrap();
        assert!(same_pixels(&redrawn, &b.pixels, count));
    }

    /// The layer's strokes and pixels agree.
    fn consistent(app: &Dudel) {
        let count = app.doc.width * app.doc.height;
        assert!(same_pixels(&rendered(app), &layer(app).pixels, count));
    }
    fn alpha(app: &Dudel, x: usize, y: usize) -> u8 {
        layer(app).pixels.rgba(y * app.doc.width + x)[3]
    }
    fn straight(app: &mut Dudel, y: f32) {
        app.brush.stabilization = 0.0;
        app.brush.scatter = 0.0;
        stroke_line(app, [8.0, y], [88.0, y]);
    }
    /// A straight stroke (no wobble) through real input.
    fn stroke_line(app: &mut Dudel, from: [f32; 2], to: [f32; 2]) {
        drag(app, from, to, egui::Modifiers::NONE);
    }
    fn drag(app: &mut Dudel, from: [f32; 2], to: [f32; 2], modifiers: egui::Modifiers) {
        trace(app, &[from, to], modifiers);
    }
    /// Pen input along `points` (16 samples per segment).
    fn trace(app: &mut Dudel, points: &[[f32; 2]], modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            modifiers,
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let segments = points.len() - 1;
        let n = 16 * segments;
        for i in 0..=n {
            let segment = (i / 16).min(segments - 1);
            let t = (i - segment * 16) as f32 / 16.0;
            let [from, to] = [points[segment], points[segment + 1]];
            let p = egui::pos2(
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            );
            let screen = app.view(app.canvas_area).to_screen(p);
            let sample = Sample {
                x: screen.x,
                y: screen.y,
                pressure: 1.0,
                pressure_known: true,
                time: Some(i as f64 * 0.012),
                ..Default::default()
            };
            let packet = match i {
                0 => Packet::Start(sample),
                i if i == n => Packet::End(sample),
                _ => Packet::Move(sample),
            };
            let mut input = raw();
            input.events.push(egui::Event::PointerMoved(screen));
            let _ = ctx.run(input, |ctx| app.canvas(ctx, &[packet]));
        }
        app.flush_render_queue();
    }
    fn run(app: &mut Dudel, edit: crate::document_edits::Edit) {
        let (doc, outcome) = edit.prepare(&app.doc).unwrap();
        assert!(outcome.changed);
        app.doc = doc;
    }

    #[test]
    fn moving_and_flipping_move_the_strokes() {
        let mut app = app();
        straight(&mut app, 16.0);
        let id = layer(&app).id;
        run(
            &mut app,
            crate::document_edits::Edit::MoveTarget {
                target: id,
                offset: [0, 20],
            },
        );
        consistent(&app);
        assert!(alpha(&app, 48, 36) > 0 && alpha(&app, 48, 16) == 0);
        run(
            &mut app,
            crate::document_edits::Edit::Transform(crate::transform::Transform::FlipVertical),
        );
        consistent(&app);
        // Flipped about the canvas middle (y 32): 36 lands on 28.
        assert!(alpha(&app, 48, 28) > 0 && alpha(&app, 48, 36) == 0);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
    }

    #[test]
    fn the_move_tool_moves_vector_strokes_with_their_pixels() {
        let mut app = app();
        straight(&mut app, 16.0);
        app.select_tool(Tool::Move);
        stroke_line(&mut app, [48.0, 16.0], [48.0, 40.0]);
        app.flush_render_queue();
        consistent(&app);
        assert!(
            alpha(&app, 48, 40) > 0 && alpha(&app, 48, 16) == 0,
            "{}",
            app.status
        );
        // Reopening redraws the same pixels from the moved strokes.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let reloaded = &loaded.layers[loaded.active];
        let redrawn = reloaded
            .vector
            .as_ref()
            .unwrap()
            .render([96, 64], &|| false)
            .unwrap();
        assert!(same_pixels(&redrawn, &layer(&app).pixels, 96 * 64));
    }

    #[test]
    fn free_transform_scales_strokes_and_their_tips() {
        let mut app = app();
        straight(&mut app, 32.0);
        let size = layer(&app).vector.as_ref().unwrap().strokes[0].brush.size;
        let mut settings = app.affine_settings;
        settings.scale_percent = [50.0, 50.0];
        settings.translation = [0.0, 0.0];
        settings.angle_degrees = 0.0;
        settings.warp = None;
        settings.mirror = None;
        let target = layer(&app).id;
        run(
            &mut app,
            crate::document_edits::Edit::AffineSettingsTarget {
                target,
                settings: Box::new(settings),
            },
        );
        consistent(&app);
        let stroke = &layer(&app).vector.as_ref().unwrap().strokes[0];
        assert!(
            (stroke.brush.size - size * 0.5).abs() < 0.01,
            "{}",
            stroke.brush.size
        );
        // Half as long about the canvas middle: 8..88 becomes 28..68.
        assert!(alpha(&app, 30, 32) > 0 && alpha(&app, 12, 32) == 0 && alpha(&app, 84, 32) == 0);
    }

    #[test]
    fn canvas_changes_keep_vector_layers_vector() {
        let mut app = app();
        straight(&mut app, 16.0);
        run(
            &mut app,
            crate::document_edits::Edit::CanvasSize(crate::canvas_size::Settings {
                width: 120,
                height: 80,
                anchor: [2, 2],
                dpi: None,
            }),
        );
        assert!(layer(&app).vector.is_some());
        consistent(&app);
        // Anchored bottom-right: everything moved by (24, 16).
        assert!(alpha(&app, 72, 32) > 0);
        run(&mut app, crate::document_edits::Edit::Rotate(1));
        assert!(layer(&app).vector.is_some());
        consistent(&app);
        // A clockwise turn sends (72, 32) to (80 - 32, 72).
        assert!(alpha(&app, 48, 72) > 0);
    }

    #[test]
    fn the_eraser_takes_only_the_touched_part_of_a_line() {
        let mut app = app();
        straight(&mut app, 32.0);
        app.select_tool(Tool::Eraser);
        app.brush.size = 6.0;
        assert_eq!(app.vector_erase_mode, EraseMode::Touched);
        stroke_line(&mut app, [48.0, 20.0], [48.0, 44.0]);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert_eq!(strokes.len(), 2, "{}", app.status);
        consistent(&app);
        assert_eq!(alpha(&app, 48, 32), 0);
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 76, 32) > 0);
        // One undo step brings the whole line back.
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert!(alpha(&app, 48, 32) > 0);
    }

    #[test]
    fn strokes_take_the_current_color_or_brush() {
        let mut app = app();
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        app.color = [10, 200, 30];
        app.restyle_vector(true);
        app.flush_render_queue();
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert!(
            vector.strokes.iter().all(|s| s.color == [10, 200, 30]),
            "{}",
            app.status
        );
        // Only strokes in the selection take the bigger brush.
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 40,
            right: 96,
            bottom: 64,
        });
        app.brush.size = 12.0;
        app.restyle_vector(false);
        app.flush_render_queue();
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert_eq!(vector.strokes[0].brush.size, 6.0);
        assert_eq!(vector.strokes[1].brush.size, 12.0);
    }

    #[test]
    fn cut_pieces_keep_the_stabilized_line_where_it_was() {
        let mut app = app();
        app.brush.scatter = 0.0;
        app.brush.stabilization = 8.0;
        stroke(&mut app, [8.0, 30.0], [88.0, 34.0]);
        let before = layer(&app).pixels.clone();
        app.select_tool(Tool::Eraser);
        app.brush.size = 4.0;
        stroke_line(&mut app, [48.0, 4.0], [48.0, 60.0]);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
        consistent(&app);
        // Away from the cut the pieces draw the line as it was drawn.
        let (mut total, mut count) = (0u64, 0u64);
        for y in 0..64 {
            for x in (0..30).chain(66..96) {
                let i = y * 96 + x;
                let (a, b) = (before.rgba(i)[3], layer(&app).pixels.rgba(i)[3]);
                total += u64::from(a.abs_diff(b));
                count += 1;
            }
        }
        let mean = total as f64 / count as f64;
        assert!(mean < 1.0, "pieces drift from the line: {mean}");
    }

    /// A vector layer holding two CSP lines, as an import makes it.
    fn csp_app() -> Dudel {
        let line = |y: f64, color: [u8; 3]| crate::clip_vector::Stroke {
            color,
            opacity: 1.0,
            width: 3.0,
            style: 1,
            antialias: 2,
            points: (0..=8)
                .map(|i| crate::clip_vector::Point {
                    x: 8.0 + f64::from(i) * 10.0,
                    y,
                    pressure: 0.6 + f64::from(i) * 0.05,
                })
                .collect(),
        };
        let lines = vec![line(20.0, [0, 0, 0]), line(44.0, [200, 0, 0])];
        let mut app = Dudel {
            doc: Document::new(96, 64),
            zoom: 4.0,
            auto_fit: false,
            ..Default::default()
        };
        let layer = &mut app.doc.layers[0];
        layer.pixels = crate::clip_vector::render(&lines, [96, 64], &|| false).unwrap();
        layer.vector = Some(VectorLayer {
            strokes: lines
                .into_iter()
                .map(|l| Arc::new(Stroke::from_csp(l)))
                .collect(),
        });
        app
    }

    #[test]
    fn csp_lines_are_editable_strokes() {
        let mut app = csp_app();
        consistent(&app);
        // The eraser cuts a CSP line where it covers it.
        app.select_tool(Tool::Eraser);
        app.brush.size = 4.0;
        stroke_line(&mut app, [48.0, 10.0], [48.0, 30.0]);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert_eq!(strokes.len(), 3, "{}", app.status);
        assert!(strokes.iter().all(|s| s.csp.is_some()));
        consistent(&app);
        assert_eq!(alpha(&app, 48, 20), 0);
        assert!(alpha(&app, 20, 20) > 0 && alpha(&app, 76, 20) > 0);
        // Moving and scaling keep them CSP lines; scaling thins them.
        let id = layer(&app).id;
        run(
            &mut app,
            crate::document_edits::Edit::MoveTarget {
                target: id,
                offset: [0, 4],
            },
        );
        consistent(&app);
        assert!(alpha(&app, 20, 24) > 0 && alpha(&app, 20, 20) == 0);
        let matrix = crate::affine::Matrix([0.5, 0.0, 0.0, 0.5, 24.0, 16.0]);
        run(
            &mut app,
            crate::document_edits::Edit::AffineTarget {
                target: id,
                matrix,
                sampling: crate::affine::Sampling::Bilinear,
            },
        );
        consistent(&app);
        let widths: Vec<f64> = layer(&app)
            .vector
            .as_ref()
            .unwrap()
            .strokes
            .iter()
            .map(|s| s.csp.as_ref().unwrap().width)
            .collect();
        assert!(widths.iter().all(|w| (w - 1.5).abs() < 1e-6), "{widths:?}");
    }

    #[test]
    fn csp_lines_recolor_take_a_brush_and_survive_saving() {
        let mut app = csp_app();
        app.color = [0, 120, 255];
        app.restyle_vector(true);
        app.flush_render_queue();
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap();
        assert!(
            vector
                .strokes
                .iter()
                .all(|s| s.csp.as_ref().unwrap().color == [0, 120, 255])
        );
        // Saved and reopened, the lines draw the same pixels.
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let reloaded = loaded.layers[0].vector.as_ref().unwrap();
        assert_eq!(reloaded, vector);
        let redrawn = reloaded.render([96, 64], &|| false).unwrap();
        assert!(same_pixels(&redrawn, &layer(&app).pixels, 96 * 64));
        // A brush turns the selected line into an ordinary stroke.
        app.doc.selection = Some(crate::selection::Selection {
            left: 0,
            top: 32,
            right: 96,
            bottom: 64,
        });
        app.brush.size = 5.0;
        app.restyle_vector(false);
        app.flush_render_queue();
        consistent(&app);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert!(strokes[0].csp.is_some());
        assert!(strokes[1].csp.is_none() && strokes[1].brush.size == 5.0);
        assert!(alpha(&app, 48, 44) > 0);
    }

    #[test]
    fn the_eraser_can_stop_at_crossing_lines() {
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 32.0);
        stroke_line(&mut app, [30.0, 8.0], [30.0, 56.0]);
        stroke_line(&mut app, [64.0, 8.0], [64.0, 56.0]);
        app.select_tool(Tool::Eraser);
        app.vector_erase_mode = EraseMode::Crossings;
        app.brush.size = 2.0;
        // A dab between the crossing lines takes all of the line between them.
        stroke_line(&mut app, [47.0, 30.0], [47.0, 34.0]);
        consistent(&app);
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes.len(),
            4,
            "{}",
            app.status
        );
        for x in [36, 47, 58] {
            assert_eq!(alpha(&app, x, 32), 0, "x {x}");
        }
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 76, 32) > 0);
        assert!(alpha(&app, 30, 20) > 0 && alpha(&app, 64, 44) > 0);
        // Past the last crossing it goes up to the line's end.
        stroke_line(&mut app, [80.0, 30.0], [80.0, 34.0]);
        consistent(&app);
        assert_eq!(alpha(&app, 76, 32), 0);
        assert_eq!(alpha(&app, 70, 32), 0);
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 64, 20) > 0);
        // A line nothing crosses goes whole.
        app.select_tool(Tool::Brush);
        stroke_line(&mut app, [8.0, 60.0], [20.0, 60.0]);
        app.select_tool(Tool::Eraser);
        let lines = layer(&app).vector.as_ref().unwrap().strokes.len();
        stroke_line(&mut app, [14.0, 58.0], [14.0, 62.0]);
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes.len(),
            lines - 1
        );
    }

    #[test]
    fn the_pointer_picks_moves_and_edits_single_lines() {
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        let before = layer(&app).pixels.clone();
        let count = 96 * 64;
        app.select_tool(Tool::Pointer);
        // A drag that starts on a line moves that line alone.
        stroke_line(&mut app, [40.0, 16.0], [40.0, 26.0]);
        consistent(&app);
        assert_eq!(app.picked_lines().len(), 1, "{}", app.status);
        assert!(alpha(&app, 40, 26) > 0 && alpha(&app, 40, 16) == 0);
        assert!(alpha(&app, 40, 48) > 0);
        // One undo step puts it back; the moved line is no longer picked.
        app.undo();
        app.flush_render_queue();
        assert!(same_pixels(&before, &layer(&app).pixels, count));
        assert!(app.picked_lines().is_empty());
        // Shift+click picks a second line; the panel's actions use both.
        let shift = egui::Modifiers::SHIFT;
        drag(&mut app, [40.0, 16.0], [40.0, 16.0], egui::Modifiers::NONE);
        drag(&mut app, [40.0, 48.0], [40.0, 48.0], shift);
        assert_eq!(app.picked_lines().len(), 2);
        assert!(same_pixels(&before, &layer(&app).pixels, count));
        app.resize_picked_lines(10.0);
        consistent(&app);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert!(strokes.iter().all(|s| s.brush.size == 10.0));
        assert!(alpha(&app, 40, 20) > 0 && alpha(&app, 40, 52) > 0);
        app.color = [0, 200, 0];
        app.recolor_picked_lines();
        consistent(&app);
        assert_eq!(&layer(&app).pixels.rgba(16 * 96 + 40)[..3], &[0, 200, 0]);
        // Shift+click on a picked line lets it go.
        drag(&mut app, [40.0, 48.0], [40.0, 48.0], shift);
        assert_eq!(app.picked_lines().len(), 1);
        app.delete_picked_lines();
        consistent(&app);
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 1);
        assert_eq!(alpha(&app, 40, 16), 0);
        app.undo();
        app.flush_render_queue();
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 2);
        // A press away from every line lets go of the pick.
        drag(&mut app, [40.0, 16.0], [40.0, 16.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1);
        drag(&mut app, [40.0, 32.0], [40.0, 32.0], egui::Modifiers::NONE);
        assert!(app.picked_lines().is_empty());
    }

    #[test]
    fn picked_csp_lines_move_and_resize() {
        let mut app = csp_app();
        app.select_tool(Tool::Pointer);
        stroke_line(&mut app, [48.0, 20.0], [48.0, 30.0]);
        consistent(&app);
        assert!(
            alpha(&app, 48, 30) > 0 && alpha(&app, 48, 20) == 0,
            "{}",
            app.status
        );
        app.resize_picked_lines(2.0);
        consistent(&app);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert_eq!(strokes[0].csp.as_ref().unwrap().width, 1.0);
        assert_eq!(strokes[1].csp.as_ref().unwrap().width, 3.0);
    }

    /// Times the crossing search on every line of a real file's vector
    /// layers (`DUDEL_CSP_FILE`).
    #[test]
    #[ignore]
    fn crossing_search_timing() {
        let path = std::env::var("DUDEL_CSP_FILE").unwrap();
        let doc = crate::clip_metadata::import_document(path.as_ref(), &|| false)
            .unwrap()
            .document;
        let size = [doc.width, doc.height];
        for layer in &doc.layers {
            let Some(vector) = &layer.vector else {
                continue;
            };
            let started = std::time::Instant::now();
            let mut slowest = std::time::Duration::ZERO;
            let mut pieces = 0;
            for stroke in &vector.strokes {
                let one = std::time::Instant::now();
                let path = stroke.path(size);
                let [x, y] = path[path.len() / 2];
                if let Some(cut) = stroke.cut_to_crossings(&[[x, y, 2.0]], &vector.strokes, size) {
                    pieces += cut.len();
                }
                slowest = slowest.max(one.elapsed());
            }
            println!(
                "{}: {} lines -> {pieces} pieces, {:?} total, slowest {:?}",
                layer.name,
                vector.strokes.len(),
                started.elapsed(),
                slowest
            );
        }
    }

    /// Times one frame of dragging each of a real file's biggest lines.
    #[test]
    #[ignore]
    fn line_drag_timing() {
        let path = std::env::var("DUDEL_CSP_FILE").unwrap();
        let doc = crate::clip_metadata::import_document(path.as_ref(), &|| false)
            .unwrap()
            .document;
        let size = [doc.width, doc.height];
        for layer in &doc.layers {
            let Some(vector) = &layer.vector else {
                continue;
            };
            let mut order: Vec<usize> = (0..vector.strokes.len()).collect();
            order.sort_by_key(|&i| std::cmp::Reverse(vector.strokes[i].path(size).len()));
            for &i in order.iter().take(3) {
                let started = std::time::Instant::now();
                let moved = Arc::new(vector.strokes[i].mapped(&|x, y| [x + 3.0, y + 2.0], size));
                let mut strokes = vector.strokes.clone();
                let rect = [vector.strokes[i].bounds(size), moved.bounds(size)]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| {
                        [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[2].max(b[2]),
                            a[3].max(b[3]),
                        ]
                    })
                    .unwrap();
                strokes[i] = moved;
                let mut pixels = layer.pixels.clone();
                VectorLayer { strokes }
                    .redraw(&mut pixels, rect, size, &|| false)
                    .unwrap();
                println!(
                    "{} {}x{}: line {i} ({} px long, rect {}x{}): {:?}",
                    layer.name,
                    size[0],
                    size[1],
                    vector.strokes[i].path(size).len(),
                    rect[2] - rect[0],
                    rect[3] - rect[1],
                    started.elapsed()
                );
            }
        }
    }

    #[test]
    fn picked_lines_smooth_and_taper() {
        let mut app = app();
        app.brush.scatter = 0.0;
        app.brush.taper_out = 0.0;
        app.brush.stabilization = 0.0;
        // A jittery line: 4 px up and down every 4 px.
        let zigzag: Vec<[f32; 2]> = (0..=20)
            .map(|i| [8.0 + i as f32 * 4.0, if i % 2 == 0 { 28.0 } else { 32.0 }])
            .collect();
        trace(&mut app, &zigzag, egui::Modifiers::NONE);
        let size = [96, 64];
        let wobble = |app: &Dudel| {
            let path = layer(app).vector.as_ref().unwrap().strokes[0]
                .path(size)
                .to_vec();
            let mean = path.iter().map(|p| p[1]).sum::<f32>() / path.len() as f32;
            path.iter().map(|p| (p[1] - mean).abs()).sum::<f32>() / path.len() as f32
        };
        let ends = |app: &Dudel| {
            let commands = &layer(app).vector.as_ref().unwrap().strokes[0].commands;
            let points: Vec<[f32; 2]> = commands
                .iter()
                .filter_map(|c| match c {
                    Command::Sample { sample, .. } => Some([sample.x, sample.y]),
                    _ => None,
                })
                .collect();
            [points[0], *points.last().unwrap()]
        };
        let (before, before_ends) = (wobble(&app), ends(&app));
        app.select_tool(Tool::Pointer);
        drag(&mut app, [8.0, 28.0], [8.0, 28.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1, "{}", app.status);
        app.smooth_picked_lines();
        consistent(&app);
        assert!(
            wobble(&app) < before * 0.5,
            "{} -> {}",
            before,
            wobble(&app)
        );
        assert_eq!(ends(&app), before_ends);
        app.taper_picked_lines(Some(30.0), None);
        consistent(&app);
        let stroke = &layer(&app).vector.as_ref().unwrap().strokes[0];
        assert_eq!(stroke.taper(), [30.0, 0.0]);
        assert_eq!(stroke.brush.taper_in, 30.0);
        // One undo step per change.
        app.undo();
        app.flush_render_queue();
        assert_eq!(
            layer(&app).vector.as_ref().unwrap().strokes[0].taper(),
            [0.0, 0.0]
        );
    }

    #[test]
    fn csp_lines_taper_and_keep_it_saved() {
        let mut app = csp_app();
        let before = alpha(&app, 12, 21);
        app.select_tool(Tool::Pointer);
        drag(&mut app, [48.0, 20.0], [48.0, 20.0], egui::Modifiers::NONE);
        assert_eq!(app.picked_lines().len(), 1);
        app.taper_picked_lines(Some(40.0), Some(10.0));
        consistent(&app);
        let vector = layer(&app).vector.as_ref().unwrap().clone();
        assert_eq!(vector.strokes[0].taper(), [40.0, 10.0]);
        assert_eq!(vector.strokes[1].taper(), [0.0, 0.0]);
        // The start narrows: rows off its middle lose coverage.
        assert!(before > 0 && alpha(&app, 12, 21) < before, "{before}");
        app.smooth_picked_lines();
        consistent(&app);
        let mut bytes = Vec::new();
        crate::tile_project::encode(&app.doc, &mut bytes).unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &bytes).unwrap();
        let loaded = crate::storage::load_project(file.path()).unwrap();
        let reloaded = loaded.layers[0].vector.as_ref().unwrap();
        assert_eq!(reloaded, layer(&app).vector.as_ref().unwrap());
        let redrawn = reloaded.render([96, 64], &|| false).unwrap();
        assert!(same_pixels(&redrawn, &layer(&app).pixels, 96 * 64));
    }

    #[test]
    fn a_lasso_picks_the_lines_inside_it() {
        let mut app = app();
        app.brush.size = 4.0;
        straight(&mut app, 16.0);
        straight(&mut app, 48.0);
        stroke_line(&mut app, [40.0, 30.0], [56.0, 30.0]);
        app.select_tool(Tool::Pointer);
        let none = egui::Modifiers::NONE;
        let around = [
            [30.0, 24.0],
            [66.0, 24.0],
            [66.0, 38.0],
            [30.0, 38.0],
            [30.0, 24.0],
        ];
        trace(&mut app, &around, none);
        let picked = app.picked_lines();
        assert_eq!(picked.len(), 1, "{}", app.status);
        assert!(Arc::ptr_eq(
            &picked[0],
            &layer(&app).vector.as_ref().unwrap().strokes[2]
        ));
        // Lines only partly inside are left out; Shift adds to the pick.
        let wide = [
            [2.0, 8.0],
            [60.0, 8.0],
            [60.0, 56.0],
            [2.0, 56.0],
            [2.0, 8.0],
        ];
        trace(&mut app, &wide, egui::Modifiers::SHIFT);
        assert_eq!(app.picked_lines().len(), 1);
        let all = [
            [2.0, 8.0],
            [94.0, 8.0],
            [94.0, 56.0],
            [2.0, 56.0],
            [2.0, 8.0],
        ];
        trace(&mut app, &all, egui::Modifiers::SHIFT);
        assert_eq!(app.picked_lines().len(), 3);
        // Nothing moved or changed while picking.
        assert_eq!(layer(&app).vector.as_ref().unwrap().strokes.len(), 3);
        // Without the lasso, empty canvas pans instead.
        app.pointer_lasso = false;
        trace(&mut app, &around, none);
        assert!(app.picked_lines().is_empty());
    }

    #[test]
    fn reshaping_bends_a_line_around_the_grab() {
        let mut app = app();
        app.brush.size = 4.0;
        app.brush.taper_out = 0.0;
        straight(&mut app, 32.0);
        straight(&mut app, 56.0);
        let before = layer(&app).pixels.clone();
        app.select_tool(Tool::Pointer);
        app.line_drag = crate::vector_pick::LineDrag::Reshape;
        app.bend_reach = 20.0;
        stroke_line(&mut app, [48.0, 32.0], [48.0, 44.0]);
        consistent(&app);
        assert_eq!(app.status, "Reshaped the line");
        // The grabbed point follows; the line beyond the reach stays.
        assert!(alpha(&app, 48, 44) > 0 && alpha(&app, 48, 32) == 0);
        assert!(alpha(&app, 20, 32) > 0 && alpha(&app, 76, 32) > 0);
        assert_eq!(alpha(&app, 20, 44), 0);
        // The other line is untouched and the bent one is picked.
        assert!(alpha(&app, 48, 56) > 0);
        assert_eq!(app.picked_lines().len(), 1);
        let strokes = &layer(&app).vector.as_ref().unwrap().strokes;
        assert!(Arc::ptr_eq(&app.picked_lines()[0], &strokes[0]));
        let ends = strokes[0].control_points();
        assert_eq!(ends[0][1], 32.0);
        assert_eq!(ends.last().unwrap()[1], 32.0);
        app.undo();
        app.flush_render_queue();
        assert!(same_pixels(&before, &layer(&app).pixels, 96 * 64));
    }

    #[test]
    fn reshaping_bends_csp_lines_between_their_points() {
        let mut app = csp_app();
        // A straight line of two points: densified, its middle still bends.
        let line = layer(&app).vector.as_ref().unwrap().strokes[0]
            .csp
            .clone()
            .unwrap();
        let two = crate::clip_vector::Stroke {
            points: vec![line.points[0], *line.points.last().unwrap()],
            ..(*line).clone()
        };
        let index = app.doc.active;
        redraw_layer(
            &mut app.doc,
            index,
            VectorLayer {
                strokes: vec![Arc::new(Stroke::from_csp(two))],
            },
        )
        .unwrap();
        app.select_tool(Tool::Pointer);
        app.line_drag = crate::vector_pick::LineDrag::Reshape;
        app.bend_reach = 15.0;
        stroke_line(&mut app, [48.0, 20.0], [48.0, 30.0]);
        consistent(&app);
        assert!(
            alpha(&app, 48, 30) > 0 && alpha(&app, 48, 20) == 0,
            "{}",
            app.status
        );
        assert!(alpha(&app, 14, 20) > 0 && alpha(&app, 82, 20) > 0);
        let stroke = &layer(&app).vector.as_ref().unwrap().strokes[0];
        assert!(stroke.csp.as_ref().unwrap().points.len() > 20);
    }

    #[test]
    fn densified_lines_draw_the_same() {
        let mut app = app();
        app.brush.scatter = 0.0;
        stroke(&mut app, [8.0, 30.0], [88.0, 30.0]);
        let original = layer(&app).vector.as_ref().unwrap().strokes[0].clone();
        let dense = original.densified(1.0);
        assert!(dense.commands.len() > original.commands.len());
        let size = [96, 64];
        let paths = [original.path(size), dense.path(size)];
        // The stabilizer sees more samples, so allow a fraction of a pixel.
        let far = paths[1].iter().map(|p| {
            paths[0]
                .iter()
                .map(|q| (p[0] - q[0]).hypot(p[1] - q[1]))
                .fold(f32::MAX, f32::min)
        });
        assert!(far.fold(0.0, f32::max) < 1.0);
    }
}
