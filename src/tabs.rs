//! Several projects open at once, one tab each. The active project lives in
//! `Dudel`'s own fields so every existing feature works unchanged; inactive
//! projects are parked in a `Session` and do no work at all until they are
//! shown again. Switching moves fields, it never copies artwork.
//!
//! A switch waits until the active project is settled: no stroke, queued
//! rendering, save, export, load, filter or clipboard job in flight. Parked
//! sessions therefore never hold running work that would need polling; only
//! display tiles may still be compositing, and they resume when shown.
use crate::{Dudel, egui};
use std::path::PathBuf;

// The active and parked projects have the same fields. Keep their accounting in
// one place so switching tabs cannot hide allocations from the estimates.
macro_rules! register_project_extras {
    ($project:expr, $registry:expr) => {{
        let project = $project;
        let registry = $registry;
        project.history.register_allocations(registry);
        project.recording.register_allocations(registry);
        if let Some(preview) = &project.adjustment {
            preview.register_allocations(registry);
        }
        if let Some(worker) = &project.retired_adjustment {
            worker.register_source(registry);
        }
        project.selection_contour.register_allocations(registry);
        if let Some(drag) = &project.move_drag {
            drag.engine.register_allocations(registry);
        }
        if let Some(preview) = &project.affine_preview {
            preview.register_allocations(registry);
        }
        if let Some(edit) = &project.text_edit {
            registry.add_document(edit.source());
        }
        if let Some(job) = &project.paste_job {
            registry.add_document(&job.source);
        }
        if let Some(job) = &project.copy_job {
            registry.add_document(&job.source);
        }
        if let Some(settlement) = &project.clipboard_settlement {
            settlement.cursor.register_allocations(registry);
        }
        project
            .render_queue
            .register_settlement_allocations(registry);
        registry.add_owned(project.render_queue.stroke_working_bytes());
        project.texture.register_snapshots(registry);
        if let Some(job) = &project.save_job {
            job.worker.register_source(registry);
        }
        if let Some(job) = &project.export_job {
            job.register_allocations(registry);
        }
        if let Some(job) = &project.recovery_job {
            job.register_source(registry);
        }
        if let Some(job) = &project.retired_recovery {
            job.register_source(registry);
        }
        project.reference_layers.register_allocations(registry);
        let display = project.texture.memory();
        registry.add_owned(display.metadata_heap_bytes + display.prefix_heap_bytes);
        registry.add_owned(
            project
                .indexed_session
                .as_ref()
                .map_or(0, |s| s.descriptor_bytes())
                + project
                    .indexed_pending
                    .as_ref()
                    .map_or(0, |p| p.descriptor_bytes()),
        );
    }};
}

/// Lists each per-project field once, generating the parked session and the
/// exchange with the live fields.
macro_rules! project_session {
    ($($field:ident : $ty:ty),* $(,)?) => {
        pub(crate) struct Session {
            $(pub(crate) $field: $ty,)*
        }
        impl Session {
            fn register_extras(&self, registry: &mut crate::snapshot_memory::Registry) {
                register_project_extras!(self, registry);
            }
            /// Exchange every per-project field with the app's live ones.
            fn swap(&mut self, app: &mut Dudel) {
                $(std::mem::swap(&mut self.$field, &mut app.$field);)*
            }
            /// A new, empty project with default view and history.
            fn blank() -> Box<Session> {
                let Dudel { $($field,)* .. } = Dudel::default();
                Box::new(Session { $($field,)* })
            }
        }
        impl Dudel {
            pub(crate) fn register_project_extras(&self, registry: &mut crate::snapshot_memory::Registry) {
                register_project_extras!(self, registry);
            }
        }
    };
}

