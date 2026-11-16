//! App side of `rulers`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::rulers`, re-exported
//! here so `crate::rulers::…` paths keep working.
use crate::{egui, view::View};
pub(crate) use dudel_core::rulers::*;
use egui::{Color32, Pos2, Stroke, pos2};

const COLOR: Color32 = Color32::from_rgb(178, 120, 255);

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
