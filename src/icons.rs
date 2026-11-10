use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, vec2};

/// Vector icons drawn with the painter so they scale cleanly at any display
/// density and do not depend on glyph coverage in the bundled fonts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Brush,
    Pencil,
    Pen,
    Airbrush,
    Eraser,
    Fill,
    Picker,
    Select,
    LassoFill,
    LassoErase,
    Move,
    Eye,
    EyeOff,
    Lock,
    Plus,
    Trash,
    Duplicate,
    MergeDown,
    Undo,
    Redo,
    Clip,
    AlphaLock,
    Folder,
    NewFolder,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    ZoomIn,
    ZoomOut,
    RotateLeft,
    RotateRight,
    Flip,
    Fit,
    Symmetry,
    Global,
    Guide,
    Pointer,
    /// The Text tool: a capital T.
    Text,
    Transparency,
    /// Layer effects: a sparkle.
    Effects,
    /// Adjustment layers: a half-filled circle.
    Adjustment,
    /// Layer mask: a frame around a filled circle.
    Mask,
    /// More actions: three dots.
    More,
    /// The Fill tool's Gradient mode: a frame with a rising ramp.
    Gradient,
}

pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let origin = rect.center() - vec2(side, side) / 2.0;
    let p = |x: f32, y: f32| origin + vec2(x, y) * side;
    let width = (side / 13.0).clamp(1.2, 2.4);
    let line = Stroke::new(width, color);
    let polyline = |points: &[Pos2]| painter.add(Shape::line(points.to_vec(), line));
    let closed = |points: &[Pos2]| painter.add(Shape::closed_line(points.to_vec(), line));
    let filled =
        |points: Vec<Pos2>| painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    let arrow_head = |tip: Pos2, toward: egui::Vec2| {
        let back = toward.normalized() * side * 0.16;
        let across = egui::vec2(-back.y, back.x) * 0.8;
        polyline(&[tip - back + across, tip, tip - back - across]);
    };
    match icon {
        Icon::Global => {
            painter.circle_stroke(p(0.5, 0.5), side * 0.36, line);
            polyline(&[p(0.14, 0.5), p(0.86, 0.5)]);
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 24.0;
                    p(0.5 + 0.17 * a.cos(), 0.5 + 0.36 * a.sin())
                })
                .collect();
            polyline(&points);
        }
        Icon::Guide => {
            closed(&[p(0.12, 0.70), p(0.70, 0.12), p(0.88, 0.30), p(0.30, 0.88)]);
            for i in 0..4 {
                let t = i as f32 * 0.12;
                polyline(&[p(0.30 + t, 0.52 - t), p(0.39 + t, 0.61 - t)]);
            }
        }
        Icon::Brush => {
            painter.line_segment(
                [p(0.84, 0.16), p(0.52, 0.48)],
                Stroke::new(width * 1.3, color),
            );
            filled(vec![
                p(0.47, 0.41),
                p(0.59, 0.53),
                p(0.36, 0.80),
                p(0.18, 0.84),
                p(0.22, 0.65),
            ]);
        }
        Icon::Pencil => {
            // Hexagonal barrel with a sharpened cone and graphite point.
            closed(&[p(0.80, 0.14), p(0.88, 0.22), p(0.40, 0.70), p(0.30, 0.60)]);
            polyline(&[p(0.72, 0.22), p(0.80, 0.30)]);
            closed(&[p(0.30, 0.60), p(0.40, 0.70), p(0.16, 0.84)]);
            filled(vec![p(0.21, 0.73), p(0.27, 0.79), p(0.16, 0.84)]);
        }
        Icon::Pen => {
            // G-pen nib: pointed shoulders, slit and breather hole.
            let nib = vec![
                p(0.16, 0.84),
                p(0.36, 0.40),
                p(0.60, 0.40),
                p(0.84, 0.16),
                p(0.60, 0.64),
            ];
            closed(&nib);
            polyline(&[p(0.16, 0.84), p(0.44, 0.56)]);
            painter.circle_filled(p(0.50, 0.50), side * 0.05, color);
        }
        Icon::Airbrush => {
            // Diagonal body with a paint cup, spraying a cone of dots.
            painter.line_segment(
                [p(0.88, 0.12), p(0.50, 0.50)],
                Stroke::new(width * 2.2, color),
            );
            closed(&[p(0.62, 0.20), p(0.74, 0.08), p(0.82, 0.16), p(0.70, 0.28)]);
            polyline(&[p(0.58, 0.52), p(0.66, 0.60)]);
            for (x, y, r) in [
                (0.40, 0.60, 0.045),
                (0.30, 0.64, 0.04),
                (0.36, 0.72, 0.04),
                (0.20, 0.70, 0.035),
                (0.28, 0.80, 0.035),
                (0.14, 0.84, 0.03),
            ] {
                painter.circle_filled(p(x, y), side * r, color);
            }
        }
        Icon::Eraser => {
            closed(&[p(0.18, 0.62), p(0.56, 0.24), p(0.84, 0.52), p(0.46, 0.90)]);
            filled(vec![
                p(0.18, 0.62),
                p(0.37, 0.43),
                p(0.65, 0.71),
                p(0.46, 0.90),
            ]);
            polyline(&[p(0.52, 0.90), p(0.88, 0.90)]);
        }
        Icon::Fill => {
            closed(&[p(0.16, 0.46), p(0.46, 0.16), p(0.76, 0.46), p(0.46, 0.76)]);
            filled(vec![p(0.16, 0.46), p(0.76, 0.46), p(0.46, 0.76)]);
            polyline(&[p(0.76, 0.46), p(0.84, 0.64)]);
            painter.circle_filled(p(0.84, 0.74), side * 0.07, color);
        }
        Icon::Picker => {
            painter.line_segment(
                [p(0.20, 0.80), p(0.58, 0.42)],
                Stroke::new(width * 1.2, color),
            );
            painter.line_segment(
                [p(0.48, 0.36), p(0.64, 0.52)],
                Stroke::new(width * 1.2, color),
            );
            painter.circle_filled(p(0.70, 0.30), side * 0.13, color);
            painter.circle_filled(p(0.18, 0.82), side * 0.05, color);
        }
        Icon::Select => {
            let corners = [
                p(0.18, 0.24),
                p(0.82, 0.24),
                p(0.82, 0.76),
                p(0.18, 0.76),
                p(0.18, 0.24),
            ];
            painter.extend(Shape::dashed_line(&corners, line, side * 0.1, side * 0.07));
        }
        Icon::LassoFill | Icon::LassoErase => {
            let shape = vec![
                p(0.18, 0.28),
                p(0.60, 0.18),
                p(0.84, 0.40),
                p(0.72, 0.78),
                p(0.30, 0.82),
                p(0.14, 0.55),
            ];
            if icon == Icon::LassoFill {
                filled(shape);
            } else {
                closed(&shape);
                polyline(&[p(0.32, 0.50), p(0.66, 0.50)]);
            }
        }
        Icon::Adjustment => {
            painter.circle_stroke(p(0.5, 0.5), side * 0.36, line);
            let half: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 16.0;
                    p(0.5 + 0.36 * a.cos(), 0.5 + 0.36 * a.sin())
                })
                .collect();
            filled(half);
        }
        Icon::Mask => {
            closed(&[p(0.14, 0.20), p(0.86, 0.20), p(0.86, 0.80), p(0.14, 0.80)]);
            painter.circle_filled(p(0.5, 0.5), side * 0.18, color);
        }
        Icon::Gradient => {
            closed(&[p(0.14, 0.22), p(0.86, 0.22), p(0.86, 0.78), p(0.14, 0.78)]);
            filled(vec![p(0.20, 0.72), p(0.80, 0.72), p(0.80, 0.30)]);
        }
        Icon::More => {
            for x in [0.22, 0.5, 0.78] {
                painter.circle_filled(p(x, 0.5), side * 0.075, color);
            }
        }
        Icon::Effects => {
            let star = |cx: f32, cy: f32, r: f32| -> Vec<Pos2> {
                (0..8)
                    .map(|i| {
                        let a = i as f32 * std::f32::consts::TAU / 8.0;
                        let d = if i % 2 == 0 { r } else { r * 0.3 };
                        p(cx + d * a.sin(), cy - d * a.cos())
                    })
                    .collect()
            };
            closed(&star(0.42, 0.58, 0.34));
            closed(&star(0.8, 0.2, 0.15));
        }
        Icon::Transparency => {
            // Checkerboard: two filled and two outlined squares.
            closed(&[p(0.18, 0.18), p(0.82, 0.18), p(0.82, 0.82), p(0.18, 0.82)]);
            filled(vec![
                p(0.18, 0.18),
                p(0.50, 0.18),
                p(0.50, 0.50),
                p(0.18, 0.50),
            ]);
            filled(vec![
                p(0.50, 0.50),
                p(0.82, 0.50),
                p(0.82, 0.82),
                p(0.50, 0.82),
            ]);
        }
        Icon::Pointer => {
            closed(&[
                p(0.28, 0.12),
                p(0.28, 0.80),
                p(0.44, 0.65),
                p(0.56, 0.90),
                p(0.67, 0.85),
                p(0.55, 0.60),
                p(0.76, 0.60),
            ]);
        }
        Icon::Text => {
            polyline(&[p(0.2, 0.28), p(0.2, 0.16), p(0.8, 0.16), p(0.8, 0.28)]);
            polyline(&[p(0.5, 0.16), p(0.5, 0.84)]);
            polyline(&[p(0.38, 0.84), p(0.62, 0.84)]);
        }
        Icon::Move => {
            polyline(&[p(0.5, 0.12), p(0.5, 0.88)]);
            polyline(&[p(0.12, 0.5), p(0.88, 0.5)]);
            arrow_head(p(0.5, 0.12), vec2(0.0, -1.0));
            arrow_head(p(0.5, 0.88), vec2(0.0, 1.0));
            arrow_head(p(0.12, 0.5), vec2(-1.0, 0.0));
            arrow_head(p(0.88, 0.5), vec2(1.0, 0.0));
        }
        Icon::Eye | Icon::EyeOff => {
            let steps = 14;
            let mut outline = Vec::with_capacity(steps * 2);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                outline.push(p(
                    0.12 + 0.76 * t,
                    0.5 - 0.24 * (t * std::f32::consts::PI).sin(),
                ));
            }
            for i in (1..steps).rev() {
                let t = i as f32 / steps as f32;
                outline.push(p(
                    0.12 + 0.76 * t,
                    0.5 + 0.24 * (t * std::f32::consts::PI).sin(),
                ));
            }
            closed(&outline);
            if icon == Icon::Eye {
                painter.circle_filled(p(0.5, 0.5), side * 0.11, color);
            } else {
                painter.line_segment([p(0.18, 0.84), p(0.82, 0.16)], line);
            }
        }
        Icon::Lock => {
            painter.rect_filled(
                Rect::from_min_max(p(0.24, 0.46), p(0.76, 0.86)),
                side * 0.06,
                color,
            );
            let arc: Vec<Pos2> = (0..=10)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 10.0);
                    p(0.5 + 0.16 * a.cos(), 0.46 + 0.2 * a.sin())
                })
                .collect();
            polyline(&arc);
        }
        Icon::Plus => {
            polyline(&[p(0.5, 0.2), p(0.5, 0.8)]);
            polyline(&[p(0.2, 0.5), p(0.8, 0.5)]);
        }
        Icon::Trash => {
            closed(&[p(0.26, 0.32), p(0.74, 0.32), p(0.68, 0.86), p(0.32, 0.86)]);
            polyline(&[p(0.18, 0.26), p(0.82, 0.26)]);
            polyline(&[p(0.40, 0.26), p(0.42, 0.16), p(0.58, 0.16), p(0.60, 0.26)]);
            polyline(&[p(0.44, 0.44), p(0.45, 0.74)]);
            polyline(&[p(0.56, 0.44), p(0.55, 0.74)]);
        }
        Icon::Duplicate => {
            closed(&[p(0.18, 0.32), p(0.62, 0.32), p(0.62, 0.84), p(0.18, 0.84)]);
            polyline(&[
                p(0.36, 0.32),
                p(0.36, 0.16),
                p(0.82, 0.16),
                p(0.82, 0.66),
                p(0.62, 0.66),
            ]);
        }
        Icon::MergeDown => {
            polyline(&[p(0.5, 0.14), p(0.5, 0.62)]);
            arrow_head(p(0.5, 0.62), vec2(0.0, 1.0));
            polyline(&[p(0.18, 0.84), p(0.82, 0.84)]);
        }
        Icon::Folder | Icon::NewFolder => {
            closed(&[
                p(0.12, 0.26),
                p(0.40, 0.26),
                p(0.48, 0.34),
                p(0.88, 0.34),
                p(0.88, 0.80),
                p(0.12, 0.80),
            ]);
            if icon == Icon::NewFolder {
                polyline(&[p(0.50, 0.46), p(0.50, 0.70)]);
                polyline(&[p(0.38, 0.58), p(0.62, 0.58)]);
            } else {
                polyline(&[p(0.12, 0.44), p(0.88, 0.44)]);
            }
        }
        Icon::ChevronRight => {
            polyline(&[p(0.38, 0.22), p(0.66, 0.5), p(0.38, 0.78)]);
        }
        Icon::ChevronDown => {
            polyline(&[p(0.22, 0.38), p(0.5, 0.66), p(0.78, 0.38)]);
        }
        Icon::ChevronUp => {
            polyline(&[p(0.22, 0.62), p(0.5, 0.34), p(0.78, 0.62)]);
        }
        Icon::ZoomIn | Icon::ZoomOut => {
            painter.circle_stroke(p(0.42, 0.42), side * 0.25, line);
            polyline(&[p(0.60, 0.60), p(0.86, 0.86)]);
            polyline(&[p(0.30, 0.42), p(0.54, 0.42)]);
            if icon == Icon::ZoomIn {
                polyline(&[p(0.42, 0.30), p(0.42, 0.54)]);
            }
        }
        Icon::RotateLeft | Icon::RotateRight => {
            let flip = |x: f32| {
                if icon == Icon::RotateRight {
                    x
                } else {
                    1.0 - x
                }
            };
            let arc: Vec<Pos2> = (0..=14)
                .map(|i| {
                    let a = -2.4 + 4.0 * i as f32 / 14.0;
                    p(flip(0.5 + 0.3 * a.cos()), 0.52 + 0.3 * a.sin())
                })
                .collect();
            polyline(&arc);
            let end = arc[arc.len() - 1];
            let before = arc[arc.len() - 2];
            arrow_head(end, end - before);
        }
        Icon::Flip => {
            painter.add(Shape::dashed_line(
                &[p(0.5, 0.12), p(0.5, 0.88)],
                line,
                side * 0.08,
                side * 0.06,
            ));
            closed(&[p(0.40, 0.24), p(0.40, 0.76), p(0.12, 0.76)]);
            filled(vec![p(0.60, 0.24), p(0.88, 0.76), p(0.60, 0.76)]);
        }
        Icon::Fit => {
            for (corner, dx, dy) in [
                (p(0.16, 0.16), 1.0, 1.0),
                (p(0.84, 0.16), -1.0, 1.0),
                (p(0.84, 0.84), -1.0, -1.0),
                (p(0.16, 0.84), 1.0, -1.0),
            ] {
                polyline(&[
                    corner + vec2(dx * side * 0.22, 0.0),
                    corner,
                    corner + vec2(0.0, dy * side * 0.22),
                ]);
            }
        }
        Icon::Symmetry => {
            painter.add(Shape::dashed_line(
                &[p(0.5, 0.1), p(0.5, 0.9)],
                line,
                side * 0.08,
                side * 0.06,
            ));
            painter.circle_stroke(p(0.27, 0.5), side * 0.13, line);
            painter.circle_stroke(p(0.73, 0.5), side * 0.13, line);
        }
        Icon::Clip => {
            polyline(&[p(0.30, 0.14), p(0.30, 0.66), p(0.82, 0.66)]);
            arrow_head(p(0.82, 0.66), vec2(1.0, 0.0));
        }
        Icon::AlphaLock => {
            closed(&[p(0.16, 0.16), p(0.84, 0.16), p(0.84, 0.84), p(0.16, 0.84)]);
            filled(vec![p(0.16, 0.16), p(0.5, 0.16), p(0.5, 0.5), p(0.16, 0.5)]);
            filled(vec![p(0.5, 0.5), p(0.84, 0.5), p(0.84, 0.84), p(0.5, 0.84)]);
        }
        Icon::Undo | Icon::Redo => {
            let flip = |x: f32| if icon == Icon::Undo { x } else { 1.0 - x };
            let arc: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                    p(flip(0.52 + 0.26 * a.cos()), 0.52 + 0.24 * a.sin())
                })
                .collect();
            polyline(&arc);
            polyline(&[p(flip(0.52), 0.76), p(flip(0.24), 0.76)]);
            polyline(&[
                p(flip(0.38), 0.62),
                p(flip(0.22), 0.76),
                p(flip(0.38), 0.90),
            ]);
        }
    }
}

/// Square icon button. Selected buttons use the accent fill.
pub fn button(ui: &mut Ui, icon: Icon, side: f32, selected: bool, enabled: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), sense);
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() && enabled {
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            visuals.weak_bg_fill
        };
        ui.painter().rect_filled(rect, 6.0, fill);
    }
    let color = if !enabled {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    } else if selected {
        Color32::WHITE
    } else {
        visuals.fg_stroke.color
    };
    paint(ui.painter(), rect.shrink(side * 0.2), icon, color);
    response
}

/// Small inline icon (no interaction) for row decorations.
pub fn mark(ui: &mut Ui, icon: Icon, side: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    paint(ui.painter(), rect, icon, color);
    response
}
