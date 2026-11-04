use crate::brush::Brush;

#[derive(Clone, Copy, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    /// Native monotonic seconds. Missing for frame-batched egui fallback input.
    pub time: Option<f64>,
    pub tilt: Option<[f32; 2]>,
    pub twist: Option<f32>,
    pub eraser: bool,
    pub barrel: bool,
    pub pressure_known: bool,
    /// Normalized screen velocity; absent when timing is unavailable.
    pub speed: Option<f32>,
    pub random: Option<f32>,
    pub taper: Option<f32>,
    pub spacing: Option<f32>,
    /// Degrees the tip turns for this dab on top of the brush angle.
    pub rotation: Option<f32>,
    /// Arc length before scattering/symmetry; f64 avoids long-stroke precision loss.
    pub distance: f64,
}

#[derive(Default)]
pub struct Stroke {
    previous: Option<Sample>,
    until_dab: f32,
    stabilized: Option<Sample>,
    /// Last direction actually drawn by the stabilizer, before scatter/symmetry.
    tangent: Option<[f32; 2]>,
    finished: bool,
    /// Raw pen samples the stabilized point still trails, oldest first. The
    /// stabilized point takes the pressure the pen had where it now is, and
    /// lift replays the rest instead of drawing it with pen-up pressure.
    trail: std::collections::VecDeque<Sample>,
    last_dab: Option<Sample>,
    dab_index: u32,
    velocity: crate::dynamics::Velocity,
    length: f64,
    /// Last dab position before scatter and the stroke heading there, for
    /// tips that turn with the stroke.
    heading_from: Option<[f32; 2]>,
    heading: f32,
    /// View zoom of the latest sample; sizes the rope in screen pixels.
    zoom: f32,
    /// Optional native-output allowance for extension-generated strokes.
    dab_budget: Option<usize>,
    budget_exhausted: bool,
}

/// Assumed pen-packet interval when input has no native timestamps.
const FALLBACK_INTERVAL: f32 = 1.0 / 120.0;
/// Longest gap treated as continuous motion; a stalled callback or a pen
/// resting without packets must not jump the line all the way to the pen.
const MAX_INTERVAL: f32 = 0.1;
/// Smoothing time per stabilizer unit: 10 damps hand tremor lightly (20 ms),
/// 50 (the maximum) follows the pen with a heavy 0.1 s easing.
const SECONDS_PER_UNIT: f32 = 0.002;
/// Rope length per stabilizer unit, in screen pixels: 50 gives a 100 px rope.
const ROPE_PX_PER_UNIT: f32 = 2.0;
/// The rope's follow is a lighter easing than the smooth stabilizer's.
const ROPE_EASING: f32 = 0.35;

/// Rope length in document pixels, or zero outside rope mode.
pub fn rope_radius(brush: &Brush, zoom: f32) -> f32 {
    if brush.stabilizer_mode != crate::brush::StabilizerMode::Rope {
        return 0.0;
    }
    let zoom = if zoom.is_finite() && zoom > 0.0 {
        zoom
    } else {
        1.0
    };
    brush
        .rope_distance
        .unwrap_or(brush.stabilization.max(0.0) * ROPE_PX_PER_UNIT)
        / zoom
}
/// Where a rope of `radius` pulls the line: it stays put while the pen is
/// within reach, otherwise it moves to `radius` short of the pen.
fn rope_goal(current: [f32; 2], pen: [f32; 2], radius: f32) -> [f32; 2] {
    let (dx, dy) = (pen[0] - current[0], pen[1] - current[1]);
    let distance = dx.hypot(dy);
    if distance <= radius {
        return current;
    }
    let keep = radius / distance;
    [pen[0] - dx * keep, pen[1] - dy * keep]
}

/// Time constant of the stabilizer: the drawn point covers about 63% of its
/// distance to the pen in this many seconds. Zero means no stabilization.
fn smoothing_seconds(brush: &Brush, speed: Option<f32>) -> f32 {
    let mut seconds = brush.stabilization.max(0.0) * SECONDS_PER_UNIT;
    if brush.stabilizer_mode == crate::brush::StabilizerMode::Rope {
        seconds *= ROPE_EASING;
    }
    if brush.stabilization_by_speed {
        seconds *= 1.0 - speed.unwrap_or(0.0).clamp(0.0, 1.0);
    }
    seconds
}

/// Share of the remaining distance to a resting pen that the stabilized line
/// covers in `elapsed` seconds; the same easing as while the pen moves.
pub fn smoothing_fraction(elapsed: f32, brush: &Brush) -> f32 {
    let seconds = smoothing_seconds(brush, None);
    if seconds > 0.0 {
        1.0 - (-elapsed.clamp(0.0, MAX_INTERVAL) / seconds).exp()
    } else {
        1.0
    }
}

impl Stroke {
    /// Bound generation itself, before output vectors grow. The caller shares
    /// the remaining allowance across tracks and all hooks of one input event.
    pub(crate) fn bounded(
        &mut self,
        remaining: &mut usize,
        generate: impl FnOnce(&mut Self) -> Vec<Sample>,
    ) -> Result<Vec<Sample>, String> {
        self.dab_budget = Some(*remaining);
        self.budget_exhausted = false;
        let dabs = generate(self);
        *remaining = self.dab_budget.take().expect("generation budget is set");
        if self.budget_exhausted {
            Err("Tool generated too many brush dabs in one input event; use shorter paths or wider spacing".into())
        } else {
            Ok(dabs)
        }
    }

    fn reserve_dab(&mut self) -> bool {
        if let Some(remaining) = &mut self.dab_budget {
            if *remaining == 0 {
                self.budget_exhausted = true;
                return false;
            }
            *remaining -= 1;
        }
        true
    }

    pub fn reset(&mut self) {
        self.previous = None;
        self.until_dab = 0.0;
        self.stabilized = None;
        self.tangent = None;
        self.finished = false;
        self.trail.clear();
        self.last_dab = None;
        self.dab_index = 0;
        self.velocity = Default::default();
        self.length = 0.0;
        self.heading_from = None;
        self.heading = 0.0;
        self.zoom = 1.0;
    }
    fn spacing(brush: &Brush, sample: &Sample) -> f32 {
        (brush.size
            * brush.aspect
            * brush.spacing
            * brush.size_factor(sample)
            * brush
                .dynamics
                .spacing
                .as_ref()
                .map_or(1.0, |d| d.evaluate(sample)))
        .max(0.5)
    }
    fn random_value(index: u32, seed: u32) -> f32 {
        let mut hash = index.wrapping_add(seed).wrapping_add(1);
        hash ^= hash >> 16;
        hash = hash.wrapping_mul(0x7feb352d);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(0x846ca68b);
        hash ^= hash >> 16;
        (hash & 0xffff) as f32 / 65535.0
    }
    /// Carry distance to the next dab across packets instead of restarting at
    /// every frame. Pressure is interpolated at each actual dab position.
    pub fn sample(&mut self, sample: Sample, brush: &Brush) -> Vec<Sample> {
        self.sample_at_zoom(sample, brush, 1.0)
    }

