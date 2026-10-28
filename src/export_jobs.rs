//! Ordered, snapshot-based PNG export. Export never changes project save state.
use crate::{Dudel, document::Document, egui, snapshot_memory::SourceWorker};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct QueuedExport {
    pub ticket: u64,
    pub path: PathBuf,
    pub transparent: bool,
    /// A .zip project package rather than a PNG.
    pub package: bool,
    pub canceled: bool,
}
pub(crate) struct ExportJob {
    pub worker: SourceWorker<Result<(), String>>,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    raster_bytes: usize,
    label: &'static str,
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
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("png"))
    {
        return Err("Choose a .png filename for image export".into());
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            // A renamed project/source must never be destroyed by image export.
            if file.metadata().map_err(|e| e.to_string())?.len() != 0 {
                let mut signature = [0; 8];
                if file.read_exact(&mut signature).is_err() || signature != *b"\x89PNG\r\n\x1a\n" {
                    return Err("Export destination is not a PNG; choose a new filename".into());
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
    use image::ImageEncoder;
    doc.validate()?;
    destination(path)?;
    let check = || crate::fill::check_cancel(Some(cancel));
    let rgba = doc.composite_bounds_checked([0, 0, doc.width, doc.height], transparent, check)?;
    crate::storage::atomic_stream(path, |file| {
        check()?;
        image::codecs::png::PngEncoder::new(file)
            .write_image(
                &rgba,
                doc.width as u32,
                doc.height as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        // PNG encoding itself is not interruptible; cancel before publication.
        before_publish();
        check()?;
        destination(path)
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
        if let Err(error) = destination(&path) {
            self.status = format!("Export failed: {error}");
            return;
        }
        self.export_snapshot_or_queue(path, transparent, false);
    }
    /// Export now, or after the strokes ahead of it finish rendering.
    pub(crate) fn export_snapshot_or_queue(
        &mut self,
        path: PathBuf,
        transparent: bool,
        package: bool,
    ) {
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
                        transparent,
                        package,
                        canceled: false,
                    });
                    self.status = "Export queued after preceding artwork".into();
                }
                Err(error) => self.status = format!("Export could not be queued: {error}"),
            }
        } else if package {
            self.start_package_snapshot(path, self.doc.clone());
        } else {
            self.start_export_snapshot(path, transparent, self.doc.clone());
        }
    }

    pub(crate) fn start_export_snapshot(
        &mut self,
        path: PathBuf,
        transparent: bool,
        document: Document,
    ) {
        self.start_export_worker(path, document, false, move |document, path, cancel| {
            write_png(document, path, transparent, cancel)
        });
    }
    pub(crate) fn start_export_worker(
        &mut self,
        path: PathBuf,
        document: Document,
        package: bool,
        write: impl FnOnce(&Document, &Path, &AtomicBool) -> Result<(), String> + Send + 'static,
    ) {
        // A package also holds one PNG per layer while it encodes.
        let raster_bytes = document.width * document.height * 4 * if package { 2 } else { 1 };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = path.clone();
        let worker = SourceWorker::spawn(document, move |document| {
            write(&document, &worker_path, &worker_cancel)
        });
        let label = if package { "Project package" } else { "PNG" };
        self.export_job = Some(ExportJob {
            worker,
            path,
            cancel,
            raster_bytes,
            label,
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
                Ok(Ok(())) => format!("{label} exported: {}", job.path.display()),
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
    use super::*;

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
        app.apply_action(crate::document_actions::Action::New(9, 3));
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
