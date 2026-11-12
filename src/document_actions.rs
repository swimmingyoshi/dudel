use crate::{
    Dudel,
    document::{Document, History},
    preferences,
};
use std::path::PathBuf;

pub enum Action {
    New(usize, usize, f64),
    Replace {
        document: Box<Document>,
        path: Option<PathBuf>,
        modified: bool,
        status: &'static str,
    },
    Close,
    /// Close the active project's tab.
    CloseTab,
}
#[derive(Debug, PartialEq)]
pub(crate) enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}
impl Dudel {
    pub(crate) fn first_close_request_in_frame(&mut self, frame: u64) -> bool {
        if self.last_close_prompt_frame == Some(frame) {
            return false;
        }
        self.last_close_prompt_frame = Some(frame);
        true
    }
    pub(crate) fn request_replace(&mut self, action: Action) -> bool {
        if self.unsaved_action.is_some() {
            return false;
        }
        if self.pending_action.is_some() {
            self.status = "A canvas action is already waiting for the save".into();
            return false;
        }
        // Replacing the project can wait, so this asks before anything closes.
        if let Some(question) = self.transform_rasterizes_text() {
            let question = format!("{question} Apply it to continue, or Cancel to stay here.");
            self.confirm(
                "Rasterize text?",
                question,
                "Rasterize and apply",
                move |app| {
                    app.text_transform_confirmed = true;
                    app.finish_affine_preview();
                    app.flush_render_queue();
                    app.request_replace(action);
                },
            );
            return false;
        }
        self.finish_layer_rename(true);
        self.cancel_gestures();
        self.track_guide_changes();
        if self.save_job.is_some() {
            self.pending_action = Some(action);
            self.status = "Canvas action will continue after saving".into();
            return true;
        }
        if !self.modified {
            self.apply_action(action);
            return true;
        }
        self.unsaved_action = Some(action);
        false
    }
    pub(crate) fn unsaved_dialog(&mut self, ctx: &eframe::egui::Context) {
        if self.unsaved_action.is_none() {
            return;
        }
        let mut choice = None;
        let name = self.tab_title(self.tabs.active).0;
        let response =
            eframe::egui::Modal::new(eframe::egui::Id::new("unsaved painting")).show(ctx, |ui| {
                ui.set_width(360.0);
                if crate::dialogs::dialog_title(ui, "Save your painting?") {
                    choice = Some(UnsavedChoice::Cancel);
                }
                ui.label(format!(
                    "‘{name}’ has unsaved changes. Save them before continuing?"
                ));
                ui.add_space(12.0);
                crate::dialogs::dialog_buttons(ui, |ui| {
                    if ui
                        .add(crate::workspace::primary_button(ui, "Save"))
                        .clicked()
                    {
                        choice = Some(UnsavedChoice::Save);
                    }
                    if ui
                        .button("Don't save")
                        .on_hover_text("Continue and lose the unsaved changes")
                        .clicked()
                    {
                        choice = Some(UnsavedChoice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(UnsavedChoice::Cancel);
                    }
                });
            });
        if choice.is_none() && response.should_close() {
            choice = Some(UnsavedChoice::Cancel);
        }
        if let Some(choice) = choice {
            self.answer_unsaved(choice);
        }
    }
    pub(crate) fn answer_unsaved(&mut self, choice: UnsavedChoice) {
        if let Some(action) = self.unsaved_action.take() {
            self.resolve_unsaved(action, choice);
        }
    }
    fn resolve_unsaved(&mut self, action: Action, choice: UnsavedChoice) -> bool {
        match choice {
            UnsavedChoice::Save => {
                self.save_before(false, Some(action));
                self.save_job.is_some() || self.queued_save.is_some() || self.dialogs.open()
            }
            UnsavedChoice::Discard => {
                self.apply_action(action);
                true
            }
            _ => false,
        }
    }
    pub(crate) fn apply_action(&mut self, action: Action) {
        // A completed load may unblock another queued open. Explicit New/Close
        // supersedes it, while Replace is the result of the existing load.
        if !matches!(&action, Action::Replace { .. }) {
            self.cancel_pending_tab_change();
        }
        if matches!(action, Action::New(..)) {
            self.new_canvas = false;
        }
        self.retire_indexed_session();
        self.cancel_gestures();
        self.cancel_load();
        if matches!(action, Action::Close) {
            self.clear_recovery();
            // Each other project with unsaved changes is shown and asked about in turn.
            if self.next_unsaved_tab_for_close() {
                self.request_replace(Action::Close);
                return;
            }
            self.clear_parked_recovery();
            self.close_ready = true;
            return;
        }
        if matches!(action, Action::CloseTab) {
            self.clear_recovery();
            self.remove_active_tab(None);
            self.heap_compact_pending = true;
            return;
        }
        let fresh_canvas = matches!(action, Action::New(..));
        let (document, path, modified, status) = match action {
            Action::New(width, height, dpi) => (
                {
                    let mut doc = Document::new(width, height);
                    doc.dpi = dpi;
                    doc
                },
                None,
                false,
                "New canvas created",
            ),
            Action::Replace {
                document,
                path,
                modified,
                status,
            } => (*document, path, modified, status),
            Action::Close | Action::CloseTab => unreachable!(),
        };
        let mut document = document;
        let guides = document.guides.take();
        let references = std::mem::take(&mut document.references);
        self.clear_recovery();
        self.doc = document;
        self.observed_canvas_frame = *self.doc.canvas_frame;
        self.canvas_resize = None;
        self.canvas_edit_ticket = None;
        self.apply_guides(guides);
        self.reference_layers.install(references);
        self.reference_layers.bind_positions(&self.doc);
        if matches!(self.fill_source, crate::fill::Source::Layer(_)) {
            self.fill_source = crate::fill::Source::Active;
        }
        if let Some(path) = &path {
            preferences::remember(&mut self.recent, path.clone());
        }
        self.path = path;
        self.history = History::new();
        self.fit_view();
        self.rotation = 0.0;
        self.mirror = false;
        self.cancel_gestures();
        self.last_stroke_end = None;
        self.renaming = None;
        self.rename_draft.clear();
        self.changed();
        self.modified = modified;
        self.stroke_preview = None;
        self.geometry_preview = None;
        self.mask_focus = None;
        self.layer_reveal = None;
        self.saved_content_revision = None;
        self.status = status.into();
        self.timelapse_document_changed(fresh_canvas);
        self.heap_compact_pending = true;
    }
    /// A replaced or closed document frees thousands of 256 KiB tiles inside
    /// the process heap, which can keep those pages committed (measured with
    /// the maximum 32-layer study, 2026-10-20). Once workers no longer hold
    /// the old document, ask the heap to coalesce and decommit large free blocks.
    pub(crate) fn compact_heap_when_settled(&mut self) {
        if !self.heap_compact_pending
            || self.texture.busy()
            || self.render_queue.busy()
            || self.load_job.is_some()
            || self.save_job.is_some()
            || self.thumbnail_revision != Some(self.revision)
        {
            return;
        }
        self.heap_compact_pending = false;
        #[cfg(windows)]
        // SAFETY: GetProcessHeap returns this process's default heap; HeapCompact
        // only coalesces and decommits free blocks under the heap's own lock.
        unsafe {
            use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapCompact};
            HeapCompact(GetProcessHeap(), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_new_canvas_keeps_its_submitted_dpi() {
        let mut app = Dudel {
            modified: true,
            ..Default::default()
        };
        assert!(!app.request_replace(Action::New(12, 14, 600.5)));
        app.new_canvas_form.dpi = "72".into();
        app.answer_unsaved(UnsavedChoice::Discard);
        assert_eq!(
            (app.doc.width, app.doc.height, app.doc.dpi),
            (12, 14, 600.5)
        );
    }
    #[test]
    fn replacement_prompts_for_committed_name_drafts_and_settled_taper() {
        let mut app = Dudel {
            renaming: Some(0),
            rename_draft: "Unsaved name".into(),
            ..Default::default()
        };
        assert!(!app.request_replace(Action::New(10, 10, 300.0)));
        assert!(app.unsaved_action.is_some());
        assert_eq!(app.doc.layers[0].name, "Unsaved name");
        let (mut stroke, expected) = crate::edit_order_tests::pending_taper();
        assert!(!stroke.modified);
        assert!(!stroke.request_replace(Action::New(10, 10, 300.0)));
        assert!(stroke.unsaved_action.is_some());
        assert_eq!(stroke.doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(!stroke.stroking);
    }
    #[test]
    fn replaced_document_compacts_heap_once_after_thumbnail_settles() {
        let mut app = Dudel::default();
        app.apply_action(Action::New(10, 10, 300.0));
        assert!(app.heap_compact_pending);
        app.thumbnail_revision = None;
        app.compact_heap_when_settled();
        assert!(app.heap_compact_pending);
        app.thumbnail_revision = Some(app.revision);
        app.compact_heap_when_settled();
        assert!(!app.heap_compact_pending);
    }
    #[test]
    fn repeated_layout_pass_does_not_prompt_again_but_later_close_can() {
        let mut app = Dudel::default();
        assert!(app.first_close_request_in_frame(10));
        assert!(!app.first_close_request_in_frame(10));
        assert!(!app.resolve_unsaved(Action::Close, UnsavedChoice::Cancel));
        assert!(!app.first_close_request_in_frame(10));
        assert!(app.first_close_request_in_frame(11));
    }
    #[test]
    fn repeated_close_requests_share_one_confirmation_and_cancel_allows_retry() {
        let mut app = Dudel {
            modified: true,
            ..Dudel::default()
        };
        assert!(!app.request_replace(Action::Close));
        assert!(matches!(app.unsaved_action, Some(Action::Close)));
        assert!(!app.request_replace(Action::Close));
        app.answer_unsaved(UnsavedChoice::Cancel);
        assert!(app.unsaved_action.is_none());
        assert!(!app.close_ready);
        assert!(app.modified);
        assert!(app.pending_action.is_none());
        assert!(!app.request_replace(Action::Close));
        app.answer_unsaved(UnsavedChoice::Discard);
        assert!(app.close_ready);
        assert!(app.unsaved_action.is_none());
        assert!(app.save_job.is_none());
    }
    #[test]
    fn answering_an_already_dismissed_confirmation_has_no_effect() {
        let mut app = Dudel {
            modified: true,
            ..Dudel::default()
        };
        app.answer_unsaved(UnsavedChoice::Discard);
        assert!(!app.close_ready);
        assert!(app.modified);
        assert!(app.unsaved_action.is_none());
    }
}
