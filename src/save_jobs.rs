use crate::{Dudel, egui, preferences, storage};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
};

pub struct IndexedPending {
    receiver: mpsc::Receiver<crate::indexed_commit::Session>,
    cancel: Arc<AtomicBool>,
    descriptors: Arc<AtomicUsize>,
    epoch: u64,
}
impl IndexedPending {
    pub(crate) fn descriptor_bytes(&self) -> usize {
        self.descriptors.load(Ordering::Relaxed)
    }
    pub(crate) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn indexed_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("didx"))
}

pub struct SaveJob {
    pub worker: crate::snapshot_memory::SourceWorker<Result<(), String>>,
    pub path: PathBuf,
    pub revision: u64,
    pub cancel: Arc<AtomicBool>,
}
pub struct QueuedSave {
    pub ticket: u64,
    pub path: PathBuf,
    pub canceled: bool,
}
impl Dudel {
    pub(crate) fn retire_indexed_session(&mut self) {
        self.indexed_epoch = self.indexed_epoch.wrapping_add(1);
        self.indexed_session = None;
        self.saving_guides = None;
        self.reference_layers.finish_save(false);
        self.saving_content_revision = None;
        if let Some(pending) = &self.indexed_pending {
            pending.cancel();
        }
    }
    pub(crate) fn save(&mut self, save_as: bool) {
        self.save_before(save_as, None);
    }
    /// Save, then run `action` once the save finishes. A project without a
    /// path asks for one first; `action` is dropped if that is cancelled.
    pub(crate) fn save_before(
        &mut self,
        save_as: bool,
        action: Option<crate::document_actions::Action>,
    ) {
        if self.adjustment.is_some() {
            self.status = "Apply or cancel the adjustment before saving".into();
            return;
        }
        if self.save_job.is_some() || self.queued_save.is_some() {
            self.status = "A project save is already running".into();
            return;
        }
        match self.path.clone().filter(|_| !save_as) {
            Some(path) => self.save_to(path, action),
            None => {
                let dialog = self.dialog_in(
                    crate::locations::Kind::Projects,
                    rfd::AsyncFileDialog::new().add_filter("Dudel project", &["dudel"]),
                );
                let dialog = if self.experimental_indexed_storage {
                    dialog
                        .add_filter("Experimental indexed project", &["didx"])
                        .set_file_name("Untitled.didx")
                } else {
                    dialog.set_file_name("Untitled.dudel")
                };
                self.ask_save_file(dialog, move |app, path| app.save_to(path, action));
            }
        }
    }
    fn save_to(&mut self, path: PathBuf, action: Option<crate::document_actions::Action>) {
        // A different tab owns its on-disk baseline, even when it has no edits.
        // Reject before settling gestures or attaching a destructive continuation.
        if let Some(index) = self.tab_with_path(&path)
            && index != self.tabs.active
        {
            self.status = format!(
                "Cannot save: this file belongs to another open tab ({}). Choose a different filename or close that tab first.",
                self.tab_title(index).0
            );
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        if self.render_queue.busy() {
            match self.render_queue.snapshot() {
                Ok(ticket) => {
                    self.queued_save = Some(QueuedSave {
                        ticket,
                        path,
                        canceled: false,
                    });
                    self.status = "Save queued after preceding artwork".into();
                }
                Err(error) => self.status = format!("Save could not be queued: {error}"),
            }
        } else {
            self.start_save_snapshot(path, self.doc.clone());
        }
        if let Some(action) = action
            && (self.save_job.is_some() || self.queued_save.is_some())
        {
            self.pending_action = Some(action);
        }
    }
    pub(crate) fn start_save_snapshot(
        &mut self,
        path: PathBuf,
        mut document: crate::document::Document,
    ) {
        if self.save_job.is_some() {
            self.status = "A project save is already running".into();
            return;
        }
        self.track_guide_changes();
        // Guides are app state; only the captured copy travels to the worker.
        let guides = self.guides_record(&document);
        document.guides = Some(Box::new(guides.clone()));
        if indexed_path(&path) && !self.experimental_indexed_storage {
            self.status =
                "Experimental indexed storage is disabled; save a new .dudel project".into();
            return;
        }
        if indexed_path(&path) && !self.reference_layers.layers.is_empty() {
            self.status =
                "Experimental indexed projects cannot hold reference layers; save a .dudel project"
                    .into();
            return;
        }
        self.reference_layers.saved_positions(&self.doc);
        document.references = self.reference_layers.begin_save();
        self.saving_guides = Some(guides);
        self.saving_content_revision = Some(self.content_revision);
        let worker_path = path.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = if indexed_path(&path) {
            let session = self.indexed_session.take();
            let (sender, receiver) = mpsc::channel();
            let descriptors = Arc::new(AtomicUsize::new(
                session.as_ref().map_or(0, |s| s.descriptor_bytes()),
            ));
            let worker_descriptors = descriptors.clone();
            self.indexed_pending = Some(IndexedPending {
                receiver,
                cancel: cancel.clone(),
                descriptors,
                epoch: self.indexed_epoch,
            });
            crate::snapshot_memory::SourceWorker::spawn(document, move |document| {
                let session = crate::indexed_commit::Session::save(
                    session,
                    &worker_path,
                    &document,
                    &|| worker_cancel.load(Ordering::Relaxed),
                )?;
                worker_descriptors.store(session.descriptor_bytes(), Ordering::Relaxed);
                sender.send(session).map_err(|_| {
                    "Indexed save completed but its editor session was retired".to_string()
                })
            })
        } else {
            // Saving a normal copy releases an idle experimental writer first.
            self.indexed_session = None;
            crate::snapshot_memory::SourceWorker::spawn(document, move |document| {
                storage::save_project_cancelable(&worker_path, &document, &worker_cancel)
            })
        };
        self.save_job = Some(SaveJob {
            worker,
            path,
            revision: self.revision,
            cancel,
        });
        self.status = "Saving project in background…".into();
    }
    fn commit_save(&mut self, path: PathBuf, revision: u64, result: Result<(), String>) {
        self.track_guide_changes();
        let captured_guides = self.saving_guides.take();
        self.reference_layers.finish_save(result.is_ok());
        let captured_content = self
            .saving_content_revision
            .take()
            .or_else(|| (revision == self.revision).then_some(self.content_revision));
        let continuation = self.pending_action.take();
        match result {
            Ok(()) => {
                self.saved_content_revision = captured_content;
                if let Some(guides) = captured_guides {
                    self.saved_guides = guides;
                }
                preferences::remember(&mut self.recent, path.clone());
                self.path = Some(path);
                if captured_content == Some(self.content_revision)
                    && !self.render_queue.changes_pending()
                    && self.geometry_preview.is_none()
                    && !self.guides_changed()
                {
                    self.modified = false;
                    self.clear_recovery();
                    self.status = "Project saved".into();
                    if let Some(action) = continuation {
                        self.apply_action(action);
                    }
                } else {
                    self.status = "Saved earlier revision · newer edits remain unsaved".into();
                    if continuation.is_some() {
                        self.status = "Saved earlier revision · pending canvas action canceled because newer edits are unsaved".into();
                        if self.new_canvas
                            && matches!(
                                continuation,
                                Some(crate::document_actions::Action::New(..))
                            )
                        {
                            self.new_canvas_form.feedback=Some("The drawing changed during saving. Review your latest changes before creating a new canvas.".into());
                        }
                    }
                }
            }
            Err(error) if error == storage::SAVE_CANCELED => {
                self.status = "Project save canceled; artwork remains open".into();
                if self.new_canvas {
                    self.new_canvas_form.feedback =
                        Some("Save canceled. The original drawing remains open.".into());
                }
            }
            Err(error) => {
                if self.new_canvas {
                    self.new_canvas_form.feedback =
                        Some(format!("The original drawing could not be saved: {error}"));
                }
                self.status = format!("Save failed: {error}");
            }
        }
    }
    pub(crate) fn cancel_save(&mut self) {
        if let Some(save) = &mut self.queued_save {
            save.canceled = true;
        }
        if let Some(save) = &self.save_job {
            save.cancel.store(true, Ordering::Relaxed);
        }
        self.pending_action = None;
        self.status = "Project save cancellation requested".into();
    }
    pub(crate) fn poll_save(&mut self, ctx: &egui::Context) {
        if self
            .save_job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.save_job.take().unwrap();
            let result = job.worker.join().unwrap_or_else(|_| {
                Err("Save worker failed; saved state was not confirmed".into())
            });
            let mut result = result;
            if let Some(pending) = self.indexed_pending.take() {
                if pending.epoch != self.indexed_epoch {
                    self.status = "Indexed save finished for a retired canvas; current artwork remains unsaved".into();
                    self.pending_action = None;
                    return;
                }
                if result.is_ok() {
                    match pending.receiver.try_recv() {
                        Ok(session) => self.indexed_session = Some(session),
                        Err(_) => {
                            result =
                                Err("Indexed save worker did not return a confirmed session".into())
                        }
                    }
                }
            }
            self.commit_save(job.path, job.revision, result);
        }
        if self.save_job.is_some() || self.queued_save.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    /// Save progress and its cancel controls, drawn inside the status row so
    /// the bottom bar and canvas keep their size while a save runs.
    /// Returns false when no save is running or queued.
    pub(crate) fn save_progress_ui(&mut self, ui: &mut egui::Ui) -> bool {
        if self.save_job.is_none() && self.queued_save.is_none() {
            return false;
        }
        // Fixed size: the theme's touch-sized interact height would grow the status row.
        ui.add(egui::Spinner::new().size(16.0));
        let canceling = self.queued_save.as_ref().is_some_and(|s| s.canceled)
            || self
                .save_job
                .as_ref()
                .is_some_and(|s| s.cancel.load(Ordering::Relaxed));
        ui.label(if canceling {
            "Canceling project save…"
        } else if self.queued_save.is_some() {
            "Waiting for preceding artwork…"
        } else {
            "Saving project… You can keep drawing."
        });
        if ui
            .add_enabled(!canceling, egui::Button::new("Cancel save").small())
            .clicked()
        {
            self.cancel_save();
        }
        if self.pending_action.is_some()
            && ui
                .add(egui::Button::new("Cancel pending canvas action").small())
                .clicked()
        {
            self.pending_action = None;
        }
        true
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn multi_tab_save_keeps_own_destination_and_allows_a_new_copy() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.dudel");
        let second = directory.path().join("second.dudel");
        let copy = directory.path().join("copy.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(1, 1),
            path: Some(first.clone()),
            ..Default::default()
        };
        storage::save_project(&first, &app.doc).unwrap();
        let original = std::fs::read(&first).unwrap();
        assert!(app.prepare_tab_for_open());
        app.doc = crate::document::Document::new(1, 1);
        app.doc.layers[0].pixels = vec![255, 0, 0, 255].into();
        app.changed();
        app.path = Some(second.clone());
        for path in [second, copy] {
            app.save_to(path.clone(), None);
            assert!(app.save_job.is_some());
            finish_background(&mut app);
            assert!(!app.modified);
            assert_eq!(app.path.as_ref(), Some(&path));
            assert_eq!(
                storage::load_project(&path).unwrap().layers[0].pixels,
                app.doc.layers[0].pixels
            );
            assert_eq!(std::fs::read(&first).unwrap(), original);
        }
    }

    #[test]
    fn save_as_cannot_overwrite_another_open_tab_or_run_its_close_action() {
        for parked_modified in [false, true] {
            for alias in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("first.dudel");
                let mut app = Dudel {
                    doc: crate::document::Document::new(1, 1),
                    path: Some(path.clone()),
                    modified: parked_modified,
                    ..Default::default()
                };
                app.doc.layers[0].pixels = vec![10, 30, 200, 128].into();
                storage::save_project(&path, &app.doc).unwrap();
                let original = std::fs::read(&path).unwrap();
                assert!(app.prepare_tab_for_open());
                app.doc = crate::document::Document::new(1, 1);
                app.doc.layers[0].pixels = vec![255, 0, 0, 255].into();
                app.changed();
                let current_path = directory.path().join("second.dudel");
                app.path = Some(current_path.clone());
                let revision = app.revision;
                let destination = if alias {
                    path.canonicalize().unwrap()
                } else {
                    path.clone()
                };
                app.save_to(destination, Some(crate::document_actions::Action::Close));
                assert!(app.save_job.is_none() && app.queued_save.is_none());
                assert!(app.pending_action.is_none() && !app.close_ready);
                assert!(app.modified && app.revision == revision);
                assert_eq!(app.path, Some(current_path));
                assert_eq!(app.doc.layers[0].pixels, vec![255, 0, 0, 255]);
                assert_eq!(std::fs::read(&path).unwrap(), original);
                assert!(app.status.contains("another open tab"));
                app.request_tab(0);
                assert_eq!(app.tabs.active, 0);
                assert_eq!(app.modified, parked_modified);
                assert_eq!(app.doc.layers[0].pixels, vec![10, 30, 200, 128]);
            }
        }
    }

