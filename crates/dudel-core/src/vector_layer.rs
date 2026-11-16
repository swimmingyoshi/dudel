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
pub const MAX_STROKES: usize = 20_000;
const MAX_COMMANDS: usize = 200_000;

/// What drove the stroke sampler, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
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
pub struct Stroke {
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
    pub fn new(brush: Brush, color: [u8; 3], symmetry: Symmetry) -> Self {
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
    pub fn from_csp(line: crate::clip_vector::Stroke) -> Self {
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
            secondary: self.color,
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
    pub fn bounds(&self, size: [usize; 2]) -> Option<[usize; 4]> {
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
    pub fn path(&self, size: [usize; 2]) -> &[[f32; 2]] {
        &self.footprint(size).path
    }
    /// Whether a circle at (`x`, `y`) of `radius` touches the stroke's line.
    pub fn touches(&self, x: f32, y: f32, radius: f32, size: [usize; 2]) -> bool {
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
    pub fn paint(&self, doc: &mut Document, layer: usize) {
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
pub fn same(a: &Option<VectorLayer>, b: &Option<VectorLayer>) -> bool {
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
pub struct VectorLayer {
    pub strokes: Vec<Arc<Stroke>>,
}

impl VectorLayer {
    pub fn validate(&self) -> Result<(), String> {
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

    /// Mark the middles of the layer's lines in `walls`, one bit a pixel. The
    /// marks join diagonally, which still stops a four-connected fill.
    pub fn mark_centres(&self, walls: &mut [u64], size: [usize; 2]) {
        let mut mark = |x: i64, y: i64| {
            if x >= 0 && y >= 0 && (x as usize) < size[0] && (y as usize) < size[1] {
                crate::fill::visit(walls, y as usize * size[0] + x as usize);
            }
        };
        for run in self.strokes.iter().flat_map(|stroke| stroke.runs(size)) {
            let last = run.len().saturating_sub(1);
            for i in 0..=last {
                let (a, b) = (run[i], run[(i + 1).min(last)]);
                let (mut x, mut y) = (a[0].floor() as i64, a[1].floor() as i64);
                let (end_x, end_y) = (b[0].floor() as i64, b[1].floor() as i64);
                let (dx, dy) = ((end_x - x).abs(), -(end_y - y).abs());
                let (step_x, step_y) = ((end_x - x).signum(), (end_y - y).signum());
                let mut error = dx + dy;
                loop {
                    mark(x, y);
                    if x == end_x && y == end_y {
                        break;
                    }
                    if 2 * error >= dy {
                        error += dy;
                        x += step_x;
                    }
                    if 2 * error <= dx {
                        error += dx;
                        y += step_y;
                    }
                }
            }
        }
    }

    /// The layer's pixels: every stroke painted in order onto a clear layer.
    pub fn render(&self, size: [usize; 2], cancel: &dyn Fn() -> bool) -> Result<Pixels, String> {
        render(self.strokes.iter(), size, cancel)
    }

    /// Redraw `pixels` inside `rect` from the strokes that reach it (the
    /// rest of the layer cannot have changed).
    pub fn redraw(
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
    pub fn mapped(&self, map: &dyn Fn(f32, f32) -> [f32; 2], size: [usize; 2]) -> Self {
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
pub enum EraseMode {
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
    pub fn cut(&self, circles: &[[f32; 3]]) -> Option<Vec<Stroke>> {
        let points = self.cut_points()?;
        let removed: Vec<bool> = points
            .iter()
            .map(|(s, r)| covered(circles, s, *r))
            .collect();
        Some(self.rebuild(&points, &removed))
    }

    /// `cut`, taking the touched part out up to the nearest lines of
    /// `others` that cross this one on either side (or up to its ends).
    pub fn cut_to_crossings(
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
        if self.csp.is_none()
            && (self.brush.scatter > 0.0 || self.symmetry.active() || self.cut_zoom().is_none())
        {
            return None;
        }
        Some(self.sized_line())
    }
    /// The line's points, each with the radius drawn there.
    fn sized_line(&self) -> Vec<(Sample, f32)> {
        if self.csp.is_some() {
            return self.line();
        }
        let half = self.brush.size * 0.5;
        let pressure_size = self.brush.pressure_size;
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
            .collect()
    }
    /// The line as unbroken runs of (x, y, radius), including its symmetry
    /// copies. A stroke lifted partway (a `Reset`) makes a new run.
    pub fn runs(&self, size: [usize; 2]) -> Vec<Vec<[f32; 3]>> {
        let mut copies: Vec<Vec<Vec<[f32; 3]>>> = Vec::new();
        for (dab, radius) in self.sized_line() {
            let spots = self.symmetry.copies(dab.x, dab.y, size[0], size[1]);
            copies.resize_with(copies.len().max(spots.len()), Vec::new);
            for (runs, ([x, y], _)) in copies.iter_mut().zip(spots) {
                let joined = runs
                    .last()
                    .and_then(|run: &Vec<[f32; 3]>| run.last())
                    .is_some_and(|p| (p[0] - x).hypot(p[1] - y) <= (radius * 8.0).max(8.0));
                if !joined {
                    runs.push(Vec::new());
                }
                runs.last_mut().unwrap().push([x, y, radius]);
            }
        }
        copies.into_iter().flatten().collect()
    }
    /// The parts of the stroke that `selected` holds: its dabs that fit
    /// inside whole, so that the line's round ends reach the edge and stop
    /// there (or, in a selection narrower than the line, its dabs centered
    /// inside). `None` when the stroke cannot be cut.
    fn inside(&self, selected: &dyn Fn(f32, f32) -> bool) -> Option<Vec<Stroke>> {
        let points = self.cut_points()?;
        let fits = |s: &Sample, r: f32| {
            selected(s.x, s.y)
                && (0..8).all(|i| {
                    let (sin, cos) = (i as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
                    selected(s.x + r * cos, s.y + r * sin)
                })
        };
        let mut removed: Vec<bool> = points.iter().map(|(s, r)| !fits(s, *r)).collect();
        if removed.iter().all(|r| *r) {
            removed = points.iter().map(|(s, _)| !selected(s.x, s.y)).collect();
        }
        if !removed.contains(&true) {
            return Some(vec![self.clone().fresh()]);
        }
        Some(self.rebuild(&points, &removed))
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
pub fn follow(
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
pub fn follow_settings(
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
pub fn follow_canvas(
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
pub fn restyle(
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
    pub fn fresh(self) -> Self {
        Self {
            footprint: OnceLock::new(),
            ..self
        }
    }
    /// The stroke in `color`.
    pub fn recolored(&self, color: [u8; 3]) -> Self {
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
pub const MAX_TAPER: f32 = 1000.0;

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
    pub fn taper(&self) -> [f32; 2] {
        if self.csp.is_some() {
            self.csp_taper
        } else {
            [self.brush.taper_in, self.brush.taper_out]
        }
    }
    /// The stroke with these tapers (`None` keeps that end's).
    pub fn with_taper(&self, start: Option<f32>, end: Option<f32>) -> Self {
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
    pub fn smoothed(&self, radius: f32) -> Self {
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
    pub fn control_points(&self) -> Vec<[f32; 2]> {
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
    pub fn densified(&self, spacing: f32) -> Self {
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
    pub fn displaced(&self, weights: &[f32], offset: [f32; 2]) -> Self {
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
/// Whether (`x`, `y`) in canvas pixels is in the selection (with none, all
/// of the canvas is). Soft selections count from half coverage.
pub fn selected_at(doc: &Document, x: f32, y: f32) -> bool {
    if x < 0.0 || y < 0.0 || x >= doc.width as f32 || y >= doc.height as f32 {
        return false;
    }
    let (x, y) = (x as usize, y as usize);
    doc.selection.is_none_or(|s| s.contains(x, y))
        && doc
            .selection_mask
            .as_ref()
            .is_none_or(|m| m.at(x, y) >= 128)
}

/// Add a finished stroke to its vector layer. Inside a selection it becomes
/// the parts of it that fit there, and the layer is redrawn to match; returns
/// the rectangle redrawn.
pub fn land_stroke(doc: &mut Document, index: usize, record: Arc<Stroke>) -> Option<[usize; 4]> {
    let size = [doc.width, doc.height];
    let pieces = (doc.selection.is_some() || doc.selection_mask.is_some())
        .then(|| record.inside(&|x, y| selected_at(doc, x, y)))
        .flatten();
    let layer = doc.layers.get_mut(index)?;
    let vector = layer.vector.as_mut()?;
    let Some(pieces) = pieces else {
        vector.strokes.push(record);
        return None;
    };
    vector.strokes.extend(pieces.into_iter().map(Arc::new));
    let rect = record.bounds(size)?;
    let vector = vector.clone();
    vector
        .redraw(&mut layer.pixels, rect, size, &|| false)
        .ok()?;
    Some(rect)
}

pub fn redraw_layer(doc: &mut Document, index: usize, vector: VectorLayer) -> Result<(), String> {
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
pub fn record(record: &mut Option<Stroke>, command: Command) {
    if let Some(record) = record
        && record.commands.len() < MAX_COMMANDS
    {
        record.commands.push(command);
    }
}

/// The vector eraser while it is dragged: strokes it touches are removed at
/// once, all in one undo step.
pub struct Erase {
    pub layer: crate::document::LayerId,
    pub brush: Brush,
    pub sampler: crate::stroke::Stroke,
    pub checkpointed: bool,
    pub modified: bool,
    pub removed: usize,
    pub mode: EraseMode,
}
