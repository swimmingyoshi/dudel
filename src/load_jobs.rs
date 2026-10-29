use crate::{Dudel, document::Document, document_actions::Action, egui, storage};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum LoadKind {
    Project,
    Indexed,
    Png,
    Clip,
    ClipPreview,
    Psd,
    Recovery,
}
pub struct LoadJob {
    worker: std::thread::JoinHandle<Result<Document, String>>,
    path: PathBuf,
    kind: LoadKind,
    revision: u64,
    cancel: Arc<AtomicBool>,
    notes: Arc<std::sync::Mutex<Vec<String>>>,
}
impl Dudel {
    pub(crate) fn load_document(&mut self, path: PathBuf, png: bool) {
        let indexed = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("didx"));
        if indexed && !self.experimental_indexed_storage {
            self.status = "Opening .didx files requires --experimental-indexed-storage".into();
            return;
        }
        let kind = if indexed {
            LoadKind::Indexed
        } else if png {
            LoadKind::Png
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("clip"))
        {
            LoadKind::Clip
        } else if path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("psd"))
        {
            LoadKind::Psd
        } else {
            LoadKind::Project
        };
        if indexed {
            self.indexed_session = None;
        }
        self.start_load(path, kind);
    }
    pub(crate) fn load_recovery(&mut self, path: PathBuf) {
        self.start_load(path, LoadKind::Recovery);
    }
    fn start_load(&mut self, path: PathBuf, kind: LoadKind) {
        if self.load_job.is_some() {
            self.status = "A document is already loading; cancel it before opening another".into();
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_path = path.clone();
        let notes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let worker_notes = notes.clone();
        let worker = std::thread::spawn(move || {
            if worker_cancel.load(Ordering::Relaxed) {
                return Err("Load canceled".into());
            }
            match kind {
                LoadKind::Png => storage::import_png(&worker_path),
                LoadKind::Clip | LoadKind::ClipPreview | LoadKind::Psd => {
                    let canceled = || worker_cancel.load(Ordering::Relaxed);
                    let imported = match kind {
                        LoadKind::Clip => {
                            crate::clip_metadata::import_document(&worker_path, &canceled)
                        }
                        LoadKind::ClipPreview => {
                            crate::clip_metadata::import_preview(&worker_path, &canceled)
                        }
                        _ => crate::psd_import::import(&worker_path, &canceled),
                    }?;
                    *worker_notes
                        .lock()
                        .map_err(|_| "Import report unavailable")? = imported.notes;
                    Ok(imported.document)
                }
                LoadKind::Project => storage::load_project(&worker_path),
                LoadKind::Indexed => crate::indexed_commit::load_document(&worker_path, &|| {
                    worker_cancel.load(Ordering::Relaxed)
                }),
                LoadKind::Recovery => {
                    storage::read_recovery(&worker_path).map(|snapshot| snapshot.document)
                }
            }
        });
        self.load_job = Some(LoadJob {
            worker,
            path,
            kind,
            revision: self.revision,
            cancel,
            notes,
        });
        self.status = "Opening document in background…".into();
    }
    pub(crate) fn cancel_load(&mut self) {
        if let Some(job) = &self.load_job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn commit_load(
        &mut self,
        path: PathBuf,
        kind: LoadKind,
        revision: u64,
        canceled: bool,
        result: Result<Document, String>,
    ) {
        if canceled {
            self.status = "Document load canceled".into();
            return;
        }
        if revision != self.revision {
            self.status = "Loaded document discarded because artwork changed; open it again".into();
            return;
        }
        match result {
            Ok(document) => {
                self.request_replace(Action::Replace {
                    document: Box::new(document),
                    path: if matches!(kind, LoadKind::Project | LoadKind::Indexed) {
                        Some(path)
                    } else {
                        None
                    },
                    modified: !matches!(kind, LoadKind::Project | LoadKind::Indexed),
                    status: match kind {
                        LoadKind::Png => "PNG imported — save a project to keep editable artwork",
                        LoadKind::Clip => "CSP raster imported — save as a Dudel project",
                        LoadKind::ClipPreview => {
                            "CSP saved preview imported — flattened, possibly reduced resolution"
                        }
                        LoadKind::Psd => {
                            "PSD imported — see Import details and save as a Dudel project"
                        }
                        LoadKind::Project => "Project opened",
                        LoadKind::Indexed => "Experimental indexed project opened",
                        LoadKind::Recovery => "Recovered painting — save a new project to keep it",
                    },
                });
            }
            Err(error) => {
                self.status = format!("Could not open {}: {error}", path.display());
                if kind == LoadKind::Clip {
                    let text = format!(
                        "Editable layers could not be imported: {error}\n\nYou can open CSP's saved preview as one flattened image. It may be smaller than the original canvas and represents only the saved frame, without editable layers or animation."
                    );
                    self.confirm(
                        "CSP import options",
                        text,
                        "Open saved preview",
                        move |app| {
                            app.open_in_tab(move |app| app.start_load(path, LoadKind::ClipPreview));
                        },
                    );
                }
            }
        }
    }
    pub(crate) fn poll_load(&mut self, ctx: &egui::Context) {
        if self
            .load_job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.load_job.take().unwrap();
            let canceled = job.cancel.load(Ordering::Relaxed);
            let result = job
                .worker
                .join()
                .unwrap_or_else(|_| Err("Document loading worker failed".into()));
            if !canceled && job.revision == self.revision && result.is_ok() {
                self.import_notes = job.notes.lock().map(|n| n.clone()).unwrap_or_default();
            }
            self.commit_load(job.path, job.kind, job.revision, canceled, result);
            self.settle_tab_after_load();
        }
        if self.load_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    /// Open progress, drawn inside the status row like save progress.
    /// Returns false when no document is loading.
    pub(crate) fn load_progress_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(job) = &self.load_job else {
            return false;
        };
        // Fixed size: the theme's touch-sized interact height would grow the status row.
        ui.add(egui::Spinner::new().size(16.0));
        ui.label(if job.cancel.load(Ordering::Relaxed) {
            "Canceling document load…"
        } else {
            "Opening document…"
        });
        if ui.add(egui::Button::new("Cancel").small()).clicked() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        true
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn csp_worker_imports_an_unsaved_copy_and_protects_current_artwork_on_cancel_or_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.CLIP");
        let source = crate::clip_metadata::tests::worker_fixture();
        std::fs::write(&path, &source).unwrap();
        let mut app = Dudel::default();
        app.load_document(path.clone(), false);
        let job = app.load_job.take().unwrap();
        assert_eq!(job.kind, LoadKind::Clip);
        let result = job.worker.join().unwrap();
        app.commit_load(job.path, job.kind, job.revision, false, result);
        assert!(app.modified);
        assert!(app.path.is_none());
        assert!(app.recent.is_empty());
        assert_eq!((app.doc.width, app.doc.height), (32, 48));
        assert!(app.doc.layers.iter().all(|layer| layer.csp_byte_opacity));
        assert_eq!(std::fs::read(&path).unwrap(), source);
        let before = app.doc.clone();
        let revision = app.revision;
        app.commit_load(
            path.clone(),
            LoadKind::Clip,
            revision,
            false,
            Ok(Document::new(1, 1)),
        );
        assert!(app.unsaved_action.is_some());
        assert!(crate::document::same_snapshot(&app.doc, &before));
        app.unsaved_action = None;
        for (canceled, rev, result) in [
            (true, revision, Ok(Document::new(1, 1))),
            (false, revision.wrapping_sub(1), Ok(Document::new(1, 1))),
            (
                false,
                revision,
                Err("Unsupported clip content: animation".into()),
            ),
        ] {
            app.commit_load(path.clone(), LoadKind::Clip, rev, canceled, result);
            assert!(crate::document::same_snapshot(&app.doc, &before));
            assert!(app.modified);
            assert!(app.path.is_none());
        }
        app.load_document(path.clone(), false);
        app.cancel_load();
        let job = app.load_job.take().unwrap();
        let canceled = job.cancel.load(Ordering::Relaxed);
        let result = job.worker.join().unwrap();
        app.commit_load(job.path, job.kind, job.revision, canceled, result);
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert_eq!(std::fs::read(&path).unwrap(), source);
        let unsupported = crate::clip_metadata::tests::worker_fixture_with_sql(
            // A canvas color profile rejects the whole file (an unsupported
            // layer would only be skipped).
            "ALTER TABLE Canvas ADD COLUMN CanvasDstProfile BLOB; UPDATE Canvas SET CanvasDstProfile=x'0102'",
        );
        std::fs::write(&path, &unsupported).unwrap();
        app.load_document(path.clone(), false);
        let job = app.load_job.take().unwrap();
        let result = job.worker.join().unwrap();
        assert!(result.as_ref().err().unwrap().contains("CanvasDstProfile"));
        app.commit_load(job.path, job.kind, job.revision, false, result);
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert!(app.modified);
        assert!(app.path.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), unsupported);
    }
    use super::*;
    #[test]
    fn recovery_worker_opens_unsaved_copy_and_keeps_source_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshot.gz");
        let mut document = Document::new(2, 1);
        document.layers[0].pixels = vec![10, 30, 200, 128, 40, 50, 60, 255].into();
        storage::save_recovery(
            &path,
            &storage::Recovery {
                document,
                original_path: Some("original.dudel".into()),
            },
        )
        .unwrap();
        let mut app = Dudel::default();
        app.load_recovery(path.clone());
        let job = app.load_job.take().unwrap();
        let result = job.worker.join().unwrap();
        app.commit_load(job.path, job.kind, job.revision, false, result);
        assert_eq!(
            app.doc.layers[0].pixels,
            [10, 30, 200, 128, 40, 50, 60, 255]
        );
        assert!(app.path.is_none());
        assert!(app.modified);
        assert!(app.recent.is_empty());
        assert!(path.exists());
        for (canceled, revision) in [(true, app.revision), (false, app.revision - 1)] {
            app.commit_load(
                path.clone(),
                LoadKind::Recovery,
                revision,
                canceled,
                Ok(Document::new(3, 3)),
            );
            assert_eq!((app.doc.width, app.doc.height), (2, 1));
            assert!(app.modified);
            assert!(app.path.is_none());
        }
    }
    #[test]
    fn real_background_load_reopens_saved_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("painting.dudel");
        let mut document = Document::new(2, 1);
        document.layers[0].pixels = vec![10, 30, 200, 128, 40, 50, 60, 255].into();
        storage::save_project(&path, &document).unwrap();
        let mut app = Dudel::default();
        app.load_document(path.clone(), false);
        let job = app.load_job.take().unwrap();
        let result = job.worker.join().unwrap();
        app.commit_load(
            job.path,
            job.kind,
            job.revision,
            job.cancel.load(Ordering::Relaxed),
            result,
        );
        assert_eq!(app.doc.layers[0].pixels, document.layers[0].pixels);
        assert_eq!(app.path, Some(path));
        assert!(!app.modified);
    }
    #[test]
    fn canceled_stale_and_failed_loads_preserve_document_and_path() {
        let mut app = Dudel {
            doc: Document::new(1, 1),
            path: Some("original.dudel".into()),
            ..Dudel::default()
        };
        app.commit_load(
            "next.dudel".into(),
            LoadKind::Project,
            app.revision,
            true,
            Ok(Document::new(2, 3)),
        );
        assert_eq!((app.doc.width, app.doc.height), (1, 1));
        let old = app.revision;
        app.changed();
        app.commit_load(
            "next.dudel".into(),
            LoadKind::Project,
            old,
            false,
            Ok(Document::new(2, 3)),
        );
        assert_eq!(app.doc.width, 1);
        assert!(app.modified);
        app.commit_load(
            "next.dudel".into(),
            LoadKind::Project,
            app.revision,
            false,
            Err("corrupt file".into()),
        );
        assert_eq!(app.path, Some(PathBuf::from("original.dudel")));
        assert!(app.recent.is_empty());
    }
    #[test]
    fn successful_current_load_sets_project_or_import_state() {
        let mut app = Dudel::default();
        app.commit_load(
            "next.dudel".into(),
            LoadKind::Project,
            app.revision,
            false,
            Ok(Document::new(2, 3)),
        );
        assert_eq!((app.doc.width, app.doc.height), (2, 3));
        assert!(!app.modified);
        assert_eq!(app.path, Some(PathBuf::from("next.dudel")));
        app.commit_load(
            "source.png".into(),
            LoadKind::Png,
            app.revision,
            false,
            Ok(Document::new(3, 2)),
        );
        assert!(app.modified);
        assert!(app.path.is_none());
    }
}
