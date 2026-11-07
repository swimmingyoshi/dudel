//! Ordered, snapshot-based PNG, PSD and package export. Export never changes
//! project save state.
use crate::{Dudel, document::Document, egui, snapshot_memory::SourceWorker};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ExportKind {
    Png {
        transparent: bool,
    },
    Raster {
        format: image::ImageFormat,
        transparent: bool,
    },
    Psd {
        effects: crate::psd_export::Effects,
    },
    /// A .zip project package.
    Package,
}
impl ExportKind {
    fn label(self) -> &'static str {
        match self {
            Self::Png { .. } => "PNG",
            Self::Raster { format, .. } => match format {
                image::ImageFormat::Jpeg => "JPEG",
                image::ImageFormat::WebP => "WebP",
                image::ImageFormat::Bmp => "BMP",
                _ => "GIF",
            },
            Self::Psd { .. } => "PSD",
            Self::Package => "Project package",
        }
    }
}
pub(crate) struct QueuedExport {
    pub ticket: u64,
    pub path: PathBuf,
    pub kind: ExportKind,
    pub canceled: bool,
}
pub(crate) struct ExportJob {
    pub worker: SourceWorker<Result<(), String>>,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    raster_bytes: usize,
    label: &'static str,
    /// What the export left out or changed, shown when it finishes.
    note: String,
}
impl ExportJob {
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        self.worker.register_source(registry);
        if !self.worker.is_finished() {
            // Conservatively reserve the entire output throughout the job.
            // PNG codec and allocator overhead remain separate.
            registry.add_owned(self.raster_bytes);
        }
    }
}

