//! Fill reference choices are session-local; explicit layer identities never fall
//! back to an index or another layer when the reference is missing.
use crate::{Dudel, document::LayerId, egui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Source {
    #[default]
    Active,
    Visible,
    Layer(LayerId),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Options {
    pub close_gap: u8,
    pub expand: i8,
    /// Preserve soft linework while replacing the background at its edge.
    pub fill_edges: bool,
}
impl Options {
    pub(crate) fn validate(self) -> Result<(), String> {
        if self.close_gap > 8 || !(-16..=16).contains(&self.expand) {
            Err("Unsupported fill gap closing or expansion".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub point: [usize; 2],
    pub color: [u8; 3],
    pub opacity: f32,
    pub tolerance: u8,
    pub source: Source,
    pub options: Options,
}
impl Request {
    pub(crate) fn validate(self) -> Result<(), String> {
        self.options.validate()?;
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err("Fill opacity must be finite and between zero and one".into());
        }
        Ok(())
    }
}
pub(crate) fn equivalent(pixel: &[u8; 4], target: &[u8; 4], tolerance: u8) -> bool {
    pixel[3].abs_diff(target[3]) <= tolerance
        && (0..3).all(|c| {
            let a = (u16::from(pixel[c]) * u16::from(pixel[3]) / 255) as u8;
            let b = (u16::from(target[c]) * u16::from(target[3]) / 255) as u8;
            a.abs_diff(b) <= tolerance
        })
}
pub(crate) fn check_cancel(cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<(), String> {
    if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
        Err("Fill canceled".into())
    } else {
        Ok(())
    }
}

pub(crate) fn visit(bits: &mut [u64], index: usize) -> bool {
    let mask = 1u64 << (index % 64);
    let word = &mut bits[index / 64];
    let fresh = *word & mask == 0;
    *word |= mask;
    fresh
}
pub(crate) fn contains(bits: &[u64], index: usize) -> bool {
    bits[index / 64] & (1u64 << (index % 64)) != 0
}
/// Separable square morphology, with clipped windows at the canvas edge.
/// Sliding counts keep work linear in image area rather than radius squared.
fn morphology(
    input: &[u64],
    width: usize,
    height: usize,
    radius: usize,
    dilate: bool,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u64>, String> {
    let mut source = input.to_vec();
    for vertical in [false, true] {
        let mut output = vec![0u64; input.len()];
        let (lines, length) = if vertical {
            (width, height)
        } else {
            (height, width)
        };
        for line in 0..lines {
            check_cancel(cancel)?;
            let index = |p| {
                if vertical {
                    p * width + line
                } else {
                    line * width + p
                }
            };
            let mut count = (0..=radius.min(length - 1))
                .filter(|&p| contains(&source, index(p)))
                .count();
            for p in 0..length {
                let left = p.saturating_sub(radius);
                let right = (p + radius + 1).min(length);
                if (dilate && count > 0) || (!dilate && count == right - left) {
                    visit(&mut output, index(p));
                }
                if p >= radius {
                    count -= usize::from(contains(&source, index(p - radius)));
                }
                if p + radius + 1 < length {
                    count += usize::from(contains(&source, index(p + radius + 1)));
                }
            }
        }
        source = output;
    }
    check_cancel(cancel)?;
    Ok(source)
}
/// Visit each four-connected horizontal run exactly once. The immutable
/// eligibility source must not change when `paint` writes the destination.
/// One bit per pixel records completed runs; the frontier contains spans, not
/// individual pixels. Checks inside long rows keep cancellation cooperative.
pub(crate) fn connected_spans(
    dimensions: [usize; 2],
    seed: usize,
    eligible: impl Fn(usize) -> bool,
    mut paint: impl FnMut(usize, usize, usize),
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u64>, String> {
    let [width, height] = dimensions;
    check_cancel(cancel)?;
    let mut connected = vec![0; (width * height).div_ceil(64)];
    let mut pending = vec![(seed / width, seed % width, seed % width + 1)];
    while let Some((y, start, end)) = pending.pop() {
        check_cancel(cancel)?;
        let base = y * width;
        let mut x = start;
        while x < end {
            if x & 1023 == 0 {
                check_cancel(cancel)?;
            }
            if contains(&connected, base + x) || !eligible(base + x) {
                x += 1;
                continue;
            }
            visit(&mut connected, base + x);
            let mut left = x;
            while left > 0 && !contains(&connected, base + left - 1) && eligible(base + left - 1) {
                left -= 1;
                visit(&mut connected, base + left);
                if left & 1023 == 0 {
                    check_cancel(cancel)?;
                }
            }
            let mut right = x + 1;
            while right < width && !contains(&connected, base + right) && eligible(base + right) {
                visit(&mut connected, base + right);
                right += 1;
                if right & 1023 == 0 {
                    check_cancel(cancel)?;
                }
            }
            check_cancel(cancel)?;
            paint(y, left, right);
            if y > 0 {
                pending.push((y - 1, left, right));
            }
            if y + 1 < height {
                pending.push((y + 1, left, right));
            }
            x = right;
        }
    }
    check_cancel(cancel)?;
    Ok(connected)
}

/// The ordinary bucket blend, shared by scanline and advanced fills.
pub(crate) fn paint_pixel(pixel: &mut [u8], color: [u8; 3], opacity: f32, alpha_locked: bool) {
    if alpha_locked {
        if pixel[3] > 0 {
            for c in 0..3 {
                pixel[c] =
                    (color[c] as f32 * opacity + pixel[c] as f32 * (1.0 - opacity)).round() as u8;
            }
        }
    } else if opacity == 1.0 {
        pixel[..3].copy_from_slice(&color);
        pixel[3] = 255;
    } else {
        let old_alpha = pixel[3] as f32 / 255.0;
        let alpha = opacity + old_alpha * (1.0 - opacity);
        if alpha > 0.0 {
            for c in 0..3 {
                pixel[c] = ((color[c] as f32 * opacity
                    + pixel[c] as f32 * old_alpha * (1.0 - opacity))
                    / alpha)
                    .round() as u8;
            }
            pixel[3] = (alpha * 255.0).round() as u8;
        }
    }
}

pub(crate) fn region_mask(
    dimensions: [usize; 2],
    seed: usize,
    options: Options,
    eligible: impl Fn(usize) -> bool,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u64>, String> {
    options.validate()?;
    let [width, height] = dimensions;
    if options.close_gap == 0 && options.expand == 0 {
        return connected_spans(dimensions, seed, eligible, |_, _, _| {}, cancel);
    }
    let mut passable = vec![0u64; (width * height).div_ceil(64)];
    for index in 0..width * height {
        if index & 1023 == 0 {
            check_cancel(cancel)?;
        }
        if eligible(index) {
            visit(&mut passable, index);
        }
    }
    if options.close_gap > 0 {
        let radius = usize::from(options.close_gap);
        passable = morphology(&passable, width, height, radius, false, cancel)?;
        passable = morphology(&passable, width, height, radius, true, cancel)?;
    }
    let mut connected = connected_spans(
        dimensions,
        seed,
        |index| contains(&passable, index),
        |_, _, _| {},
        cancel,
    )?;
    if options.expand != 0 {
        connected = morphology(
            &connected,
            width,
            height,
            usize::from(options.expand.unsigned_abs()),
            options.expand > 0,
            cancel,
        )?;
    }
    check_cancel(cancel)?;
    Ok(connected)
}

fn distance(pixel: [u8; 4], seed: [u8; 4]) -> u8 {
    (0..3).fold(pixel[3].abs_diff(seed[3]), |distance, c| {
        let a = u16::from(pixel[c]) * u16::from(pixel[3]) / 255;
        let b = u16::from(seed[c]) * u16::from(seed[3]) / 255;
        distance.max(a.abs_diff(b) as u8)
    })
}

fn same_ramp(seed: [u8; 4], from: [u8; 4], to: [u8; 4]) -> bool {
    let vector = |pixel: [u8; 4]| {
        std::array::from_fn::<_, 4, _>(|c| {
            if c == 3 {
                f64::from(pixel[3]) - f64::from(seed[3])
            } else {
                f64::from(pixel[c]) * f64::from(pixel[3]) / 255.0
                    - f64::from(seed[c]) * f64::from(seed[3]) / 255.0
            }
        })
    };
    let (a, b) = (vector(from), vector(to));
    let dot: f64 = (0..4).map(|c| a[c] * b[c]).sum();
    let length_a: f64 = a.iter().map(|v| v * v).sum();
    let length_b: f64 = b.iter().map(|v| v * v).sum();
    dot > 0.0 && dot * dot >= length_a * length_b * 0.95
}

fn neighbors(at: usize, width: usize, height: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (at % width, at / width);
    [
        (x > 0).then(|| at - 1),
        (x + 1 < width).then_some(at + 1),
        (y > 0).then(|| at - width),
        (y + 1 < height).then_some(at + width),
    ]
    .into_iter()
    .flatten()
}

const EDGE_REACH: u8 = 16;
/// Only travel up the boundary's color/alpha ramp, never down into the next
/// region. A bounded reach covers antialiasing and ordinary soft pencil edges.
pub(crate) fn soft_edges(
    dimensions: [usize; 2],
    core: &[u64],
    seed: [u8; 4],
    tolerance: u8,
    read: impl Fn(usize) -> [u8; 4],
    selected: impl Fn(usize) -> bool,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<u64>, String> {
    let [width, height] = dimensions;
    check_cancel(cancel)?;
    let count = width * height;
    // A component covering the canvas has no external boundary to soften.
    // Check packed words instead of reading every source pixel and neighbor.
    if core[..count / 64].iter().all(|word| *word == u64::MAX)
        && (count.is_multiple_of(64) || core[count / 64] == (1u64 << (count % 64)) - 1)
    {
        check_cancel(cancel)?;
        return Ok(vec![0; core.len()]);
    }
    let mut visited = core.to_vec();
    let mut edges = vec![0u64; core.len()];
    let mut queue = std::collections::VecDeque::new();
    for at in 0..width * height {
        if at & 1023 == 0 {
            check_cancel(cancel)?;
        }
        if !contains(core, at) {
            continue;
        }
        for next in neighbors(at, width, height) {
            if contains(&visited, next) || !selected(next) {
                continue;
            }
            let from = distance(read(at), seed);
            let to = distance(read(next), seed);
            if to > tolerance
                && to >= from
                && to < 255
                && (from <= tolerance || same_ramp(seed, read(at), read(next)))
            {
                visit(&mut visited, next);
                visit(&mut edges, next);
                queue.push_back((next, 1u8));
            }
        }
    }
    let mut processed = 0usize;
    while let Some((at, depth)) = queue.pop_front() {
        if processed & 1023 == 0 {
            check_cancel(cancel)?;
        }
        processed += 1;
        if depth == EDGE_REACH {
            continue;
        }
        let from = distance(read(at), seed);
        for next in neighbors(at, width, height) {
            if contains(&visited, next) || !selected(next) {
                continue;
            }
            let to = distance(read(next), seed);
            if to > tolerance && to >= from && to < 255 && same_ramp(seed, read(at), read(next)) {
                visit(&mut visited, next);
                visit(&mut edges, next);
                queue.push_back((next, depth + 1));
            }
        }
    }
    check_cancel(cancel)?;
    Ok(edges)
}

pub(crate) fn edge_coverage(
    dimensions: [usize; 2],
    at: usize,
    seed: [u8; 4],
    read: impl Fn(usize) -> [u8; 4],
) -> f32 {
    if seed[3] == 0 {
        return 1.0 - f32::from(read(at)[3]) / 255.0;
    }
    let [width, height] = dimensions;
    let initial = distance(read(at), seed);
    let mut strongest = initial;
    let mut position = at;
    for _ in 0..EDGE_REACH {
        let next = neighbors(position, width, height)
            .filter(|next| same_ramp(seed, read(at), read(*next)))
            .map(|next| (distance(read(next), seed), next))
            .max();
        match next {
            Some((value, next)) if value > strongest => {
                strongest = value;
                position = next;
            }
            _ => break,
        }
    }
    if strongest == 0 {
        0.0
    } else {
        1.0 - f32::from(initial) / f32::from(strongest)
    }
}

/// Replace only the seed-background contribution in premultiplied space. The
/// remaining contribution is the original linework, including its soft alpha.
pub(crate) fn replace_edge(
    pixel: &mut [u8],
    seed: [u8; 4],
    color: [u8; 3],
    opacity: f32,
    coverage: f32,
) {
    let alpha = f32::from(pixel[3]) / 255.0;
    let seed_alpha = f32::from(seed[3]) / 255.0;
    let amount = opacity * coverage;
    let out_alpha = (alpha + (1.0 - seed_alpha) * amount).clamp(0.0, 1.0);
    if out_alpha <= 0.0 {
        return;
    }
    for c in 0..3 {
        let premultiplied = f32::from(pixel[c]) * alpha
            + (f32::from(color[c]) - f32::from(seed[c]) * seed_alpha) * amount;
        pixel[c] = (premultiplied / out_alpha).clamp(0.0, 255.0).round() as u8;
    }
    pixel[3] = (out_alpha * 255.0).round() as u8;
}
impl Dudel {
    pub(crate) fn fill_edge_options_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.input_stroke_target().is_raster(), |ui| {
            ui.checkbox(&mut self.fill_options.fill_edges, "Fill to edges")
                .on_hover_text("Fill beneath soft outlines without a pale seam. Follows up to 16 pixels of soft edging, stopping when the boundary fades toward another region. Applies to artwork, not mask coverage.");
        });
        ui.add(egui::Slider::new(&mut self.fill_options.close_gap, 0..=8).text("Close gaps (px)"))
            .on_hover_text("Closes small breaks in outlines. Larger values can skip narrow areas.");
        ui.add(egui::Slider::new(&mut self.fill_options.expand, -16..=16).text("Expand (px)"))
            .on_hover_text("Grow the fill under outlines, or use negative values to shrink. Stays inside the selection.");
    }
    pub(crate) fn fill_canvas(&mut self, x: usize, y: usize) {
        if self.canvas_edit_ticket.is_some() {
            self.status = "Wait for the canvas bounds change to finish".into();
            return;
        }
        if let crate::paint_target::Target::Mask(id) = self.input_stroke_target() {
            self.edit_canvas_mask(crate::layer_mask_edits::Command::Fill {
                id,
                request: Request {
                    point: [x, y],
                    color: self.paint_color(),
                    opacity: self.brush.opacity,
                    tolerance: self.fill_tolerance,
                    source: self.fill_source,
                    options: self.fill_options,
                },
            });
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        self.close_input_gestures();
        let edit = crate::document_edits::Edit::FillTarget {
            target,
            source: self.fill_source,
            point: [x, y],
            color: self.color,
            opacity: self.brush.opacity,
            tolerance: self.fill_tolerance,
            options: self.fill_options,
        };
        match self.render_queue.edit(edit) {
            Ok(ticket) => {
                self.edit_feedback.insert(
                    ticket,
                    crate::render_controller::EditFeedback {
                        changed: "Filled connected area",
                        unchanged: "Fill made no changes",
                    },
                );
                self.status = "Fill queued".into();
                self.note_recent_color();
            }
            Err(error) => self.status = error,
        }
    }
    pub(crate) fn fill_source_ui(&mut self, ui: &mut egui::Ui) {
        if self.render_queue.has_background_edit() && ui.button("Cancel background edit").clicked()
        {
            self.render_queue.cancel_background_edit();
            self.status = "Canceling background edit".into();
        }
        let active_label = if self.input_stroke_target().is_raster() {
            "Active layer"
        } else {
            "Active mask"
        };
        let label = match self.fill_source {
            Source::Active => active_label,
            Source::Visible => "Visible layers",
            Source::Layer(id) => self
                .doc
                .layers
                .iter()
                .find(|layer| layer.id == id)
                .map_or("Missing reference layer", |layer| layer.name.as_str()),
        };
        egui::ComboBox::from_id_salt("fill reference")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.fill_source, Source::Active, active_label);
                ui.selectable_value(&mut self.fill_source, Source::Visible, "Visible layers");
                ui.separator();
                for layer in self.doc.layers.iter().rev().filter(|layer| !layer.folder) {
                    ui.selectable_value(
                        &mut self.fill_source,
                        Source::Layer(layer.id),
                        &layer.name,
                    );
                }
            });
    }
}

/// Headless fill-only timing, with pixel verification outside timing. Reported
/// visitation payload excludes references, destination COW tiles, history and RSS.
pub fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let options = Options {
        fill_edges: std::env::var_os("DUDEL_FILL_BENCH_EDGES").is_some(),
        ..Default::default()
    };
    let tolerance = if options.fill_edges { 12 } else { 0 };
    #[derive(serde::Serialize)]
    struct Case {
        width: usize,
        height: usize,
        workload: &'static str,
        initial_history_bytes: usize,
        source: &'static str,
        samples_ms: Vec<f64>,
        worker_max_call_ms: Vec<f64>,
        worker_wall_ms: Vec<f64>,
        worker_settlement_payload_peak_bytes: Vec<usize>,
        worker_history_retained_bytes: Vec<usize>,
        worker_publication_ms: Vec<f64>,
        worker_settlement_max_call_ms: Vec<f64>,
        worker_settlement_calls: Vec<usize>,
        visitation_payload_bytes: usize,
        verified_pixels: usize,
    }
    let sizes = match std::env::var("DUDEL_FILL_BENCH_SIZE") {
        Ok(value) => {
            let size = value
                .parse::<usize>()
                .map_err(|_| "Invalid fill benchmark size")?;
            if !(512..=4096).contains(&size) {
                return Err("Fill benchmark size must be between 512 and 4096".into());
            }
            vec![size]
        }
        Err(_) => vec![512, 2048],
    };
    let mut cases = Vec::new();
    for size in sizes {
        for workload in ["sparse", "dense-flat", "dense-tiled", "dense-tiled-history"] {
            let mut fixture = crate::document::Document::new(size, size);
            if workload != "sparse" {
                let dense = [9, 11, 13, 255].repeat(size * size);
                if workload == "dense-flat" {
                    fixture.layers[0].pixels = dense.into();
                } else {
                    fixture.layers[0].pixels.writer().write_region(
                        size,
                        [0, 0, size, size],
                        &dense,
                    );
                }
            }
            if workload == "sparse" {
                fixture.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(size - 1)
                    .copy_from_slice(&[9, 11, 13, 0]);
            }
            fixture.add_layer();
            for y in 0..size {
                fixture.layers[1]
                    .pixels
                    .writer()
                    .rgba_mut(y * size + size / 2)
                    .copy_from_slice(&[0, 0, 0, 255]);
            }
            fixture.active = 0;
            let reference = fixture.layers[1].id;
            for (name, source) in [
                ("active", Source::Active),
                ("specific", Source::Layer(reference)),
                ("visible", Source::Visible),
            ] {
                let mut samples_ms = Vec::new();
                let mut worker_max_call_ms = Vec::new();
                let mut worker_wall_ms = Vec::new();
                let mut worker_settlement_payload_peak_bytes = Vec::new();
                let mut worker_history_retained_bytes = Vec::new();
                let mut worker_publication_ms = Vec::new();
                let mut worker_settlement_max_call_ms = Vec::new();
                let mut worker_settlement_calls = Vec::new();
                let mut initial_history_bytes = 0;
                for _ in 0..3 {
                    let mut candidate = fixture.clone();
                    let started = std::time::Instant::now();
                    let changed = candidate.fill_request(
                        Request {
                            point: [0, size / 2],
                            color: [17, 83, 201],
                            opacity: 1.0,
                            tolerance,
                            source,
                            options,
                        },
                        None,
                    )?;
                    samples_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                    if !changed {
                        return Err("Benchmark fill made no changes".into());
                    }
                    for y in 0..size {
                        for x in 0..size {
                            let expected = if source == Source::Active || x < size / 2 {
                                [17, 83, 201, 255]
                            } else {
                                fixture.layers[0].pixels.rgba(y * size + x)
                            };
                            if candidate.layers[0].pixels.rgba(y * size + x) != expected {
                                return Err("Benchmark fill pixels differ".into());
                            }
                        }
                    }
                    if candidate.layers[1].pixels != fixture.layers[1].pixels {
                        return Err("Benchmark modified reference".into());
                    }
                    let mut queued = fixture.clone();
                    let mut history = crate::document::History::new();
                    if workload == "dense-tiled-history" {
                        for index in 0..24 {
                            history.checkpoint(&queued);
                            queued.layers[0].name = format!("History {index}");
                            queued.layers[0]
                                .pixels
                                .writer()
                                .rgba_mut(index * size)
                                .copy_from_slice(&[index as u8, 29, 31, 255]);
                        }
                        queued.layers[0].pixels = fixture.layers[0].pixels.clone();
                        queued.layers[0].name = fixture.layers[0].name.clone();
                        history.refresh_memory(&queued);
                    }
                    initial_history_bytes = history.retained_bytes();
                    let mut queue = crate::render_queue::Queue::default();
                    queue.edit(crate::document_edits::Edit::FillTarget {
                        target: queued.layers[0].id,
                        source,
                        point: [0, size / 2],
                        color: [17, 83, 201],
                        opacity: 1.0,
                        tolerance,
                        options,
                    })?;
                    let started = std::time::Instant::now();
                    let mut maximum = 0.0f64;
                    let mut peak_payload = 0usize;
                    let mut publication = 0.0f64;
                    let mut settlement_maximum = 0.0f64;
                    let mut settlement_calls = 0usize;
                    let mut published = false;
                    while queue.busy() {
                        if queue.needs_work() {
                            let call = std::time::Instant::now();
                            let progress = queue.advance_until(
                                &mut queued,
                                &mut history,
                                4096,
                                call + std::time::Duration::from_millis(2),
                            )?;
                            let elapsed = call.elapsed().as_secs_f64() * 1000.0;
                            maximum = maximum.max(elapsed);
                            if !progress.damage.is_empty() {
                                published = true;
                                publication = elapsed;
                            } else if published {
                                settlement_calls += 1;
                                settlement_maximum = settlement_maximum.max(elapsed);
                            }
                            peak_payload = peak_payload.max(progress.settlement_payload_peak_bytes);
                            if progress.edits.iter().any(|(_, outcome)| outcome.is_err()) {
                                return Err("Benchmark queued fill failed".into());
                            }
                        } else {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                        if started.elapsed() > std::time::Duration::from_secs(30) {
                            return Err("Benchmark fill worker timed out".into());
                        }
                    }
                    worker_wall_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                    worker_max_call_ms.push(maximum);
                    worker_settlement_payload_peak_bytes.push(peak_payload);
                    worker_history_retained_bytes.push(history.retained_bytes());
                    worker_publication_ms.push(publication);
                    worker_settlement_max_call_ms.push(settlement_maximum);
                    worker_settlement_calls.push(settlement_calls);
                    if queued.layers[0].pixels != candidate.layers[0].pixels
                        || queued.layers[1].pixels != fixture.layers[1].pixels
                    {
                        return Err("Benchmark worker pixels differ".into());
                    }
                    if !history.undo(&mut queued)
                        || queued.layers[0].pixels != fixture.layers[0].pixels
                    {
                        return Err("Benchmark worker undo differs".into());
                    }
                }
                cases.push(Case {
                    width: size,
                    height: size,
                    workload,
                    initial_history_bytes,
                    source: name,
                    samples_ms,
                    worker_max_call_ms,
                    worker_wall_ms,
                    worker_settlement_payload_peak_bytes,
                    worker_history_retained_bytes,
                    worker_publication_ms,
                    worker_settlement_max_call_ms,
                    worker_settlement_calls,
                    visitation_payload_bytes: (size * size).div_ceil(64)
                        * std::mem::size_of::<u64>(),
                    verified_pixels: size * size,
                });
            }
        }
    }
    let advanced_cases = benchmark_advanced()?;
    let report = serde_json::json!({"version": 6, "options": options, "tolerance": tolerance, "advanced_scope":"One direct fill repetition per size/source/expansion. Includes reference/mask construction and painting, excludes history setup/commit, undo/redo, verification and UI/GPU. Broken one-pixel wall, close-gap radius 1, signed square expansion. Exact destination/reference pixels and undo/redo checked. No worker latency or process peak claims.", "advanced_cases":advanced_cases, "profile": "headless-fill-and-worker", "scope": "Default 512/2048-square (or DUDEL_FILL_BENCH_SIZE override) sparse, dense flat compatibility, dense tiled and dense tiled with 24 metadata/raster Full undo snapshots; history preparation outside timing. Direct fill includes reference preparation, excluding UI/GPU/history/result verification and total process memory. Visitation payload counts one bitset, excluding additional edge/morphology masks and the traversal frontier. Worker maximum call includes publication/sliced history; worker wall includes 1 ms polling sleeps. Publication call contains damage and may also settle history within its remaining budget; settlement calls are later calls. Settlement peak is prepared patch payload capacity, excluding retained snapshots and total memory; history bytes estimate exclusive retained allocations, not RSS. Three sequential repetitions per source, size and workload; exact direct/worker/reference pixels and worker undo checked outside timing.", "cases": cases});
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn benchmark_advanced() -> Result<Vec<serde_json::Value>, String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        let mut fixture = crate::document::Document::new(size, size);
        fixture.layers[0]
            .pixels
            .writer()
            .rgba_mut(size - 1)
            .copy_from_slice(&[9, 11, 13, 0]);
        fixture.add_layer();
        for y in 0..size {
            if y != size / 2 {
                fixture.layers[1]
                    .pixels
                    .writer()
                    .rgba_mut(y * size + size / 2)
                    .copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        fixture.active = 0;
        for source in [Source::Layer(fixture.layers[1].id), Source::Visible] {
            for expand in [-1i8, 0, 1] {
                let mut candidate = fixture.clone();
                let mut history = crate::document::History::new();
                history.checkpoint(&candidate);
                let started = std::time::Instant::now();
                candidate.fill_request(
                    Request {
                        point: [0, size / 2],
                        color: [17, 83, 201],
                        opacity: 1.0,
                        tolerance: 0,
                        source,
                        options: Options {
                            close_gap: 1,
                            expand,
                            fill_edges: false,
                        },
                    },
                    None,
                )?;
                let fill_ms = started.elapsed().as_secs_f64() * 1000.0;
                let limit = (size / 2) as isize + isize::from(expand);
                for y in 0..size {
                    for x in 0..size {
                        let expected = if (x as isize) < limit {
                            [17, 83, 201, 255]
                        } else {
                            fixture.layers[0].pixels.rgba(y * size + x)
                        };
                        if candidate.layers[0].pixels.rgba(y * size + x) != expected {
                            return Err("Advanced fill benchmark pixels differ".into());
                        }
                    }
                }
                if candidate.layers[1].pixels != fixture.layers[1].pixels {
                    return Err("Advanced fill benchmark modified reference".into());
                }
                let after = candidate.clone();
                history.settle(&candidate);
                if !history.undo(&mut candidate)
                    || candidate.layers[0].pixels != fixture.layers[0].pixels
                    || !history.redo(&mut candidate)
                    || candidate.layers[0].pixels != after.layers[0].pixels
                {
                    return Err("Advanced fill benchmark undo/redo differs".into());
                }
                cases.push(
                    serde_json::json!({"size":size,"close_gap":1,"expand":expand,
                    "source":if source == Source::Visible { "visible" } else { "specific" },
                    "fill_ms":fill_ms,"pixels_reference_undo_redo_verified":true}),
                );
            }
        }
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, document_edits::Edit, selection::Selection};

    // Deliberately scalar, independent four-neighbor oracle for the span walk.
    fn connected_oracle(
        width: usize,
        height: usize,
        seed: usize,
        eligible: impl Fn(usize) -> bool,
    ) -> Vec<bool> {
        let mut seen = vec![false; width * height];
        let mut result = seen.clone();
        let mut queue = std::collections::VecDeque::from([seed]);
        while let Some(at) = queue.pop_front() {
            if seen[at] {
                continue;
            }
            seen[at] = true;
            if !eligible(at) {
                continue;
            }
            result[at] = true;
            let (x, y) = (at % width, at / width);
            if x > 0 {
                queue.push_back(at - 1);
            }
            if x + 1 < width {
                queue.push_back(at + 1);
            }
            if y > 0 {
                queue.push_back(at - width);
            }
            if y + 1 < height {
                queue.push_back(at + width);
            }
        }
        result
    }

    #[test]
    fn scanlines_match_scalar_connectivity_and_visit_each_pixel_once() {
        for (width, height) in [(1, 1), (1, 131), (131, 1), (67, 39), (517, 19)] {
            for pattern in 0..6 {
                let eligible = |at: usize| {
                    let (x, y) = (at % width, at / width);
                    match pattern {
                        0 => true,
                        1 => false,
                        2 => (x + y) % 2 == 0, // Diagonals must stay disconnected.
                        3 => y % 2 == 0 || x == if y % 4 == 1 { width - 1 } else { 0 },
                        4 => x % 3 != 1 || y == height / 2, // Branched comb.
                        _ => (at.wrapping_mul(1664525).wrapping_add(y * 1013904223) >> 5) % 11 < 7,
                    }
                };
                for seed in [0, width * height / 2, width * height - 1] {
                    let expected = connected_oracle(width, height, seed, eligible);
                    let mut visits = vec![0; width * height];
                    let actual = connected_spans(
                        [width, height],
                        seed,
                        eligible,
                        |y, left, right| {
                            assert!(left < right && right <= width);
                            for count in &mut visits[y * width + left..y * width + right] {
                                *count += 1;
                            }
                        },
                        None,
                    )
                    .unwrap();
                    for at in 0..width * height {
                        assert_eq!(
                            contains(&actual, at),
                            expected[at],
                            "{width}x{height}, pattern {pattern}, seed {seed}, pixel {at}"
                        );
                        assert_eq!(visits[at], usize::from(expected[at]));
                    }
                }
            }
        }
    }

    #[test]
    fn scanlines_cancel_during_long_rows_and_after_a_paint_callback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        for (width, height) in [(4096, 1), (1, 4096), (67, 39)] {
            let cancel = AtomicBool::new(false);
            let calls = std::cell::Cell::new(0);
            let result = connected_spans(
                [width, height],
                width * height / 2,
                |_| {
                    calls.set(calls.get() + 1);
                    if calls.get() == 100 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    true
                },
                |_, _, _| {},
                Some(&cancel),
            );
            assert_eq!(result.unwrap_err(), "Fill canceled");
            assert!(calls.get() <= 1124);
        }
        let cancel = AtomicBool::new(false);
        assert_eq!(
            connected_spans(
                [1, 1],
                0,
                |_| true,
                |_, _, _| {
                    cancel.store(true, Ordering::Relaxed);
                },
                Some(&cancel)
            )
            .unwrap_err(),
            "Fill canceled"
        );
    }

    #[test]
    fn full_component_has_no_soft_boundary_including_partial_bitset_words() {
        for (width, height) in [(1, 1), (64, 1), (67, 39), (256, 257)] {
            let core = connected_spans([width, height], 0, |_| true, |_, _, _| {}, None).unwrap();
            let edges = soft_edges(
                [width, height],
                &core,
                [0; 4],
                12,
                |_| panic!("a full component has no outside neighbors"),
                |_| true,
                None,
            )
            .unwrap();
            assert!(edges.iter().all(|word| *word == 0));
            let cancel = std::sync::atomic::AtomicBool::new(true);
            assert!(
                soft_edges(
                    [width, height],
                    &core,
                    [0; 4],
                    12,
                    |_| [0; 4],
                    |_| true,
                    Some(&cancel)
                )
                .is_err()
            );
        }
    }

    #[test]
    fn scanline_fill_matches_scalar_blending_across_tiles_and_soft_selections() {
        let (width, height) = (517, 19);
        let bytes: Vec<u8> = (0..width * height)
            .flat_map(|at| {
                [
                    ((at % width) % 31) as u8,
                    29,
                    63,
                    [0, 64, 128, 255][at / width % 4],
                ]
            })
            .collect();
        for flat in [false, true] {
            for active_source in [false, true] {
                for alpha_locked in [false, true] {
                    for opacity in [0.37, 1.0] {
                        for tolerance in [0, 25, 255] {
                            let mut doc = Document::new(width, height);
                            if flat {
                                doc.layers[0].pixels = bytes.clone().into();
                            } else {
                                doc.layers[0].pixels.writer().write_region(
                                    width,
                                    [0, 0, width, height],
                                    &bytes,
                                );
                            }
                            doc.layers[0].alpha_locked = alpha_locked;
                            doc.add_layer();
                            doc.active = 0;
                            doc.selection = Some(Selection {
                                left: 3,
                                top: 1,
                                right: width - 2,
                                bottom: height - 1,
                            });
                            let coverage: Vec<u8> = (0..width * height)
                                .map(|at| {
                                    if at % width == 260 && at / width != 9 {
                                        0
                                    } else {
                                        [64, 128, 255][at % 3]
                                    }
                                })
                                .collect();
                            doc.selection_mask = Some(
                                crate::selection_mask::Mask::from_coverage(
                                    width,
                                    height,
                                    coverage.clone(),
                                )
                                .unwrap(),
                            );
                            let seed = 9 * width + 255;
                            let source = if active_source {
                                Source::Active
                            } else {
                                Source::Layer(doc.layers[1].id)
                            };
                            let reference = |at: usize| -> [u8; 4] {
                                if active_source {
                                    bytes[at * 4..at * 4 + 4].try_into().unwrap()
                                } else {
                                    [0; 4]
                                }
                            };
                            let selected = connected_oracle(width, height, seed, |at| {
                                doc.selection.unwrap().contains(at % width, at / width)
                                    && coverage[at] != 0
                                    && equivalent(&reference(at), &reference(seed), tolerance)
                            });
                            let mut expected = bytes.clone();
                            let color = [17, 83, 201];
                            for (at, pixel) in expected.chunks_exact_mut(4).enumerate() {
                                if !selected[at] {
                                    continue;
                                }
                                let amount = opacity * f32::from(coverage[at]) / 255.0;
                                let old_alpha = pixel[3] as f32 / 255.0;
                                if alpha_locked {
                                    if pixel[3] > 0 {
                                        for c in 0..3 {
                                            pixel[c] = (color[c] as f32 * amount
                                                + pixel[c] as f32 * (1.0 - amount))
                                                .round()
                                                as u8;
                                        }
                                    }
                                } else {
                                    let alpha = amount + old_alpha * (1.0 - amount);
                                    for c in 0..3 {
                                        pixel[c] = ((color[c] as f32 * amount
                                            + pixel[c] as f32 * old_alpha * (1.0 - amount))
                                            / alpha)
                                            .round()
                                            as u8;
                                    }
                                    pixel[3] = (alpha * 255.0).round() as u8;
                                }
                            }
                            let before = doc.clone();
                            assert_eq!(
                                doc.fill_using(
                                    seed % width,
                                    seed / width,
                                    color,
                                    opacity,
                                    tolerance,
                                    source
                                )
                                .unwrap(),
                                expected != bytes
                            );
                            for at in 0..width * height {
                                assert_eq!(
                                    doc.layers[0].pixels.rgba(at).as_slice(),
                                    &expected[at * 4..at * 4 + 4]
                                );
                                assert_eq!(
                                    before.layers[0].pixels.rgba(at).as_slice(),
                                    &bytes[at * 4..at * 4 + 4]
                                );
                            }
                            assert_eq!(doc.layers[1].pixels, before.layers[1].pixels);
                        }
                    }
                }
            }
        }
    }

    fn feathered_border(opaque_paper: bool, peak: u8) -> Document {
        let mut doc = Document::new(13, 7);
        for y in 0..7 {
            for x in 0..13 {
                let alpha = match x {
                    4 | 8 => 64,
                    5 | 7 => 128,
                    6 => peak,
                    _ => 0,
                };
                let pixel = if opaque_paper {
                    [255 - alpha, 255 - alpha, 255 - alpha, 255]
                } else {
                    [0, 0, 0, alpha]
                };
                doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 13 + x)
                    .copy_from_slice(&pixel);
            }
        }
        doc
    }
    fn edge_request(source: Source) -> Request {
        Request {
            point: [1, 3],
            color: [40, 120, 200],
            opacity: 1.0,
            tolerance: 12,
            source,
            options: Options {
                fill_edges: true,
                ..Default::default()
            },
        }
    }
    #[test]
    fn edge_fill_removes_halos_on_transparent_and_flattened_linework_without_crossing() {
        for opaque in [false, true] {
            let original = feathered_border(opaque, 255);
            let mut filled = original.clone();
            filled
                .fill_request(edge_request(Source::Active), None)
                .unwrap();
            for y in 0..7 {
                for x in 0..13 {
                    let at = y * 13 + x;
                    let pixel = filled.layers[0].pixels.rgba(at);
                    match x {
                        0..=3 => assert_eq!(pixel, [40, 120, 200, 255]),
                        4 | 5 => {
                            let alpha = if x == 4 { 64 } else { 128 };
                            let expected =
                                [40u16, 120, 200].map(|c| ((c * (255 - alpha) + 127) / 255) as u8);
                            assert_eq!(pixel, [expected[0], expected[1], expected[2], 255]);
                        }
                        _ => assert_eq!(
                            pixel,
                            original.layers[0].pixels.rgba(at),
                            "outside/outline x={x}"
                        ),
                    }
                }
            }
        }
        // Even a translucent outline with no fully opaque core is a barrier.
        let original = feathered_border(false, 192);
        let mut filled = original.clone();
        filled
            .fill_request(edge_request(Source::Active), None)
            .unwrap();
        assert_eq!(filled.layers[0].pixels.rgba(3 * 13 + 6)[3], 255);
        for x in 7..13 {
            assert_eq!(
                filled.layers[0].pixels.rgba(3 * 13 + x),
                original.layers[0].pixels.rgba(3 * 13 + x)
            );
        }
        // A colored opaque-paper edge uses its actual ink color, not an
        // assumption that every outline is black.
        let mut colored = feathered_border(true, 255);
        let ink = [80u16, 132, 187];
        for y in 0..7 {
            for x in 4..=8 {
                let alpha = match x {
                    4 | 8 => 64,
                    5 | 7 => 128,
                    _ => 255,
                };
                let rgb = ink.map(|c| ((c * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
                colored.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 13 + x)
                    .copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        colored
            .fill_request(edge_request(Source::Active), None)
            .unwrap();
        for x in [4, 5] {
            let alpha = if x == 4 { 64 } else { 128 };
            let pixel = colored.layers[0].pixels.rgba(3 * 13 + x);
            for (c, fill) in [40u16, 120, 200].into_iter().enumerate() {
                let expected = ((ink[c] * alpha + fill * (255 - alpha) + 127) / 255) as u8;
                assert!(pixel[c].abs_diff(expected) <= 1);
            }
        }
        let mut adjacent = Document::new(9, 3);
        for y in 0..3 {
            for x in 0..9 {
                let pixel = if x < 4 {
                    [255; 4]
                } else if x == 4 {
                    [180, 180, 180, 255]
                } else {
                    [80, 180, 240, 255]
                };
                adjacent.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 9 + x)
                    .copy_from_slice(&pixel);
            }
        }
        let before = adjacent.clone();
        adjacent
            .fill_request(
                Request {
                    point: [1, 1],
                    ..edge_request(Source::Active)
                },
                None,
            )
            .unwrap();
        for x in 4..9 {
            assert_eq!(
                adjacent.layers[0].pixels.rgba(9 + x),
                before.layers[0].pixels.rgba(9 + x)
            );
        }
        let mut faint = feathered_border(false, 255);
        for y in 0..7 {
            for (x, alpha) in [(3, 1), (4, 64), (5, 128), (6, 255)] {
                faint.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 13 + x)
                    .copy_from_slice(&[80, 132, 187, alpha]);
            }
        }
        faint
            .fill_request(
                Request {
                    tolerance: 0,
                    ..edge_request(Source::Active)
                },
                None,
            )
            .unwrap();
        for x in 3..6 {
            assert_eq!(faint.layers[0].pixels.rgba(3 * 13 + x)[3], 255);
        }
    }
    #[test]
    fn edge_fill_reference_layers_selection_opacity_and_queued_undo_remain_exact() {
        for visible in [false, true] {
            let mut doc = feathered_border(false, 255);
            let line = doc.layers[0].pixels.clone();
            doc.add_layer();
            doc.layers[1].pixels = line.clone();
            doc.layers[0].pixels = crate::document::Document::new(13, 7).layers[0]
                .pixels
                .clone();
            doc.active = 0;
            let source = if visible {
                Source::Visible
            } else {
                Source::Layer(doc.layers[1].id)
            };
            let original = doc.clone();
            let request = edge_request(source);
            let mut expected = doc.clone();
            expected.fill_request(request, None).unwrap();
            assert_eq!(
                expected.layers[0].pixels.rgba(3 * 13 + 5),
                [40, 120, 200, 255]
            );
            assert_eq!(expected.layers[0].pixels.rgba(3 * 13 + 7), [0; 4]);
            assert!(expected.layers[1].pixels == line);
            let mut history = crate::document::History::new();
            let mut queue = crate::render_queue::Queue::default();
            queue
                .edit(Edit::FillTarget {
                    target: doc.layers[0].id,
                    source,
                    point: request.point,
                    color: request.color,
                    opacity: request.opacity,
                    tolerance: request.tolerance,
                    options: request.options,
                })
                .unwrap();
            while queue.busy() {
                queue.advance(&mut doc, &mut history, 4096).unwrap();
                std::thread::yield_now();
            }
            assert!(doc.layers[0].pixels == expected.layers[0].pixels);
            assert!(doc.layers[1].pixels == line);
            assert!(history.undo(&mut doc));
            assert!(doc.layers[0].pixels == original.layers[0].pixels);
            assert!(history.redo(&mut doc));
            assert!(doc.layers[0].pixels == expected.layers[0].pixels);
        }
        let mut doc = feathered_border(false, 255);
        let before = doc.clone();
        doc.selection = Some(Selection {
            left: 0,
            top: 0,
            right: 5,
            bottom: 7,
        });
        doc.fill_request(
            Request {
                opacity: 0.5,
                ..edge_request(Source::Active)
            },
            None,
        )
        .unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(3 * 13)[3], 128);
        for x in 5..13 {
            assert_eq!(
                doc.layers[0].pixels.rgba(3 * 13 + x),
                before.layers[0].pixels.rgba(3 * 13 + x)
            );
        }
        let mut locked = feathered_border(false, 255);
        locked.layers[0].alpha_locked = true;
        let before = locked.clone();
        assert!(
            !locked
                .fill_request(edge_request(Source::Active), None)
                .unwrap()
        );
        assert!(locked.layers[0].pixels == before.layers[0].pixels);
    }
    #[test]
    fn advanced_fill_selection_locks_and_invalid_options_preserve_artwork() {
        let mut doc = broken_outline();
        doc.selection = Some(Selection {
            left: 10,
            top: 8,
            right: 20,
            bottom: 16,
        });
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(12 * 32 + 15)
            .copy_from_slice(&[9, 11, 13, 128]);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(12 * 32 + 16)
            .copy_from_slice(&[9, 11, 13, 0]);
        doc.layers[0].alpha_locked = true;
        let request = advanced_request(&doc, 16);
        assert!(doc.fill_request(request, None).unwrap());
        assert_eq!(doc.layers[0].pixels.rgba(12 * 32 + 15), [17, 83, 201, 128]);
        assert_eq!(doc.layers[0].pixels.rgba(12 * 32 + 16), [9, 11, 13, 0]);
        doc.layers[0].alpha_locked = false;
        let request = advanced_request(&doc, 16);
        assert!(doc.fill_request(request, None).unwrap());
        for y in 0..24 {
            for x in 0..32 {
                assert_eq!(
                    doc.layers[0].pixels.rgba(y * 32 + x),
                    if (10..20).contains(&x) && (8..16).contains(&y) {
                        [17, 83, 201, 255]
                    } else {
                        [0; 4]
                    }
                );
            }
        }
        for options in [
            Options {
                close_gap: 9,
                expand: 0,
                fill_edges: false,
            },
            Options {
                close_gap: 1,
                expand: 17,
                fill_edges: false,
            },
        ] {
            let before = doc.clone();
            let request = Request {
                options,
                ..advanced_request(&doc, 0)
            };
            assert!(doc.fill_request(request, None).is_err());
            assert!(crate::document::same_snapshot(&doc, &before));
        }
        doc.layers[0].locked = true;
        let before = doc.clone();
        let request = advanced_request(&doc, 1);
        assert!(doc.fill_request(request, None).is_err());
        assert!(crate::document::same_snapshot(&doc, &before));
    }
    #[test]
    fn mask_construction_cancellation_and_legacy_command_defaults_are_validated() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let result = region_mask(
            [67, 39],
            0,
            Options {
                close_gap: 1,
                expand: 1,
                fill_edges: false,
            },
            |index| {
                if index == 300 {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                true
            },
            Some(&cancel),
        );
        assert_eq!(result.unwrap_err(), "Fill canceled");
        let doc = broken_outline();
        let edit = Edit::FillTarget {
            target: doc.layers[0].id,
            source: Source::Layer(doc.layers[1].id),
            point: [15, 12],
            color: [17, 83, 201],
            opacity: 1.0,
            tolerance: 0,
            options: Options::default(),
        };
        let mut value = serde_json::to_value(edit).unwrap();
        value["FillTarget"]
            .as_object_mut()
            .unwrap()
            .remove("options");
        let restored: Edit = serde_json::from_value(value).unwrap();
        assert!(
            matches!(restored, Edit::FillTarget { options, .. } if options == Options::default())
        );
        let mut queue = crate::render_queue::Queue::default();
        assert!(
            queue
                .edit(Edit::FillTarget {
                    target: doc.layers[0].id,
                    source: Source::Active,
                    point: [0, 0],
                    color: [0, 0, 0],
                    opacity: 1.0,
                    tolerance: 0,
                    options: Options {
                        close_gap: 255,
                        expand: 0,
                        fill_edges: false,
                    }
                })
                .is_err()
        );
        assert!(!queue.busy());
    }
    #[test]
    fn square_morphology_matches_neighborhood_oracle_at_edges_and_small_dimensions() {
        for (width, height) in [(1usize, 1usize), (2, 7), (17, 19), (67, 3)] {
            let mut input = vec![0; (width * height).div_ceil(64)];
            for index in 0..width * height {
                if (index * 37 + index / width * 13) % 11 < 5 {
                    visit(&mut input, index);
                }
            }
            for radius in [0, 1, 2, 8] {
                for dilate in [false, true] {
                    let actual = morphology(&input, width, height, radius, dilate, None).unwrap();
                    for y in 0..height {
                        for x in 0..width {
                            let mut values = (y.saturating_sub(radius)
                                ..(y + radius + 1).min(height))
                                .flat_map(|py| {
                                    (x.saturating_sub(radius)..(x + radius + 1).min(width))
                                        .map(move |px| py * width + px)
                                });
                            let expected = if dilate {
                                values.any(|index| contains(&input, index))
                            } else {
                                values.all(|index| contains(&input, index))
                            };
                            assert_eq!(contains(&actual, y * width + x), expected);
                        }
                    }
                }
            }
        }
    }
    fn broken_outline() -> Document {
        let mut doc = Document::new(32, 24);
        doc.add_layer();
        for y in 4..20 {
            for x in 4..28 {
                if (x == 4 || x == 27 || y == 4 || y == 19) && (x, y) != (15, 4) {
                    doc.layers[1]
                        .pixels
                        .writer()
                        .rgba_mut(y * 32 + x)
                        .copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        doc.active = 0;
        doc
    }
    fn advanced_request(doc: &Document, expand: i8) -> Request {
        Request {
            point: [15, 12],
            color: [17, 83, 201],
            opacity: 1.0,
            tolerance: 0,
            source: Source::Layer(doc.layers[1].id),
            options: Options {
                close_gap: 1,
                expand,
                fill_edges: false,
            },
        }
    }
    #[test]
    fn closed_gap_and_signed_expansion_fill_expected_region_without_changing_reference() {
        for expand in [-1, 0, 1] {
            let mut doc = broken_outline();
            let reference = doc.layers[1].pixels.clone();
            let request = advanced_request(&doc, expand);
            assert!(doc.fill_request(request, None).unwrap());
            let margin = (5i32 - i32::from(expand)) as usize;
            for y in 0..24 {
                for x in 0..32 {
                    let filled = x >= margin && x < 32 - margin && y >= margin && y < 24 - margin;
                    assert_eq!(
                        doc.layers[0].pixels.rgba(y * 32 + x),
                        if filled { [17, 83, 201, 255] } else { [0; 4] },
                        "expand={expand}, ({x},{y})"
                    );
                }
            }
            assert_eq!(doc.layers[1].pixels, reference);
        }
        let mut doc = broken_outline();
        let request = Request {
            options: Options::default(),
            ..advanced_request(&doc, 0)
        };
        assert!(doc.fill_request(request, None).unwrap());
        assert_eq!(doc.layers[0].pixels.rgba(0), [17, 83, 201, 255]);
    }
    #[test]
    fn advanced_fill_worker_orders_snapshot_and_undo_and_cancellation_discards_candidate() {
        for cancel in [false, true] {
            let mut doc = broken_outline();
            let before = doc.clone();
            let mut history = crate::document::History::new();
            let mut queue = crate::render_queue::Queue::default();
            queue
                .edit(Edit::FillTarget {
                    target: doc.layers[0].id,
                    source: Source::Layer(doc.layers[1].id),
                    point: [15, 12],
                    color: [17, 83, 201],
                    opacity: 1.0,
                    tolerance: 0,
                    options: Options {
                        close_gap: 1,
                        expand: 1,
                        fill_edges: false,
                    },
                })
                .unwrap();
            let snapshot_ticket = queue.snapshot().unwrap();
            queue.advance(&mut doc, &mut history, 1).unwrap();
            assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
            if cancel {
                queue.cancel_background_edit();
            }
            queue.wait_edit();
            let mut snapshot = None;
            while queue.busy() {
                let progress = queue.advance(&mut doc, &mut history, 4096).unwrap();
                if let Some(value) = progress.snapshot {
                    snapshot = Some(value);
                }
            }
            let (ticket, frozen) = snapshot.unwrap();
            assert_eq!(ticket, snapshot_ticket);
            assert_eq!(frozen.layers[0].pixels, doc.layers[0].pixels);
            if cancel {
                assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
                assert!(!history.can_undo());
            } else {
                let after = doc.clone();
                assert_eq!(doc.layers[0].pixels.rgba(4 * 32 + 15), [17, 83, 201, 255]);
                assert_eq!(doc.layers[0].pixels.rgba(0), [0; 4]);
                assert!(history.undo(&mut doc));
                assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
                assert!(history.redo(&mut doc));
                assert_eq!(doc.layers[0].pixels, after.layers[0].pixels);
            }
            assert_eq!(doc.layers[1].pixels, before.layers[1].pixels);
        }
    }

    fn line_art() -> Document {
        let mut doc = Document::new(5, 3);
        doc.add_layer();
        for y in 0..3 {
            doc.layers[1]
                .pixels
                .writer()
                .rgba_mut(y * 5 + 2)
                .copy_from_slice(&[0, 0, 0, 255]);
        }
        doc.active = 0;
        doc
    }
    #[test]
    fn reference_boundaries_write_only_destination_and_visible_composite_obeys_visibility() {
        for source_visible in [false, true] {
            for visible in [false, true] {
                let mut doc = line_art();
                doc.layers[1].visible = visible;
                let line = doc.layers[1].pixels.clone();
                let source = if source_visible {
                    Source::Visible
                } else {
                    Source::Layer(doc.layers[1].id)
                };
                assert!(doc.fill_using(0, 1, [17, 83, 201], 1.0, 0, source).unwrap());
                for y in 0..3 {
                    for x in 0..5 {
                        let filled = x < 2 || (source_visible && !visible);
                        assert_eq!(
                            doc.layers[0].pixels.rgba(y * 5 + x),
                            if filled { [17, 83, 201, 255] } else { [0; 4] }
                        );
                    }
                }
                assert_eq!(doc.layers[1].pixels, line);
            }
        }
        let mut doc = line_art();
        doc.layers[1].clipped = true; // Invisible against the empty layer below.
        assert!(
            doc.fill_using(0, 1, [17, 83, 201], 1.0, 0, Source::Visible)
                .unwrap()
        );
        assert!(
            doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
    }
    #[test]
    fn reference_fill_respects_selection_alpha_lock_and_hidden_rgb() {
        let mut doc = line_art();
        let reference = Source::Layer(doc.layers[1].id);
        doc.selection = Some(Selection {
            left: 1,
            top: 0,
            right: 4,
            bottom: 3,
        });
        assert!(
            doc.fill_using(1, 1, [17, 83, 201], 1.0, 0, reference)
                .unwrap()
        );
        for y in 0..3 {
            for x in 0..5 {
                assert_eq!(
                    doc.layers[0].pixels.rgba(y * 5 + x),
                    if x == 1 { [17, 83, 201, 255] } else { [0; 4] }
                );
            }
        }
        let mut doc = line_art();
        let reference = Source::Layer(doc.layers[1].id);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[13, 97, 23, 128]);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[11, 31, 51, 0]);
        doc.layers[0].alpha_locked = true;
        assert!(
            doc.fill_using(0, 0, [255, 0, 0], 0.5, 0, reference)
                .unwrap()
        );
        assert_eq!(doc.layers[0].pixels.rgba(0), [134, 49, 12, 128]);
        assert_eq!(doc.layers[0].pixels.rgba(1), [11, 31, 51, 0]);
        assert_eq!(doc.layers[0].pixels.rgba(2), [0; 4]);
    }
    #[test]
    fn invalid_reference_target_and_opacity_are_atomic_and_preserve_redo() {
        let doc = line_art();
        let target = doc.layers[0].id;
        for (captured, source, opacity) in [
            (target, Source::Layer(crate::document::LayerId::new()), 1.0),
            (doc.layers[1].id, Source::Active, 1.0),
            (target, Source::Active, f32::NAN),
        ] {
            assert!(
                Edit::FillTarget {
                    target: captured,
                    source,
                    point: [0, 0],
                    color: [17, 83, 201],
                    opacity,
                    tolerance: 0,
                    options: crate::fill::Options::default(),
                }
                .prepare(&doc)
                .is_err()
            );
            assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        }
        let mut app = Dudel {
            doc,
            ..Default::default()
        };
        app.set_canvas_property(target, crate::document_edits::Property::Opacity(0.5));
        app.undo();
        app.render_queue.edit(Edit::SelectLayer(0)).unwrap();
        app.fill_source = Source::Layer(crate::document::LayerId::new());
        app.fill_canvas(0, 0);
        app.flush_render_queue();
        assert!(app.status.contains("reference layer no longer exists"));
        assert!(app.history.can_redo() && !app.history.can_undo());
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
    }
    #[test]
    fn queued_reference_paint_then_fill_uses_completed_pixels_and_separate_undo() {
        use crate::{
            stroke::Sample,
            stroke_buffer::{Settings, StrokeBuffer},
        };
        let mut app = Dudel {
            doc: Document::new(13, 7),
            ..Default::default()
        };
        app.doc.add_layer();
        let line_id = app.doc.layers[1].id;
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 1.0,
                pressure_size: false,
                tip: crate::brush::Tip::Square,
                hardness: 1.0,
                flow: 1.0,
                ..Default::default()
            },
            color: [0; 3],
            erase: false,
            layer: 1,
            symmetry: Default::default(),
        };
        let points: Vec<_> = (0..7)
            .map(|y| Sample {
                x: 6.5,
                y: y as f32 + 0.5,
                pressure: 1.0,
                distance: y as f64,
                ..Default::default()
            })
            .collect();
        let mut expected = app.doc.clone();
        let mut buffer = StrokeBuffer::default();
        for &point in &points {
            buffer.paint(&mut expected, point, &settings);
        }
        buffer.finish_tail(&mut expected, &settings);
        let line = expected.layers[1].pixels.clone();
        assert!((0..7).all(|y| line.rgba(y * 13 + 6)[3] == 255));
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(points);
        assert!(app.select_layer_target(0));
        app.fill_source = Source::Layer(line_id);
        app.color = [17, 83, 201];
        app.brush.opacity = 1.0;
        app.fill_tolerance = 0;
        app.fill_canvas(0, 3);
        assert!(app.doc.layers[1].pixels.iter().all(|byte| *byte == 0));
        app.flush_render_queue();
        assert_eq!(app.doc.layers[1].pixels, line);
        for y in 0..7 {
            for x in 0..13 {
                assert_eq!(
                    app.doc.layers[0].pixels.rgba(y * 13 + x),
                    if x < 6 { [17, 83, 201, 255] } else { [0; 4] }
                );
            }
        }
        app.undo();
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert_eq!(app.doc.layers[1].pixels, line);
        app.undo();
        assert!(app.doc.layers[1].pixels.iter().all(|byte| *byte == 0));
        assert!(!app.history.can_undo());
    }
    #[test]
    fn visitation_bits_cover_word_edges_once() {
        let mut bits = vec![0u64; 129usize.div_ceil(64)];
        for index in 0..129 {
            assert!(visit(&mut bits, index));
            assert!(!visit(&mut bits, index));
        }
        assert_eq!(bits, [u64::MAX, u64::MAX, 1]);
        assert_eq!(
            (2048usize * 2048).div_ceil(64) * std::mem::size_of::<u64>(),
            524_288
        );
    }

    #[test]
    fn folder_references_reject_and_document_replacement_resets_session_layer_reference() {
        let mut doc = Document::new(5, 3);
        doc.group_active().unwrap();
        doc.active = 0;
        let folder = Source::Layer(doc.layers[1].id);
        assert!(doc.fill_using(0, 0, [17, 83, 201], 1.0, 0, folder).is_err());
        assert!(
            doc.fill_using(0, 0, [17, 83, 201], 0.0, 0, Source::Layer(LayerId::new()))
                .is_err()
        );
        assert!(doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        let mut app = Dudel {
            fill_source: Source::Layer(doc.layers[0].id),
            doc,
            ..Default::default()
        };
        app.apply_action(crate::document_actions::Action::New(5, 3));
        assert_eq!(app.fill_source, Source::Active);
        app.fill_source = Source::Visible;
        app.apply_action(crate::document_actions::Action::New(5, 3));
        assert_eq!(app.fill_source, Source::Visible);
    }

    #[test]
    fn saved_worker_fill_excludes_later_fill_and_keeps_two_undo_states() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("worker-fill.dudel");
        let mut app = Dudel {
            doc: Document::new(75, 73),
            path: Some(path.clone()),
            color: [17, 83, 201],
            ..Default::default()
        };
        app.brush.opacity = 1.0;
        app.fill_canvas(0, 0);
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        app.save(false);
        assert!(app.queued_save.is_some() && app.save_job.is_none());
        app.color = [201, 17, 83];
        app.fill_canvas(0, 0);
        app.flush_render_queue();
        assert!(
            app.doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [201, 17, 83, 255])
        );
        let job = app.save_job.take().unwrap();
        assert_ne!(job.revision, app.revision);
        job.worker.join().unwrap().unwrap();
        assert!(
            crate::storage::load_project(&path).unwrap().layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
        assert!(app.modified);
        app.undo();
        assert!(
            app.doc.layers[0]
                .pixels
                .chunks_exact(4)
                .all(|rgba| rgba == [17, 83, 201, 255])
        );
        app.undo();
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        assert!(!app.history.can_undo());
    }
}
