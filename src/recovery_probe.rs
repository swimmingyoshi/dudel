//! Opt-in native recovery/restart evidence in an explicitly new disposable folder.
//! The controller terminates the producer; normal artwork recovery has no fault hook.
use crate::{Dudel, document::Document, egui, storage};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MARKER: &[u8] = b"Dudel isolated native artwork recovery v1";

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    use std::io::Write;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, value).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

fn record(app: &Dudel) -> Document {
    let mut doc = app.doc.clone();
    doc.guides = Some(Box::new(app.guides_record(&doc)));
    doc
}

fn equal(actual: &Document, expected: &Document) -> Result<(), String> {
    // Compare complete serialized state without constructing a serde Value per
    // channel (which amplifies a large fixture's diagnostic memory considerably).
    if serde_json::to_vec(actual).map_err(|e| e.to_string())?
        != serde_json::to_vec(expected).map_err(|e| e.to_string())?
    {
        return Err("Recovered editable state differs from the published snapshot".into());
    }
    Ok(())
}

fn revise(app: &mut Dudel, generation: u8) {
    app.doc.layers[0].name = format!("Recovery generation {generation}");
    app.doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[17, 83, 91 + generation, 0]);
    app.doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(app.doc.width * app.doc.height - 1)
        .copy_from_slice(&[91, 28, 147 + generation, 191]);
    app.changed();
}

fn fixture(masked: bool, large: bool) -> Dudel {
    let (width, height) = if large { (2048, 2048) } else { (65, 33) };
    let mut app = Dudel {
        doc: Document::new(width, height),
        ..Default::default()
    };
    app.doc.add_layer();
    if large {
        let mut noise = 735u32;
        for layer in &mut app.doc.layers {
            let bytes = (0..width * height * 4)
                .map(|_| {
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    noise as u8
                })
                .collect::<Vec<_>>();
            layer.pixels = bytes.into();
        }
    }
    app.doc.layers[1].opacity = 0.4;
    app.doc.layers[1].clipped = true;
    if masked {
        app.doc.version = 2;
        app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: false,
            coverage: crate::layer_mask::Mask::from_coverage(
                width,
                height,
                &vec![73; width * height],
                None,
            )
            .unwrap(),
        });
    }
    app.doc.active = 0;
    app.fit_view();
    app
}

