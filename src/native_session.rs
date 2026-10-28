//! Isolated persistent native-window diagnostic. Never loads user preferences/artwork.
use crate::{
    Dudel, affine,
    document::{Document, History},
    egui,
};
use std::time::{Duration, Instant};

const CYCLES: usize = 16;
// Region-patch Undo may restore equal bytes in new storage. Allocation identity
// is checked separately for operations that promise to share untouched artwork.
fn equivalent_snapshot(expected: &Document, actual: &Document) -> Result<bool, String> {
    if expected.layers.len() != actual.layers.len() {
        return Ok(false);
    }
    let mut normalized = actual.clone();
    for ((expected, actual), normalized) in expected
        .layers
        .iter()
        .zip(&actual.layers)
        .zip(&mut normalized.layers)
    {
        if expected.pixels != actual.pixels
            || !crate::layer_mask::equivalent_state(&expected.mask, &actual.mask, None)?
        {
            return Ok(false);
        }
        normalized.pixels = expected.pixels.clone();
        normalized.mask = expected.mask.clone();
    }
    Ok(crate::document::same_snapshot(expected, &normalized))
}
macro_rules! command {
    ($session:ident,$call:expr) => {{
        let started = Instant::now();
        let result = $call;
        $session.command_ms += started.elapsed().as_secs_f64() * 1000.0;
        result
    }};
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
enum Phase {
    Warmup,
    BrushStart,
    BrushInput,
    BrushApplied,
    BrushUndone,
    BrushRedone,
    Start,
    Preview,
    Canceled,
    Applied,
    Undone,
    Redone,
    UndoAll,
    RedoAll,
    ReleasedHistory,
    Save,
    Reopen,
    Release,
    FinalIdle,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FrameSample {
    round: usize,
    cycle: usize,
    phase: Phase,
    callback_interval_ms: f64,
    ui_update_before_diagnostic_ms: f64,
    composite_ms: f64,
    texture_preparation_ms: f64,
    completed_display_worker_ms: f64,
    queued_display_bytes: usize,
    diagnostic_driver_ms: f64,
    diagnostic_app_command_ms: f64,
    diagnostic_previous_frame_log_ms: f64,
}

#[derive(serde::Serialize)]
struct CompletedJob {
    round: usize,
    operation: &'static str,
    command_to_editor_ready_ms: f64,
}

// Keep sampling storage fixed after startup. Retaining JSON Values here made
// diagnostic metadata grow during the workload being measured.
#[derive(serde::Serialize)]
struct Checkpoint {
    label: &'static str,
    cycle: usize,
    round: usize,
    elapsed_seconds: f64,
    process_memory: Option<crate::benchmark::ProcessMemory>,
    #[cfg(feature = "allocation-profile")]
    rust_allocations: Option<crate::allocation_profile::Snapshot>,
    #[cfg(feature = "allocation-profile")]
    gpu_allocations: Option<crate::gpu_profile::Snapshot>,
    #[cfg(feature = "allocation-profile")]
    memory_regions: crate::region_census::Snapshot,
    #[cfg(all(feature = "allocation-stack-profile", windows))]
    allocation_trace: crate::allocation_trace::Checkpoint,
    #[cfg(all(feature = "allocation-stack-profile", windows))]
    allocation_size_live_bytes: [usize; 32],
    #[cfg(not(feature = "allocation-profile"))]
    rust_allocations: Option<()>,
    tracked_live_bytes: usize,
    tracked_additional_bytes: usize,
    history_retained_bytes: usize,
    display_rgba_bytes: usize,
    display_prefix_bytes: usize,
    diagnostic_frame_capacity_bytes: usize,
    diagnostic_frame_used_bytes: usize,
    diagnostic_checkpoint_capacity_bytes: usize,
    diagnostic_checkpoint_used_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
enum Density {
    Sparse,
    Pattern,
    Noise,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
struct Options {
    rounds: usize,
    layers: usize,
    density: Density,
    /// Square canvas side: 2048 (default) or 4096 for the 4K canvas envelope.
    side: usize,
}
impl Options {
    fn parse(
        sustained: bool,
        rounds: Option<&str>,
        layers: Option<&str>,
        density: Option<&str>,
    ) -> Result<Self, String> {
        if !sustained {
            return Ok(Self {
                rounds: 1,
                layers: 4,
                density: Density::Sparse,
                side: 2048,
            });
        }
        fn number(
            value: Option<&str>,
            default: usize,
            min: usize,
            max: usize,
            name: &str,
        ) -> Result<usize, String> {
            let parsed = value
                .map_or(Ok(default), |value| value.parse::<usize>())
                .map_err(|_| format!("{name} must be an integer in {min}..={max}"))?;
            if !(min..=max).contains(&parsed) {
                return Err(format!("{name} must be in {min}..={max}"));
            }
            Ok(parsed)
        }
        Ok(Self {
            rounds: number(rounds, 12, 2, 240, "DUDEL_SOAK_ROUNDS")?,
            layers: number(layers, 4, 4, 32, "DUDEL_SOAK_LAYERS")?,
            density: match density.unwrap_or("sparse") {
                "sparse" => Density::Sparse,
                "pattern" => Density::Pattern,
                "noise" => Density::Noise,
                _ => return Err("DUDEL_SOAK_DENSITY must be sparse, pattern or noise".into()),
            },
            side: 2048,
        })
    }
    /// DUDEL_SOAK_SIDE selects a 4096-square canvas. Dense 4096 fixtures stay
    /// inside the unchanged 512 MiB payload budget, so they allow at most 7 layers.
    fn with_side(self, side: Option<&str>) -> Result<Self, String> {
        let side = match side.unwrap_or("2048") {
            "2048" => 2048,
            "4096" => 4096,
            _ => return Err("DUDEL_SOAK_SIDE must be 2048 or 4096".into()),
        };
        if side == 4096 && self.density != Density::Sparse && self.layers > 7 {
            return Err("Dense 4096-square fixtures support at most 7 layers".into());
        }
        Ok(Self { side, ..self })
    }
    fn checkpoint_slots(self) -> usize {
        self.rounds * (CYCLES + 7 + usize::from(self.rounds > 1))
    }
    fn paint_bounds(self, extra_layer: usize) -> [usize; 4] {
        if self.density == Density::Sparse {
            [
                (400 + extra_layer * 160) % (self.side - 300),
                (300 + extra_layer * 200) % (self.side - 600),
                300,
                600,
            ]
        } else {
            // A full 32-layer RGBA canvas already consumes 512 MiB. Reserve
            // 4 MiB for the dense mask by omitting two 256px tile rows from
            // the final raster. This exercises the actual supported boundary.
            let height = if self.side == 2048 && self.layers == 32 && extra_layer == 30 {
                1536
            } else {
                self.side
            };
            [0, 0, height, self.side]
        }
    }
    fn timeout_seconds(self) -> u64 {
        self.rounds as u64 * self.round_budget_seconds() + 600
    }
    fn round_budget_seconds(self) -> u64 {
        if self.density == Density::Sparse {
            120
        } else {
            300
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
struct RunPlan {
    minimum_seconds: u64,
    mixed_brushes: bool,
}
impl RunPlan {
    fn parse(
        sustained: bool,
        seconds: Option<&str>,
        brushes: Option<&str>,
    ) -> Result<Self, String> {
        if !sustained {
            return Ok(Self::default());
        }
        let minimum_seconds = seconds
            .unwrap_or("0")
            .parse::<u64>()
            .map_err(|_| "DUDEL_SOAK_MIN_SECONDS must be 0 or an integer in 60..=21600")?;
        if minimum_seconds != 0 && !(60..=21600).contains(&minimum_seconds) {
            return Err("DUDEL_SOAK_MIN_SECONDS must be 0 or in 60..=21600".into());
        }
        let mixed_brushes = match brushes.unwrap_or("controlled") {
            "controlled" => false,
            "mixed" => true,
            _ => return Err("DUDEL_SOAK_BRUSHES must be controlled or mixed".into()),
        };
        Ok(Self {
            minimum_seconds,
            mixed_brushes,
        })
    }
    fn minimum_rounds(self, options: Options) -> usize {
        options.rounds.max(if self.mixed_brushes { 3 } else { 1 })
    }
    fn round_limit(self, options: Options) -> usize {
        if self.minimum_seconds == 0 {
            self.minimum_rounds(options)
        } else {
            240
        }
    }
    fn timeout_seconds(self, options: Options) -> u64 {
        Options {
            rounds: self.minimum_rounds(options),
            ..options
        }
        .timeout_seconds()
        .max(self.minimum_seconds + options.round_budget_seconds() + 600)
    }
    fn checkpoint_slots(self, options: Options) -> usize {
        Options {
            rounds: self.round_limit(options),
            ..options
        }
        .checkpoint_slots()
    }
    fn needs_more(
        self,
        options: Options,
        completed: usize,
        elapsed: Duration,
    ) -> Result<bool, String> {
        if completed < self.minimum_rounds(options) {
            return Ok(true);
        }
        if elapsed >= Duration::from_secs(self.minimum_seconds) {
            return Ok(false);
        }
        if completed >= self.round_limit(options) {
            return Err("Minimum diagnostic duration not reached within 240 rounds".into());
        }
        Ok(true)
    }
    fn brushes(self) -> Result<Vec<crate::brush::Brush>, String> {
        use crate::brush::{AngleSource, Brush, Tip, TipImage};
        let brushes = if self.mixed_brushes {
            let defaults = crate::workspace::default_brushes();
            let builtin = |name: &str| {
                defaults
                    .iter()
                    .find(|brush| brush.name == name)
                    .cloned()
                    .ok_or_else(|| format!("Missing diagnostic brush preset: {name}"))
            };
            let texture = TipImage {
                width: 2,
                height: 2,
                alpha: (vec![255, 0, 128, 64]).into(),
            };
            vec![
                builtin("G-pen")?,
                builtin("Pencil")?,
                Brush {
                    name: "QA textured stamp".into(),
                    size: 20.0,
                    tip: Tip::Image,
                    image: Some(texture.clone()),
                    grain: Some(texture),
                    grain_amount: 0.45,
                    grain_scale: 8.0,
                    scatter_seed: 735,
                    angle_source: AngleSource::Direction,
                    angle_jitter: 12.0,
                    ..Default::default()
                },
            ]
        } else {
            vec![Brush {
                size: 24.0,
                hardness: 0.8,
                flow: 0.7,
                stabilization: 4.0,
                ..Default::default()
            }]
        };
        for brush in &brushes {
            brush.validate()?;
        }
        Ok(brushes)
    }
}

fn fixture(options: Options) -> Result<(Document, crate::document::LayerId), String> {
    let (mut doc, id) =
        crate::mask_transform_benchmark::fixture(options.side, options.side, "whole-dense")?;
    for i in 0..options.layers - 1 {
        doc.add_layer();
        let mut writer = doc.layers[i + 1].pixels.writer();
        let mut random = 0x1234_5678u32.wrapping_add(i as u32);
        let [top, left, height, width] = options.paint_bounds(i);
        for y in top..top + height {
            for x in left..left + width {
                let rgba = if options.density == Density::Noise {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    let mut bytes = random.to_le_bytes();
                    bytes[3] = 140;
                    bytes
                } else if options.density == Density::Pattern {
                    [
                        ((x / 8 + y / 8 + i * 13) % 256) as u8,
                        ((x / 16 + i * 47) % 256) as u8,
                        170,
                        140,
                    ]
                } else {
                    [((60 + i * 60) % 256) as u8, 80, 170, 140]
                };
                writer.rgba_mut(y * options.side + x).copy_from_slice(&rgba);
            }
        }
        if options.layers > 4 {
            doc.layers[i + 1].clipped = i % 4 == 0;
            if i % 4 == 1 {
                doc.layers[i + 1].blend = crate::blend::BlendMode::Multiply;
            }
        }
    }
    doc.active = 0;
    doc.validate()?;
    Ok((doc, id))
}

pub(crate) struct Session {
    phase: Phase,
    started: Instant,
    observed_started: Option<Instant>,
    changed: Instant,
    previous_frame: Instant,
    previous_frame_log_ms: f64,
    ready_frames: usize,
    cycle: usize,
    round: usize,
    rounds: usize,
    options: Options,
    plan: RunPlan,
    workload_brushes: Vec<crate::brush::Brush>,
    packets: Vec<crate::input::Packet>,
    brush_next: usize,
    brush_started: Instant,
    command_ms: f64,
    steps: usize,
    canceled_verified: bool,
    baseline: Document,
    source: Option<Document>,
    result: Option<Document>,
    frames: crate::native_frame_log::FrameLog<FrameSample>,
    job_started: Option<Instant>,
    completed_jobs: Vec<CompletedJob>,
    checkpoints: Vec<Checkpoint>,
    renderer: String,
    #[cfg(feature = "allocation-profile")]
    gpu_device: Option<eframe::wgpu::Device>,
    /// One preallocated default-heap census per round, taken at final idle.
    #[cfg(feature = "allocation-profile")]
    heap_walks: Vec<crate::region_census::HeapWalk>,
    #[cfg(feature = "allocation-profile")]
    heap_walks_used: usize,
    files: tempfile::TempDir,
}
impl Session {
    fn observed_elapsed(&self) -> Duration {
        self.observed_started
            .map_or(Duration::ZERO, |started| started.elapsed())
    }
    pub(crate) fn configure(
        app: &mut Dudel,
        renderer: String,
        sustained: bool,
    ) -> Result<Self, String> {
        let options = Options::parse(
            sustained,
            std::env::var("DUDEL_SOAK_ROUNDS").ok().as_deref(),
            std::env::var("DUDEL_SOAK_LAYERS").ok().as_deref(),
            std::env::var("DUDEL_SOAK_DENSITY").ok().as_deref(),
        )?
        .with_side(std::env::var("DUDEL_SOAK_SIDE").ok().as_deref())?;
        let plan = RunPlan::parse(
            sustained,
            std::env::var("DUDEL_SOAK_MIN_SECONDS").ok().as_deref(),
            std::env::var("DUDEL_SOAK_BRUSHES").ok().as_deref(),
        )?;
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        crate::allocation_trace::start();
        Self::configure_with_options(app, renderer, sustained, options, plan)
    }
    fn configure_with_options(
        app: &mut Dudel,
        renderer: String,
        sustained: bool,
        options: Options,
        plan: RunPlan,
    ) -> Result<Self, String> {
        let workload_brushes = plan.brushes()?;
        let (doc, id) = fixture(options)?;
        app.doc = doc;
        if sustained {
            app.symmetry = Default::default();
            app.snapping = false;
            app.guides_locked = true;
            app.brush = workload_brushes[0].clone();
            app.color = [186, 52, 142];
            app.hsv = crate::color_wheel::rgb_to_hsv(app.color);
        }
        app.history = History::new();
        app.mask_focus = Some((app.doc.layers[0].id, id));
        app.select_tool(crate::tools::Tool::Move);
        app.fit_view();
        app.changed();
        app.track_guide_changes();
        app.saved_guides = app.guides_record(&app.doc);
        app.modified = false;
        app.status = "Isolated native mask session diagnostic".into();
        let now = Instant::now();
        Ok(Self {
            phase: Phase::Warmup,
            started: now,
            observed_started: None,
            changed: now,
            previous_frame: now,
            previous_frame_log_ms: 0.0,
            ready_frames: 0,
            cycle: 0,
            round: 0,
            rounds: plan.minimum_rounds(options),
            options,
            plan,
            workload_brushes,
            packets: Vec::new(),
            brush_next: 0,
            brush_started: now,
            command_ms: 0.0,
            steps: 0,
            canceled_verified: false,
            baseline: app.doc.clone(),
            source: None,
            result: None,
            frames: crate::native_frame_log::FrameLog::new()?,
            job_started: None,
            completed_jobs: Vec::with_capacity(plan.round_limit(options) * 2),
            checkpoints: Vec::with_capacity(plan.checkpoint_slots(options)),
            renderer,
            #[cfg(feature = "allocation-profile")]
            gpu_device: None,
            #[cfg(feature = "allocation-profile")]
            heap_walks: (0..plan.round_limit(options))
                .map(|_| crate::region_census::HeapWalk::reserve())
                .collect(),
            #[cfg(feature = "allocation-profile")]
            heap_walks_used: 0,
            files: tempfile::tempdir().map_err(|e| e.to_string())?,
        })
    }
    pub(crate) fn take_packets(&mut self) -> Option<Vec<crate::input::Packet>> {
        (!self.packets.is_empty()).then(|| std::mem::take(&mut self.packets))
    }
    #[cfg(feature = "allocation-profile")]
    pub(crate) fn with_gpu_device(mut self, device: Option<eframe::wgpu::Device>) -> Self {
        self.gpu_device = device;
        self
    }
    fn action_count(&self) -> usize {
        CYCLES + usize::from(self.rounds > 1)
    }
    fn finish_job(&mut self, operation: &'static str) -> Result<(), String> {
        let started = self
            .job_started
            .take()
            .ok_or("Missing native job start timestamp")?;
        if self.completed_jobs.len() == self.completed_jobs.capacity() {
            return Err("Completed-job storage exceeded its fixed capacity".into());
        }
        self.completed_jobs.push(CompletedJob {
            round: self.round,
            operation,
            command_to_editor_ready_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
        Ok(())
    }
    fn next(&mut self, phase: Phase) {
        self.phase = phase;
        self.changed = Instant::now();
        self.ready_frames = 0;
    }
    fn checkpoint(&mut self, app: &Dudel, label: &'static str) -> Result<(), String> {
        if self.checkpoints.len() == self.checkpoints.capacity() {
            return Err("Native diagnostic exceeded its fixed checkpoint reserve".into());
        }
        let mut registry = crate::snapshot_memory::Registry::new(&app.doc);
        app.history.register_allocations(&mut registry);
        app.texture.register_snapshots(&mut registry);
        // Diagnostic reference ownership is included, rather than hidden as app memory.
        registry.add_document(&self.baseline);
        if let Some(source) = &self.source {
            registry.add_document(source);
        }
        if let Some(result) = &self.result {
            registry.add_document(result);
        }
        let display = app.texture.memory();
        #[cfg(feature = "allocation-profile")]
        let gpu_allocations = self
            .gpu_device
            .as_ref()
            .and_then(crate::gpu_profile::snapshot);
        #[cfg(feature = "allocation-profile")]
        let rust_allocations = Some(crate::allocation_profile::snapshot());
        #[cfg(not(feature = "allocation-profile"))]
        let rust_allocations: Option<()> = None;
        self.checkpoints.push(Checkpoint {
            label,
            cycle: self.cycle,
            round: self.round,
            elapsed_seconds: self.started.elapsed().as_secs_f64(),
            process_memory: crate::benchmark::process_memory(),
            rust_allocations,
            #[cfg(feature = "allocation-profile")]
            gpu_allocations,
            #[cfg(feature = "allocation-profile")]
            memory_regions: crate::region_census::snapshot(),
            #[cfg(all(feature = "allocation-stack-profile", windows))]
            allocation_trace: crate::allocation_trace::checkpoint(),
            #[cfg(all(feature = "allocation-stack-profile", windows))]
            allocation_size_live_bytes: crate::allocation_trace::live_sizes(),
            tracked_live_bytes: registry.live_bytes(),
            tracked_additional_bytes: registry.additional_bytes(),
            history_retained_bytes: app.history.retained_bytes(),
            display_rgba_bytes: display.texture_base_rgba_bytes,
            display_prefix_bytes: display.prefix_heap_bytes,
            diagnostic_frame_capacity_bytes: self.frames.capacity_bytes(),
            diagnostic_frame_used_bytes: self.frames.used_bytes(),
            diagnostic_checkpoint_capacity_bytes: self.checkpoints.capacity()
                * std::mem::size_of::<Checkpoint>(),
            diagnostic_checkpoint_used_bytes: self.checkpoints.len()
                * std::mem::size_of::<Checkpoint>(),
        });
        // Walked after the checkpoint's samples so the heap lock and walk
        // cannot perturb them.
        #[cfg(feature = "allocation-profile")]
        if label == "final-idle-history-released"
            && let Some(walk) = self.heap_walks.get_mut(self.heap_walks_used)
        {
            crate::region_census::walk_default_heap(walk, self.round);
            self.heap_walks_used += 1;
        }
        eprintln!(
            "Native session: {label}, round {}/{}, cycle {}/{} ({:.1}s)",
            self.round + 1,
            self.rounds,
            self.cycle,
            CYCLES,
            self.started.elapsed().as_secs_f64()
        );
        Ok(())
    }
    pub(crate) fn write_failure_report(
        &self,
        output: &std::path::Path,
        error: &str,
    ) -> Result<(), String> {
        crate::mask_transform_benchmark::write_report(
            &output.with_extension("incomplete.json"),
            &serde_json::json!({
                "version": 1, "complete": false, "release_gate_accepted": false,
                "error": error, "elapsed_seconds": self.started.elapsed().as_secs_f64(),
                "observed_session_seconds": self.observed_elapsed().as_secs_f64(),
                "planned_rounds": self.rounds, "last_round_zero_based": self.round,
                "phase": self.phase, "cycle": self.cycle,
                "completed_jobs": self.completed_jobs, "checkpoints": self.checkpoints,
                "frame_samples_recorded": self.frames.len(),
                "frame_staging_capacity_bytes": self.frames.capacity_bytes(),
                "frame_staging_used_bytes": self.frames.used_bytes(),
                "frame_storage": "Fixed buffered temporary JSONL; complete frames stream into the final report",
                "diagnostic_timeout_seconds": self.plan.timeout_seconds(self.options),
                "configured_minimum_rounds": self.options.rounds, "session_plan": self.plan,
                "workload_brushes": self.workload_brushes,
                "scope": "Incomplete diagnostic only. Preserves completed job and checkpoint evidence after an error; omits raw frame samples. Never a completed workload or release acceptance."
            }),
        )
    }
    pub(crate) fn update(&mut self, app: &mut Dudel, ctx: &egui::Context) -> Result<bool, String> {
        if self.started.elapsed() > Duration::from_secs(self.plan.timeout_seconds(self.options)) {
            return Err(format!("Timed out in {:?}: {}", self.phase, app.status));
        }
        let now = Instant::now();
        self.observed_started.get_or_insert(now);
        let interval = now.duration_since(self.previous_frame).as_secs_f64() * 1000.0;
        self.previous_frame = now;
        let p = &app.performance;
        let mut frame = FrameSample {
            round: self.round,
            cycle: self.cycle,
            phase: self.phase,
            callback_interval_ms: interval,
            ui_update_before_diagnostic_ms: p.frame_ms,
            composite_ms: p.composite_ms,
            texture_preparation_ms: p.upload_ms,
            completed_display_worker_ms: p.display_worker_ms,
            queued_display_bytes: p.display_queued_bytes,
            diagnostic_driver_ms: 0.0,
            diagnostic_app_command_ms: 0.0,
            diagnostic_previous_frame_log_ms: self.previous_frame_log_ms,
        };
        let settled = !app.dirty
            && !app.texture.busy()
            && !app.render_queue.busy()
            && app.thumbnail_revision == Some(app.revision)
            && app
                .affine_preview
                .as_ref()
                .is_none_or(|p| !p.needs_repaint());
        self.ready_frames = if settled { self.ready_frames + 1 } else { 0 };
        let driver = Instant::now();
        self.command_ms = 0.0;
        let done = self.step(app, ctx)?;
        frame.diagnostic_driver_ms = driver.elapsed().as_secs_f64() * 1000.0;
        frame.diagnostic_app_command_ms = self.command_ms;
        let logging = Instant::now();
        self.frames.push(&frame)?;
        self.previous_frame_log_ms = logging.elapsed().as_secs_f64() * 1000.0;
        if done {
            let observed_session_seconds = self.observed_elapsed().as_secs_f64();
            #[cfg(all(feature = "allocation-stack-profile", windows))]
            crate::allocation_trace::finish_to(
                &app.capture
                    .as_ref()
                    .unwrap()
                    .path
                    .with_extension("allocations.json"),
            )?;
            #[cfg(feature = "allocation-profile")]
            crate::mask_transform_benchmark::write_report(
                &app.capture
                    .as_ref()
                    .unwrap()
                    .path
                    .with_extension("heapwalk.json"),
                &serde_json::json!({
                    "scope": "Busy blocks of the default process heap at each round's final idle. Keys below 2^63 are a block's first word that pointed into a loaded image (usually a C++ vtable); others are 2^63 + exact block size. Rust and native blocks share this heap.",
                    "modules": crate::region_census::modules(),
                    "walks": &self.heap_walks[..self.heap_walks_used],
                }),
            )?;
            let mut completed_options = self.options;
            completed_options.rounds = self.rounds;
            let report = serde_json::json!({"version":3,"complete":true,"diagnostic_timeout_seconds":self.plan.timeout_seconds(self.options),"profile":if cfg!(debug_assertions){"debug"}else{"release"},
                "allocation_stack_profile_enabled":cfg!(all(feature="allocation-stack-profile",windows)),
                "allocation_stack_scope":"Optional Windows trace records requested blocks >=64 KiB in fixed atomic event storage. Sidecar .allocations.json contains raw stacks/local symbols and a visible dropped-event count. Checkpoints record event cuts and a 32-bin live size histogram: bin i covers [2^i,2^(i+1)), bin 31 includes all larger sizes. Histogram reads/cuts are concurrent samples; nonzero writers or incomplete realloc transactions make a cut unsuitable for exact ownership claims. Trace/static/report overhead is diagnostic, never baseline timing.",
                "allocation_profile_enabled":cfg!(feature="allocation-profile"),
                "gpu_allocation_profile_enabled":cfg!(feature="allocation-profile"),
                "gpu_allocation_profile_scope":"When supported, wgpu Device::generate_allocator_report supplies GPU suballocator allocated/reserved bytes and allocation/block counts. This excludes ownership outside that suballocator and is not total VRAM or Windows process commitment. Unsupported/unavailable reports are null. Temporary report vectors/names are dropped before sampling live Rust requests; their transient cost remains diagnostic overhead and lifetime peak may include it. No device polling or forced cleanup is performed.",
                "allocation_profile_scope":"Optional allocation-profile feature replaces the Rust global allocator with a System wrapper counting successful requested layout bytes. Live/peak are process-wide Rust requests, not heap backing, fragmentation, VRAM, or external C/driver allocations. Realloc accounts full old/new extents in cumulative released/acquired counts. Counters are sampled independently while workers may run. Profiling atomics affect timing; use uninstrumented builds for baseline responsiveness.",
                "canvas":[self.baseline.width,self.baseline.height],"layers":self.baseline.layers.len(),"fixture_options":completed_options,
                "configured_minimum_rounds":self.options.rounds,"session_plan":self.plan,"workload_brushes":self.workload_brushes,
                "observed_session_seconds":observed_session_seconds,
                "brush_schedule":"Generated stroke in round r uses workload_brushes[r % workload_brushes.len()]. G-pen/Pencil are unchanged built-ins; QA textured stamp is an explicit generated image/grain fixture, not a shipped preset.",
                "extra_layer_bounds":(0..self.options.layers-1).map(|i|self.options.paint_bounds(i)).collect::<Vec<_>>(),
                "initial_raster_and_layer_mask_payload_bytes":self.baseline.layers.iter().map(|l|l.pixels.payload_bound()+l.mask.as_ref().map_or(0,|m|m.coverage.payload_bytes())).sum::<usize>(),
                "masked_document_payload_limit_bytes":512*1024*1024,
                "completed_jobs":self.completed_jobs,
                "completed_job_capacity_bytes":self.completed_jobs.capacity()*std::mem::size_of::<CompletedJob>(),
                "completed_job_timing_scope":"Wall time from command invocation through worker completion and owner polling; Open also waits for display/thumbnail settlement and three ready frames. Measured before pixel/mask oracles. Conservative editor-ready duration, not isolated worker CPU time. Open immediately follows Save: warm OS cache; no cold-cache or physical-input claim.",
                "process_peak_scope":"Windows PROCESS_MEMORY_COUNTERS_EX PeakPagefileUsage/PeakWorkingSetSize at each checkpoint retain process-lifetime private-commit/working-set peaks through that checkpoint, including earlier transient jobs and fixed diagnostic storage. Not total VRAM or system memory. Final report/screenshot/shutdown allocations after the last checkpoint are outside the measured workload.",
                "cycles":CYCLES,"rounds":self.rounds,"generated_brush_samples_per_round":if self.rounds>1 {240}else{0},"renderer":self.renderer,
                "pixels_per_point":ctx.pixels_per_point(),"application_zoom":ctx.zoom_factor(),
                "viewport_points":[ctx.content_rect().width(),ctx.content_rect().height()],
                "elapsed_seconds":self.started.elapsed().as_secs_f64(),"checkpoints":self.checkpoints,
                "frame_samples_recorded":self.frames.len(),
                "diagnostic_final_frame_log_ms":self.previous_frame_log_ms,
                "frame_storage":"Every callback is buffered into temporary JSONL with fixed 64 KiB RAM staging, then streamed into this report without materializing the frame array. Checkpoint frame capacity/used bytes refer to RAM staging, not the growing disk file. Serialization and file-cache costs are diagnostic overhead. Temporary files are removed when the session closes. No sampling, dropped-frame allowance, callback-rate ceiling or count-based capacity assumption.",
                "verification":"Cancel restores clean source without history. Every committed mask compared with independent four-tap byte oracle; artwork allocations and mask flags unchanged. Per-edit Undo/Redo and full sequence Undo/Redo checked; asynchronous native save/reopen coverage and artwork compared, requiring fresh mask identity. History released before final idle.",
                "sustained_scope":"Long mode adds one pressure-varying stabilized brush stroke per round through the app canvas packet path, paced at 240 generated samples/second for one second. Target artwork must change, other artwork and mask coverage stay unchanged, and stroke Undo/Redo must match exact snapshots. Each round resets to the same retained fixture after save/reopen and final idle. Generated packets bypass OS/tablet delivery; they are not hardware validation. Frame RAM staging is fixed up front; all samples spool to temporary disk, and RAM used/capacity bytes are recorded separately. Verification/oracle work is diagnostic overhead, not ordinary UI cost.",
                "command_timing_scope":"diagnostic_app_command_ms times the instrumented preview start/apply/cancel, Undo/Redo, save and load app calls. It is included within diagnostic_driver_ms. Snapshot/oracle work, diagnostic bookkeeping and reset overhead remain separate from the UI prefix; neither metric is GPU time or end-to-end pen latency.",
                "scope":"One visible persistent native egui window with real renderer, canvas textures, thumbnails, navigator and accumulating history. Scripted commands, not physical input. UI update times stop before diagnostic driver/oracles; callback intervals include driver verification, OS scheduling and renderer work and are not GPU duration or pen latency. No vsync/FPS claim. Memory snapshots include fixed diagnostic baseline ownership; tracked counts exclude allocator/driver overhead. Temporary project file is isolated and removed when session closes.",
            });
            self.frames
                .write_report(&app.capture.as_ref().unwrap().path, &report)?;
            return Ok(true);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
        Ok(false)
    }
    fn step(&mut self, app: &mut Dudel, _ctx: &egui::Context) -> Result<bool, String> {
        if matches!(self.phase, Phase::BrushInput) {
            let due = ((self.brush_started.elapsed().as_secs_f64() * 240.0) as usize).clamp(1, 240);
            let view = app.view(app.canvas_area);
            for i in self.brush_next..due {
                let t = i as f32 / 239.0;
                let point = view.to_screen(egui::pos2(
                    600.0 + 800.0 * t,
                    650.0 + 400.0 * t + 120.0 * (t * std::f32::consts::TAU).sin(),
                ));
                let sample = crate::stroke::Sample {
                    x: point.x,
                    y: point.y,
                    pressure: 0.3 + 0.7 * (std::f32::consts::PI * t).sin(),
                    pressure_known: true,
                    time: Some(i as f64 / 240.0),
                    ..Default::default()
                };
                self.packets.push(if i == 0 {
                    crate::input::Packet::Start(sample)
                } else if i == 239 {
                    crate::input::Packet::End(sample)
                } else {
                    crate::input::Packet::Move(sample)
                });
            }
            self.brush_next = due;
            if due == 240 {
                self.next(Phase::BrushApplied);
            }
            return Ok(false);
        }
        if self.ready_frames < 3 {
            return Ok(false);
        }
        match self.phase {
            Phase::Warmup => {
                if self.changed.elapsed() < Duration::from_secs(2) {
                    return Ok(false);
                }
                self.checkpoint(app, "initial-idle")?;
                self.next(Phase::Start);
            }
            Phase::BrushStart => {
                app.brush = self.workload_brushes[self.round % self.workload_brushes.len()].clone();
                self.source = Some(app.doc.clone());
                app.focus_current_mask(false);
                app.select_tool(crate::tools::Tool::Brush);
                self.brush_next = 0;
                self.brush_started = Instant::now();
                self.next(Phase::BrushInput);
            }
            Phase::BrushInput => unreachable!(),
            Phase::BrushApplied => {
                let source = self.source.as_ref().unwrap();
                if app.doc.layers.len() != source.layers.len() {
                    return Err("Generated stroke changed layer count".into());
                }
                let mut metadata = app.doc.clone();
                metadata.layers[0].pixels = source.layers[0].pixels.clone();
                if app.doc.layers[0].pixels == source.layers[0].pixels
                    || !crate::document::same_snapshot(source, &metadata)
                    || app
                        .doc
                        .layers
                        .iter()
                        .skip(1)
                        .zip(source.layers.iter().skip(1))
                        .any(|(a, b)| !a.pixels.shares(&b.pixels))
                    || !app.doc.layers[0]
                        .mask
                        .as_ref()
                        .unwrap()
                        .coverage
                        .equivalent(&source.layers[0].mask.as_ref().unwrap().coverage, None)?
                {
                    return Err(
                        "Generated native brush stroke did not change only its target artwork"
                            .into(),
                    );
                }
                self.result = Some(app.doc.clone());
                command!(self, app.undo());
                self.next(Phase::BrushUndone);
            }
            Phase::BrushUndone => {
                if !equivalent_snapshot(self.source.as_ref().unwrap(), &app.doc)? {
                    return Err("Generated stroke Undo differs".into());
                }
                command!(self, app.redo());
                self.next(Phase::BrushRedone);
            }
            Phase::BrushRedone => {
                if !equivalent_snapshot(self.result.as_ref().unwrap(), &app.doc)? {
                    return Err("Generated stroke Redo differs".into());
                }
                self.source = None;
                self.result = None;
                self.checkpoint(app, "brush-undo-redo-settled")?;
                self.next(Phase::Start);
            }
            Phase::Start => {
                app.select_tool(crate::tools::Tool::Move);
                app.focus_current_mask(true);
                self.source = Some(app.doc.clone());
                app.affine_settings = affine::Settings {
                    translation: [17.5, -8.5],
                    sampling: affine::Sampling::Bilinear,
                    ..Default::default()
                };
                command!(self, app.start_affine_preview());
                if app.affine_preview.is_none() {
                    return Err(app.status.clone());
                }
                self.next(Phase::Preview);
            }
            Phase::Preview => {
                if !self.canceled_verified {
                    command!(self, app.cancel_affine_preview());
                    self.next(Phase::Canceled);
                    return Ok(false);
                }
                command!(self, app.finish_affine_preview());
                self.next(Phase::Applied);
            }
            Phase::Canceled => {
                if !crate::document::same_snapshot(self.source.as_ref().unwrap(), &app.doc)
                    || app.history.can_undo()
                    || app.modified
                {
                    return Err(
                        "Native canceled preview changed artwork/history/saved state".into(),
                    );
                }
                self.source = None;
                self.canceled_verified = true;
                self.checkpoint(app, "cancel-restored")?;
                self.next(if self.rounds > 1 {
                    Phase::BrushStart
                } else {
                    Phase::Start
                });
            }
            Phase::Applied => {
                crate::mask_transform_benchmark::verify_half(
                    self.source.as_ref().unwrap(),
                    &app.doc,
                )?;
                if app
                    .doc
                    .layers
                    .iter()
                    .zip(&self.source.as_ref().unwrap().layers)
                    .any(|(a, b)| !a.pixels.shares(&b.pixels))
                {
                    return Err("Native mask transform rewrote artwork".into());
                }
                self.result = Some(app.doc.clone());
                command!(self, app.undo());
                self.next(Phase::Undone);
            }
            Phase::Undone => {
                if !crate::document::same_snapshot(self.source.as_ref().unwrap(), &app.doc) {
                    return Err("Native per-edit Undo differs".into());
                }
                command!(self, app.redo());
                self.next(Phase::Redone);
            }
            Phase::Redone => {
                if !crate::document::same_snapshot(self.result.as_ref().unwrap(), &app.doc) {
                    return Err("Native per-edit Redo differs".into());
                }
                self.source = None;
                self.result = None;
                self.cycle += 1;
                self.checkpoint(app, "edit-undo-redo-settled")?;
                if self.cycle == CYCLES {
                    self.result = Some(app.doc.clone());
                    self.steps = 0;
                    self.next(Phase::UndoAll);
                } else {
                    self.next(Phase::Start);
                }
            }
            Phase::UndoAll => {
                if self.steps < self.action_count() {
                    command!(self, app.undo());
                    self.steps += 1;
                    self.ready_frames = 0;
                } else {
                    if app.history.can_undo() || !equivalent_snapshot(&self.baseline, &app.doc)? {
                        return Err("Native full Undo sequence differs".into());
                    }
                    self.checkpoint(app, "all-undone")?;
                    self.steps = 0;
                    self.next(Phase::RedoAll);
                }
            }
            Phase::RedoAll => {
                if self.steps < self.action_count() {
                    command!(self, app.redo());
                    self.steps += 1;
                    self.ready_frames = 0;
                } else {
                    if app.history.can_redo()
                        || !equivalent_snapshot(self.result.as_ref().unwrap(), &app.doc)?
                    {
                        return Err("Native full Redo sequence differs".into());
                    }
                    self.checkpoint(app, "all-redone")?;
                    app.history = History::new();
                    self.next(Phase::ReleasedHistory);
                }
            }
            Phase::ReleasedHistory => {
                if self.changed.elapsed() < Duration::from_secs(2) {
                    return Ok(false);
                }
                self.checkpoint(app, "history-released-before-save")?;
                app.path = Some(self.files.path().join("native-session.dudel"));
                self.job_started = Some(Instant::now());
                command!(self, app.save(false));
                self.next(Phase::Save);
            }
            Phase::Save => {
                if app.save_job.is_some() || app.queued_save.is_some() {
                    return Ok(false);
                }
                if app.modified || !app.path.as_ref().unwrap().is_file() {
                    return Err(format!("Native save failed: {}", app.status));
                }
                self.finish_job("Save")?;
                self.job_started = Some(Instant::now());
                command!(self, app.load_document(app.path.clone().unwrap(), false));
                self.next(Phase::Reopen);
            }
            Phase::Reopen => {
                if app.load_job.is_some() {
                    return Ok(false);
                }
                self.finish_job("Open")?;
                let expected = self.result.as_ref().unwrap();
                if app.modified
                    || app.doc.layers.len() != expected.layers.len()
                    || app.doc.width != expected.width
                    || app.doc.height != expected.height
                    || app.doc.layers[0].mask.as_ref().map(|m| m.id)
                        == expected.layers[0].mask.as_ref().map(|m| m.id)
                {
                    return Err("Native reopened document differs".into());
                }
                for (a, b) in app.doc.layers.iter().zip(&expected.layers) {
                    if a.pixels != b.pixels {
                        return Err("Native reopened artwork differs".into());
                    }
                    match (&a.mask, &b.mask) {
                        (Some(a), Some(b))
                            if a.enabled == b.enabled
                                && a.linked == b.linked
                                && a.coverage.equivalent(&b.coverage, None)? => {}
                        (None, None) => {}
                        _ => return Err("Native reopened mask differs".into()),
                    }
                }
                self.result = None;
                self.checkpoint(app, "save-reopen-verified")?;
                self.next(Phase::Release);
            }
            Phase::Release => {
                app.history = History::new();
                let id = app.doc.layers[0].mask.as_ref().unwrap().id;
                app.mask_focus = Some((app.doc.layers[0].id, id));
                app.status =
                    "Native diagnostic complete: save/reopen and Undo/Redo verified".into();
                self.next(Phase::FinalIdle);
            }
            Phase::FinalIdle => {
                if self.changed.elapsed() < Duration::from_secs(2) {
                    return Ok(false);
                }
                self.checkpoint(app, "final-idle-history-released")?;
                if self
                    .plan
                    .needs_more(self.options, self.round + 1, self.observed_elapsed())?
                {
                    self.rounds = self.rounds.max(self.round + 2);
                    app.apply_action(crate::document_actions::Action::Replace {
                        document: Box::new(self.baseline.clone()),
                        path: None,
                        modified: false,
                        status: "Next isolated diagnostic round",
                    });
                    let id = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.mask_focus = Some((app.doc.layers[0].id, id));
                    self.round += 1;
                    self.cycle = 0;
                    self.steps = 0;
                    self.canceled_verified = false;
                    self.next(Phase::Warmup);
                    return Ok(false);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duration_plan_requires_both_minima_and_reserves_the_full_round_limit() {
        let options = Options::parse(true, Some("31"), Some("32"), Some("noise")).unwrap();
        let plan = RunPlan::parse(true, Some("60"), Some("mixed")).unwrap();
        assert!(
            plan.needs_more(options, 30, Duration::from_secs(3600))
                .unwrap()
        );
        assert!(
            plan.needs_more(options, 31, Duration::from_secs(59))
                .unwrap()
        );
        assert!(
            !plan
                .needs_more(options, 31, Duration::from_secs(60))
                .unwrap()
        );
        assert!(
            plan.needs_more(options, 240, Duration::from_secs(59))
                .is_err()
        );
        assert_eq!(plan.round_limit(options), 240);
        assert_eq!(plan.checkpoint_slots(options), 240 * 24);
        for invalid in ["-1", "59", "21601", "invalid"] {
            assert!(RunPlan::parse(true, Some(invalid), None).is_err());
        }
        assert!(RunPlan::parse(true, None, Some("unknown")).is_err());
        assert_eq!(
            RunPlan::parse(false, Some("invalid"), Some("unknown")).unwrap(),
            RunPlan::default()
        );
        let legacy = RunPlan::default();
        assert_eq!(legacy.checkpoint_slots(options), options.checkpoint_slots());
        assert!(!legacy.needs_more(options, 31, Duration::ZERO).unwrap());
    }
    #[test]
    fn mixed_workload_keeps_real_builtin_presets_and_distinct_deterministic_texture() {
        let plan = RunPlan::parse(true, None, Some("mixed")).unwrap();
        let brushes = plan.brushes().unwrap();
        let short = Options::parse(true, Some("2"), None, None).unwrap();
        assert!(
            plan.needs_more(short, 2, Duration::from_secs(3600))
                .unwrap()
        );
        assert_eq!(plan.minimum_rounds(short), 3);
        assert_eq!(plan.round_limit(short), 3);
        assert_eq!(plan.checkpoint_slots(short), 3 * 24);
        let defaults = crate::workspace::default_brushes();
        for (index, name) in ["G-pen", "Pencil"].into_iter().enumerate() {
            assert!(brushes[index] == *defaults.iter().find(|brush| brush.name == name).unwrap());
        }
        assert!(brushes[2].image.is_some() && brushes[2].grain.is_some());
        let rendered: Vec<_> = brushes
            .iter()
            .map(|brush| crate::brush_studio::render(brush, [200, 72]))
            .collect();
        assert_ne!(rendered[0], rendered[1]);
        assert_ne!(rendered[0], rendered[2]);
        assert_ne!(rendered[1], rendered[2]);
        assert_eq!(
            rendered[2],
            crate::brush_studio::render(&brushes[2], [200, 72])
        );
    }
    #[test]
    fn failed_session_preserves_partial_evidence_without_publishing_a_completed_report() {
        let mut app = Dudel::default();
        let session = Session::configure_with_options(
            &mut app,
            "failure-report-test".into(),
            false,
            Options::parse(false, None, None, None).unwrap(),
            RunPlan::default(),
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let completed = directory.path().join("session.json");
        let incomplete = completed.with_extension("incomplete.json");
        session
            .write_failure_report(&completed, "forced diagnostic failure")
            .unwrap();
        assert!(!completed.exists());
        let bytes = std::fs::read(&incomplete).unwrap();
        let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["release_gate_accepted"], false);
        assert_eq!(report["error"], "forced diagnostic failure");
        assert_eq!(report["planned_rounds"], 1);
        assert!(report["completed_jobs"].as_array().unwrap().is_empty());
        assert!(report.get("frames").is_none());
        assert!(
            session
                .write_failure_report(&completed, "later failure")
                .is_err()
        );
        assert_eq!(std::fs::read(incomplete).unwrap(), bytes);
    }

    #[test]
    fn maximum_dense_geometry_reserves_the_mask_without_lowering_layer_count() {
        for density in [Density::Pattern, Density::Noise] {
            let options = Options {
                rounds: 30,
                layers: 32,
                density,
                side: 2048,
            };
            let raster = 2048 * 2048 * 4
                + (0..31)
                    .map(|i| {
                        let [_, _, height, width] = options.paint_bounds(i);
                        height * width * 4
                    })
                    .sum::<usize>();
            assert_eq!(raster + 2048 * 2048, 512 * 1024 * 1024);
            assert_eq!(options.paint_bounds(29), [0, 0, 2048, 2048]);
            assert_eq!(options.paint_bounds(30), [0, 0, 1536, 2048]);
        }
        let standard = Options {
            rounds: 30,
            layers: 20,
            density: Density::Noise,
            side: 2048,
        };
        assert_eq!(standard.paint_bounds(18), [0, 0, 2048, 2048]);
        let sparse = Options {
            layers: 32,
            density: Density::Sparse,
            ..standard
        };
        assert_eq!(sparse.paint_bounds(30)[2..], [300, 600]);
    }

    #[test]
    #[ignore = "512 MiB maximum masked/noisy fixture; run separately before dense acceptance"]
    fn maximum_masked_noise_fixture_is_valid_at_the_payload_boundary() {
        let options = Options {
            rounds: 30,
            layers: 32,
            density: Density::Noise,
            side: 2048,
        };
        let (doc, _) = fixture(options).unwrap();
        assert_eq!((doc.width, doc.height, doc.layers.len()), (2048, 2048, 32));
        let bytes: usize = doc
            .layers
            .iter()
            .map(|l| {
                l.pixels.payload_bound() + l.mask.as_ref().map_or(0, |m| m.coverage.payload_bytes())
            })
            .sum();
        assert_eq!(bytes, 512 * 1024 * 1024);
        assert_eq!(doc.layers[31].pixels.rgba(1535 * 2048)[3], 140);
        assert_eq!(doc.layers[31].pixels.rgba(1536 * 2048), [0; 4]);
        doc.validate().unwrap();
    }
    #[test]
    fn soak_options_bound_resources_and_preserve_legacy_fixture_defaults() {
        let default = Options::parse(true, None, None, None).unwrap();
        assert_eq!(
            default,
            Options {
                rounds: 12,
                layers: 4,
                density: Density::Sparse,
                side: 2048,
            }
        );
        assert_eq!(default.checkpoint_slots(), 288);
        let extended = Options::parse(true, Some("30"), Some("20"), Some("pattern")).unwrap();
        assert_eq!(
            (extended.rounds, extended.layers, extended.density),
            (30, 20, Density::Pattern)
        );
        assert_eq!(extended.checkpoint_slots(), 720);
        assert_eq!(extended.timeout_seconds(), 9600);
        assert_eq!(default.timeout_seconds(), 2040);
        assert!(Options::parse(true, Some("240"), Some("32"), Some("noise")).is_ok());
        let noise = Options::parse(true, Some("30"), Some("7"), Some("noise")).unwrap();
        let wide = noise.with_side(Some("4096")).unwrap();
        assert_eq!(wide.side, 4096);
        assert_eq!(wide.paint_bounds(5), [0, 0, 4096, 4096]);
        assert!(
            Options::parse(true, Some("30"), Some("8"), Some("noise"))
                .unwrap()
                .with_side(Some("4096"))
                .is_err()
        );
        assert!(default.with_side(Some("4096")).unwrap().paint_bounds(3)[0] < 4096 - 300);
        assert!(default.with_side(Some("3000")).is_err());
        assert_eq!(default.with_side(None).unwrap(), default);
        for invalid in ["0", "1", "241", "-1", "invalid"] {
            assert!(Options::parse(true, Some(invalid), None, None).is_err());
        }
        for invalid in ["0", "3", "33", "invalid"] {
            assert!(Options::parse(true, None, Some(invalid), None).is_err());
        }
        assert!(Options::parse(true, None, None, Some("unknown")).is_err());
        let short = Options::parse(false, Some("30"), Some("32"), Some("noise")).unwrap();
        assert_eq!(short.rounds, 1);
        assert_eq!(short.layers, 4);
        assert_eq!(short.checkpoint_slots(), 23);
    }

    #[test]
    fn snapshot_oracle_accepts_restored_storage_but_rejects_changed_bytes_and_metadata() {
        let mut source = Document::new(8, 8);
        source.version = 2;
        source.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[32, 84, 150, 255]);
        source.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: false,
            coverage: crate::layer_mask::Mask::from_coverage(8, 8, &[128; 64], None).unwrap(),
        });
        let mut actual = source.clone();
        actual.layers[0].pixels.writer().rgba_mut(0)[0] = 33;
        actual.layers[0].pixels.writer().rgba_mut(0)[0] = 32;
        actual.layers[0].mask.as_mut().unwrap().coverage =
            crate::layer_mask::Mask::from_coverage(8, 8, &[128; 64], None).unwrap();
        assert!(!crate::document::same_snapshot(&source, &actual));
        assert!(equivalent_snapshot(&source, &actual).unwrap());
        actual.layers[0].locked = true;
        assert!(!equivalent_snapshot(&source, &actual).unwrap());
        actual.layers[0].locked = false;
        actual.layers[0].pixels.writer().rgba_mut(0)[1] = 85;
        assert!(!equivalent_snapshot(&source, &actual).unwrap());
        actual.layers[0].pixels.writer().rgba_mut(0)[1] = 84;
        actual.layers[0].mask.as_mut().unwrap().enabled = false;
        assert!(!equivalent_snapshot(&source, &actual).unwrap());
        actual.layers[0].mask.as_mut().unwrap().enabled = true;
        let mut coverage = [128; 64];
        coverage[63] = 129;
        actual.layers[0].mask.as_mut().unwrap().coverage =
            crate::layer_mask::Mask::from_coverage(8, 8, &coverage, None).unwrap();
        assert!(!equivalent_snapshot(&source, &actual).unwrap());
    }
}