project_session! {
    doc: crate::document::Document,
    history: crate::document::History,
    path: Option<PathBuf>,
    modified: bool,
    revision: u64,
    content_revision: u64,
    saving_content_revision: Option<u64>,
    saved_content_revision: Option<u64>,
    dirty: bool,
    dirty_region: crate::render_region::Damage,
    texture: crate::canvas_textures::CanvasTextures,
    layer_thumbnails: crate::Thumbnails,
    mask_thumbnails: crate::Thumbnails,
    thumbnail_revision: Option<u64>,
    thumbnail_timer: std::time::Instant,
    thumbnail_stamps: std::collections::HashMap<
        crate::document::LayerId,
        (bool, crate::pixels::RegionStamp, crate::layer_mask::LayerStamp),
    >,
    layer_reveal: Option<(crate::document::LayerId, crate::paint_target::Target, [i32; 2])>,
    navigator_paint: Option<crate::navigator::PaintRequest>,
    selection_drag: Option<crate::selection_ui::Drag>,
    selection_contour: crate::selection_contour::Cache,
    affine_preview: Option<crate::affine_preview::Preview>,
    mask_focus: Option<(crate::document::LayerId, crate::document::LayerId)>,
    stroke_preview: Option<crate::render_controller::StrokePreview>,
    geometry_preview: Option<crate::render_controller::GeometryPreview>,
    render_queue: crate::render_queue::Queue,
    queued_stroke: Option<crate::render_queue::StrokeId>,
    edit_feedback: std::collections::BTreeMap<u64, crate::render_controller::EditFeedback>,
    logical_layer: Option<crate::document::LayerId>,
    pending_layers: Option<crate::pending_layers::PendingLayers>,
    stroke_buffer: Option<crate::stroke_buffer::StrokeBuffer>,
    stroke_settings: Option<crate::stroke_buffer::Settings>,
    last_stroke_end: Option<crate::stroke::Sample>,
    stroke_previous_end: Option<crate::stroke::Sample>,
    zoom: f32,
    pan: egui::Vec2,
    rotation: f32,
    mirror: bool,
    auto_fit: bool,
    move_drag: Option<crate::canvas::MoveDrag>,
    pending_move: Option<crate::canvas::PendingMove>,
    deferred_move_ticket: Option<u64>,
    renaming: Option<usize>,
    rename_draft: String,
    opacity_edit: Option<crate::layers_panel::OpacityEdit>,
    opacity_preview_modified: Option<(u64, bool)>,
    effects_edit: Option<crate::effects_panel::Edit>,
    adjustment_edit: Option<crate::adjustment_panel::Edit>,
    adjustment_sent: Option<(crate::document::LayerId, crate::adjustment::Settings)>,
    effects_sent: Option<(crate::document::LayerId, crate::effects_panel::Style)>,
    effects_shown: Vec<u64>,
    mask_view_shown: bool,
    import_report: crate::import_report::ImportReport,
    symmetry: crate::symmetry::Symmetry,
    symmetry_drag: bool,
    layer_symmetry: std::collections::HashMap<crate::document::LayerId, crate::symmetry::Symmetry>,
    guides_locked: bool,
    grid: crate::guides::Grid,
    guide_lines: Vec<crate::guides::GuideLine>,
    show_rulers: bool,
    guide_drag: Option<crate::guides::GuideDrag>,
    rulers: Vec<crate::rulers::Ruler>,
    ruler_owners: Vec<Option<crate::document::LayerId>>,
    snapping: bool,
    snap_grid: bool,
    stroke_snap: Option<crate::rulers::StrokeSnap>,
    ruler_drag: Option<crate::rulers::RulerDrag>,
    saved_guides: crate::guides::Guides,
    observed_guides: crate::guides::Guides,
    observed_canvas_frame: crate::canvas_size::Frame,
    saving_guides: Option<crate::guides::Guides>,
    canvas_resize: Option<crate::canvas_size::Settings>,
    canvas_edit_ticket: Option<u64>,
    adjustment: Option<Box<crate::adjustments::Preview>>,
    adjustment_ticket: Option<u64>,
    retired_adjustment: Option<crate::snapshot_memory::SourceWorker<Result<crate::document::Document, String>>>,
    recovery_path: Option<PathBuf>,
    recovery_job: Option<crate::snapshot_memory::SourceWorker<Result<u64, String>>>,
    retired_recovery: Option<crate::snapshot_memory::SourceWorker<Result<(), String>>>,
    recovery_revision: u64,
    recovery_timer: std::time::Instant,
    recovery_saved: Option<std::time::Instant>,
    pending_clipboard: std::collections::BTreeMap<u64, crate::clipboard::Action>,
    paste_job: Option<crate::clipboard::PasteJob>,
    copy_job: Option<crate::clipboard::CopyJob>,
    clipboard_settlement: Option<crate::clipboard::ClipboardSettlement>,
    filter_job: Option<crate::jobs::FilterJob>,
    save_job: Option<crate::save_jobs::SaveJob>,
    queued_save: Option<crate::save_jobs::QueuedSave>,
    export_job: Option<crate::export_jobs::ExportJob>,
    queued_export: Option<crate::export_jobs::QueuedExport>,
    indexed_session: Option<crate::indexed_commit::Session>,
    indexed_pending: Option<crate::save_jobs::IndexedPending>,
    indexed_epoch: u64,
    load_job: Option<crate::load_jobs::LoadJob>,
    pending_action: Option<crate::document_actions::Action>,
    references: crate::references::References,
    reference_layers: crate::reference_layers::State,
    vector_pick: crate::vector_pick::Pick,
    text_edit: Option<crate::text_tool::Editing>,
    text_drag: Option<crate::text_tool::Drag>,
    held_transform: Option<crate::affine_preview::HeldTransform>,
    layer_navigation: crate::layer_navigation::Navigation,
    layer_selection: crate::layer_batch::Selection,
    recording: crate::timelapse::Recording,
}

pub(crate) const DEFAULT_MAX: usize = 8;
pub(crate) const LIMIT: usize = 32;

/// Owned CPU allocation estimates and canvas RGBA payloads, not process or VRAM
/// commitment. One registry deduplicates shared pixels across every open project.
#[derive(Default, serde::Serialize)]
pub(crate) struct OpenProjectsMemory {
    pub projects: usize,
    pub tracked_cpu_bytes: usize,
    pub canvas_rgba_bytes: usize,
    pub auxiliary_rgba_bytes: usize,
}
type OpenProject = Box<dyn FnOnce(&mut Dudel)>;

pub(crate) struct Tab {
    pub(crate) id: u64,
    /// None for the active tab, whose project is in `Dudel`'s fields.
    session: Option<Box<Session>>,
}

pub(crate) struct Tabs {
    pub(crate) list: Vec<Tab>,
    pub(crate) active: usize,
    next_id: u64,
    pub(crate) max: usize,
    /// Tab waiting to become active once the current project settles.
    request: Option<Request>,
    /// Run only after the outgoing project's recovery snapshot is settled.
    pending_open: Option<OpenProject>,
    /// Tab to return to if the project opening in a new tab fails.
    return_to: Option<u64>,
    /// Recovery file name of the first tab, which later tabs extend.
    recovery_base: Option<PathBuf>,
    /// Ctrl+Tab steps, taken from raw input.
    pub(crate) cycle: i32,
}
struct Request {
    target: u64,
    /// Ask to close the target once it is active.
    close: bool,
}
impl Default for Tabs {
    fn default() -> Self {
        Self {
            list: vec![Tab {
                id: 0,
                session: None,
            }],
            active: 0,
            next_id: 1,
            max: DEFAULT_MAX,
            request: None,
            pending_open: None,
            return_to: None,
            recovery_base: None,
            cycle: 0,
        }
    }
}
impl Tabs {
    fn index(&self, id: u64) -> Option<usize> {
        self.list.iter().position(|t| t.id == id)
    }
    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }
}

/// Remove a parked project's recovery snapshot when it is closed unsaved-free.
fn remove_recovery(session: &Session) {
    if let Some(path) = &session.recovery_path {
        let _ = std::fs::remove_file(path);
    }
}

