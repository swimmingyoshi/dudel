//! Pixel-footprint coverage for analytic round and square tips.
use crate::brush::{SoftnessProfile, Tip};

pub fn compensated_delivery(delivery: f32, interval_ratio: f32) -> f32 {
    let delivery = delivery.clamp(0.0, 1.0);
    if delivery == 1.0 {
        return 1.0;
    }
    // log1p/expm1 preserve delivery below one output byte.
    -((-delivery).ln_1p() * interval_ratio.max(0.0)).exp_m1()
}

/// Used for delivery <= .95, where uniform interpolation is well conditioned.
/// High-flow brushes retain the exact path near the endpoint singularity.
pub struct FlowLookup([f32; 257]);
impl FlowLookup {
    pub fn new(max_delivery: f32, ratio: f32) -> Self {
        Self(std::array::from_fn(|index| {
            compensated_delivery(max_delivery * index as f32 / 256.0, ratio)
        }))
    }
    pub fn evaluate(&self, coverage: f32) -> f32 {
        let position = coverage.clamp(0.0, 1.0) * 256.0;
        let index = (position as usize).min(255);
        let t = position - index as f32;
        self.0[index] + (self.0[index + 1] - self.0[index]) * t
    }
}

/// Legacy's tiny-tip floor remains explicit to preserve existing preset pixels.
pub fn interior(distance: f32, hardness: f32, profile: SoftnessProfile, legacy_floor: f32) -> f32 {
    if distance >= 1.0 {
        return 0.0;
    }
    match profile {
        SoftnessProfile::LegacyLinear => {
            if distance <= hardness {
                1.0
            } else {
                ((1.0 - distance) / (1.0 - hardness).max(legacy_floor)).clamp(0.0, 1.0)
            }
        }
        SoftnessProfile::Eased => {
            if hardness >= 1.0 {
                return 1.0;
            }
            let core = 1.3 * hardness - 0.3;
            if distance <= core {
                return 1.0;
            }
            let t = ((distance - core) / (1.0 - core)).clamp(0.0, 1.0);
            if t < 0.5 {
                1.0 - 2.0 * t * t
            } else {
                2.0 * (1.0 - t).powi(2)
            }
        }
    }
}

#[cfg(test)]
pub fn coverage(u: f32, v: f32, radius: f32, aspect: f32, hardness: f32, tip: Tip) -> f32 {
    coverage_with_profile(
        u,
        v,
        radius,
        aspect,
        hardness,
        tip,
        SoftnessProfile::LegacyLinear,
    )
}

