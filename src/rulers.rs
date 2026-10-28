//! Special rulers that strokes snap to, after Clip Studio's: a straight line,
//! parallel lines, focus (radial) lines and concentric ellipses. Guide lines
//! and, optionally, grid lines act as straight rulers too. A stroke picks its
//! track at pen-down; when several fit, its first few pixels of movement choose
//! the best-aligned one, and every later point is projected onto that track.
use crate::{egui, view::View};
use egui::{Color32, Pos2, Stroke, pos2};

pub const MAX_RULERS: usize = 64;
/// Screen distance within which a straight ruler, guide or grid line catches
/// a stroke starting near it.
const CATCH: f32 = 12.0;
/// Screen distance the pen travels before an ambiguous stroke picks a track.
const DECIDE: f32 = 6.0;
const HANDLE: f32 = 10.0;
const COLOR: Color32 = Color32::from_rgb(178, 120, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Line,
    Parallel,
    Focus,
    Ellipse,
    TwoPoint,
    ThreePoint,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Line,
        Kind::Parallel,
        Kind::Focus,
        Kind::Ellipse,
        Kind::TwoPoint,
        Kind::ThreePoint,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Line => "Line",
            Kind::Parallel => "Parallel",
            Kind::Focus => "Focus",
            Kind::Ellipse => "Ellipse",
            Kind::TwoPoint => "2-point",
            Kind::ThreePoint => "3-point",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Kind::Line => "Drag to place a straight ruler; strokes starting near it follow it",
            Kind::Parallel => "Drag to set a direction; every stroke runs parallel to it",
            Kind::Focus => "Click a focus point; every stroke radiates from it",
            Kind::Ellipse => {
                "Drag from the center; every stroke follows a matching concentric ellipse"
            }
            Kind::TwoPoint => {
                "Drag from one vanishing point to the other along the horizon; strokes head to a vanishing point or stay upright"
            }
            Kind::ThreePoint => {
                "Drag between the two horizon vanishing points; drag the third to set where verticals converge"
            }
        }
    }
}

/// A ruler in document coordinates. Points may lie outside the canvas, as
/// focus points often do.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Ruler {
    Line {
        a: [f32; 2],
        b: [f32; 2],
    },
    Parallel {
        a: [f32; 2],
        b: [f32; 2],
    },
    Focus {
        center: [f32; 2],
    },
    /// Radii along the rotated x and y axes; `angle` in degrees, clockwise.
    Ellipse {
        center: [f32; 2],
        radii: [f32; 2],
        angle: f32,
    },
    /// Vanishing points: two on the horizon, and a third for verticals when
    /// `three` is set. Two-point verticals stay perpendicular to the horizon.
    Perspective {
        vanishing: [[f32; 2]; 3],
        three: bool,
    },
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn length(a: [f32; 2]) -> f32 {
    a[0].hypot(a[1])
}
fn unit(a: [f32; 2]) -> Option<[f32; 2]> {
    let l = length(a);
    (l > 1e-4).then(|| [a[0] / l, a[1] / l])
}

