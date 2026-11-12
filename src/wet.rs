//! Wetness: where the canvas is still wet, and what happens there. Wet
//! brushes leave water with their paint. While painting, a wet dab blends
//! into paint that is still wet under it (wet-on-wet) and stays crisp on dry
//! paper (wet-on-dry). After the stroke, wet areas keep spreading and dry out
//! over a few seconds.
//!
//! Only wet 32 px tiles are stored and processed, so a dry canvas costs
//! nothing and a drying stroke costs in proportion to its wet area. This is
//! stage three of the canvas material plan.
use crate::{Dudel, document::Document, document::LayerId, egui};
use std::{collections::HashMap, sync::Arc};

const TILE: usize = 32;
/// Simulation steps per second while paint dries.
const RATE: f32 = 30.0;
/// Seconds for full water to dry on paper of middling absorbency.
const DRY_SECONDS: f32 = 3.0;
/// Below this a pixel counts as dry.
const DRY: f32 = 0.01;

/// Wet tiles by tile column and row, each 32 x 32 water values.
type Tiles = HashMap<(u32, u32), Box<[f32]>>;
/// One tile's drying result: its new pixels, its water, and whether any is left.
type TileStep = (Vec<[u8; 4]>, Box<[f32]>, bool);

/// Water per pixel (0 to 1) on one layer, in sparse tiles. Cloning shares
/// the tiles until one copy changes.
#[derive(Clone, Default)]
pub struct Field {
    layer: Option<LayerId>,
    tiles: Arc<Tiles>,
}

impl Field {
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
    pub fn clear(&mut self) {
        if !self.tiles.is_empty() {
            self.tiles = Default::default();
        }
        self.layer = None;
    }
    #[cfg(test)]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.tiles
            .get(&((x / TILE) as u32, (y / TILE) as u32))
            .map_or(0.0, |tile| tile[(y % TILE) * TILE + x % TILE])
    }
    /// Water along row `y` from `left`, for the wet layer `layer`.
    pub fn read_row(&self, layer: LayerId, y: usize, left: usize, out: &mut [f32]) {
        out.fill(0.0);
        if self.layer != Some(layer) || self.tiles.is_empty() {
            return;
        }
        let mut at = 0;
        while at < out.len() {
            let x = left + at;
            let count = (TILE - x % TILE).min(out.len() - at);
            if let Some(tile) = self.tiles.get(&((x / TILE) as u32, (y / TILE) as u32)) {
                let offset = (y % TILE) * TILE + x % TILE;
                out[at..at + count].copy_from_slice(&tile[offset..offset + count]);
            }
            at += count;
        }
    }
    /// Add water along row `y` from `left`. Water on another layer dries at
    /// once: one layer is wet at a time.
    pub fn add_row(&mut self, layer: LayerId, y: usize, left: usize, water: &[f32]) {
        if water.iter().all(|w| *w <= 0.0) {
            return;
        }
        if self.layer != Some(layer) {
            self.clear();
            self.layer = Some(layer);
        }
        let tiles = Arc::make_mut(&mut self.tiles);
        let mut at = 0;
        while at < water.len() {
            let x = left + at;
            let count = (TILE - x % TILE).min(water.len() - at);
            let span = &water[at..at + count];
            if span.iter().any(|w| *w > 0.0) {
                let tile = tiles
                    .entry(((x / TILE) as u32, (y / TILE) as u32))
                    .or_insert_with(|| vec![0.0; TILE * TILE].into_boxed_slice());
                let offset = (y % TILE) * TILE + x % TILE;
                for (cell, add) in tile[offset..offset + count].iter_mut().zip(span) {
                    *cell = (*cell + add.max(0.0)).min(1.0);
                }
            }
            at += count;
        }
    }
}

