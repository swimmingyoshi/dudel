//! Artwork-only interval recording for speed-paint playback. Each canvas owns
//! its recording, so it travels with its tab and continues when a saved
//! project is reopened. One COW snapshot is in flight at a time; frames live
//! on disk as PNG, play back in the Timelapse window and export as a GIF
//! that encodes only what changed between frames.
use crate::{Dudel, document::Document, egui, snapshot_memory::SourceWorker};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const MAX_FRAMES: usize = 7200;
const MAX_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_SIDE: u32 = 1280;
const MAX_FRAME_FILE: u64 = 12 * 1024 * 1024;
/// Longest GIF side offered for export.
const EXPORT_SIDES: [(u32, &str); 4] = [
    (1280, "Large · 1280 px"),
    (960, "Medium · 960 px"),
    (720, "Small · 720 px"),
    (480, "Tiny · 480 px"),
];

/// Timelapse choices that carry between sessions.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Prefs {
    /// Record every new canvas from its first stroke.
    pub record_new: bool,
    /// Seconds between captured frames.
    pub interval: u64,
    /// Playback frames per second, for the preview and GIF.
    pub fps: u32,
    /// Longest side of exported GIFs.
    pub export_side: u32,
    /// Seconds a long recording is fitted into, final hold included; None
    /// plays every frame.
    pub length: Option<u32>,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            record_new: false,
            interval: 5,
            fps: 12,
            export_side: 960,
            length: None,
        }
    }
}
/// Lengths a timelapse can be fitted into.
const LENGTHS: [(Option<u32>, &str); 4] = [
    (None, "Full length"),
    (Some(60), "1 minute"),
    (Some(30), "30 seconds"),
    (Some(15), "15 seconds"),
];
/// Seconds the finished artwork stays up at the end.
const HOLD_SECONDS: u32 = 2;

/// The recorded frames a timelapse shows, in order. Fitted into `length`
/// seconds, it keeps evenly spaced frames, always the first and the last,
/// and every kept frame is shown as recorded.
fn chosen(frames: usize, fps: u32, length: Option<u32>) -> Vec<usize> {
    let room = length.map_or(usize::MAX, |seconds| {
        (seconds.saturating_sub(HOLD_SECONDS) * fps).max(2) as usize
    });
    if frames <= room {
        return (0..frames).collect();
    }
    (0..room).map(|k| k * (frames - 1) / (room - 1)).collect()
}
impl Prefs {
    pub(crate) fn sanitized(mut self) -> Self {
        self.interval = self.interval.clamp(2, 60);
        self.fps = self.fps.clamp(1, 30);
        if !LENGTHS.iter().any(|(length, _)| *length == self.length) {
            self.length = None;
        }
        if !EXPORT_SIDES
            .iter()
            .any(|(side, _)| *side == self.export_side)
        {
            self.export_side = Prefs::default().export_side;
        }
        self
    }
}

/// `recording.json`: what a recording folder holds and whom it belongs to.
/// Older recordings carry only the first four fields.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
struct Inventory {
    format: String,
    version: u32,
    frames: usize,
    bytes: u64,
    #[serde(default)]
    width: usize,
    #[serde(default)]
    height: usize,
    /// The saved project this records, so reopening it continues here.
    #[serde(default)]
    project: Option<PathBuf>,
    /// False once stopped; only active recordings continue on reopen.
    #[serde(default)]
    active: bool,
    /// Hash of the last frame, so a continued recording skips a repeat.
    #[serde(default)]
    hash: Option<u64>,
}
impl Inventory {
    fn read(root: &Path) -> Result<Self, String> {
        let text = crate::read_bounded(&root.join("recording.json"), 4096)?;
        let inventory: Inventory = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if inventory.format != "dudel-timelapse"
            || inventory.version != 1
            || inventory.frames > MAX_FRAMES
            || inventory.bytes > MAX_BYTES + MAX_FRAME_FILE
        {
            return Err("Unsupported recording inventory".into());
        }
        Ok(inventory)
    }
    fn write(&self, root: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        crate::storage::atomic_write(&root.join("recording.json"), &bytes)
    }
}

enum Captured {
    Written {
        bytes: u64,
        hash: u64,
    },
    /// The artwork looked the same as the previous frame.
    Same,
}

/// One canvas's recording. It is parked with its tab; a frame still being
/// written while parked is counted when the tab is shown again.
#[derive(Default)]
pub(crate) struct Recording {
    on: bool,
    paused: bool,
    /// Capture the final state, then stop.
    finish: bool,
    /// Folder of frames, chosen when the first frame is captured.
    root: Option<PathBuf>,
    frames: usize,
    bytes: u64,
    dimensions: [usize; 2],
    last: Option<Instant>,
    revision: Option<u64>,
    hash: Option<u64>,
    /// Project path last written to the inventory.
    project: Option<PathBuf>,
    /// The project the folder's name was last made to follow.
    named_for: Option<PathBuf>,
    worker: Option<SourceWorker<Result<Captured, String>>>,
    message: String,
}
impl Recording {
    pub(crate) fn recording(&self) -> bool {
        self.on
    }
    fn inventory(&self, active: bool) -> Inventory {
        Inventory {
            format: "dudel-timelapse".into(),
            version: 1,
            frames: self.frames,
            bytes: self.bytes,
            width: self.dimensions[0],
            height: self.dimensions[1],
            project: self.project.clone(),
            active,
            hash: self.hash,
        }
    }
    /// Record the canvas as it changes. A fresh canvas waits for its first
    /// change, so an unused canvas leaves no folder behind.
    fn begin(&mut self, dimensions: [usize; 2], revision: Option<u64>) {
        *self = Recording {
            on: true,
            dimensions,
            revision,
            ..Default::default()
        };
    }
    /// Keep adding frames to a stopped recording.
    fn resume(&mut self) {
        self.on = true;
        self.paused = false;
        self.last = None;
        self.message.clear();
    }
    fn stop(&mut self) {
        self.on = false;
        self.paused = false;
        self.finish = true;
    }
    /// End capture with a message; the frames stay on disk.
    fn end(&mut self, message: String) {
        self.on = false;
        self.paused = false;
        self.finish = false;
        if let Some(root) = &self.root
            && self.worker.is_none()
        {
            let _ = self.inventory(false).write(root);
        }
        self.message = message;
    }
    /// Count the frame being written, waiting for it.
    fn join_frame(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        match worker.join() {
            Ok(Ok(Captured::Written { bytes, hash })) => {
                self.frames += 1;
                self.bytes += bytes;
                self.hash = Some(hash);
            }
            Ok(Ok(Captured::Same)) => {}
            Ok(Err(e)) => self.end(format!("Recording stopped: {e}")),
            Err(_) => self.end("Recording stopped: frame worker failed".into()),
        }
    }
    /// The canvas is being replaced. An active recording stays active on
    /// disk, so reopening its project continues it; a requested stop is
    /// recorded first.
    fn detach(&mut self) {
        if self.finish {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            if let Some(root) = &self.root {
                let _ = self.inventory(false).write(root);
            }
        }
        *self = Recording::default();
    }
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        if let Some(worker) = &self.worker {
            worker.register_source(registry);
            if !worker.is_finished() {
                registry.add_owned(
                    self.dimensions[0] * self.dimensions[1] * 4
                        + MAX_SIDE as usize * MAX_SIDE as usize * 4,
                );
            }
        }
    }
}

/// The Timelapse window: preview player and GIF export, shared by all tabs.
#[derive(Default)]
pub struct Timelapse {
    pub open: bool,
    pub(crate) prefs: Prefs,
    /// A recording opened from disk instead of the active canvas's.
    opened: Option<(PathBuf, usize)>,
    player: Player,
    export: Option<std::thread::JoinHandle<Result<(), String>>>,
    export_frames: usize,
    export_control: crate::plugins::FilterControl,
    exported: Option<PathBuf>,
    message: String,
}

type Decode = std::thread::JoinHandle<Result<egui::ColorImage, String>>;

#[derive(Default)]
struct Player {
    frame: usize,
    playing: bool,
    /// Fraction of a frame already played.
    phase: f32,
    tick: Option<Instant>,
    shown: Option<(PathBuf, usize)>,
    texture: Option<egui::TextureHandle>,
    decode: Option<(PathBuf, usize, Decode)>,
}

