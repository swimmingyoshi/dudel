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
pub enum Stage {
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

pub const STAGES: usize = 16;
pub const NAMES: [&str; STAGES] = [
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

pub static ON: AtomicBool = AtomicBool::new(false);
static NANOS: [AtomicU64; STAGES] = [const { AtomicU64::new(0) }; STAGES];
static CALLS: [AtomicU64; STAGES] = [const { AtomicU64::new(0) }; STAGES];

pub fn enable() {
    ON.store(true, Relaxed);
}

/// Times from here to the end of the enclosing block, when tracing is on.
pub struct Scope(Option<(Stage, Instant)>);

pub fn scope(stage: Stage) -> Scope {
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
pub fn take() -> [(f64, u64); STAGES] {
    std::array::from_fn(|i| {
        (
            NANOS[i].swap(0, Relaxed) as f64 / 1e6,
            CALLS[i].swap(0, Relaxed),
        )
    })
}

/// One frame of a traced session.
#[derive(Clone, serde::Serialize)]
pub struct Frame {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_has_a_name() {
        assert_eq!(Stage::Display as usize, STAGES - 1);
        assert_eq!(NAMES[Stage::Display as usize], "display");
        assert_eq!(NAMES[Stage::Raster as usize], "raster");
    }
}