/// One drying step: paint spreads between wet neighbors, water spreads a
/// little and evaporates. Returns the changed area, or None once dry.
pub fn dry_step(doc: &mut Document) -> Option<[usize; 4]> {
    if doc.wet.is_empty() {
        return None;
    }
    let (width, height) = (doc.width, doc.height);
    let layer_index = doc
        .wet
        .layer
        .and_then(|id| doc.layers.iter().position(|l| l.id == id));
    let Some(index) = layer_index.filter(|&i| {
        let layer = &doc.layers[i];
        !layer.folder && !layer.locked && layer.vector.is_none() && layer.text.is_none()
    }) else {
        doc.wet.clear();
        return None;
    };
    let paper = doc.paper;
    let absorbency = paper.map_or(0.5, |p| p.absorbency);
    // Absorbent paper dries faster and lets paint travel less.
    let evaporation = (0.4 + 1.2 * absorbency) / (DRY_SECONDS * RATE);
    let mobility = 0.24 * (1.0 - 0.5 * absorbency);
    // Fibers carry paint further along their direction.
    let (along_x, along_y) = match paper {
        Some(p) if p.pattern == crate::paper::Pattern::Fibers => {
            let (sin, cos) = p.direction.to_radians().sin_cos();
            (0.6 + 0.8 * cos * cos, 0.6 + 0.8 * sin * sin)
        }
        _ => (1.0, 1.0),
    };
    let alpha_locked = doc.layers[index].alpha_locked;
    // On a watercolor layer, water flowing toward drier parts of a wash
    // carries pigment with it: rims darken as a wash dries, and a sudden
    // rush of fresh water (a large difference) pushes paint into blooms.
    let (carry, push) = doc.layers[index]
        .material
        .as_ref()
        .filter(|m| m.kind() == crate::material::Kind::Watercolor)
        .map_or((0.0, 0.0), |m| {
            (0.3 * m.settings.edges, 0.3 * m.settings.blooms)
        });
    let pigment = doc.layers[index].material.as_ref().is_some_and(|m| {
        matches!(
            m.kind(),
            crate::material::Kind::Watercolor | crate::material::Kind::Paint
        )
    });
    let tiles = doc.wet.tiles.clone();
    let keys: Vec<(u32, u32)> = tiles.keys().copied().collect();
    let pixels = &doc.layers[index].pixels;
    // Every tile is computed from the state before the step, then written.
    let step = |key: (u32, u32)| -> TileStep {
        let (left, top) = (key.0 as usize * TILE, key.1 as usize * TILE);
        let (right, bottom) = ((left + TILE).min(width), (top + TILE).min(height));
        // The tile with a one-pixel border, premultiplied.
        let (bl, bt) = (left.saturating_sub(1), top.saturating_sub(1));
        let (br, bb) = ((right + 1).min(width), (bottom + 1).min(height));
        let (bw, bh) = (br - bl, bb - bt);
        let bytes = pixels.region(width, [bl, bt, br, bb]);
        // Premultiplied once; every pixel is read up to five times.
        let lifted: Vec<[f32; 4]> = bytes
            .chunks_exact(4)
            .map(|p| {
                let a = f32::from(p[3]) / 255.0;
                [
                    f32::from(p[0]) * a,
                    f32::from(p[1]) * a,
                    f32::from(p[2]) * a,
                    f32::from(p[3]),
                ]
            })
            .collect();
        debug_assert_eq!(lifted.len(), bw * bh);
        let around = |x: usize, y: usize| lifted[(y - bt) * bw + (x - bl)];
        // Water over the same bordered area: a few tile lookups per row
        // instead of one per neighbor.
        let mut damp = vec![0.0f32; bw * bh];
        for y in bt..bb {
            let mut x = bl;
            while x < br {
                let count = (TILE - x % TILE).min(br - x);
                if let Some(t) = tiles.get(&((x / TILE) as u32, (y / TILE) as u32)) {
                    let from = (y % TILE) * TILE + x % TILE;
                    let to = (y - bt) * bw + (x - bl);
                    damp[to..to + count].copy_from_slice(&t[from..from + count]);
                }
                x += count;
            }
        }
        let tile = &tiles[&key];
        let mut out = Vec::with_capacity((right - left) * (bottom - top));
        let mut water = vec![0.0f32; TILE * TILE].into_boxed_slice();
        let mut wet = false;
        for y in top..bottom {
            for x in left..right {
                let w = tile[(y - top) * TILE + (x - left)];
                let here = around(x, y);
                let mut moved = [0.0f32; 4];
                // Paint arriving from each neighbor, for mixing as pigment.
                let mut arrivals = [[0.0f32; 4]; 8];
                let mut arrived = 0;
                let mut water_flow = 0.0;
                for (dx, dy, weight) in [
                    (-1i64, 0i64, along_x),
                    (1, 0, along_x),
                    (0, -1, along_y),
                    (0, 1, along_y),
                ] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let wn = damp[(ny as usize - bt) * bw + (nx as usize - bl)];
                    // Paint moves only where both sides are wet, the same
                    // amount each way, so none is made or lost.
                    let rate = mobility * weight * w.min(wn);
                    if rate > 0.0 {
                        let there = around(nx as usize, ny as usize);
                        for c in 0..4 {
                            moved[c] += rate * (there[c] - here[c]);
                        }
                        arrivals[arrived] = there.map(|v| v * rate);
                        arrived += 1;
                        // Carried by the flow: out to a drier wet neighbor,
                        // in from a wetter one. Each pair computes the same
                        // amount from both sides, so paint is conserved.
                        let gap = w - wn;
                        // Capped so a pixel never sends out more than half
                        // its paint in one step.
                        let flow = (weight * gap.abs() * (carry + push * gap.abs())).min(0.12);
                        if flow > 0.0 {
                            let from = if gap > 0.0 { &here } else { &there };
                            let sign = if gap > 0.0 { -1.0 } else { 1.0 };
                            for c in 0..4 {
                                moved[c] += sign * flow * from[c];
                            }
                            if gap < 0.0 {
                                arrivals[arrived] = there.map(|v| v * flow);
                                arrived += 1;
                            }
                        }
                    }
                    water_flow += 0.04 * weight * (wn - w);
                }
                let mut pixel = [0u8; 4];
                let alpha = (here[3] + moved[3]).clamp(0.0, 255.0);
                if alpha >= 0.5 {
                    let scale = 255.0 / alpha;
                    for c in 0..3 {
                        pixel[c] = ((here[c] + moved[c]) * scale).round().clamp(0.0, 255.0) as u8;
                    }
                    pixel[3] = alpha.round() as u8;
                    // Different paint running in mixes as pigment does: what
                    // stays here, then each neighbor's paint in turn.
                    let straight = |v: &[f32; 4]| {
                        let a = v[3] / 255.0;
                        [0, 1, 2].map(|c| (v[c] / a).round().clamp(0.0, 255.0) as u8)
                    };
                    let own = (pigment && here[3] >= 0.5).then(|| straight(&here));
                    if let Some(own) = own
                        && arrivals[..arrived].iter().any(|a| {
                            a[3] >= 0.05 && {
                                let theirs = straight(a);
                                (0..3).any(|c| theirs[c].abs_diff(own[c]) > 4)
                            }
                        })
                    {
                        let incoming: f32 = arrivals[..arrived].iter().map(|a| a[3]).sum();
                        let mut held = (alpha - incoming).max(0.0);
                        let mut color = own;
                        for arrival in &arrivals[..arrived] {
                            if arrival[3] < 0.05 {
                                continue;
                            }
                            held += arrival[3];
                            color =
                                crate::pigment::mix(color, straight(arrival), arrival[3] / held);
                        }
                        pixel[..3].copy_from_slice(&color);
                    }
                } else {
                    // Clear: keep the hidden color.
                    let p = &bytes[((y - bt) * bw + (x - bl)) * 4..][..4];
                    pixel = [p[0], p[1], p[2], 0];
                }
                if alpha_locked {
                    pixel[3] = bytes[((y - bt) * bw + (x - bl)) * 4 + 3];
                }
                out.push(pixel);
                let w = (w + water_flow - evaporation).max(0.0);
                let w = if w < DRY { 0.0 } else { w.min(1.0) };
                wet |= w > 0.0;
                water[(y - top) * TILE + (x - left)] = w;
            }
        }
        (out, water, wet)
    };
    // Large wet areas use the worker pool; small ones are not worth the handoff.
    let results: Vec<TileStep> = if keys.len() >= 24 {
        crate::parallel::run(keys.len(), |i| step(keys[i]))
            .into_iter()
            .map(|r| r.expect("a drying tile"))
            .collect()
    } else {
        keys.iter().map(|key| step(*key)).collect()
    };
    // Water spreading into dry neighbors of a tile's edge starts new tiles.
    let mut next: HashMap<(u32, u32), Box<[f32]>> = HashMap::with_capacity(keys.len());
    let mut damage: Option<[usize; 4]> = None;
    {
        let mut writer = doc.layers[index].pixels.writer();
        for (key, (out, water, wet)) in keys.iter().zip(results) {
            let (left, top) = (key.0 as usize * TILE, key.1 as usize * TILE);
            let (right, bottom) = ((left + TILE).min(width), (top + TILE).min(height));
            let mut i = 0;
            let mut changed = false;
            for y in top..bottom {
                for x in left..right {
                    let target = writer.rgba_mut(y * width + x);
                    if target != out[i] {
                        target.copy_from_slice(&out[i]);
                        changed = true;
                    }
                    i += 1;
                }
            }
            if changed {
                crate::render_region::include(&mut damage, [left, top, right, bottom]);
            }
            if wet {
                next.insert(*key, water);
            }
        }
    }
    doc.wet.tiles = Arc::new(next);
    if doc.wet.tiles.is_empty() {
        doc.wet.clear();
    }
    // Still wet with nothing moved this step is not yet dry.
    damage.or((!doc.wet.is_empty()).then_some([0, 0, 0, 0]))
}

