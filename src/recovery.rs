use crate::{Dudel, egui, storage};

fn remove_snapshot(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not clear recovery snapshot: {error}")),
    }
}

impl Dudel {
    pub(crate) fn close_viewport_if_ready(&mut self, ctx: &egui::Context) {
        if self.close_ready {
            if self.retired_adjustment.is_some()
                || self.retired_recovery.is_some()
                || self.export_job.is_some()
                || self.queued_export.is_some()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            } else {
                self.clipboard_backend = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
    pub(crate) fn init_recovery(&mut self) {
        match storage::recovery_directory() {
            Ok(directory) => self.init_recovery_in(directory),
            Err(e) => self.status = format!("Recovery unavailable: {e}"),
        }
    }
    /// Shared startup discovery; diagnostics supply their own disposable folder.
    pub(crate) fn init_recovery_in(&mut self, directory: std::path::PathBuf) {
        if let Ok(entries) = std::fs::read_dir(&directory) {
            self.recovery_candidates = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "gz"))
                .collect();
            self.recovery_candidates.sort();
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.recovery_path =
            Some(directory.join(format!("session-{stamp}-{}.gz", std::process::id())));
        self.recovery_window = !self.recovery_candidates.is_empty();
    }
    pub(crate) fn clear_recovery(&mut self) {
        self.thumbnail_revision = None;
        self.cancel_filter();
        self.selection_drag = None;
        self.clear_recovery_snapshot();
    }
    /// Retire a now-saved snapshot without canceling unrelated input gestures.
    pub(crate) fn clear_recovery_snapshot(&mut self) {
        if let Some(job) = self.recovery_job.take() {
            let path = self.recovery_path.clone();
            self.retired_recovery =
                Some(job.after(move |_| path.as_deref().map_or(Ok(()), remove_snapshot)));
        }
        if self.retired_recovery.is_none()
            && let Some(path) = &self.recovery_path
            && let Err(error) = remove_snapshot(path)
        {
            self.status = error;
        }
        self.recovery_revision = self.revision;
        self.recovery_timer = std::time::Instant::now();
        self.recovery_saved = None;
    }
    pub(crate) fn poll_recovery(&mut self, ctx: &egui::Context) {
        self.track_guide_changes();
        if self
            .retired_recovery
            .as_ref()
            .is_some_and(|job| job.is_finished())
        {
            match self.retired_recovery.take().unwrap().join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => self.status = error,
                Err(_) => self.status = "Recovery cleanup worker failed".into(),
            }
        }
        if self
            .recovery_job
            .as_ref()
            .is_some_and(|job| job.is_finished())
        {
            match self.recovery_job.take().unwrap().join() {
                Ok(Ok(revision)) => {
                    self.recovery_revision = revision;
                    self.recovery_saved = Some(std::time::Instant::now());
                }
                Ok(Err(e)) => {
                    self.cancel_pending_tab_change();
                    self.status = format!(
                        "Recovery snapshot failed: {e}; save before opening another project"
                    );
                }
                Err(_) => {
                    self.cancel_pending_tab_change();
                    self.status =
                        "Recovery worker failed; save your project before opening another".into();
                }
            }
        }
        if self.adjustment.is_none()
            && self.modified
            && !self.close_ready
            && self.revision != self.recovery_revision
            && self.recovery_job.is_none()
            && self.retired_recovery.is_none()
            && self.recovery_timer.elapsed().as_secs() >= 15
            && !self.stroking
            && self.guide_drag.is_none()
            && !self.symmetry_drag
            && !self.render_queue.busy()
            && self.opacity_edit.is_none()
            && self.text_edit.is_none()
            && let Some(path) = self.recovery_path.clone()
        {
            let mut document = self.doc.clone();
            document.guides = Some(Box::new(self.guides_record(&document)));
            self.reference_layers.saved_positions(&self.doc);
            document.references = self.reference_layers.layers.clone();
            let original_path = self.path.clone();
            let revision = self.revision;
            self.recovery_job = Some(crate::snapshot_memory::SourceWorker::spawn(
                document,
                move |document| {
                    let snapshot = storage::Recovery {
                        document,
                        original_path,
                    };
                    storage::save_recovery(&path, &snapshot).map(|()| revision)
                },
            ));
            self.recovery_timer = std::time::Instant::now();
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let mut open = self.recovery_window;
        egui::Window::new("Recover artwork").open(&mut open).default_width(480.0).show(ctx,|ui| {
            ui.label("Recovery snapshots are available. Restore opens a copy; Save as keeps your original project safe.");
            ui.label("Snapshots from another running Dudel instance may also appear here.");
            egui::ScrollArea::vertical().max_height(300.0).show(ui,|ui| {
                for path in self.recovery_candidates.clone() {
                    ui.group(|ui| {
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        ui.horizontal(|ui| {
                            if ui.button("Restore copy").clicked() {
                                let path = path.clone();
                                self.open_in_tab(move |app| app.load_recovery(path));
                            }
                            if ui.button("Remove snapshot").clicked() {
                                let path=path.clone();
                                self.confirm("Remove recovery snapshot","Permanently remove this recovery snapshot?","Remove",move|app| {
                                    match std::fs::remove_file(&path) {Ok(())=>app.recovery_candidates.retain(|p|p!=&path),Err(e)=>app.status=e.to_string()}
                                });
                            }
                        });
                    });
                }
            });
        });
        self.recovery_window = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guide_only_edits_trigger_recovery_even_when_artwork_was_already_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guides.gz");
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            recovery_path: Some(path.clone()),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        for spacing in [32, 64] {
            app.grid.visible = true;
            app.grid.spacing = spacing;
            app.recovery_timer = std::time::Instant::now() - std::time::Duration::from_secs(16);
            let old_revision = app.revision;
            let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
            assert!(app.recovery_job.is_some() && app.modified);
            assert_ne!(app.revision, old_revision);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.recovery_job.is_some() {
                assert!(std::time::Instant::now() < deadline);
                let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
                std::thread::yield_now();
            }
            let recovered = storage::read_recovery(&path).unwrap();
            assert_eq!(recovered.document.guides.unwrap().grid.spacing, spacing);
            assert_eq!(app.recovery_revision, app.revision);
            let settled = app.revision;
            app.recovery_timer = std::time::Instant::now() - std::time::Duration::from_secs(16);
            let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
            assert!(app.recovery_job.is_none());
            assert_eq!(app.revision, settled);
        }
        assert!(!app.history.can_undo());
    }
    fn pending_recovery(path: std::path::PathBuf) -> (Dudel, std::sync::mpsc::Sender<()>) {
        let mut app = Dudel {
            doc: crate::document::Document::new(32, 32),
            recovery_path: Some(path.clone()),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        let snapshot = app.doc.clone();
        storage::save_recovery(
            &path,
            &storage::Recovery {
                document: snapshot.clone(),
                original_path: None,
            },
        )
        .unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        app.recovery_job = Some(crate::snapshot_memory::SourceWorker::spawn(
            snapshot,
            move |document| {
                receive
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .map_err(|error| error.to_string())?;
                storage::save_recovery(
                    &path,
                    &storage::Recovery {
                        document,
                        original_path: None,
                    },
                )?;
                Ok(999)
            },
        ));
        (app, send)
    }
    #[test]
    fn clear_recovery_retires_without_waiting_and_gates_path_reuse_until_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retired.gz");
        let (mut app, send) = pending_recovery(path.clone());
        app.clear_recovery();
        assert!(app.recovery_job.is_none());
        assert!(!app.retired_recovery.as_ref().unwrap().is_finished());
        assert!(
            path.exists(),
            "active writer must finish before its path is removed"
        );
        app.clear_recovery();
        assert!(!app.retired_recovery.as_ref().unwrap().is_finished());
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 255]);
        app.changed();
        app.recovery_timer = std::time::Instant::now() - std::time::Duration::from_secs(16);
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
        assert!(
            app.recovery_job.is_none(),
            "new writer must wait for old path cleanup"
        );
        let mut registry = crate::snapshot_memory::Registry::new(&app.doc);
        app.retired_recovery
            .as_ref()
            .unwrap()
            .register_source(&mut registry);
        assert!(registry.additional_bytes() >= 32 * 32 * 4);
        send.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.retired_recovery.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!path.exists());
        let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
        assert!(app.retired_recovery.is_none());
        assert_ne!(
            app.recovery_revision, 999,
            "retired revision must never be published"
        );
        let job = app.recovery_job.take().unwrap();
        assert_eq!(job.join().unwrap().unwrap(), app.revision);
        assert_eq!(
            storage::read_recovery(&path).unwrap().document.layers[0].pixels,
            app.doc.layers[0].pixels
        );
    }
    #[test]
    fn accepted_close_cancels_viewport_close_until_cleanup_and_never_starts_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("closing.gz");
        let (mut app, send) = pending_recovery(path.clone());
        app.apply_action(crate::document_actions::Action::Close);
        assert!(app.close_ready);
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| app.close_viewport_if_ready(ctx));
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::CancelClose))
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        );
        app.modified = true;
        app.revision = app.revision.wrapping_add(1);
        app.recovery_timer = std::time::Instant::now() - std::time::Duration::from_secs(16);
        send.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.retired_recovery.as_ref().unwrap().is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let output = ctx.run(Default::default(), |ctx| {
            app.poll_recovery(ctx);
            app.close_viewport_if_ready(ctx);
        });
        assert!(app.recovery_job.is_none());
        assert!(app.retired_recovery.is_none());
        assert!(!path.exists());
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        );
    }
    #[test]
    fn retired_cleanup_removes_its_snapshot_after_owner_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dropped.gz");
        let (mut app, send) = pending_recovery(path.clone());
        app.clear_recovery();
        drop(app);
        send.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while path.exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
}
