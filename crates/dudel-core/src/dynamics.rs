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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