impl Ruler {
    pub fn kind(&self) -> Kind {
        match self {
            Ruler::Line { .. } => Kind::Line,
            Ruler::Parallel { .. } => Kind::Parallel,
            Ruler::Focus { .. } => Kind::Focus,
            Ruler::Ellipse { .. } => Kind::Ellipse,
            Ruler::Perspective { three: false, .. } => Kind::TwoPoint,
            Ruler::Perspective { three: true, .. } => Kind::ThreePoint,
        }
    }
    /// A new ruler whose every handle starts at `at`, ready to be dragged out.
    pub fn new(kind: Kind, at: [f32; 2]) -> Self {
        match kind {
            Kind::Line => Ruler::Line { a: at, b: at },
            Kind::Parallel => Ruler::Parallel { a: at, b: at },
            Kind::Focus => Ruler::Focus { center: at },
            Kind::Ellipse => Ruler::Ellipse {
                center: at,
                radii: [0.0, 0.0],
                angle: 0.0,
            },
            Kind::TwoPoint | Kind::ThreePoint => Ruler::Perspective {
                vanishing: [at; 3],
                three: kind == Kind::ThreePoint,
            },
        }
    }
    fn ellipse_axes(angle: f32) -> ([f32; 2], [f32; 2]) {
        let (sin, cos) = angle.to_radians().sin_cos();
        ([cos, sin], [-sin, cos])
    }
    /// Draggable handle positions.
    pub fn handles(&self) -> Vec<[f32; 2]> {
        match *self {
            Ruler::Line { a, b } | Ruler::Parallel { a, b } => vec![a, b],
            Ruler::Focus { center } => vec![center],
            Ruler::Ellipse {
                center,
                radii,
                angle,
            } => {
                let (u, v) = Self::ellipse_axes(angle);
                vec![
                    center,
                    [center[0] + u[0] * radii[0], center[1] + u[1] * radii[0]],
                    [center[0] + v[0] * radii[1], center[1] + v[1] * radii[1]],
                ]
            }
            Ruler::Perspective { vanishing, three } => {
                vanishing[..if three { 3 } else { 2 }].to_vec()
            }
        }
    }
    /// Move handle `index` to `at`. Ellipse: 0 moves it, 1 sets the x radius
    /// and rotation, 2 sets the y radius.
    pub fn set_handle(&mut self, index: usize, at: [f32; 2]) {
        match self {
            Ruler::Line { a, b } | Ruler::Parallel { a, b } => {
                if index == 0 {
                    *a = at;
                } else {
                    *b = at;
                }
            }
            Ruler::Focus { center } => *center = at,
            Ruler::Ellipse {
                center,
                radii,
                angle,
            } => match index {
                0 => *center = at,
                1 => {
                    let d = sub(at, *center);
                    radii[0] = length(d);
                    if radii[0] > 1e-3 {
                        *angle = d[1].atan2(d[0]).to_degrees();
                    }
                }
                _ => {
                    let (_, v) = Self::ellipse_axes(*angle);
                    radii[1] = dot(sub(at, *center), v).abs();
                }
            },
            Ruler::Perspective { vanishing, .. } => vanishing[index.min(2)] = at,
        }
    }
    /// Shape the ruler while it is first dragged out from `from` to `to`.
    pub fn drag_out(&mut self, from: [f32; 2], to: [f32; 2]) {
        match self {
            Ruler::Line { b, .. } | Ruler::Parallel { b, .. } => *b = to,
            Ruler::Focus { center } => *center = to,
            Ruler::Ellipse {
                center,
                radii,
                angle,
            } => {
                *center = from;
                *radii = [(to[0] - from[0]).abs(), (to[1] - from[1]).abs()];
                *angle = 0.0;
            }
            Ruler::Perspective { vanishing, three } => {
                vanishing[0] = from;
                vanishing[1] = to;
                // Start the third point below the horizon's middle, as far
                // from it as the horizon points are apart.
                let horizon = sub(to, from);
                let mut down = [-horizon[1], horizon[0]];
                if down[1] < 0.0 {
                    down = [-down[0], -down[1]];
                }
                let middle = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
                vanishing[2] = if *three {
                    [middle[0] + down[0], middle[1] + down[1]]
                } else {
                    middle
                };
            }
        }
    }
    /// Whether the ruler is big enough to snap to.
    pub fn usable(&self) -> bool {
        match *self {
            Ruler::Line { a, b } | Ruler::Parallel { a, b } => length(sub(b, a)) >= 2.0,
            Ruler::Focus { .. } => true,
            Ruler::Ellipse { radii, .. } => radii[0] >= 1.0 && radii[1] >= 1.0,
            Ruler::Perspective { vanishing, three } => {
                let middle = [
                    (vanishing[0][0] + vanishing[1][0]) / 2.0,
                    (vanishing[0][1] + vanishing[1][1]) / 2.0,
                ];
                length(sub(vanishing[1], vanishing[0])) >= 2.0
                    && (!three || length(sub(vanishing[2], middle)) >= 2.0)
            }
        }
    }
    /// Follow the canvas through a quarter turn clockwise; `height` is the
    /// canvas height before turning.
    #[cfg(test)]
    pub fn rotate_clockwise(&mut self, height: usize) {
        let before = crate::canvas_size::Frame::default();
        self.map_canvas(
            before,
            before
                .rotated(height)
                .expect("Bounded initial canvas frame"),
        );
    }
    pub(crate) fn map_canvas(
        &mut self,
        before: crate::canvas_size::Frame,
        after: crate::canvas_size::Frame,
    ) {
        let turn = |p: &mut [f32; 2]| *p = after.point_from(before, *p);
        match self {
            Ruler::Line { a, b } | Ruler::Parallel { a, b } => {
                turn(a);
                turn(b);
            }
            Ruler::Focus { center } => turn(center),
            Ruler::Ellipse { center, angle, .. } => {
                turn(center);
                *angle += after.angle_from(before);
            }
            Ruler::Perspective { vanishing, .. } => vanishing.iter_mut().for_each(turn),
        }
    }
    pub fn is_finite(&self) -> bool {
        self.handles().iter().flatten().all(|v| v.is_finite())
            && match self {
                Ruler::Ellipse { angle, .. } => angle.is_finite(),
                _ => true,
            }
    }
}