#[cfg(windows)]
fn locked_destination(path: &Path, original: &[u8]) -> Result<std::fs::File, String> {
    use std::{io::Write, os::windows::fs::OpenOptionsExt};
    let mut initial = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    initial.write_all(original).map_err(|e| e.to_string())?;
    initial.sync_all().map_err(|e| e.to_string())?;
    drop(initial);
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn locked_destination(_: &Path, _: &[u8]) -> Result<std::fs::File, String> {
    Err("The sharing-lock save probe requires Windows".into())
}

struct Probe {
    app: Dudel,
    root: PathBuf,
    restore: bool,
    generation: u8,
    step: u8,
    frames: usize,
    started: Instant,
    edited: Instant,
    cadence: Vec<f64>,
    busy_adjustment: bool,
    busy_preview_seen: bool,
    deferred_frames: usize,
    busy_released_seconds: Option<f64>,
    save_sharing_failure: bool,
    save_blocker: Option<std::fs::File>,
    save_failure_verified: bool,
    save_failure_status: Option<String>,
    events: VecDeque<Vec<egui::Event>>,
    screenshot: Option<PathBuf>,
    captured: bool,
    snapshot_bytes: Vec<u8>,
    original_bytes: Vec<u8>,
    error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl Probe {
    fn image(&mut self, ctx: &egui::Context, name: &str) {
        self.screenshot = Some(self.root.join(name));
        self.captured = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    fn tick(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) -> Result<(), String> {
        if self.started.elapsed() > Duration::from_secs(240) {
            return Err("Native recovery probe timed out".into());
        }
        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        }) && let Some(path) = self.screenshot.take()
        {
            let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer_with_format(
                path,
                &bytes,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .map_err(|e| e.to_string())?;
            self.captured = true;
        }
        if !self.restore && self.busy_adjustment && self.busy_released_seconds.is_none() {
            if self.frames == 0 {
                self.app
                    .start_adjustment(crate::adjustments::Kind::BrightnessContrast);
                self.app
                    .adjustment
                    .as_mut()
                    .ok_or("Could not start busy adjustment")?
                    .settings = crate::adjustments::Settings::BrightnessContrast {
                    brightness: 35.0,
                    contrast: 0.0,
                };
            }
            if self.edited.elapsed() >= Duration::from_secs(20) {
                if !self.busy_preview_seen || self.deferred_frames == 0 {
                    return Err(
                        "Busy fixture did not observe a live preview beyond recovery eligibility"
                            .into(),
                    );
                }
                self.app.cancel_adjustment();
                self.busy_released_seconds = Some(self.edited.elapsed().as_secs_f64());
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        eframe::App::update(&mut self.app, ctx, frame);
        self.frames += 1;
        if self.restore {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        if !self.restore {
            if self.busy_adjustment && self.busy_released_seconds.is_none() {
                let last = self.app.doc.width * self.app.doc.height - 1;
                self.busy_preview_seen |= self.app.adjustment.as_ref().is_some_and(|p| p.ready())
                    && self.app.doc.layers[0].pixels.rgba(last)[0] != 91;
                if self.edited.elapsed() >= Duration::from_secs(15) {
                    if self.app.recovery_job.is_some()
                        || self.app.recovery_revision == self.app.revision
                        || self.app.recovery_path.as_ref().is_some_and(|p| p.exists())
                    {
                        return Err("Recovery published a temporary adjustment preview".into());
                    }
                    self.deferred_frames += 1;
                }
                return Ok(());
            }
            if self.step == 0
                && self.app.recovery_job.is_none()
                && self.app.recovery_revision == self.app.revision
            {
                let path = self
                    .app
                    .recovery_path
                    .clone()
                    .ok_or("Missing recovery pathname")?;
                let observed_completion_seconds = self.edited.elapsed().as_secs_f64();
                let recovered = storage::read_recovery(&path)?;
                equal(&recovered.document, &record(&self.app))?;
                if recovered.original_path != self.app.path {
                    return Err("Recovery lost the original project path".into());
                }
                self.cadence.push(observed_completion_seconds);
                if self.generation == 1 {
                    self.generation = 2;
                    revise(&mut self.app, 2);
                    self.edited = Instant::now();
                } else {
                    write_json(&self.root.join("expected.json"), &recovered.document)?;
                    // Simulate edits newer than the last completed checkpoint.
                    // Restart must recover generation 2, never invent generation 3.
                    revise(&mut self.app, 3);
                    write_json(
                        &self.root.join("write-ready.json"),
                        &serde_json::json!({
                            "pid": std::process::id(), "snapshot": path,
                            "generations_verified": 2, "seconds_from_edit_to_completion": self.cadence,
                            "normal_15_second_timer": true, "native_app_update": true,
                            "uncheckpointed_generation": 3, "expected_restore_generation": 2,
                            "producer_uses_normal_idle_repaints": !self.busy_adjustment,
                            "busy_phase_requests_100ms_repaints": self.busy_adjustment,
                            "canvas": [self.app.doc.width, self.app.doc.height], "raster_layers": 2,
                            "busy_adjustment": self.busy_adjustment,
                            "busy_preview_seen": self.busy_preview_seen,
                            "eligible_frames_deferred": self.deferred_frames,
                            "busy_released_seconds": self.busy_released_seconds,
                            "completion_timing_excludes_verification": true,
                        }),
                    )?;
                    self.step = 1; // Remain alive until the controller kills this process.
                }
            }
            return Ok(());
        }
        match self.step {
            0 if self.frames >= 12 => {
                if !self.app.recovery_window || self.app.recovery_candidates.len() != 1 {
                    return Err(
                        "Startup did not offer exactly the interrupted session's snapshot".into(),
                    );
                }
                self.image(ctx, "recovery-dialog.png");
                self.step = 1;
            }
            1 if self.captured => {
                let at = text(ctx, "Restore copy").ok_or("Restore copy is not visibly rendered")?;
                for pressed in [true, false] {
                    self.events.push_back(vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ]);
                }
                self.step = 2;
            }
            2 if self.app.load_job.is_none()
                && self.app.status.starts_with("Recovered painting") =>
            {
                if !self.app.modified || self.app.path.is_some() {
                    return Err("Recovery must open an unsaved copy".into());
                }
                let expected: Document = serde_json::from_slice(
                    &std::fs::read(self.root.join("expected.json")).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                equal(&record(&self.app), &expected)?;
                if self.save_sharing_failure {
                    self.save_blocker = Some(locked_destination(
                        &self.root.join("restored-copy.dudel"),
                        &self.original_bytes,
                    )?);
                }
                self.app.start_save_snapshot(
                    self.root.join("restored-copy.dudel"),
                    self.app.doc.clone(),
                );
                self.step = if self.save_sharing_failure { 20 } else { 3 };
            }
            20 if self.app.save_job.is_none() => {
                use std::io::Read;
                if !self.app.status.starts_with("Save failed:")
                    || !self.app.modified
                    || self.app.path.is_some()
                {
                    return Err(format!(
                        "Locked save did not preserve the unsaved document: {}",
                        self.app.status
                    ));
                }
                let mut preserved = Vec::new();
                self.save_blocker
                    .as_mut()
                    .ok_or("Missing sharing-lock handle")?
                    .read_to_end(&mut preserved)
                    .map_err(|e| e.to_string())?;
                if preserved != self.original_bytes {
                    return Err("Failed save changed the existing destination".into());
                }
                let expected: Document = serde_json::from_slice(
                    &std::fs::read(self.root.join("expected.json")).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                equal(&record(&self.app), &expected)?;
                self.save_failure_verified = true;
                self.save_failure_status = Some(self.app.status.clone());
                self.image(ctx, "save-sharing-failure.png");
                self.step = 21;
            }
            21 if self.captured => {
                self.save_blocker.take();
                self.app.start_save_snapshot(
                    self.root.join("restored-copy.dudel"),
                    self.app.doc.clone(),
                );
                self.step = 3;
            }
            3 if self.app.save_job.is_none() => {
                if self.app.modified
                    || self.app.path.as_deref()
                        != Some(self.root.join("restored-copy.dudel").as_path())
                {
                    return Err(format!(
                        "Recovered copy save did not commit: {}",
                        self.app.status
                    ));
                }
                equal(
                    &storage::load_project(&self.root.join("restored-copy.dudel"))?,
                    &record(&self.app),
                )?;
                if std::fs::read(self.root.join("original.dudel")).map_err(|e| e.to_string())?
                    != self.original_bytes
                    || std::fs::read(&self.app.recovery_candidates[0]).map_err(|e| e.to_string())?
                        != self.snapshot_bytes
                {
                    return Err(
                        "Restoring/saving changed the original project or interrupted snapshot"
                            .into(),
                    );
                }
                self.image(ctx, "restored-saved.png");
                self.step = 4;
            }
            4 if self.captured => {
                write_json(
                    &self.root.join("restore.json"),
                    &serde_json::json!({
                        "passed": true, "native_app_update": true, "startup_discovery": true,
                        "restore_copy_egui_hit_test": true, "full_editable_state_equal": true,
                        "original_and_snapshot_unchanged": true, "separate_copy_save_reopen": true,
                        "save_sharing_failure_requested": self.save_sharing_failure,
                        "save_failure_verified": self.save_failure_verified,
                        "save_failure_status": self.save_failure_status,
                        "scope": "Native diagnostic wrapper, normal recovery discovery/timer/UI/load/save; synthetic egui pointer, not OS/tablet input or power-loss certification",
                    }),
                )?;
                self.step = 5;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
        Ok(())
    }
}

fn text(ctx: &egui::Context, wanted: &str) -> Option<egui::Pos2> {
    fn find(shape: &egui::Shape, wanted: &str, clip: egui::Rect) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == wanted => {
                let visible = t.galley.rect.translate(t.pos.to_vec2()).intersect(clip);
                visible.is_positive().then(|| visible.center())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, wanted, clip)),
            _ => None,
        }
    }
    let layers = ctx.memory(|m| m.areas().visible_layer_ids());
    ctx.graphics(|g| {
        layers.iter().find_map(|id| {
            g.get(*id).and_then(|list| {
                list.all_entries()
                    .find_map(|s| find(&s.shape, wanted, s.clip_rect))
            })
        })
    })
}

impl eframe::App for Probe {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(events) = self.events.pop_front() {
            input.events.extend(events);
        }
        eframe::App::raw_input_hook(&mut self.app, ctx, input);
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Err(error) = self.tick(ctx, frame) {
            eprintln!("Recovery probe: {error}");
            *self.error.lock().unwrap() = Some(error);
            self.app.close_ready = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

pub(crate) fn run(root: &Path, phase: &str) -> Result<(), String> {
    let restore = match phase {
        "write-v4"
        | "write-v5"
        | "write-v4-large"
        | "write-v5-large"
        | "write-v4-large-adjustment"
        | "write-v5-large-adjustment" => false,
        "restore" | "restore-save-sharing-failure" => true,
        _ => return Err("Unknown artwork recovery probe phase".into()),
    };
    if !restore {
        std::fs::create_dir(root).map_err(|e| e.to_string())?;
        std::fs::write(root.join("fixture-marker"), MARKER).map_err(|e| e.to_string())?;
        std::fs::create_dir(root.join("recovery")).map_err(|e| e.to_string())?;
    } else if std::fs::read(root.join("fixture-marker")).map_err(|e| e.to_string())? != MARKER {
        return Err("Recovery probe requires its disposable fixture marker".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if root
        .join(if restore {
            "restore.json"
        } else {
            "write-ready.json"
        })
        .exists()
    {
        return Err("Probe result already exists".into());
    }
    let mut app = if restore {
        Dudel::default()
    } else {
        fixture(phase.starts_with("write-v5"), phase.contains("-large"))
    };
    app.init_recovery_in(root.join("recovery"));
    let original_bytes;
    let mut snapshot_bytes = Vec::new();
    if restore {
        let path = app
            .recovery_candidates
            .first()
            .ok_or("No interrupted recovery snapshot")?;
        snapshot_bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        original_bytes = std::fs::read(root.join("original.dudel")).map_err(|e| e.to_string())?;
    } else {
        app.path = Some(root.join("original.dudel"));
        storage::save_project(app.path.as_ref().unwrap(), &record(&app))?;
        original_bytes = std::fs::read(app.path.as_ref().unwrap()).map_err(|e| e.to_string())?;
        revise(&mut app, 1);
    }
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let probe = Probe {
        app,
        root: root.clone(),
        restore,
        generation: 1,
        step: 0,
        frames: 0,
        started: Instant::now(),
        edited: Instant::now(),
        cadence: Vec::new(),
        busy_adjustment: phase.ends_with("-adjustment"),
        busy_preview_seen: false,
        deferred_frames: 0,
        busy_released_seconds: None,
        save_sharing_failure: phase == "restore-save-sharing-failure",
        save_blocker: None,
        save_failure_verified: false,
        save_failure_status: None,
        events: VecDeque::new(),
        screenshot: None,
        captured: false,
        snapshot_bytes,
        original_bytes,
        error: error.clone(),
    };
    let display = crate::display_settings::DisplaySettings::default().with_env_override();
    eframe::run_native(
        "Dudel isolated artwork recovery probe",
        display.native_options(eframe::NativeOptions {
            persist_window: false,
            persistence_path: Some(root.join("isolated-app.ron")),
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 800.0])
                .with_visible(false),
            ..Default::default()
        }),
        Box::new(move |_| Ok(Box::new(probe))),
    )
    .map_err(|e| e.to_string())?;
    if let Some(error) = error.lock().unwrap().take() {
        return Err(error);
    }
    if !restore || !root.join("restore.json").is_file() {
        return Err("Native probe exited without completed restore evidence".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_detects_hidden_pixels_masks_and_guide_metadata() {
        let app = fixture(true, false);
        let original = record(&app);
        equal(&original, &original.clone()).unwrap();
        let mut changed = original.clone();
        changed.layers[0].pixels.writer().rgba_mut(0)[0] = 17;
        assert!(equal(&original, &changed).is_err());
        let mut changed = original.clone();
        changed.layers[0].mask.as_mut().unwrap().enabled = false;
        assert!(equal(&original, &changed).is_err());
        let mut changed = original.clone();
        changed.guides.as_mut().unwrap().locked = !original.guides.as_ref().unwrap().locked;
        assert!(equal(&original, &changed).is_err());
    }
}
