//! Scripted tools: an extension's Rhai program turns pen samples into the
//! points of one or more lines ("tracks"), which the native brush draws.
//!
//! The script never sees or changes the document. Each track gets its own
//! native stroke sampler (spacing, stabilizer, pressure dynamics), and all
//! dabs go into the one ordinary stroke the brush would make, so undo, masks,
//! selections, locks, symmetry and cancellation behave exactly as for a
//! hand-drawn stroke. Every call runs under an operation limit, so a script
//! that loops forever fails the stroke instead of freezing the app.
use crate::{brush::Brush, stroke::Sample, stroke::Stroke};
use rhai::packages::{Package, StandardPackage};
use rhai::{AST, CallFnOptions, Dynamic, Engine, Map, Scope};
use std::collections::BTreeMap;
use std::rc::Rc;

pub(crate) const MAX_SOURCE: usize = 32 * 1024;
/// Separate lines one stroke may draw at once.
const MAX_TRACKS: usize = 64;
/// Points one call may return.
const MAX_POINTS: usize = 4096;
/// Script steps one call may take (a few milliseconds of work).
const MAX_OPERATIONS: u64 = 200_000;
/// Farther than this from the canvas a point is a script error.
const MAX_COORDINATE: f64 = 1.0e6;

fn engine() -> Engine {
    let mut engine = Engine::new_raw();
    StandardPackage::new().register_into_engine(&mut engine);
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    engine.set_max_operations(MAX_OPERATIONS);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_call_levels(16);
    engine.set_max_array_size(MAX_POINTS);
    engine.set_max_map_size(256);
    engine.set_max_string_size(4096);
    engine
}

/// Compile a tool script, which must define `point(x, y, pressure)` and may
/// define `begin` and `end` with the same parameters.
pub(crate) fn compile(source: &str) -> Result<Rc<AST>, String> {
    if source.len() > MAX_SOURCE {
        return Err("Tool script exceeds 32 KiB".into());
    }
    let ast = engine()
        .compile(source)
        .map_err(|e| format!("Invalid tool script: {e}"))?;
    let params = |name: &str| {
        ast.iter_functions()
            .filter(|f| f.name == name)
            .map(|f| f.params.len())
            .collect::<Vec<_>>()
    };
    if params("point") != [3] {
        return Err("A tool script must define point(x, y, pressure) once".into());
    }
    for name in ["begin", "end"] {
        if !params(name).iter().all(|n| *n == 3) || params(name).len() > 1 {
            return Err(format!(
                "{name} must take (x, y, pressure) and be defined once"
            ));
        }
    }
    Ok(Rc::new(ast))
}

/// The scripted tool in use: which extension command, and its program.
pub(crate) struct Active {
    pub package: String,
    pub command: String,
    pub label: String,
    pub ast: Rc<AST>,
}

/// What a script may read from `this` when the stroke starts.
pub(crate) struct Canvas {
    pub width: usize,
    pub height: usize,
    pub brush_size: f32,
    pub zoom: f32,
}

/// A point a script asked for.
struct Point {
    track: i64,
    x: f32,
    y: f32,
    pressure: f32,
    /// Ends the track's line here; its next point starts a new one.
    lift: bool,
}

#[derive(Default)]
struct Track {
    stroke: Stroke,
    last: Option<Sample>,
}

/// One stroke of a scripted tool: the script's state and its tracks.
pub(crate) struct ScriptedStroke {
    engine: Engine,
    ast: Rc<AST>,
    /// The script's `this`: a map kept for the whole stroke.
    state: Dynamic,
    tracks: BTreeMap<i64, Track>,
    began: bool,
}

impl ScriptedStroke {
    pub(crate) fn new(ast: Rc<AST>, canvas: Canvas) -> Self {
        let mut state = Map::new();
        state.insert("canvas_width".into(), (canvas.width as i64).into());
        state.insert("canvas_height".into(), (canvas.height as i64).into());
        state.insert("brush_size".into(), f64::from(canvas.brush_size).into());
        state.insert("zoom".into(), f64::from(canvas.zoom).into());
        Self {
            engine: engine(),
            ast,
            state: state.into(),
            tracks: BTreeMap::new(),
            began: false,
        }
    }

