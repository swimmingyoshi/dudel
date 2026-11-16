//! App side of `dynamics`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::dynamics`, re-exported
//! here so `crate::dynamics::…` paths keep working.
pub(crate) use dudel_core::dynamics::*;

/// Drag points to shape a curve; click empty space to add a point and
/// double-click an inner point to remove it. Returns true when changed.
pub fn curve_ui(ui: &mut eframe::egui::Ui, curve: &mut Curve) -> bool {
    use eframe::egui::{self, Color32, Sense, Stroke, pos2, vec2};
    let width = ui.available_width().clamp(120.0, 320.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, width * 0.62), Sense::click_and_drag());
    let plot = rect.shrink(10.0);
    let to_screen = |p: [f32; 2]| {
        pos2(
            plot.left() + p[0] * plot.width(),
            plot.bottom() - p[1] * plot.height(),
        )
    };
    let from_screen = |p: egui::Pos2| {
        [
            ((p.x - plot.left()) / plot.width()).clamp(0.0, 1.0),
            ((plot.bottom() - p.y) / plot.height()).clamp(0.0, 1.0),
        ]
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let grid = Stroke::new(1.0_f32, Color32::from_gray(52));
        painter.line_segment([to_screen([f, 0.0]), to_screen([f, 1.0])], grid);
        painter.line_segment([to_screen([0.0, f]), to_screen([1.0, f])], grid);
    }
    painter.line_segment(
        [to_screen([0.0, 0.0]), to_screen([1.0, 1.0])],
        Stroke::new(1.0_f32, Color32::from_gray(70)),
    );
    let accent = ui.visuals().selection.stroke.color;
    let line: Vec<_> = (0..=64)
        .map(|i| {
            let x = i as f32 / 64.0;
            to_screen([x, curve.evaluate(x)])
        })
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0_f32, accent)));
    let id = response.id.with("dragged point");
    let nearest = |pointer: egui::Pos2, curve: &Curve| {
        curve
            .points
            .iter()
            .enumerate()
            .map(|(index, p)| (index, (to_screen(*p) - pointer).length()))
            .filter(|(_, distance)| *distance <= 10.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    };
    let mut changed = false;
    let pointer = response.interact_pointer_pos();
    // A drag is only reported once the pointer has moved, so the grabbed
    // point is the one under the press, not under the current position.
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin()).or(pointer)
    {
        let grabbed = nearest(origin, curve);
        ui.data_mut(|d| d.insert_temp(id, grabbed));
    }
    let dragged: Option<usize> = ui.data(|d| d.get_temp(id)).flatten();
    if response.dragged()
        && let (Some(index), Some(pointer)) = (dragged, pointer)
    {
        let count = curve.points.len();
        let [mut x, y] = from_screen(pointer);
        // The ends stay at x = 0 and x = 1; inner points stay between their
        // neighbours so the curve remains a function of the input.
        x = if index == 0 {
            0.0
        } else if index + 1 == count {
            1.0
        } else {
            x.clamp(
                curve.points[index - 1][0] + 0.01,
                curve.points[index + 1][0] - 0.01,
            )
        };
        if curve.points[index] != [x, y] {
            curve.points[index] = [x, y];
            changed = true;
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Option<usize>>(id));
    }
    if response.double_clicked()
        && let Some(pointer) = pointer
        && let Some(index) = nearest(pointer, curve)
        && index > 0
        && index + 1 < curve.points.len()
    {
        curve.points.remove(index);
        changed = true;
    } else if response.clicked()
        && let Some(pointer) = pointer
        && nearest(pointer, curve).is_none()
        && curve.points.len() < 16
    {
        let point = from_screen(pointer);
        let after = curve
            .points
            .iter()
            .position(|p| p[0] > point[0])
            .unwrap_or(curve.points.len() - 1);
        let (low, high) = (curve.points[after - 1][0], curve.points[after][0]);
        if point[0] > low + 0.01 && point[0] < high - 0.01 {
            curve.points.insert(after, point);
            changed = true;
        }
    }
    for (index, p) in curve.points.iter().enumerate() {
        let active = dragged == Some(index);
        painter.circle(
            to_screen(*p),
            if active { 6.0 } else { 4.5 },
            Color32::WHITE,
            Stroke::new(1.5_f32, accent),
        );
    }
    response
        .on_hover_text("Drag points to shape the curve · click to add · double-click to remove");
    changed
}