/// Dry completely, for previews: at most ten seconds' worth of steps.
pub fn settle(doc: &mut Document) {
    for _ in 0..(10.0 * RATE) as usize {
        if dry_step(doc).is_none() {
            break;
        }
    }
    doc.wet.clear();
}

/// The app's drying state for the open project.
#[derive(Default)]
pub(crate) struct Drying {
    /// Steps owed, in fractions of a step.
    owed: f32,
    /// The document revision after our last step: anything else changing
    /// the document in between ends our undo step.
    revision: Option<u64>,
    /// Undo or redo happened: what was wet is no longer there.
    pub(crate) reset: bool,
    /// Seconds one step took lately, to keep drying to a share of each frame.
    cost: f32,
}

impl Dudel {
    /// Let wet paint spread and dry while nothing else is working on the
    /// document. The drying after a stroke is one undo step of its own.
    pub(crate) fn dry_tick(&mut self, ctx: &egui::Context) {
        if self.doc.wet.is_empty() {
            self.drying.revision = None;
            self.drying.reset = false;
            return;
        }
        let busy = self.render_queue.busy()
            || self.stroking
            || self.queued_stroke.is_some()
            || self.move_drag.is_some()
            || self.pending_move.is_some()
            || self.affine_preview.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
            || self.canvas_edit_ticket.is_some()
            || self.filter_job.is_some()
            || self.pending_layers.is_some()
            || self.text_moving
            || self.dialogs.open();
        if busy {
            // Water waits; it does not dry behind the artist's back.
            self.drying.owed = 0.0;
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if std::mem::take(&mut self.drying.reset) {
            self.doc.wet.clear();
            self.drying.revision = None;
            return;
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        // Drying may use about a quarter of the time: a huge wet wash dries
        // more slowly instead of making painting stutter.
        let rate = RATE.min(0.25 / self.drying.cost.max(1e-4));
        self.drying.owed = (self.drying.owed + dt * rate).min(4.0);
        let steps = self.drying.owed.floor() as usize;
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        if steps == 0 {
            return;
        }
        self.drying.owed -= steps as f32;
        if self.drying.revision != Some(self.revision) {
            // A new drying step in the undo history; an earlier one ended
            // when something else changed the document.
            self.history.begin_gesture_checkpoint(&self.doc);
        }
        let mut damage = None;
        let started = std::time::Instant::now();
        let mut done = 0;
        for _ in 0..steps {
            done += 1;
            match dry_step(&mut self.doc) {
                Some([_, _, 0, 0]) => {}
                Some(rect) => crate::render_region::include(&mut damage, rect),
                None => break,
            }
        }
        let cost = started.elapsed().as_secs_f32() / done as f32;
        self.drying.cost = self.drying.cost * 0.7 + cost * 0.3;
        if let Some(rect) = damage {
            self.dirty = true;
            self.dirty_region.include(rect);
            self.modified = true;
            self.revision = self.revision.wrapping_add(1);
            self.content_revision = self.content_revision.wrapping_add(1);
        }
        self.drying.revision = Some(self.revision);
        if self.doc.wet.is_empty() {
            self.history.settle(&self.doc);
            self.drying.revision = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::{Blend, Brush};

    fn wash() -> Brush {
        Brush {
            size: 24.0,
            hardness: 0.9,
            flow: 0.5,
            spacing: 0.1,
            pressure_size: false,
            blend: Blend::Wet,
            wetness: 1.0,
            ..Default::default()
        }
    }
    fn stroke(doc: &mut Document, brush: &Brush, color: [u8; 3], y: f32) {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color,
            secondary: color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=20 {
            let sample = crate::stroke::Sample {
                x: 30.0 + i as f32 * 5.0,
                y,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 20 {
                path.finish(sample, brush)
            } else {
                path.sample(sample, brush)
            };
            for dab in dabs {
                buffer.paint(doc, dab, &settings);
            }
        }
        buffer.finish_tail(doc, &settings);
    }
    /// Total paint on the layer: alpha summed over every pixel.
    fn paint(doc: &Document) -> f64 {
        let (w, h) = (doc.width, doc.height);
        doc.layers[0]
            .pixels
            .region(w, [0, 0, w, h])
            .chunks_exact(4)
            .map(|p| f64::from(p[3]))
            .sum()
    }

    #[test]
    fn wet_strokes_leave_water_that_spreads_paint_and_dries() {
        let mut doc = Document::new(160, 80);
        stroke(&mut doc, &wash(), [200, 30, 30], 34.0);
        assert!(doc.wet.at(80, 34) > 0.9, "the stroke's core is wet");
        assert_eq!(doc.wet.at(80, 5), 0.0);
        // A blue stroke beside it, overlapping a little while both are wet.
        stroke(&mut doc, &wash(), [30, 30, 200], 54.0);
        // In the red just above the blue's edge, blue creeps in as it dries.
        let blue = |doc: &Document| doc.layers[0].pixels.rgba(40 * 160 + 80)[2];
        let before = (blue(&doc), paint(&doc));
        let mut steps = 0;
        while dry_step(&mut doc).is_some() {
            steps += 1;
            assert!(steps < 600, "drying ends");
        }
        assert!(doc.wet.is_empty());
        assert!(steps > 30, "drying takes a while: {steps}");
        assert!(
            blue(&doc) > before.0 + 5,
            "{} then {}",
            before.0,
            blue(&doc)
        );
        // Spreading moves paint; it does not make or lose it (within rounding).
        let after = paint(&doc);
        assert!(
            (after - before.1).abs() / before.1 < 0.02,
            "{} {after}",
            before.1
        );
        // Far from the stroke nothing changed.
        assert_eq!(doc.layers[0].pixels.rgba(5 * 160 + 80)[3], 0);
    }

    #[test]
    fn wet_on_wet_blends_and_wet_on_dry_stays_crisp() {
        // Blue over red: while the red is wet the overlap mixes more red in.
        let overlap = |dry_first: bool| {
            let mut doc = Document::new(160, 80);
            stroke(&mut doc, &wash(), [200, 30, 30], 34.0);
            if dry_first {
                settle(&mut doc);
            }
            let light = Brush {
                flow: 0.15,
                ..wash()
            };
            stroke(&mut doc, &light, [30, 30, 200], 52.0);
            // Just inside the blue stroke's upper edge, over red's lower edge.
            doc.layers[0].pixels.rgba(42 * 160 + 80)
        };
        let (wet, dry) = (overlap(false), overlap(true));
        assert_ne!(wet, dry, "wet paint underneath changes the result");
        // A plain brush leaves no water.
        let mut doc = Document::new(160, 80);
        stroke(&mut doc, &Brush::default(), [0, 0, 0], 40.0);
        assert!(doc.wet.is_empty());
    }

    #[test]
    fn drying_in_the_app_is_one_undo_step_and_undo_stops_it() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(160, 80),
            ..Default::default()
        };
        stroke(&mut app.doc, &wash(), [200, 30, 30], 34.0);
        stroke(&mut app.doc, &wash(), [30, 30, 200], 54.0);
        let painted = app.doc.layers[0].pixels.clone();
        let mut ticks = 0;
        while !app.doc.wet.is_empty() {
            app.dry_tick(&ctx);
            ticks += 1;
            assert!(ticks < 5000, "drying ends");
        }
        assert!(
            app.doc.layers[0].pixels != painted,
            "paint spread while drying"
        );
        assert!(app.modified);
        // One undo takes back the drying, and only the drying.
        app.undo();
        assert!(app.doc.layers[0].pixels == painted);
        assert!(!app.history.can_undo());
        // Redo brings the dried result back; nothing is wet any more.
        app.redo();
        assert!(app.doc.layers[0].pixels != painted);
        app.dry_tick(&ctx);
        assert!(app.doc.wet.is_empty());
        // Undo in the middle of drying stops it.
        let mut app = Dudel {
            doc: Document::new(160, 80),
            ..Default::default()
        };
        stroke(&mut app.doc, &wash(), [200, 30, 30], 34.0);
        for _ in 0..10 {
            app.dry_tick(&ctx);
        }
        assert!(!app.doc.wet.is_empty());
        app.undo();
        app.dry_tick(&ctx);
        assert!(app.doc.wet.is_empty());
    }

    #[test]
    fn water_follows_one_layer_and_locked_layers_do_not_dry() {
        let mut doc = Document::new(160, 80);
        stroke(&mut doc, &wash(), [200, 30, 30], 40.0);
        assert!(!doc.wet.is_empty());
        doc.layers[0].locked = true;
        let before = doc.layers[0].pixels.clone();
        assert!(dry_step(&mut doc).is_none());
        assert!(doc.wet.is_empty() && doc.layers[0].pixels == before);
        // Saved documents carry no water.
        let mut doc = Document::new(64, 64);
        stroke(&mut doc, &wash(), [200, 30, 30], 30.0);
        let json = serde_json::to_string(&doc).unwrap();
        assert!(!json.contains("wet"));
    }
}

/// `DUDEL_WET_SHEET=out.png cargo test --release wet_sheet -- --ignored
/// --nocapture` paints overlapping washes on cold-pressed paper, saves them
/// wet (top) and dried (bottom), and times drying a large wet area.
#[cfg(test)]
#[test]
#[ignore = "writes a picture and prints timings for review"]
fn wet_sheet() {
    let Ok(path) = std::env::var("DUDEL_WET_SHEET") else {
        return;
    };
    let wash = crate::workspace::default_brushes()
        .into_iter()
        .find(|b| b.name == "Watercolor wash")
        .unwrap();
    let (w, h) = (640usize, 260usize);
    let mut doc = Document::new(w, h);
    doc.paper = Some(crate::paper::Paper::presets()[1].1);
    let run = |doc: &mut Document, color: [u8; 3], points: &[[f32; 2]], pressure: f32| {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: wash.clone(),
            color,
            secondary: color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        let steps = 80;
        for i in 0..=steps {
            let t = i as f32 / steps as f32 * (points.len() - 1) as f32;
            let (a, b) = (
                points[t.floor() as usize],
                points[(t.ceil() as usize).min(points.len() - 1)],
            );
            let f = t - t.floor();
            let sample = crate::stroke::Sample {
                x: a[0] + (b[0] - a[0]) * f,
                y: a[1] + (b[1] - a[1]) * f,
                pressure,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == steps {
                path.finish(sample, &wash)
            } else {
                path.sample(sample, &wash)
            };
            for dab in dabs {
                buffer.paint(doc, dab, &settings);
            }
        }
    };
    for pass in 0..3 {
        let y = 70.0 + pass as f32 * 8.0;
        run(
            &mut doc,
            [36, 84, 170],
            &[[60.0, y], [330.0, y + 40.0]],
            0.9,
        );
    }
    for pass in 0..3 {
        let y = 150.0 + pass as f32 * 8.0;
        run(
            &mut doc,
            [200, 60, 50],
            &[[250.0, y], [580.0, y - 60.0]],
            0.9,
        );
    }
    run(
        &mut doc,
        [230, 180, 60],
        &[[120.0, 200.0], [300.0, 120.0], [480.0, 200.0]],
        0.7,
    );
    let mut sheet = doc.composite();
    let wet_tiles = doc.wet.tiles.len();
    let start = std::time::Instant::now();
    let mut steps = 0;
    while dry_step(&mut doc).is_some() {
        steps += 1;
    }
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{wet_tiles} wet tiles ({} px), {steps} steps, {:.2} ms per step",
        wet_tiles * TILE * TILE,
        elapsed / f64::from(steps.max(1))
    );
    sheet.extend(doc.composite());
    image::save_buffer(
        path,
        &sheet,
        w as u32,
        (h * 2) as u32,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    // A large wash: 2000 x 1000 px wet at once.
    let mut big = Document::new(2000, 1000);
    let row = vec![1.0f32; 2000];
    let id = big.layers[0].id;
    for y in 0..1000 {
        big.wet.add_row(id, y, 0, &row);
    }
    let start = std::time::Instant::now();
    for _ in 0..10 {
        dry_step(&mut big);
    }
    println!(
        "2,000,000 wet px: {:.2} ms per step",
        start.elapsed().as_secs_f64() * 100.0
    );
}