    pub fn sample_at_zoom(&mut self, mut sample: Sample, brush: &Brush, zoom: f32) -> Vec<Sample> {
        self.finished = false;
        let zoom = if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        };
        self.zoom = zoom;
        sample.speed = self.velocity.measure(&sample, zoom);
        let smoothing = smoothing_seconds(brush, sample.speed);
        let radius = rope_radius(brush, zoom);
        let previous_pen = self.trail.back().copied();
        self.trail.push_back(sample);
        let Some(mut current) = self.stabilized else {
            self.stabilized = Some(sample);
            let dabs = self.sample_path(sample, brush);
            return self.scatter(dabs, brush);
        };
        // The drawn point eases toward the pen: each second it covers the same
        // share of the remaining distance, so it smooths jitter while moving
        // and glides onto a resting pen. Follow the pen's path in short steps;
        // one long hop toward a far sample would cut corners off fast curves.
        let travel = previous_pen.map_or(0.0, |pen| (sample.x - pen.x).hypot(sample.y - pen.y));
        let steps = if smoothing > 0.0 && travel > 0.0 {
            (travel * zoom / 2.0).ceil().clamp(1.0, 32.0) as usize
        } else {
            1
        };
        let elapsed = match (sample.time, previous_pen.and_then(|pen| pen.time)) {
            (Some(now), Some(before)) => ((now - before) as f32).clamp(0.0, MAX_INTERVAL),
            _ => FALLBACK_INTERVAL,
        };
        let pull = if smoothing > 0.0 {
            1.0 - (-elapsed / steps as f32 / smoothing).exp()
        } else {
            1.0
        };
        let from = previous_pen.unwrap_or(sample);
        let mut dabs = Vec::new();
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let target = [
                from.x + (sample.x - from.x) * t,
                from.y + (sample.y - from.y) * t,
            ];
            let goal = rope_goal([current.x, current.y], target, radius);
            let dx = goal[0] - current.x;
            let dy = goal[1] - current.y;
            let (x, y) = (current.x + dx * pull, current.y + dy * pull);
            // Path length still between this point and the pen.
            let lag = (target[0] - x).hypot(target[1] - y) + travel * (1.0 - t);
            let (_, pressure) = self.trail_behind(lag);
            let next = Sample {
                x,
                y,
                pressure,
                ..sample
            };
            let (mx, my) = (next.x - current.x, next.y - current.y);
            let moved = mx.hypot(my);
            if moved > 0.0001 {
                self.tangent = Some([mx / moved, my / moved]);
            }
            current = next;
            dabs.extend(self.sample_path(current, brush));
        }
        let lag = (sample.x - current.x).hypot(sample.y - current.y);
        let (ahead, _) = self.trail_behind(lag);
        self.trail.drain(..ahead.saturating_sub(1));
        self.stabilized = Some(current);
        self.scatter(dabs, brush)
    }

    /// The latest dab placed, before scatter and symmetry.
    pub fn last_dab(&self) -> Option<Sample> {
        self.last_dab
    }
    /// The latest raw pen position the stabilizer is following.
    pub fn pen(&self) -> Option<Sample> {
        self.trail
            .back()
            .copied()
            .filter(|_| self.stabilized.is_some() && !self.finished)
    }
    /// Whether the drawn line still trails where the stabilizer is taking
    /// it by more than `tolerance`: the pen, or the rope's end.
    pub fn lagging(&self, tolerance: f32, brush: &Brush) -> bool {
        match (self.stabilized, self.pen()) {
            (Some(at), Some(pen)) => {
                let radius = rope_radius(brush, self.zoom);
                (pen.x - at.x).hypot(pen.y - at.y) - radius > tolerance
            }
            _ => false,
        }
    }
    /// The drawn point and the pen while a rope stabilizer holds the line.
    pub fn rope(&self, brush: &Brush) -> Option<([f32; 2], [f32; 2], f32)> {
        let radius = rope_radius(brush, self.zoom);
        match (self.stabilized, self.pen()) {
            (Some(at), Some(pen)) if radius > 0.0 => Some(([at.x, at.y], [pen.x, pen.y], radius)),
            _ => None,
        }
    }
    /// While the pen rests, pull the drawn line `fraction` of the remaining
    /// way to it, so a stabilized stroke reaches the pen instead of stopping a
    /// stabilizer radius short. Pressure follows the pen's recorded path.
    pub fn catch_up(&mut self, fraction: f32, brush: &Brush) -> Vec<Sample> {
        let (Some(previous), Some(pen)) = (self.stabilized, self.pen()) else {
            return vec![];
        };
        // A rope only tightens to its length; it never reaches a resting pen.
        let goal = rope_goal(
            [previous.x, previous.y],
            [pen.x, pen.y],
            rope_radius(brush, self.zoom),
        );
        let dx = goal[0] - previous.x;
        let dy = goal[1] - previous.y;
        let distance = dx.hypot(dy);
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if distance < 0.01 || fraction == 0.0 {
            return vec![];
        }
        let (x, y) = (previous.x + dx * fraction, previous.y + dy * fraction);
        let (ahead, pressure) = self.trail_behind((pen.x - x).hypot(pen.y - y));
        self.trail.drain(..ahead.saturating_sub(1));
        let next = Sample {
            x,
            y,
            pressure,
            ..pen
        };
        self.tangent = Some([dx / distance, dy / distance]);
        self.stabilized = Some(next);
        let dabs = self.sample_path(next, brush);
        self.scatter(dabs, brush)
    }

    /// Lift completes the stabilizer's trailing segment; it does not leave the
    /// final pointer position unpainted or apply another dab to a stationary tap.
    pub fn finish(&mut self, sample: Sample, brush: &Brush) -> Vec<Sample> {
        self.finish_at_zoom(sample, brush, 1.0)
    }
    pub fn finish_at_zoom(&mut self, mut sample: Sample, brush: &Brush, zoom: f32) -> Vec<Sample> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let zoom = if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        };
        let contact = self.trail.back().copied();
        let released = sample.pressure == 0.0;
        if brush.stabilization > 0.0
            && released
            && sample.pressure_known
            && let Some(contact) = contact
        {
            // The out-of-contact packet may already contain hover movement.
            // Complete the smoothed ink at the last contact, not that air path.
            sample.x = contact.x;
            sample.y = contact.y;
        }
        // A rope ends the line where it holds it, not at the pen, so lifting
        // never adds a final jump; pressure is what the pen had there.
        let mut rope_pressure = None;
        if rope_radius(brush, zoom) > 0.0
            && let Some(at) = self.stabilized
        {
            let lag = (sample.x - at.x).hypot(sample.y - at.y);
            rope_pressure = Some(self.trail_behind(lag).1);
            sample.x = at.x;
            sample.y = at.y;
        }
        sample.speed = self.velocity.measure(&sample, zoom);
        let mut contact_end = sample;
        if released && let Some(contact) = contact {
            contact_end.pressure = contact.pressure;
        }
        if let Some(pressure) = rope_pressure {
            contact_end.pressure = pressure;
        }
        let pending = self.stabilized.filter(|start| {
            brush.stabilization > 0.0 && (sample.x - start.x).hypot(sample.y - start.y) > 0.0001
        });
        let mut dabs = if let Some(start) = pending {
            // Raw positions supply pressure history only. Replaying their
            // geometry would put the rejected jitter back into the finished line.
            self.trail.push_back(contact_end);
            self.finish_smoothed(start, sample, brush, zoom, released)
        } else {
            // Preserve the existing unsmoothed and stationary-tap behavior.
            sample.pressure = contact_end.pressure;
            let points = self.sample_path(sample, brush);
            self.scatter(points, brush)
        };
        self.trail.clear();
        self.stabilized = Some(sample);
        self.tangent = None;
        if self
            .last_dab
            .is_some_and(|last| (last.x - sample.x).hypot(last.y - sample.y) > 0.001)
        {
            sample.spacing = self
                .last_dab
                .map(|last| (sample.x - last.x).hypot(sample.y - last.y));
            sample.taper = Self::taper(brush, self.length);
            sample.distance = self.length;
            self.last_dab = Some(sample);
            if self.reserve_dab() {
                dabs.extend(self.scatter(vec![sample], brush));
            }
        }
        dabs
    }

    /// A forward quadratic continuation preserves the incoming smoothed tangent
    /// without retracing raw wiggles. Its control point cannot overshoot the end
    /// along the chord, so it cannot create a pen-up loop or backward spike.
    fn finish_smoothed(
        &mut self,
        start: Sample,
        end: Sample,
        brush: &Brush,
        zoom: f32,
        released: bool,
    ) -> Vec<Sample> {
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let lag = dx.hypot(dy);
        let tangent = self.tangent.unwrap_or([dx / lag, dy / lag]);
        let forward = (tangent[0] * dx + tangent[1] * dy) / lag;
        let handle = if forward >= 0.0 { lag * 0.5 } else { 0.0 };
        let control = [start.x + tangent[0] * handle, start.y + tangent[1] * handle];
        let steps = ((lag * zoom * 8.0).sqrt().ceil() as usize).clamp(8, 256);
        let point = |at: usize| {
            if at == steps {
                return [end.x, end.y];
            }
            let t = at as f32 / steps as f32;
            let s = 1.0 - t;
            [
                s * s * start.x + 2.0 * s * t * control[0] + t * t * end.x,
                s * s * start.y + 2.0 * s * t * control[1] + t * t * end.y,
            ]
        };
        let mut length = 0.0;
        let mut previous = [start.x, start.y];
        for at in 1..=steps {
            let next = point(at);
            length += (next[0] - previous[0]).hypot(next[1] - previous[1]);
            previous = next;
        }
        // Compact pressure knots in reverse arc distance. Stationary raw
        // packets add no knots; each raw sample is inspected once at lift.
        let mut pressures = vec![(0.0, self.trail.back().map_or(end.pressure, |p| p.pressure))];
        let mut behind = 0.0;
        for at in (1..self.trail.len()).rev() {
            let (a, b) = (self.trail[at], self.trail[at - 1]);
            let segment = (a.x - b.x).hypot(a.y - b.y);
            if segment <= f32::EPSILON {
                continue;
            }
            let take = segment.min(lag - behind);
            behind += take;
            pressures.push((
                behind,
                a.pressure + (b.pressure - a.pressure) * (take / segment),
            ));
            if behind >= lag {
                break;
            }
        }
        if pressures.last().unwrap().0 < lag {
            pressures.push((lag, start.pressure));
        }
        pressures.last_mut().unwrap().1 = start.pressure;
        let mut dabs = Vec::new();
        let mut distance = 0.0;
        previous = [start.x, start.y];
        for at in 1..=steps {
            let next = point(at);
            distance += (next[0] - previous[0]).hypot(next[1] - previous[1]);
            previous = next;
            let progress = if at == steps {
                1.0
            } else {
                (distance / length).clamp(0.0, 1.0)
            };
            let remaining = lag * (1.0 - progress);
            let upper = pressures.partition_point(|(position, _)| *position < remaining);
            let mut pressure = if upper == 0 {
                pressures[0].1
            } else if upper == pressures.len() {
                start.pressure
            } else {
                let (a, b) = (pressures[upper - 1], pressures[upper]);
                a.1 + (b.1 - a.1) * ((remaining - a.0) / (b.0 - a.0))
            };
            if released {
                // Keep the first half's recorded pressure. Ease only the last
                // half to actual release pressure, giving a fine C1 needle tip.
                let t = ((progress - 0.5) * 2.0).clamp(0.0, 1.0);
                pressure *= 1.0 - t * t * (3.0 - 2.0 * t);
            }
            let sample = Sample {
                x: next[0],
                y: next[1],
                pressure,
                time: start
                    .time
                    .zip(end.time)
                    .map(|(a, b)| a + (b - a) * f64::from(progress)),
                speed: start
                    .speed
                    .zip(end.speed)
                    .map(|(a, b)| a + (b - a) * progress)
                    .or(end.speed),
                ..end
            };
            let points = self.sample_path(sample, brush);
            // Advance random/dab state between subdivisions just as for live
            // sampling, rather than restarting it for each curve segment.
            dabs.extend(self.scatter(points, brush));
        }
        dabs
    }

    /// Walks the trail back `lag` path length from its newest sample. Returns
    /// the index of the oldest sample still ahead of that point and the pen
    /// pressure interpolated there.
    fn trail_behind(&self, lag: f32) -> (usize, f32) {
        let mut behind = 0.0;
        for i in (1..self.trail.len()).rev() {
            let (a, b) = (self.trail[i], self.trail[i - 1]);
            let segment = (a.x - b.x).hypot(a.y - b.y);
            if behind + segment >= lag {
                let t = if segment > 0.0 {
                    (lag - behind) / segment
                } else {
                    0.0
                };
                return (i, a.pressure + (b.pressure - a.pressure) * t);
            }
            behind += segment;
        }
        (0, self.trail.front().map_or(1.0, |s| s.pressure))
    }

    fn scatter(&mut self, mut dabs: Vec<Sample>, brush: &Brush) -> Vec<Sample> {
        for dab in &mut dabs {
            let mut hash = self
                .dab_index
                .wrapping_add(brush.scatter_seed)
                .wrapping_add(1);
            self.dab_index = self.dab_index.wrapping_add(1);
            let mut random = || {
                hash ^= hash >> 16;
                hash = hash.wrapping_mul(0x7feb352d);
                hash ^= hash >> 15;
                hash = hash.wrapping_mul(0x846ca68b);
                hash ^= hash >> 16;
                (hash & 0xffff) as f32 / 65535.0
            };
            let value = random();
            dab.random = Some(value);
            // Rotation is measured on the unscattered path. Its random draw
            // only happens with jitter on, so other brushes paint as before.
            if brush.angle_source != crate::brush::AngleSource::Fixed || brush.angle_jitter > 0.0 {
                if let Some(from) = self.heading_from {
                    let (dx, dy) = (dab.x - from[0], dab.y - from[1]);
                    if dx.hypot(dy) > 0.01 {
                        self.heading = dy.atan2(dx).to_degrees();
                    }
                }
                self.heading_from = Some([dab.x, dab.y]);
                let base = match brush.angle_source {
                    crate::brush::AngleSource::Fixed => 0.0,
                    crate::brush::AngleSource::Direction => self.heading,
                    crate::brush::AngleSource::Tilt => dab
                        .tilt
                        .filter(|[x, y]| x.hypot(*y) > 1.0)
                        .map_or(self.heading, |[x, y]| y.atan2(x).to_degrees()),
                };
                let jitter = if brush.angle_jitter > 0.0 {
                    brush.angle_jitter * (random() * 2.0 - 1.0)
                } else {
                    0.0
                };
                dab.rotation = Some(base + jitter);
            }
            if brush.scatter == 0.0 {
                continue;
            }
            let angle = value * std::f32::consts::TAU;
            let radius = random().sqrt() * brush.scatter * brush.size * brush.size_factor(dab);
            dab.x += angle.cos() * radius;
            dab.y += angle.sin() * radius;
        }
        dabs
    }

    fn sample_path(&mut self, mut sample: Sample, brush: &Brush) -> Vec<Sample> {
        sample.random = Some(Self::random_value(self.dab_index, brush.scatter_seed));
        let Some(previous) = self.previous.replace(sample) else {
            sample.distance = 0.0;
            sample.taper = Self::taper(brush, 0.0);
            self.until_dab = Self::spacing(brush, &sample);
            sample.spacing = Some(self.until_dab);
            self.last_dab = Some(sample);
            return if self.reserve_dab() {
                vec![sample]
            } else {
                vec![]
            };
        };
        let dx = sample.x - previous.x;
        let dy = sample.y - previous.y;
        let length = dx.hypot(dy);
        if length < f32::EPSILON {
            return vec![];
        }
        let mut at = self.until_dab;
        let mut dabs = vec![];
        while at <= length {
            if !self.reserve_dab() {
                break;
            }
            let t = at / length;
            let mut dab = Sample {
                x: previous.x + dx * t,
                y: previous.y + dy * t,
                pressure: previous.pressure + (sample.pressure - previous.pressure) * t,
                time: previous
                    .time
                    .zip(sample.time)
                    .map(|(a, b)| a + (b - a) * t as f64),
                speed: previous
                    .speed
                    .zip(sample.speed)
                    .map(|(a, b)| a + (b - a) * t)
                    .or(sample.speed),
                random: Some(Self::random_value(
                    self.dab_index.wrapping_add(dabs.len() as u32),
                    brush.scatter_seed,
                )),
                taper: Self::taper(brush, self.length + at as f64),
                distance: self.length + at as f64,
                ..sample
            };
            dab.spacing = Some(Self::spacing(brush, &dab));
            dabs.push(dab);
            self.last_dab = Some(dab);
            at += Self::spacing(brush, &dab);
        }
        self.until_dab = at - length;
        self.length += length as f64;
        dabs
    }
    fn taper(brush: &Brush, length: f64) -> Option<f32> {
        (brush.taper_in > 0.0)
            .then(|| 0.05 + 0.95 * (length / brush.taper_in as f64).min(1.0) as f32)
    }
}

