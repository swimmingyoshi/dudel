//! Smooth animation playback on any canvas size. Redrawing the canvas tile by
//! tile for every frame takes far longer than a frame lasts on a large
//! canvas, so playback shows each frame as one picture rendered at its
//! on-screen size (`Document::composite_reduced`), cached for the rest of
//! playback. A background job renders frames in playing order from the
//! playhead; the clock only moves on to a frame once it is ready, so the
//! first loop may hold a heavy frame briefly and later loops play from the
//! cache. Drawing never uses the cache.

use crate::animation::{self, Animation, Frame};
use crate::document::Document;
use crate::egui;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// Most memory the cached frames may use; larger animations are cached at
/// a lower resolution.
const BUDGET: usize = 512 << 20;

/// Where playback is: the frame it started on and the seconds played since,
/// which only advance while the next frame is ready.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Play {
    pub(crate) start: usize,
    pub(crate) played: f64,
    pub(crate) last: std::time::Instant,
}
impl Play {
    pub(crate) fn new(start: usize) -> Self {
        Self {
            start,
            played: 0.0,
            last: std::time::Instant::now(),
        }
    }
}

/// What a cache was made from: a change to any of these makes a new one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Key {
    pub(crate) revision: u64,
    pub(crate) step: usize,
    pub(crate) frames: usize,
}

pub(crate) struct Cache {
    pub(crate) key: Key,
    size: [usize; 2],
    textures: Vec<Option<egui::TextureHandle>>,
    results: mpsc::Receiver<(usize, Result<Vec<u8>, String>)>,
    cancel: Arc<AtomicBool>,
    worker: Option<crate::background::Handle<()>>,
    error: Option<String>,
}

/// The sampling step that shows a canvas at `zoom` without wasting pixels,
/// raised until `frames` pictures fit the memory budget.
pub(crate) fn step(width: usize, height: usize, zoom: f32, frames: usize) -> usize {
    let mut step = if zoom.is_finite() && zoom > 0.0 {
        ((1.0 / zoom).floor() as usize).max(1)
    } else {
        1
    };
    while frames * width.div_ceil(step) * height.div_ceil(step) * 4 > BUDGET && step < 64 {
        step += 1;
    }
    step
}

impl Cache {
    /// Start rendering every frame of `doc`, the `first` frame first.
    pub(crate) fn start(
        doc: Document,
        animation: Animation,
        frames: Vec<Frame>,
        key: Key,
        first: usize,
    ) -> Self {
        let count = frames.len();
        let size = [doc.width.div_ceil(key.step), doc.height.div_ceil(key.step)];
        let (sender, results) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = crate::background::spawn(move || {
            let check = || crate::fill::check_cancel(Some(&stop));
            for offset in 0..count {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let index = (first + offset) % count;
                let view = animation::frame_view(&doc, &animation, &frames, Some(index), false);
                let result = view
                    .composite_reduced(key.step, true, check)
                    .map(|(rgba, _)| rgba);
                if sender.send((index, result)).is_err() {
                    return;
                }
            }
        });
        Self {
            key,
            size,
            textures: vec![None; count],
            results,
            cancel,
            worker: Some(worker),
            error: None,
        }
    }

    /// Upload the frames the job has finished.
    pub(crate) fn poll(&mut self, ctx: &egui::Context, options: egui::TextureOptions) {
        while let Ok((index, result)) = self.results.try_recv() {
            match result {
                Ok(rgba) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(self.size, &rgba);
                    self.textures[index] =
                        Some(ctx.load_texture(format!("animation frame {index}"), image, options));
                }
                Err(error) => self.error = Some(error),
            }
        }
        if self.worker.as_ref().is_some_and(|w| w.is_finished()) {
            let _ = self.worker.take().unwrap().join();
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(4));
        }
    }

    pub(crate) fn frame(&self, index: usize) -> Option<&egui::TextureHandle> {
        self.textures.get(index)?.as_ref()
    }

    pub(crate) fn ready(&self) -> usize {
        self.textures.iter().filter(|t| t.is_some()).count()
    }

    pub(crate) fn len(&self) -> usize {
        self.textures.len()
    }

    pub(crate) fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Paint `texture` over the canvas, following the view's zoom, rotation and
/// mirroring.
pub(crate) fn paint(
    painter: &egui::Painter,
    view: crate::view::View,
    texture: &egui::TextureHandle,
) {
    paint_tinted(painter, view, texture, egui::Color32::WHITE);
}

/// `paint`, with each pixel multiplied by `tint`.
pub(crate) fn paint_tinted(
    painter: &egui::Painter,
    view: crate::view::View,
    texture: &egui::TextureHandle,
    tint: egui::Color32,
) {
    let corners = view.corners();
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (corner, uv) in corners.iter().zip([
        egui::pos2(0.0, 0.0),
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
    ]) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *corner,
            uv,
            color: tint,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_zoom_and_fit_the_memory_budget() {
        assert_eq!(step(3840, 2160, 0.39, 12), 2);
        assert_eq!(step(1920, 1080, 2.0, 12), 1);
        // 200 4K frames do not fit at step 1 or 2.
        let crowded = step(3840, 2160, 1.0, 200);
        assert!(crowded >= 3);
        assert!(200 * 3840usize.div_ceil(crowded) * 2160usize.div_ceil(crowded) * 4 <= BUDGET);
        assert_eq!(step(64, 64, f32::NAN, 1), 1);
    }

    #[test]
    fn the_job_renders_every_frame_reduced_from_the_playhead() {
        let mut doc = Document::new(8, 4);
        doc.layers = (0..3)
            .map(|i| {
                let mut layer = crate::document::Layer::raster(format!("F{i}"), 8, 4);
                layer.pixels.writer().write_region(
                    8,
                    [0, 0, 8, 4],
                    &[(i * 80) as u8, 0, 0, 255].repeat(32),
                );
                layer
            })
            .collect();
        let animation = Animation::default();
        let frames = animation::frames(&doc, &animation);
        let key = Key {
            revision: 1,
            step: 2,
            frames: 3,
        };
        let mut cache = Cache::start(doc, animation, frames, key, 1);
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while cache.ready() < 3 {
            assert!(std::time::Instant::now() < deadline);
            let _ = ctx.run(Default::default(), |ctx| {
                cache.poll(ctx, egui::TextureOptions::LINEAR)
            });
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(cache.size, [4, 2]);
        assert!(cache.take_error().is_none());
        assert_eq!(cache.len(), 3);
    }
}
