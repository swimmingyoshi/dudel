//! Named timers for finding where a stroke's time goes.
//!
//! Off unless a trace is asked for: `--profile-stroke REPORT.json` runs
//! scripted strokes without a window (`stroke_profile`), and the environment
//! variable `DUDEL_TRACE_STROKE=FILE` makes a normal session write one line
//! per frame of every stroke to FILE, after a line saying what the stroke
//! was painted with. While off, a timed section costs one atomic load.
//!
//! Stages are timed on the thread that asks for the work, so a stage that
//! fans out across cores counts its wall time once. `Pump` contains every
//! dab stage; what is left of it is queue and history bookkeeping. `Serial`
//! contains `SerialWater`.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Stage {
    /// Brush tables for one dab (`DabRaster::new`).
    Raster,
    /// The dab's coverage per pixel, in bands across cores; includes paper.
    Coverage,
    /// Copying the area a blending dab reads.
    Capture,
    /// Softening that copy (wet and blur dabs).
    Blur,
    /// Reading the water already under the dab.
    WaterRead,
    /// Blending a large dab into the layer, in bands across cores.
    Blend,
    /// A small blending dab: coverage, water and blending, pixel by pixel.
    Serial,
    /// The water reads and writes inside `Serial`.
    SerialWater,
    /// Writing the water the dab leaves.
    WaterWrite,
    /// Thick paint relief.
    Relief,
    /// A dab that lays down color without blending.
    Paint,
    /// Retiring an old dab's coverage behind a tapered stroke.
    Prefix,
    /// The whole per-frame painting pump.
    Pump,
    /// Paper surface tiles made for the view, a few per frame.
    PaperView,
    /// Layer effects and lit materials drawn for display.
    Expand,
    /// Compositing and preparing display textures.
    Display,
}

pub(crate) const STAGES: usize = 16;
pub(crate) const NAMES: [&str; STAGES] = [
    "raster",
    "coverage",
    "capture",
    "blur",
    "water_read",
    "blend",
    "serial",
    "serial_water",
    "water_write",
    "relief",
    "paint",
    "prefix",
    "pump",
    "paper_view",
    "expand",
    "display",
];

static ON: AtomicBool = AtomicBool::new(false);
static NANOS: [AtomicU64; STAGES] = [const { AtomicU64::new(0) }; STAGES];
static CALLS: [AtomicU64; STAGES] = [const { AtomicU64::new(0) }; STAGES];

pub(crate) fn enable() {
    ON.store(true, Relaxed);
}

/// Times from here to the end of the enclosing block, when tracing is on.
pub(crate) struct Scope(Option<(Stage, Instant)>);

pub(crate) fn scope(stage: Stage) -> Scope {
    Scope(ON.load(Relaxed).then(|| (stage, Instant::now())))
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some((stage, start)) = self.0 {
            NANOS[stage as usize].fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
            CALLS[stage as usize].fetch_add(1, Relaxed);
        }
    }
}

/// Milliseconds and calls per stage since the last call; resets them.
pub(crate) fn take() -> [(f64, u64); STAGES] {
    std::array::from_fn(|i| {
        (
            NANOS[i].swap(0, Relaxed) as f64 / 1e6,
            CALLS[i].swap(0, Relaxed),
        )
    })
}

/// One frame of a traced session.
#[derive(Clone, serde::Serialize)]
pub(crate) struct Frame {
    /// Milliseconds since the trace began.
    pub t_ms: f64,
    /// Since the previous frame began: above the CPU time when the frame
    /// waited on the display or the GPU.
    pub interval_ms: f64,
    /// CPU time of this frame's update.
    pub frame_ms: f64,
    pub stroking: bool,
    pub dabs: usize,
    /// Dabs asked for by the pen and not yet painted.
    pub queued: usize,
    pub composite_ms: f64,
    pub upload_ms: f64,
    pub display_worker_ms: f64,
    /// Milliseconds per stage, by name.
    pub stages: std::collections::BTreeMap<&'static str, f64>,
    /// Calls per stage, by name.
    pub calls: std::collections::BTreeMap<&'static str, u64>,
}

impl crate::Dudel {
    /// This frame's record; resets the stage timers.
    pub(crate) fn stage_frame(&self, t_ms: f64, interval_ms: f64, frame_ms: f64) -> Frame {
        let (mut stages, mut calls) = (
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::new(),
        );
        for (name, (ms, count)) in NAMES.iter().zip(take()) {
            if count > 0 {
                stages.insert(*name, (ms * 1000.0).round() / 1000.0);
                calls.insert(*name, count);
            }
        }
        Frame {
            t_ms,
            interval_ms,
            frame_ms,
            stroking: self.stroking,
            dabs: self.performance.dabs,
            queued: self.render_queue.queued_dabs(usize::MAX),
            composite_ms: self.performance.composite_ms,
            upload_ms: self.performance.upload_ms,
            display_worker_ms: self.performance.display_worker_ms,
            stages,
            calls,
        }
    }
}

/// What a traced stroke was painted with and on: the line before its first
/// frame. Without it a trace cannot tell a wet brush from a plain one that
/// was meant to be wet.
#[derive(serde::Serialize)]
struct Setup<'a> {
    tool: &'static str,
    brush: &'a str,
    mode: &'static str,
    size_px: f32,
    aspect: f32,
    angle: f32,
    spacing: f32,
    hardness: f32,
    opacity: f32,
    flow: f32,
    water: f32,
    pigment: f32,
    granulation: f32,
    paper_response: f32,
    pressure_size: bool,
    pressure_opacity: bool,
    canvas: [usize; 2],
    paper: bool,
    /// The active layer's material, or "Plain".
    layer: &'static str,
    zoom: f32,
}