    /// Dabs for one pen sample (the first also calls `begin`).
    pub(crate) fn sample(
        &mut self,
        sample: Sample,
        brush: &Brush,
        zoom: f32,
    ) -> Result<Vec<Sample>, String> {
        let mut dabs = Vec::new();
        if !self.began {
            self.began = true;
            let points = self.call("begin", sample)?;
            self.draw(points, sample, brush, zoom, &mut dabs)?;
        }
        let points = self.call("point", sample)?;
        self.draw(points, sample, brush, zoom, &mut dabs)?;
        Ok(dabs)
    }

    /// Dabs for the lift: its sample, then `end`, then every open line
    /// finished where it last was.
    pub(crate) fn finish(
        &mut self,
        sample: Sample,
        brush: &Brush,
        zoom: f32,
    ) -> Result<Vec<Sample>, String> {
        let mut dabs = self.sample(sample, brush, zoom)?;
        let points = self.call("end", sample)?;
        self.draw(points, sample, brush, zoom, &mut dabs)?;
        for track in self.tracks.values_mut() {
            if let Some(last) = track.last.take() {
                dabs.extend(track.stroke.finish_at_zoom(last, brush, zoom));
                track.stroke.reset();
            }
        }
        Ok(dabs)
    }

    /// Whether any line's stabilizer trails its latest point.
    pub(crate) fn lagging(&self, tolerance: f32, brush: &Brush) -> bool {
        self.tracks
            .values()
            .any(|t| t.last.is_some() && t.stroke.lagging(tolerance, brush))
    }
    /// Stabilized lines eased toward their latest points.
    pub(crate) fn catch_up(&mut self, fraction: f32, brush: &Brush) -> Vec<Sample> {
        self.tracks
            .values_mut()
            .filter(|t| t.last.is_some())
            .flat_map(|t| t.stroke.catch_up(fraction, brush))
            .collect()
    }

    /// Run `name` (skipped when the script lacks an optional one) and read
    /// the points it returns.
    fn call(&mut self, name: &str, sample: Sample) -> Result<Vec<Point>, String> {
        if name != "point" && !self.ast.iter_functions().any(|f| f.name == name) {
            return Ok(Vec::new());
        }
        let options = CallFnOptions::new()
            .eval_ast(false)
            .rewind_scope(true)
            .bind_this_ptr(&mut self.state);
        let result: Dynamic = self
            .engine
            .call_fn_with_options(
                options,
                &mut Scope::new(),
                &self.ast,
                name,
                (
                    f64::from(sample.x),
                    f64::from(sample.y),
                    f64::from(sample.pressure),
                ),
            )
            .map_err(|e| format!("{name}: {e}"))?;
        points(result, sample.pressure)
    }

    fn draw(
        &mut self,
        points: Vec<Point>,
        base: Sample,
        brush: &Brush,
        zoom: f32,
        dabs: &mut Vec<Sample>,
    ) -> Result<(), String> {
        for point in points {
            if !self.tracks.contains_key(&point.track) && self.tracks.len() >= MAX_TRACKS {
                return Err(format!("A stroke may draw at most {MAX_TRACKS} lines"));
            }
            let track = self.tracks.entry(point.track).or_default();
            // Pen timing, tilt and buttons stay those of the real pen.
            let sample = Sample {
                x: point.x,
                y: point.y,
                pressure: point.pressure,
                distance: 0.0,
                ..base
            };
            if point.lift {
                dabs.extend(track.stroke.finish_at_zoom(sample, brush, zoom));
                track.stroke.reset();
                track.last = None;
            } else {
                dabs.extend(track.stroke.sample_at_zoom(sample, brush, zoom));
                track.last = Some(sample);
            }
        }
        Ok(())
    }
}

