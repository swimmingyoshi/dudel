//! Images arriving from outside: clipboard bitmaps, files copied in a file
//! manager, and files dropped on the window. Every image becomes a new layer
//! above the current one through the clipboard paste transaction, so pasting
//! and dropping share ordering, undo and layer reservation. Images larger than
//! the canvas are scaled down to fit; nothing is cropped away.
use crate::{Dudel, clipboard::Raster, egui};
use std::path::{Path, PathBuf};

/// Formats Dudel can decode into a layer.
pub(crate) const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
/// Largest incoming image side; it is scaled to the canvas afterwards.
pub(crate) const MAX_SIDE: usize = 16384;
const MAX_PIXELS: usize = 64 * 1024 * 1024;

pub(crate) fn is_image(path: &Path) -> bool {
    extension(path).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Check an incoming image before it is scaled to the canvas.
pub(crate) fn validate_incoming(raster: &Raster) -> Result<(), String> {
    let pixels = raster
        .width
        .checked_mul(raster.height)
        .ok_or("Image dimensions overflow")?;
    if raster.width == 0
        || raster.height == 0
        || raster.width > MAX_SIDE
        || raster.height > MAX_SIDE
        || pixels > MAX_PIXELS
    {
        return Err(format!(
            "Images can be at most {MAX_SIDE} × {MAX_SIDE} pixels"
        ));
    }
    if raster.rgba.len() != pixels * 4 {
        return Err("Image has invalid RGBA data".into());
    }
    Ok(())
}

/// Decode an image file of any supported format, guessing from its bytes.
pub(crate) fn decode(path: &Path) -> Result<Raster, String> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE as u32);
    limits.max_image_height = Some(MAX_SIDE as u32);
    limits.max_alloc = Some((MAX_PIXELS * 4) as u64 + (64 << 20));
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|e| format!("{}: {e}", file_name(path)))?;
    let (width, height) = decoder.dimensions();
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("{}: {e}", file_name(path)))?
        .into_rgba8();
    let raster = Raster {
        width: width as usize,
        height: height as usize,
        rgba: image.into_raw(),
    };
    validate_incoming(&raster)?;
    Ok(raster)
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Scale `raster` down (never up) so it fits inside `width` × `height`.
pub(crate) fn fit(raster: Raster, width: usize, height: usize) -> Result<Raster, String> {
    if raster.width <= width && raster.height <= height {
        return Ok(raster);
    }
    let scale = (width as f64 / raster.width as f64).min(height as f64 / raster.height as f64);
    let w = ((raster.width as f64 * scale).round() as u32).clamp(1, width as u32);
    let h = ((raster.height as f64 * scale).round() as u32).clamp(1, height as u32);
    let source = image::RgbaImage::from_raw(raster.width as u32, raster.height as u32, raster.rgba)
        .ok_or("Invalid image pixels")?;
    let resized = crate::layer_import::resize_rgba(&source, w, h);
    Ok(Raster {
        width: w as usize,
        height: h as usize,
        rgba: resized.into_raw(),
    })
}

/// Top-left corner placing a `size` image centered on `center`, kept on the canvas.
pub(crate) fn placement(size: [usize; 2], canvas: [usize; 2], center: [f32; 2]) -> [usize; 2] {
    let place = |size: usize, canvas: usize, center: f32| {
        let start = (center - size as f32 / 2.0).round().max(0.0) as usize;
        start.min(canvas - size)
    };
    [
        place(size[0], canvas[0], center[0]),
        place(size[1], canvas[1], center[1]),
    ]
}

/// What a dropped or pasted file is for.
#[derive(Debug, PartialEq)]
pub(crate) enum Dropped {
    Image(PathBuf),
    Project(PathBuf),
    Package(PathBuf),
    Brushes(PathBuf),
    Palette(PathBuf),
    Theme(PathBuf),
    Unsupported(PathBuf),
}
pub(crate) fn classify(path: PathBuf) -> Dropped {
    match extension(&path).as_deref() {
        _ if is_image(&path) => Dropped::Image(path),
        Some("dudel" | "clip" | "psd" | "didx") => Dropped::Project(path),
        Some("zip") => Dropped::Package(path),
        Some("dudelbrushes") => Dropped::Brushes(path),
        Some("dudelpalette") => Dropped::Palette(path),
        Some("dudeltheme") => Dropped::Theme(path),
        _ => Dropped::Unsupported(path),
    }
}