fn destination(path: &Path) -> Result<(), String> {
    destination_of(path, "png", b"\x89PNG\r\n\x1a\n", "PNG")
}
fn psd_destination(path: &Path) -> Result<(), String> {
    destination_of(path, "psd", b"8BPS", "PSD")
}
fn destination_of(path: &Path, extension: &str, magic: &[u8], kind: &str) -> Result<(), String> {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(extension))
    {
        return Err(format!("Choose a .{extension} filename for {kind} export"));
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            // A renamed project/source must never be destroyed by image export.
            if file.metadata().map_err(|e| e.to_string())?.len() != 0 {
                let mut signature = vec![0; magic.len()];
                if file.read_exact(&mut signature).is_err() || signature != magic {
                    return Err(format!(
                        "Export destination is not a {kind}; choose a new filename"
                    ));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

fn write_png(
    doc: &Document,
    path: &Path,
    transparent: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    write_png_with_hook(doc, path, transparent, cancel, || {})
}

fn write_png_with_hook(
    doc: &Document,
    path: &Path,
    transparent: bool,
    cancel: &AtomicBool,
    before_publish: impl FnOnce(),
) -> Result<(), String> {
    doc.validate()?;
    destination(path)?;
    let check = || crate::fill::check_cancel(Some(cancel));
    let rgba = doc.composite_bounds_checked([0, 0, doc.width, doc.height], transparent, check)?;
    crate::storage::atomic_stream(path, |file| {
        check()?;
        let mut encoder = png::Encoder::new(file, doc.width as u32, doc.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let ppm = (doc.dpi / 0.0254).round() as u32;
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
        // PNG encoding itself is not interruptible; cancel before publication.
        before_publish();
        check()?;
        destination(path)
    })
}

fn raster_destination(path: &Path, format: image::ImageFormat) -> Result<(), String> {
    if image::ImageFormat::from_path(path).ok() != Some(format) {
        return Err("Export filename does not match the selected format".into());
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            if file.metadata().map_err(|e| e.to_string())?.len() > 0 {
                let mut header = [0; 32];
                let count = file.read(&mut header).map_err(|e| e.to_string())?;
                if image::guess_format(&header[..count]).ok() != Some(format) {
                    return Err(
                        "Export destination is not the same image format; choose a new filename"
                            .into(),
                    );
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

fn write_raster(
    doc: &Document,
    path: &Path,
    format: image::ImageFormat,
    transparent: bool,
    cancel: &AtomicBool,
) -> Result<(), String> {
    doc.validate()?;
    raster_destination(path, format)?;
    let check = || crate::fill::check_cancel(Some(cancel));
    let opaque = matches!(format, image::ImageFormat::Jpeg | image::ImageFormat::Bmp);
    let rgba =
        doc.composite_bounds_checked([0, 0, doc.width, doc.height], transparent && !opaque, check)?;
    let image = image::RgbaImage::from_raw(doc.width as u32, doc.height as u32, rgba)
        .ok_or("Invalid export dimensions")?;
    crate::storage::atomic_stream(path, |file| {
        check()?;
        let image = image::DynamicImage::ImageRgba8(image);
        if format == image::ImageFormat::Jpeg {
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, 92)
                .encode_image(&image::DynamicImage::ImageRgb8(image.to_rgb8()))
                .map_err(|e| e.to_string())?;
        } else {
            image.write_to(file, format).map_err(|e| e.to_string())?;
        }
        check()?;
        raster_destination(path, format)
    })
}

impl Dudel {
    pub(crate) fn export_to(&mut self, path: PathBuf, transparent: bool) {
        if self.adjustment.is_some() {
            self.status = "Apply or cancel the adjustment before exporting".into();
            return;
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            self.status = "An image export is already running".into();
            return;
        }
        let format = match image::ImageFormat::from_path(&path) {
            Ok(
                format @ (image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::WebP
                | image::ImageFormat::Bmp
                | image::ImageFormat::Gif),
            ) => format,
            _ => {
                self.status = "Choose PNG, JPEG, WebP, BMP or GIF for image export".into();
                return;
            }
        };
        if let Err(error) = raster_destination(&path, format) {
            self.status = format!("Export failed: {error}");
            return;
        }
        let kind = if format == image::ImageFormat::Png {
            ExportKind::Png { transparent }
        } else {
            ExportKind::Raster {
                format,
                transparent,
            }
        };
        self.export_snapshot_or_queue(path, kind);
    }
    pub(crate) fn export_psd_to(&mut self, path: PathBuf, effects: crate::psd_export::Effects) {
        if let Err(error) = psd_destination(&path) {
            self.status = format!("Export failed: {error}");
            return;
        }
        self.export_snapshot_or_queue(path, ExportKind::Psd { effects });
    }
    /// Export now, or after the strokes ahead of it finish rendering.
    pub(crate) fn export_snapshot_or_queue(&mut self, path: PathBuf, kind: ExportKind) {
        if self.adjustment.is_some() {
            self.status = "Apply or cancel the adjustment before exporting".into();
            return;
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            self.status = "An export is already running".into();
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        if self.render_queue.busy() {
            match self.render_queue.snapshot() {
                Ok(ticket) => {
                    self.queued_export = Some(QueuedExport {
                        ticket,
                        path,
                        kind,
                        canceled: false,
                    });
                    self.status = "Export queued after preceding artwork".into();
                }
                Err(error) => self.status = format!("Export could not be queued: {error}"),
            }
        } else {
            self.start_export_snapshot(path, kind, self.doc.clone());
        }
    }

    pub(crate) fn start_export_snapshot(
        &mut self,
        path: PathBuf,
        kind: ExportKind,
        document: Document,
    ) {
        match kind {
            ExportKind::Raster {
                format,
                transparent,
            } => self.start_export_worker(
                path,
                document,
                kind,
                if matches!(format, image::ImageFormat::Jpeg | image::ImageFormat::Bmp) {
                    "Transparency flattened onto the canvas background.".into()
                } else {
                    String::new()
                },
                move |document, path, cancel| {
                    write_raster(document, path, format, transparent, cancel)
                },
            ),
            ExportKind::Package => self.start_package_snapshot(path, document),
            ExportKind::Psd { effects } => {
                let note = crate::psd_export::notes(&document, effects).join(" ");
                self.start_export_worker(
                    path,
                    document,
                    kind,
                    note,
                    move |document, path, cancel| {
                        psd_destination(path)?;
                        crate::psd_export::write(document, effects, path, cancel)
                    },
                );
            }
            ExportKind::Png { transparent } => self.start_export_worker(
                path,
                document,
                kind,
                String::new(),
                move |document, path, cancel| write_png(document, path, transparent, cancel),
            ),
        }
    }
    pub(crate) fn start_export_worker(
        &mut self,
        path: PathBuf,
        document: Document,
        kind: ExportKind,
        note: String,
        write: impl FnOnce(&Document, &Path, &AtomicBool) -> Result<(), String> + Send + 'static,
    ) {
        // A package also holds one PNG per layer while it encodes, and a PSD
        // its whole file beside the flattened image.
        let copies = if matches!(kind, ExportKind::Png { .. }) {
            1
        } else {
            2
        };
        let raster_bytes = document.width * document.height * 4 * copies;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = path.clone();
        let worker = SourceWorker::spawn(document, move |document| {
            write(&document, &worker_path, &worker_cancel)
        });
        let label = kind.label();
        self.export_job = Some(ExportJob {
            worker,
            path,
            cancel,
            raster_bytes,
            label,
            note,
        });
        self.status = format!("Exporting {label} in background…");
    }

    fn cancel_export(&mut self) {
        if let Some(export) = &mut self.queued_export {
            export.canceled = true;
        }
        if let Some(export) = &self.export_job {
            export.cancel.store(true, Ordering::Relaxed);
        }
        self.status = "Export cancellation requested".into();
    }

    pub(crate) fn poll_export(&mut self, ctx: &egui::Context) {
        if self
            .export_job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.export_job.take().unwrap();
            let label = job.label;
            self.status = match job.worker.join() {
                Ok(Ok(())) if job.note.is_empty() => {
                    format!("{label} exported: {}", job.path.display())
                }
                Ok(Ok(())) => format!("{label} exported: {}. {}", job.path.display(), job.note),
                Ok(Err(_)) if job.cancel.load(Ordering::Relaxed) => {
                    format!("{label} export canceled")
                }
                Ok(Err(error)) => format!("Export failed: {error}"),
                Err(_) => "Export worker failed; export was not confirmed".into(),
            };
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
            egui::TopBottomPanel::top("export progress").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(match &self.export_job {
                        Some(job) => format!("Exporting {}…", job.label),
                        None => "Preparing export…".into(),
                    });
                    if ui.button("Cancel export").clicked() {
                        self.cancel_export();
                    }
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn png_export_keeps_resolution_and_exact_pixels_on_reimport() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dpi.png");
        let mut doc = crate::document::Document::new(3, 2);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[23, 74, 156, 255]);
        for dpi in [1.0, 72.0, 300.0, 600.5, 12000.0] {
            doc.dpi = dpi;
            super::write_png(
                &doc,
                &path,
                true,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let loaded = crate::storage::import_png(&path).unwrap();
            assert!((loaded.dpi - dpi).abs() < 0.013, "{dpi}: {}", loaded.dpi);
            assert_eq!(
                loaded.layers[0].pixels.as_slice(),
                doc.layers[0].pixels.as_slice()
            );
        }
    }
    use super::*;

    #[test]
    fn common_formats_roundtrip_and_protect_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Document::new(3, 2);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[25, 90, 170, 128]);
        for (ext, format) in [
            ("jpg", image::ImageFormat::Jpeg),
            ("webp", image::ImageFormat::WebP),
            ("bmp", image::ImageFormat::Bmp),
            ("gif", image::ImageFormat::Gif),
        ] {
            let path = dir.path().join(format!("image.{ext}"));
            let cancel = AtomicBool::new(false);
            write_raster(&doc, &path, format, true, &cancel).unwrap();
            let decoded = image::open(&path).unwrap().to_rgba8();
            assert_eq!(decoded.dimensions(), (3, 2));
            if format == image::ImageFormat::WebP {
                assert_eq!(decoded.as_raw(), &doc.composite_transparent());
            }
            if matches!(format, image::ImageFormat::Jpeg | image::ImageFormat::Bmp) {
                assert!(decoded.pixels().all(|p| p[3] == 255));
            }
            let before = std::fs::read(&path).unwrap();
            cancel.store(true, Ordering::Relaxed);
            assert!(write_raster(&doc, &path, format, false, &cancel).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), before);
            std::fs::write(&path, b"DUDL source bytes").unwrap();
            cancel.store(false, Ordering::Relaxed);
            assert!(write_raster(&doc, &path, format, false, &cancel).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"DUDL source bytes");
        }
    }

    fn queued_taper() -> (Dudel, Document) {
        let (mut app, expected) = crate::edit_order_tests::pending_taper();
        let settings = app.stroke_settings.clone().unwrap();
        app.cancel_painted_stroke();
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 7.5,
            y: 9.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        (app, expected)
    }

    fn finish_export(app: &mut Dudel) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.export_job.is_some() || app.queued_export.is_some() {
            assert!(std::time::Instant::now() < deadline);
            let _ = ctx.run(Default::default(), |ctx| {
                app.pump_render_queue(ctx);
                app.poll_export(ctx);
            });
            std::thread::yield_now();
        }
    }

    #[test]
    fn queued_export_includes_finished_taper_and_excludes_later_artwork_without_saving_project() {
        for transparent in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("painting.png");
            let (mut app, expected) = queued_taper();
            app.path = Some(dir.path().join("source.dudel"));
            let project_path = app.path.clone();
            let target = app.doc.layers[app.doc.active].id;
            app.export_to(path.clone(), transparent);
            assert!(app.queued_export.is_some() && app.export_job.is_none());
            app.dispatch_document_edit(
                crate::document_edits::Edit::ClearTarget {
                    target,
                    whole_layer: true,
                },
                "Cleared",
                "Unchanged",
            );
            finish_export(&mut app);
            let image = image::open(&path).unwrap().into_rgba8();
            assert_eq!(
                image.dimensions(),
                (expected.width as u32, expected.height as u32)
            );
            assert_eq!(
                image.as_raw(),
                &if transparent {
                    expected.composite_transparent()
                } else {
                    expected.composite()
                }
            );
            assert_ne!(
                app.doc.layers[app.doc.active].pixels,
                expected.layers[expected.active].pixels
            );
            assert!(app.modified);
            assert_eq!(app.path, project_path);
            app.undo();
            assert_eq!(
                app.doc.layers[app.doc.active].pixels,
                expected.layers[expected.active].pixels
            );
            app.undo();
            assert!(!app.history.can_undo());
        }
    }

    #[test]
    fn export_preserves_saved_state_and_finishes_its_captured_document_after_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("original.png");
        let mut app = Dudel {
            doc: Document::new(7, 5),
            ..Default::default()
        };
        let before_revision = app.revision;
        let before_path = app.path.clone();
        app.export_to(path.clone(), true);
        assert!(!app.modified && !app.history.can_undo());
        assert_eq!(app.revision, before_revision);
        assert_eq!(app.path, before_path);
        app.apply_action(crate::document_actions::Action::New(9, 3, 300.0));
        finish_export(&mut app);
        assert_eq!(image::open(path).unwrap().into_rgba8().dimensions(), (7, 5));
        assert_eq!((app.doc.width, app.doc.height), (9, 3));
        assert!(!app.modified && app.path.is_none());
    }

    #[test]
    fn cancellation_before_and_after_encoding_preserves_destination_and_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.png");
        let doc = Document::new(3, 2);
        let cancel = AtomicBool::new(false);
        write_png(&doc, &path, true, &cancel).unwrap();
        let before = std::fs::read(&path).unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(write_png(&doc, &path, false, &cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
        assert!(
            write_png_with_hook(&doc, &path, false, &cancel, || cancel
                .store(true, Ordering::Relaxed))
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_export_and_wrong_destinations_preserve_project_and_existing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            doc: Document::new(3, 2),
            ..Default::default()
        };
        for name in ["source.dudel", "source.clip", "renamed.png"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"DUDL source bytes").unwrap();
            app.export_to(path.clone(), false);
            assert!(app.export_job.is_none() && app.queued_export.is_none());
            assert_eq!(std::fs::read(path).unwrap(), b"DUDL source bytes");
        }
        app.export_to(dir.path().join("missing").join("image.png"), false);
        finish_export(&mut app);
        assert!(app.status.starts_with("Export failed:"));
        assert!(!app.modified && app.path.is_none() && !app.history.can_undo());
    }

    #[test]
    fn queued_cancellation_consumes_its_ticket_without_writing_and_close_waits_for_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canceled.png");
        let (mut app, _) = queued_taper();
        app.export_to(path.clone(), true);
        app.cancel_export();
        finish_export(&mut app);
        assert!(!path.exists());
        assert!(!app.status.contains("Unrecognized"));

        // PSD export goes through the same queue and never replaces a file
        // that is not a PSD.
        let psd = dir.path().join("painting.psd");
        let (mut app, expected) = queued_taper();
        std::fs::write(&psd, b"not a psd").unwrap();
        app.export_psd_to(psd.clone(), crate::psd_export::Effects::Editable);
        assert!(app.status.contains("not a PSD"), "{}", app.status);
        std::fs::remove_file(&psd).unwrap();
        app.export_psd_to(psd.clone(), crate::psd_export::Effects::Editable);
        assert!(app.queued_export.is_some());
        finish_export(&mut app);
        assert!(app.status.starts_with("PSD exported"), "{}", app.status);
        let back = crate::psd_import::import(&psd, &|| false).unwrap().document;
        assert_eq!(back.composite(), expected.composite());

        // A controlled live worker establishes that close waits rather than
        // detaching an in-flight export at application shutdown.
        let (send, receive) = std::sync::mpsc::channel();
        app.export_job = Some(ExportJob {
            worker: SourceWorker::spawn(app.doc.clone(), move |_| {
                receive.recv().unwrap();
                Ok(())
            }),
            path,
            cancel: Arc::new(AtomicBool::new(false)),
            raster_bytes: 0,
            label: "PNG",
            note: String::new(),
        });
        app.close_ready = true;
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| app.close_viewport_if_ready(ctx));
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!commands.contains(&egui::ViewportCommand::Close));
        send.send(()).unwrap();
        finish_export(&mut app);
        let output = ctx.run(Default::default(), |ctx| app.close_viewport_if_ready(ctx));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::Close)
        );
    }
}