/// A script's result as points: nothing, or an array whose items are maps
/// (`x`, `y`, optional `pressure`, `track` and `lift`) or arrays
/// `[x, y]`, `[x, y, pressure]` or `[x, y, pressure, track]`.
fn points(result: Dynamic, pressure: f32) -> Result<Vec<Point>, String> {
    if result.is_unit() {
        return Ok(Vec::new());
    }
    let items = result
        .try_cast::<rhai::Array>()
        .ok_or("must return an array of points")?;
    if items.len() > MAX_POINTS {
        return Err(format!("at most {MAX_POINTS} points a call"));
    }
    let number = |value: &Dynamic| -> Result<f64, String> {
        let v = value
            .as_float()
            .or_else(|_| value.as_int().map(|i| i as f64))
            .map_err(|_| "point values must be numbers".to_string())?;
        if v.is_finite() && v.abs() <= MAX_COORDINATE {
            Ok(v)
        } else {
            Err("point values must be finite and near the canvas".into())
        }
    };
    let track = |value: &Dynamic| -> Result<i64, String> {
        value
            .as_int()
            .ok()
            .filter(|t| (0..1024).contains(t))
            .ok_or_else(|| "track must be a whole number from 0 to 1023".into())
    };
    items
        .into_iter()
        .map(|item| {
            let mut point = Point {
                track: 0,
                x: 0.0,
                y: 0.0,
                pressure,
                lift: false,
            };
            if let Some(map) = item.clone().try_cast::<Map>() {
                let get = |key: &str| map.get(key);
                point.x = number(get("x").ok_or("a point needs x")?)? as f32;
                point.y = number(get("y").ok_or("a point needs y")?)? as f32;
                if let Some(p) = get("pressure") {
                    point.pressure = number(p)? as f32;
                }
                if let Some(t) = get("track") {
                    point.track = track(t)?;
                }
                if let Some(lift) = get("lift") {
                    point.lift = lift.as_bool().map_err(|_| "lift must be true or false")?;
                }
            } else if let Some(values) = item.try_cast::<rhai::Array>() {
                if !(2..=4).contains(&values.len()) {
                    return Err("a point array is [x, y, pressure?, track?]".into());
                }
                point.x = number(&values[0])? as f32;
                point.y = number(&values[1])? as f32;
                if let Some(p) = values.get(2) {
                    point.pressure = number(p)? as f32;
                }
                if let Some(t) = values.get(3) {
                    point.track = track(t)?;
                }
            } else {
                return Err("each point must be a map or an array".into());
            }
            point.pressure = point.pressure.clamp(0.0, 1.0);
            Ok(point)
        })
        .collect()
}