/// The path one stroke is held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Track {
    Line {
        origin: [f32; 2],
        direction: [f32; 2],
    },
    Ellipse {
        center: [f32; 2],
        radii: [f32; 2],
        angle: f32,
    },
}

impl Track {
    pub fn project(&self, p: [f32; 2]) -> [f32; 2] {
        match *self {
            Track::Line { origin, direction } => {
                let t = dot(sub(p, origin), direction);
                [origin[0] + direction[0] * t, origin[1] + direction[1] * t]
            }
            Track::Ellipse {
                center,
                radii,
                angle,
            } => {
                let (u, v) = Ruler::ellipse_axes(angle);
                let d = sub(p, center);
                let (x, y) = (dot(d, u), dot(d, v));
                // Angle in the circle the ellipse is a stretch of.
                let phi = (y / radii[1].max(1e-3)).atan2(x / radii[0].max(1e-3));
                let (sin, cos) = phi.sin_cos();
                let (lx, ly) = (radii[0] * cos, radii[1] * sin);
                [
                    center[0] + u[0] * lx + v[0] * ly,
                    center[1] + u[1] * lx + v[1] * ly,
                ]
            }
        }
    }
    /// Unit direction of the track at a point on it.
    pub fn tangent(&self, p: [f32; 2]) -> [f32; 2] {
        match *self {
            Track::Line { direction, .. } => direction,
            Track::Ellipse {
                center,
                radii,
                angle,
            } => {
                let (u, v) = Ruler::ellipse_axes(angle);
                let d = sub(p, center);
                let phi = (dot(d, v) / radii[1].max(1e-3)).atan2(dot(d, u) / radii[0].max(1e-3));
                let (sin, cos) = phi.sin_cos();
                let (tx, ty) = (-radii[0] * sin, radii[1] * cos);
                unit([u[0] * tx + v[0] * ty, u[1] * tx + v[1] * ty]).unwrap_or([1.0, 0.0])
            }
        }
    }
}

/// What a stroke may snap to, gathered at pen-down.
pub struct Sources<'a> {
    pub rulers: &'a [Ruler],
    pub guides: &'a [crate::guides::GuideLine],
    /// Grid spacing when grid snapping is on and the grid is shown.
    pub grid: Option<f32>,
    pub zoom: f32,
}

