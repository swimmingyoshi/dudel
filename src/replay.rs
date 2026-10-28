//! Opt-in active-layer stroke fixtures. Input timing is preserved, never invented
//! from playback frames. No replay operation mutates the open application document.
use crate::{
    document::Document,
    stroke::Sample,
    stroke_buffer::{Settings, StrokeBuffer},
    view::View,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    time::Instant,
};

const MAGIC: &[u8] = b"DRPL\x01\r\n";
const MAX_COMMANDS: usize = 65_536;
const MAX_METADATA: usize = 32 * 1024 * 1024;
const MAX_PROJECT: usize = 20 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Mapping {
    origin: [f32; 2],
    zoom: f32,
    rotation: f32,
    mirror: bool,
    half: [f32; 2],
}
impl From<View> for Mapping {
    fn from(v: View) -> Self {
        Self {
            origin: [v.origin.x, v.origin.y],
            zoom: v.zoom,
            rotation: v.rotation,
            mirror: v.mirror,
            half: [v.half.x, v.half.y],
        }
    }
}
impl Mapping {
    fn view(self) -> View {
        View {
            origin: eframe::egui::pos2(self.origin[0], self.origin[1]),
            zoom: self.zoom,
            rotation: self.rotation,
            mirror: self.mirror,
            half: eframe::egui::vec2(self.half[0], self.half[1]),
        }
    }
    fn local(self, mut sample: Sample) -> Sample {
        let p = self.view().to_doc(eframe::egui::pos2(sample.x, sample.y));
        sample.x = p.x;
        sample.y = p.y;
        sample
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum Command {
    Point {
        screen: Sample,
        mapping: Mapping,
        lift: bool,
    },
    /// Shift-line anchor primes the sampler without painting its initial dab.
    Seed { local: Sample, zoom: f32 },
    /// Leaving the canvas resets sampling but keeps the existing stroke buffer.
    Reset,
    /// A resting pen pulled the stabilized line this fraction of the way.
    CatchUp { fraction: f32 },
    /// A document-space point already snapped onto a ruler.
    Local {
        local: Sample,
        zoom: f32,
        lift: bool,
    },
}

#[derive(Serialize, Deserialize)]
struct Trace {
    version: u8,
    settings: Settings,
    commands: Vec<Command>,
    cancelled: bool,
}
struct Fixture {
    trace: Trace,
    base: Document,
    expected: Document,
}
struct Pending {
    trace: Trace,
    base: Document,
}

#[derive(Default)]
pub struct Recorder {
    pub enabled: bool,
    pub status: String,
    pending: Option<Pending>,
    last: Option<Fixture>,
}
fn active_layer(doc: &Document) -> Document {
    let source = &doc.layers[doc.active];
    let mut copy = Document::new(doc.width, doc.height);
    copy.layers[0].pixels = source.pixels.clone();
    copy.layers[0].name = source.name.clone();
    copy.layers[0].alpha_locked = source.alpha_locked;
    copy.selection = doc.selection;
    copy.selection_mask = doc.selection_mask.clone();
    copy
}
fn snapshot_fits(doc: &Document) -> bool {
    let pixels = &doc.layers[doc.active].pixels;
    let coverage = doc.selection_mask.as_ref().map_or(0, |m| m.allocation().1);
    pixels.capacity().saturating_add(coverage) <= MAX_SNAPSHOT
        && pixels.nonzero_tiles(doc.width, doc.height).len()
            * crate::pixels::SIDE
            * crate::pixels::SIDE
            * 4
            <= MAX_SNAPSHOT.saturating_sub(coverage)
}
impl Recorder {
    pub fn clear(&mut self) {
        self.pending = None;
        self.last = None;
    }
    pub fn ready(&self) -> bool {
        self.last.is_some()
    }
    pub fn begin(&mut self, doc: &Document, settings: &Settings) {
        if !self.enabled {
            return;
        }
        self.clear();
        if !snapshot_fits(doc) {
            self.status = "Stroke capture exceeds its 16MiB starting-layer budget".into();
            return;
        }
        let mut settings = settings.clone();
        settings.layer = 0;
        self.pending = Some(Pending {
            trace: Trace {
                version: 1,
                settings,
                commands: vec![],
                cancelled: false,
            },
            base: active_layer(doc),
        });
        self.status = "Capturing stroke".into();
    }
    fn push(&mut self, command: Command) {
        if let Some(pending) = &mut self.pending {
            if pending.trace.commands.len() == MAX_COMMANDS {
                self.pending = None;
                self.status =
                    "Stroke capture exceeded 65,536 commands; no partial fixture retained".into();
            } else {
                pending.trace.commands.push(command);
            }
        }
    }
    pub fn point(&mut self, screen: Sample, view: View, lift: bool) {
        self.push(Command::Point {
            screen,
            mapping: view.into(),
            lift,
        });
    }
    pub fn seed(&mut self, local: Sample, zoom: f32) {
        self.push(Command::Seed { local, zoom });
    }
    #[cfg(test)]
    pub fn reset(&mut self) {
        self.push(Command::Reset);
    }
    pub fn local(&mut self, local: Sample, zoom: f32, lift: bool) {
        self.push(Command::Local { local, zoom, lift });
    }
    pub fn catch_up(&mut self, fraction: f32) {
        self.push(Command::CatchUp { fraction });
    }
    pub fn finish(&mut self, doc: &Document, cancelled: bool) {
        if let Some(mut pending) = self.pending.take() {
            if !snapshot_fits(doc) {
                self.status = "Stroke capture exceeds its 16MiB result-layer budget".into();
                return;
            }
            pending.trace.cancelled = cancelled;
            self.last = Some(Fixture {
                trace: pending.trace,
                base: pending.base,
                expected: active_layer(doc),
            });
            self.status = if cancelled {
                "Canceled stroke ready to export"
            } else {
                "Last painted stroke ready to export"
            }
            .into();
        }
    }
    pub fn export(&self, path: &Path) -> Result<(), String> {
        let fixture = self.last.as_ref().ok_or("No completed stroke capture")?;
        fixture.validate()?;
        crate::storage::atomic_stream(path, |file| {
            file.write_all(MAGIC).map_err(|e| e.to_string())?;
            let metadata = serde_json::to_vec(&fixture.trace).map_err(|e| e.to_string())?;
            if metadata.len() > MAX_METADATA {
                return Err("Stroke metadata exceeds limit".into());
            }
            write_chunk(file, &metadata)?;
            for doc in [&fixture.base, &fixture.expected] {
                let mut bytes = Vec::new();
                crate::tile_project::encode(doc, &mut bytes)?;
                if bytes.len() > MAX_PROJECT {
                    return Err("Stroke project exceeds limit".into());
                }
                write_chunk(file, &bytes)?;
            }
            Ok(())
        })
    }
}
fn write_chunk(writer: &mut impl Write, bytes: &[u8]) -> Result<(), String> {
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|()| writer.write_all(bytes))
        .map_err(|e| e.to_string())
}
fn read_chunk(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .map_err(|e| format!("Truncated stroke fixture: {e}"))?;
    let length = u32::from_le_bytes(length) as usize;
    if length > limit {
        return Err("Stroke fixture chunk exceeds limit".into());
    }
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| format!("Truncated stroke fixture: {e}"))?;
    Ok(bytes)
}
fn finite_sample(s: Sample) -> bool {
    [s.x, s.y, s.pressure].iter().all(|v| v.is_finite())
        && (0.0..=1.0).contains(&s.pressure)
        && s.time.is_none_or(|v| v.is_finite() && v >= 0.0)
        && s.tilt
            .is_none_or(|v| v.iter().all(|v| v.is_finite() && v.abs() <= 90.0))
        && s.twist
            .is_none_or(|v| v.is_finite() && (0.0..=360.0).contains(&v))
        && [s.speed, s.random, s.taper, s.spacing]
            .iter()
            .all(|v| v.is_none_or(|v| v.is_finite() && v >= 0.0))
        && s.distance.is_finite()
        && s.distance >= 0.0
}
impl Fixture {
    fn load(path: &Path) -> Result<Self, String> {
        let mut reader =
            std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
        let mut magic = [0; 7];
        reader.read_exact(&mut magic).map_err(|e| e.to_string())?;
        if magic != MAGIC {
            return Err("Unsupported stroke replay container".into());
        }
        let trace = serde_json::from_slice(&read_chunk(&mut reader, MAX_METADATA)?)
            .map_err(|e| e.to_string())?;
        let mut projects = vec![];
        for _ in 0..2 {
            let bytes = read_chunk(&mut reader, MAX_PROJECT)?;
            if !bytes.starts_with(crate::tile_project::MAGIC) {
                return Err("Stroke fixture requires v4 sparse project chunks".into());
            }
            projects.push(crate::tile_project::decode_single_layer(
                &bytes[crate::tile_project::MAGIC.len()..],
                MAX_SNAPSHOT,
            )?);
        }
        let mut extra = [0];
        if reader.read(&mut extra).map_err(|e| e.to_string())? != 0 {
            return Err("Trailing stroke fixture data".into());
        }
        let expected = projects.pop().unwrap();
        let base = projects.pop().unwrap();
        let fixture = Self {
            trace,
            base,
            expected,
        };
        fixture.validate()?;
        Ok(fixture)
    }
    fn validate(&self) -> Result<(), String> {
        self.base.validate()?;
        self.expected.validate()?;
        self.trace.settings.brush.validate()?;
        if self.trace.version != 1
            || self.trace.commands.is_empty()
            || self.trace.commands.len() > MAX_COMMANDS
            || self.base.layers.len() != 1
            || self.expected.layers.len() != 1
            || self.base.active != 0
            || self.expected.active != 0
            || self.trace.settings.layer != 0
            || self.base.width != self.expected.width
            || self.base.height != self.expected.height
            || self.base.selection != self.expected.selection
            || self.base.selection_mask != self.expected.selection_mask
            || self.base.layers[0].alpha_locked != self.expected.layers[0].alpha_locked
            || self.base.layers[0].pixels.capacity() > MAX_SNAPSHOT
            || self.expected.layers[0].pixels.capacity() > MAX_SNAPSHOT
        {
            return Err("Invalid stroke fixture dimensions, settings or commands".into());
        }
        let symmetry = self.trace.settings.symmetry;
        if !(2..=16).contains(&symmetry.segments)
            || symmetry
                .center
                .is_some_and(|v| v.iter().any(|v| !v.is_finite()))
        {
            return Err("Invalid stroke fixture symmetry".into());
        }
        // Live contacts may start outside the page and cross its edges.
        // Keep the existing bounded local-coordinate envelope for replay input.
        let within_reach = |s: Sample| {
            let reach = (self.base.width + self.base.height) as f32 * 4.0;
            s.x.abs() <= reach && s.y.abs() <= reach
        };
        for command in &self.trace.commands {
            let valid = match command {
                Command::Reset => true,
                Command::CatchUp { fraction } => (0.0..=1.0).contains(fraction),
                Command::Local { local, zoom, .. } => {
                    // Snapped points may sit just past the edge on a ruler.
                    let reach = (self.base.width + self.base.height) as f32 * 4.0;
                    finite_sample(*local)
                        && local.x.abs() <= reach
                        && local.y.abs() <= reach
                        && zoom.is_finite()
                        && (crate::view::MIN_ZOOM..=crate::view::MAX_ZOOM).contains(zoom)
                }
                Command::Seed { local, zoom } => {
                    finite_sample(*local)
                        && within_reach(*local)
                        && zoom.is_finite()
                        && (crate::view::MIN_ZOOM..=crate::view::MAX_ZOOM).contains(zoom)
                }
                Command::Point {
                    screen, mapping, ..
                } => {
                    finite_sample(*screen)
                        && mapping.origin.iter().all(|v| v.is_finite())
                        && mapping.rotation.is_finite()
                        && mapping.half
                            == [self.base.width as f32 / 2.0, self.base.height as f32 / 2.0]
                        && mapping.zoom.is_finite()
                        && (crate::view::MIN_ZOOM..=crate::view::MAX_ZOOM).contains(&mapping.zoom)
                        && within_reach(mapping.local(*screen))
                }
            };
            if !valid {
                return Err("Invalid stroke sample or view mapping".into());
            }
        }
        Ok(())
    }
    fn replay(&self, batch_size: usize) -> Result<(Document, Report), String> {
        self.replay_with_work(batch_size, 0)
    }
    fn replay_with_work(
        &self,
        batch_size: usize,
        work_budget: usize,
    ) -> Result<(Document, Report), String> {
        self.validate()?;
        if !(1..=MAX_COMMANDS).contains(&batch_size) {
            return Err("Replay batch size must be 1..=65,536".into());
        }
        if work_budget > MAX_COMMANDS {
            return Err("Replay work budget must be 0..=65,536 (0 drains synchronously)".into());
        }
        let started = Instant::now();
        let mut document = self.base.clone();
        let mut stroke = crate::stroke::Stroke::default();
        let mut buffer = StrokeBuffer::default();
        let settings = &self.trace.settings;
        let shared_settings = (work_budget > 0).then(|| std::sync::Arc::new(settings.clone()));
        let mut render_step_calls = 0;
        let mut max_render_step_ms = 0.0f64;
        let mut cursor_peak_payload_bytes = 0;
        let mut dabs = 0;
        for batch in self.trace.commands.chunks(batch_size) {
            for command in batch {
                let samples = match command {
                    Command::Reset => {
                        stroke.reset();
                        vec![]
                    }
                    Command::CatchUp { fraction } => stroke.catch_up(*fraction, &settings.brush),
                    Command::Local { local, zoom, lift } => {
                        if *lift {
                            stroke.finish_at_zoom(*local, &settings.brush, *zoom)
                        } else {
                            stroke.sample_at_zoom(*local, &settings.brush, *zoom)
                        }
                    }
                    Command::Seed { local, zoom } => {
                        let _ = stroke.sample_at_zoom(*local, &settings.brush, *zoom);
                        vec![]
                    }
                    Command::Point {
                        screen,
                        mapping,
                        lift,
                    } => {
                        let local = mapping.local(*screen);
                        if *lift {
                            stroke.finish_at_zoom(local, &settings.brush, mapping.zoom)
                        } else {
                            stroke.sample_at_zoom(local, &settings.brush, mapping.zoom)
                        }
                    }
                };
                for dab in samples {
                    if let Some(settings) = &shared_settings {
                        let mut cursor = buffer
                            .begin_dab(&document, dab, settings.clone())
                            .ok_or("Replay layer changed")?;
                        cursor_peak_payload_bytes =
                            cursor_peak_payload_bytes.max(cursor.peak_payload_capacity_bytes());
                        loop {
                            let start = Instant::now();
                            let step = cursor.step(&mut buffer, &mut document, work_budget)?;
                            max_render_step_ms =
                                max_render_step_ms.max(start.elapsed().as_secs_f64() * 1000.0);
                            render_step_calls += 1;
                            cursor_peak_payload_bytes =
                                cursor_peak_payload_bytes.max(cursor.peak_payload_capacity_bytes());
                            dabs += step.painted.dabs;
                            if step.complete {
                                break;
                            }
                        }
                    } else {
                        dabs += buffer.paint(&mut document, dab, settings).dabs;
                    }
                }
            }
        }
        let finishing = Instant::now();
        if self.trace.cancelled {
            buffer.restore(&mut document);
        } else {
            if work_budget > 0 {
                if let Some(mut cursor) = buffer.begin_tail(&document, settings) {
                    cursor_peak_payload_bytes =
                        cursor_peak_payload_bytes.max(cursor.peak_payload_capacity_bytes());
                    loop {
                        let start = Instant::now();
                        let step = cursor.step(&mut buffer, &mut document, work_budget)?;
                        max_render_step_ms =
                            max_render_step_ms.max(start.elapsed().as_secs_f64() * 1000.0);
                        render_step_calls += 1;
                        cursor_peak_payload_bytes =
                            cursor_peak_payload_bytes.max(cursor.peak_payload_capacity_bytes());
                        dabs += step.painted.dabs;
                        if step.complete {
                            break;
                        }
                    }
                }
            } else {
                dabs += buffer.finish_tail(&mut document, settings).dabs;
            }
            buffer.reclaim_empty_tiles(&mut document);
        }
        let finish_ms = finishing.elapsed().as_secs_f64() * 1000.0;
        let render_ms = started.elapsed().as_secs_f64() * 1000.0;
        for index in 0..document.width * document.height {
            if document.layers[0].pixels.rgba(index) != self.expected.layers[0].pixels.rgba(index) {
                return Err(format!(
                    "Stroke replay mismatch at ({}, {})",
                    index % document.width,
                    index / document.width
                ));
            }
        }
        let mut checksum = 0xcbf29ce484222325u64;
        for index in 0..document.width * document.height {
            for byte in document.layers[0].pixels.rgba(index) {
                checksum = (checksum ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
        let report = Report {
            version: 1,
            commands: self.trace.commands.len(),
            batch_size,
            work_budget,
            render_step_calls,
            max_render_step_ms,
            cursor_peak_payload_bytes,
            cancelled: self.trace.cancelled,
            dabs,
            render_ms,
            finish_ms,
            exact_pixels_verified: document.width * document.height,
            rgba_fnv1a64: format!("{checksum:016x}"),
            scope: "Active-layer raster stroke only; preserved packet timestamps and per-packet view mapping. Packet batches and rendering slices introduce no new samples/times. Chunks drain without UI waits. CPU render timing excludes load, full-pixel verification and output. Maximum step time excludes cursor construction, sampling, cancellation, tile reclamation and history. Cursor capacity excludes shared settings, original pixels/coverage, allocator and map overhead. Does not reproduce driver, event delivery, UI or display latency.",
        };
        Ok((document, report))
    }
}
#[derive(Serialize)]
pub struct Report {
    version: u8,
    commands: usize,
    batch_size: usize,
    work_budget: usize,
    render_step_calls: usize,
    max_render_step_ms: f64,
    cursor_peak_payload_bytes: usize,
    cancelled: bool,
    dabs: usize,
    render_ms: f64,
    finish_ms: f64,
    exact_pixels_verified: usize,
    rgba_fnv1a64: String,
    scope: &'static str,
}
pub fn run(
    path: &Path,
    batch_size: usize,
    work_budget: usize,
    output: Option<&Path>,
    report: Option<&Path>,
) -> Result<(), String> {
    let fixture_path = path_identity(path)?;
    let output_path = output.map(path_identity).transpose()?;
    let report_path = report.map(path_identity).transpose()?;
    if output_path.as_ref() == Some(&fixture_path)
        || report_path.as_ref() == Some(&fixture_path)
        || (output_path.is_some() && output_path == report_path)
    {
        return Err(
            "Replay outputs must use distinct paths from the fixture and each other".into(),
        );
    }
    let fixture = Fixture::load(path)?;
    let (document, result) = if work_budget == 0 {
        fixture.replay(batch_size)?
    } else {
        fixture.replay_with_work(batch_size, work_budget)?
    };
    if let Some(output) = output {
        crate::storage::save_project(output, &document)?;
    }
    let bytes = serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?;
    if let Some(report) = report {
        crate::storage::atomic_write(report, &bytes)?;
    }
    println!("{}", String::from_utf8_lossy(&bytes));
    Ok(())
}
fn path_identity(path: &Path) -> Result<String, String> {
    let resolved = if path.exists() {
        path.canonicalize()
    } else {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        parent
            .canonicalize()
            .map(|parent| parent.join(path.file_name().unwrap_or_default()))
    }
    .map_err(|e| e.to_string())?;
    let value = resolved.to_string_lossy().into_owned();
    #[cfg(windows)]
    let value = value.to_lowercase();
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Dudel,
        input::Packet,
        symmetry::{Mode, Symmetry},
    };
    use eframe::egui::{self, pos2, vec2};

    fn frame(app: &mut Dudel, ctx: &egui::Context, packets: &[Packet], shift: bool) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(700.0, 500.0),
                )),
                modifiers: egui::Modifiers {
                    shift,
                    ..Default::default()
                },
                ..Default::default()
            },
            |ctx| app.canvas(ctx, packets),
        );
    }
    fn screen(app: &Dudel, x: f32, y: f32, pressure: f32, time: Option<f64>) -> Sample {
        let p = app.view(app.canvas_area).to_screen(pos2(x, y));
        Sample {
            x: p.x,
            y: p.y,
            pressure,
            pressure_known: true,
            time,
            tilt: Some([-15.0, 20.0]),
            twist: Some(36.0),
            ..Default::default()
        }
    }
    fn app() -> Dudel {
        let mut app = Dudel {
            doc: Document::new(137, 97),
            zoom: 2.0,
            auto_fit: false,
            rotation: 0.35,
            mirror: true,
            ..Default::default()
        };
        app.replay.enabled = true;
        app.brush = crate::brush::Brush {
            size: 13.0,
            stabilization: 6.0,
            taper_in: 10.0,
            taper_out: 18.0,
            scatter: 0.35,
            scatter_seed: 123,
            adjust_flow_by_spacing: true,
            opacity: 0.65,
            flow: 0.3,
            ..Default::default()
        };
        app.symmetry = Symmetry {
            mode: Mode::FourWay,
            ..Default::default()
        };
        app.doc.selection = Some(crate::selection::Selection {
            left: 4,
            top: 3,
            right: 130,
            bottom: 93,
        });
        for (index, rgba) in [(3, [12, 34, 56, 0]), (30 * 137 + 30, [90, 80, 70, 130])] {
            app.doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(index)
                .copy_from_slice(&rgba);
        }
        app
    }
    #[test]
    fn canvas_capture_roundtrips_exactly_across_batches_and_changed_views() {
        for native_time in [false, true] {
            let mut app = app();
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, &[], false);
            let start = screen(&app, 22.0, 24.0, 0.0, native_time.then_some(1.0));
            frame(&mut app, &ctx, &[Packet::Start(start)], false);
            // UI brush changes must not replace the settings captured at start.
            app.brush.size = 100.0;
            let moved = screen(&app, 48.0, 28.0, 0.7, native_time.then_some(1.015));
            frame(&mut app, &ctx, &[Packet::Move(moved)], false);
            frame(
                &mut app,
                &ctx,
                &[Packet::Move(Sample {
                    x: -1000.0,
                    y: -1000.0,
                    ..moved
                })],
                false,
            );
            app.zoom = 1.4;
            app.rotation = -0.5;
            app.mirror = false;
            app.pan = vec2(25.0, -14.0);
            let moved = screen(&app, 82.0, 48.0, 0.9, native_time.then_some(1.03));
            let end = screen(&app, 102.0, 66.0, 0.4, native_time.then_some(1.045));
            frame(
                &mut app,
                &ctx,
                &[Packet::Move(moved), Packet::End(end)],
                false,
            );
            assert!(app.replay.ready(), "{}", app.replay.status);
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("stroke.dudelreplay");
            app.replay.export(&path).unwrap();
            let fixture = Fixture::load(&path).unwrap();
            let mut hashes = vec![];
            for batch in [1, 2, 64, MAX_COMMANDS] {
                let (result, report) = fixture.replay(batch).unwrap();
                assert_eq!(result.layers[0].pixels, app.doc.layers[0].pixels);
                assert_eq!(result.layers[0].pixels.rgba(3), [12, 34, 56, 0]);
                hashes.push(report.rgba_fnv1a64);
                let (sliced, sliced_report) = fixture.replay_with_work(batch, 23).unwrap();
                assert_eq!(sliced.layers[0].pixels, result.layers[0].pixels);
                assert!(sliced_report.render_step_calls > 0);
                hashes.push(sliced_report.rgba_fnv1a64);
            }
            assert!(hashes.windows(2).all(|w| w[0] == w[1]));
            assert!(app.history.undo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels, fixture.base.layers[0].pixels);
            assert!(app.history.redo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels, fixture.expected.layers[0].pixels);
        }
    }
    #[test]
    fn snapped_strokes_capture_and_replay_exactly() {
        let mut app = app();
        app.rulers = vec![crate::rulers::Ruler::Focus {
            center: [68.0, 48.0],
        }];
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, &[], false);
        let start = screen(&app, 90.0, 50.0, 0.6, Some(1.0));
        frame(&mut app, &ctx, &[Packet::Start(start)], false);
        for (k, (x, y)) in [(104.0, 58.0), (118.0, 55.0), (126.0, 62.0)]
            .into_iter()
            .enumerate()
        {
            let moved = screen(&app, x, y, 0.8, Some(1.0 + 0.01 * (k + 1) as f64));
            frame(&mut app, &ctx, &[Packet::Move(moved)], false);
        }
        let end = screen(&app, 128.0, 60.0, 0.3, Some(1.05));
        frame(&mut app, &ctx, &[Packet::End(end)], false);
        assert!(app.replay.ready(), "{}", app.replay.status);
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("snapped.dudelreplay");
        app.replay.export(&path).unwrap();
        let fixture = Fixture::load(&path).unwrap();
        assert!(
            fixture
                .trace
                .commands
                .iter()
                .any(|c| matches!(c, Command::Local { .. }))
        );
        for batch in [1, 3, MAX_COMMANDS] {
            let (result, _) = fixture.replay(batch).unwrap();
            assert_eq!(result.layers[0].pixels, app.doc.layers[0].pixels);
        }
    }
    #[test]
    fn strokes_starting_near_a_guide_follow_it() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(80, 60),
            ..Default::default()
        };
        app.brush = crate::brush::Brush {
            size: 4.0,
            hardness: 1.0,
            pressure_size: false,
            ..Default::default()
        };
        app.guide_lines = vec![crate::guides::GuideLine {
            vertical: true,
            position: 40.0,
        }];
        frame(&mut app, &ctx, &[], false);
        let wobble = [
            (41.0, 8.0),
            (43.0, 20.0),
            (37.0, 32.0),
            (44.0, 44.0),
            (39.0, 52.0),
        ];
        let samples: Vec<Sample> = wobble
            .iter()
            .map(|&(x, y)| screen(&app, x, y, 1.0, None))
            .collect();
        frame(&mut app, &ctx, &[Packet::Start(samples[0])], false);
        for sample in &samples[1..samples.len() - 1] {
            frame(&mut app, &ctx, &[Packet::Move(*sample)], false);
        }
        frame(
            &mut app,
            &ctx,
            &[Packet::End(samples[samples.len() - 1])],
            false,
        );
        app.flush_render_queue();
        let painted: Vec<usize> = (0..80 * 60)
            .filter(|&i| app.doc.layers[0].pixels.rgba(i)[3] > 0)
            .map(|i| i % 80)
            .collect();
        assert!(!painted.is_empty());
        // A 4px tip centred on x = 40 covers columns 38..=41 only.
        assert!(painted.iter().all(|x| (37..=42).contains(x)), "{painted:?}");
        // Without snapping the same wobble strays well off the line.
        app.snapping = false;
        app.doc = Document::new(80, 60);
        frame(&mut app, &ctx, &[Packet::Start(samples[0])], false);
        for sample in &samples[1..] {
            frame(&mut app, &ctx, &[Packet::Move(*sample)], false);
        }
        frame(
            &mut app,
            &ctx,
            &[Packet::End(samples[samples.len() - 1])],
            false,
        );
        app.flush_render_queue();
        assert!((0..80 * 60).any(|i| app.doc.layers[0].pixels.rgba(i)[3] > 0 && i % 80 > 43));
    }
    #[test]
    fn cancellation_replays_restoration_and_preserves_redo() {
        let mut app = app();
        let anchor = Sample {
            x: 12.0,
            y: 14.0,
            pressure: 0.8,
            ..Default::default()
        };
        app.last_stroke_end = Some(anchor);
        app.history.checkpoint(&app.doc);
        app.doc.dab(90.0, 35.0, 1.0, &app.brush, [1, 2, 3], false);
        app.history.settle(&app.doc);
        assert!(app.history.undo(&mut app.doc));
        let base = app.doc.layers[0].pixels.clone();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, &[], false);
        let start = screen(&app, 22.0, 24.0, 0.6, Some(1.0));
        let moved = screen(&app, 45.0, 26.0, 0.9, Some(1.01));
        frame(
            &mut app,
            &ctx,
            &[Packet::Start(start), Packet::Move(moved), Packet::Cancel],
            false,
        );
        assert_eq!(app.doc.layers[0].pixels, base);
        assert_eq!(app.last_stroke_end, Some(anchor));
        assert!(!app.history.can_undo());
        assert!(app.history.can_redo());
        let fixture = app.replay.last.as_ref().unwrap();
        assert!(fixture.trace.cancelled);
        assert_eq!(fixture.replay(1).unwrap().0.layers[0].pixels, base);
        assert_eq!(
            fixture.replay_with_work(1, 1).unwrap().0.layers[0].pixels,
            base
        );
    }
    #[test]
    fn checked_in_square_oracle_verifies_all_channels_in_headless_output() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("research/fixtures/replay/hard-square.dudelreplay");
        let fixture = Fixture::load(&path).unwrap();
        let (doc, report) = fixture.replay(1).unwrap();
        assert_eq!(report.rgba_fnv1a64, "5075632377dbe46b");
        assert_eq!(report.exact_pixels_verified, 64);
        let (_, completed) = fixture.replay_with_work(64, 64).unwrap();
        assert!(
            completed.cursor_peak_payload_bytes
                >= std::mem::size_of::<crate::document::DabRaster>()
        );
        assert_eq!(doc.layers[0].pixels.rgba(0), [9, 8, 7, 0]);
        for y in 0..8 {
            for x in 0..8 {
                if (2..6).contains(&x) && (2..6).contains(&y) {
                    assert_eq!(doc.layers[0].pixels.rgba(y * 8 + x), [17, 83, 201, 255]);
                }
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("replayed.dudel");
        let report = directory.path().join("report.json");
        run(&path, 64, 1, Some(&output), Some(&report)).unwrap();
        let loaded = crate::storage::load_project(&output).unwrap();
        assert_eq!(loaded.layers[0].pixels, doc.layers[0].pixels);
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
        assert_eq!(result["rgba_fnv1a64"], "5075632377dbe46b");
    }
    #[test]
    fn shift_line_seed_eraser_and_alpha_lock_match_live_canvas() {
        for erase in [false, true] {
            let mut app = app();
            app.symmetry.mode = Mode::Off;
            app.tool = if erase {
                crate::tools::Tool::Eraser
            } else {
                crate::tools::Tool::Brush
            };
            app.doc.layers[0].pixels = vec![100; 137 * 97 * 4].into();
            app.doc.layers[0].alpha_locked = !erase;
            app.last_stroke_end = Some(Sample {
                x: 14.0,
                y: 18.0,
                pressure: 0.5,
                time: Some(0.9),
                ..Default::default()
            });
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, &[], false);
            let point = screen(&app, 45.0, 30.0, 0.8, Some(1.0));
            frame(
                &mut app,
                &ctx,
                &[Packet::Start(point), Packet::End(point)],
                true,
            );
            let fixture = app.replay.last.as_ref().unwrap();
            assert!(matches!(fixture.trace.commands[0], Command::Seed { .. }));
            assert_eq!(
                fixture.replay(2).unwrap().0.layers[0].pixels,
                app.doc.layers[0].pixels
            );
        }
    }
    #[test]
    fn malformed_mismatched_and_oversized_fixtures_fail_without_overwriting_files() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, &[], false);
        let point = screen(&app, 22.0, 24.0, 0.7, Some(1.0));
        frame(
            &mut app,
            &ctx,
            &[Packet::Start(point), Packet::End(point)],
            false,
        );
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("stroke.dudelreplay");
        app.replay.export(&path).unwrap();
        let saved = std::fs::read(&path).unwrap();
        let alias = folder.path().join(".").join("stroke.dudelreplay");
        assert!(run(&path, 1, 64, Some(&alias), None).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        for cut in [0, 6, 11, saved.len() - 1] {
            std::fs::write(&path, &saved[..cut]).unwrap();
            assert!(Fixture::load(&path).is_err());
        }
        let mut oversized = MAGIC.to_vec();
        oversized.extend_from_slice(&((MAX_METADATA + 1) as u32).to_le_bytes());
        std::fs::write(&path, oversized).unwrap();
        assert!(Fixture::load(&path).is_err());
        std::fs::write(&path, &saved).unwrap();
        let mut fixture = Fixture::load(&path).unwrap();
        // Reject multi-layer metadata before allocating its pixel records.
        let mut layers = fixture.base.clone();
        layers.add_layer();
        let mut encoded = vec![];
        crate::tile_project::encode(&layers, &mut encoded).unwrap();
        assert!(
            crate::tile_project::decode_single_layer(
                &encoded[crate::tile_project::MAGIC.len()..],
                MAX_SNAPSHOT
            )
            .is_err()
        );
        let mut encoded = vec![];
        crate::tile_project::encode(&fixture.base, &mut encoded).unwrap();
        assert!(
            crate::tile_project::decode_single_layer(
                &encoded[crate::tile_project::MAGIC.len()..],
                1
            )
            .is_err()
        );
        fixture.expected.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 4]);
        assert!(fixture.replay(64).err().unwrap().contains("mismatch"));
        assert!(fixture.replay(0).is_err());
        assert!(fixture.replay_with_work(1, MAX_COMMANDS + 1).is_err());
        if let Command::Point { screen, .. } = &mut fixture.trace.commands[0] {
            screen.time = Some(f64::NAN);
        }
        assert!(fixture.validate().is_err());
        // Overflow discards the pending capture, never silently truncates it.
        app.replay.begin(
            &app.doc,
            &Settings {
                target: crate::paint_target::Target::Raster,
                brush: app.brush.clone(),
                color: app.color,
                erase: false,
                layer: 0,
                symmetry: app.symmetry,
            },
        );
        for _ in 0..=MAX_COMMANDS {
            app.replay.reset();
        }
        assert!(app.replay.pending.is_none());
        assert!(!app.replay.ready());
    }
}