impl crate::Dudel {
    /// Start the scripted tool's state for a stroke that just began, if one
    /// is in use and its extension is still enabled.
    pub(crate) fn begin_script_stroke(&mut self) {
        self.script_stroke = None;
        let Some(active) = &self.scripted_tool else {
            return;
        };
        let enabled = self
            .extensions
            .iter()
            .any(|e| e.enabled && e.package.id == active.package);
        if !enabled {
            self.status = format!("{} is no longer available", active.label);
            self.scripted_tool = None;
            return;
        }
        let brush_size = self
            .stroke_settings
            .as_ref()
            .map_or(self.brush.size, |s| s.brush.size);
        self.script_stroke = Some(ScriptedStroke::new(
            active.ast.clone(),
            Canvas {
                width: self.doc.width,
                height: self.doc.height,
                brush_size,
                zoom: self.zoom,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egui;

    fn canvas() -> Canvas {
        Canvas {
            width: 100,
            height: 100,
            brush_size: 4.0,
            zoom: 1.0,
        }
    }
    fn pen(x: f32, y: f32) -> Sample {
        Sample {
            x,
            y,
            pressure: 0.5,
            pressure_known: true,
            ..Default::default()
        }
    }

    #[test]
    fn scripts_need_point_and_well_formed_hooks() {
        assert!(compile("fn point(x, y, p) { [] }").is_ok());
        assert!(compile("fn begin(x, y, p) {} fn end(x, y, p) {}").is_err());
        assert!(compile("fn point(x, y) { [] }").is_err());
        assert!(compile("fn point(x, y, p) { [] } fn end(x) {}").is_err());
        assert!(compile("fn point(x, y, p) { [ }").is_err());
        assert!(
            compile(&format!(
                "fn point(x, y, p) {{ [] }} // {}",
                "x".repeat(MAX_SOURCE)
            ))
            .is_err()
        );
    }

    #[test]
    fn tracks_are_drawn_by_their_own_samplers_and_state_persists() {
        // Two parallel lines 20 px apart; `this` counts calls.
        let ast = compile(
            r#"
            fn begin(x, y, p) { this.calls = 0; }
            fn point(x, y, p) {
                this.calls += 1;
                [#{ x: x, y: y - 10.0 }, [x, y + 10.0, p, 1]]
            }
            "#,
        )
        .unwrap();
        let brush = Brush::default();
        let mut stroke = ScriptedStroke::new(ast, canvas());
        let mut dabs = Vec::new();
        for x in 0..=40 {
            dabs.extend(
                stroke
                    .sample(pen(10.0 + x as f32, 50.0), &brush, 1.0)
                    .unwrap(),
            );
        }
        dabs.extend(stroke.finish(pen(51.0, 50.0), &brush, 1.0).unwrap());
        assert!(dabs.iter().any(|d| (d.y - 40.0).abs() < 0.5));
        assert!(dabs.iter().any(|d| (d.y - 60.0).abs() < 0.5));
        assert!(
            dabs.iter()
                .all(|d| (d.y - 40.0).abs() < 0.5 || (d.y - 60.0).abs() < 0.5)
        );
        // Both lines reach the lift point.
        for y in [40.0, 60.0] {
            assert!(dabs.iter().any(|d| d.y == y && d.x > 50.0), "{y}");
        }
        let calls = stroke.state.clone_cast::<Map>()["calls"].as_int().unwrap();
        assert_eq!(calls, 42);
    }

    #[test]
    fn lifted_points_start_new_lines() {
        // Every call draws a separate 4 px tick at the pen.
        let ast =
            compile("fn point(x, y, p) { [#{ x: x, y: y }, #{ x: x, y: y + 4.0, lift: true }] }")
                .unwrap();
        let brush = Brush::default();
        let mut stroke = ScriptedStroke::new(ast, canvas());
        let mut dabs = stroke.sample(pen(10.0, 10.0), &brush, 1.0).unwrap();
        dabs.extend(stroke.sample(pen(30.0, 10.0), &brush, 1.0).unwrap());
        // No dab bridges the two ticks.
        assert!(!dabs.iter().any(|d| d.x > 11.0 && d.x < 29.0), "{dabs:?}");
        assert!(dabs.iter().any(|d| d.x == 30.0));
    }

    fn example() -> crate::extensions::Package {
        serde_json::from_str(include_str!(
            "../plugins/example-scripted-tools.dudelplugin"
        ))
        .unwrap()
    }

    #[test]
    fn every_example_tool_draws_along_a_curve() {
        let package = example();
        package.validate().unwrap();
        let brush = Brush {
            size: 6.0,
            ..Brush::default()
        };
        let mut tools = 0;
        for command in &package.contributes.commands {
            let crate::extension_api::Action::Tool {
                tool: crate::extension_api::ToolPreset::Script { source, .. },
            } = &command.action
            else {
                continue;
            };
            tools += 1;
            let mut stroke = ScriptedStroke::new(compile(source).unwrap(), canvas());
            let mut dabs = Vec::new();
            // A slow arc, then a fast jump across the canvas.
            for i in 0..60 {
                let a = i as f32 * 0.05;
                let sample = pen(50.0 + 30.0 * a.cos(), 50.0 + 30.0 * a.sin());
                dabs.extend(stroke.sample(sample, &brush, 1.0).expect(&command.id));
            }
            dabs.extend(
                stroke
                    .finish(pen(5.0, 5.0), &brush, 1.0)
                    .expect(&command.id),
            );
            assert!(dabs.len() > 20, "{}: {} dabs", command.id, dabs.len());
            assert!(
                dabs.iter().all(|d| d.x.is_finite() && d.y.is_finite()),
                "{}",
                command.id
            );
        }
        assert_eq!(tools, 4);
    }

    #[test]
    fn a_scripted_stroke_paints_its_lines_as_one_undo_step_and_stops_with_another_tool() {
        use crate::{input::Packet, tools::Tool};
        let ctx = egui::Context::default();
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(64, 64),
            brush: Brush {
                size: 2.0,
                hardness: 1.0,
                pressure_size: false,
                stabilization: 0.0,
                ..Brush::default()
            },
            zoom: 4.0,
            auto_fit: false,
            ..Default::default()
        };
        app.extensions.push(crate::extensions::Installed {
            package: example(),
            enabled: true,
        });
        app.run_extension_command("dudel-scripted-tools", "parallel");
        assert!(app.tool == Tool::Brush);
        assert!(app.scripted_tool.is_some(), "{}", app.status);
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let stroke = |app: &mut crate::Dudel| {
            let xs: Vec<f32> = (0..=12).map(|i| 8.0 + i as f32 * 4.0).collect();
            for (index, x) in xs.iter().enumerate() {
                let screen = app.view(app.canvas_area).to_screen(egui::pos2(*x, 32.0));
                let sample = Sample {
                    x: screen.x,
                    y: screen.y,
                    pressure: 1.0,
                    time: Some(index as f64 * 0.02),
                    ..Default::default()
                };
                let packet = match index {
                    0 => Packet::Start(sample),
                    i if i + 1 == xs.len() => Packet::End(sample),
                    _ => Packet::Move(sample),
                };
                let mut input = raw();
                input.events.push(egui::Event::PointerMoved(screen));
                let _ = ctx.run(input, |ctx| app.canvas(ctx, &[packet]));
            }
            app.flush_render_queue();
        };
        stroke(&mut app);
        let alpha = |app: &crate::Dudel, y: usize| app.doc.layers[0].pixels.rgba(y * 64 + 32)[3];
        // Rails 4 px apart (the gap is 1.5 brush sizes, at least 4 px).
        for y in [28, 32, 36] {
            assert!(alpha(&app, y) > 0, "rail at {y}");
        }
        assert_eq!(alpha(&app, 30), 0);
        assert_eq!(alpha(&app, 34), 0);
        app.undo();
        assert!(!app.history.can_undo(), "one stroke is one undo step");
        assert!((0..64 * 64).all(|i| app.doc.layers[0].pixels.rgba(i)[3] == 0));
        // Disabling the extension stops the tool at the next stroke, which
        // then draws as the plain brush.
        app.extensions[0].enabled = false;
        stroke(&mut app);
        assert!(app.scripted_tool.is_none());
        assert!(alpha(&app, 32) > 0 && alpha(&app, 28) == 0);
        // Choosing a tool ends a scripted tool.
        app.extensions[0].enabled = true;
        app.run_extension_command("dudel-scripted-tools", "coil");
        assert!(app.scripted_tool.is_some());
        app.select_tool(Tool::Brush);
        assert!(app.scripted_tool.is_none());
    }

    #[test]
    fn misbehaving_scripts_fail_the_call_not_the_app() {
        let brush = Brush::default();
        for source in [
            "fn point(x, y, p) { loop {} }",
            "fn point(x, y, p) { 42 }",
            "fn point(x, y, p) { [[x]] }",
            "fn point(x, y, p) { [[1.0 / 0.0, y]] }",
            "fn point(x, y, p) { [#{ x: x, y: y, track: 5000 }] }",
            "fn point(x, y, p) { let a = []; for i in 0..65 { a.push([x, y, p, i]); } a }",
            "fn point(x, y, p) { throw \"nope\"; }",
        ] {
            let mut stroke = ScriptedStroke::new(compile(source).unwrap(), canvas());
            assert!(
                stroke.sample(pen(1.0, 1.0), &brush, 1.0).is_err(),
                "{source}"
            );
        }
        // Returning nothing draws nothing.
        let mut quiet = ScriptedStroke::new(compile("fn point(x, y, p) {}").unwrap(), canvas());
        assert!(quiet.sample(pen(1.0, 1.0), &brush, 1.0).unwrap().is_empty());
        assert!(quiet.finish(pen(2.0, 1.0), &brush, 1.0).unwrap().is_empty());
    }
}