/// Every track a stroke starting at `start` could follow. Straight rulers,
/// guides and grid lines count only when the stroke starts near them; parallel,
/// focus and ellipse rulers shape every stroke.
pub fn candidates(sources: &Sources, start: [f32; 2]) -> Vec<Track> {
    let catch = CATCH / sources.zoom.max(1e-3);
    let mut near: Vec<(f32, Track)> = Vec::new();
    let mut line = |origin: [f32; 2], direction: [f32; 2]| {
        let track = Track::Line { origin, direction };
        let distance = length(sub(track.project(start), start));
        if distance <= catch {
            near.push((distance, track));
        }
    };
    for guide in sources.guides {
        if guide.vertical {
            line([guide.position, 0.0], [0.0, 1.0]);
        } else {
            line([0.0, guide.position], [1.0, 0.0]);
        }
    }
    if let Some(step) = sources.grid.filter(|step| *step > 0.0) {
        line([(start[0] / step).round() * step, 0.0], [0.0, 1.0]);
        line([0.0, (start[1] / step).round() * step], [1.0, 0.0]);
    }
    let mut always = Vec::new();
    for ruler in sources.rulers.iter().filter(|ruler| ruler.usable()) {
        match *ruler {
            Ruler::Line { a, b } => {
                if let Some(direction) = unit(sub(b, a)) {
                    line(a, direction);
                }
            }
            Ruler::Parallel { a, b } => {
                if let Some(direction) = unit(sub(b, a)) {
                    always.push(Track::Line {
                        origin: start,
                        direction,
                    });
                }
            }
            Ruler::Focus { center } => {
                if let Some(direction) = unit(sub(start, center)) {
                    always.push(Track::Line {
                        origin: center,
                        direction,
                    });
                }
            }
            Ruler::Perspective { vanishing, three } => {
                for point in &vanishing[..if three { 3 } else { 2 }] {
                    if let Some(direction) = unit(sub(start, *point)) {
                        always.push(Track::Line {
                            origin: *point,
                            direction,
                        });
                    }
                }
                let horizon = sub(vanishing[1], vanishing[0]);
                if !three && let Some(direction) = unit([-horizon[1], horizon[0]]) {
                    always.push(Track::Line {
                        origin: start,
                        direction,
                    });
                }
            }
            Ruler::Ellipse {
                center,
                radii,
                angle,
            } => {
                let (u, v) = Ruler::ellipse_axes(angle);
                let d = sub(start, center);
                let (x, y) = (dot(d, u), dot(d, v) * radii[0] / radii[1]);
                let r = x.hypot(y);
                if r > 1e-3 {
                    always.push(Track::Ellipse {
                        center,
                        radii: [r, r * radii[1] / radii[0]],
                        angle,
                    });
                }
            }
        }
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.into_iter()
        .map(|(_, track)| track)
        .chain(always)
        .collect()
}

/// Snapping state for one stroke.
pub struct StrokeSnap {
    start: [f32; 2],
    candidates: Vec<Track>,
    chosen: Option<Track>,
    /// Samples held while an ambiguous stroke has not moved far enough to
    /// show which track it follows.
    pending: Vec<crate::stroke::Sample>,
}

impl StrokeSnap {
    /// Snapping for a stroke starting at `start`, or None when nothing applies.
    pub fn begin(sources: &Sources, start: [f32; 2]) -> Option<Self> {
        let candidates = candidates(sources, start);
        let chosen = (candidates.len() == 1).then(|| candidates[0]);
        (!candidates.is_empty()).then_some(Self {
            start,
            candidates,
            chosen,
            pending: Vec::new(),
        })
    }
    pub fn chosen(&self) -> Option<Track> {
        self.chosen
    }
    /// Feed one document-space sample; returns the samples to draw now,
    /// projected onto the chosen track. `end` forces a decision.
    pub fn feed(
        &mut self,
        sample: crate::stroke::Sample,
        zoom: f32,
        end: bool,
    ) -> Vec<crate::stroke::Sample> {
        if self.chosen.is_none() {
            self.pending.push(sample);
            let moved = sub([sample.x, sample.y], self.start);
            if length(moved) * zoom < DECIDE && !end {
                return vec![];
            }
            // The track whose direction at the start best matches the motion;
            // a stroke lifted without moving keeps the nearest one.
            let direction = unit(moved);
            self.chosen = Some(
                direction
                    .and_then(|d| {
                        self.candidates.iter().copied().max_by(|a, b| {
                            let fit = |t: &Track| dot(t.tangent(t.project(self.start)), d).abs();
                            fit(a).total_cmp(&fit(b))
                        })
                    })
                    .unwrap_or(self.candidates[0]),
            );
        } else {
            self.pending.push(sample);
        }
        let track = self.chosen.unwrap();
        self.pending
            .drain(..)
            .map(|sample| {
                let [x, y] = track.project([sample.x, sample.y]);
                crate::stroke::Sample { x, y, ..sample }
            })
            .collect()
    }
}

/// A ruler handle being dragged with the Pointer tool (or unlocked guides).
#[derive(Clone, Copy, Debug)]
pub struct RulerDrag {
    pub index: usize,
    pub handle: usize,
    /// Where a new ruler started being dragged out; None when editing.
    pub from: Option<[f32; 2]>,
    pub original: Ruler,
}

/// Screen-space hit test: (ruler index, handle index) under `point`.
pub fn handle_at(rulers: &[Ruler], view: &View, point: Pos2) -> Option<(usize, usize)> {
    rulers
        .iter()
        .enumerate()
        .rev()
        .flat_map(|(index, ruler)| {
            ruler
                .handles()
                .into_iter()
                .enumerate()
                .map(move |(handle, at)| (index, handle, at))
        })
        .map(|(index, handle, at)| {
            (
                index,
                handle,
                (view.to_screen(pos2(at[0], at[1])) - point).length(),
            )
        })
        .filter(|(_, _, distance)| *distance <= HANDLE)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(index, handle, _)| (index, handle))
}