fn frame_path(root: &Path, index: usize) -> PathBuf {
    root.join(format!("frame-{index:06}.png"))
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Opaque artwork frame, at most `MAX_SIDE` on its longest side.
fn render_frame(doc: &Document) -> Result<image::RgbImage, String> {
    render_frame_banded(doc).map(|(image, _)| image)
}

/// Composite a bounded band of source rows, then reduce it directly into RGB.
/// Match image's thumbnail box windows and integer rounding so existing frame
/// hashes (and repeat-frame detection) remain stable. Supported dimensions only
/// downsample: every output window contains at least one source pixel.
fn render_frame_banded(doc: &Document) -> Result<(image::RgbImage, usize), String> {
    if doc.width == 0
        || doc.height == 0
        || doc.width > crate::document::MAX_SIDE
        || doc.height > crate::document::MAX_SIDE
    {
        return Err("Invalid frame dimensions".into());
    }
    let scale = (MAX_SIDE as f64 / doc.width.max(doc.height) as f64).min(1.0);
    let width = (doc.width as f64 * scale).round().max(1.0) as usize;
    let height = (doc.height as f64 * scale).round().max(1.0) as usize;
    let window = |index: usize, source: usize, target: usize| {
        let ratio = source as f32 / target as f32;
        let start = index as f32 * ratio;
        let left = (start.ceil() as usize).min(source - 1);
        let right = ((start + ratio).ceil() as usize).clamp(left, source);
        (left, right)
    };
    let columns: Vec<_> = (0..width).map(|x| window(x, doc.width, width)).collect();
    let mut output = image::RgbImage::new(width as u32, height as u32);
    let mut peak_band_bytes = 0;
    // Keep already-small images in one parallel composite. Splitting these
    // below the compositor's parallel threshold makes recording slower.
    let band_rows = if width == doc.width && height == doc.height {
        height
    } else {
        64
    };
    for first in (0..height).step_by(band_rows) {
        let last = (first + band_rows).min(height);
        let top = window(first, doc.height, height).0;
        let bottom = window(last - 1, doc.height, height).1;
        let rgba = doc.composite_bounds_checked([0, top, doc.width, bottom], false, || Ok(()))?;
        peak_band_bytes = peak_band_bytes.max(rgba.capacity());
        if width == doc.width && height == doc.height {
            let rgb: &mut [u8] = output.as_mut();
            for (target, source) in rgb[first * width * 3..last * width * 3]
                .chunks_exact_mut(3)
                .zip(rgba.chunks_exact(4))
            {
                target.copy_from_slice(&source[..3]);
            }
            continue;
        }
        for y in first..last {
            let (row_top, row_bottom) = window(y, doc.height, height);
            for (x, &(left, right)) in columns.iter().enumerate() {
                let mut sums = [0u32; 3];
                for row in row_top..row_bottom {
                    let start = ((row - top) * doc.width + left) * 4;
                    let end = start + (right - left) * 4;
                    for pixel in rgba[start..end].chunks_exact(4) {
                        for c in 0..3 {
                            sums[c] += u32::from(pixel[c]);
                        }
                    }
                }
                let count = ((row_bottom - row_top) * (right - left)) as u32;
                output.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgb(sums.map(|sum| ((sum + count / 2) / count) as u8)),
                );
            }
        }
    }
    Ok((output, peak_band_bytes))
}

fn write_png(image: &image::RgbImage, path: &Path) -> Result<u64, String> {
    crate::storage::atomic_stream(path, |file| {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new_with_quality(
            file,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(
            image,
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())
    })?;
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| e.to_string())
}

/// Write frame `inventory.frames` unless it repeats the previous one, then
/// the inventory that counts it.
fn capture(doc: Document, root: &Path, mut inventory: Inventory) -> Result<Captured, String> {
    let image = render_frame(&doc)?;
    drop(doc);
    let hash = hash_bytes(image.as_raw());
    if inventory.hash == Some(hash) {
        return Ok(Captured::Same);
    }
    if inventory.frames == 0 {
        crate::storage::atomic_write(&root.join("README.txt"), b"Dudel timelapse: artwork-only PNG frames in playback order. Open this folder from View > Timelapse to play or export it, or import frame-*.png as an image sequence in a video editor. No UI, cursor or reference images are recorded.\n")?;
    }
    let bytes = write_png(&image, &frame_path(root, inventory.frames))?;
    inventory.frames += 1;
    inventory.bytes += bytes;
    inventory.hash = Some(hash);
    inventory.write(root)?;
    Ok(Captured::Written { bytes, hash })
}

fn read_frame(root: &Path, index: usize) -> Result<image::RgbaImage, String> {
    let path = frame_path(root, index);
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_FRAME_FILE {
        return Err("Timelapse frame exceeds 12 MiB".into());
    }
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(u64::from(MAX_SIDE) * u64::from(MAX_SIDE) * 4);
    reader.limits(limits);
    let mut image = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    // GIF has no partial alpha; frames are opaque, older ones included.
    for pixel in image.pixels_mut() {
        pixel[3] = 255;
    }
    Ok(image)
}

/// UTC calendar date of a Unix time, for folder names.
fn date(seconds: u64) -> String {
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A new, empty folder under `parent` named for the canvas and date.
fn new_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    claim(parent, &format!("{name} {}", date(seconds)), |path| {
        std::fs::create_dir(path)
    })
}