/// Update viewport translation so the canvas point beneath the cursor stays put.
pub fn zoom_pan(pan: [f32; 2], anchor: [f32; 2], ratio: f32) -> [f32; 2] {
    [
        anchor[0] - (anchor[0] - pan[0]) * ratio,
        anchor[1] - (anchor[1] - pan[1]) * ratio,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_rope_distance_is_independent_of_smoothing_and_persists_with_presets() {
        let mut brush = rope_brush();
        assert_eq!(rope_radius(&brush, 2.0), 10.0);
        brush.rope_distance = Some(120.0);
        for amount in [0.0, 10.0, 50.0] {
            brush.stabilization = amount;
            assert_eq!(rope_radius(&brush, 2.0), 60.0);
        }
        let restored: Brush =
            serde_json::from_str(&serde_json::to_string(&brush).unwrap()).unwrap();
        assert_eq!(restored.rope_distance, Some(120.0));
        assert!(restored.validate().is_ok());
        brush.rope_distance = Some(f32::NAN);
        assert!(brush.validate().is_err());
        brush.rope_distance = Some(501.0);
        assert!(brush.validate().is_err());
        brush.stabilizer_mode = crate::brush::StabilizerMode::Smooth;
        assert_eq!(rope_radius(&brush, 2.0), 0.0);
    }
    fn rope_brush() -> Brush {
        Brush {
            stabilization: 10.0,
            stabilizer_mode: crate::brush::StabilizerMode::Rope,
            spacing: 0.05,
            ..Brush::default()
        }
    }
    fn at(x: f32, y: f32, time: f64) -> Sample {
        Sample {
            x,
            y,
            pressure: 1.0,
            time: Some(time),
            ..Default::default()
        }
    }
    #[test]
    fn zero_stabilization_follows_the_pen_exactly_in_both_modes() {
        let path: Vec<Sample> = (0..60)
            .map(|i| {
                let t = i as f32 * 0.1;
                at(
                    100.0 + 40.0 * t.cos(),
                    80.0 + 25.0 * (2.0 * t).sin(),
                    i as f64 / 240.0,
                )
            })
            .collect();
        let draw = |mode, rope_distance| {
            let brush = Brush {
                stabilization: 0.0,
                stabilizer_mode: mode,
                rope_distance,
                spacing: 0.05,
                ..Brush::default()
            };
            let mut stroke = Stroke::default();
            let dabs: Vec<[f32; 2]> = path
                .iter()
                .flat_map(|&sample| stroke.sample_at_zoom(sample, &brush, 1.0))
                .map(|dab| [dab.x, dab.y])
                .collect();
            (dabs, stroke.stabilized.unwrap())
        };
        let (smooth, smooth_end) = draw(crate::brush::StabilizerMode::Smooth, None);
        let (rope, rope_end) = draw(crate::brush::StabilizerMode::Rope, Some(0.0));
        assert_eq!(smooth, rope, "Smooth at 0 must match an untensioned rope");
        let pen = path.last().unwrap();
        for end in [smooth_end, rope_end] {
            assert_eq!([end.x, end.y], [pen.x, pen.y], "the line reaches the pen");
        }
    }
    #[test]
    fn rope_holds_still_within_reach_then_trails_the_pen_by_its_length() {
        let brush = rope_brush();
        let mut stroke = Stroke::default();
        // 10 units at zoom 1: a 20-pixel rope.
        stroke.sample_at_zoom(at(0.0, 0.0, 0.0), &brush, 1.0);
        let wobble = stroke.sample_at_zoom(at(12.0, 5.0, 0.01), &brush, 1.0);
        assert!(
            wobble.is_empty(),
            "a small move inside the rope draws nothing"
        );
        assert_eq!(stroke.stabilized.unwrap().x, 0.0);
        let mut t = 0.02;
        for x in (20..=200).step_by(4) {
            stroke.sample_at_zoom(at(x as f32, 0.0, t), &brush, 1.0);
            t += 0.01;
        }
        // Resting pen: catch-up tightens the rope but never reaches the pen.
        for _ in 0..200 {
            stroke.catch_up(smoothing_fraction(0.01, &brush), &brush);
        }
        let held = stroke.stabilized.unwrap();
        assert!(
            (200.0 - held.x - 20.0).abs() < 0.5,
            "trails by the rope: {}",
            held.x
        );
        assert!(held.y.abs() < 1e-3);
        assert!(!stroke.lagging(0.5, &brush));
        // Zooming in shortens the rope in document pixels.
        assert!((rope_radius(&brush, 4.0) - 5.0).abs() < 1e-6);
        // Lifting ends the line where the rope held it, with no final jump.
        let tail = stroke.finish_at_zoom(at(200.0, 0.0, t), &brush, 1.0);
        assert!(
            tail.iter().all(|d| d.x <= held.x + 0.01),
            "{:?}",
            tail.last().map(|d| d.x)
        );
    }
    #[test]
    fn rope_smooths_a_jittery_line_more_than_it_follows_the_jitter() {
        let brush = rope_brush();
        let mut stroke = Stroke::default();
        let mut dabs = Vec::new();
        for i in 0..=300 {
            let jitter = if i % 2 == 0 { 3.0 } else { -3.0 };
            dabs.extend(stroke.sample_at_zoom(at(i as f32, jitter, i as f64 * 0.008), &brush, 1.0));
        }
        // The line keeps the offset it started at; what matters is that it
        // stops zigzagging with the hand.
        let settled: Vec<f32> = dabs.iter().filter(|d| d.x > 60.0).map(|d| d.y).collect();
        let low = settled.iter().copied().fold(f32::MAX, f32::min);
        let high = settled.iter().copied().fold(f32::MIN, f32::max);
        assert!(settled.len() > 50);
        assert!(
            high - low < 1.0,
            "±3 px jitter leaves a {} px wobble",
            high - low
        );
    }
    fn jittering_tail(brush: &Brush, zoom: f32) -> Stroke {
        let mut stroke = Stroke::default();
        for (x, y) in [
            (0.0, 0.0),
            (30.0, 0.0),
            (60.0, 0.0),
            (90.0, 0.0),
            (100.0, 8.0),
            (98.0, -6.0),
            (104.0, 4.0),
            (106.0, 0.0),
        ] {
            stroke.sample_at_zoom(
                Sample {
                    x: x / zoom,
                    y: y / zoom,
                    pressure: 1.0,
                    pressure_known: true,
                    ..Default::default()
                },
                brush,
                zoom,
            );
        }
        stroke
    }
    #[test]
    fn smoothed_lift_does_not_reintroduce_raw_zigzags_and_finishes_in_a_fine_tip() {
        let brush = Brush {
            size: 6.0,
            spacing: 0.1,
            stabilization: 20.0,
            ..Default::default()
        };
        let mut stroke = jittering_tail(&brush, 1.0);
        let start = stroke.stabilized.unwrap();
        let incoming = stroke.tangent.unwrap();
        let tail = stroke.finish(
            Sample {
                x: 106.0,
                y: 0.0,
                pressure: 0.0,
                pressure_known: true,
                ..Default::default()
            },
            &brush,
        );
        assert!(!tail.is_empty());
        assert_eq!(
            (tail.last().unwrap().x, tail.last().unwrap().y),
            (106.0, 0.0)
        );
        assert!(tail.last().unwrap().pressure < 0.001);
        let first = tail[0];
        let dx = first.x - start.x;
        let dy = first.y - start.y;
        assert!(
            (dx * incoming[0] + dy * incoming[1]) / dx.hypot(dy) > 0.99,
            "tail should leave along the already drawn tangent"
        );
        for dab in &tail {
            assert!(dab.x >= start.x - 0.001 && dab.x <= 106.001, "{dab:?}");
            assert!(
                dab.y >= -0.001 && dab.y <= start.y + 0.001,
                "raw jitter leaked into lift: {dab:?}"
            );
            assert!(dab.time.is_none(), "do not invent device timestamps");
        }
        assert!(
            tail.windows(2)
                .all(|w| w[1].pressure <= w[0].pressure + 0.0001)
        );
        for triple in tail.windows(3) {
            let a = [triple[1].x - triple[0].x, triple[1].y - triple[0].y];
            let b = [triple[2].x - triple[1].x, triple[2].y - triple[1].y];
            if a[0].hypot(a[1]) > 0.001 && b[0].hypot(b[1]) > 0.001 {
                assert!((a[0] * b[0] + a[1] * b[1]) / (a[0].hypot(a[1]) * b[0].hypot(b[1])) > 0.98);
            }
        }
    }
    #[test]
    fn smoothed_zero_pressure_lift_ignores_hover_jump_and_duplicate_up_without_adding_to_taps() {
        let brush = Brush {
            stabilization: 20.0,
            ..Default::default()
        };
        let mut stroke = jittering_tail(&brush, 1.0);
        let lift = Sample {
            x: 1000.0,
            y: 900.0,
            pressure: 0.0,
            pressure_known: true,
            ..Default::default()
        };
        let tail = stroke.finish(lift, &brush);
        assert_eq!(
            (tail.last().unwrap().x, tail.last().unwrap().y),
            (106.0, 0.0)
        );
        assert!(stroke.finish(lift, &brush).is_empty());
        stroke.reset();
        let tap = Sample {
            x: 30.0,
            y: 40.0,
            pressure: 1.0,
            pressure_known: true,
            ..Default::default()
        };
        assert_eq!(stroke.sample(tap, &brush).len(), 1);
        assert!(
            stroke
                .finish(
                    Sample {
                        pressure: 0.0,
                        ..tap
                    },
                    &brush
                )
                .is_empty()
        );
    }
    #[test]
    fn smoothed_lift_screen_geometry_and_pressure_are_zoom_invariant_and_pointer_pressure_is_respected()
     {
        let mut expected: Option<Vec<Sample>> = None;
        for zoom in [0.5, 1.0, 2.0] {
            let brush = Brush {
                size: 24.0 / zoom,
                pressure_size: false,
                stabilization: 20.0,
                stabilization_space: crate::brush::StabilizationSpace::Screen,
                ..Default::default()
            };
            let mut stroke = jittering_tail(&brush, zoom);
            let tail = stroke.finish_at_zoom(
                Sample {
                    x: 106.0 / zoom,
                    y: 0.0,
                    pressure: 0.0,
                    pressure_known: true,
                    ..Default::default()
                },
                &brush,
                zoom,
            );
            let tail: Vec<_> = tail
                .into_iter()
                .map(|s| Sample {
                    x: s.x * zoom,
                    y: s.y * zoom,
                    ..s
                })
                .collect();
            if let Some(expected) = &expected {
                assert_eq!(tail.len(), expected.len());
                for (a, b) in tail.iter().zip(expected) {
                    assert!((a.x - b.x).abs() < 0.001 && (a.y - b.y).abs() < 0.001);
                    assert!((a.pressure - b.pressure).abs() < 0.001);
                }
            } else {
                expected = Some(tail);
            }
        }
        let brush = Brush {
            stabilization: 20.0,
            ..Default::default()
        };
        let mut stroke = jittering_tail(&brush, 1.0);
        let tail = stroke.finish(
            Sample {
                x: 106.0,
                y: 0.0,
                pressure: 1.0,
                ..Default::default()
            },
            &brush,
        );
        assert!(
            tail.iter().all(|s| s.pressure == 1.0),
            "nonzero pointer-up must not force a pressure taper"
        );
    }
    #[test]
    fn smoothed_lift_never_backtracks_or_overshoots_its_pending_chord_on_sharp_turns() {
        let brush = Brush {
            size: 8.0,
            spacing: 0.1,
            stabilization: 30.0,
            ..Default::default()
        };
        let mut seed = 0x12345678u32;
        for _ in 0..80 {
            let mut stroke = Stroke::default();
            let mut end = Sample {
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            for _ in 0..24 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                end.x += (seed & 31) as f32 - 15.0;
                end.y += ((seed >> 8) & 31) as f32 - 15.0;
                stroke.sample(end, &brush);
            }
            let start = stroke.stabilized.unwrap();
            let (dx, dy) = (end.x - start.x, end.y - start.y);
            let squared = dx * dx + dy * dy;
            let tail = stroke.finish(
                Sample {
                    pressure: 0.0,
                    ..end
                },
                &brush,
            );
            if squared > 0.001 {
                let mut previous = 0.0;
                for dab in tail {
                    let progress = ((dab.x - start.x) * dx + (dab.y - start.y) * dy) / squared;
                    assert!(
                        progress >= previous - 0.001 && progress <= 1.001,
                        "{progress}/{previous}"
                    );
                    previous = progress;
                }
            }
        }
    }
    #[test]
    fn start_taper_is_independent_of_packet_subdivision_and_resets_per_stroke() {
        let brush = Brush {
            size: 20.0,
            pressure_size: false,
            taper_in: 20.0,
            ..Default::default()
        };
        let render = |step| {
            let mut doc = crate::document::Document::new(120, 40);
            let mut stroke = Stroke::default();
            let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
            for x in (10..=110).step_by(step) {
                for dab in stroke.sample(
                    Sample {
                        x: x as f32,
                        y: 20.0,
                        pressure: 1.0,
                        ..Default::default()
                    },
                    &brush,
                ) {
                    doc.dab_sample_buffered(
                        dab,
                        &brush,
                        [0, 0, 0],
                        false,
                        crate::symmetry::IDENTITY,
                        Some(&mut buffer),
                    );
                }
            }
            doc.layers[0].pixels.region(120, [0, 0, 120, 40])
        };
        let a = render(1);
        let b = render(5);
        assert!(a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1));
        let mut stroke = Stroke::default();
        let first = stroke.sample(
            Sample {
                pressure: 1.0,
                ..Default::default()
            },
            &brush,
        )[0];
        assert_eq!(first.taper, Some(0.05));
        stroke.sample(
            Sample {
                x: 100.0,
                pressure: 1.0,
                ..Default::default()
            },
            &brush,
        );
        stroke.reset();
        assert_eq!(
            stroke.sample(
                Sample {
                    pressure: 1.0,
                    ..Default::default()
                },
                &brush
            )[0]
            .taper,
            first.taper
        );
    }
    #[test]
    fn smoothing_is_zoom_invariant_and_follows_pen_timing() {
        let mut expected: Option<Vec<f32>> = None;
        for zoom in [0.5, 1.0, 2.0] {
            for space in [
                crate::brush::StabilizationSpace::Document,
                crate::brush::StabilizationSpace::Screen,
            ] {
                let brush = Brush {
                    stabilization: 10.0,
                    stabilization_space: space,
                    ..Default::default()
                };
                let mut stroke = Stroke::default();
                let path: Vec<f32> = [0.0, 3.0, 8.0, 25.0, 55.0, 61.0]
                    .into_iter()
                    .map(|x| {
                        stroke.sample_at_zoom(
                            Sample {
                                x: x / zoom,
                                pressure: 1.0,
                                ..Default::default()
                            },
                            &brush,
                            zoom,
                        );
                        stroke.stabilized.unwrap().x * zoom
                    })
                    .collect();
                if let Some(expected) = &expected {
                    for (a, b) in path.iter().zip(expected) {
                        assert!((a - b).abs() < 1e-3, "{path:?} vs {expected:?}");
                    }
                } else {
                    expected = Some(path);
                }
            }
        }
        // The same path drawn more slowly gives the line more time to follow.
        let brush = Brush {
            stabilization: 10.0,
            ..Default::default()
        };
        let follow = |interval: f64| {
            let mut stroke = Stroke::default();
            for (index, x) in [0.0, 20.0, 40.0].into_iter().enumerate() {
                stroke.sample(
                    Sample {
                        x,
                        pressure: 1.0,
                        time: Some(index as f64 * interval),
                        ..Default::default()
                    },
                    &brush,
                );
            }
            stroke.stabilized.unwrap().x
        };
        let (fast, slow) = (follow(1.0 / 240.0), follow(1.0 / 30.0));
        assert!(fast < slow && slow < 40.0, "{fast} {slow}");
    }

    #[test]
    fn native_timestamps_interpolate_without_inventing_fallback_timing() {
        let brush = Brush {
            size: 10.0,
            spacing: 0.5,
            pressure_size: false,
            ..Default::default()
        };
        let mut stroke = Stroke::default();
        stroke.sample(
            Sample {
                pressure: 0.2,
                time: Some(10.0),
                ..Default::default()
            },
            &brush,
        );
        let dabs = stroke.sample(
            Sample {
                x: 10.0,
                pressure: 0.8,
                time: Some(10.1),
                ..Default::default()
            },
            &brush,
        );
        assert_eq!(dabs.len(), 2);
        assert!((dabs[0].time.unwrap() - 10.05).abs() < 1e-6);
        assert!((dabs[0].pressure - 0.5).abs() < 1e-6);
        let dabs = stroke.sample(
            Sample {
                x: 20.0,
                pressure: 1.0,
                ..Default::default()
            },
            &brush,
        );
        assert!(dabs.iter().all(|s| s.time.is_none()));
    }
    #[test]
    fn tip_rotation_follows_direction_tilt_and_jitter() {
        let draw = |brush: &Brush, tilt: Option<[f32; 2]>| {
            let mut stroke = Stroke::default();
            let mut dabs = Vec::new();
            for [x, y] in [[0.0, 0.0], [0.0, 40.0], [0.0, 80.0]] {
                dabs.extend(stroke.sample(
                    Sample {
                        x,
                        y,
                        pressure: 1.0,
                        tilt,
                        ..Default::default()
                    },
                    brush,
                ));
            }
            dabs
        };
        let fixed = Brush {
            pressure_size: false,
            ..Brush::default()
        };
        assert!(draw(&fixed, None).iter().all(|d| d.rotation.is_none()));
        // Drawing straight down turns the tip a quarter turn.
        let direction = Brush {
            angle_source: crate::brush::AngleSource::Direction,
            ..fixed.clone()
        };
        let dabs = draw(&direction, None);
        assert!(dabs.len() > 3);
        assert!(
            dabs[1..]
                .iter()
                .all(|d| (d.rotation.unwrap() - 90.0).abs() < 0.01)
        );
        // Pen tilt wins over the direction when the tablet reports it.
        let tilt = Brush {
            angle_source: crate::brush::AngleSource::Tilt,
            ..fixed.clone()
        };
        let dabs = draw(&tilt, Some([-30.0, 0.0]));
        assert!(
            dabs.iter()
                .all(|d| (d.rotation.unwrap() - 180.0).abs() < 0.01)
        );
        // Jitter stays within its range and does not move the dabs.
        let jitter = Brush {
            angle_jitter: 25.0,
            ..fixed.clone()
        };
        let jittered = draw(&jitter, None);
        let plain = draw(&fixed, None);
        assert_eq!(jittered.len(), plain.len());
        assert!(jittered.iter().all(|d| d.rotation.unwrap().abs() <= 25.0));
        assert!(jittered.iter().any(|d| d.rotation.unwrap().abs() > 1.0));
        for (a, b) in jittered.iter().zip(&plain) {
            assert!((a.x - b.x).abs() < 0.001 && (a.y - b.y).abs() < 0.001);
        }
    }
    #[test]
    fn scatter_is_replayable_bounded_and_independent_of_packet_boundaries() {
        let brush = Brush {
            size: 20.0,
            pressure_size: false,
            scatter: 0.8,
            scatter_seed: 42,
            ..Brush::default()
        };
        let render = |points: &[f32]| {
            let mut stroke = Stroke::default();
            let mut output = Vec::new();
            for x in points {
                output.extend(stroke.sample(
                    Sample {
                        x: *x,
                        y: 0.0,
                        pressure: 1.0,
                        ..Default::default()
                    },
                    &brush,
                ));
            }
            output.extend(stroke.finish(
                Sample {
                    x: 60.0,
                    y: 0.0,
                    pressure: 1.0,
                    ..Default::default()
                },
                &brush,
            ));
            output
        };
        let whole = render(&[0.0, 60.0]);
        let split = render(&[0.0, 15.0, 30.0, 45.0, 60.0]);
        assert_eq!(whole.len(), split.len());
        for (a, b) in whole.iter().zip(&split) {
            assert!((a.x - b.x).abs() < 0.001 && (a.y - b.y).abs() < 0.001);
            assert!(a.y.abs() <= 16.0);
        }
        let mut stroke = Stroke::default();
        let sample = Sample {
            x: 10.0,
            y: 10.0,
            pressure: 1.0,
            ..Default::default()
        };
        let first = stroke.sample(sample, &brush)[0];
        stroke.reset();
        let again = stroke.sample(sample, &brush)[0];
        assert_eq!((first.x, first.y), (again.x, again.y));
        assert!((first.x - 10.0).hypot(first.y - 10.0) <= 16.0);
    }
    #[test]
    fn stabilization_rejects_small_jitter_and_finishes_at_pointer() {
        let brush = Brush {
            stabilization: 10.0,
            pressure_size: false,
            ..Brush::default()
        };
        let mut stroke = Stroke::default();
        stroke.sample(
            Sample {
                x: 0.0,
                y: 0.0,
                pressure: 1.0,
                ..Default::default()
            },
            &brush,
        );
        assert!(
            stroke
                .sample(
                    Sample {
                        x: 2.0,
                        y: 3.0,
                        pressure: 1.0,
                        ..Default::default()
                    },
                    &brush
                )
                .is_empty()
        );
        let end = Sample {
            x: 50.0,
            y: 0.0,
            pressure: 1.0,
            ..Default::default()
        };
        let dabs = stroke.sample(end, &brush);
        assert!(dabs.last().unwrap().x <= 40.0);
        let tail = stroke.finish(end, &brush);
        assert_eq!(tail.last().unwrap().x, 50.0);
        assert!(stroke.finish(end, &brush).is_empty());
    }
    #[test]
    fn stabilized_lift_preserves_delayed_pressure_then_eases_to_release() {
        let brush = Brush {
            stabilization: 20.0,
            ..Brush::default()
        };
        let at = |x: f32, pressure: f32| Sample {
            x,
            pressure,
            ..Default::default()
        };
        let mut stroke = Stroke::default();
        let mut body = vec![];
        for x in (0..=100).step_by(2) {
            body.extend(stroke.sample(at(x as f32, 1.0), &brush));
        }
        body.extend(stroke.sample(at(102.0, 0.8), &brush));
        body.extend(stroke.sample(at(104.0, 0.6), &brush));
        // The stabilized point lags behind, so it still draws full pressure.
        assert!(body.iter().all(|dab| dab.pressure > 0.99));
        let tail = stroke.finish(at(104.0, 0.0), &brush);
        assert_eq!(tail.last().unwrap().x, 104.0);
        assert!(tail.last().unwrap().pressure < 0.001);
        for dab in &tail {
            if dab.x <= 94.0 {
                assert!(dab.pressure > 0.99, "{dab:?}");
            }
            assert!((0.0..=1.0).contains(&dab.pressure), "{dab:?}");
        }
    }
    #[test]
    fn resting_pen_lets_the_stabilized_line_catch_up() {
        let brush = Brush {
            stabilization: 20.0,
            pressure_size: false,
            ..Brush::default()
        };
        let mut stroke = Stroke::default();
        for x in 0..=10 {
            stroke.sample(
                Sample {
                    x: x as f32 * 10.0,
                    pressure: 0.8,
                    ..Default::default()
                },
                &brush,
            );
        }
        // The smoothed line trails the moving pen.
        let behind = stroke.stabilized.unwrap().x;
        assert!(behind > 20.0 && behind < 99.0, "{behind}");
        assert!(stroke.lagging(0.25, &brush));
        let half = stroke.catch_up(0.5, &brush);
        let midway = behind + (100.0 - behind) * 0.5;
        assert!((stroke.stabilized.unwrap().x - midway).abs() < 0.01);
        assert!(
            half.iter()
                .all(|dab| dab.x > behind && dab.x <= midway + 1e-3)
        );
        let rest = stroke.catch_up(1.0, &brush);
        assert!((rest.last().unwrap().x - 100.0).abs() < 1.0);
        assert!(!stroke.lagging(0.25, &brush));
        assert!(stroke.catch_up(1.0, &brush).is_empty());
        // Nothing to catch up once lifted.
        stroke.finish(
            Sample {
                x: 100.0,
                pressure: 0.8,
                ..Default::default()
            },
            &brush,
        );
        assert!(stroke.catch_up(1.0, &brush).is_empty());
    }
    #[test]
    fn resting_catch_up_is_frame_rate_independent_and_bounded() {
        let brush = Brush {
            stabilization: 25.0,
            ..Brush::default()
        };
        assert_eq!(smoothing_fraction(0.0, &brush), 0.0);
        // Two 8 ms frames cover exactly what one 16 ms frame does.
        let short = smoothing_fraction(0.008, &brush);
        let long = smoothing_fraction(0.016, &brush);
        assert!((1.0 - (1.0 - short) * (1.0 - short) - long).abs() < 1e-5);
        assert!(short < long && long < 1.0);
        // A stalled frame cannot jump the whole way to the pen.
        assert!(smoothing_fraction(5.0, &brush) < 1.0);
        assert_eq!(
            smoothing_fraction(5.0, &brush),
            smoothing_fraction(0.1, &brush)
        );
        let off = Brush {
            stabilization: 0.0,
            ..Brush::default()
        };
        assert_eq!(smoothing_fraction(0.001, &off), 1.0);
    }
    #[test]
    fn fast_turns_with_heavy_stabilization_curve_instead_of_cornering() {
        let brush = Brush {
            stabilization: 30.0,
            spacing: 0.05,
            pressure_size: false,
            ..Brush::default()
        };
        let mut stroke = Stroke::default();
        let mut dabs = vec![];
        // A fast right-angle turn: three far-apart samples.
        for (x, y) in [(0.0, 0.0), (200.0, 0.0), (200.0, 200.0)] {
            dabs.extend(stroke.sample(
                Sample {
                    x,
                    y,
                    pressure: 1.0,
                    ..Default::default()
                },
                &brush,
            ));
        }
        let headings: Vec<f32> = dabs
            .windows(2)
            .filter_map(|pair| {
                let (dx, dy) = (pair[1].x - pair[0].x, pair[1].y - pair[0].y);
                (dx.hypot(dy) > 0.01).then(|| dy.atan2(dx))
            })
            .collect();
        let sharpest = headings
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs().to_degrees())
            .fold(0.0, f32::max);
        assert!(sharpest < 30.0, "turn of {sharpest} degrees between dabs");
    }
    #[test]
    fn packet_boundaries_do_not_change_straight_strokes() {
        let brush = Brush {
            size: 20.0,
            spacing: 0.15,
            ..Brush::default()
        };
        let render = |points: &[f32]| {
            let mut stroke = Stroke::default();
            points
                .iter()
                .flat_map(|x| {
                    stroke.sample(
                        Sample {
                            x: *x,
                            y: 0.0,
                            pressure: 0.2 + *x * 0.008,
                            ..Default::default()
                        },
                        &brush,
                    )
                })
                .collect::<Vec<_>>()
        };
        let whole = render(&[0.0, 100.0]);
        let packets = render(&[0.0, 1.0, 2.0, 13.0, 37.0, 65.0, 100.0]);
        assert_eq!(whole.len(), packets.len());
        for (a, b) in whole.iter().zip(packets) {
            assert!((a.x - b.x).abs() < 0.001);
            assert!((a.pressure - b.pressure).abs() < 0.001);
        }
    }
    #[test]
    fn stationary_packets_do_not_overpaint_and_reset_starts_a_new_stroke() {
        let mut stroke = Stroke::default();
        let sample = Sample {
            x: 10.0,
            y: 20.0,
            pressure: 0.5,
            ..Default::default()
        };
        let b = Brush::default();
        assert_eq!(stroke.sample(sample, &b).len(), 1);
        assert!(stroke.sample(sample, &b).is_empty());
        stroke.reset();
        assert_eq!(stroke.sample(sample, &b).len(), 1);
    }
    #[test]
    fn zoom_keeps_cursor_canvas_coordinate() {
        let pan = [10.0, -25.0];
        let anchor = [200.0, 150.0];
        let next = zoom_pan(pan, anchor, 1.8);
        for i in 0..2 {
            assert!(((anchor[i] - next[i]) / 1.8 - (anchor[i] - pan[i])).abs() < 0.001);
        }
    }
}
