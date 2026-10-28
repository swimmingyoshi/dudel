//! Display-only mask boundaries, built off-thread and guarded by source identity.
use crate::{egui, selection_mask::Mask, snapshot_memory::Registry, view::View};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const MAX_SEGMENTS: usize = 8192;
const MAX_EDGE_LENGTH: usize = 16_384;
#[derive(Debug)]
pub(crate) struct Contour {
    segments: Vec<[u16; 4]>,
    pub(crate) step: usize,
}
impl Contour {
    fn build(mask: &Mask, cancel: &AtomicBool) -> Result<Self, String> {
        crate::fill::check_cancel(Some(cancel))?;
        if mask.bounds().is_none() {
            return Ok(Self {
                segments: Vec::new(),
                step: 1,
            });
        }
        let [width, height] = mask.dimensions();
        let mut step = 1;
        loop {
            crate::fill::check_cancel(Some(cancel))?;
            let (columns, rows) = (width.div_ceil(step), height.div_ceil(step));
            let mut grid = vec![0u8; columns * rows];
            if let Some(bounds) = mask.bounds() {
                for y in bounds.top..bounds.bottom {
                    crate::fill::check_cancel(Some(cancel))?;
                    for x in bounds.left..bounds.right {
                        if mask.at(x, y) > 0 {
                            grid[(y / step) * columns + x / step] = 1;
                        }
                    }
                }
            }
            let mut segments = Vec::with_capacity(MAX_SEGMENTS);
            if collect(
                &grid,
                [columns, rows],
                [width, height],
                step,
                cancel,
                &mut segments,
            )? {
                crate::fill::check_cancel(Some(cancel))?;
                return Ok(Self { segments, step });
            }
            step *= 2;
        }
    }
    fn bytes(&self) -> usize {
        self.segments.capacity() * std::mem::size_of::<[u16; 4]>()
    }
    pub(crate) fn paint(&self, painter: &egui::Painter, view: View) {
        for &[x0, y0, x1, y1] in &self.segments {
            let a = view.to_screen(egui::pos2(f32::from(x0), f32::from(y0)));
            let b = view.to_screen(egui::pos2(f32::from(x1), f32::from(y1)));
            if let Some(points) = clip([a, b], painter.clip_rect()) {
                painter.line_segment(points, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
                painter.extend(egui::Shape::dashed_line(
                    &points,
                    egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
                    5.0,
                    5.0,
                ));
            }
        }
    }
}
fn collect(
    grid: &[u8],
    [columns, rows]: [usize; 2],
    [width, height]: [usize; 2],
    step: usize,
    cancel: &AtomicBool,
    segments: &mut Vec<[u16; 4]>,
) -> Result<bool, String> {
    let mut edge_length = 0;
    let occupied = |x: usize, y: usize| x < columns && y < rows && grid[y * columns + x] != 0;
    for horizontal in [true, false] {
        let (lines, length) = if horizontal {
            (rows + 1, columns)
        } else {
            (columns + 1, rows)
        };
        for line in 0..lines {
            crate::fill::check_cancel(Some(cancel))?;
            let mut start = None;
            for p in 0..=length {
                let edge = p < length
                    && if horizontal {
                        (line > 0 && occupied(p, line - 1)) != occupied(p, line)
                    } else {
                        (line > 0 && occupied(line - 1, p)) != occupied(line, p)
                    };
                if edge && start.is_none() {
                    start = Some(p);
                }
                if !edge && let Some(from) = start.take() {
                    if segments.len() == MAX_SEGMENTS {
                        return Ok(false);
                    }
                    let (a, b) = if horizontal {
                        (
                            [from * step, (line * step).min(height)],
                            [(p * step).min(width), (line * step).min(height)],
                        )
                    } else {
                        (
                            [(line * step).min(width), from * step],
                            [(line * step).min(width), (p * step).min(height)],
                        )
                    };
                    edge_length += a[0].abs_diff(b[0]) + a[1].abs_diff(b[1]);
                    if edge_length > MAX_EDGE_LENGTH {
                        return Ok(false);
                    }
                    segments.push([a[0] as u16, a[1] as u16, b[0] as u16, b[1] as u16]);
                }
            }
        }
    }
    Ok(true)
}
/// Clip before dash tessellation so off-screen zoomed edges cannot create huge
/// geometry. Slab clipping also works for rotated/mirrored canvas placement.
fn clip([a, b]: [egui::Pos2; 2], rect: egui::Rect) -> Option<[egui::Pos2; 2]> {
    let delta = b - a;
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    for (origin, direction, min, max) in [
        (a.x, delta.x, rect.min.x, rect.max.x),
        (a.y, delta.y, rect.min.y, rect.max.y),
    ] {
        if direction == 0.0 {
            if origin < min || origin > max {
                return None;
            }
        } else {
            let t0 = (min - origin) / direction;
            let t1 = (max - origin) / direction;
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
            if lo > hi {
                return None;
            }
        }
    }
    Some([a + delta * lo, a + delta * hi])
}
struct Job {
    source: Mask,
    cancel: Arc<AtomicBool>,
    worker: std::thread::JoinHandle<Result<Contour, String>>,
}
#[derive(Default)]
pub(crate) struct Cache {
    ready: Option<(Mask, Contour)>,
    job: Option<Job>,
    failed: Option<Mask>,
}
impl Cache {
    pub(crate) fn update(&mut self, mask: Option<&Mask>, ctx: &egui::Context) -> Option<String> {
        let matches = |source: &Mask| mask.is_some_and(|mask| mask.shares(source));
        if self
            .ready
            .as_ref()
            .is_some_and(|(source, _)| !matches(source))
        {
            self.ready = None;
        }
        if self.failed.as_ref().is_some_and(|source| !matches(source)) {
            self.failed = None;
        }
        if let Some(job) = &self.job {
            if !matches(&job.source) {
                job.cancel.store(true, Ordering::Relaxed);
            }
            if !job.worker.is_finished() {
                ctx.request_repaint_after(std::time::Duration::from_millis(8));
                return None;
            }
        }
        if let Some(job) = self.job.take() {
            // Joining a finished handle cannot wait for raster computation.
            let Job {
                source,
                cancel,
                worker,
            } = job;
            let result = worker
                .join()
                .unwrap_or_else(|_| Err("Selection contour worker failed".into()));
            if matches(&source) && !cancel.load(Ordering::Relaxed) {
                match result {
                    Ok(contour) => self.ready = Some((source, contour)),
                    Err(error) => {
                        self.failed = Some(source);
                        return Some(error);
                    }
                }
            }
        }
        if self.ready.is_none()
            && self.failed.is_none()
            && let Some(mask) = mask
        {
            let source = mask.clone();
            let frozen = source.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let control = cancel.clone();
            let repaint = ctx.clone();
            match std::thread::Builder::new()
                .name("dudel-selection-contour".into())
                .spawn(move || {
                    let result = Contour::build(&frozen, &control);
                    repaint.request_repaint();
                    result
                }) {
                Ok(worker) => {
                    self.job = Some(Job {
                        source,
                        cancel,
                        worker,
                    })
                }
                Err(error) => {
                    self.failed = Some(source);
                    return Some(format!("Could not start selection contour: {error}"));
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        None
    }
    pub(crate) fn contour(&self, mask: &Mask) -> Option<&Contour> {
        self.ready
            .as_ref()
            .filter(|(source, _)| source.shares(mask))
            .map(|(_, contour)| contour)
    }
    pub(crate) fn register_allocations(&self, registry: &mut Registry) {
        if let Some((mask, contour)) = &self.ready {
            let (id, bytes) = mask.allocation();
            registry.add_buffer(id, bytes);
            registry.add_buffer(contour.segments.as_ptr() as usize, contour.bytes());
        }
        if let Some(job) = &self.job {
            let (id, bytes) = job.source.allocation();
            registry.add_buffer(id, bytes);
        }
        if let Some(mask) = &self.failed {
            let (id, bytes) = mask.allocation();
            registry.add_buffer(id, bytes);
        }
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        for shape in ["ring", "islands", "checkerboard"] {
            let mut bytes = vec![0; size * size];
            for y in 0..size {
                for x in 0..size {
                    let inside = match shape {
                        "ring" => {
                            (size / 4..size * 3 / 4).contains(&x)
                                && (size / 4..size * 3 / 4).contains(&y)
                                && !((size * 3 / 8..size * 5 / 8).contains(&x)
                                    && (size * 3 / 8..size * 5 / 8).contains(&y))
                        }
                        "islands" => x % 2 == 0 && y % 2 == 0,
                        _ => (x + y) % 2 == 0,
                    };
                    if inside {
                        bytes[y * size + x] = if shape == "ring" { 64 } else { 255 };
                    }
                }
            }
            let mask = Mask::from_coverage(size, size, bytes)?;
            let frozen = mask.clone();
            let ctx = egui::Context::default();
            let mut cache = Cache::default();
            let started = std::time::Instant::now();
            let mut max_owner_ms = 0.0_f64;
            let mut calls = 0;
            while cache.contour(&mask).is_none() {
                let owner = std::time::Instant::now();
                if let Some(error) = cache.update(Some(&mask), &ctx) {
                    return Err(error);
                }
                max_owner_ms = max_owner_ms.max(owner.elapsed().as_secs_f64() * 1000.0);
                calls += 1;
                if started.elapsed() > std::time::Duration::from_secs(30) {
                    return Err("Contour probe did not finish".into());
                }
                if cache
                    .job
                    .as_ref()
                    .is_some_and(|job| !job.worker.is_finished())
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            let contour = cache.contour(&mask).unwrap();
            let expected = if shape == "ring" {
                let (a, b, c, d) = (
                    (size / 4) as u16,
                    (size * 3 / 8) as u16,
                    (size * 5 / 8) as u16,
                    (size * 3 / 4) as u16,
                );
                vec![
                    [a, a, d, a],
                    [b, b, c, b],
                    [b, c, c, c],
                    [a, d, d, d],
                    [a, a, a, d],
                    [b, b, b, c],
                    [c, b, c, c],
                    [d, a, d, d],
                ]
            } else {
                let d = size as u16;
                vec![[0, 0, d, 0], [0, d, d, d], [0, 0, 0, d], [d, 0, d, d]]
            };
            if contour.segments != expected
                || !mask.shares(&frozen)
                || (shape == "ring" && contour.step != 1)
                || (shape != "ring" && contour.step != 2)
            {
                return Err("Contour geometry/ownership oracle differs".into());
            }
            let mut paint = Vec::new();
            for zoom in [600.0 / size as f32, 8.0] {
                let mut view = View {
                    origin: egui::pos2(450.0, 350.0),
                    zoom,
                    rotation: 0.37,
                    mirror: true,
                    half: egui::vec2(size as f32 / 2.0, size as f32 / 2.0),
                };
                if zoom > 2.0 {
                    let coordinate = if shape == "ring" {
                        size as f32 / 4.0
                    } else {
                        0.0
                    };
                    view.origin += egui::pos2(450.0, 350.0)
                        - view.to_screen(egui::pos2(coordinate, coordinate));
                }
                let owner = std::time::Instant::now();
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            contour.paint(ui.painter(), view);
                        });
                    },
                );
                let shape_ms = owner.elapsed().as_secs_f64() * 1000.0;
                let owner = std::time::Instant::now();
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                let tessellate_ms = owner.elapsed().as_secs_f64() * 1000.0;
                let vertices: usize = primitives
                    .iter()
                    .map(|p| match &p.primitive {
                        egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                        _ => 0,
                    })
                    .sum();
                paint.push(serde_json::json!({"zoom":zoom,"shape_generation_ms":shape_ms,"tessellation_ms":tessellate_ms,"vertices":vertices}));
            }
            cases.push(serde_json::json!({"size":size,"shape":shape,"grid_step":contour.step,"segments":contour.segments.len(),"payload_capacity_bytes":contour.bytes(),"max_owner_update_ms":max_owner_ms,"owner_calls":calls,"wall_ms":wall_ms,"geometry_oracle_verified":true,"mask_source_shared":true,"paint":paint}));
        }
    }
    crate::storage::atomic_write(path,&serde_json::to_vec_pretty(&serde_json::json!({"version":1,"scope":"One repetition per size/shape. Async cache owner update includes worker start, ready-only join and guarded publication; 1 ms polling sleeps excluded from owner timing, included in wall. Fixture construction/verification excluded. Exact low-coverage ring edges and conservative 2 px fragmented outlines verified against explicit geometry. Headless egui shape generation and tessellation measured separately with rotated/mirrored fit and 8x views; includes egui frame setup, excludes real GUI/GPU/display/input. Mask payload is shared; contour capacity only excludes worker scratch, handles, UI geometry and RSS. Display coarsens beyond 8192 segments or 16384 document edge pixels; editing coverage stays exact.","cases":cases})).map_err(|e|e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn edges(contour: &Contour) -> BTreeSet<[u16; 4]> {
        let mut edges = BTreeSet::new();
        for &[x0, y0, x1, y1] in &contour.segments {
            if y0 == y1 {
                for x in x0..x1 {
                    assert!(edges.insert([x, y0, x + 1, y0]));
                }
            } else {
                for y in y0..y1 {
                    assert!(edges.insert([x0, y, x0, y + 1]));
                }
            }
        }
        edges
    }
    #[test]
    fn exact_contours_match_independent_pixel_edge_oracle_with_holes_and_feather() {
        for seed in 0..8 {
            let (width, height) = (31, 17);
            let bytes: Vec<u8> = (0..width * height)
                .map(|i| {
                    if (i * 73 + seed * 17) % 11 < 5 {
                        0
                    } else {
                        ((i % 254) + 1) as u8
                    }
                })
                .collect();
            let mask = Mask::from_coverage(width, height, bytes.clone()).unwrap();
            let contour = Contour::build(&mask, &AtomicBool::new(false)).unwrap();
            assert_eq!(contour.step, 1);
            let mut expected = BTreeSet::new();
            let occupied = |x: usize, y: usize| x < width && y < height && bytes[y * width + x] > 0;
            for y in 0..height {
                for x in 0..width {
                    if !occupied(x, y) {
                        continue;
                    }
                    let (xx, yy) = (x as u16, y as u16);
                    if y == 0 || !occupied(x, y - 1) {
                        expected.insert([xx, yy, xx + 1, yy]);
                    }
                    if !occupied(x, y + 1) {
                        expected.insert([xx, yy + 1, xx + 1, yy + 1]);
                    }
                    if x == 0 || !occupied(x - 1, y) {
                        expected.insert([xx, yy, xx, yy + 1]);
                    }
                    if !occupied(x + 1, y) {
                        expected.insert([xx + 1, yy, xx + 1, yy + 1]);
                    }
                }
            }
            assert_eq!(edges(&contour), expected);
            assert_eq!(mask.bytes(), bytes);
        }
    }
    #[test]
    fn fragmented_outline_coarsens_with_bounded_geometry_and_empty_is_allocation_free() {
        let (width, height) = (517, 259);
        let bytes = (0..width * height)
            .map(|i| {
                if (i % width) % 2 == 0 && (i / width) % 2 == 0 {
                    255
                } else {
                    0
                }
            })
            .collect();
        let mask = Mask::from_coverage(width, height, bytes).unwrap();
        let original = mask.clone();
        let contour = Contour::build(&mask, &AtomicBool::new(false)).unwrap();
        assert_eq!(contour.step, 2);
        assert_eq!(contour.segments.len(), 4);
        assert!(contour.bytes() <= MAX_SEGMENTS * 8 && mask.shares(&original));
        assert_eq!(
            contour.segments,
            vec![
                [0, 0, 517, 0],
                [0, 259, 517, 259],
                [0, 0, 0, 259],
                [517, 0, 517, 259]
            ]
        );
        let empty = Mask::from_coverage(1, 1, vec![0]).unwrap();
        let contour = Contour::build(&empty, &AtomicBool::new(false)).unwrap();
        assert!(contour.segments.is_empty() && contour.bytes() == 0);
        assert!(Contour::build(&mask, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn stale_contour_worker_coalesces_and_never_publishes_for_another_mask() {
        let ctx = egui::Context::default();
        let a = Mask::from_coverage(2, 1, vec![255, 0]).unwrap();
        let b = Mask::from_coverage(2, 1, vec![0, 255]).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let control = cancel.clone();
        let source = a.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            receive.recv().unwrap();
            Contour::build(&source, &control)
        });
        let mut cache = Cache {
            job: Some(Job {
                source: a.clone(),
                cancel: cancel.clone(),
                worker,
            }),
            ready: None,
            failed: None,
        };
        assert!(cache.update(Some(&b), &ctx).is_none());
        assert!(cancel.load(Ordering::Relaxed) && cache.contour(&b).is_none());
        assert!(cache.job.as_ref().unwrap().source.shares(&a));
        send.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while cache.contour(&b).is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "Contour workers did not complete"
            );
            assert!(cache.update(Some(&b), &ctx).is_none());
            assert!(cache.contour(&a).is_none());
            std::thread::yield_now();
        }
        assert!(cache.contour(&a).is_none() && cache.contour(&b).is_some());
        let mut doc = crate::document::Document::new(2, 1);
        doc.selection = b.bounds();
        doc.selection_mask = Some(b);
        let mut registry = Registry::new(&doc);
        cache.register_allocations(&mut registry);
        assert_eq!(
            registry.additional_bytes(),
            cache.ready.as_ref().unwrap().1.bytes()
        );
        cache.update(None, &ctx);
        assert!(cache.ready.is_none() && cache.job.is_none());
    }
    #[test]
    fn segment_clipping_bounds_dashes_for_zoomed_rotated_edges() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0));
        assert_eq!(
            clip(
                [egui::pos2(-10000.0, 50.0), egui::pos2(10000.0, 50.0)],
                rect
            ),
            Some([egui::pos2(0.0, 50.0), egui::pos2(100.0, 50.0)])
        );
        assert!(clip([egui::pos2(-5.0, 0.0), egui::pos2(-5.0, 100.0)], rect).is_none());
        let clipped = clip([egui::pos2(-100.0, -100.0), egui::pos2(200.0, 200.0)], rect).unwrap();
        assert_eq!(clipped, [egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0)]);
    }
}