impl crate::Dudel {
    fn stroke_setup(&self) -> Setup<'_> {
        let brush = &self.brush;
        Setup {
            tool: self.tool.name(),
            brush: &brush.name,
            mode: brush.blend.name(),
            size_px: brush.size,
            aspect: brush.aspect,
            angle: brush.angle,
            spacing: brush.spacing,
            hardness: brush.hardness,
            opacity: brush.opacity,
            flow: brush.flow,
            water: brush.wetness,
            pigment: brush.pigment,
            granulation: brush.granulation,
            paper_response: brush.paper_response,
            pressure_size: brush.pressure_size,
            pressure_opacity: brush.pressure_opacity,
            canvas: [self.doc.width, self.doc.height],
            paper: self.doc.paper.is_some(),
            layer: self
                .doc
                .layers
                .get(self.doc.active)
                .and_then(|layer| layer.material.as_ref())
                .map_or("Plain", |material| material.settings.kind.name()),
            zoom: self.zoom,
        }
    }
}

/// The trace file of a normal session (`DUDEL_TRACE_STROKE`).
struct Live {
    writer: std::io::BufWriter<std::fs::File>,
    began: Instant,
    previous: Instant,
    /// The last frame written had work, so the next idle frame flushes.
    active: bool,
}

static LIVE: std::sync::Mutex<Option<Live>> = std::sync::Mutex::new(None);

/// Start tracing to the file named by `DUDEL_TRACE_STROKE`, if it is set.
pub(crate) fn start_live_from_env() {
    let Some(path) = std::env::var_os("DUDEL_TRACE_STROKE").filter(|p| !p.is_empty()) else {
        return;
    };
    if let Err(error) = start_live(std::path::Path::new(&path)) {
        eprintln!("DUDEL_TRACE_STROKE: {error}");
    }
}

fn start_live(path: &std::path::Path) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let now = Instant::now();
    *LIVE.lock().unwrap() = Some(Live {
        writer: std::io::BufWriter::new(file),
        began: now,
        previous: now,
        active: false,
    });
    enable();
    Ok(())
}

impl crate::Dudel {
    /// End of a frame in a traced session: one JSON line per frame that
    /// painted or still has dabs waiting, and before a stroke's first frame
    /// a line with its `Setup` under "stroke".
    pub(crate) fn trace_live_frame(&self, frame_start: Instant) {
        if !ON.load(Relaxed) {
            return;
        }
        let mut guard = LIVE.lock().unwrap();
        let Some(live) = guard.as_mut() else {
            return;
        };
        use std::io::Write;
        let interval = frame_start.duration_since(live.previous).as_secs_f64() * 1000.0;
        live.previous = frame_start;
        let frame = self.stage_frame(
            frame_start.duration_since(live.began).as_secs_f64() * 1000.0,
            interval,
            self.performance.frame_ms,
        );
        if frame.stroking || frame.dabs > 0 || frame.queued > 0 {
            if !live.active {
                let setup = serde_json::json!({ "stroke": self.stroke_setup() });
                if serde_json::to_writer(&mut live.writer, &setup).is_ok() {
                    let _ = live.writer.write_all(b"\n");
                }
            }
            if serde_json::to_writer(&mut live.writer, &frame).is_ok() {
                let _ = live.writer.write_all(b"\n");
            }
            live.active = true;
        } else if std::mem::take(&mut live.active) {
            // The stroke is over: the file is complete up to here even if
            // the session ends badly.
            let _ = live.writer.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_has_a_name() {
        assert_eq!(Stage::Display as usize, STAGES - 1);
        assert_eq!(NAMES[Stage::Display as usize], "display");
        assert_eq!(NAMES[Stage::Raster as usize], "raster");
    }

    #[test]
    fn a_traced_session_writes_the_setup_then_a_line_per_stroke_frame_and_none_when_idle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trace.jsonl");
        start_live(&path).unwrap();
        let mut app = crate::Dudel::default();
        let start = Instant::now();
        // Idle: nothing is written.
        app.trace_live_frame(start);
        app.stroking = true;
        app.performance.dabs = 3;
        app.performance.frame_ms = 4.5;
        drop(scope(Stage::Blend));
        app.trace_live_frame(start + std::time::Duration::from_millis(16));
        // The stroke is over: the idle frame flushes and adds no line.
        app.stroking = false;
        app.performance.dabs = 0;
        app.trace_live_frame(start + std::time::Duration::from_millis(32));
        *LIVE.lock().unwrap() = None;
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        let setup: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(setup["stroke"]["mode"], "Paint");
        assert_eq!(setup["stroke"]["canvas"][0], app.doc.width);
        assert_eq!(setup["stroke"]["size_px"], f64::from(app.brush.size));
        assert_eq!(setup["stroke"]["layer"], "Plain");
        let frame: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(frame["dabs"], 3);
        assert_eq!(frame["stroking"], true);
        assert_eq!(frame["frame_ms"], 4.5);
        assert!((frame["interval_ms"].as_f64().unwrap() - 16.0).abs() < 1e-6);
        // Other tests may paint while tracing is on.
        assert!(frame["calls"]["blend"].as_u64().unwrap() >= 1);
    }
}
