use crate::{brush::Brush, document::Document, egui, stroke};
use std::hash::{Hash, Hasher};

pub const WIDTH: usize = 200;
pub const HEIGHT: usize = 40;

/// Rendered sample strokes for the brush library, re-rendered only when a
/// preset's settings change.
#[derive(Default)]
pub struct Previews {
    entries: std::collections::HashMap<usize, (u64, egui::TextureHandle)>,
}

impl Previews {
    pub fn texture(&mut self, ctx: &egui::Context, index: usize, brush: &Brush) -> egui::TextureId {
        let key = fingerprint(brush);
        let stale = self.entries.get(&index).is_none_or(|(k, _)| *k != key);
        if stale {
            let image = egui::ColorImage::from_rgba_unmultiplied([WIDTH, HEIGHT], &render(brush));
            if let Some(entry) = self.entries.get_mut(&index) {
                entry.0 = key;
                entry.1.set(image, egui::TextureOptions::LINEAR);
            } else {
                let texture =
                    ctx.load_texture("brush preview", image, egui::TextureOptions::LINEAR);
                self.entries.insert(index, (key, texture));
            }
        }
        self.entries[&index].1.id()
    }
    pub fn truncate(&mut self, len: usize) {
        self.entries.retain(|index, _| *index < len);
    }
}