impl Dudel {
    pub(crate) fn open_projects_memory(&self) -> OpenProjectsMemory {
        let mut registry = crate::snapshot_memory::Registry::new(&self.doc);
        self.register_project_extras(&mut registry);
        let mut memory = OpenProjectsMemory {
            projects: self.tabs.len(),
            canvas_rgba_bytes: self.texture.memory().texture_base_rgba_bytes,
            ..Default::default()
        };
        let mut textures = std::collections::HashSet::new();
        let mut add_texture = |texture: &egui::TextureHandle| {
            if textures.insert(texture.id()) {
                memory.auxiliary_rgba_bytes += texture.size()[0] * texture.size()[1] * 4;
            }
        };
        for texture in self
            .layer_thumbnails
            .values()
            .chain(self.mask_thumbnails.values())
        {
            add_texture(texture);
        }
        self.references.visit_textures(&mut add_texture);
        self.reference_layers.visit_textures(&mut add_texture);
        for session in self.tabs.list.iter().filter_map(|tab| tab.session.as_ref()) {
            registry.add_document(&session.doc);
            session.register_extras(&mut registry);
            memory.canvas_rgba_bytes += session.texture.memory().texture_base_rgba_bytes;
            for texture in session
                .layer_thumbnails
                .values()
                .chain(session.mask_thumbnails.values())
            {
                add_texture(texture);
            }
            session.references.visit_textures(&mut add_texture);
            session.reference_layers.visit_textures(&mut add_texture);
        }
        memory.tracked_cpu_bytes = registry.live_bytes() + registry.additional_bytes();
        memory
    }

