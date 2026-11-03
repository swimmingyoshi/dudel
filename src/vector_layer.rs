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
    /// Where its dabs land, worked out on first use.
    footprint: OnceLock<Footprint>,
}
impl PartialEq for Stroke {
    fn eq(&self, other: &Self) -> bool {
        self.brush == other.brush
            && self.color == other.color
            && self.symmetry == other.symmetry
            && self.commands == other.commands
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

/// Dab circles (center and a radius that covers the dab) and their bounds.
#[derive(Clone, Default)]
struct Footprint {
    circles: Vec<[f32; 3]>,
    bounds: Option<[f32; 4]>,
}

impl Stroke {
    pub(crate) fn new(brush: Brush, color: [u8; 3], symmetry: Symmetry) -> Self {
        Self {
            brush: Arc::new(brush),
            color,
            symmetry,
            commands: Vec::new(),
            footprint: OnceLock::new(),
        }
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
            let reach = self.brush.size * (1.0 + self.brush.scatter) + 2.0;
            let mut footprint = Footprint::default();
            for dab in self.dabs() {
                for ([x, y], _) in self.symmetry.copies(dab.x, dab.y, size[0], size[1]) {
                    footprint.circles.push([x, y, reach]);
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
        let line = self.brush.size * 0.5;
        footprint.circles.iter().any(|&[cx, cy, _]| {
            let r = radius + line;
            (cx - x) * (cx - x) + (cy - y) * (cy - y) <= r * r
        })
    }
    /// Paint the stroke onto layer `layer` of `doc`, as it was drawn.
    pub(crate) fn paint(&self, doc: &mut Document, layer: usize) {
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
    fn mapped(&self, map: &dyn Fn(f32, f32) -> [f32; 2], size: [usize; 2]) -> Self {
        let mut stroke = Stroke {
            brush: self.brush.clone(),
            color: self.color,
            symmetry: self.symmetry,
            commands: self.commands.clone(),
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
}

impl Stroke {
    /// The stroke with the parts under `circles` (x, y, radius) taken out,
    /// as the pieces that remain (none when it is all covered). Each piece
    /// follows the line as drawn: its recorded points are the line's dabs,
    /// drawn without the stabilizer, which already shaped them. `None` when
    /// the stroke cannot be cut (scattered or mirrored dabs are not a line).
    fn cut(&self, circles: &[[f32; 3]]) -> Option<Vec<Stroke>> {
        if self.brush.scatter > 0.0 || self.symmetry.active() {
            return None;
        }
        let zoom = self.commands.iter().find_map(|c| match c {
            Command::Sample { zoom, .. } => Some(*zoom),
            _ => None,
        })?;
        let dabs = self.dabs();
        let half = self.brush.size * 0.5;
        let covered = |d: &Sample| {
            let r = half
                * if self.brush.pressure_size {
                    d.pressure.max(0.05)
                } else {
                    1.0
                };
            circles.iter().any(|&[x, y, radius]| {
                let reach = radius + r;
                (d.x - x) * (d.x - x) + (d.y - y) * (d.y - y) <= reach * reach
            })
        };
        let mut pieces = Vec::new();
        let mut run: Vec<Sample> = Vec::new();
        let mut first_run = true;
        let last = dabs.len();
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
                    footprint: OnceLock::new(),
                });
            }
            run.clear();
        };
        for (i, dab) in dabs.iter().enumerate() {
            if covered(dab) {
                flush(&mut run, first_run, false);
                first_run = false;
            } else {
                run.push(*dab);
            }
            if i + 1 == last {
                flush(&mut run, first_run, true);
            }
        }
        Some(pieces)
    }
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
            let mut restyled = Stroke {
                footprint: OnceLock::new(),
                ..(**stroke).clone()
            };
            if let Some(color) = color {
                restyled.color = color;
            }
            if let Some(brush) = &brush {
                restyled.brush = brush.clone();
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
        brush: usize,
        color: [u8; 3],
        #[serde(default)]
        symmetry: Symmetry,
        commands: String,
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
            Some("Vector layers draw without selections — deselect (Ctrl+D) to draw here")
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
}