/// Settings that affect appearance. Names and tags are excluded.
pub(crate) fn fingerprint(brush: &Brush) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hash_dynamics(&brush.dynamics, &mut hasher);
    brush.blend.hash(&mut hasher);
    brush.graph.is_some().hash(&mut hasher);
    if let Some(graph) = &brush.graph {
        graph.hash_into(&mut hasher);
    }
    brush.adjust_flow_by_spacing.hash(&mut hasher);
    brush.taper_in.to_bits().hash(&mut hasher);
    brush.taper_out.to_bits().hash(&mut hasher);
    for value in [
        brush.size,
        brush.opacity,
        brush.flow,
        brush.hardness,
        brush.spacing,
        brush.pressure_gamma,
        brush.aspect,
        brush.angle,
        brush.grain_amount,
        brush.grain_scale,
        brush.scatter,
    ]
    .into_iter()
    .chain(brush.pressure_curve)
    {
        value.to_bits().hash(&mut hasher);
    }
    (
        brush.pressure_size,
        brush.pressure_opacity,
        brush.custom_pressure,
        brush.scatter_seed,
        brush.tip as u8,
        brush.softness_profile as u8,
        brush.angle_source,
    )
        .hash(&mut hasher);
    brush.angle_jitter.to_bits().hash(&mut hasher);
    for image in [&brush.image, &brush.grain] {
        if let Some(image) = image {
            (image.width, image.height).hash(&mut hasher);
            image.alpha.fingerprint().hash(&mut hasher);
        } else {
            0u8.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Cache identity only: visit the settings without allocating or formatting JSON.
/// Include option/sequence boundaries so different curve structures stay distinct.
fn hash_dynamics(dynamics: &crate::dynamics::BrushDynamics, hasher: &mut impl Hasher) {
    for parameter in [
        &dynamics.size,
        &dynamics.opacity,
        &dynamics.flow,
        &dynamics.spacing,
    ] {
        parameter.is_some().hash(hasher);
        if let Some(parameter) = parameter {
            parameter.minimum.to_bits().hash(hasher);
            (parameter.minimum_mode as u8).hash(hasher);
            parameter.inputs.len().hash(hasher);
            for effector in &parameter.inputs {
                (effector.input as u8).hash(hasher);
                effector.curve.points.len().hash(hasher);
                for point in &effector.curve.points {
                    for coordinate in point {
                        coordinate.to_bits().hash(hasher);
                    }
                }
            }
        }
    }
}

/// A tapered S-curve: pressure rises and falls so pressure dynamics show.
pub fn render(brush: &Brush) -> Vec<u8> {
    let mut doc = Document::new(WIDTH, HEIGHT);
    let mut sample_brush = brush.clone();
    sample_brush.size = brush.size.min(HEIGHT as f32 * 0.6);
    sample_brush.stabilization = 0.0;
    sample_brush.stabilizer_mode = crate::brush::StabilizerMode::Smooth;
    let mut path = stroke::Stroke::default();
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    let settings = crate::stroke_buffer::Settings {
        target: crate::paint_target::Target::Raster,
        brush: sample_brush.clone(),
        color: [232, 236, 244],
        erase: false,
        layer: 0,
        symmetry: Default::default(),
    };
    if !brush.blend.is_paint() {
        crate::blend_brush::preview_stripes(&mut doc, settings.color, [0, WIDTH]);
    }
    let steps = 80;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let sample = stroke::Sample {
            x: 14.0 + t * (WIDTH as f32 - 28.0),
            y: HEIGHT as f32 / 2.0 - (t * std::f32::consts::TAU).sin() * HEIGHT as f32 * 0.2,
            pressure: (t * std::f32::consts::PI).sin().max(0.08),
            time: Some(i as f64 * 0.01),
            ..Default::default()
        };
        let dabs = if i == steps {
            path.finish(sample, &sample_brush)
        } else {
            path.sample(sample, &sample_brush)
        };
        for dab in dabs {
            buffer.paint(&mut doc, dab, &settings);
        }
    }
    buffer.finish_tail(&mut doc, &settings);
    std::mem::take(&mut doc.layers[0].pixels).into_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_preview_skips_unchanged_settings_and_refreshes_edited_dynamics() {
        let ctx = egui::Context::default();
        let mut previews = Previews::default();
        let mut brush = Brush::default();
        let first = ctx.run(Default::default(), |ctx| {
            previews.texture(ctx, 0, &brush);
        });
        let id = previews.entries[&0].1.id();
        assert!(
            first
                .textures_delta
                .set
                .iter()
                .any(|(texture, _)| *texture == id)
        );
        let unchanged = ctx.run(Default::default(), |ctx| {
            previews.texture(ctx, 0, &brush);
        });
        assert!(unchanged.textures_delta.set.is_empty());
        let original = render(&brush);
        brush.dynamics.opacity = Some(crate::dynamics::Dynamics {
            minimum: 0.0,
            inputs: vec![crate::dynamics::Effector {
                input: crate::dynamics::Input::Pressure,
                curve: crate::dynamics::Curve {
                    points: vec![[0.0, 0.0], [1.0, 0.25]],
                },
            }],
            ..Default::default()
        });
        let edited = ctx.run(Default::default(), |ctx| {
            previews.texture(ctx, 0, &brush);
        });
        assert_eq!(edited.textures_delta.set.len(), 1);
        assert_eq!(id, edited.textures_delta.set[0].0);
        assert_ne!(original, render(&brush));
        let before_curve_edit = render(&brush);
        brush.dynamics.opacity.as_mut().unwrap().inputs[0]
            .curve
            .points[1][1] = 0.75;
        let curve_edit = ctx.run(Default::default(), |ctx| {
            previews.texture(ctx, 0, &brush);
        });
        assert_eq!(curve_edit.textures_delta.set.len(), 1);
        assert_eq!(id, curve_edit.textures_delta.set[0].0);
        assert_ne!(before_curve_edit, render(&brush));
        let settled = ctx.run(Default::default(), |ctx| {
            previews.texture(ctx, 0, &brush);
        });
        assert!(settled.textures_delta.set.is_empty());
    }

    #[test]
    #[ignore = "manual component microbenchmark; no timing threshold in CI"]
    fn benchmark_dynamics_cache_key() {
        let dense = crate::dynamics::Dynamics {
            inputs: [
                crate::dynamics::Input::Pressure,
                crate::dynamics::Input::Speed,
                crate::dynamics::Input::TiltAltitude,
                crate::dynamics::Input::Random,
            ]
            .map(|input| crate::dynamics::Effector {
                input,
                curve: crate::dynamics::Curve {
                    points: (0..16)
                        .map(|i| {
                            let v = i as f32 / 15.0;
                            [v, v * v]
                        })
                        .collect(),
                },
            })
            .into(),
            ..Default::default()
        };
        let dense = crate::dynamics::BrushDynamics {
            size: Some(dense.clone()),
            opacity: Some(dense.clone()),
            flow: Some(dense.clone()),
            spacing: Some(dense),
        };
        for (name, settings) in [("default", Default::default()), ("dense", dense)] {
            settings.validate().unwrap();
            let iterations = 10_000;
            for round in 0..5 {
                let measure = |json: bool| {
                    let started = std::time::Instant::now();
                    for _ in 0..iterations {
                        let settings = std::hint::black_box(&settings);
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        if json {
                            serde_json::to_vec(settings).unwrap().hash(&mut hasher);
                        } else {
                            hash_dynamics(settings, &mut hasher);
                        }
                        std::hint::black_box(hasher.finish());
                    }
                    started.elapsed().as_secs_f64() * 1e9 / f64::from(iterations)
                };
                let (json_ns, direct_ns) = if round % 2 == 0 {
                    (measure(true), measure(false))
                } else {
                    let direct = measure(false);
                    (measure(true), direct)
                };
                println!(
                    "{{\"fixture\":\"{name}\",\"round\":{round},\"iterations\":{iterations},\"json_ns\":{json_ns},\"direct_ns\":{direct_ns}}}"
                );
            }
        }
    }

    #[test]
    fn previews_show_ink_and_fingerprints_ignore_names_but_not_settings() {
        let brush = Brush::default();
        let pixels = render(&brush);
        assert_eq!(pixels.len(), WIDTH * HEIGHT * 4);
        assert!(pixels.chunks_exact(4).any(|p| p[3] > 200));
        let renamed = Brush {
            name: "Other".into(),
            ..brush.clone()
        };
        assert_eq!(fingerprint(&brush), fingerprint(&renamed));
        let resized = Brush {
            size: 3.0,
            ..brush.clone()
        };
        assert_ne!(fingerprint(&brush), fingerprint(&resized));
    }
}