pub fn coverage_with_profile(
    u: f32,
    v: f32,
    radius: f32,
    aspect: f32,
    hardness: f32,
    tip: Tip,
    profile: SoftnessProfile,
) -> f32 {
    let distance = match tip {
        Tip::Round => (u * u + v * v).sqrt(),
        _ => u.abs().max(v.abs()),
    };
    let fringe = 0.5 / (radius * aspect);
    if distance <= 1.0 - fringe {
        return interior(distance, hardness, profile, 0.0);
    }
    if distance >= 1.0 + fringe {
        return 0.0;
    }
    let gradient = match tip {
        Tip::Round if distance > 0.0 => {
            ((u / radius).powi(2) + (v / (radius * aspect)).powi(2)).sqrt() / distance
        }
        Tip::Round => 1.0 / radius,
        _ if u.abs() > v.abs() => 1.0 / radius,
        _ => 1.0 / (radius * aspect),
    };
    let edge = (0.5 - (distance - 1.0) / gradient).clamp(0.0, 1.0);
    let edge = edge * edge * (3.0 - 2.0 * edge);
    let interior = interior(distance, hardness, profile, 0.0);
    // Hardness controls the interior softness, while the outer AA footprint
    // stays approximately one pixel wide even for large brushes.
    if hardness >= 1.0 {
        edge
    } else {
        edge * interior
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brush::Brush, document::Document};
    #[test]
    fn low_flow_lookup_stays_below_output_byte_error_against_direct_formula() {
        for delivery in [0.00001, 0.1, 0.5, 0.95] {
            for ratio in [0.02, 0.5, 2.5, 10.0] {
                let table = FlowLookup::new(delivery, ratio);
                for i in 0..=10000 {
                    let coverage = i as f32 / 10000.0;
                    let error = (table.evaluate(coverage)
                        - compensated_delivery(delivery * coverage, ratio))
                    .abs();
                    assert!(
                        error < 1.0 / 255.0,
                        "delivery {delivery}, ratio {ratio}, coverage {coverage}: {error}"
                    );
                }
            }
        }
    }
    #[test]
    fn spacing_compensation_preserves_soft_line_density_and_low_delivery() {
        assert!(compensated_delivery(0.00001, 0.5) > 0.0);
        let render = |spacing, adjust| {
            let brush = Brush {
                size: 30.0,
                pressure_size: false,
                flow: 0.1,
                hardness: 0.3,
                spacing,
                adjust_flow_by_spacing: adjust,
                ..Default::default()
            };
            let mut doc = Document::new(256, 64);
            let mut stroke = crate::stroke::Stroke::default();
            let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
            for x in 20..=236 {
                let sample = crate::stroke::Sample {
                    x: x as f32,
                    y: 32.0,
                    pressure: 1.0,
                    ..Default::default()
                };
                for dab in stroke.sample(sample, &brush) {
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
            (60..196)
                .map(|x| doc.layers[0].pixels.rgba(32 * 256 + x)[3] as f32 / 255.0)
                .sum::<f32>()
                / 136.0
        };
        let adjusted_difference = (render(0.05, true) - render(0.25, true)).abs();
        assert!(
            adjusted_difference < 0.03,
            "density difference {adjusted_difference}"
        );
        assert!((render(0.05, false) - render(0.25, false)).abs() > 0.2);
    }
    #[test]
    fn eased_profile_has_smooth_core_midpoint_and_rim() {
        let hardness = 0.5;
        let core = 1.3 * hardness - 0.3;
        let f = |r| interior(r, hardness, SoftnessProfile::Eased, 0.0);
        let step = 0.0001;
        assert_eq!(f(core), 1.0);
        assert!(f(core + step) > 0.99999);
        let middle = (core + 1.0) * 0.5;
        assert!((f(middle) - 0.5).abs() < 1e-6);
        let left_slope = (f(middle) - f(middle - step)) / step;
        let right_slope = (f(middle + step) - f(middle)) / step;
        assert!((left_slope - right_slope).abs() < 0.01);
        assert!(f(1.0 - step) < 1e-6);
        assert_eq!(f(1.0), 0.0);
        for hardness in [0.0, 0.2, 0.9, 1.0] {
            let values: Vec<_> = (0..=100)
                .map(|i| interior(i as f32 / 100.0, hardness, SoftnessProfile::Eased, 0.0))
                .collect();
            assert!(
                values
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            );
            assert!(values.windows(2).all(|v| v[0] >= v[1]));
        }
    }

    #[test]
    fn legacy_presets_keep_pixels_and_eased_profile_changes_soft_tips() {
        let current = Brush {
            size: 20.0,
            hardness: 0.5,
            pressure_size: false,
            ..Default::default()
        };
        let mut value = serde_json::to_value(&current).unwrap();
        for field in ["softness_profile", "stabilization_space"] {
            value.as_object_mut().unwrap().remove(field);
        }
        let legacy: Brush = serde_json::from_value(value).unwrap();
        let render = |brush: &Brush| {
            let mut doc = Document::new(40, 40);
            doc.dab(20.3, 20.7, 1.0, brush, [0, 0, 0], false);
            doc.layers[0].pixels.region(40, [0, 0, 40, 40])
        };
        assert_eq!(render(&current), render(&legacy));
        let eased = Brush {
            softness_profile: SoftnessProfile::Eased,
            ..current
        };
        assert_ne!(render(&eased), render(&legacy));
        let restored: Brush = serde_json::from_slice(&serde_json::to_vec(&eased).unwrap()).unwrap();
        assert_eq!(render(&restored), render(&eased));
    }
    #[test]
    fn flow_controls_dab_delivery_and_roundtrips_with_legacy_default() {
        use crate::{brush::Brush, document::Document};
        let mut brush = Brush {
            size: 10.0,
            hardness: 1.0,
            pressure_size: false,
            flow: 0.25,
            ..Brush::default()
        };
        let mut doc = Document::new(16, 16);
        doc.dab(8.5, 8.5, 1.0, &brush, [0, 0, 0], false);
        let index = (8 * 16 + 8) * 4 + 3;
        assert_eq!(doc.layers[0].pixels[index], 64);
        doc.dab(8.5, 8.5, 1.0, &brush, [0, 0, 0], false);
        assert_eq!(doc.layers[0].pixels[index], 112);
        let pixels = doc.layers[0].pixels.clone();
        brush.flow = 0.0;
        doc.dab(8.5, 8.5, 1.0, &brush, [255, 0, 0], false);
        assert_eq!(doc.layers[0].pixels, pixels);
        let mut value = serde_json::to_value(&brush).unwrap();
        let restored: Brush = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.flow, 0.0);
        value.as_object_mut().unwrap().remove("flow");
        assert_eq!(serde_json::from_value::<Brush>(value).unwrap().flow, 1.0);
        brush.flow = f32::NAN;
        assert!(brush.validate().is_err());
    }
    #[test]
    fn thin_diagonal_and_curved_replays_are_connected() {
        use crate::{
            brush::Brush,
            document::Document,
            stroke::{Sample, Stroke},
        };
        for tip in [Tip::Round, Tip::Square] {
            for size in [1.0, 2.0] {
                for phase in [0.0, 0.375, 0.75] {
                    let brush = Brush {
                        tip,
                        size,
                        hardness: 1.0,
                        pressure_size: false,
                        ..Brush::default()
                    };
                    let mut doc = Document::new(64, 64);
                    let mut stroke = Stroke::default();
                    let mut end = Sample {
                        x: 0.0,
                        y: 0.0,
                        pressure: 1.0,
                        ..Default::default()
                    };
                    for i in 0..17 {
                        let t = i as f32 / 16.0;
                        end = Sample {
                            x: 4.0 + 54.0 * t + phase,
                            y: 5.0 + 40.0 * t + (t * std::f32::consts::TAU).sin() * 7.0 + phase,
                            pressure: 1.0,
                            ..Default::default()
                        };
                        for dab in stroke.sample(end, &brush) {
                            doc.dab(dab.x, dab.y, dab.pressure, &brush, [0, 0, 0], false);
                        }
                    }
                    for dab in stroke.finish(end, &brush) {
                        doc.dab(dab.x, dab.y, dab.pressure, &brush, [0, 0, 0], false);
                    }
                    let painted: Vec<bool> = doc.layers[0]
                        .pixels
                        .chunks_exact(4)
                        .map(|p| p[3] > 0)
                        .collect();
                    let first = painted.iter().position(|value| *value).unwrap();
                    let mut visited = vec![false; 64 * 64];
                    let mut queue = std::collections::VecDeque::from([first]);
                    visited[first] = true;
                    while let Some(index) = queue.pop_front() {
                        let x = (index % 64) as i32;
                        let y = (index / 64) as i32;
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let (nx, ny) = (x + dx, y + dy);
                                if !(0..64).contains(&nx) || !(0..64).contains(&ny) {
                                    continue;
                                }
                                let next = ny as usize * 64 + nx as usize;
                                if painted[next] && !visited[next] {
                                    visited[next] = true;
                                    queue.push_back(next);
                                }
                            }
                        }
                    }
                    assert!(
                        painted.iter().zip(visited).all(|(p, v)| !p || v),
                        "disconnected {tip:?} size {size} phase {phase}"
                    );
                }
            }
        }
    }
    #[test]
    fn one_pixel_lines_cover_every_column_at_subpixel_offsets() {
        use crate::{
            brush::Brush,
            document::Document,
            stroke::{Sample, Stroke},
        };
        let brush = Brush {
            size: 1.0,
            hardness: 1.0,
            pressure_size: false,
            ..Brush::default()
        };
        for offset in [0.0, 0.125, 0.5, 0.875] {
            let mut doc = Document::new(64, 8);
            let mut stroke = Stroke::default();
            for x in [2.0, 14.0, 31.0, 61.0] {
                for dab in stroke.sample(
                    Sample {
                        x,
                        y: 3.0 + offset,
                        pressure: 1.0,
                        ..Default::default()
                    },
                    &brush,
                ) {
                    doc.dab(dab.x, dab.y, dab.pressure, &brush, [0, 0, 0], false);
                }
            }
            for x in 3..60 {
                assert!(
                    (0..8).any(|y| doc.layers[0].pixels[(y * 64 + x) * 4 + 3] > 0),
                    "gap at column {x}, offset {offset}"
                );
            }
        }
    }
    #[test]
    fn one_pixel_round_dab_approximates_circle_area_between_pixel_centers() {
        use crate::{brush::Brush, document::Document};
        let brush = Brush {
            size: 1.0,
            hardness: 1.0,
            pressure_size: false,
            ..Brush::default()
        };
        let mut doc = Document::new(4, 4);
        doc.dab(2.0, 2.0, 1.0, &brush, [0, 0, 0], false);
        let area: f32 = doc.layers[0]
            .pixels
            .chunks_exact(4)
            .map(|p| p[3] as f32 / 255.0)
            .sum();
        assert!(
            (area - std::f32::consts::PI / 4.0).abs() < 0.06,
            "area {area}"
        );
    }
    #[test]
    fn hard_edge_has_size_independent_pixel_falloff() {
        for radius in [1.0, 10.0, 100.0] {
            assert!((coverage(1.0, 0.0, radius, 1.0, 1.0, Tip::Round) - 0.5).abs() < 1e-6);
            assert_eq!(
                coverage(1.0 - 0.6 / radius, 0.0, radius, 1.0, 1.0, Tip::Round),
                1.0
            );
            assert_eq!(
                coverage(1.0 + 0.6 / radius, 0.0, radius, 1.0, 1.0, Tip::Round),
                0.0
            );
        }
    }
}