    /// Name and unsaved state of tab `index`.
    pub(crate) fn tab_title(&self, index: usize) -> (String, bool) {
        let tab = &self.tabs.list[index];
        let (path, modified) = match &tab.session {
            None => (self.path.as_deref(), self.modified),
            Some(session) => (session.path.as_deref(), session.modified),
        };
        let name = path
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if tab.id == 0 {
                    "Untitled".into()
                } else {
                    format!("Untitled {}", tab.id + 1)
                }
            });
        (name, modified)
    }
    /// An untouched new canvas, which an opened project may simply replace.
    fn active_tab_blank(&self) -> bool {
        self.path.is_none()
            && !self.modified
            && !self.history.can_undo()
            && !self.history.can_redo()
            && self.load_job.is_none()
    }
    /// Why the active project cannot be parked yet, or None when it can.
    fn tab_switch_blocker(&self) -> Option<&'static str> {
        if self.stroking || self.render_queue.busy() || self.stroke_buffer.is_some() {
            return Some("painting");
        }
        if self.save_job.is_some() || self.queued_save.is_some() || self.indexed_pending.is_some() {
            return Some("saving");
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            return Some("exporting");
        }
        if self.load_job.is_some() {
            return Some("loading");
        }
        if self.filter_job.is_some()
            || self.canvas_edit_ticket.is_some()
            || self.adjustment_ticket.is_some()
            || self.retired_adjustment.is_some()
            || self.deferred_move_ticket.is_some()
            || self.pending_move.is_some()
        {
            return Some("applying an edit");
        }
        if self.paste_job.is_some()
            || self.copy_job.is_some()
            || self.clipboard_settlement.is_some()
            || !self.pending_clipboard.is_empty()
        {
            return Some("using the clipboard");
        }
        if self.recovery_job.is_some() || self.retired_recovery.is_some() {
            return Some("writing a recovery snapshot");
        }
        if self.reference_layers.importing() {
            return Some("importing references");
        }
        None
    }
    /// Work only the user can finish; a switch request is dropped instead of waiting.
    fn tab_switch_needs_user(&self) -> Option<&'static str> {
        if self.affine_preview.is_some() {
            Some("Apply or cancel the transform before switching projects")
        } else if self.adjustment.is_some() {
            Some("Apply or cancel the adjustment before switching projects")
        } else if self.canvas_resize.is_some() {
            Some("Finish the canvas size change before switching projects")
        } else if self.unsaved_action.is_some() || self.dialogs.open() {
            Some("Answer the open dialog before switching projects")
        } else if self.move_drag.is_some() {
            Some("Finish moving before switching projects")
        } else {
            None
        }
    }
    /// Make tab `index` active as soon as the current project settles.
    pub(crate) fn request_tab(&mut self, index: usize) {
        self.request_tab_then(index, false);
    }
    fn request_tab_then(&mut self, index: usize, close: bool) {
        if self.tabs.pending_open.is_some() {
            self.status = "A project is waiting to open after recovery finishes".into();
            return;
        }
        let Some(tab) = self.tabs.list.get(index) else {
            return;
        };
        if index == self.tabs.active {
            if close {
                self.request_replace(crate::document_actions::Action::CloseTab);
            }
            return;
        }
        if let Some(reason) = self.tab_switch_needs_user() {
            self.status = reason.into();
            return;
        }
        self.tabs.request = Some(Request {
            target: tab.id,
            close,
        });
        self.process_tab_request();
    }
    /// Called each frame: complete a waiting switch once nothing is in flight.
    pub(crate) fn process_tab_request(&mut self) {
        if let Some(open) = self.tabs.pending_open.take() {
            self.finish_tab_open(open);
            return;
        }
        if self.tabs.cycle != 0 && self.tabs.len() > 1 {
            let n = self.tabs.len() as i32;
            let next = (self.tabs.active as i32 + self.tabs.cycle).rem_euclid(n) as usize;
            self.tabs.cycle = 0;
            self.request_tab(next);
            return;
        }
        self.tabs.cycle = 0;
        let Some(request) = &self.tabs.request else {
            return;
        };
        let close = request.close;
        let Some(index) = self.tabs.index(request.target) else {
            self.tabs.request = None;
            return;
        };
        if let Some(reason) = self.tab_switch_needs_user() {
            self.tabs.request = None;
            self.status = reason.into();
            return;
        }
        if !self.tab_ready_to_park() {
            return;
        }
        self.tabs.request = None;
        self.switch_tab_now(index);
        if close {
            self.request_replace(crate::document_actions::Action::CloseTab);
        }
    }
    /// Park the active project and bring tab `index` forward. Callers ensure
    /// the active project is settled.
    fn switch_tab_now(&mut self, index: usize) {
        self.finish_layer_rename(true);
        self.cancel_gestures();
        let mut incoming = self.tabs.list[index]
            .session
            .take()
            .expect("inactive tab holds its project");
        incoming.swap(self);
        let previous = self.tabs.active;
        self.tabs.list[previous].session = Some(incoming);
        self.tabs.active = index;
        // Display tiles are kept only for the tab just left, so switching
        // back and forth is instant while other parked tabs hold no tiles.
        for (i, tab) in self.tabs.list.iter_mut().enumerate() {
            if i != previous
                && let Some(session) = &mut tab.session
            {
                session.texture = Default::default();
                session.dirty = true;
            }
        }
        self.after_project_change();
        self.status = format!("Project: {}", self.tab_title(index).0);
    }
    /// Reset app-wide state that referred to the previous project.
    fn after_project_change(&mut self) {
        if matches!(self.fill_source, crate::fill::Source::Layer(_)) {
            self.fill_source = crate::fill::Source::Active;
        }
        if matches!(self.selection_wand.source, crate::fill::Source::Layer(_)) {
            self.selection_wand.source = crate::fill::Source::Active;
        }
        self.preview.reset();
        self.preview.hidden.clear();
        self.track_guide_changes();
    }
    /// Opening a project must not strand unsnapshotted artwork in a parked tab.
    /// The callback runs after recovery settles, without blocking the UI thread.
    pub(crate) fn open_in_tab(&mut self, open: impl FnOnce(&mut Dudel) + 'static) {
        if self.tabs.pending_open.is_some() {
            self.status = "A project is already waiting to open".into();
            return;
        }
        self.tabs.request = None;
        self.finish_tab_open(Box::new(open));
    }

    pub(crate) fn cancel_pending_tab_change(&mut self) {
        self.tabs.pending_open = None;
        self.tabs.request = None;
    }

    pub(crate) fn tab_open_pending(&self) -> bool {
        self.tabs.pending_open.is_some()
    }

    fn tab_ready_to_park(&mut self) -> bool {
        if let Some(reason) = self.tab_switch_blocker() {
            self.status = format!("Changing projects after {reason} finishes");
            return false;
        }
        // These commits may queue an edit; wait for its settlement before taking
        // a snapshot. No gesture or worker is finished by a blocking join here.
        self.finish_layer_rename(true);
        self.finish_opacity_edit();
        self.finish_effects_edit();
        self.finish_adjustment_edit();
        self.track_guide_changes();
        if let Some(reason) = self.tab_switch_blocker() {
            self.status = format!("Changing projects after {reason} finishes");
            return false;
        }
        if self.modified && self.revision != self.recovery_revision && self.recovery_path.is_some()
        {
            self.recovery_timer = std::time::Instant::now() - std::time::Duration::from_secs(60);
            self.status = "Preparing recovery before changing projects".into();
            return false;
        }
        true
    }

    fn finish_tab_open(&mut self, open: OpenProject) {
        if self.close_ready {
            return;
        }
        self.track_guide_changes();
        // Reusing the active tab goes through its existing unsaved-work prompt.
        if self.active_tab_blank() || self.tabs.len() >= self.tabs.max {
            open(self);
            return;
        }
        if let Some(reason) = self.tab_switch_needs_user() {
            self.status = reason.into();
            return;
        }
        if !self.tab_ready_to_park() {
            self.tabs.pending_open = Some(open);
            return;
        }
        if self.prepare_tab_for_open() {
            open(self);
        }
    }

    /// Immediate exchange, used after `open_in_tab` settles recovery, and by
    /// synthetic capture/test setup that has no user recovery to preserve.
    pub(crate) fn prepare_tab_for_open(&mut self) -> bool {
        if self.active_tab_blank() {
            return true;
        }
        if self.tabs.len() >= self.tabs.max {
            // At the tab limit the current project is replaced, asking about
            // unsaved changes first; a limit of one is single-project mode.
            return true;
        }
        if let Some(reason) = self.tab_switch_needs_user() {
            self.status = reason.into();
            return false;
        }
        if let Some(reason) = self.tab_switch_blocker() {
            self.status = format!("Wait for {reason} to finish before opening another project");
            return false;
        }
        let returning = self.tabs.list[self.tabs.active].id;
        self.new_tab_now();
        self.tabs.return_to = Some(returning);
        true
    }
    fn new_tab_now(&mut self) {
        let id = self.tabs.next_id;
        self.tabs.next_id += 1;
        let mut session = Session::blank();
        if self.tabs.recovery_base.is_none() {
            self.tabs.recovery_base = self.recovery_path.clone();
        }
        session.recovery_path = self.tabs.recovery_base.as_ref().map(|base| {
            let stem = base.file_stem().unwrap_or_default().to_string_lossy();
            base.with_file_name(format!("{stem}-tab{id}.gz"))
        });
        self.tabs.list.insert(
            self.tabs.active + 1,
            Tab {
                id,
                session: Some(session),
            },
        );
        self.switch_tab_now(self.tabs.active + 1);
    }
    /// The tab already showing `path`, if any.
    pub(crate) fn tab_with_path(&self, path: &std::path::Path) -> Option<usize> {
        let same = |other: &std::path::Path| {
            other == path
                || matches!(
                    (other.canonicalize(), path.canonicalize()),
                    (Ok(a), Ok(b)) if a == b
                )
        };
        (0..self.tabs.len()).find(|&i| {
            let path = match &self.tabs.list[i].session {
                None => self.path.as_deref(),
                Some(session) => session.path.as_deref(),
            };
            path.is_some_and(same)
        })
    }
    /// After a load finishes: a tab opened for it that is still blank failed,
    /// so it closes and the previous project comes back.
    pub(crate) fn settle_tab_after_load(&mut self) {
        let Some(previous) = self.tabs.return_to.take() else {
            return;
        };
        if !self.active_tab_blank() || self.tabs.len() < 2 {
            return;
        }
        if let Some(index) = self.tabs.index(previous) {
            let status = std::mem::take(&mut self.status);
            self.remove_active_tab(Some(index));
            self.status = status;
        }
    }
    /// Close tab `index`, asking about unsaved changes first.
    pub(crate) fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if index == self.tabs.active {
            self.request_replace(crate::document_actions::Action::CloseTab);
            return;
        }
        let modified = self.tabs.list[index]
            .session
            .as_ref()
            .is_some_and(|s| s.modified);
        if modified {
            // Show the project, then ask about it.
            self.request_tab_then(index, true);
        } else {
            let tab = self.tabs.list.remove(index);
            if let Some(session) = &tab.session {
                remove_recovery(session);
            }
            if index < self.tabs.active {
                self.tabs.active -= 1;
            }
        }
    }
    /// Discard the active project and show `next` (a neighbor when None).
    /// The last tab is replaced by a new blank canvas instead.
    pub(crate) fn remove_active_tab(&mut self, next: Option<usize>) {
        let active = self.tabs.active;
        if self.tabs.len() == 1 {
            let mut blank = Session::blank();
            blank.recovery_path = self.recovery_path.clone();
            blank.swap(self);
            self.after_project_change();
            self.status = "Project closed".into();
            return;
        }
        let next = next
            .filter(|&i| i != active && i < self.tabs.len())
            .unwrap_or(if active + 1 < self.tabs.len() {
                active + 1
            } else {
                active - 1
            });
        let mut incoming = self.tabs.list[next]
            .session
            .take()
            .expect("inactive tab holds its project");
        incoming.swap(self);
        // `incoming` now holds the closed project; dropping it releases it.
        drop(incoming);
        self.tabs.list.remove(active);
        self.tabs.active = if next > active { next - 1 } else { next };
        self.after_project_change();
        let title = self.tab_title(self.tabs.active).0;
        self.status = format!("Project closed · now showing {title}");
    }
    /// For quitting: show the next parked project with unsaved changes after
    /// discarding the active one. False when none remain.
    pub(crate) fn next_unsaved_tab_for_close(&mut self) -> bool {
        let Some(index) = (0..self.tabs.len()).find(|&i| {
            i != self.tabs.active
                && self.tabs.list[i]
                    .session
                    .as_ref()
                    .is_some_and(|s| s.modified)
        }) else {
            return false;
        };
        self.remove_active_tab(Some(index));
        true
    }
    /// For quitting: forget recovery snapshots of parked projects without changes.
    pub(crate) fn clear_parked_recovery(&mut self) {
        for tab in &self.tabs.list {
            if let Some(session) = &tab.session {
                remove_recovery(session);
            }
        }
    }
    pub(crate) fn tab_bar(&mut self, ctx: &egui::Context) {
        if self.tabs.len() < 2 {
            return;
        }
        let mut select = None;
        let mut close = None;
        egui::TopBottomPanel::top("project tabs")
            .frame(
                egui::Frame::NONE
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(6, 3)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("project tab strip")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for index in 0..self.tabs.len() {
                                let (name, modified) = self.tab_title(index);
                                let active = index == self.tabs.active;
                                // The bundled font has no bullet glyph; editors use a trailing star.
                                let label = if modified {
                                    format!("{name} *")
                                } else {
                                    name.clone()
                                };
                                let path = match &self.tabs.list[index].session {
                                    None => self.path.clone(),
                                    Some(session) => session.path.clone(),
                                };
                                let (response, close_clicked) = project_tab(
                                    ui,
                                    egui::Id::new(("project tab", index)),
                                    &label,
                                    active,
                                );
                                if close_clicked {
                                    close = Some(index);
                                }
                                let response = response.on_hover_text(match &path {
                                    Some(path) => format!(
                                        "{}{}",
                                        path.display(),
                                        if modified { "\nUnsaved changes" } else { "" }
                                    ),
                                    None => "Not saved yet".into(),
                                });
                                let recording = match &self.tabs.list[index].session {
                                    None => self.recording.recording(),
                                    Some(session) => session.recording.recording(),
                                };
                                if recording {
                                    ui.painter().circle_filled(
                                        response.rect.right_top() + egui::vec2(-4.0, 4.0),
                                        3.0,
                                        egui::Color32::from_rgb(220, 60, 60),
                                    );
                                }
                                let response = if recording {
                                    response.on_hover_text("Recording a timelapse")
                                } else {
                                    response
                                };
                                if active && self.tabs.request.is_none() {
                                    response.scroll_to_me(None);
                                }
                                if response.clicked() && !close_clicked {
                                    select = Some(index);
                                }
                                if response.middle_clicked() {
                                    close = Some(index);
                                }
                                response.context_menu(|ui| {
                                    if ui.button("Close").clicked() {
                                        close = Some(index);
                                        ui.close();
                                    }
                                    if let Some(folder) =
                                        path.as_deref().and_then(std::path::Path::parent)
                                        && ui.button("Show in folder").clicked()
                                    {
                                        if let Err(error) = crate::locations::reveal(folder) {
                                            self.status = error;
                                        }
                                        ui.close();
                                    }
                                });
                                ui.add_space(4.0);
                            }
                            if crate::icons::button(
                                ui,
                                crate::icons::Icon::Plus,
                                TAB_HEIGHT,
                                false,
                                true,
                            )
                            .on_hover_text(self.command_hint(
                                crate::commands::Action::New,
                                "New canvas in a new tab",
                            ))
                            .clicked()
                            {
                                self.open_new_canvas();
                            }
                        });
                    });
            });
        if let Some(index) = close {
            self.close_tab(index);
        } else if let Some(index) = select {
            self.request_tab(index);
        }
    }
}