    #[test]
    fn completed_save_becomes_clean_after_a_later_export_fence_settles() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("saved.dudel");
        let png = directory.path().join("export.png");
        let mut app = Dudel {
            doc: crate::document::Document::new(17, 17),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 255]);
        app.changed();
        app.start_save_snapshot(project.clone(), app.doc.clone());
        let ticket = app.render_queue.snapshot().unwrap();
        app.queued_export = Some(crate::export_jobs::QueuedExport {
            ticket,
            path: png.clone(),
            kind: crate::export_jobs::ExportKind::Png { transparent: true },
            canceled: false,
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.save_job.as_ref().unwrap().worker.is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.poll_save(ctx));
        // The save is confirmed as soon as it finishes, even with the export
        // fence still queued behind it.
        assert!(project.is_file() && app.render_queue.busy());
        crate::workflow_tests::finish_jobs(&mut app);
        assert!(
            !app.modified,
            "read-only export did not change the saved artwork"
        );
        assert_eq!(
            storage::load_project(&project).unwrap().layers[0].pixels,
            app.doc.layers[0].pixels
        );
        assert_eq!(
            image::open(png).unwrap().into_rgba8().as_raw(),
            &app.doc.composite_transparent()
        );
    }

    fn click_cancel_save(app: &mut Dudel, ctx: &egui::Context) {
        fn text(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == "Cancel save" => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(text),
                _ => None,
            }
        }
        let frame = |app: &mut Dudel, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 240.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.poll_save(ctx);
                    app.workspace(ctx);
                },
            )
        };
        frame(app, vec![]);
        let output = frame(app, vec![]);
        let at = output
            .shapes
            .iter()
            .find_map(|shape| text(&shape.shape))
            .expect("Visible Cancel save control");
        for pressed in [true, false] {
            frame(
                app,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    #[test]
    fn ordinary_save_cancel_button_preserves_old_file_recovery_and_pending_canvas() {
        let directory = tempfile::tempdir().unwrap();
        let original_path = directory.path().join("original.dudel");
        let destination = directory.path().join("save-as.dudel");
        let recovery = directory.path().join("recovery.gz");
        let mut app = Dudel {
            doc: crate::document::Document::new(33, 35),
            path: Some(original_path.clone()),
            recovery_path: Some(recovery.clone()),
            modified: true,
            ..Default::default()
        };
        storage::save_project(&destination, &app.doc).unwrap();
        let old = std::fs::read(&destination).unwrap();
        std::fs::write(&recovery, b"retained recovery fixture").unwrap();
        app.history.checkpoint(&app.doc);
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 255]);
        app.changed();
        let source = app.doc.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = destination.clone();
        let (release, wait) = mpsc::channel();
        app.save_job = Some(SaveJob {
            worker: crate::snapshot_memory::SourceWorker::spawn(source.clone(), move |doc| {
                wait.recv_timeout(std::time::Duration::from_secs(10))
                    .map_err(|e| e.to_string())?;
                storage::save_project_cancelable(&worker_path, &doc, &worker_cancel)
            }),
            path: destination.clone(),
            revision: app.revision,
            cancel: cancel.clone(),
        });
        app.pending_action = Some(crate::document_actions::Action::New(2, 2, 300.0));
        let ctx = egui::Context::default();
        click_cancel_save(&mut app, &ctx);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(app.pending_action.is_none());
        release.send(()).unwrap();
        finish_background(&mut app);
        assert_eq!(app.path, Some(original_path));
        assert!(app.modified && recovery.exists());
        assert!(app.status.contains("save canceled"), "{}", app.status);
        assert_eq!(std::fs::read(&destination).unwrap(), old);
        assert!(crate::document::same_snapshot(&app.doc, &source));
        assert!(app.history.undo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn cancel_after_publication_confirms_save_without_running_pending_canvas_action() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("published.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(33, 35),
            modified: true,
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 128]);
        let snapshot = app.doc.clone();
        let worker_path = path.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (published, observed) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        app.save_job = Some(SaveJob {
            worker: crate::snapshot_memory::SourceWorker::spawn(snapshot, move |doc| {
                storage::save_project_cancelable(&worker_path, &doc, &worker_cancel)?;
                published.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(10))
                    .map_err(|e| e.to_string())
            }),
            path: path.clone(),
            revision: app.revision,
            cancel,
        });
        app.pending_action = Some(crate::document_actions::Action::New(2, 2, 300.0));
        observed
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        app.cancel_save();
        release.send(()).unwrap();
        finish_background(&mut app);
        assert!(!app.modified);
        assert_eq!(app.path, Some(path.clone()));
        assert_eq!((app.doc.width, app.doc.height), (33, 35));
        assert_eq!(app.status, "Project saved");
        assert_eq!(
            storage::load_project(&path).unwrap().layers[0].pixels,
            app.doc.layers[0].pixels
        );
    }

    #[test]
    fn queued_save_cancel_consumes_snapshot_ticket_without_losing_the_stroke() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(33, 35),
            path: Some(path.clone()),
            ..Default::default()
        };
        storage::save_project(&path, &app.doc).unwrap();
        let old = std::fs::read(&path).unwrap();
        assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        }));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 16.5,
            y: 17.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.save(false);
        assert!(app.queued_save.is_some());
        click_cancel_save(&mut app, &egui::Context::default());
        assert!(app.queued_save.as_ref().unwrap().canceled);
        app.flush_render_queue();
        assert!(app.queued_save.is_none() && app.save_job.is_none());
        assert!(app.modified && app.doc.layers[0].pixels.iter().any(|byte| *byte != 0));
        assert_eq!(std::fs::read(&path).unwrap(), old);
        assert!(app.history.undo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        app.save(false);
        finish_background(&mut app);
        assert!(
            !app.modified,
            "A canceled ticket must not block following saves"
        );
    }

    #[test]
    fn save_finishing_before_a_queued_export_snapshot_leaves_the_project_saved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(33, 35),
            path: Some(path.clone()),
            ..Default::default()
        };
        assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        }));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 16.5,
            y: 17.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.save(false);
        app.export_to(directory.path().join("export.png"), true);
        let ctx = egui::Context::default();
        while app.queued_save.is_some() {
            app.pump_render_queue(&ctx);
        }
        assert!(app.render_queue.busy() && !app.render_queue.changes_pending());
        let job = app.save_job.take().unwrap();
        app.commit_save(job.path, job.revision, job.worker.join().unwrap());
        assert!(!app.modified, "{}", app.status);
        crate::workflow_tests::finish_jobs(&mut app);
        assert!(!app.modified);
    }

    #[test]
    fn panicking_save_worker_keeps_artwork_recovery_and_unsaved_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.dudel");
        let recovery = directory.path().join("recovery.gz");
        std::fs::write(&recovery, b"retained recovery fixture").unwrap();
        let mut app = Dudel {
            doc: crate::document::Document::new(33, 35),
            path: Some(path.clone()),
            recovery_path: Some(recovery.clone()),
            modified: true,
            ..Default::default()
        };
        let snapshot = app.doc.clone();
        app.save_job = Some(SaveJob {
            worker: crate::snapshot_memory::SourceWorker::spawn(snapshot.clone(), |_| {
                panic!("Injected save-worker panic")
            }),
            path: directory.path().join("failed.dudel"),
            revision: app.revision,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        app.pending_action = Some(crate::document_actions::Action::New(2, 2, 300.0));
        finish_background(&mut app);
        assert!(app.modified && recovery.exists());
        assert_eq!(app.path, Some(path));
        assert!(app.pending_action.is_none() && app.recent.is_empty());
        assert!(crate::document::same_snapshot(&snapshot, &app.doc));
        assert!(app.status.contains("Save worker failed"));
    }

    fn finish_background(app: &mut Dudel) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.save_job.is_some() {
            assert!(std::time::Instant::now() < deadline);
            let _ = ctx.run(Default::default(), |ctx| app.poll_save(ctx));
            std::thread::yield_now();
        }
    }
    #[test]
    fn guide_save_baseline_advances_only_on_success_and_newer_guides_stay_unsaved() {
        for indexed in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let suffix = if indexed { "didx" } else { "dudel" };
            let failed = dir.path().join("missing").join(format!("failed.{suffix}"));
            let path = dir.path().join(format!("saved.{suffix}"));
            let recovery = dir.path().join("recovery.gz");
            std::fs::write(&recovery, b"keep recovery").unwrap();
            let mut app = Dudel {
                experimental_indexed_storage: indexed,
                doc: crate::document::Document::new(32, 32),
                recovery_path: Some(recovery.clone()),
                ..Default::default()
            };
            app.grid.visible = true;
            app.start_save_snapshot(failed, app.doc.clone());
            finish_background(&mut app);
            assert!(app.modified && app.guides_changed() && recovery.exists());
            assert_eq!(app.saved_guides, crate::guides::Guides::default());
            assert!(app.saving_guides.is_none() && app.path.is_none());
            app.start_save_snapshot(path.clone(), app.doc.clone());
            let captured_revision = app.save_job.as_ref().unwrap().revision;
            app.grid.spacing = 64;
            finish_background(&mut app);
            assert_ne!(app.revision, captured_revision);
            assert!(app.modified && app.guides_changed() && recovery.exists());
            assert!(app.saved_guides.grid.visible);
            assert_eq!(app.saved_guides.grid.spacing, 100);
            app.indexed_session = None;
            let saved = if indexed {
                crate::indexed_commit::load_document(&path, &|| false).unwrap()
            } else {
                storage::load_project(&path).unwrap()
            };
            assert_eq!(saved.guides.as_ref().unwrap().grid.spacing, 100);
            app.start_save_snapshot(path, app.doc.clone());
            finish_background(&mut app);
            assert!(!app.modified && !app.guides_changed() && !recovery.exists());
            assert_eq!(app.saved_guides.grid.spacing, 64);
            assert!(
                !app.history.can_undo(),
                "guide persistence never adds artwork undo"
            );
        }
    }
    #[test]
    fn experimental_editor_save_keeps_newer_edits_unsaved_then_saves_and_reopens_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("painting.didx");
        let recovery = dir.path().join("recovery.gz");
        std::fs::write(&recovery, b"preserve until current revision saved").unwrap();
        let mut app = Dudel {
            experimental_indexed_storage: true,
            doc: crate::indexed_document::fixture(),
            modified: true,
            revision: 15,
            recovery_path: Some(recovery.clone()),
            ..Default::default()
        };
        let original = app.doc.clone();
        app.start_save_snapshot(path.clone(), app.doc.clone());
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 19, 29, 39]);
        app.metadata_changed();
        finish_background(&mut app);
        assert!(app.modified && recovery.exists());
        assert_eq!(app.path.as_ref(), Some(&path));
        assert!(app.indexed_session.is_some());
        // Loading the active .didx releases the idle writer before acquiring its
        // shared reader, and the ordinary load revision fence still applies.
        app.indexed_session = None;
        let mut saved = crate::indexed_commit::load_document(&path, &|| false).unwrap();
        // The saved copy carries guides; the live document never does.
        assert_eq!(
            saved.guides.take().as_deref(),
            Some(&app.guides_record(&original))
        );
        assert!(crate::indexed_document::equivalent(&original, &saved).unwrap());
        app.start_save_snapshot(path.clone(), app.doc.clone());
        finish_background(&mut app);
        assert!(!app.modified && !recovery.exists());
        assert!(app.indexed_session.as_ref().unwrap().descriptor_bytes() > 0);
        let current = app.doc.clone();
        app.load_document(path.clone(), false);
        assert!(app.indexed_session.is_none());
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.load_job.is_some() {
            assert!(std::time::Instant::now() < deadline);
            let _ = ctx.run(Default::default(), |ctx| app.poll_load(ctx));
            std::thread::yield_now();
        }
        assert_eq!(app.path.as_ref(), Some(&path));
        assert!(crate::indexed_document::equivalent(&current, &app.doc).unwrap());
    }
    #[test]
    fn indexed_completion_cannot_rebind_a_replaced_canvas_and_ordinary_copy_stays_v4() {
        let dir = tempfile::tempdir().unwrap();
        let indexed = dir.path().join("retired.didx");
        let native = dir.path().join("copy.dudel");
        let mut app = Dudel {
            experimental_indexed_storage: true,
            doc: crate::indexed_document::fixture(),
            modified: true,
            ..Default::default()
        };
        app.start_save_snapshot(indexed, app.doc.clone());
        app.apply_action(crate::document_actions::Action::New(17, 19, 300.0));
        app.modified = true;
        finish_background(&mut app);
        assert!(app.path.is_none() && app.modified && app.indexed_session.is_none());
        assert_eq!((app.doc.width, app.doc.height), (17, 19));
        app.start_save_snapshot(native.clone(), app.doc.clone());
        finish_background(&mut app);
        assert!(!app.modified && app.indexed_session.is_none());
        let bytes = std::fs::read(&native).unwrap();
        assert!(bytes.starts_with(crate::tile_project::MAGIC));
    }
    #[test]
    fn indexed_opt_in_and_foreign_destination_failures_preserve_recovery_and_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("protected.didx");
        let recovery = dir.path().join("recover.gz");
        std::fs::write(&path, b"CSFCHUNK original").unwrap();
        std::fs::write(&recovery, b"recovery").unwrap();
        let mut app = Dudel {
            modified: true,
            recovery_path: Some(recovery.clone()),
            ..Default::default()
        };
        app.start_save_snapshot(path.clone(), app.doc.clone());
        assert!(app.save_job.is_none());
        app.load_document(path.clone(), false);
        assert!(app.load_job.is_none());
        app.experimental_indexed_storage = true;
        app.start_save_snapshot(path.clone(), app.doc.clone());
        finish_background(&mut app);
        assert!(app.modified && app.path.is_none() && app.indexed_session.is_none());
        assert!(recovery.exists());
        assert_eq!(std::fs::read(&path).unwrap(), b"CSFCHUNK original");
    }
    #[test]
    fn indexed_save_queue_captures_preceding_metadata_and_excludes_following_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ordered.didx");
        let mut app = Dudel {
            experimental_indexed_storage: true,
            doc: crate::document::Document::new(32, 32),
            path: Some(path.clone()),
            ..Default::default()
        };
        let id = app.doc.layers[0].id;
        app.render_queue
            .edit(crate::document_edits::Edit::SelectLayer(0))
            .unwrap();
        app.set_target_opacity(id, 0.7);
        app.flush_render_queue();
        app.save(false);
        assert!(app.queued_save.is_some());
        app.set_target_opacity(id, 0.5);
        app.finish_opacity_edit();
        app.flush_render_queue();
        finish_background(&mut app);
        assert!(app.modified && app.doc.layers[0].opacity == 0.5);
        app.indexed_session = None;
        assert_eq!(
            crate::indexed_commit::load_document(&path, &|| false)
                .unwrap()
                .layers[0]
                .opacity,
            0.7
        );
    }
    #[test]
    fn save_closes_queued_opacity_and_excludes_following_preview() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued-opacity.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            path: Some(path.clone()),
            ..Default::default()
        };
        let target = app.doc.layers[0].id;
        app.render_queue
            .edit(crate::document_edits::Edit::SelectLayer(0))
            .unwrap();
        app.set_target_opacity(target, 0.7);
        app.flush_render_queue();
        assert!(app.history.has_pending_stroke());
        app.save(false);
        assert!(app.queued_save.is_some() && app.save_job.is_none());
        assert!(app.opacity_edit.is_none());
        assert!(
            app.history.has_pending_stroke(),
            "save request must not drain the opacity end"
        );
        app.set_target_opacity(target, 0.5);
        app.finish_opacity_edit();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].opacity, 0.5);
        assert!(!app.history.has_pending_stroke());
        let saved_revision = app.save_job.as_ref().unwrap().revision;
        assert_ne!(saved_revision, app.revision);
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.save_job.is_some() {
            assert!(std::time::Instant::now() < deadline);
            let _ = ctx.run(Default::default(), |ctx| app.poll_save(ctx));
            std::thread::yield_now();
        }
        assert!(app.modified);
        assert_eq!(
            crate::storage::load_project(&path).unwrap().layers[0].opacity,
            0.7
        );
        app.undo();
        assert_eq!(app.doc.layers[0].opacity, 0.7);
        app.undo();
        assert_eq!(app.doc.layers[0].opacity, 1.0);
        assert!(!app.history.can_undo());
    }
    use super::*;
    #[test]
    fn queued_save_observes_undo_before_it_and_excludes_following_redo() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("undo-boundary.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            path: Some(path.clone()),
            ..Default::default()
        };
        let original = app.doc.layers[0].pixels.clone();
        assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        }));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 16.5,
            y: 16.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.undo();
        app.save(false);
        app.redo();
        let ctx = egui::Context::default();
        while app.queued_save.is_some() {
            app.pump_render_queue(&ctx);
        }
        assert_eq!(app.doc.layers[0].pixels, original);
        assert!(app.render_queue.busy());
        let job = app.save_job.take().unwrap();
        app.flush_render_queue();
        assert_ne!(app.doc.layers[0].pixels, original);
        assert_ne!(app.revision, job.revision);
        app.commit_save(job.path, job.revision, job.worker.join().unwrap());
        assert!(app.modified);
        assert_eq!(
            storage::load_project(&path).unwrap().layers[0].pixels,
            original
        );
    }
    #[test]
    fn queued_save_captures_only_preceding_strokes_and_preserves_newer_artwork() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ordered.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(64, 64),
            path: Some(path.clone()),
            ..Default::default()
        };
        let settings = |color| crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 19.0,
                pressure_size: false,
                taper_out: 20.0,
                ..Default::default()
            },
            color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let sample = |x| crate::stroke::Sample {
            x,
            y: 32.5,
            pressure: 1.0,
            ..Default::default()
        };
        let first = settings([17, 83, 201]);
        let mut expected = app.doc.clone();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        buffer.paint(&mut expected, sample(24.5), &first);
        buffer.finish_tail(&mut expected, &first);
        buffer.reclaim_empty_tiles(&mut expected);
        assert!(app.begin_canvas_stroke(first));
        app.deliver_canvas_dabs(vec![sample(24.5)]);
        app.save(false);
        assert!(!app.stroking && app.queued_stroke.is_none());
        assert!(app.queued_save.is_some() && app.save_job.is_none());
        let bytes = app.render_queue.queued_input_bytes();
        let ticket = app.queued_save.as_ref().unwrap().ticket;
        app.save(false);
        assert_eq!(app.queued_save.as_ref().unwrap().ticket, ticket);
        assert_eq!(app.render_queue.queued_input_bytes(), bytes);
        assert!(app.begin_canvas_stroke(settings([201, 17, 83])));
        app.deliver_canvas_dabs(vec![sample(40.5)]);
        app.finish_painted_stroke();
        let ctx = egui::Context::default();
        while app.queued_save.is_some() {
            app.pump_render_queue(&ctx);
        }
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(app.render_queue.busy());
        let job = app.save_job.take().unwrap();
        let saved_revision = job.revision;
        app.flush_render_queue();
        assert_ne!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        let newer = app.doc.layers[0].pixels.clone();
        assert_ne!(app.revision, saved_revision);
        app.pending_action = Some(crate::document_actions::Action::Close);
        app.commit_save(job.path, job.revision, job.worker.join().unwrap());
        assert!(app.modified && !app.close_ready);
        assert!(app.pending_action.is_none());
        assert_eq!(app.doc.layers[0].pixels, newer);
        assert_eq!(
            storage::load_project(&path).unwrap().layers[0].pixels,
            expected.layers[0].pixels
        );
    }
    #[test]
    fn save_completion_cannot_replace_accepted_but_unrendered_strokes() {
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            ..Default::default()
        };
        let revision = app.revision;
        app.pending_action = Some(crate::document_actions::Action::New(16, 16, 300.0));
        assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        }));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 16.5,
            y: 16.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.finish_painted_stroke();
        assert_eq!(app.revision, revision);
        app.commit_save(PathBuf::from("previous.dudel"), revision, Ok(()));
        assert_eq!(app.doc.width, 32);
        assert!(app.render_queue.busy());
        assert!(app.pending_action.is_none());
        app.flush_render_queue();
        assert!(app.modified && app.revision != revision);
    }
    #[test]
    fn save_commits_the_pending_name_draft_before_its_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("renamed.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(1, 1),
            path: Some(path.clone()),
            ..Default::default()
        };
        let original = app.doc.layers[0].name.clone();
        app.renaming = Some(0);
        app.rename_draft = "Saved sketch".into();
        app.save(false);
        assert!(app.renaming.is_none());
        assert_eq!(app.doc.layers[0].name, "Saved sketch");
        let job = app.save_job.take().unwrap();
        assert_eq!(job.revision, app.revision);
        let result = job.worker.join().unwrap();
        assert_eq!(
            storage::load_project(&path).unwrap().layers[0].name,
            "Saved sketch"
        );
        app.commit_save(job.path, job.revision, result);
        assert!(!app.modified);
        app.undo();
        assert_eq!(app.doc.layers[0].name, original);
        assert!(app.modified);
    }
    #[test]
    fn saving_during_a_stroke_snapshots_final_taper_and_settled_revision() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settled.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(33, 35),
            path: Some(path.clone()),
            modified: true,
            ..Default::default()
        };
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 9.0,
                taper_out: 20.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [30, 40, 50],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        app.history.begin_stroke_checkpoint(&app.doc);
        app.stroking = true;
        app.stroke_settings = Some(settings.clone());
        app.stroke_buffer = Some(Default::default());
        app.stroke_buffer.as_mut().unwrap().paint(
            &mut app.doc,
            crate::stroke::Sample {
                x: 16.5,
                y: 17.5,
                pressure: 1.0,
                ..Default::default()
            },
            &settings,
        );
        let preview = app.doc.layers[0].pixels.clone();
        let revision = app.revision;
        app.save(false);
        assert!(!app.stroking);
        assert!(!app.history.has_pending_stroke());
        assert_ne!(app.doc.layers[0].pixels, preview);
        assert_ne!(app.revision, revision);
        let settled = app.doc.layers[0].pixels.clone();
        let job = app.save_job.take().unwrap();
        assert_eq!(job.revision, app.revision);
        let result = job.worker.join().unwrap();
        assert!(result.is_ok());
        assert_eq!(
            storage::load_project(&path).unwrap().layers[0].pixels,
            settled
        );
        app.commit_save(job.path, job.revision, result);
        assert!(!app.modified);
        assert!(app.history.undo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|v| *v == 0));
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, settled);
    }
    #[test]
    fn deferred_actions_run_only_after_successful_current_save() {
        use crate::document_actions::Action;
        let mut app = Dudel {
            modified: true,
            pending_action: Some(Action::Close),
            ..Dudel::default()
        };
        app.commit_save(
            "failed.dudel".into(),
            app.revision,
            Err("write failed".into()),
        );
        assert!(!app.close_ready);
        assert!(app.modified);
        assert!(app.pending_action.is_none());
        app.pending_action = Some(Action::New(3, 2, 300.0));
        app.commit_save("saved.dudel".into(), app.revision, Ok(()));
        assert_eq!((app.doc.width, app.doc.height), (3, 2));
        assert!(!app.modified);
        assert!(app.path.is_none());
        app.pending_action = Some(Action::Close);
        let revision = app.revision;
        app.changed();
        app.commit_save("saved.dudel".into(), revision, Ok(()));
        assert!(!app.close_ready);
        assert!(app.modified);
        assert!(app.pending_action.is_none());
        app.pending_action = Some(Action::Close);
        app.commit_save("saved.dudel".into(), app.revision, Ok(()));
        assert!(app.close_ready);
    }
    #[test]
    fn background_write_contains_snapshot_and_pending_save_blocks_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("painting.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(1, 1),
            modified: true,
            ..Dudel::default()
        };
        app.doc.layers[0].pixels = vec![10, 30, 200, 128].into();
        let snapshot = app.doc.clone();
        let worker_path = path.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = crate::snapshot_memory::SourceWorker::spawn(snapshot, move |snapshot| {
            receive.recv().map_err(|e| e.to_string())?;
            storage::save_project(&worker_path, &snapshot)
        });
        app.save_job = Some(SaveJob {
            worker,
            path: path.clone(),
            revision: app.revision,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        assert!(app.request_replace(crate::document_actions::Action::Close));
        assert!(!app.close_ready);
        app.doc.layers[0].pixels = vec![255, 0, 0, 255].into();
        app.changed();
        send.send(()).unwrap();
        let job = app.save_job.take().unwrap();
        app.commit_save(job.path, job.revision, job.worker.join().unwrap());
        assert!(app.modified);
        assert_eq!(app.doc.layers[0].pixels, vec![255, 0, 0, 255]);
        assert_eq!(
            storage::load_project(&path).unwrap().layers[0].pixels,
            vec![10, 30, 200, 128]
        );
    }
    #[test]
    fn only_successful_current_revision_clears_unsaved_state() {
        let mut app = Dudel {
            modified: true,
            ..Dudel::default()
        };
        app.commit_save("failed.dudel".into(), app.revision, Err("disk full".into()));
        assert!(app.modified);
        assert!(app.path.is_none());
        assert!(app.recent.is_empty());
        let old = app.revision;
        app.changed();
        app.commit_save("saved.dudel".into(), old, Ok(()));
        assert!(app.modified);
        assert_eq!(app.path, Some(PathBuf::from("saved.dudel")));
        app.commit_save("saved.dudel".into(), app.revision, Ok(()));
        assert!(!app.modified);
        assert_eq!(app.recent.len(), 1);
    }
    #[test]
    fn save_confirmed_during_live_preview_becomes_clean_after_rollback_only() {
        for canceled in [false, true] {
            for successful in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("before-stroke.dudel");
                let recovery = directory.path().join("recovery.gz");
                std::fs::write(&recovery, b"Earlier unsaved artwork").unwrap();
                let mut app = Dudel {
                    doc: crate::document::Document::new(32, 32),
                    modified: true,
                    recovery_path: Some(recovery.clone()),
                    ..Default::default()
                };
                app.start_save_snapshot(path, app.doc.clone());
                let job = app.save_job.take().unwrap();
                let result = job.worker.join().unwrap();
                assert!(result.is_ok());
                assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
                    target: crate::paint_target::Target::Raster,
                    brush: crate::brush::Brush {
                        size: 8.0,
                        pressure_size: false,
                        taper_out: 0.0,
                        ..Default::default()
                    },
                    color: [17, 83, 201],
                    erase: false,
                    layer: 0,
                    symmetry: Default::default(),
                }));
                app.deliver_canvas_dabs(vec![crate::stroke::Sample {
                    x: 16.5,
                    y: 16.5,
                    pressure: 1.0,
                    ..Default::default()
                }]);
                app.flush_render_queue();
                assert!(app.modified && app.revision != job.revision);
                app.commit_save(
                    job.path,
                    job.revision,
                    if successful {
                        result
                    } else {
                        Err("Simulated write failure".into())
                    },
                );
                assert!(app.modified); // The preview is still live.
                assert!(recovery.exists());
                if canceled {
                    app.cancel_painted_stroke();
                } else {
                    app.finish_painted_stroke();
                }
                app.flush_render_queue();
                assert_eq!(app.modified, !(successful && canceled));
                assert_eq!(recovery.exists(), !(successful && canceled));
                assert_eq!(app.content_revision, u64::from(!canceled));
                assert_eq!(
                    app.doc.layers[0].pixels.rgba(16 * 32 + 16)[3] > 0,
                    !canceled
                );
            }
        }
    }
    #[test]
    fn saved_mask_snapshot_matches_after_cancel_or_no_op_but_not_a_guide_edit() {
        for canceled in [false, true] {
            for guide_edit in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let mut app = Dudel {
                    doc: crate::document::Document::new(32, 32),
                    modified: true,
                    ..Default::default()
                };
                app.doc.version = 2;
                let id = crate::document::LayerId::new();
                app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
                    id,
                    enabled: true,
                    linked: true,
                    coverage: crate::layer_mask::Mask::new(32, 32, 255).unwrap(),
                });
                app.start_save_snapshot(directory.path().join("mask.dudel"), app.doc.clone());
                let job = app.save_job.take().unwrap();
                let result = job.worker.join().unwrap();
                assert!(result.is_ok());
                assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
                    target: crate::paint_target::Target::Mask(id),
                    brush: crate::brush::Brush {
                        size: 8.0,
                        pressure_size: false,
                        taper_out: 0.0,
                        ..Default::default()
                    },
                    color: if canceled { [0; 3] } else { [255; 3] },
                    erase: false,
                    layer: 0,
                    symmetry: Default::default(),
                }));
                app.deliver_canvas_dabs(vec![crate::stroke::Sample {
                    x: 16.5,
                    y: 16.5,
                    pressure: 1.0,
                    ..Default::default()
                }]);
                app.flush_render_queue();
                if guide_edit {
                    app.symmetry.mode = crate::symmetry::Mode::LeftRight;
                    app.track_guide_changes();
                }
                if canceled {
                    app.cancel_painted_stroke();
                } else {
                    app.finish_painted_stroke();
                }
                app.flush_render_queue();
                assert!(app.modified); // Save confirmation has not arrived yet.
                app.commit_save(job.path, job.revision, result);
                assert_eq!(app.modified, guide_edit);
                assert_eq!(app.content_revision, u64::from(guide_edit));
                assert_eq!(
                    app.doc.layers[0].mask.as_ref().unwrap().coverage.at(16, 16),
                    255
                );
                assert!(!app.history.can_undo());
            }
        }
    }

    #[test]
    fn geometry_preview_save_confirmation_preserves_guides_and_commits_only_final_content() {
        for masked in [false, true] {
            for outcome in 0..3 {
                // Cancel, return to identity, apply.
                for confirm_during in [false, true] {
                    for successful in [false, true] {
                        for guide_edit in [false, true] {
                            let directory = tempfile::tempdir().unwrap();
                            let path = directory.path().join("before-transform.dudel");
                            let recovery = directory.path().join("recovery.gz");
                            std::fs::write(&recovery, b"Unsaved source").unwrap();
                            let mut app = Dudel {
                                doc: crate::document::Document::new(8, 6),
                                modified: true,
                                recovery_path: Some(recovery.clone()),
                                ..Default::default()
                            };
                            app.doc.layers[0]
                                .pixels
                                .writer()
                                .rgba_mut(10)
                                .copy_from_slice(&[17, 83, 201, 255]);
                            if masked {
                                app.doc.version = 2;
                                let id = crate::document::LayerId::new();
                                app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
                                    id,
                                    enabled: true,
                                    linked: false,
                                    coverage: crate::layer_mask::Mask::new(8, 6, 0).unwrap(),
                                });
                                app.mask_focus = Some((app.doc.layers[0].id, id));
                            }
                            let source = app.doc.clone();
                            app.start_save_snapshot(path, source.clone());
                            let job = app.save_job.take().unwrap();
                            let written = job.worker.join().unwrap();
                            assert!(written.is_ok());
                            let confirmation = if successful {
                                written
                            } else {
                                Err("Injected failure".into())
                            };
                            app.affine_settings = crate::affine::Settings {
                                translation: [2.0, 0.0],
                                sampling: crate::affine::Sampling::Nearest,
                                ..Default::default()
                            };
                            app.start_affine_preview();
                            let started = std::time::Instant::now();
                            while crate::document::same_snapshot(&app.doc, &source) {
                                app.poll_affine_preview();
                                assert!(
                                    started.elapsed() < std::time::Duration::from_secs(5),
                                    "{}",
                                    app.status
                                );
                                std::thread::yield_now();
                            }
                            assert!(app.geometry_preview.is_some() && app.modified);
                            assert_eq!(app.content_revision, 0);
                            if guide_edit {
                                app.grid.visible = true;
                                app.track_guide_changes();
                            }
                            let mut confirmation = Some(confirmation);
                            if confirm_during {
                                app.commit_save(
                                    job.path.clone(),
                                    job.revision,
                                    confirmation.take().unwrap(),
                                );
                                assert!(app.modified && recovery.exists());
                            }
                            match outcome {
                                0 => app.cancel_affine_preview(),
                                1 => {
                                    app.affine_settings = Default::default();
                                    app.finish_affine_preview();
                                }
                                _ => app.finish_affine_preview(),
                            }
                            app.flush_render_queue();
                            assert!(app.geometry_preview.is_none());
                            if !confirm_during {
                                assert!(app.modified); // Original save still unconfirmed.
                                app.commit_save(
                                    job.path,
                                    job.revision,
                                    confirmation.take().unwrap(),
                                );
                            }
                            let changed = outcome == 2;
                            assert_eq!(
                                app.modified,
                                !successful || changed || guide_edit,
                                "masked={masked} outcome={outcome} during={confirm_during} success={successful} guide={guide_edit}"
                            );
                            assert_eq!(recovery.exists(), app.modified);
                            assert_eq!(
                                app.content_revision,
                                u64::from(changed) + u64::from(guide_edit)
                            );
                            assert_eq!(app.grid.visible, guide_edit);
                            assert_eq!(app.history.can_undo(), changed);
                            if !changed {
                                assert!(crate::document::same_snapshot(&app.doc, &source));
                            }
                            if masked {
                                assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
                            }
                        }
                    }
                }
            }
        }
    }
}
