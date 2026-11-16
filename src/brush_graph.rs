//! App side of `brush_graph`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::brush_graph`, re-exported
//! here so `crate::brush_graph::…` paths keep working.
pub(crate) use dudel_core::brush_graph::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(kinds: Vec<NodeKind>, links: &[(u32, u32, u8)]) -> BrushGraph {
        BrushGraph::new(
            kinds
                .into_iter()
                .enumerate()
                .map(|(i, kind)| Node {
                    id: NodeId(i as u32 + 1),
                    kind,
                })
                .collect(),
            links
                .iter()
                .map(|&(from, to, port)| Link {
                    from: NodeId(from),
                    to: NodeId(to),
                    port,
                    from_port: 0,
                })
                .collect(),
        )
    }

    /// Paint one recorded stroke with pressure, tilt and timing (so speed),
    /// through the stroke buffer as live drawing does. Smudge and blur
    /// brushes work on stripes.
    fn paint(brush: &crate::brush::Brush) -> Vec<u8> {
        use crate::stroke::{Sample, Stroke};
        let mut doc = crate::document::Document::new(320, 200);
        if brush.blend.moves_paint() {
            crate::blend_brush::preview_stripes(&mut doc, [200, 60, 40], [0, 320]);
        }
        let start = doc.layers[0].pixels.region(320, [0, 0, 320, 200]).to_vec();
        let mut stroke = Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [38, 44, 59],
            secondary: [220, 60, 40],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let count = 90;
        for i in 0..count {
            let t = i as f32 / (count - 1) as f32;
            // Uneven timing so speed rises and falls along the stroke.
            let time = f64::from(t) * 0.6 + f64::from((t * 9.0).sin()) * 0.02;
            let sample = Sample {
                x: 30.0 + t * 260.0,
                y: 100.0 + (t * std::f32::consts::TAU).sin() * 60.0,
                pressure: (t * std::f32::consts::PI).sin() * 0.9 + 0.05,
                time: Some(time),
                tilt: Some([60.0 * (1.0 - t) - 10.0, 25.0 * t]),
                twist: Some(t * 300.0),
                pressure_known: true,
                ..Default::default()
            };
            let mut dabs = stroke.sample_at_zoom(sample, brush, 1.0);
            if i + 1 == count {
                dabs.extend(stroke.finish_at_zoom(sample, brush, 1.0));
            }
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
        buffer.finish_tail(&mut doc, &settings);
        let end = doc.layers[0].pixels.region(320, [0, 0, 320, 200]).to_vec();
        assert!(end != start, "{} changed the layer", brush.name);
        end
    }

    #[test]
    fn a_node_knife_is_spaced_along_the_stroke_like_the_built_in_one() {
        use NodeKind::*;
        let knife = crate::workspace::default_brushes()
            .into_iter()
            .find(|b| b.name == "Palette knife")
            .map(|b| crate::brush::Brush { size: 300.0, ..b })
            .unwrap();
        // The same knife turned by nodes instead of the Direction setting.
        let noded = |graph: BrushGraph| crate::brush::Brush {
            angle_source: crate::brush::AngleSource::Fixed,
            graph: Some(graph),
            ..knife.clone()
        };
        let spacing = |brush: &crate::brush::Brush| {
            brush.dab_spacing(&brush.factors(&crate::stroke::Sample {
                pressure: 1.0,
                heading: Some(30.0),
                ..Default::default()
            }))
        };
        let follows = graph(
            vec![Direction, RotationOutput { degrees: 180.0 }],
            &[(1, 2, 0)],
        );
        assert!(
            follows
                .compiled()
                .unwrap()
                .turn_with_stroke()
                .unwrap()
                .abs()
                < 0.5
        );
        assert_eq!(spacing(&noded(follows)), spacing(&knife));
        assert!((spacing(&knife)[1] - 9.0).abs() < 0.01);
        // Turning by something besides the stroke is not spaced along it.
        for other in [
            // Direction with a random wobble added.
            graph(
                vec![
                    Direction,
                    Random { seed: 3 },
                    Constant { value: 0.1 },
                    Multiply,
                    Add,
                    RotationOutput { degrees: 180.0 },
                ],
                &[(2, 4, 0), (3, 4, 1), (1, 5, 0), (4, 5, 1), (5, 6, 0)],
            ),
            graph(
                vec![TiltDirection, RotationOutput { degrees: 180.0 }],
                &[(1, 2, 0)],
            ),
        ] {
            assert_eq!(other.compiled().unwrap().turn_with_stroke(), None);
            let [asked, used] = spacing(&noded(other));
            assert_eq!(asked, used);
        }
        // The same wobble at nothing still turns with the stroke: the probe
        // judges what the graph does, not which nodes it has.
        let still = graph(
            vec![
                Direction,
                Random { seed: 3 },
                Constant { value: 0.0 },
                Multiply,
                Add,
                RotationOutput { degrees: 180.0 },
            ],
            &[(2, 4, 0), (3, 4, 1), (1, 5, 0), (4, 5, 1), (5, 6, 0)],
        );
        assert!(still.compiled().unwrap().turn_with_stroke().is_some());
    }

    #[test]
    fn converted_brushes_paint_identical_pixels() {
        use crate::dynamics::{Curve, Dynamics, Effector, Input, MinimumMode};
        let curve = |points: Vec<[f32; 2]>| Curve { points };
        let mut brushes = crate::workspace::default_brushes();
        // Clear water paints nothing on a blank layer.
        brushes.retain(|b| b.pigment > 0.0);
        // Dynamics on every setting, both minimum modes, every input, and a
        // custom pressure curve.
        let mut dynamic = crate::brush::Brush {
            name: "All dynamics".into(),
            custom_pressure: true,
            pressure_curve: [0.0, 0.1, 0.6, 0.85, 1.0],
            pressure_opacity: true,
            adjust_flow_by_spacing: true,
            opacity: 0.8,
            flow: 0.7,
            taper_in: 30.0,
            taper_out: 25.0,
            ..Default::default()
        };
        dynamic.dynamics.size = Some(Dynamics {
            minimum: 0.1,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![
                Effector {
                    input: Input::Pressure,
                    curve: curve(vec![[0.0, 0.0], [0.4, 0.7], [1.0, 1.0]]),
                },
                Effector {
                    input: Input::TiltAltitude,
                    curve: curve(vec![[0.0, 0.3], [1.0, 1.0]]),
                },
            ],
        });
        dynamic.dynamics.flow = Some(Dynamics {
            minimum: 0.2,
            minimum_mode: MinimumMode::Clamp,
            inputs: vec![
                Effector {
                    input: Input::Speed,
                    curve: curve(vec![[0.0, 1.0], [1.0, 0.2]]),
                },
                Effector {
                    input: Input::Random,
                    curve: curve(vec![[0.0, 0.5], [1.0, 1.0]]),
                },
            ],
        });
        dynamic.dynamics.spacing = Some(Dynamics {
            minimum: 0.3,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![Effector {
                input: Input::Speed,
                curve: Curve::default(),
            }],
        });
        brushes.push(dynamic.clone());
        // Opacity dynamics replace the pressure toggle; no inputs at all.
        dynamic.dynamics.opacity = Some(Dynamics {
            minimum: 0.4,
            minimum_mode: MinimumMode::Remap,
            inputs: vec![],
        });
        dynamic.name = "Opacity without inputs".into();
        brushes.push(dynamic);
        let mut driven = 0;
        for brush in brushes {
            let graph = BrushGraph::from_brush(&brush);
            graph.validate().unwrap();
            driven += usize::from(graph.drives(Output::Size) || graph.drives(Output::Opacity));
            let converted = crate::brush::Brush {
                graph: Some(graph),
                ..brush.clone()
            };
            converted.validate().unwrap();
            let (before, after) = (paint(&brush), paint(&converted));
            assert!(before == after, "{} changed when converted", brush.name);
        }
        assert!(driven >= 15, "conversions drive real settings: {driven}");
    }

    #[test]
    fn packs_with_graphs_are_version_3_and_preferences_drop_damaged_graphs() {
        use crate::brush_pack::BrushPack;
        let brush = crate::brush::Brush {
            graph: Some(BrushGraph::from_brush(&crate::brush::Brush::default())),
            ..Default::default()
        };
        let pack = BrushPack::new(vec![brush.clone()]);
        assert_eq!(pack.version, 3);
        let json = serde_json::to_string(&pack).unwrap();
        let back: BrushPack = serde_json::from_str(&json).unwrap();
        back.validate().unwrap();
        assert!(back.brushes[0] == brush);
        let mut old = BrushPack::new(vec![brush.clone()]);
        old.version = 2;
        assert!(old.validate().is_err());
        assert_eq!(BrushPack::new(vec![Default::default()]).version, 2);

        // Preferences: a good graph round-trips, a damaged one is dropped.
        let app = crate::Dudel::default();
        let mut prefs =
            crate::preferences::Preferences::decode(&app.encode_preferences().unwrap()).unwrap();
        let mut damaged = brush.clone();
        damaged.graph.as_mut().unwrap().edit().1.push(Link {
            from: NodeId(999),
            to: NodeId(1),
            port: 0,
            from_port: 0,
        });
        prefs.brushes = vec![brush.clone(), damaged];
        let json = serde_json::to_string(&prefs).unwrap();
        let loaded = crate::preferences::Preferences::decode(&json).unwrap();
        assert_eq!(loaded.graphs_dropped, 1);
        assert!(loaded.brushes[0] == brush);
        assert!(loaded.brushes[1].graph.is_none());
    }
}