const TAB_HEIGHT: f32 = 26.0;

/// One project tab: the name and its close target drawn as a single shape, so
/// the close control lines up inside the tab instead of floating beside it.
/// Returns the tab's response and whether its close target was clicked.
fn project_tab(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    active: bool,
) -> (egui::Response, bool) {
    const PAD: f32 = 10.0;
    const CLOSE: f32 = 18.0;
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        180.0,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(PAD + galley.size().x + 4.0 + CLOSE + 4.0, TAB_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 4.0 - CLOSE / 2.0, rect.center().y),
        egui::Vec2::splat(CLOSE),
    );
    // Registered after the tab, so it receives clicks on top of it.
    let close = ui.interact(close_rect, id.with("close"), egui::Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let widget = if response.hovered() || close.hovered() {
            &visuals.widgets.hovered
        } else {
            &visuals.widgets.inactive
        };
        let (fill, text) = if active {
            (visuals.selection.bg_fill, visuals.selection.stroke.color)
        } else {
            (widget.weak_bg_fill, widget.fg_stroke.color)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, widget.corner_radius, fill);
        painter.galley(
            egui::pos2(rect.left() + PAD, rect.center().y - galley.size().y / 2.0),
            galley,
            text,
        );
        if close.hovered() {
            painter.circle_filled(
                close_rect.center(),
                CLOSE / 2.0,
                visuals.widgets.active.weak_bg_fill,
            );
        }
        // Visible on the active or hovered tab; faint elsewhere so names stay readable.
        let color = if active || response.hovered() || close.hovered() {
            text
        } else {
            text.gamma_multiply(0.45)
        };
        let arm = 3.5;
        let c = close_rect.center();
        let stroke = egui::Stroke::new(1.5_f32, color);
        painter.line_segment(
            [c + egui::vec2(-arm, -arm), c + egui::vec2(arm, arm)],
            stroke,
        );
        painter.line_segment(
            [c + egui::vec2(-arm, arm), c + egui::vec2(arm, -arm)],
            stroke,
        );
    }
    let clicked = close.on_hover_text("Close tab").clicked();
    (response, clicked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_project_memory_counts_shared_references_once_and_releases_closed_tabs() {
        let mut app = Dudel {
            doc: crate::document::Document::new(256, 256),
            path: Some("memory-first.dudel".into()),
            ..Default::default()
        };
        app.history.checkpoint(&app.doc);
        paint(&mut app, [200, 40, 70, 255]);
        app.history.settle(&app.doc);
        app.insert_reference(
            "Study".into(),
            crate::reference_layers::Image {
                width: 256,
                height: 256,
                rgba: [20, 160, 70, 255].repeat(256 * 256),
            },
        );
        app.reference_layers.active = None;
        let reference = std::sync::Arc::downgrade(&app.reference_layers.layers[0].image);
        app.place_reference(app.reference_layers.layers[0].id, 0, 0);
        app.reference_layers.active = None;
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.prepare_reference_layers(ctx));
        let first = app.open_projects_memory();
        assert_eq!(first.projects, 1);
        assert!(first.tracked_cpu_bytes >= 2 * 256 * 256 * 4);
        // An on-canvas reference uses its existing 128-square thumbnail texture.
        assert!(first.auxiliary_rgba_bytes >= 128 * 128 * 4);
        assert!(app.prepare_tab_for_open());
        app.doc = crate::document::Document::new(256, 256);
        app.path = Some("memory-second.dudel".into());
        let shared = app.tabs.list[0]
            .session
            .as_ref()
            .unwrap()
            .reference_layers
            .layers
            .clone();
        app.reference_layers.install(shared);
        let shared_bytes = app.open_projects_memory().tracked_cpu_bytes;
        let image = &mut app.reference_layers.layers[0].image;
        *image = std::sync::Arc::new(crate::reference_layers::Image {
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
        });
        assert_eq!(
            app.open_projects_memory().tracked_cpu_bytes - shared_bytes,
            256 * 256 * 4
        );
        app.reference_layers.install(Vec::new());
        let two = app.open_projects_memory();
        assert_eq!(two.projects, 2);
        assert!(two.tracked_cpu_bytes >= first.tracked_cpu_bytes);
        app.request_tab(0);
        let switched = app.open_projects_memory();
        assert_eq!(switched.tracked_cpu_bytes, two.tracked_cpu_bytes);
        assert_eq!(switched.auxiliary_rgba_bytes, two.auxiliary_rgba_bytes);
        app.request_tab(1);
        app.tabs.list[0].session.as_mut().unwrap().modified = false;
        app.close_tab(0);
        let closed = app.open_projects_memory();
        assert_eq!(closed.projects, 1);
        assert!(closed.tracked_cpu_bytes < two.tracked_cpu_bytes);
        assert_eq!(closed.auxiliary_rgba_bytes, 0);
        assert!(
            reference.upgrade().is_none(),
            "accounting must not retain closed references"
        );
    }
    fn poll_tab_recovery(app: &mut Dudel, done: impl Fn(&Dudel) -> bool) {
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(app) {
            assert!(std::time::Instant::now() < deadline, "{}", app.status);
            let _ = ctx.run(Default::default(), |ctx| {
                app.pump_render_queue(ctx);
                app.process_tab_request();
                app.poll_recovery(ctx);
            });
            std::thread::yield_now();
        }
    }

    #[test]
    fn new_tab_waits_for_recovery_of_pixels_masks_guides_and_folder_references() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("outgoing.gz");
        let original = directory.path().join("original.dudel");
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            path: Some(original.clone()),
            recovery_path: Some(path.clone()),
            ..Default::default()
        };
        paint(&mut app, [17, 83, 201, 255]);
        app.doc.version = 2;
        app.doc.layers[0].depth = 1;
        app.doc
            .layers
            .push(crate::document::Layer::new_folder("Studies".into()));
        let mut coverage = crate::layer_mask::Mask::new(16, 16, 255).unwrap();
        coverage.writer().set(0, 0, 37);
        app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage,
        });
        app.grid.visible = true;
        app.grid.spacing = 64;
        app.reference_layers
            .layers
            .push(crate::reference_layers::ReferenceLayer {
                id: 1,
                name: "Hair".into(),
                center: [4.0, 5.0],
                scale: 1.0,
                rotation: 0.0,
                opacity: 1.0,
                visible: true,
                locked: false,
                above: true,
                flip: false,
                images: Vec::new(),
                position: None,
                image: std::sync::Arc::new(crate::reference_layers::Image {
                    width: 2,
                    height: 2,
                    rgba: [9, 11, 13, 255].repeat(4),
                }),
            });
        app.place_reference(1, 1, 1);
        app.renaming = Some(0);
        app.rename_draft = "Final ink".into();
        app.doc.validate().unwrap();
        let recovery_for_callback = path.clone();
        app.open_in_tab(move |app| {
            assert!(
                recovery_for_callback.is_file(),
                "snapshot must precede parking"
            );
            app.request_replace(crate::document_actions::Action::New(8, 8, 300.0));
        });
        assert_eq!(app.tabs.len(), 1);
        assert!(app.tab_open_pending());
        // Dropped files must remain queued while this open waits for recovery.
        let dropped = directory.path().join("next.dudel");
        app.pending_opens.push(dropped.clone());
        app.open_pending_projects();
        assert_eq!(app.pending_opens, [dropped]);
        poll_tab_recovery(&mut app, |app| !app.tab_open_pending());
        assert_eq!(app.tabs.len(), 2, "{}", app.status);
        assert_eq!(app.tabs.active, 1);
        assert_eq!(app.doc.width, 8);
        let recovered = crate::storage::read_recovery(&path).unwrap();
        assert_eq!(recovered.original_path, Some(original));
        assert_eq!(recovered.document.layers[0].name, "Final ink");
        assert_eq!(
            recovered.document.layers[0].pixels.rgba(0),
            [17, 83, 201, 255]
        );
        assert_eq!(
            recovered.document.layers[0]
                .mask
                .as_ref()
                .unwrap()
                .coverage
                .at(0, 0),
            37
        );
        assert_eq!(recovered.document.guides.unwrap().grid.spacing, 64);
        assert_eq!(recovered.document.references.len(), 1);
        let reference = &recovered.document.references[0];
        assert_eq!(reference.image.rgba, [9, 11, 13, 255].repeat(4));
        assert_eq!(reference.position.as_ref().unwrap().depth, 1);
        assert_eq!(reference.position.as_ref().unwrap().gap, 1);
        assert!(
            app.tabs.list[0]
                .session
                .as_ref()
                .unwrap()
                .recovery_job
                .is_none()
        );
        assert!(path.exists(), "parked artwork keeps its completed snapshot");
    }

    #[test]
    fn failed_snapshot_cancels_tab_navigation_without_parking_or_replacing_artwork() {
        for opening in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = Dudel {
                doc: crate::document::Document::new(16, 16),
                path: Some(directory.path().join("first.dudel")),
                recovery_path: Some(directory.path().join("failed.gz")),
                ..Default::default()
            };
            if !opening {
                assert!(app.prepare_tab_for_open());
            }
            let active = app.tabs.active;
            let count = app.tabs.len();
            paint(&mut app, [1, 2, 3, 255]);
            app.recovery_job = Some(crate::snapshot_memory::SourceWorker::spawn(
                app.doc.clone(),
                |_| Err("injected write failure".into()),
            ));
            if opening {
                app.open_in_tab(|_| panic!("failed recovery must not open another project"));
            } else {
                app.request_tab(0);
            }
            poll_tab_recovery(&mut app, |app| {
                !app.tab_open_pending() && app.tabs.request.is_none()
            });
            assert_eq!(app.tabs.len(), count);
            assert_eq!(app.tabs.active, active);
            assert_eq!(app.doc.layers[0].pixels.rgba(0), [1, 2, 3, 255]);
            assert!(app.modified);
            assert!(app.status.contains("injected write failure"));
        }
    }

    #[test]
    fn closing_cancels_a_pending_open_and_waits_for_snapshot_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("closing.gz");
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            recovery_path: Some(path.clone()),
            ..Default::default()
        };
        paint(&mut app, [5, 6, 7, 255]);
        let (send, receive) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        app.recovery_job = Some(crate::snapshot_memory::SourceWorker::spawn(
            app.doc.clone(),
            move |document| {
                receive
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
                crate::storage::save_recovery(
                    &worker_path,
                    &crate::storage::Recovery {
                        document,
                        original_path: None,
                    },
                )?;
                Ok(1)
            },
        ));
        app.open_in_tab(|_| panic!("closed project must not open the queued project"));
        assert!(app.tab_open_pending());
        assert!(!app.recovery_job.as_ref().unwrap().is_finished());
        app.apply_action(crate::document_actions::Action::Close);
        assert!(!app.tab_open_pending());
        assert!(app.close_ready);
        assert!(app.retired_recovery.is_some());
        send.send(()).unwrap();
        poll_tab_recovery(&mut app, |app| app.retired_recovery.is_none());
        assert!(
            !path.exists(),
            "late recovery writer must finish before cleanup"
        );
        assert_eq!(app.tabs.len(), 1);
    }

    fn paint(app: &mut Dudel, rgba: [u8; 4]) {
        let w = app.doc.width;
        app.doc.layers[0]
            .pixels
            .writer()
            .write_region(w, [0, 0, 2, 2], &rgba.repeat(4));
        app.changed();
    }
    #[test]
    fn tabs_keep_their_own_layer_filter_and_multi_selection() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            ..Default::default()
        };
        paint(&mut app, [255, 0, 0, 255]);
        let layer = app.doc.layers[0].id;
        app.layer_navigation.query = "ink".into();
        app.layer_selection.ids = vec![layer];
        assert!(app.prepare_tab_for_open());
        assert!(app.layer_navigation.query.is_empty());
        assert!(app.layer_selection.ids.is_empty());
        app.layer_navigation.query = "sketch".into();
        app.request_tab(0);
        assert_eq!(app.layer_navigation.query, "ink");
        assert_eq!(app.layer_selection.ids, [layer]);
        app.request_tab(1);
        assert_eq!(app.layer_navigation.query, "sketch");
    }
    #[test]
    fn tabs_keep_independent_documents_history_views_and_paths() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            path: Some("first.dudel".into()),
            ..Default::default()
        };
        app.zoom = 2.0;
        paint(&mut app, [255, 0, 0, 255]);
        assert!(app.prepare_tab_for_open());
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs.active, 1);
        assert!(app.path.is_none() && !app.modified);
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [0, 0, 0, 0]);
        // An untouched tab is reused rather than opening a third.
        assert!(app.prepare_tab_for_open());
        assert_eq!(app.tabs.len(), 2);
        app.path = Some("second.dudel".into());
        app.zoom = 0.5;
        app.request_tab(0);
        assert_eq!(app.tabs.active, 0);
        assert_eq!(
            app.path.as_deref(),
            Some(std::path::Path::new("first.dudel"))
        );
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [255, 0, 0, 255]);
        assert_eq!(app.zoom, 2.0);
        assert!(app.modified);
        assert_eq!(app.tab_title(1), ("second".into(), false));
        assert_eq!(
            app.tab_with_path(std::path::Path::new("second.dudel")),
            Some(1)
        );
        app.tabs.cycle = 1;
        app.process_tab_request();
        assert_eq!(app.tabs.active, 1);
        assert_eq!(app.zoom, 0.5);
    }
    #[test]
    fn tab_navigation_preserves_project_symmetry_snapping_and_saved_state() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            path: Some("first.dudel".into()),
            ..Default::default()
        };
        let first = crate::guides::Guides {
            symmetry: crate::symmetry::Symmetry {
                mode: crate::symmetry::Mode::Radial,
                segments: 8,
                center: Some([4.0, 7.0]),
                angle: 30.0,
            },
            snapping: false,
            snap_grid: true,
            ..Default::default()
        };
        app.apply_guides(Some(Box::new(first.clone())));
        assert!(app.prepare_tab_for_open());
        assert_eq!(
            app.guides_record(&app.doc),
            crate::guides::Guides::default()
        );
        assert!(
            !app.modified,
            "opening a blank tab must not edit its guides"
        );
        let second = crate::guides::Guides {
            symmetry: crate::symmetry::Symmetry {
                mode: crate::symmetry::Mode::TopBottom,
                ..Default::default()
            },
            ..Default::default()
        };
        app.apply_guides(Some(Box::new(second.clone())));
        app.path = Some("second.dudel".into());
        for index in [0, 1, 0, 1] {
            app.request_tab(index);
            assert_eq!(app.tabs.active, index);
            assert_eq!(
                app.guides_record(&app.doc),
                if index == 0 {
                    first.clone()
                } else {
                    second.clone()
                }
            );
            assert!(
                !app.modified && app.revision == 0,
                "tab navigation must not create edits"
            );
        }
        app.close_tab(1);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.guides_record(&app.doc), first);
        assert!(!app.modified && app.unsaved_action.is_none());
        app.close_tab(0);
        assert_eq!(
            app.guides_record(&app.doc),
            crate::guides::Guides::default()
        );
        assert!(!app.modified && app.path.is_none());
    }

    #[test]
    fn switching_waits_for_rendering_and_refuses_open_transforms() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            ..Default::default()
        };
        paint(&mut app, [1, 2, 3, 255]);
        assert!(app.prepare_tab_for_open());
        app.stroking = true;
        app.request_tab(0);
        assert_eq!(app.tabs.active, 1, "waits while painting");
        assert!(app.status.contains("painting"));
        app.stroking = false;
        app.process_tab_request();
        assert_eq!(app.tabs.active, 0);
        // At the tab limit the current tab is reused (with the usual prompt).
        app.tabs.max = 2;
        assert!(app.prepare_tab_for_open());
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs.active, 0);
    }
    #[test]
    fn closing_tabs_prompts_only_for_unsaved_work_and_keeps_one_canvas() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            ..Default::default()
        };
        paint(&mut app, [9, 9, 9, 255]);
        assert!(app.prepare_tab_for_open());
        app.path = Some("clean.dudel".into());
        assert!(app.prepare_tab_for_open());
        assert_eq!(app.tabs.len(), 3);
        // A clean parked tab closes without a question.
        app.request_tab(0);
        assert_eq!(app.tabs.active, 0);
        app.close_tab(1);
        assert_eq!(app.tabs.len(), 2);
        assert!(app.unsaved_action.is_none());
        // The modified active tab asks first; discarding shows its neighbor.
        app.close_tab(0);
        assert!(app.unsaved_action.is_some());
        let action = app.unsaved_action.take().unwrap();
        app.apply_action(action);
        assert_eq!(app.tabs.len(), 1);
        assert!(!app.modified);
        // Closing the last tab leaves a blank canvas.
        app.path = Some("last.dudel".into());
        app.close_tab(0);
        assert_eq!(app.tabs.len(), 1);
        assert!(app.path.is_none());
    }
    #[test]
    fn quitting_visits_each_unsaved_tab_before_closing() {
        let mut app = Dudel {
            doc: crate::document::Document::new(16, 16),
            ..Default::default()
        };
        paint(&mut app, [5, 5, 5, 255]);
        assert!(app.prepare_tab_for_open());
        paint(&mut app, [6, 6, 6, 255]);
        assert!(!app.request_replace(crate::document_actions::Action::Close));
        let action = app.unsaved_action.take().unwrap();
        app.apply_action(action);
        // The other unsaved project is shown and asked about next.
        assert!(!app.close_ready);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [5, 5, 5, 255]);
        let action = app.unsaved_action.take().expect("second prompt");
        app.apply_action(action);
        assert!(app.close_ready);
    }
}
