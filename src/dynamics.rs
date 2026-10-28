//! Typed per-parameter curves. Missing sensor values leave a parameter unchanged.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    Pressure,
    Speed,
    TiltAltitude,
    Random,
}

impl Input {
    pub fn name(self) -> &'static str {
        match self {
            Input::Pressure => "Pressure",
            Input::Speed => "Speed",
            Input::TiltAltitude => "Tilt",
            Input::Random => "Random",
        }
    }
}

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

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub points: Vec<[f32; 2]>,
}
impl Default for Curve {
    fn default() -> Self {
        Self {
            points: vec![[0.0, 0.0], [1.0, 1.0]],
        }
    }
}
impl Curve {
    pub fn validate(&self) -> Result<(), String> {
        if !(2..=16).contains(&self.points.len())
            || self.points.first().map(|p| p[0]) != Some(0.0)
            || self.points.last().map(|p| p[0]) != Some(1.0)
            || self
                .points
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.points.windows(2).any(|p| p[0][0] >= p[1][0])
        {
            return Err(
                "Dynamics curves need 2–16 points, ascending x from 0 to 1, and values in 0–1"
                    .into(),
            );
        }
        Ok(())
    }
    pub fn evaluate(&self, input: f32) -> f32 {
        let x = input.clamp(0.0, 1.0);
        for pair in self.points.windows(2) {
            if x <= pair[1][0] {
                let t = (x - pair[0][0]) / (pair[1][0] - pair[0][0]);
                return pair[0][1] + (pair[1][1] - pair[0][1]) * t;
            }
        }
        self.points.last().map_or(1.0, |p| p[1])
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MinimumMode {
    /// Preserve legacy pressure-size max(response, minimum) semantics.
    Clamp,
    #[default]
    Remap,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Effector {
    pub input: Input,
    pub curve: Curve,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Dynamics {
    pub minimum: f32,
    #[serde(default)]
    pub minimum_mode: MinimumMode,
    pub inputs: Vec<Effector>,
}
impl Default for Dynamics {
    fn default() -> Self {
        Self {
            minimum: 0.0,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![Effector {
                input: Input::Pressure,
                curve: Curve::default(),
            }],
        }
    }
}
impl Dynamics {
    pub fn validate(&self) -> Result<(), String> {
        if !self.minimum.is_finite()
            || !(0.0..=1.0).contains(&self.minimum)
            || self.inputs.len() > 4
        {
            return Err("Invalid dynamics minimum or input count".into());
        }
        for (i, effector) in self.inputs.iter().enumerate() {
            effector.curve.validate()?;
            if self.inputs[..i].iter().any(|e| e.input == effector.input) {
                return Err("Duplicate dynamics input".into());
            }
        }
        Ok(())
    }
    pub fn evaluate(&self, sample: &crate::stroke::Sample) -> f32 {
        let value = self.inputs.iter().fold(1.0, |product, effector| {
            let input = match effector.input {
                Input::Pressure => Some(sample.pressure),
                Input::Speed => sample.speed,
                Input::TiltAltitude => sample
                    .tilt
                    .map(|[x, y]| 1.0 - x.abs().max(y.abs()).min(90.0) / 90.0),
                Input::Random => sample.random,
            };
            product * input.map_or(1.0, |x| effector.curve.evaluate(x))
        });
        match self.minimum_mode {
            MinimumMode::Clamp => value.max(self.minimum),
            MinimumMode::Remap => self.minimum + (1.0 - self.minimum) * value,
        }
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrushDynamics {
    pub size: Option<Dynamics>,
    pub opacity: Option<Dynamics>,
    pub flow: Option<Dynamics>,
    pub spacing: Option<Dynamics>,
}

pub fn editor(ui: &mut eframe::egui::Ui, settings: &mut BrushDynamics) {
    use eframe::egui;
    ui.small("Each enabled parameter uses its own curves. Missing speed or tilt leaves that input unchanged.");
    for (label, setting) in [
        ("Size", &mut settings.size),
        ("Opacity", &mut settings.opacity),
        ("Flow", &mut settings.flow),
        ("Spacing", &mut settings.spacing),
    ] {
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
impl BrushDynamics {
    pub fn validate(&self) -> Result<(), String> {
        for dynamic in [&self.size, &self.opacity, &self.flow, &self.spacing]
            .into_iter()
            .flatten()
        {
            dynamic.validate()?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Velocity {
    points: std::collections::VecDeque<(f64, f64)>,
    last: Option<(f32, f32)>,
    distance: f64,
    count: usize,
}
impl Velocity {
    /// Screen pixels / second normalized to 1 at 1000 px/s, 40 ms window.
    /// No estimate is produced for missing, repeated or decreasing timestamps.
    pub fn measure(&mut self, sample: &crate::stroke::Sample, zoom: f32) -> Option<f32> {
        let Some(time) = sample.time.filter(|t| t.is_finite()) else {
            *self = Self::default();
            return None;
        };
        if self.points.back().is_some_and(|p| time <= p.0) {
            return None;
        }
        if let Some((x, y)) = self.last {
            self.distance += (sample.x - x).hypot(sample.y - y) as f64 * zoom as f64;
        }
        self.last = Some((sample.x, sample.y));
        self.points.push_back((time, self.distance));
        self.count += 1;
        while self.points.len() > 2 && self.points.get(1).is_some_and(|p| p.0 < time - 0.04) {
            self.points.pop_front();
        }
        while self.points.len() > 128 {
            self.points.pop_front();
        }
        let &(start, distance) = self.points.front()?;
        if time <= start {
            return None;
        }
        let ramp = (self.count.saturating_sub(1) as f64 / 3.0).min(1.0);
        Some(((self.distance - distance) / (time - start) / 1000.0 * ramp).clamp(0.0, 1.0) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::Sample;
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
    #[test]
    fn runtime_dynamics_reach_the_renderer_and_roundtrip_with_independent_curves() {
        let mut brush = crate::brush::Brush {
            size: 20.0,
            pressure_size: false,
            ..Default::default()
        };
        brush.dynamics.size = Some(Dynamics {
            minimum: 0.05,
            minimum_mode: MinimumMode::Clamp,
            inputs: vec![Effector {
                input: Input::Speed,
                curve: Curve {
                    points: vec![[0.0, 1.0], [1.0, 0.0]],
                },
            }],
        });
        brush.validate().unwrap();
        let brush: crate::brush::Brush =
            serde_json::from_slice(&serde_json::to_vec(&brush).unwrap()).unwrap();
        let footprint = |speed| {
            let mut doc = crate::document::Document::new(40, 40);
            doc.dab_sample_buffered(
                Sample {
                    x: 20.0,
                    y: 20.0,
                    pressure: 1.0,
                    speed,
                    ..Default::default()
                },
                &brush,
                [0, 0, 0],
                false,
                crate::symmetry::IDENTITY,
                None,
            );
            doc.layers[0]
                .pixels
                .region(40, [0, 0, 40, 40])
                .chunks_exact(4)
                .filter(|p| p[3] > 0)
                .count()
        };
        assert_eq!(footprint(None), footprint(Some(0.0)));
        assert!(footprint(Some(1.0)) < footprint(Some(0.0)) / 10);
    }
    #[test]
    fn descending_curves_and_minimum_modes_are_distinct_and_missing_speed_is_neutral() {
        let curve = Curve {
            points: vec![[0.0, 1.0], [0.3, 0.8], [1.0, 0.0]],
        };
        curve.validate().unwrap();
        assert!((curve.evaluate(0.65) - 0.4).abs() < 1e-6);
        let mut dynamic = Dynamics {
            minimum: 0.05,
            minimum_mode: MinimumMode::Clamp,
            ..Default::default()
        };
        let sample = Sample {
            pressure: 0.5,
            ..Default::default()
        };
        assert_eq!(dynamic.evaluate(&sample), 0.5);
        dynamic.minimum_mode = MinimumMode::Remap;
        assert_eq!(dynamic.evaluate(&sample), 0.525);
        dynamic.inputs.push(Effector {
            input: Input::Speed,
            curve,
        });
        assert_eq!(dynamic.evaluate(&sample), 0.525);
        assert_eq!(
            dynamic.evaluate(&Sample {
                speed: Some(1.0),
                ..sample
            }),
            0.05
        );
        dynamic.inputs.push(dynamic.inputs[0].clone());
        assert!(dynamic.validate().is_err());
        let invalid = Curve {
            points: vec![[0.0, 0.0], [0.0, 0.5], [1.0, 1.0]],
        };
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn velocity_is_zoom_invariant_and_rejects_unknown_or_nonmonotonic_time() {
        let mut reference = None;
        for zoom in [0.5, 1.0, 2.0] {
            let mut velocity = Velocity::default();
            let estimates: Vec<_> = (0..10)
                .map(|i| {
                    velocity.measure(
                        &Sample {
                            x: i as f32 * 4.0 / zoom,
                            time: Some(i as f64 * 0.01),
                            ..Default::default()
                        },
                        zoom,
                    )
                })
                .collect();
            assert!(estimates[0].is_none());
            assert!((estimates[9].unwrap() - 0.4).abs() < 1e-6);
            if let Some(reference) = &reference {
                assert_eq!(&estimates, reference);
            } else {
                reference = Some(estimates);
            }
            assert!(
                velocity
                    .measure(
                        &Sample {
                            time: Some(0.09),
                            ..Default::default()
                        },
                        zoom
                    )
                    .is_none()
            );
            assert!(velocity.measure(&Sample::default(), zoom).is_none());
            assert!(
                velocity
                    .measure(
                        &Sample {
                            time: Some(0.1),
                            ..Default::default()
                        },
                        zoom
                    )
                    .is_none()
            );
        }
    }
}