pub fn paint(
    painter: &egui::Painter,
    view: &View,
    rulers: &[Ruler],
    snapping: bool,
    active: Option<Track>,
) {
    let alpha = if snapping { 230 } else { 110 };
    let color = Color32::from_rgba_unmultiplied(COLOR.r(), COLOR.g(), COLOR.b(), alpha);
    let faint = Color32::from_rgba_unmultiplied(COLOR.r(), COLOR.g(), COLOR.b(), alpha / 3);
    let line = Stroke::new(1.5_f32, color);
    let screen = |p: [f32; 2]| view.to_screen(pos2(p[0], p[1]));
    let reach = 100_000.0;
    let infinite = |origin: [f32; 2], direction: [f32; 2], stroke: Stroke| {
        painter.line_segment(
            [
                screen([
                    origin[0] - direction[0] * reach,
                    origin[1] - direction[1] * reach,
                ]),
                screen([
                    origin[0] + direction[0] * reach,
                    origin[1] + direction[1] * reach,
                ]),
            ],
            stroke,
        );
    };
    let ellipse = |center: [f32; 2], radii: [f32; 2], angle: f32, stroke: Stroke| {
        let (u, v) = Ruler::ellipse_axes(angle);
        let points: Vec<Pos2> = (0..=96)
            .map(|i| {
                let phi = std::f32::consts::TAU * i as f32 / 96.0;
                let (sin, cos) = phi.sin_cos();
                let (x, y) = (radii[0] * cos, radii[1] * sin);
                screen([
                    center[0] + u[0] * x + v[0] * y,
                    center[1] + u[1] * x + v[1] * y,
                ])
            })
            .collect();
        painter.add(egui::Shape::line(points, stroke));
    };
    for ruler in rulers {
        match *ruler {
            Ruler::Line { a, b } => {
                if let Some(direction) = unit(sub(b, a)) {
                    infinite(a, direction, line);
                }
            }
            Ruler::Parallel { a, b } => {
                if let Some(direction) = unit(sub(b, a)) {
                    let normal = [-direction[1], direction[0]];
                    for k in -3..=3 {
                        let offset = k as f32 * 60.0 / view.zoom.max(1e-3);
                        let origin = [a[0] + normal[0] * offset, a[1] + normal[1] * offset];
                        infinite(
                            origin,
                            direction,
                            if k == 0 {
                                line
                            } else {
                                Stroke::new(1.0_f32, faint)
                            },
                        );
                    }
                }
            }
            Ruler::Focus { center } => {
                for k in 0..24 {
                    let phi = std::f32::consts::TAU * k as f32 / 24.0;
                    let (sin, cos) = phi.sin_cos();
                    painter.line_segment(
                        [
                            screen(center),
                            screen([center[0] + cos * reach, center[1] + sin * reach]),
                        ],
                        Stroke::new(1.0_f32, faint),
                    );
                }
            }
            Ruler::Ellipse {
                center,
                radii,
                angle,
            } => {
                ellipse(center, radii, angle, line);
                for scale in [0.5, 1.5] {
                    ellipse(
                        center,
                        [radii[0] * scale, radii[1] * scale],
                        angle,
                        Stroke::new(1.0_f32, faint),
                    );
                }
            }
            Ruler::Perspective { vanishing, three } => {
                let [a, b, _] = vanishing;
                let horizon = sub(b, a);
                if let Some(direction) = unit(horizon) {
                    infinite(a, direction, line);
                }
                // Rays toward each vanishing point, aimed across the middle of
                // the scene so they read as a perspective grid.
                let points = &vanishing[..if three { 3 } else { 2 }];
                let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let span = length(horizon).max(1.0);
                for point in points {
                    for k in -6..=6 {
                        let normal = unit([-horizon[1], horizon[0]]).unwrap_or([0.0, 1.0]);
                        let target = [
                            middle[0] + normal[0] * span * k as f32 / 8.0,
                            middle[1] + normal[1] * span * k as f32 / 8.0,
                        ];
                        if let Some(direction) = unit(sub(target, *point)) {
                            painter.line_segment(
                                [
                                    screen(*point),
                                    screen([
                                        point[0] + direction[0] * reach,
                                        point[1] + direction[1] * reach,
                                    ]),
                                ],
                                Stroke::new(1.0_f32, faint),
                            );
                        }
                    }
                }
                if !three && let Some(along) = unit(horizon) {
                    let upright = [-along[1], along[0]];
                    for k in -4..=4 {
                        let origin = [
                            middle[0] + along[0] * span * k as f32 / 8.0,
                            middle[1] + along[1] * span * k as f32 / 8.0,
                        ];
                        infinite(origin, upright, Stroke::new(1.0_f32, faint));
                    }
                }
            }
        }
        for at in ruler.handles() {
            let p = screen(at);
            let diamond = vec![
                p + egui::vec2(0.0, -5.0),
                p + egui::vec2(5.0, 0.0),
                p + egui::vec2(0.0, 5.0),
                p + egui::vec2(-5.0, 0.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                diamond,
                Color32::WHITE,
                Stroke::new(1.5_f32, color),
            ));
        }
    }
    // The track the current stroke is held to.
    match active {
        Some(Track::Line { origin, direction }) => {
            infinite(origin, direction, Stroke::new(1.0_f32, color))
        }
        Some(Track::Ellipse {
            center,
            radii,
            angle,
        }) => ellipse(center, radii, angle, Stroke::new(1.0_f32, color)),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{guides::GuideLine, stroke::Sample};
    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }
    fn sample(x: f32, y: f32) -> Sample {
        Sample {
            x,
            y,
            pressure: 1.0,
            ..Default::default()
        }
    }
    #[test]
    fn tracks_project_onto_lines_focus_and_ellipses() {
        let line = Track::Line {
            origin: [0.0, 0.0],
            direction: [1.0, 0.0],
        };
        assert!(close(line.project([5.0, 3.0]), [5.0, 0.0]));
        // Ellipse with radii 20 x 10 rotated 90 degrees: x axis points down.
        let ellipse = Track::Ellipse {
            center: [100.0, 100.0],
            radii: [20.0, 10.0],
            angle: 90.0,
        };
        assert!(close(ellipse.project([100.0, 150.0]), [100.0, 120.0]));
        assert!(close(ellipse.project([60.0, 100.0]), [90.0, 100.0]));
        let t = ellipse.tangent([100.0, 120.0]);
        assert!(t[1].abs() < 1e-3 && t[0].abs() > 0.99);
        // Focus rulers radiate: a stroke from (110, 100) stays on y = 100.
        let sources = Sources {
            rulers: &[Ruler::Focus {
                center: [100.0, 100.0],
            }],
            guides: &[],
            grid: None,
            zoom: 1.0,
        };
        let mut snap = StrokeSnap::begin(&sources, [110.0, 100.0]).unwrap();
        let drawn = snap.feed(sample(150.0, 108.0), 1.0, false);
        assert!(close([drawn[0].x, drawn[0].y], [150.0, 100.0]));
        // Concentric ellipses keep the shape and pass through the start.
        let sources = Sources {
            rulers: &[Ruler::Ellipse {
                center: [0.0, 0.0],
                radii: [40.0, 20.0],
                angle: 0.0,
            }],
            guides: &[],
            grid: None,
            zoom: 1.0,
        };
        let candidates = candidates(&sources, [80.0, 0.0]);
        assert_eq!(
            candidates,
            vec![Track::Ellipse {
                center: [0.0, 0.0],
                radii: [80.0, 40.0],
                angle: 0.0
            }]
        );
    }
    #[test]
    fn near_lines_catch_strokes_and_motion_picks_between_tracks() {
        let guides = [GuideLine {
            vertical: true,
            position: 50.0,
        }];
        let rulers = [Ruler::Parallel {
            a: [0.0, 0.0],
            b: [10.0, 0.0],
        }];
        let sources = Sources {
            rulers: &rulers,
            guides: &guides,
            grid: None,
            zoom: 2.0,
        };
        // Far from the guide only the parallel ruler applies, immediately.
        let snap = StrokeSnap::begin(&sources, [10.0, 30.0]).unwrap();
        assert_eq!(
            snap.chosen(),
            Some(Track::Line {
                origin: [10.0, 30.0],
                direction: [1.0, 0.0]
            })
        );
        // Near the guide (4 document px = 8 screen px) both apply; moving down
        // picks the guide, and held samples come out projected onto it.
        let mut snap = StrokeSnap::begin(&sources, [54.0, 30.0]).unwrap();
        assert!(snap.chosen().is_none());
        assert!(snap.feed(sample(54.0, 30.0), 2.0, false).is_empty());
        assert!(snap.feed(sample(55.0, 31.0), 2.0, false).is_empty());
        let drawn = snap.feed(sample(55.0, 36.0), 2.0, false);
        assert_eq!(drawn.len(), 3);
        assert!(drawn.iter().all(|s| s.x == 50.0));
        // Moving sideways from the same point picks the parallel ruler.
        let mut snap = StrokeSnap::begin(&sources, [54.0, 30.0]).unwrap();
        let drawn = snap.feed(sample(62.0, 31.0), 2.0, false);
        assert!(drawn.iter().all(|s| s.y == 30.0));
        // Nothing near and no family rulers: no snapping at all.
        let sources = Sources {
            rulers: &[],
            guides: &guides,
            grid: Some(100.0),
            zoom: 1.0,
        };
        assert!(StrokeSnap::begin(&sources, [20.0, 30.0]).is_none());
        // Grid lines catch nearby strokes when grid snapping is on.
        let snap = StrokeSnap::begin(&sources, [205.0, 30.0]).unwrap();
        assert_eq!(
            snap.chosen(),
            Some(Track::Line {
                origin: [200.0, 0.0],
                direction: [0.0, 1.0]
            })
        );
    }
    #[test]
    fn perspective_rulers_head_to_vanishing_points_or_stay_upright() {
        let mut ruler = Ruler::new(Kind::TwoPoint, [0.0, 100.0]);
        ruler.drag_out([0.0, 100.0], [400.0, 100.0]);
        assert!(ruler.usable());
        assert_eq!(ruler.handles(), vec![[0.0, 100.0], [400.0, 100.0]]);
        let rulers = [ruler];
        let sources = Sources {
            rulers: &rulers,
            guides: &[],
            grid: None,
            zoom: 1.0,
        };
        let start = [200.0, 200.0];
        // Three candidate tracks: toward each vanishing point, and upright.
        assert_eq!(candidates(&sources, start).len(), 3);
        let toward_left = [start[0] - 20.0, start[1] - 10.0];
        let mut snap = StrokeSnap::begin(&sources, start).unwrap();
        snap.feed(sample(start[0], start[1]), 1.0, false);
        let drawn = snap.feed(sample(toward_left[0], toward_left[1] + 1.5), 1.0, false);
        let last = drawn.last().unwrap();
        // On the line from (0, 100) through the start: slope 0.5.
        assert!((last.y - (100.0 + last.x * 0.5)).abs() < 1e-3, "{last:?}");
        let mut snap = StrokeSnap::begin(&sources, start).unwrap();
        snap.feed(sample(start[0], start[1]), 1.0, false);
        let drawn = snap.feed(sample(start[0] + 1.0, start[1] + 12.0), 1.0, false);
        assert!(drawn.iter().all(|s| (s.x - 200.0).abs() < 1e-3));
        // Three-point: the third point starts below the horizon and replaces
        // the upright track.
        let mut ruler = Ruler::new(Kind::ThreePoint, [0.0, 100.0]);
        ruler.drag_out([0.0, 100.0], [400.0, 100.0]);
        assert_eq!(ruler.handles()[2], [200.0, 500.0]);
        let rulers = [ruler];
        let sources = Sources {
            rulers: &rulers,
            guides: &[],
            grid: None,
            zoom: 1.0,
        };
        let mut snap = StrokeSnap::begin(&sources, start).unwrap();
        snap.feed(sample(start[0], start[1]), 1.0, false);
        let drawn = snap.feed(sample(start[0], start[1] + 12.0), 1.0, false);
        // Down toward (200, 500): the track through the third point.
        assert!(drawn.iter().all(|s| (s.x - 200.0).abs() < 1e-3));
        let mut turned = ruler;
        turned.rotate_clockwise(600);
        assert_eq!(turned.handles()[0], [500.0, 0.0]);
    }
    #[test]
    fn handles_shape_rulers() {
        let mut ruler = Ruler::new(Kind::Ellipse, [10.0, 10.0]);
        ruler.drag_out([10.0, 10.0], [40.0, 30.0]);
        assert_eq!(
            ruler,
            Ruler::Ellipse {
                center: [10.0, 10.0],
                radii: [30.0, 20.0],
                angle: 0.0
            }
        );
        ruler.set_handle(1, [10.0, 60.0]);
        let Ruler::Ellipse { radii, angle, .. } = ruler else {
            unreachable!()
        };
        assert!((radii[0] - 50.0).abs() < 1e-3 && (angle - 90.0).abs() < 1e-3);
        assert!(ruler.usable());
        assert!(!Ruler::new(Kind::Line, [1.0, 1.0]).usable());
    }
}