impl Dudel {
    /// Add each image file as a new layer, stacked in order. `at` centers
    /// them on a canvas point, such as where they were dropped.
    pub(crate) fn paste_image_files(&mut self, paths: Vec<PathBuf>, at: Option<[f32; 2]>) {
        let total = paths.len();
        let mut queued = 0;
        for path in paths {
            match self.queue_paste_layer(crate::clipboard::PasteSource::File(path), at) {
                Ok(()) => queued += 1,
                Err(error) => {
                    self.status = format!("Clipboard: {error}");
                    break;
                }
            }
        }
        if queued > 1 {
            self.status = format!("Adding {queued} images as layers…");
        }
        if queued < total && queued > 0 {
            self.status = format!(
                "Added {queued} of {total} images · {}",
                "the rest exceed the layer or queue limit"
            );
        }
    }
    /// Files copied in a file manager, if the clipboard holds any images.
    pub(crate) fn clipboard_image_files(&mut self) -> Vec<PathBuf> {
        #[cfg(windows)]
        let files = crate::clipboard_files::read();
        #[cfg(not(windows))]
        let files = self.clipboard_file_list();
        files.into_iter().filter(|p| is_image(p)).collect()
    }
    #[cfg(not(windows))]
    fn clipboard_file_list(&mut self) -> Vec<PathBuf> {
        let Ok(clipboard) = self.clipboard_backend() else {
            return Vec::new();
        };
        clipboard.get().file_list().unwrap_or_default()
    }
    /// Edit › Paste as reference image: the clipboard image or copied files
    /// become a new reference layer.
    pub(crate) fn paste_as_reference(&mut self) {
        let files = self.clipboard_image_files();
        if let Some(path) = files.into_iter().next() {
            self.add_reference_layer_from(path);
            return;
        }
        let image = self.clipboard_backend().and_then(|c| {
            c.get_image()
                .map_err(|_| "The clipboard holds no image".to_string())
        });
        match image {
            Ok(image) => {
                let raster = Raster {
                    width: image.width,
                    height: image.height,
                    rgba: image.bytes.into_owned(),
                };
                match validate_incoming(&raster).and_then(|()| {
                    crate::reference_layers::downscale(crate::reference_layers::Image {
                        width: raster.width,
                        height: raster.height,
                        rgba: raster.rgba,
                    })
                }) {
                    Ok(image) => self.add_reference_image_layer("Pasted reference".into(), image),
                    Err(error) => self.status = format!("Reference: {error}"),
                }
            }
            Err(error) => self.status = format!("Reference: {error}"),
        }
    }
    /// Files dropped on the window: images become layers at the drop point,
    /// projects open in tabs, and libraries are imported.
    pub(crate) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, pointer) =
            ctx.input(|i| (i.raw.dropped_files.clone(), i.pointer.latest_pos()));
        if dropped.is_empty() {
            return;
        }
        if self.dialogs.open() || self.unsaved_action.is_some() || self.new_canvas {
            self.status = "Close the open dialog before dropping files".into();
            return;
        }
        let at = pointer.filter(|p| self.canvas_area.contains(*p)).map(|p| {
            let point = self.view(self.canvas_area).to_doc(p);
            [point.x, point.y]
        });
        let mut images = Vec::new();
        let mut unsupported = Vec::new();
        for file in dropped {
            let Some(path) = file.path else {
                continue;
            };
            match classify(path) {
                Dropped::Image(path) => images.push(path),
                Dropped::Project(path) => self.pending_opens.push(path),
                Dropped::Package(path) => self.import_package_from(path),
                Dropped::Brushes(path) => self.import_brush_pack(path),
                Dropped::Palette(path) => self.import_palette(path),
                Dropped::Theme(path) => self.import_theme_file(path),
                Dropped::Unsupported(path) => unsupported.push(file_name(&path)),
            }
        }
        if !images.is_empty() {
            self.paste_image_files(images, at);
        }
        if !unsupported.is_empty() {
            self.status = format!(
                "Cannot open {} · drop images ({}), projects or packages",
                unsupported.join(", "),
                EXTENSIONS.join(", ")
            );
        }
    }
    /// Open dropped projects one at a time, each in its own tab.
    pub(crate) fn open_pending_projects(&mut self) {
        if self.pending_opens.is_empty() || self.load_job.is_some() || self.tab_open_pending() {
            return;
        }
        let path = self.pending_opens.remove(0);
        self.open_path(path);
    }
    /// Show what a drag in progress will do.
    pub(crate) fn drop_overlay(&self, ctx: &egui::Context) {
        let hovered = ctx.input(|i| i.raw.hovered_files.clone());
        if hovered.is_empty() {
            return;
        }
        let images = hovered
            .iter()
            .filter(|f| f.path.as_deref().is_some_and(is_image))
            .count();
        let others = hovered.len() - images;
        let text = match (images, others) {
            (0, _) => "Drop to open".to_owned(),
            (1, 0) => "Drop to add the image as a new layer".to_owned(),
            (n, 0) => format!("Drop to add {n} images as new layers"),
            (n, _) => format!("Drop to add {n} images as layers and open the other files"),
        };
        let area = if self.canvas_area.width() > 1.0 {
            self.canvas_area
        } else {
            ctx.content_rect()
        };
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("file drop overlay"),
        ));
        let accent = ctx.style().visuals.selection.bg_fill;
        painter.rect_filled(area.shrink(6.0), 10.0, accent.gamma_multiply(0.18));
        painter.rect_stroke(
            area.shrink(6.0),
            10.0,
            egui::Stroke::new(2.0_f32, accent),
            egui::StrokeKind::Inside,
        );
        let galley = painter.layout_no_wrap(
            text,
            egui::FontId::proportional(18.0),
            ctx.style().visuals.text_color(),
        );
        let pill =
            egui::Rect::from_center_size(area.center(), galley.size() + egui::vec2(32.0, 18.0));
        painter.rect_filled(pill, 8.0, ctx.style().visuals.panel_fill);
        painter.galley(
            pill.center() - galley.size() / 2.0,
            galley,
            ctx.style().visuals.text_color(),
        );
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_common_formats_and_scales_oversized_images_to_the_canvas() {
        let dir = tempfile::tempdir().unwrap();
        let mut image = image::RgbaImage::new(40, 20);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgba([if x < 20 { 255 } else { 0 }, 0, 0, 255]);
        }
        for name in ["a.png", "b.jpg", "c.bmp", "d.webp"] {
            let path = dir.path().join(name);
            if name.ends_with(".jpg") {
                // JPEG has no alpha channel.
                image::DynamicImage::ImageRgba8(image.clone())
                    .to_rgb8()
                    .save(&path)
                    .unwrap();
            } else {
                image.save(&path).unwrap();
            }
            let raster = decode(&path).unwrap();
            assert_eq!([raster.width, raster.height], [40, 20], "{name}");
            assert!(raster.rgba[0] > 200, "{name}");
        }
        // A PNG saved with the wrong extension still decodes from its bytes.
        let renamed = dir.path().join("photo.jpg");
        image
            .save_with_format(&renamed, image::ImageFormat::Png)
            .unwrap();
        assert_eq!(decode(&renamed).unwrap().width, 40);
        std::fs::write(dir.path().join("bad.png"), b"not an image").unwrap();
        assert!(decode(&dir.path().join("bad.png")).is_err());
        let raster = decode(&dir.path().join("a.png")).unwrap();
        let fitted = fit(raster, 10, 10).unwrap();
        assert_eq!([fitted.width, fitted.height], [10, 5]);
        let small = fit(decode(&dir.path().join("a.png")).unwrap(), 100, 100).unwrap();
        assert_eq!(small.width, 40, "never scaled up");
    }
    #[test]
    fn a_frame_with_dropped_files_adds_layers_at_the_pointer_and_reports_unsupported_ones() {
        let dir = tempfile::tempdir().unwrap();
        let dot = dir.path().join("Dot.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .save(&dot)
            .unwrap();
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            ..Default::default()
        };
        // 32×32 canvas shown at 2× in a 100×100 area starting at (10, 10).
        app.canvas_area =
            egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(100.0, 100.0));
        app.zoom = 2.0;
        let ctx = egui::Context::default();
        let screen = egui::pos2(60.0 + 2.0 * 10.0, 60.0 - 2.0 * 6.0); // canvas (26, 10)
        let raw = egui::RawInput {
            events: vec![egui::Event::PointerMoved(screen)],
            dropped_files: vec![
                egui::DroppedFile {
                    path: Some(dot),
                    ..Default::default()
                },
                egui::DroppedFile {
                    path: Some("notes.txt".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.handle_dropped_files(ctx));
        assert!(app.status.contains("notes.txt"), "{}", app.status);
        app.flush_render_queue();
        while app.paste_job.is_some() || !app.pending_clipboard.is_empty() {
            app.wait_paste();
            app.flush_render_queue();
        }
        assert_eq!(app.doc.layers.len(), 2);
        assert_eq!(app.doc.layers[1].name, "Dot");
        let pixels = &app.doc.layers[1].pixels;
        assert_eq!(pixels.rgba(9 * 32 + 25), [1, 2, 3, 255]);
        assert_eq!(pixels.rgba(10 * 32 + 26), [1, 2, 3, 255]);
        assert_eq!(pixels.rgba(8 * 32 + 24), [0; 4]);
    }
    #[test]
    fn placement_centers_on_the_point_but_stays_on_the_canvas() {
        assert_eq!(placement([10, 10], [100, 100], [50.0, 50.0]), [45, 45]);
        assert_eq!(placement([10, 10], [100, 100], [2.0, 99.0]), [0, 90]);
        assert_eq!(placement([100, 50], [100, 100], [80.0, 80.0]), [0, 50]);
    }
    #[test]
    fn dropped_files_are_routed_by_kind() {
        assert_eq!(classify("a.JPEG".into()), Dropped::Image("a.JPEG".into()));
        assert_eq!(
            classify("a.dudel".into()),
            Dropped::Project("a.dudel".into())
        );
        assert_eq!(classify("a.clip".into()), Dropped::Project("a.clip".into()));
        assert_eq!(classify("a.zip".into()), Dropped::Package("a.zip".into()));
        assert_eq!(
            classify("a.dudeltheme".into()),
            Dropped::Theme("a.dudeltheme".into())
        );
        assert_eq!(
            classify("a.txt".into()),
            Dropped::Unsupported("a.txt".into())
        );
    }
    #[test]
    fn dropped_and_pasted_files_become_stacked_layers_with_one_undo_each() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("Big photo.jpg");
        image::RgbImage::from_pixel(64, 32, image::Rgb([10, 200, 30]))
            .save(&big)
            .unwrap();
        let small = dir.path().join("Sticker.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 10, 30, 255]))
            .save(&small)
            .unwrap();
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            ..Default::default()
        };
        app.paste_image_files(vec![big, small], Some([1.0, 14.0]));
        app.flush_render_queue();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.paste_job.is_some() || !app.pending_clipboard.is_empty() {
            assert!(std::time::Instant::now() < deadline, "{}", app.status);
            app.wait_paste();
            app.flush_render_queue();
        }
        assert_eq!(app.doc.layers.len(), 3, "{}", app.status);
        assert_eq!(app.doc.layers[1].name, "Big photo");
        assert_eq!(app.doc.layers[2].name, "Sticker");
        assert_eq!(app.doc.active, 2);
        // The 64×32 photo is scaled to 16×8 and kept on the canvas, so the
        // drop near the bottom edge puts it in rows 8-15.
        let photo = &app.doc.layers[1].pixels;
        assert!(photo.rgba(12 * 16 + 8)[1] > 150);
        assert_eq!(photo.rgba(7 * 16 + 8), [0; 4]);
        // The sticker sits at the drop point, kept inside the canvas.
        assert_eq!(app.doc.layers[2].pixels.rgba(12 * 16)[0], 200);
        assert_eq!(app.doc.layers[2].pixels.rgba(11 * 16)[3], 0);
        for remaining in [2, 1] {
            app.undo();
            app.flush_render_queue();
            assert_eq!(app.doc.layers.len(), remaining);
        }
    }
}