/// `driven` marks size, opacity, flow and spacing when a brush graph sets
/// them instead; those are shown but cannot be changed.
pub fn editor(ui: &mut eframe::egui::Ui, settings: &mut BrushDynamics, driven: [bool; 4]) {
    use eframe::egui;
    ui.small("Each enabled parameter uses its own curves. Missing speed or tilt leaves that input unchanged.");
    for ((label, setting), driven) in [
        ("Size", &mut settings.size),
        ("Opacity", &mut settings.opacity),
        ("Flow", &mut settings.flow),
        ("Spacing", &mut settings.spacing),
    ]
    .into_iter()
    .zip(driven)
    {
        if driven {
            ui.weak(format!("{label}: set by the brush graph"));
            continue;
        }
        ui.push_id(label, |ui| {
            let mut enabled = setting.is_some();
            if ui
                .checkbox(&mut enabled, format!("{label} dynamics"))
                .changed()
            {
                *setting = enabled.then(|| Dynamics {
                    minimum: if label == "Size" { 0.05 } else { 0.0 },
                    minimum_mode: MinimumMode::Clamp,
                    ..Default::default()
                });
            }
            if let Some(dynamic) = setting {
                ui.add(egui::Slider::new(&mut dynamic.minimum, 0.0..=1.0).text("Minimum"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut dynamic.minimum_mode, MinimumMode::Clamp, "Floor");
                    ui.selectable_value(&mut dynamic.minimum_mode, MinimumMode::Remap, "Remap");
                });
                for input in [
                    Input::Pressure,
                    Input::Speed,
                    Input::TiltAltitude,
                    Input::Random,
                ] {
                    let current = dynamic.inputs.iter().position(|e| e.input == input);
                    let mut enabled = current.is_some();
                    if ui.checkbox(&mut enabled, input.name()).changed() {
                        if let Some(index) = current {
                            dynamic.inputs.remove(index);
                        } else {
                            dynamic.inputs.push(Effector {
                                input,
                                curve: Curve::default(),
                            });
                        }
                    }
                }
                for effector in &mut dynamic.inputs {
                    egui::CollapsingHeader::new(format!("{} curve", effector.input.name()))
                        .id_salt(format!("{:?}", effector.input))
                        .default_open(true)
                        .show(ui, |ui| {
                            curve_ui(ui, &mut effector.curve);
                            if ui.small_button("Reset curve").clicked() {
                                effector.curve = Curve::default();
                            }
                        });
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_editor_drags_adds_and_removes_points_with_the_pointer() {
        use eframe::egui::{self, Event, PointerButton, pos2, vec2};
        let ctx = egui::Context::default();
        let mut curve = Curve::default();
        let area = egui::Rect::from_min_size(pos2(20.0, 20.0), vec2(300.0, 400.0));
        // The editor fills 300 wide (186 tall) with a 10pt inset plot.
        let plot = egui::Rect::from_min_size(pos2(20.0, 20.0), vec2(300.0, 186.0)).shrink(10.0);
        let at = |x: f32, y: f32| {
            pos2(
                plot.left() + x * plot.width(),
                plot.bottom() - y * plot.height(),
            )
        };
        let mut time = 0.0;
        let mut frame = |events: Vec<Event>, curve: &mut Curve| {
            time += 0.05;
            let input = egui::RawInput {
                events,
                time: Some(time),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::Area::new(egui::Id::new("curve test"))
                    .fixed_pos(area.min)
                    .show(ctx, |ui| {
                        ui.set_width(area.width());
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        curve_ui(ui, curve);
                    });
            });
        };
        let press = |p, pressed| Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        // An Area is invisible on its first (sizing) frame, and hits use the
        // previous frame's layout, so show it a few times first.
        for _ in 0..3 {
            frame(vec![], &mut curve);
        }
        // Drag the right end down: x stays at 1.
        let end = at(1.0, 1.0);
        frame(vec![Event::PointerMoved(end), press(end, true)], &mut curve);
        frame(vec![Event::PointerMoved(at(0.9, 0.7))], &mut curve);
        frame(vec![Event::PointerMoved(at(0.8, 0.4))], &mut curve);
        frame(vec![press(at(0.8, 0.4), false)], &mut curve);
        assert_eq!(curve.points.len(), 2);
        assert_eq!(curve.points[1][0], 1.0);
        assert!(
            (curve.points[1][1] - 0.4).abs() < 0.02,
            "{:?}",
            curve.points
        );
        // A click on empty space adds a point there.
        let empty = at(0.5, 0.9);
        frame(
            vec![Event::PointerMoved(empty), press(empty, true)],
            &mut curve,
        );
        frame(vec![press(empty, false)], &mut curve);
        assert_eq!(curve.points.len(), 3);
        assert!((curve.points[1][0] - 0.5).abs() < 0.02 && (curve.points[1][1] - 0.9).abs() < 0.02);
        curve.validate().unwrap();
        // A double-click on it, after a pause, removes it again.
        for _ in 0..10 {
            frame(vec![], &mut curve);
        }
        for _ in 0..2 {
            frame(
                vec![Event::PointerMoved(empty), press(empty, true)],
                &mut curve,
            );
            frame(vec![press(empty, false)], &mut curve);
        }
        assert_eq!(curve.points.len(), 2, "{:?}", curve.points);
    }
}