/// The first of `base`, `base (2)`, `base (3)`… under `parent` that `take`
/// can claim; a name already in use is passed over.
fn claim(
    parent: &Path,
    base: &str,
    mut take: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    for n in 1..1000 {
        let path = parent.join(if n == 1 {
            base.to_string()
        } else {
            format!("{base} ({n})")
        });
        if path.exists() {
            continue;
        }
        match take(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Too many timelapse folders with this name today".into())
}

/// Rename a recording's folder after the project it belongs to, keeping
/// the date it was started, so "Untitled 2026-11-17 (8)" becomes
/// "LAVA_BASE 2026-11-17" once the canvas is saved as LAVA_BASE. None when
/// the name already fits.
fn follow_project(root: &Path, project: &Path) -> Result<Option<PathBuf>, String> {
    let stem = project
        .file_stem()
        .ok_or("The project has no name")?
        .to_string_lossy();
    let name = root
        .file_name()
        .ok_or("The recording has no folder")?
        .to_string_lossy();
    if name.starts_with(&format!("{stem} ")) {
        return Ok(None);
    }
    let parent = root.parent().ok_or("The recording has no folder")?;
    let metadata = std::fs::metadata(root).map_err(|e| e.to_string())?;
    let started = metadata
        .created()
        .or_else(|_| metadata.modified())
        .map_err(|e| e.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    claim(parent, &format!("{stem} {}", date(started)), |path| {
        std::fs::rename(root, path)
    })
    .map(Some)
}

pub(crate) fn gif_destination(path: &Path) -> Result<(), String> {
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("gif"))
    {
        return Err("Choose a .gif filename".into());
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            use std::io::Read;
            let mut magic = [0; 6];
            file.read_exact(&mut magic)
                .map_err(|_| "Destination is not a GIF; choose a new filename")?;
            if magic != *b"GIF89a" && magic != *b"GIF87a" {
                return Err("Destination is not a GIF; choose a new filename".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// Centiseconds from the start of playback to frame `index`.
fn frame_time(index: usize, fps: u32) -> u64 {
    (index as u64 * 100 + u64::from(fps) / 2) / u64::from(fps)
}

type PaletteIndex = Box<dyn Fn(&[u8]) -> u8>;

/// Pixels of `current` that differ from `previous` (all of them for the
/// first frame) as one GIF frame: their bounding box, with unchanged pixels
/// left transparent so the previous frame shows through.
fn delta_frame(
    previous: Option<&image::RgbaImage>,
    current: &image::RgbaImage,
) -> Option<gif::Frame<'static>> {
    let (width, height) = current.dimensions();
    let differs = |x, y| previous.is_none_or(|p| p.get_pixel(x, y) != current.get_pixel(x, y));
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            if differs(x, y) {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if left >= right {
        return None;
    }
    let mut changed = Vec::new();
    let mut mask = Vec::with_capacity(((right - left) * (bottom - top)) as usize);
    for y in top..bottom {
        for x in left..right {
            let differs = differs(x, y);
            mask.push(differs);
            if differs {
                changed.extend_from_slice(&current.get_pixel(x, y).0);
            }
        }
    }
    // Index 255 is reserved for "unchanged". Few colors get an exact
    // palette; NeuQuant samples every pixel of a small change, since a
    // sparse sample of it would leave the palette untrained.
    let mut exact = std::collections::HashMap::new();
    for pixel in changed.chunks_exact(4) {
        let next = exact.len();
        exact.entry([pixel[0], pixel[1], pixel[2]]).or_insert(next);
        if exact.len() > 255 {
            break;
        }
    }
    let (palette, index): (Vec<u8>, PaletteIndex) = if exact.len() <= 255 {
        let mut palette = vec![0; 256 * 3];
        for (color, i) in &exact {
            palette[i * 3..i * 3 + 3].copy_from_slice(color);
        }
        (
            palette,
            Box::new(move |p: &[u8]| exact[&[p[0], p[1], p[2]]] as u8),
        )
    } else {
        let speed = if changed.len() < 4 * 50_000 { 1 } else { 10 };
        let quant = color_quant::NeuQuant::new(speed, 255, &changed);
        let mut palette = quant.color_map_rgb();
        palette.resize(256 * 3, 0);
        (palette, Box::new(move |p: &[u8]| quant.index_of(p) as u8))
    };
    let mut pixels = changed.chunks_exact(4);
    let buffer: Vec<u8> = mask
        .iter()
        .map(|&differs| {
            if differs {
                index(pixels.next().unwrap())
            } else {
                255
            }
        })
        .collect();
    Some(gif::Frame {
        left: left as u16,
        top: top as u16,
        width: (right - left) as u16,
        height: (bottom - top) as u16,
        buffer: buffer.into(),
        palette: Some(palette),
        transparent: Some(255),
        dispose: gif::DisposalMethod::Keep,
        ..Default::default()
    })
}

fn export_gif(
    root: &Path,
    frames: &[usize],
    fps: u32,
    side: u32,
    destination: &Path,
    control: &crate::plugins::FilterControl,
) -> Result<(), String> {
    if frames.is_empty() || !(1..=30).contains(&fps) || !(16..=MAX_SIDE).contains(&side) {
        return Err("No frames or invalid playback settings".into());
    }
    gif_destination(destination)?;
    crate::storage::atomic_stream(destination, |file| {
        let mut encoder: Option<gif::Encoder<&mut std::fs::File>> = None;
        let mut file = Some(file);
        let mut previous: Option<image::RgbaImage> = None;
        // A frame is written once the next one shows how long it stays up;
        // repeated frames only lengthen it.
        let mut pending: Option<gif::Frame> = None;
        let mut size = None;
        for (index, &frame) in frames.iter().enumerate() {
            crate::fill::check_cancel(Some(&control.cancel))?;
            let image = read_frame(root, frame)?;
            let fit = fit(image.dimensions(), side, false);
            let image = fitted(image, fit);
            if *size.get_or_insert(image.dimensions()) != image.dimensions() {
                return Err("Recording frames have different sizes".into());
            }
            if encoder.is_none() {
                let (w, h) = image.dimensions();
                let mut new = gif::Encoder::new(file.take().unwrap(), w as u16, h as u16, &[])
                    .map_err(|e| e.to_string())?;
                new.set_repeat(gif::Repeat::Infinite)
                    .map_err(|e| e.to_string())?;
                encoder = Some(new);
            }
            let encoder = encoder.as_mut().unwrap();
            let delay = (frame_time(index + 1, fps) - frame_time(index, fps)) as u16;
            match delta_frame(previous.as_ref(), &image) {
                Some(mut frame) => {
                    if let Some(done) = pending.take() {
                        encoder.write_frame(&done).map_err(|e| e.to_string())?;
                    }
                    frame.delay = delay;
                    pending = Some(frame);
                }
                None => {
                    if let Some(frame) = &mut pending {
                        frame.delay = frame.delay.saturating_add(delay);
                    }
                }
            }
            previous = Some(image);
            control.progress.store(index + 1, Ordering::Relaxed);
        }
        crate::fill::check_cancel(Some(&control.cancel))?;
        let mut encoder = encoder.ok_or("No frames")?;
        if let Some(mut last) = pending {
            // Hold the finished artwork.
            last.delay = last.delay.saturating_add(HOLD_SECONDS as u16 * 100);
            encoder.write_frame(&last).map_err(|e| e.to_string())?;
        }
        encoder.into_inner().map_err(|e| e.to_string())?;
        Ok(())
    })?;
    gif_destination(destination)
}

pub(crate) fn mp4_destination(path: &Path) -> Result<(), String> {
    use std::io::Read;
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
    {
        return Err("Choose a .mp4 filename".into());
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            if file.metadata().map_err(|e| e.to_string())?.len() != 0 {
                let mut header = [0; 12];
                file.read_exact(&mut header)
                    .map_err(|_| "Destination is not MP4; choose a new filename")?;
                if &header[4..8] != b"ftyp" {
                    return Err("Destination is not MP4; choose a new filename".into());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

/// Stream bounded decoded frames to the video encoder, publishing only on success.
fn export_mp4(
    root: &Path,
    frames: &[usize],
    fps: u32,
    side: u32,
    destination: &Path,
    control: &crate::plugins::FilterControl,
) -> Result<(), String> {
    if frames.is_empty()
        || frames.len() > MAX_FRAMES
        || !(1..=30).contains(&fps)
        || !(1..=MAX_SIDE).contains(&side)
    {
        return Err("Invalid video export settings".into());
    }
    crate::fill::check_cancel(Some(&control.cancel))?;
    mp4_destination(destination)?;
    let first = read_frame(root, frames[0])?;
    let dimensions = first.dimensions();
    let fit = fit(dimensions, side, true);
    let (width, height) = fit.size(dimensions);
    drop(first);
    let size = [width as usize, height as usize];
    crate::video::write(destination, size, fps, &control.cancel, |sink| {
        for (index, &recorded) in frames.iter().enumerate() {
            crate::fill::check_cancel(Some(&control.cancel))?;
            let frame = read_frame(root, recorded)?;
            if frame.dimensions() != dimensions {
                return Err("Recording frames have different sizes".into());
            }
            let frame = fitted(frame, fit);
            let repeats = if index + 1 == frames.len() {
                1 + HOLD_SECONDS * fps
            } else {
                1
            };
            for _ in 0..repeats {
                sink(frame.as_raw())?;
            }
            control.progress.store(index + 1, Ordering::Relaxed);
        }
        Ok(())
    })
}

/// How a recorded frame becomes an exported one.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fit {
    /// Already the right size.
    Keep,
    /// Longer than the export size: shrunk smoothly to this size.
    Shrink(u32, u32),
    /// Small enough to fit twice or more: each pixel becomes a sharp square
    /// this many pixels wide, so pixel art stays crisp instead of being
    /// blurred by whatever player scales it up.
    Grow(u32),
}

impl Fit {
    fn size(self, (width, height): (u32, u32)) -> (u32, u32) {
        match self {
            Fit::Keep => (width, height),
            Fit::Shrink(w, h) => (w, h),
            Fit::Grow(steps) => (width * steps, height * steps),
        }
    }
}

/// How a frame of `dimensions` fits `side`, its longest side's limit. Video
/// keeps an even number of steps, so that its half-size color samples stay
/// inside each pixel's square.
fn fit((width, height): (u32, u32), side: u32, video: bool) -> Fit {
    let longest = width.max(height).max(1);
    if longest > side {
        let scale = side as f64 / longest as f64;
        return Fit::Shrink(
            ((width as f64 * scale).round() as u32).max(1),
            ((height as f64 * scale).round() as u32).max(1),
        );
    }
    let mut steps = (side / longest).min(u32::from(u8::MAX));
    if video && steps > 2 && steps % 2 == 1 {
        steps -= 1;
    }
    if steps >= 2 {
        Fit::Grow(steps)
    } else {
        Fit::Keep
    }
}

fn fitted(image: image::RgbaImage, fit: Fit) -> image::RgbaImage {
    match fit {
        Fit::Keep => image,
        Fit::Shrink(w, h) => {
            image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle)
        }
        Fit::Grow(steps) => {
            let (width, height) = image.dimensions();
            let rgba = crate::export_jobs::upscaled(
                image.into_raw(),
                width as usize,
                height as usize,
                steps as u8,
            );
            image::RgbaImage::from_raw(width * steps, height * steps, rgba)
                .expect("an upscaled frame has its pixels")
        }
    }
}

/// The preview enlarges small recordings with sharp pixels and shrinks
/// large ones smoothly.
const PREVIEW_TEXTURE: egui::TextureOptions = egui::TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    ..egui::TextureOptions::LINEAR
};

fn decode_preview(root: &Path, index: usize) -> Result<egui::ColorImage, String> {
    let image = read_frame(root, index)?;
    let size = [image.width() as usize, image.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}

fn duration_label(frames: usize, fps: u32) -> String {
    let seconds =
        frame_time(frames, fps) as f32 / 100.0 + if frames > 0 { HOLD_SECONDS as f32 } else { 0.0 };
    if seconds >= 60.0 {
        format!("{}:{:02}", seconds as u32 / 60, seconds as u32 % 60)
    } else {
        format!("{seconds:.1} s")
    }
}

impl Dudel {
    pub(crate) fn timelapse_status(&self) -> Option<&'static str> {
        let r = &self.recording;
        r.on.then_some(if r.paused {
            "Timelapse paused"
        } else {
            "Recording timelapse"
        })
    }

    /// Record this canvas, or keep adding to its stopped recording.
    pub(crate) fn start_timelapse(&mut self) {
        let dimensions = [self.doc.width, self.doc.height];
        if self.recording.root.is_some() && self.recording.dimensions == dimensions {
            self.recording.resume();
        } else {
            self.recording.begin(dimensions, None);
        }
        self.timelapse.opened = None;
    }
    /// The canvas is being replaced by a new or opened one.
    pub(crate) fn timelapse_document_changed(&mut self, fresh_canvas: bool) {
        self.recording.detach();
        if fresh_canvas && self.timelapse.prefs.record_new {
            self.recording
                .begin([self.doc.width, self.doc.height], Some(self.revision));
        } else if let Some(path) = self.path.clone() {
            self.continue_project_timelapse(&path);
        }
    }
    /// Bring back the latest recording of the project at `path`: one still
    /// recording continues, and a stopped one is shown, ready to continue,
    /// so recording again adds to it instead of starting a separate folder.
    fn continue_project_timelapse(&mut self, path: &Path) {
        // Tests never read the user's documents.
        if cfg!(test) && self.locations.library.is_none() {
            return;
        }
        let folder = self.locations.get(crate::locations::Kind::Timelapses);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            return;
        };
        let found = entries
            .flatten()
            .take(2000)
            .filter_map(|entry| {
                let root = entry.path();
                let inventory = Inventory::read(&root).ok()?;
                let modified = std::fs::metadata(root.join("recording.json"))
                    .and_then(|m| m.modified())
                    .ok()?;
                (inventory.project.as_deref() == Some(path)
                    && [inventory.width, inventory.height] == [self.doc.width, self.doc.height]
                    && inventory.frames > 0
                    && inventory.frames < MAX_FRAMES)
                    .then_some((modified, root, inventory))
            })
            .max_by_key(|(modified, ..)| *modified);
        if let Some((_, root, inventory)) = found {
            self.recording = Recording {
                on: inventory.active,
                root: Some(root),
                frames: inventory.frames,
                bytes: inventory.bytes,
                dimensions: [inventory.width, inventory.height],
                revision: Some(self.revision),
                hash: inventory.hash,
                project: inventory.project,
                message: if inventory.active {
                    format!(
                        "Continuing this project's timelapse ({} frames)",
                        inventory.frames
                    )
                } else {
                    format!(
                        "This project's timelapse ({} frames) is stopped; \
                         Continue recording adds to it",
                        inventory.frames
                    )
                },
                ..Default::default()
            };
        }
    }

    /// The canvas is about to be replaced or closed, perhaps in the same
    /// step that saved it under its first name: its recording must know the
    /// project before it is let go, or reopening the project starts another.
    pub(crate) fn settle_timelapse(&mut self) {
        if self.path.is_some() && self.recording.project != self.path {
            self.recording.join_frame();
            self.claim_timelapse();
        }
    }

    fn claim_timelapse(&mut self) {
        let r = &mut self.recording;
        // Saving as a new path moves the recording to that project.
        if r.worker.is_none()
            && r.frames > 0
            && r.project != self.path
            && let Some(root) = &r.root
        {
            r.project = self.path.clone();
            let _ = r.inventory(r.on).write(root);
        }
        // Its folder takes the project's name, once nothing is reading it.
        let t = &self.timelapse;
        if r.worker.is_none()
            && r.frames > 0
            && r.project.is_some()
            && r.named_for != r.project
            && t.export.is_none()
            && let Some(root) = r.root.clone()
            && t.opened.as_ref().is_none_or(|(opened, _)| *opened != root)
            && t.player
                .decode
                .as_ref()
                .is_none_or(|(decoding, ..)| *decoding != root)
        {
            r.named_for = r.project.clone();
            if let Some(project) = &r.project {
                match follow_project(&root, project) {
                    Ok(Some(moved)) => r.root = Some(moved),
                    Ok(None) => {}
                    Err(e) => r.message = format!("The recording's folder keeps its name: {e}"),
                }
            }
        }
    }

    pub(crate) fn poll_timelapse(&mut self, ctx: &egui::Context) {
        let interval = self.timelapse.prefs.interval;
        let r = &mut self.recording;
        if r.worker.as_ref().is_some_and(|w| w.is_finished()) {
            r.join_frame();
        }
        if (r.on || r.finish) && r.dimensions != [self.doc.width, self.doc.height] {
            r.end("Recording stopped because the canvas size changed; its frames are kept".into());
        }
        if r.on && (r.frames >= MAX_FRAMES || r.bytes >= MAX_BYTES) {
            r.end("Recording limit reached (7,200 frames / 1 GiB); frames are kept".into());
        }
        let stable = (r.finish || !self.performance.defer_optional())
            && !self.stroking
            && !self.render_queue.busy()
            && self.adjustment.is_none()
            && self.opacity_edit.is_none()
            && self.load_job.is_none();
        if (r.on && !r.paused || r.finish) && r.worker.is_none() && stable {
            let due = r.revision != Some(self.revision)
                && (r.last.is_none()
                    || r.finish
                    || r.last
                        .is_some_and(|t| t.elapsed() >= Duration::from_secs(interval)));
            if due && r.root.is_none() {
                let name = self
                    .path
                    .as_deref()
                    .and_then(Path::file_stem)
                    .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned());
                let parent = if cfg!(test) && self.locations.library.is_none() {
                    std::env::temp_dir().join("dudel-test-timelapses")
                } else {
                    self.locations.get(crate::locations::Kind::Timelapses)
                };
                match new_folder(&parent, &name) {
                    Ok(root) => r.root = Some(root),
                    Err(e) => r.end(format!("Recording could not start: {e}")),
                }
            }
            if due && let Some(root) = r.root.clone() {
                r.project = self.path.clone();
                let inventory = r.inventory(r.on);
                let ctx = ctx.clone();
                r.worker = Some(SourceWorker::spawn(self.doc.clone(), move |doc| {
                    let result = capture(doc, &root, inventory);
                    ctx.request_repaint();
                    result
                }));
                r.last = Some(Instant::now());
                r.revision = Some(self.revision);
            }
            if r.finish {
                r.finish = false;
                // The final frame's inventory already says stopped.
                if r.worker.is_none() {
                    r.end(format!("Stopped · {} frames", r.frames));
                } else {
                    r.message = "Stopped".into();
                }
            }
        }
        self.claim_timelapse();
        let r = &self.recording;
        let t = &self.timelapse;
        if r.on || r.worker.is_some() || r.finish || t.export.is_some() {
            ctx.request_repaint_after(Duration::from_millis(
                if r.worker.is_some() || t.export.is_some() || r.finish {
                    100
                } else {
                    1000
                },
            ));
        }
    }

    /// The recording shown in the window: one opened from disk, or this canvas's.
    fn timelapse_source(&self) -> Option<(PathBuf, usize)> {
        self.timelapse.opened.clone().or_else(|| {
            let r = &self.recording;
            r.root
                .clone()
                .filter(|_| r.frames > 0)
                .map(|root| (root, r.frames))
        })
    }

    pub(crate) fn timelapse_ui(&mut self, ctx: &egui::Context) {
        if let Some(export) = self.timelapse.export.take_if(|w| w.is_finished()) {
            let t = &mut self.timelapse;
            match export.join() {
                Ok(Ok(())) => t.message = "Timelapse exported".into(),
                Ok(Err(e)) => {
                    t.exported = None;
                    t.message = format!("Export: {e}");
                }
                Err(_) => {
                    t.exported = None;
                    t.message = "Export worker failed".into();
                }
            }
        }
        if !self.timelapse.open {
            return;
        }
        let source = self.timelapse_source();
        self.poll_player(ctx, source.as_ref());
        let mut open = true;
        let mut action = None;
        egui::Window::new("Timelapse")
            .open(&mut open)
            .default_width(400.0)
            .default_height((ctx.content_rect().height() - 120.0).clamp(200.0, 640.0))
            .max_height((ctx.content_rect().height() - 80.0).max(200.0))
            .vscroll(true)
            .show(ctx, |ui| {
                action = self.timelapse_contents(ui, source.as_ref());
            });
        self.timelapse.open = open;
        match action {
            Some(Action::Open) => {
                let dialog =
                    rfd::AsyncFileDialog::new().set_title("Open a timelapse recording folder");
                let dialog = match self.locations.ensure(crate::locations::Kind::Timelapses) {
                    Some(folder) if !cfg!(test) => dialog.set_directory(folder),
                    _ => dialog,
                };
                self.ask_folder(dialog, |app, root| match Inventory::read(&root) {
                    Ok(inventory) if inventory.frames > 0 => {
                        app.timelapse.opened = Some((root, inventory.frames));
                        app.timelapse.player = Player::default();
                        app.timelapse.message = "Recording opened".into();
                    }
                    Ok(_) => app.timelapse.message = "That recording has no frames".into(),
                    Err(e) => app.timelapse.message = format!("Open recording: {e}"),
                });
            }
            Some(Action::Export(video)) => {
                let name = source
                    .as_ref()
                    .and_then(|(root, _)| root.file_name())
                    .map_or("Timelapse".into(), |s| s.to_string_lossy().into_owned());
                let dialog = self.dialog_in(
                    crate::locations::Kind::Exports,
                    rfd::AsyncFileDialog::new()
                        .add_filter(
                            if video { "MP4 video" } else { "Animated GIF" },
                            if video { &["mp4"] } else { &["gif"] },
                        )
                        .set_file_name(format!("{name}.{}", if video { "mp4" } else { "gif" })),
                );
                self.ask_save_file(dialog, move |app, path| {
                    app.start_timelapse_export(path, video)
                });
            }
            None => {}
        }
    }

    fn start_timelapse_export(&mut self, path: PathBuf, video: bool) {
        if self.timelapse.export.is_some() {
            return;
        }
        let Some((root, frames)) = self.timelapse_source() else {
            return;
        };
        let t = &mut self.timelapse;
        let (fps, side) = (t.prefs.fps, t.prefs.export_side);
        let frames = chosen(frames, fps, t.prefs.length);
        t.export_control = Default::default();
        t.export_frames = frames.len();
        t.exported = Some(path.clone());
        let control = t.export_control.clone();
        t.export = Some(std::thread::spawn(move || {
            if video {
                export_mp4(&root, &frames, fps, side, &path, &control)
            } else {
                export_gif(&root, &frames, fps, side, &path, &control)
            }
        }));
        t.message = "Exporting timelapse in the background…".into();
    }

    /// Advance playback and keep the shown frame decoded.
    fn poll_player(&mut self, ctx: &egui::Context, source: Option<&(PathBuf, usize)>) {
        let fps = self.timelapse.prefs.fps;
        let p = &mut self.timelapse.player;
        let Some((root, frames)) = source else {
            *p = Player::default();
            return;
        };
        p.frame = p.frame.min(frames - 1);
        if p.playing {
            let now = Instant::now();
            let elapsed = p.tick.map_or(0.0, |t| (now - t).as_secs_f32());
            p.tick = Some(now);
            p.phase += elapsed * fps as f32;
            // Never skip ahead of decoding: a slow disk slows playback.
            if p.phase >= 1.0 && p.shown.as_ref() == Some(&(root.clone(), p.frame)) {
                p.phase = p.phase.min(1.0) - 1.0;
                if p.frame + 1 < *frames {
                    p.frame += 1;
                } else {
                    p.playing = false;
                }
            }
            ctx.request_repaint();
        } else {
            p.tick = None;
            p.phase = 0.0;
        }
        if p.decode.as_ref().is_some_and(|(.., w)| w.is_finished()) {
            let (decoded_root, index, worker) = p.decode.take().unwrap();
            if let Ok(Ok(image)) = worker.join() {
                match &mut p.texture {
                    Some(texture) => texture.set(image, PREVIEW_TEXTURE),
                    None => {
                        p.texture =
                            Some(ctx.load_texture("timelapse frame", image, PREVIEW_TEXTURE))
                    }
                }
                p.shown = Some((decoded_root, index));
            }
        }
        let wanted = (root.clone(), p.frame);
        if p.decode.is_none() && p.shown.as_ref() != Some(&wanted) {
            let ctx = ctx.clone();
            let (decode_root, index) = wanted;
            let worker_root = decode_root.clone();
            p.decode = Some((
                decode_root,
                index,
                std::thread::spawn(move || {
                    let image = decode_preview(&worker_root, index);
                    ctx.request_repaint();
                    image
                }),
            ));
        }
    }

    fn timelapse_contents(
        &mut self,
        ui: &mut egui::Ui,
        source: Option<&(PathBuf, usize)>,
    ) -> Option<Action> {
        let mut action = None;
        let busy = self.timelapse.export.is_some();
        let r = &self.recording;
        let state = if r.on {
            if r.paused { "Paused" } else { "Recording" }
        } else if r.finish || r.worker.is_some() {
            "Saving the last frame…"
        } else if r.frames > 0 {
            "Stopped"
        } else {
            "Not recording"
        };
        let continues = r.root.is_some() && r.dimensions == [self.doc.width, self.doc.height];
        ui.horizontal(|ui| {
            let r = &self.recording;
            if r.on && !r.paused {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    rect.center(),
                    5.0,
                    egui::Color32::from_rgb(220, 60, 60),
                );
            }
            ui.strong(state);
            if r.frames > 0 {
                ui.label(format!(
                    "{} frames · {}",
                    r.frames,
                    crate::locations::human_bytes(r.bytes)
                ));
            }
        });
        ui.horizontal(|ui| {
            let r = &mut self.recording;
            if r.on {
                let label = if r.paused { "Resume" } else { "Pause" };
                if ui
                    .button(label)
                    .on_hover_text(
                        "Breaks are skipped anyway; pause to leave out work you will undo",
                    )
                    .clicked()
                {
                    r.paused = !r.paused;
                }
                if ui.button("Stop").clicked() {
                    r.stop();
                }
            } else if !r.finish && r.worker.is_none() {
                if continues {
                    if ui.button("Continue recording").clicked() {
                        self.start_timelapse();
                    }
                    if ui
                        .button("New recording")
                        .on_hover_text("Start a separate recording; the stopped one is kept")
                        .clicked()
                    {
                        self.recording.root = None;
                        self.start_timelapse();
                    }
                } else if ui
                    .button("Record this canvas")
                    .on_hover_text("Frames are captured as the artwork changes")
                    .clicked()
                {
                    self.start_timelapse();
                }
            }
        });
        if !self.recording.message.is_empty() {
            ui.label(egui::RichText::new(&self.recording.message).weak());
        }
        let prefs = &mut self.timelapse.prefs;
        egui::CollapsingHeader::new("Recording options")
            .id_salt("timelapse recording options")
            .show(ui, |ui| {
                ui.add(
                    egui::Slider::new(&mut prefs.interval, 2..=60).text("seconds between frames"),
                )
                .on_hover_text("Unchanged artwork is never captured twice, so breaks are skipped");
                ui.checkbox(&mut prefs.record_new, "Record new canvases automatically")
                    .on_hover_text("Also offered when you create a canvas");
            });
        ui.separator();

        let t = &mut self.timelapse;
        ui.horizontal(|ui| {
            ui.strong(if t.opened.is_some() {
                "Opened recording"
            } else {
                "Playback"
            });
            if t.opened.is_some() && ui.small_button("Back to this canvas").clicked() {
                t.opened = None;
                t.player = Player::default();
            }
        });
        match source {
            None => {
                ui.label(egui::RichText::new("Frames appear here as you paint. Start recording above, or open an earlier recording.").weak());
            }
            Some((root, frames)) => {
                let p = &mut t.player;
                if let Some(texture) = &p.texture {
                    let size = texture.size_vec2();
                    let width = ui.available_width().min(480.0);
                    // Short windows keep the playback controls in view.
                    let height = (ui.ctx().content_rect().height() * 0.3).clamp(100.0, 240.0);
                    let scale = (width / size.x).min(height / size.y);
                    // Whole steps when enlarging, so every pixel is as wide
                    // as the next.
                    let scale = if scale >= 1.0 { scale.floor() } else { scale };
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Image::new((texture.id(), size * scale)));
                    });
                } else {
                    ui.add_space(4.0);
                    ui.spinner();
                }
                ui.horizontal(|ui| {
                    let label = if p.playing { "Pause" } else { "Play" };
                    if ui.button(label).clicked() {
                        if !p.playing && p.frame + 1 >= *frames {
                            p.frame = 0;
                        }
                        p.playing = !p.playing;
                    }
                    // The count is laid out first, on the right, and the
                    // slider takes exactly what is left. Sizing the slider
                    // from a guess at the count's width made each frame a
                    // little wider than the window, which grew to fit it,
                    // frame after frame, once counts reached four digits.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Clear of the window's scroll bar.
                        ui.add_space(ui.spacing().scroll.bar_width);
                        ui.label(format!("{} / {frames}", p.frame + 1));
                        let mut frame = p.frame + 1;
                        ui.spacing_mut().slider_width = (ui.available_width() - 4.0).max(60.0);
                        if ui
                            .add(egui::Slider::new(&mut frame, 1..=*frames).show_value(false))
                            .changed()
                        {
                            p.frame = frame - 1;
                            p.playing = false;
                        }
                    });
                });
                ui.add(egui::Slider::new(&mut t.prefs.fps, 1..=30).text("frames per second"))
                    .on_hover_text("Playback speed for the preview and exported timelapse");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("timelapse length")
                        .selected_text(
                            LENGTHS
                                .iter()
                                .find(|(length, _)| *length == t.prefs.length)
                                .map_or("", |(_, label)| label),
                        )
                        .show_ui(ui, |ui| {
                            for (length, label) in LENGTHS {
                                ui.selectable_value(&mut t.prefs.length, length, label);
                            }
                        })
                        .response
                        .on_hover_text(
                            "Fit a long session into a set time by keeping evenly spaced \
                             frames. Each frame keeps its full quality, and a shorter video \
                             is a smaller file.",
                        );
                    ui.label("exported length");
                });
                let kept = chosen(*frames, t.prefs.fps, t.prefs.length).len();
                let length = duration_label(kept, t.prefs.fps);
                ui.label(
                    egui::RichText::new(if kept < *frames {
                        format!(
                            "{length} long: {kept} of the {frames} frames, evenly spaced, \
                             then the finished artwork for {HOLD_SECONDS} s."
                        )
                    } else {
                        format!(
                            "{length} long at this speed, holding the finished artwork \
                             for {HOLD_SECONDS} s."
                        )
                    })
                    .weak(),
                );
                ui.separator();
                ui.strong("Export");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("timelapse gif size")
                        .selected_text(
                            EXPORT_SIDES
                                .iter()
                                .find(|(side, _)| *side == t.prefs.export_side)
                                .map_or("", |(_, label)| label),
                        )
                        .show_ui(ui, |ui| {
                            for (side, label) in EXPORT_SIDES {
                                ui.selectable_value(&mut t.prefs.export_side, side, label);
                            }
                        });
                    if ui
                        .add_enabled(!busy, egui::Button::new("Export GIF…"))
                        .clicked()
                    {
                        action = Some(Action::Export(false));
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Export MP4\u{2026}"))
                        .on_hover_text("H.264 video for sharing")
                        .clicked()
                    {
                        action = Some(Action::Export(true));
                    }
                    if ui
                        .button("Show frames")
                        .on_hover_text("PNG frames, for full-color video in a video editor")
                        .clicked()
                        && let Err(e) = crate::locations::reveal(root)
                    {
                        t.message = e;
                    }
                });
                if busy {
                    let done = t.export_control.progress.load(Ordering::Relaxed);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::ProgressBar::new(done as f32 / t.export_frames.max(1) as f32)
                                .text(format!("{done}/{} frames", t.export_frames))
                                .desired_width((ui.available_width() - 70.0).max(80.0)),
                        );
                        if ui.button("Cancel").clicked() {
                            t.export_control.cancel.store(true, Ordering::Relaxed);
                        }
                    });
                }
            }
        }
        ui.horizontal(|ui| {
            if !t.message.is_empty() {
                ui.label(&t.message);
            }
            if !busy
                && let Some(folder) = t.exported.as_deref().and_then(Path::parent)
                && ui.small_button("Show in folder").clicked()
                && let Err(e) = crate::locations::reveal(folder)
            {
                t.message = e;
            }
        });
        ui.separator();
        if ui
            .add_enabled(!busy, egui::Button::new("Open an earlier recording…"))
            .clicked()
        {
            action = Some(Action::Open);
        }
        ui.small("Records artwork only (no panels, cursor or reference images), up to 1280 px. Small canvases, such as pixel art, export enlarged in whole steps so every pixel stays sharp. GIF uses a limited palette.");
        action
    }
}

impl Dudel {
    /// Capture mode: this canvas as a short recording in progress, so the
    /// window shows its player.
    pub(crate) fn sample_timelapse(&mut self) -> Result<(), String> {
        let root =
            std::env::temp_dir().join(format!("dudel-capture-timelapse-{}", std::process::id()));
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let mut image = image::RgbImage::from_pixel(320, 240, image::Rgb([250, 248, 240]));
        let mut bytes = 0;
        for frame in 0..24u32 {
            for x in frame * 12..frame * 12 + 12 {
                for y in 0..240 {
                    let wave = 120.0 + 60.0 * (x as f32 / 30.0).sin();
                    if (y as f32 - wave).abs() < 14.0 {
                        image.put_pixel(x.min(319), y, image::Rgb([40 + frame as u8 * 8, 90, 200]));
                    }
                }
            }
            bytes += write_png(&image, &frame_path(&root, frame as usize))?;
        }
        self.recording = Recording {
            on: true,
            root: Some(root),
            frames: 24,
            bytes,
            dimensions: [self.doc.width, self.doc.height],
            revision: Some(self.revision),
            last: Some(Instant::now()),
            ..Default::default()
        };
        self.timelapse.player.frame = 17;
        Ok(())
    }
}

/// A timelapse button that needs a file dialog, asked after the window draws.
enum Action {
    Open,
    Export(bool),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_recording_keeps_the_window_width() {
        // Four-digit counts once made every frame a little wider than the
        // window, which grew to fit until it reached the edge of the screen.
        let mut app = Dudel::default();
        app.sample_timelapse().unwrap();
        app.recording.frames = 1145;
        app.timelapse.player.frame = 705;
        app.timelapse.open = true;
        let ctx = egui::Context::default();
        let mut widths = Vec::new();
        for frame in 0..30 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1920.0, 1080.0),
                )),
                time: Some(frame as f64 / 30.0),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                crate::workspace::theme(ctx);
                app.timelapse_ui(ctx)
            });
            widths.push(
                ctx.memory(|m| m.area_rect(egui::Id::new("Timelapse")))
                    .unwrap()
                    .width(),
            );
        }
        let settled = widths[5];
        assert!(widths[5..].iter().all(|w| *w == settled), "{widths:?}");
        assert!(settled < 600.0, "{widths:?}");
        let _ = std::fs::remove_dir_all(app.recording.root.unwrap());
    }
    fn full_frame_reference(doc: &Document) -> image::RgbImage {
        let image = image::RgbaImage::from_raw(
            doc.width as u32,
            doc.height as u32,
            doc.composite_bounds_checked([0, 0, doc.width, doc.height], false, || Ok(()))
                .unwrap(),
        )
        .unwrap();
        let image = image::DynamicImage::ImageRgba8(image);
        if doc.width.max(doc.height) > MAX_SIDE as usize {
            image.thumbnail(MAX_SIDE, MAX_SIDE).to_rgb8()
        } else {
            image.to_rgb8()
        }
    }

    #[test]
    #[ignore = "requires ffprobe on PATH"]
    fn mp4_export_dimensions_duration_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..3 {
            write_png(
                &image::RgbImage::from_pixel(7, 5, image::Rgb([index as u8 * 100, 50, 25])),
                &frame_path(dir.path(), index),
            )
            .unwrap();
        }
        let path = dir.path().join("video.mp4");
        export_mp4(dir.path(), &[0, 1, 2], 10, 960, &path, &Default::default()).unwrap();
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let value: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(value["streams"][0]["codec_name"], "h264");
        // 7 × 5 grows 136 times (137, kept even), sharp and padded to nothing.
        assert_eq!(value["streams"][0]["width"], 952);
        assert_eq!(value["streams"][0]["height"], 680);
        let duration: f64 = value["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((duration - 2.3).abs() < 0.15, "{duration}");
        let original = std::fs::read(&path).unwrap();
        let control = crate::plugins::FilterControl::default();
        control.cancel.store(true, Ordering::Relaxed);
        assert!(export_mp4(dir.path(), &[0, 1, 2], 10, 960, &path, &control).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // A malformed sequence must not publish an otherwise valid partial video.
        write_png(&image::RgbImage::new(9, 5), &frame_path(dir.path(), 1)).unwrap();
        assert!(export_mp4(dir.path(), &[0, 1, 2], 10, 960, &path, &Default::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        for index in 0..100 {
            write_png(
                &image::RgbImage::new(64, 64),
                &frame_path(dir.path(), index),
            )
            .unwrap();
        }
        let control = crate::plugins::FilterControl::default();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                export_mp4(
                    dir.path(),
                    &(0..100).collect::<Vec<_>>(),
                    10,
                    960,
                    &path,
                    &control,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while control.progress.load(Ordering::Relaxed) == 0 && !worker.is_finished() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            control.cancel.store(true, Ordering::Relaxed);
            assert!(worker.join().unwrap().is_err());
        });
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::write(&path, b"DUDL source bytes").unwrap();
        assert!(export_mp4(dir.path(), &[0, 1, 2], 10, 960, &path, &Default::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"DUDL source bytes");
    }

    #[test]
    fn banded_frames_match_full_thumbnail_at_odd_edges_masks_and_thin_dimensions() {
        for (width, height) in [
            (71, 69),
            (1280, 71),
            (1281, 73),
            (137, 1411),
            (1, 4096),
            (4096, 1),
        ] {
            let mut doc = Document::new(width, height);
            doc.version = 2;
            for layer in 0..2 {
                if layer == 1 {
                    doc.add_layer();
                }
                let mut writer = doc.layers[layer].pixels.writer();
                for y in 0..height {
                    for x in 0..width {
                        writer.rgba_mut_at(width, x, y).copy_from_slice(&[
                            (x * 17 + y * 7) as u8,
                            (x + y * 11) as u8,
                            (y + layer * 37) as u8,
                            (x * 13 + y * 3) as u8,
                        ]);
                    }
                }
            }
            doc.layers[1].opacity = 0.73;
            doc.layers[1].blend = crate::blend::BlendMode::Multiply;
            doc.layers[1].mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: true,
                linked: true,
                coverage: crate::layer_mask::Mask::new(width, height, 91).unwrap(),
            });
            let (actual, peak) = render_frame_banded(&doc).unwrap();
            assert_eq!(actual, full_frame_reference(&doc), "{width} x {height}");
            assert!(peak <= width * 206 * 4);
        }
    }

    #[test]
    #[ignore = "release performance probe; run without compiler or other benchmark workloads"]
    fn benchmark_banded_frames() {
        let mut cases = Vec::new();
        for (width, height, dense) in [
            (512, 512, false),
            (2048, 2048, false),
            (4096, 4096, false),
            (4096, 4096, true),
        ] {
            let mut doc = Document::new(width, height);
            if dense {
                let row: Vec<u8> = (0..width)
                    .flat_map(|x| [(x * 7) as u8, (x * 11) as u8, 71, 153])
                    .collect();
                let mut writer = doc.layers[0].pixels.writer();
                for y in 0..height {
                    writer.write_region(width, [0, y, width, y + 1], &row);
                }
            }
            doc.dab(
                700.0,
                700.0,
                1.0,
                &crate::brush::Brush {
                    size: 300.0,
                    ..Default::default()
                },
                [17, 81, 193],
                false,
            );
            let mut before = Vec::new();
            let mut after = Vec::new();
            let mut peak = 0;
            for _ in 0..3 {
                let started = Instant::now();
                let reference = full_frame_reference(&doc);
                before.push(started.elapsed().as_secs_f64() * 1000.0);
                let started = Instant::now();
                let (actual, band) = render_frame_banded(&doc).unwrap();
                after.push(started.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(actual, reference);
                peak = peak.max(band);
            }
            cases.push(serde_json::json!({"canvas":[width,height], "dense":dense,
                "full_frame_ms":before,"banded_frame_ms":after,"full_source_rgba_bytes":width*height*4,
                "peak_band_rgba_bytes":peak,"exact_rgb_match":true}));
        }
        std::fs::create_dir_all("artifacts").unwrap();
        std::fs::write("artifacts/timelapse-bands.json", serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"Three sequential full/banded trials per case in the same build; opaque RGB output verified byte-for-byte outside timing. Band bytes exclude output RGB, compositing scratch, source snapshots and allocator overhead; not RSS or pen latency.","cases":cases
        })).unwrap()).unwrap();
    }
    fn settle(app: &mut Dudel, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.recording.worker.is_some() {
            assert!(Instant::now() < deadline, "frame worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
            app.poll_timelapse(ctx);
        }
    }
    fn library(app: &mut Dudel) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        app.locations.library = Some(root.path().to_owned());
        root
    }

    #[test]
    fn recording_waits_for_settlement_skips_repeats_and_stops_on_disk() {
        let mut app = Dudel {
            doc: Document::new(4, 4),
            ..Default::default()
        };
        let _library = library(&mut app);
        let ctx = egui::Context::default();
        app.start_timelapse();
        app.stroking = true;
        app.poll_timelapse(&ctx);
        assert!(app.recording.worker.is_none());
        app.stroking = false;
        app.poll_timelapse(&ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.recording.frames, 1);
        let root = app.recording.root.clone().unwrap();
        assert!(root.starts_with(app.locations.get(crate::locations::Kind::Timelapses)));
        // Idle time and changes that leave the artwork as it was add nothing.
        app.recording.last = Some(Instant::now() - Duration::from_secs(60));
        app.poll_timelapse(&ctx);
        assert!(app.recording.worker.is_none());
        app.revision += 1;
        app.poll_timelapse(&ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.recording.frames, 1);
        app.recording.paused = true;
        app.doc.layers[0].pixels = [9, 9, 9, 255].repeat(16).into();
        app.revision += 1;
        app.poll_timelapse(&ctx);
        assert!(app.recording.worker.is_none());
        // Stopping captures the latest artwork and marks the recording stopped.
        app.recording.stop();
        app.poll_timelapse(&ctx);
        settle(&mut app, &ctx);
        assert_eq!(app.recording.frames, 2);
        assert!(frame_path(&root, 1).exists() && !app.recording.recording());
        let inventory = Inventory::read(&root).unwrap();
        assert_eq!((inventory.frames, inventory.active), (2, false));
        assert_eq!([inventory.width, inventory.height], [4, 4]);
    }

    #[test]
    fn a_fresh_canvas_records_from_its_first_change_and_reopening_continues() {
        let mut app = Dudel::default();
        let _library = library(&mut app);
        app.timelapse.prefs.record_new = true;
        let ctx = egui::Context::default();
        app.apply_action(crate::document_actions::Action::New(6, 5, 300.0));
        assert!(app.recording.recording());
        app.poll_timelapse(&ctx);
        assert!(
            app.recording.root.is_none(),
            "an untouched canvas leaves no folder"
        );
        app.doc.layers[0].pixels = [200, 10, 10, 255].repeat(30).into();
        app.revision += 1;
        app.poll_timelapse(&ctx);
        settle(&mut app, &ctx);
        let untitled = app.recording.root.clone().unwrap();
        assert!(
            untitled
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Untitled ")
        );
        // Saving names the project in the inventory, and the folder takes
        // the project's name, keeping the date it was started.
        let project = app.locations.library.clone().unwrap().join("art.dudel");
        app.path = Some(project.clone());
        app.poll_timelapse(&ctx);
        let root = app.recording.root.clone().unwrap();
        assert!(!untitled.exists());
        assert_eq!(
            root.file_name().unwrap().to_string_lossy(),
            format!(
                "art {}",
                &untitled.file_name().unwrap().to_string_lossy()["Untitled ".len()..]
            )
        );
        assert_eq!(
            Inventory::read(&root).unwrap().project,
            Some(project.clone())
        );
        let reopen = |app: &mut Dudel| {
            app.apply_action(crate::document_actions::Action::New(6, 5, 300.0));
            assert_ne!(app.recording.root.as_ref(), Some(&root));
            let document = app.doc.clone();
            app.apply_action(crate::document_actions::Action::Replace {
                document: Box::new(document),
                path: Some(project.clone()),
                modified: false,
                status: "Opened",
            });
        };
        // Opening another canvas leaves it active; reopening the project continues it.
        reopen(&mut app);
        assert!(app.recording.recording());
        assert_eq!(
            (app.recording.root.as_ref(), app.recording.frames),
            (Some(&root), 1)
        );
        // A stopped recording comes back too, ready to continue, so
        // recording again adds to it instead of starting a separate folder.
        app.recording.stop();
        app.poll_timelapse(&ctx);
        settle(&mut app, &ctx);
        assert!(!Inventory::read(&root).unwrap().active);
        reopen(&mut app);
        assert!(!app.recording.recording());
        assert_eq!(
            (app.recording.root.as_ref(), app.recording.frames),
            (Some(&root), 1)
        );
        app.start_timelapse();
        assert!(app.recording.recording());
        assert_eq!(app.recording.root.as_ref(), Some(&root));
    }

    /// Saving on the way to another canvas, out of a tab or out of Dudel
    /// leaves no frame in which to notice the project's name.
    #[test]
    fn saving_on_the_way_out_keeps_the_recording_with_its_project() {
        use crate::document_actions::Action;
        for frame_in_flight in [false, true] {
            for leave in [
                (|| Action::New(6, 5, 300.0)) as fn() -> Action,
                || Action::CloseTab,
                || Action::Close,
            ] {
                let mut app = Dudel::default();
                let library = library(&mut app);
                let ctx = egui::Context::default();
                app.apply_action(Action::New(6, 5, 300.0));
                app.start_timelapse();
                app.doc.layers[0].pixels = [200, 10, 10, 255].repeat(30).into();
                app.revision += 1;
                app.poll_timelapse(&ctx);
                if !frame_in_flight {
                    settle(&mut app, &ctx);
                }
                let document = app.doc.clone();
                let project = library.path().join("art.dudel");
                app.modified = true;
                app.save_to(project.clone(), Some(leave()));
                let deadline = Instant::now() + Duration::from_secs(10);
                while app.save_job.is_some() || app.pending_action.is_some() {
                    assert!(Instant::now() < deadline, "save did not finish");
                    std::thread::sleep(Duration::from_millis(1));
                    app.poll_save(&ctx);
                }
                // As the next start of Dudel would open it.
                let mut app = Dudel::default();
                app.locations.library = Some(library.path().to_owned());
                app.apply_action(Action::Replace {
                    document: Box::new(document),
                    path: Some(project.clone()),
                    modified: false,
                    status: "Opened",
                });
                assert!(app.recording.recording(), "in flight: {frame_in_flight}");
                assert_eq!(app.recording.frames, 1);
                let folders: Vec<_> =
                    std::fs::read_dir(app.locations.get(crate::locations::Kind::Timelapses))
                        .unwrap()
                        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                        .collect();
                assert!(
                    folders.len() == 1 && folders[0].starts_with("art "),
                    "{folders:?}"
                );
            }
        }
    }

    #[test]
    fn a_long_recording_fits_a_length_with_evenly_spaced_frames() {
        assert_eq!(chosen(5, 12, None), [0, 1, 2, 3, 4]);
        assert_eq!(chosen(5, 12, Some(15)), [0, 1, 2, 3, 4], "short enough");
        // 15 s at 12 frames per second, less the 2 s hold: 156 frames.
        let fitted = chosen(1145, 12, Some(15));
        assert_eq!(fitted.len(), 156);
        assert_eq!((fitted[0], fitted[155]), (0, 1144));
        assert!(fitted.windows(2).all(|w| (7..=8).contains(&(w[1] - w[0]))));
        assert_eq!(duration_label(fitted.len(), 12), "15.0 s");
        assert_eq!(chosen(7200, 1, Some(15)).len(), 13);
        assert_eq!(chosen(7200, 30, Some(60)).len(), 1740);
    }

    #[test]
    fn gif_export_encodes_changes_in_order_and_never_overwrites_artwork() {
        use image::AnimationDecoder;
        let root = tempfile::tempdir().unwrap();
        let colors = [[255, 0, 0], [255, 0, 0], [0, 0, 255]];
        for (i, color) in colors.iter().enumerate() {
            let mut image = image::RgbImage::from_pixel(3, 2, image::Rgb([255, 255, 255]));
            image.put_pixel(0, 0, image::Rgb(*color));
            write_png(&image, &frame_path(root.path(), i)).unwrap();
        }
        let gif = root.path().join("test.gif");
        export_gif(root.path(), &[0, 1, 2], 10, 960, &gif, &Default::default()).unwrap();
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(&gif).unwrap(),
        ))
        .unwrap();
        let frames = decoder.into_frames().collect_frames().unwrap();
        // The repeated frame lengthens the first; the last holds two seconds.
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].delay().numer_denom_ms(), (200, 1));
        assert_eq!(frames[1].delay().numer_denom_ms(), (2100, 1));
        // A tiny canvas is enlarged with sharp pixels: 255 steps at most.
        assert_eq!(frames[0].buffer().dimensions(), (765, 510));
        assert!(frames[0].buffer().get_pixel(0, 0)[0] > 240);
        assert!(frames[1].buffer().get_pixel(254, 254)[2] > 240);
        assert!(
            frames[1].buffer().get_pixel(255, 0)[1] > 240,
            "the next pixel starts sharp"
        );
        assert!(
            frames[1].buffer().get_pixel(600, 300)[1] > 240,
            "unchanged pixels show through"
        );
        let original = std::fs::read(&gif).unwrap();
        let canceled = crate::plugins::FilterControl::default();
        canceled.cancel.store(true, Ordering::Relaxed);
        assert!(export_gif(root.path(), &[0, 1, 2], 10, 960, &gif, &canceled).is_err());
        assert_eq!(std::fs::read(&gif).unwrap(), original);
        let protected = root.path().join("art.gif");
        std::fs::write(&protected, b"DUDEL artwork").unwrap();
        assert!(
            export_gif(
                root.path(),
                &[0, 1, 2],
                10,
                960,
                &protected,
                &Default::default()
            )
            .is_err()
        );
        assert_eq!(std::fs::read(protected).unwrap(), b"DUDEL artwork");
    }

    #[test]
    fn small_recordings_export_enlarged_with_sharp_pixels() {
        // Pixel art grows by whole steps up to the export size.
        assert_eq!(fit((32, 24), 960, false), Fit::Grow(30));
        assert_eq!(fit((64, 64), 960, true), Fit::Grow(14), "15 kept even");
        assert_eq!(fit((300, 200), 960, true), Fit::Grow(2));
        assert_eq!(fit((300, 200), 960, false), Fit::Grow(3));
        assert_eq!(fit((32, 24), 960, false).size((32, 24)), (960, 720));
        // Paintings that nearly fit stay as they are; larger ones shrink.
        assert_eq!(fit((700, 500), 960, false), Fit::Keep);
        assert_eq!(fit((1280, 640), 960, false), Fit::Shrink(960, 480));
        let mut image = image::RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 0, 255]));
        image.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        let grown = fitted(image, Fit::Grow(3));
        assert_eq!(grown.dimensions(), (6, 3));
        let row: Vec<u8> = (0..6).map(|x| grown.get_pixel(x, 2)[0]).collect();
        assert_eq!(row, [0, 0, 0, 255, 255, 255], "no blended edge");
    }

    #[test]
    fn folder_dates_and_playback_times_are_exact() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_792_281_600), "2026-10-18");
        // 12 fps cannot be whole centiseconds; times stay on schedule anyway.
        assert_eq!(frame_time(12, 12), 100);
        assert_eq!(frame_time(1, 12), 8);
        assert_eq!(duration_label(12, 12), "3.0 s");
        let legacy: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy, Prefs::default());
    }
}
