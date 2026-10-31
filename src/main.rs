#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]

mod adjustment;
mod adjustment_panel;
mod adjustments;
mod affine;
mod affine_benchmark;
mod affine_preview;
#[cfg(feature = "allocation-profile")]
mod allocation_profile;
#[cfg(all(feature = "allocation-stack-profile", windows))]
mod allocation_trace;
mod benchmark;
mod binary_project;
mod blend;
mod brush;
mod brush_edge;
mod brush_library;
mod brush_pack;
mod brush_previews;
mod brush_studio;
mod canvas;
mod canvas_size;
mod canvas_textures;
mod capture;
mod clip_attributes;
mod clip_blocks;
mod clip_container;
mod clip_document;
mod clip_metadata;
mod clip_preview;
mod clip_raster;
mod clip_timeline;
mod clip_vector;
mod clipboard;
#[cfg(windows)]
mod clipboard_files;
#[cfg(windows)]
mod clipboard_shortcuts;
mod color_wheel;
mod composite_cursor;
mod dialogs;
mod display_settings;
mod display_workers;
mod document;
mod document_actions;
mod document_edits;
mod dynamics;
mod effects_panel;
mod export_jobs;
mod extension_api;
mod extensions;
mod file_preview;
mod fill;
mod folders;
#[cfg(feature = "allocation-profile")]
mod gpu_profile;
mod guides;
mod history_pixels;
mod icons;
mod image_paste;
mod import_report;
mod indexed_commit;
mod indexed_document;
mod indexed_mask;
mod indexed_thumbnail;
mod indexed_tile;
mod input;
mod jobs;
mod lasso_paint;
mod layer_effects;
mod layer_guides;
mod layer_import;
mod layer_mask;
#[cfg(test)]
mod layer_mask_document_tests;
#[cfg(test)]
mod layer_mask_edit_tests;
mod layer_mask_edits;
#[cfg(test)]
mod layer_mask_geometry_tests;
#[cfg(test)]
mod layer_opacity_tests;
mod layers_panel;
mod load_jobs;
mod locations;
mod mask_clipboard;
mod mask_controller;
#[cfg(test)]
mod mask_feedback_tests;
mod mask_fill;
#[cfg(test)]
mod mask_fill_tests;
#[cfg(test)]
mod mask_stroke_tests;
mod mask_transform;
mod mask_transform_benchmark;
mod move_preview;
mod native_frame_log;
#[cfg(windows)]
mod native_pen;
mod psd_import;
mod psd_vector;
#[cfg(target_os = "linux")]
use wayland_pen as native_pen;
mod native_session;
mod navigator;
mod new_canvas;
mod paint_target;
mod palette;
mod parallel;
mod pending_layers;
mod performance;
mod pixels;
mod plugins;
mod preference_recovery;
mod preferences;
mod prefix_cache;
mod pressure_editor;
mod preview;
mod preview_window;
mod project_package;
mod recovery;
mod recovery_probe;
mod reference_layers;
mod reference_stack;
mod references;
#[cfg(feature = "allocation-profile")]
mod region_census;
mod render_controller;
mod render_queue;
mod render_region;
mod replay;
mod rulers;
mod save_jobs;
mod screentone;
mod selection;
mod selection_contour;
mod selection_mask;
mod selection_ui;
mod settings_store;
mod settings_ui;
mod snapshot_memory;
mod storage;
mod stroke;
mod stroke_buffer;
mod symmetry;
mod tabs;
mod themes;
mod thumbnails;
mod tile_project;
mod timelapse;
mod tip_cursor;
mod tip_import;
mod tool_panel;
mod tool_shift;
mod tools;
mod transform;
mod view;
mod wand;
mod wasm_filters;
#[cfg(target_os = "linux")]
mod wayland_pen;
mod window_chrome;
#[cfg(test)]
mod workflow_tests;
mod workspace;
use tools::Tool;

use brush::Brush;
use document::{Document, History};
use eframe::egui;
use std::path::PathBuf;

struct Dudel {
    theme: themes::Theme,
    theme_editor: Option<themes::Editor>,
    timelapse: timelapse::Timelapse,
    recording: timelapse::Recording,
    performance: performance::Performance,
    capture: Option<capture::Capture>,
    doc: Document,
    history: History,
    brush: Brush,
    color: [u8; 3],
    mask_colors: [u8; 2],
    palette: Vec<[u8; 3]>,
    tool: Tool,
    /// Lasso fill/erase mode and outline shape.
    lasso: lasso_paint::Options,
    /// Which paint group the Brush tool shows, and each group's last preset.
    brush_group: brush::BrushGroup,
    group_presets: [String; 4],
    /// Eraser: the current brush's shape inverted, or its own eraser tip.
    eraser_own: bool,
    /// The eraser tip, or the paint brush while the tip is in `brush`.
    eraser_brush: Brush,
    /// The eraser (tip, or just its size) is in `brush` so every brush
    /// control edits it while the Eraser tool is active.
    eraser_swap: Option<tools::EraserSwap>,
    /// Session copy of each group's brush, including unsaved tweaks.
    group_live: [Option<(usize, Brush)>; 4],
    fill_tolerance: u8,
    fill_options: fill::Options,
    fill_source: fill::Source,
    selection_drag: Option<selection_ui::Drag>,
    selection_shape: selection_ui::Shape,
    selection_combine: selection_mask::Combine,
    selection_feather: u8,
    selection_wand: wand::Settings,
    affine_settings: affine::Settings,
    transform_mode: affine_preview::Mode,
    brush_live_preview: brush_studio::LivePreview,
    brush_studio: bool,
    preference_recovery: Option<preference_recovery::Recovery>,
    settings_store: Option<settings_store::Handle>,
    brush_studio_section: brush_studio::Section,
    brush_studio_preview: brush_studio::LivePreview,
    /// Mesh warp grid size (columns, rows) for new meshes.
    mesh_grid: [u8; 2],
    affine_preview: Option<affine_preview::Preview>,
    selection_contour: selection_contour::Cache,
    texture: canvas_textures::CanvasTextures,
    navigator_paint: Option<navigator::PaintRequest>,
    layer_thumbnails: Vec<egui::TextureHandle>,
    mask_thumbnails: Vec<Option<egui::TextureHandle>>,
    layer_reveal: Option<(document::LayerId, paint_target::Target, [i32; 2])>,
    thumbnail_revision: Option<u64>,
    thumbnail_timer: std::time::Instant,
    /// Pixel identity each thumbnail was drawn from, by stack position.
    thumbnail_stamps: Vec<(bool, pixels::RegionStamp, layer_mask::LayerStamp)>,
    dirty: bool,
    dirty_region: render_region::Damage,
    modified: bool,
    mask_focus: Option<(document::LayerId, document::LayerId)>,
    /// Redraw/source invalidation uses `revision`; these tokens identify only
    /// committed content, so rolling back a contact can recognize a saved state.
    stroke_preview: Option<render_controller::StrokePreview>,
    geometry_preview: Option<render_controller::GeometryPreview>,
    content_revision: u64,
    saving_content_revision: Option<u64>,
    saved_content_revision: Option<u64>,
    stroke: stroke::Stroke,
    input: input::Input,
    replay: replay::Recorder,
    frame_packets: Vec<input::Packet>,
    stroking: bool,
    render_queue: render_queue::Queue,
    queued_stroke: Option<render_queue::StrokeId>,
    edit_feedback: std::collections::BTreeMap<u64, render_controller::EditFeedback>,
    logical_layer: Option<document::LayerId>,
    pending_layers: Option<pending_layers::PendingLayers>,
    stroke_buffer: Option<stroke_buffer::StrokeBuffer>,
    stroke_settings: Option<stroke_buffer::Settings>,
    zoom: f32,
    pan: egui::Vec2,
    rotation: f32,
    mirror: bool,
    secondary: [u8; 3],
    hsv: [f32; 3],
    hex_input: String,
    recent_colors: Vec<[u8; 3]>,
    last_stroke_end: Option<stroke::Sample>,
    stroke_previous_end: Option<stroke::Sample>,
    move_drag: Option<canvas::MoveDrag>,
    pending_move: Option<canvas::PendingMove>,
    deferred_move_ticket: Option<u64>,
    renaming: Option<usize>,
    rename_draft: String,
    opacity_edit: Option<layers_panel::OpacityEdit>,
    opacity_preview_modified: Option<(u64, bool)>,
    effects_edit: Option<effects_panel::Edit>,
    adjustment_edit: Option<adjustment_panel::Edit>,
    /// Adjustment settings last sent through the busy render queue.
    adjustment_sent: Option<(document::LayerId, adjustment::Settings)>,
    adjustment_window_open: bool,
    /// The effects list last sent through the busy render queue, which the
    /// window builds on until the queue has published it.
    effects_sent: Option<(document::LayerId, Vec<layer_effects::LayerEffect>)>,
    effects_window_open: bool,
    /// Show the targeted layer mask on the canvas instead of the artwork.
    mask_view: bool,
    mask_view_layers: Option<([usize; 2], pixels::Pixels, pixels::Pixels)>,
    mask_view_ids: (document::LayerId, document::LayerId),
    mask_view_shown: bool,
    /// Effect drawings (generations) the canvas tiles last showed.
    effects_shown: Vec<u64>,
    tool_shift: tool_shift::ToolShift,
    shortcuts_window: bool,
    about_window: bool,
    layers_height: Option<f32>,
    library_height: Option<f32>,
    /// Stable sidebar pages keep common brush controls within reach.
    brush_panel_page: tool_panel::BrushPage,
    brush_panel_section: brush_studio::Section,
    navigator_height: Option<f32>,
    /// Dragged widths of the tool property panel and the inspector.
    sidebar_widths: [Option<f32>; 2],
    canvas_area: egui::Rect,
    symmetry: symmetry::Symmetry,
    symmetry_drag: bool,
    /// Session symmetry that replaces the global one on specific layers.
    layer_symmetry: std::collections::HashMap<document::LayerId, symmetry::Symmetry>,
    /// Locked guides draw but cannot be dragged on the canvas.
    guides_locked: bool,
    grid: guides::Grid,
    guide_lines: Vec<guides::GuideLine>,
    show_rulers: bool,
    guide_drag: Option<guides::GuideDrag>,
    /// Special rulers strokes snap to, and whether snapping is on.
    rulers: Vec<rulers::Ruler>,
    /// Layer each ruler belongs to, by index into `rulers`; missing or None
    /// means it applies on every layer.
    ruler_owners: Vec<Option<document::LayerId>>,
    snapping: bool,
    snap_grid: bool,
    /// The track the current stroke is held to.
    stroke_snap: Option<rulers::StrokeSnap>,
    ruler_drag: Option<rulers::RulerDrag>,
    /// Ruler kind the next Pointer drag creates.
    ruler_create: Option<rulers::Kind>,
    display_window: bool,
    settings_open: bool,
    settings_section: usize,
    /// Library folder and per-kind file locations for dialogs.
    locations: locations::Locations,
    storage_window: bool,
    /// Pop-out artwork window for another monitor or capture software.
    preview: preview_window::Preview,
    /// Themes found in the Themes folder and when they were scanned.
    theme_library: Option<(std::time::Instant, Vec<themes::Theme>)>,
    /// Saved startup graphics choices, and what actually runs this session.
    display_settings: display_settings::DisplaySettings,
    display_active: String,
    /// When the pen last moved during a stroke, and when the stabilizer last
    /// caught up toward it.
    /// When the stabilized line last advanced toward the pen.
    smoothing_clock: Option<std::time::Instant>,
    /// Guides as last saved or loaded; a difference marks unsaved changes.
    saved_guides: guides::Guides,
    observed_guides: guides::Guides,
    observed_canvas_frame: canvas_size::Frame,
    saving_guides: Option<guides::Guides>,
    brush_previews: brush_previews::Previews,
    status: String,
    pressure: Option<f32>,
    path: Option<PathBuf>,
    show_panels: bool,
    inspector_override: Option<bool>,
    auto_fit: bool,
    tool_panel_override: Option<bool>,
    scratchpad: preview::Scratchpad,
    input_settings: bool,
    pressure_seen: bool,
    brushes: Vec<Brush>,
    selected_brush: usize,
    brush_search: String,
    /// Show only starred brushes in the library.
    brush_favorites_only: bool,
    /// Names of recently chosen brushes, newest first.
    recent_brushes: Vec<String>,
    new_canvas: bool,
    new_canvas_form: new_canvas::Form,
    canvas_resize: Option<canvas_size::Settings>,
    canvas_edit_ticket: Option<u64>,
    adjustment: Option<Box<adjustments::Preview>>,
    adjustment_ticket: Option<u64>,
    retired_adjustment: Option<snapshot_memory::SourceWorker<Result<Document, String>>>,
    recent: Vec<PathBuf>,
    recovery_path: Option<PathBuf>,
    clipboard_backend: Option<arboard::Clipboard>,
    pending_clipboard: std::collections::BTreeMap<u64, clipboard::Action>,
    paste_job: Option<clipboard::PasteJob>,
    copy_job: Option<clipboard::CopyJob>,
    clipboard_settlement: Option<clipboard::ClipboardSettlement>,
    #[cfg(windows)]
    clipboard_shortcuts: Option<clipboard_shortcuts::Backend>,
    recovery_job: Option<snapshot_memory::SourceWorker<Result<u64, String>>>,
    retired_recovery: Option<snapshot_memory::SourceWorker<Result<(), String>>>,
    recovery_revision: u64,
    revision: u64,
    filter_job: Option<jobs::FilterJob>,
    save_job: Option<save_jobs::SaveJob>,
    queued_save: Option<save_jobs::QueuedSave>,
    export_job: Option<export_jobs::ExportJob>,
    queued_export: Option<export_jobs::QueuedExport>,
    package_import: Option<project_package::ImportJob>,
    experimental_indexed_storage: bool,
    indexed_session: Option<indexed_commit::Session>,
    indexed_pending: Option<save_jobs::IndexedPending>,
    indexed_epoch: u64,
    load_job: Option<load_jobs::LoadJob>,
    pending_action: Option<document_actions::Action>,
    unsaved_action: Option<document_actions::Action>,
    close_ready: bool,
    heap_compact_pending: bool,
    last_close_prompt_frame: Option<u64>,
    extensions: Vec<extensions::Installed>,
    extension_errors: Vec<String>,
    extension_manager: bool,
    import_report: import_report::ImportReport,
    extension_panels: std::collections::BTreeSet<String>,
    references: references::References,
    /// Images placed on the canvas to draw from; saved with the project.
    reference_layers: reference_layers::State,
    /// View > Show transparency: checkerboard instead of white paper.
    show_transparency: bool,
    /// Height of the color area under the tool settings; None uses a default.
    color_height: Option<f32>,
    recovery_timer: std::time::Instant,
    /// When the latest crash-recovery backup finished writing.
    recovery_saved: Option<std::time::Instant>,
    recovery_candidates: Vec<PathBuf>,
    recovery_window: bool,
    /// The file dialog or confirmation waiting for an answer.
    dialogs: dialogs::Dialogs,
    /// Open projects; inactive ones are parked here.
    tabs: tabs::Tabs,
    /// Texture of the current image tip for the brush cursor.
    tip_cursor: tip_cursor::TipTexture,
    /// Dropped projects waiting to open, one tab each.
    pending_opens: Vec<PathBuf>,
}

impl Default for Dudel {
    fn default() -> Self {
        Self {
            performance: Default::default(),
            capture: None,
            theme: Default::default(),
            theme_editor: None,
            timelapse: Default::default(),
            recording: Default::default(),
            doc: Document::new(1024, 768),
            history: History::new(),
            brush: workspace::default_brushes()[0].clone(),
            color: [38, 44, 59],
            mask_colors: [0, 255],
            palette: palette::defaults(),
            tool: Tool::Brush,
            lasso: Default::default(),
            brush_group: workspace::default_brushes()[0].group(),
            group_presets: Default::default(),
            group_live: Default::default(),
            eraser_own: false,
            eraser_brush: tools::default_eraser(),
            eraser_swap: None,
            fill_tolerance: 12,
            fill_options: fill::Options {
                fill_edges: true,
                ..Default::default()
            },
            fill_source: Default::default(),
            selection_drag: None,
            selection_shape: selection_ui::Shape::Rectangle,
            selection_combine: selection_mask::Combine::Replace,
            selection_feather: 8,
            selection_wand: Default::default(),
            affine_settings: affine::Settings::default(),
            transform_mode: Default::default(),
            brush_live_preview: Default::default(),
            brush_studio: false,
            preference_recovery: None,
            settings_store: None,
            brush_studio_section: Default::default(),
            brush_studio_preview: Default::default(),
            mesh_grid: [affine::MESH_DEFAULT; 2],
            affine_preview: None,
            selection_contour: Default::default(),
            texture: Default::default(),
            navigator_paint: None,
            layer_thumbnails: vec![],
            mask_thumbnails: vec![],
            layer_reveal: None,
            thumbnail_revision: None,
            thumbnail_timer: std::time::Instant::now(),
            thumbnail_stamps: Vec::new(),
            dirty: true,
            dirty_region: Default::default(),
            modified: false,
            mask_focus: None,
            stroke_preview: None,
            geometry_preview: None,
            content_revision: 0,
            saving_content_revision: None,
            saved_content_revision: None,
            stroke: stroke::Stroke::default(),
            input: input::Input::default(),
            replay: Default::default(),
            frame_packets: Vec::new(),
            stroking: false,
            render_queue: Default::default(),
            queued_stroke: None,
            edit_feedback: Default::default(),
            logical_layer: None,
            pending_layers: None,
            stroke_buffer: None,
            stroke_settings: None,
            zoom: 0.0,
            pan: egui::Vec2::ZERO,
            rotation: 0.0,
            mirror: false,
            secondary: [255, 255, 255],
            hsv: color_wheel::rgb_to_hsv([38, 44, 59]),
            hex_input: String::new(),
            recent_colors: vec![],
            last_stroke_end: None,
            stroke_previous_end: None,
            move_drag: None,
            pending_move: None,
            deferred_move_ticket: None,
            renaming: None,
            rename_draft: String::new(),
            opacity_edit: None,
            opacity_preview_modified: None,
            effects_edit: None,
            adjustment_edit: None,
            adjustment_sent: None,
            adjustment_window_open: false,
            effects_sent: None,
            effects_window_open: false,
            mask_view: false,
            mask_view_layers: None,
            mask_view_ids: (document::LayerId::new(), document::LayerId::new()),
            mask_view_shown: false,
            effects_shown: Vec::new(),
            tool_shift: Default::default(),
            shortcuts_window: false,
            about_window: false,
            layers_height: None,
            library_height: None,
            brush_panel_page: Default::default(),
            brush_panel_section: brush_studio::Section::Pressure,
            navigator_height: None,
            sidebar_widths: [None; 2],
            canvas_area: egui::Rect::NOTHING,
            symmetry: symmetry::Symmetry::default(),
            symmetry_drag: false,
            layer_symmetry: Default::default(),
            guides_locked: true,
            grid: Default::default(),
            guide_lines: Vec::new(),
            show_rulers: false,
            guide_drag: None,
            rulers: Vec::new(),
            ruler_owners: Vec::new(),
            snapping: true,
            snap_grid: false,
            stroke_snap: None,
            ruler_drag: None,
            ruler_create: None,
            display_window: false,
            settings_open: false,
            settings_section: 0,
            locations: Default::default(),
            storage_window: false,
            preview: Default::default(),
            theme_library: None,
            display_settings: Default::default(),
            display_active: String::new(),
            smoothing_clock: None,
            saved_guides: Default::default(),
            observed_guides: Default::default(),
            observed_canvas_frame: Default::default(),
            saving_guides: None,
            brush_previews: Default::default(),
            status: "Ready".into(),
            pressure: None,
            path: None,
            show_panels: true,
            inspector_override: None,
            auto_fit: true,
            tool_panel_override: None,
            scratchpad: preview::Scratchpad::default(),
            input_settings: false,
            pressure_seen: false,
            brushes: workspace::default_brushes(),
            selected_brush: 0,
            brush_search: String::new(),
            brush_favorites_only: false,
            recent_brushes: Vec::new(),
            new_canvas: false,
            new_canvas_form: Default::default(),
            canvas_resize: None,
            canvas_edit_ticket: None,
            adjustment: None,
            adjustment_ticket: None,
            retired_adjustment: None,
            recent: vec![],
            recovery_path: None,
            clipboard_backend: None,
            pending_clipboard: Default::default(),
            paste_job: None,
            copy_job: None,
            clipboard_settlement: None,
            #[cfg(windows)]
            clipboard_shortcuts: None,
            recovery_job: None,
            retired_recovery: None,
            recovery_revision: 0,
            revision: 0,
            filter_job: None,
            save_job: None,
            queued_save: None,
            export_job: None,
            queued_export: None,
            package_import: None,
            experimental_indexed_storage: false,
            indexed_session: None,
            indexed_pending: None,
            indexed_epoch: 0,
            load_job: None,
            pending_action: None,
            unsaved_action: None,
            close_ready: false,
            heap_compact_pending: false,
            last_close_prompt_frame: None,
            extensions: vec![],
            extension_errors: vec![],
            extension_manager: false,
            import_report: Default::default(),
            extension_panels: Default::default(),
            references: references::References::default(),
            reference_layers: Default::default(),
            show_transparency: false,
            color_height: None,
            recovery_timer: std::time::Instant::now(),
            recovery_saved: None,
            recovery_candidates: vec![],
            recovery_window: false,
            dialogs: Default::default(),
            tabs: Default::default(),
            pending_opens: Vec::new(),
            tip_cursor: Default::default(),
        }
    }
}

fn read_bounded(path: &std::path::Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("File exceeds size limit".into());
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

impl Dudel {
    fn transform_active(&mut self, transform: transform::Transform) {
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let edit = match self.input_stroke_target() {
            paint_target::Target::Raster => {
                document_edits::Edit::TransformTarget { target, transform }
            }
            paint_target::Target::Mask(mask) => document_edits::Edit::MaskTransform {
                target,
                mask,
                operation: mask_transform::Operation::Flip(transform),
            },
        };
        self.dispatch_document_edit(
            edit,
            "Transformed active layer",
            "Transform made no changes",
        );
    }
    /// Rotate the artwork by quarter turns clockwise (changes pixels).
    fn rotate_canvas(&mut self, quarter_turns: u8) {
        self.dispatch_canvas_size(document_edits::Edit::Rotate(quarter_turns));
    }
    fn set_selection(&mut self, selection: Option<selection::Selection>) {
        self.dispatch_document_edit(
            document_edits::Edit::Selection(selection),
            if selection.is_some() {
                "Selection active"
            } else {
                "Selection cleared"
            },
            "Selection unchanged",
        );
    }
    fn select_all(&mut self) {
        self.set_selection(Some(selection::Selection {
            left: 0,
            top: 0,
            right: self.doc.width,
            bottom: self.doc.height,
        }));
    }
    fn clear_selection_pixels(&mut self) {
        self.clear_pixels(false);
    }
    fn clear_layer_pixels(&mut self) {
        self.clear_pixels(true);
    }
    fn clear_pixels(&mut self, whole_layer: bool) {
        if let paint_target::Target::Mask(id) = self.input_stroke_target() {
            self.edit_canvas_mask(layer_mask_edits::Command::Clear { id, whole_layer });
            return;
        }
        let Some(target) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        self.dispatch_document_edit(
            document_edits::Edit::ClearTarget {
                target,
                whole_layer,
            },
            if !whole_layer && self.doc.selection.is_some() {
                "Cleared selected pixels"
            } else {
                "Cleared active layer"
            },
            "No pixels to clear",
        );
    }
    fn changed(&mut self) {
        self.sync_canvas_frame();
        self.dirty = true;
        self.dirty_region = Default::default();
        self.metadata_changed();
    }
    fn metadata_changed(&mut self) {
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        self.content_revision = self.content_revision.wrapping_add(1);
        self.reconcile_mask_focus();
    }
    fn open(&mut self) {
        let mut dialog = self.dialog_in(
            locations::Kind::Projects,
            rfd::AsyncFileDialog::new()
                .add_filter("Dudel project", &["dudel"])
                .add_filter("CSP raster canvas", &["clip"])
                .add_filter("Photoshop document", &["psd", "psb"]),
        );
        if self.experimental_indexed_storage {
            dialog = dialog.add_filter("Experimental indexed project", &["didx"]);
        }
        self.ask_open_file(dialog, Self::open_path);
    }
    fn open_path(&mut self, path: PathBuf) {
        // A project that is already open is shown rather than opened twice.
        if let Some(index) = self.tab_with_path(&path) {
            self.request_tab(index);
            return;
        }
        self.open_in_tab(move |app| app.load_document(path, false));
    }
    fn load_preferences(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut app = Self::default();
        if let Some(source) = storage.and_then(|s| s.get_string(preferences::KEY)) {
            app.restore_preferences_source(&source);
        }
        app.init_recovery();
        app.load_extensions();
        app
    }
    fn apply_preferences(&mut self, p: preferences::Preferences) {
        self.unswap_eraser();
        self.theme = p.theme;
        self.brushes = p.brushes;
        self.brush = p.current_brush;
        self.selected_brush = p.selected_brush;
        self.color = p.color;
        self.palette = p.palette;
        self.show_panels = p.show_panels;
        self.input.native_requested = p.native_pen;
        self.inspector_override = p.inspector_override;
        self.tool_panel_override = p.tool_panel_override;
        self.recent = p.recent;
        self.secondary = p.secondary;
        self.recent_colors = p.recent_colors;
        self.set_show_transparency(p.show_transparency);
        self.lasso = p.tools.lasso;
        // The current brush decides the group, so the two never disagree.
        self.brush_group = self.brush.group();
        self.group_presets = p.tools.group_presets;
        self.locations = p.storage;
        self.eraser_own = p.tools.eraser_own;
        if let Some(eraser) = p.tools.eraser.filter(|e| e.validate().is_ok()) {
            self.eraser_brush = eraser;
        }
        self.sync_eraser_slot();
        self.preview.settings = p.preview;
        self.tabs.max = p.tabs.max.clamp(1, tabs::LIMIT);
        self.timelapse.prefs = p.timelapse.sanitized();
        let finite = |h: Option<f32>| h.filter(|h| h.is_finite() && *h > 0.0);
        self.library_height = finite(p.layout.library_height);
        self.color_height = finite(p.layout.color_height);
        self.navigator_height = finite(p.layout.navigator_height);
        self.layers_height = finite(p.layout.layers_height);
        self.hsv = color_wheel::rgb_to_hsv(self.color);
    }
    fn encode_preferences(&self) -> Result<String, String> {
        // While the eraser tip is in the brush slot the two are exchanged.
        let (paint, eraser) = (self.paint_brush(), self.eraser_tip());
        let (paint, eraser) = (paint.as_ref(), eraser.as_ref());
        let p = preferences::Preferences {
            theme: self.theme_for_preferences().clone(),
            version: 1,
            brushes: self.brushes.clone(),
            current_brush: if paint.validate().is_ok() {
                paint.clone()
            } else {
                self.brushes[self.selected_brush].clone()
            },
            selected_brush: self.selected_brush,
            color: self.color,
            palette: self.palette.clone(),
            show_panels: self.show_panels,
            native_pen: self.input.native_requested,
            inspector_override: self.inspector_override,
            tool_panel_override: self.tool_panel_override,
            recent: self.recent.clone(),
            secondary: self.secondary,
            recent_colors: self.recent_colors.clone(),
            show_transparency: self.show_transparency,
            tools: preferences::ToolPrefs {
                lasso: self.lasso,
                brush_group: self.brush_group,
                group_presets: self.group_presets.clone(),
                eraser_own: self.eraser_own,
                eraser: eraser.validate().is_ok().then(|| eraser.clone()),
            },
            storage: self.locations.clone(),
            preview: self.preview.settings.clone(),
            tabs: preferences::TabPrefs { max: self.tabs.max },
            timelapse: self.timelapse.prefs.clone(),
            layout: preferences::LayoutPrefs {
                library_height: self.library_height,
                color_height: self.color_height,
                navigator_height: self.navigator_height,
                layers_height: self.layers_height,
            },
        };
        serde_json::to_string(&p).map_err(|e| e.to_string())
    }
    fn export(&mut self, transparent: bool) {
        if self.adjustment.is_some() {
            self.status = "Apply or cancel the adjustment before exporting".into();
            return;
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            self.status = "An image export is already running".into();
            return;
        }
        let name = self
            .path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map_or("Painting".into(), |s| s.to_string_lossy().into_owned());
        let dialog = self.dialog_in(
            locations::Kind::Exports,
            rfd::AsyncFileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name(format!("{name}.png")),
        );
        self.ask_save_file(dialog, move |app, path| app.export_to(path, transparent));
    }
    fn import_png(&mut self) {
        let dialog = rfd::AsyncFileDialog::new().add_filter("PNG image", &["png"]);
        self.ask_open_file(dialog, |app, path| {
            app.open_in_tab(move |app| app.load_document(path, true));
        });
    }
    fn import_clip(&mut self) {
        let dialog = self.dialog_in(
            locations::Kind::Projects,
            rfd::AsyncFileDialog::new().add_filter("CSP raster canvas", &["clip"]),
        );
        self.ask_open_file(dialog, |app, path| {
            app.open_in_tab(move |app| app.load_document(path, false));
        });
    }
    fn import_psd(&mut self) {
        let dialog = rfd::AsyncFileDialog::new().add_filter("Photoshop document", &["psd", "psb"]);
        self.ask_open_file(dialog, |app, path| {
            app.open_in_tab(move |app| app.load_document(path, false));
        });
    }
    fn filter(&mut self, source: &str) {
        self.start_filter(plugins::FilterProgram::Rhai(source.to_owned()));
    }
    fn start_filter(&mut self, program: plugins::FilterProgram) {
        if !self.require_artwork_target() {
            return;
        }
        if self.filter_job.is_some() {
            self.status = "A filter is already running; cancel it before starting another".into();
            return;
        }
        self.cancel_gestures();
        if self.doc.layers[self.doc.active].folder {
            self.status = "Select a layer inside the folder to filter it".into();
            return;
        }
        if self.doc.layers[self.doc.active].locked {
            self.status = "Unlock the layer before filtering".into();
            return;
        }
        let mut layer = self.doc.layers[self.doc.active].clone();
        let total = layer.pixels.len() / 4;
        let control = plugins::FilterControl::default();
        let worker_control = control.clone();
        let selection = self.doc.selection;
        let mask = self.doc.selection_mask.clone();
        let width = self.doc.width;
        let worker = std::thread::spawn(move || {
            match program {
                plugins::FilterProgram::Rhai(source) => plugins::apply_filter_masked(
                    &mut layer,
                    &source,
                    selection,
                    mask.as_ref(),
                    width,
                    &worker_control,
                )?,
                plugins::FilterProgram::Wasm(bytes) => wasm_filters::apply_masked(
                    &mut layer,
                    &bytes,
                    selection,
                    mask.as_ref(),
                    width,
                    &worker_control,
                )?,
            }
            Ok(layer)
        });
        self.filter_job = Some(jobs::FilterJob {
            worker,
            control,
            revision: self.revision,
            layer_index: self.doc.active,
            total,
        });
        self.status = "Filter running in background".into();
    }
    fn brush_file(&mut self, import: bool) {
        let dialog = self.dialog_in(
            crate::locations::Kind::Brushes,
            rfd::AsyncFileDialog::new().add_filter("Brush JSON", &["json"]),
        );
        if import {
            self.ask_open_file(dialog, Self::import_brush);
        } else {
            self.ask_save_file(
                dialog.set_file_name("custom-brush.json"),
                Self::export_brush,
            );
        }
    }
    fn import_brush(&mut self, path: PathBuf) {
        let result = (|| {
            let b: Brush = serde_json::from_str(&read_bounded(&path, 512 * 1024)?)
                .map_err(|e| e.to_string())?;
            b.validate()?;
            Ok::<_, String>(b)
        })();
        match result {
            Ok(b) => {
                if self.brushes.len() >= 512 {
                    self.status = "Brush library is full (512 presets)".into();
                    return;
                }
                self.brushes.push(b.clone());
                self.selected_brush = self.brushes.len() - 1;
                self.brush = b;
                self.status = "Brush imported".into();
            }
            Err(e) => self.status = e,
        }
    }
    fn export_brush(&mut self, path: PathBuf) {
        let result = self
            .brush
            .validate()
            .and_then(|()| serde_json::to_vec_pretty(&self.brush).map_err(|e| e.to_string()))
            .and_then(|data| storage::atomic_write(&path, &data));
        self.status = match result {
            Ok(()) => "Brush saved".into(),
            Err(e) => e,
        };
    }
}

impl eframe::App for Dudel {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if (self.capture.is_some() && self.settings_store.is_none())
            || self.preference_recovery.is_some()
        {
            return;
        }
        if let Ok(source) = self.encode_preferences() {
            storage.set_string(preferences::KEY, source.clone());
            if let Some(handle) = &self.settings_store {
                if handle.status().error.is_some() {
                    storage.set_string(
                        "dudel.settings.retry",
                        format!("{:?}", std::time::SystemTime::now()),
                    );
                }
                handle.request(source);
            }
        }
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.capture.as_ref().is_some_and(|c| c.session.is_some()) {
            raw_input.events.clear();
            raw_input.modifiers = Default::default();
            return;
        }
        if let Some(pointer) = self.capture.as_ref().and_then(|c| c.pointer) {
            raw_input.events.push(egui::Event::PointerMoved(pointer));
        }
        #[cfg(target_os = "linux")]
        self.input.raw_input_hook(raw_input);
        if self.dialogs.native_open() {
            // Nothing in the window answers a system dialog.
            raw_input.events.clear();
            return;
        }
        // Ctrl+Tab and Ctrl+Shift+Tab step through open projects.
        raw_input.events.retain(|event| match event {
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed,
                modifiers,
                ..
            } if modifiers.command => {
                if *pressed {
                    self.tabs.cycle += if modifiers.shift { -1 } else { 1 };
                }
                false
            }
            _ => true,
        });
        if !self.new_canvas
            && !self.dialogs.open()
            && !raw_input.events.iter().any(new_canvas::opening_shortcut)
            && workspace::take_panel_toggle(ctx, raw_input)
        {
            self.show_panels = !self.show_panels;
        }
    }
    fn persist_egui_memory(&self) -> bool {
        self.capture.is_none() && self.preference_recovery.is_none()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let frame_start = std::time::Instant::now();
        // wgpu-core queues the views/bind groups of destroyed textures and only
        // releases them in Device::poll, which egui-wgpu never calls. Without
        // this nonblocking poll every freed display texture leaks them.
        if let Some(state) = frame.wgpu_render_state() {
            let _ = state.device.poll(eframe::wgpu::PollType::Poll);
        }
        self.performance.begin_frame();
        if self.stroking {
            self.performance.note_drawing();
        }
        self.track_guide_changes();
        themes::apply(ctx, &self.theme);
        self.poll_filter(ctx);
        self.poll_save(ctx);
        self.poll_export(ctx);
        self.poll_package_import(ctx);
        self.poll_load(ctx);
        self.poll_paste(ctx);
        self.process_tab_request();
        self.handle_dropped_files(ctx);
        self.open_pending_projects();
        self.poll_adjustment(ctx);
        self.compact_heap_when_settled();
        if self.close_ready {
            self.poll_recovery(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // A save finishing before close keeps its progress and cancel controls.
                    if !self.save_progress_ui(ui) {
                        ui.spinner();
                        ui.label("Closing project...");
                    }
                });
            });
            self.close_viewport_if_ready(ctx);
            self.performance.frame_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
            return;
        }
        let previous_tool = self.tool;
        let events = ctx.input(|i| i.events.clone());
        let generated = self
            .capture
            .as_mut()
            .and_then(|c| c.session.as_mut())
            .and_then(|s| s.take_packets());
        let packets =
            generated.unwrap_or_else(|| self.input.frame_packets(&events, ctx.pixels_per_point()));
        self.frame_packets.clone_from(&packets);
        self.pressure = self.input.diagnostics.pressure;
        self.pressure_seen |= self
            .input
            .diagnostics
            .last
            .is_some_and(|s| s.pressure_known);
        self.update_tool_shift(ctx);
        if self.unsaved_action.is_none() && !self.dialogs.open() {
            self.shortcuts(ctx);
        }
        self.preference_recovery_banner(ctx);
        if self.settings_store_banner(ctx)
            && let Some(storage) = frame.storage_mut()
        {
            eframe::App::save(self, storage);
            storage.flush();
        }
        self.workspace(ctx);
        self.canvas_after_tool_change(ctx, &packets, previous_tool);
        self.track_guide_changes();
        if !self.stroking
            && !self.render_queue.busy()
            && self.opacity_edit.is_none()
            && self.effects_edit.is_none()
            && self.adjustment_edit.is_none()
            && packets.is_empty()
            && self.adjustment.is_none()
            && self.history.cool_idle(&self.doc)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.poll_recovery(ctx);
        self.performance_window(ctx);
        self.display_settings_ui(ctx);
        self.storage_window(ctx);
        self.effects_window(ctx);
        self.adjustment_window(ctx);
        self.drop_overlay(ctx);
        self.note_preview_pointer(ctx);
        self.preview_window(ctx);
        self.brush_studio_ui(ctx);
        let recovery_capture = preference_recovery::CaptureStorage;
        self.preference_recovery_ui(
            ctx,
            if self.capture.is_some() {
                Some(&recovery_capture)
            } else {
                frame.storage()
            },
        );
        self.performance.frame_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
        self.poll_timelapse(ctx);
        self.timelapse_ui(ctx);
        self.theme_editor_ui(ctx);
        self.capture_frame(ctx);
        if self.capture.is_none()
            && !self.close_ready
            && ctx.input(|i| i.viewport().close_requested())
        {
            let frame = ctx.cumulative_frame_nr();
            self.dismiss_dialogs();
            if self.first_close_request_in_frame(frame) {
                self.request_replace(document_actions::Action::Close);
            }
            if !self.close_ready {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        self.unsaved_dialog(ctx);
        self.dialogs_ui(ctx);
        self.poll_dialogs(ctx);
        self.close_viewport_if_ready(ctx);
        self.performance.frame_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
        self.performance.frames.record(self.performance.frame_ms);
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let mode = args.next();
    if mode.as_deref() == Some("--check-plugin") {
        let path = args.next().map(PathBuf::from);
        let result = match path {
            Some(path) if args.next().is_none() => extensions::check_package(&path),
            _ => Err("Usage: --check-plugin PACKAGE.dudelplugin".into()),
        };
        if let Err(error) = result {
            eprintln!("Plugin validation: {error}");
            std::process::exit(1);
        }
        println!("Plugin API, metadata and program validation passed");
        return Ok(());
    }
    if mode.as_deref() == Some("--probe-artwork-recovery") {
        let root = args.next().map(PathBuf::from);
        let phase = args.next();
        if root.is_none() || phase.is_none() || args.next().is_some() {
            eprintln!(
                "Usage: --probe-artwork-recovery NEW_ISOLATED_FOLDER write-v4|write-v5|restore"
            );
            std::process::exit(1);
        }
        if let Err(error) = recovery_probe::run(&root.unwrap(), &phase.unwrap()) {
            eprintln!("Artwork recovery probe failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--probe-settings-store") {
        let root = args.next().map(PathBuf::from);
        let phase = args.next();
        if root.is_none() || phase.is_none() || args.next().is_some() {
            eprintln!(
                "Usage: --probe-settings-store ISOLATED_FOLDER first|reopen|corrupt|repair|failure|retry"
            );
            std::process::exit(1);
        }
        if let Err(error) = settings_store::probe(&root.unwrap(), &phase.unwrap()) {
            eprintln!("Settings probe failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if matches!(
        mode.as_deref(),
        Some(
            "--study-indexed-commits"
                | "--study-indexed-tiles"
                | "--study-indexed-documents"
                | "--study-indexed-stream"
                | "--study-indexed-thumbnails"
                | "--study-indexed-compaction"
                | "--benchmark-indexed-lifecycle"
        )
    ) {
        let report = args.next().map(PathBuf::from);
        if args.next().is_some() {
            eprintln!("Usage: {} [NEW_REPORT.json]", mode.as_deref().unwrap());
            std::process::exit(1);
        }
        let result = if mode.as_deref() == Some("--benchmark-indexed-lifecycle") {
            indexed_commit::benchmark_lifecycle(report.as_deref())
        } else if mode.as_deref() == Some("--study-indexed-compaction") {
            indexed_commit::study_compaction(report.as_deref())
        } else if mode.as_deref() == Some("--study-indexed-thumbnails") {
            indexed_commit::study_thumbnails(report.as_deref())
        } else if mode.as_deref() == Some("--study-indexed-stream") {
            indexed_commit::study_stream(report.as_deref())
        } else if mode.as_deref() == Some("--study-indexed-documents") {
            indexed_commit::study_documents(report.as_deref())
        } else if mode.as_deref() == Some("--study-indexed-tiles") {
            indexed_commit::study_tiles(report.as_deref())
        } else {
            indexed_commit::study(report.as_deref())
        };
        if let Err(error) = result {
            eprintln!("Indexed storage study: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--compare-clip-export") {
        let input = args.next().map(PathBuf::from);
        let reference = args.next().map(PathBuf::from);
        let report = args.next().map(PathBuf::from);
        let (Some(input), Some(reference)) = (input, reference) else {
            eprintln!("Usage: --compare-clip-export FILE.clip REFERENCE.png [NEW_REPORT.json]");
            std::process::exit(1);
        };
        if args.next().is_some() {
            eprintln!("Usage: --compare-clip-export FILE.clip REFERENCE.png [NEW_REPORT.json]");
            std::process::exit(1);
        }
        if let Err(error) = clip_metadata::compare_export(&input, &reference, report.as_deref()) {
            eprintln!("Clip export comparison: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if matches!(
        mode.as_deref(),
        Some(
            "--inspect-clip"
                | "--inspect-clip-metadata"
                | "--inspect-clip-blocks"
                | "--verify-clip-raster"
                | "--verify-clip-document"
                | "--verify-import"
                | "--verify-clip-preview"
                | "--compare-clip-rendering"
                | "--compare-clip-folder-caches"
        )
    ) {
        let Some(input) = args.next().map(PathBuf::from) else {
            eprintln!(
                "Usage: {} FILE.clip [NEW_REPORT.json]",
                mode.as_deref().unwrap()
            );
            std::process::exit(1);
        };
        let report = args.next().map(PathBuf::from);
        // --verify-import also takes a reference image to compare against.
        let reference = if mode.as_deref() == Some("--verify-import") {
            args.next().map(PathBuf::from)
        } else {
            None
        };
        if args.next().is_some() {
            eprintln!(
                "Usage: {} FILE.clip [NEW_REPORT.json]{}",
                mode.as_deref().unwrap(),
                if mode.as_deref() == Some("--verify-import") {
                    " [REFERENCE.png]"
                } else {
                    ""
                }
            );
            std::process::exit(1);
        }
        let result = if matches!(
            mode.as_deref(),
            Some("--verify-import" | "--verify-clip-preview")
        ) {
            import_report::verify(
                &input,
                report.as_deref(),
                reference.as_deref(),
                mode.as_deref() == Some("--verify-clip-preview"),
            )
        } else if mode.as_deref() == Some("--inspect-clip-metadata") {
            clip_metadata::inspect(&input, report.as_deref())
        } else if mode.as_deref() == Some("--compare-clip-folder-caches") {
            clip_metadata::compare_folder_caches(&input, report.as_deref())
        } else if mode.as_deref() == Some("--compare-clip-rendering") {
            clip_metadata::compare_rendering(&input, report.as_deref())
        } else if mode.as_deref() == Some("--verify-clip-document") {
            clip_metadata::verify_document(&input, report.as_deref())
        } else if mode.as_deref() == Some("--verify-clip-raster") {
            clip_metadata::verify_rasters(&input, report.as_deref())
        } else if mode.as_deref() == Some("--inspect-clip-blocks") {
            clip_blocks::inspect(&input, report.as_deref())
        } else {
            clip_container::inspect(&input, report.as_deref())
        };
        if let Err(error) = result {
            eprintln!("Clip inspection: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-prefix-plan") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("prefix-plan.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-prefix-plan [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = prefix_cache::benchmark_plan(&path) {
            eprintln!("Prefix plan benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if matches!(
        mode.as_deref(),
        Some("--benchmark-live-display" | "--benchmark-moving-display")
    ) {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("live-display.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-live-display / --benchmark-moving-display [FILE.json]");
            std::process::exit(1);
        }
        let result = if mode.as_deref() == Some("--benchmark-moving-display") {
            canvas_textures::benchmark_live_motion(&path)
        } else {
            canvas_textures::benchmark_live(&path)
        };
        if let Err(error) = result {
            eprintln!("Live display benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-display-memory") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("display-memory.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-display-memory [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = canvas_textures::benchmark(&path) {
            eprintln!("Display memory benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-clipboard") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("clipboard.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-clipboard [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = clipboard::benchmark(&path) {
            eprintln!("Clipboard benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-move-preview") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("move-preview.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-move-preview [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = canvas::benchmark_moves(&path) {
            eprintln!("Move preview benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-mask-transform") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("mask-transform.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-mask-transform [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = transform::benchmark_masks(&path) {
            eprintln!("Mask transform benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-selection-contour") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("selection-contour.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-selection-contour [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = selection_contour::benchmark(&path) {
            eprintln!("Selection contour benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-affine-dense") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("affine-dense.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-affine-dense [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = affine_benchmark::run(&path) {
            eprintln!("Dense affine benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-affine") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("affine.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-affine [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = affine::benchmark(&path) {
            eprintln!("Affine benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-selection") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("selection.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-selection [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = selection_mask::benchmark(&path) {
            eprintln!("Selection benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-fill") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("fill.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-fill [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = fill::benchmark(&path) {
            eprintln!("Fill benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-render-queue") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("render-queue.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-render-queue [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = render_queue::benchmark(&path) {
            eprintln!("Render queue: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--replay-input") {
        let result = (|| {
            let path = args.next().map(PathBuf::from).ok_or("Usage: --replay-input FILE [--batch-size N] [--work-budget N] [--output FILE.dudel] [--report FILE.json]")?;
            let mut batch_size = 64;
            let mut work_budget = 0;
            let mut output = None;
            let mut report = None;
            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("Missing value for {flag}"))?;
                match flag.as_str() {
                    "--batch-size" => {
                        batch_size = value
                            .parse::<usize>()
                            .map_err(|_| "Invalid replay batch size")?
                    }
                    "--output" => output = Some(PathBuf::from(value)),
                    "--work-budget" => {
                        work_budget = value
                            .parse::<usize>()
                            .map_err(|_| "Invalid rendering work budget")?
                    }
                    "--report" => report = Some(PathBuf::from(value)),
                    _ => return Err(format!("Unknown replay flag: {flag}")),
                }
            }
            replay::run(
                &path,
                batch_size,
                work_budget,
                output.as_deref(),
                report.as_deref(),
            )
        })();
        if let Err(error) = result {
            eprintln!("Replay: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark") {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("benchmark.json"));
        let flags: Vec<String> = args.collect();
        let incremental = flags.iter().any(|flag| flag == "--incremental");
        let buffered = !flags.iter().any(|flag| flag == "--unbuffered");
        if flags
            .iter()
            .any(|flag| flag != "--incremental" && flag != "--unbuffered")
        {
            eprintln!("Unknown benchmark flag; supported: --incremental, --unbuffered");
            std::process::exit(1);
        }
        if let Err(error) = benchmark::run(&path, incremental, buffered) {
            eprintln!("Benchmark: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if mode.as_deref() == Some("--benchmark-layer-masks") {
        let report = args.next().map(PathBuf::from);
        if report.is_none() || args.next().is_some() {
            eprintln!("Usage: --benchmark-layer-masks NEW_REPORT.json");
            std::process::exit(1);
        }
        if let Err(error) = layer_mask::benchmark(&report.unwrap()) {
            eprintln!("Layer mask engine: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if matches!(
        mode.as_deref(),
        Some("--benchmark-mask-transforms" | "--soak-mask-transforms")
    ) {
        let path = args.next().map(PathBuf::from);
        if path.is_none() || args.next().is_some() {
            eprintln!("Usage: {} NEW_REPORT.json", mode.as_deref().unwrap());
            std::process::exit(1);
        }
        let path = path.unwrap();
        let result = if mode.as_deref() == Some("--soak-mask-transforms") {
            mask_transform_benchmark::soak(&path)
        } else {
            mask_transform_benchmark::run(&path)
        };
        if let Err(error) = result {
            eprintln!("Mask transforms: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let experimental_indexed_storage = mode.as_deref() == Some("--experimental-indexed-storage");
    let sustained_session = mode.as_deref() == Some("--soak-native-drawing-session");
    let native_session = mode.as_deref() == Some("--soak-native-mask-session") || sustained_session;
    let capture_large_detail = mode.as_deref() == Some("--capture-large-canvas-detail");
    let capture_large = mode.as_deref() == Some("--capture-large-canvas") || capture_large_detail;
    let capture_editor = mode.as_deref() == Some("--capture-brush-editor");
    let capture_studio = mode.as_deref() == Some("--capture-brush-studio");
    let capture_settings_recovery = mode.as_deref() == Some("--capture-settings-recovery");
    let capture_new_canvas_invalid = mode.as_deref() == Some("--capture-new-canvas-invalid");
    let capture_new_canvas = matches!(
        mode.as_deref(),
        Some("--capture-new-canvas" | "--capture-new-canvas-invalid")
    );
    let capture_long_toolbar = matches!(
        mode.as_deref(),
        Some(
            "--capture-long-toolbar"
                | "--capture-long-toolbar-options"
                | "--capture-long-toolbar-brushes"
        )
    );
    let capture_extensions = mode.as_deref() == Some("--capture-extensions");
    let capture_settings = mode.as_deref() == Some("--capture-settings");
    let capture_preview = mode.as_deref() == Some("--capture-preview");
    let capture_project_memory = mode.as_deref() == Some("--capture-project-memory");
    let capture_tabs = mode.as_deref() == Some("--capture-tabs") || capture_project_memory;
    let capture_rope = mode.as_deref() == Some("--capture-rope");
    let capture_layer_guides = mode.as_deref() == Some("--capture-layer-guides");
    let capture_eraser = mode.as_deref() == Some("--capture-eraser");
    let capture_fill = mode.as_deref() == Some("--capture-fill");
    let capture_tip_cursor = mode.as_deref() == Some("--capture-tip-cursor");
    let capture_reference_boards =
        mode.as_deref() == Some("--capture-reference-boards") || capture_preview;
    let capture_reference_layers =
        mode.as_deref() == Some("--capture-reference-layers") || capture_reference_boards;
    let capture_theme_editor = mode.as_deref() == Some("--capture-theme-editor");
    let capture_timelapse = mode.as_deref() == Some("--capture-timelapse");
    let capture_references = mode.as_deref() == Some("--capture-references");
    let capture_mirror_pair = mode.as_deref() == Some("--capture-mirror-pair");
    let capture_affine = mode.as_deref() == Some("--capture-affine") || capture_mirror_pair;
    let capture_adjustment = mode.as_deref() == Some("--capture-adjustment");
    let capture_mask_stroke = mode.as_deref() == Some("--capture-layer-mask-stroke");
    let capture_mask_fill = mode.as_deref() == Some("--capture-layer-mask-fill");
    let capture_mask_lasso = mode.as_deref() == Some("--capture-layer-mask-lasso");
    let capture_mask_paste = mode.as_deref() == Some("--capture-layer-mask-paste");
    let capture_mask_transform = mode.as_deref() == Some("--capture-layer-mask-transform");
    let capture_mask_target_transform = mode.as_deref() == Some("--capture-mask-target-transform");
    let capture_mask_feedback = mode.as_deref() == Some("--capture-mask-coverage");
    let capture_layer_effects = mode.as_deref() == Some("--capture-layer-effects");
    let capture_adjustment_layer = mode.as_deref() == Some("--capture-adjustment-layer");
    let capture_layer_mask = mode.as_deref() == Some("--capture-layer-mask")
        || capture_layer_effects
        || capture_adjustment_layer
        || capture_mask_transform
        || capture_mask_target_transform
        || capture_mask_feedback
        || capture_mask_stroke
        || capture_mask_fill
        || capture_mask_lasso
        || capture_mask_paste;
    let capture_canvas_size = mode.as_deref() == Some("--capture-canvas-size");
    let capture_popup = match mode.as_deref() {
        Some("--capture-file-menu") => Some(capture::Popup::FileMenu),
        Some("--capture-view-menu") => Some(capture::Popup::ViewMenu),
        Some("--capture-brush-picker") => Some(capture::Popup::Brushes),
        Some("--capture-color-picker") => Some(capture::Popup::Color),
        Some("--capture-mask-coverage") => Some(capture::Popup::Color),
        Some("--capture-long-toolbar-options") => Some(capture::Popup::StrokeOptions),
        Some("--capture-long-toolbar-brushes") => Some(capture::Popup::Brushes),
        _ => None,
    };
    let capture = if mode.as_deref() == Some("--capture-ui")
        || native_session
        || capture_large
        || capture_editor
        || capture_studio
        || capture_settings_recovery
        || capture_new_canvas
        || capture_long_toolbar
        || capture_extensions
        || capture_settings
        || capture_references
        || capture_reference_layers
        || capture_tabs
        || capture_rope
        || capture_layer_guides
        || capture_eraser
        || capture_fill
        || capture_tip_cursor
        || capture_timelapse
        || capture_theme_editor
        || capture_affine
        || capture_canvas_size
        || capture_layer_mask
        || capture_adjustment
        || capture_popup.is_some()
    {
        args.next().map(PathBuf::from)
    } else {
        None
    };
    if native_session {
        let Some(path) = &capture else {
            eprintln!(
                "Usage: {} NEW_REPORT.json [WIDTH HEIGHT SCALE]",
                if sustained_session {
                    "--soak-native-drawing-session"
                } else {
                    "--soak-native-mask-session"
                }
            );
            std::process::exit(1);
        };
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            eprintln!("Native session report must have a .json extension");
            std::process::exit(1);
        }
        if path.exists() || path.with_extension("png").exists() {
            eprintln!("Native session report or screenshot already exists");
            std::process::exit(1);
        }
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        if path.with_extension("allocations.json").exists() {
            eprintln!("Allocation trace sidecar already exists");
            std::process::exit(1);
        }
    }
    let width = args
        .next()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1280.0);
    let height = args
        .next()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(800.0);
    let scale = args
        .next()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(1.0, 2.0);
    let isolated = capture.is_some();
    let startup_error =
        |error: String| eframe::Error::AppCreation(Box::new(std::io::Error::other(error)));
    let mut settings_session = if isolated {
        None
    } else {
        let directory = eframe::storage_dir("Dudel \u{2014} Drawing Studio")
            .ok_or_else(|| startup_error("Settings directory is unavailable".into()))?;
        Some(settings_store::Session::open(directory.join("app.ron")).map_err(startup_error)?)
    };
    let capture_directory = if isolated {
        Some(tempfile::tempdir().map_err(|e| startup_error(e.to_string()))?)
    } else {
        None
    };
    let persistence_path = settings_session
        .as_ref()
        .map(|s| s.runtime.clone())
        .or_else(|| capture_directory.as_ref().map(|d| d.path().join("app.ron")));
    // Captures keep the default renderer so screenshots stay comparable.
    let display = if isolated {
        display_settings::DisplaySettings::default()
    } else {
        display_settings::DisplaySettings::load()
    }
    .with_env_override();
    let base = || eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Wayland app_id / X11 WM_CLASS, so compositors and launchers can match the window.
            .with_app_id("dudel")
            .with_decorations(false)
            .with_inner_size(if isolated {
                [width, height]
            } else {
                [1400.0, 950.0]
            })
            .with_min_inner_size([960.0, 640.0])
            .with_visible(!isolated || native_session),
        persist_window: !isolated,
        persistence_path: persistence_path.clone(),
        ..Default::default()
    };
    let creator = |display: display_settings::DisplaySettings| -> eframe::AppCreator<'static> {
        let capture = capture.clone();
        let settings_handle = settings_session.as_ref().map(|s| s.handle.clone());
        let settings_issue = settings_session.as_ref().and_then(|s| s.issue.clone());
        Box::new(move |cc| {
            if let Some(path) = capture {
                cc.egui_ctx
                    .memory_mut(|memory| *memory = egui::Memory::default());
                cc.egui_ctx.set_zoom_factor(scale);
                let mut app = Dudel::for_capture(path);
                if capture_large {
                    // DUDEL_CAPTURE_SIDE checks larger canvases (default 4096).
                    let side = std::env::var("DUDEL_CAPTURE_SIDE")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|side| {
                            side * side <= document::MAX_PIXELS && *side <= document::MAX_SIDE
                        })
                        .unwrap_or(4096);
                    app.doc = document::Document::new(side, side);
                    app.doc.layers[0].pixels = (0..side * side)
                        .flat_map(|i| {
                            let (x, y) = (i % side, i / side);
                            if x % 128 < 3 || y % 128 < 3 {
                                [30, 40, 50, 255]
                            } else {
                                [
                                    40 + (x / 20 % 200) as u8,
                                    40 + (y / 20 % 200) as u8,
                                    180,
                                    255,
                                ]
                            }
                        })
                        .collect::<Vec<_>>()
                        .into();
                    app.zoom = if capture_large_detail { 1.0 } else { 0.0 };
                    app.auto_fit = !capture_large_detail;
                    app.rotation = if capture_large_detail { 0.37 } else { 0.0 };
                    app.mirror = capture_large_detail;
                    app.texture = Default::default();
                    app.dirty = true;
                    app.thumbnail_revision = None;
                }
                if let Ok(name) = std::env::var("DUDEL_CAPTURE_THEME")
                    && let Some(theme) = themes::Theme::presets()
                        .into_iter()
                        .find(|t| t.name == name)
                {
                    app.theme = theme;
                }
                app.timelapse.open = capture_timelapse;
                if capture_timelapse && let Err(error) = app.sample_timelapse() {
                    eprintln!("Timelapse sample: {error}");
                }
                if capture_layer_guides {
                    app.symmetry.mode = symmetry::Mode::LeftRight;
                    let id = app.doc.layers[app.doc.active].id;
                    app.layer_symmetry.insert(
                        id,
                        symmetry::Symmetry {
                            mode: symmetry::Mode::TopBottom,
                            ..Default::default()
                        },
                    );
                    app.push_ruler(rulers::Ruler::Line {
                        a: [150.0, 150.0],
                        b: [650.0, 450.0],
                    });
                    *app.ruler_owners.last_mut().unwrap() = Some(id);
                    app.inspector_override = Some(true);
                    app.tool_panel_override = Some(false);
                    app.status =
                        "Layer symmetry and guide controls · global settings remain independent"
                            .into();
                }
                if capture_theme_editor {
                    // A distinct surround proves this is a view color in captures.
                    app.theme.canvas_background = [92, 96, 104];
                    app.begin_theme_editor();
                }
                if capture_reference_layers {
                    for index in 0..3 {
                        let mut rgba = vec![0; 64 * 48 * 4];
                        for pixel in rgba.chunks_exact_mut(4) {
                            pixel.copy_from_slice(&[80 + index * 45, 120, 180, 255]);
                        }
                        app.insert_reference(
                            format!(
                                "Reference study {} with a deliberately long descriptive name",
                                index + 1
                            ),
                            reference_layers::Image {
                                width: 64,
                                height: 48,
                                rgba,
                            },
                        );
                    }
                    app.reference_layers.layers[0].above = false;
                    if capture_reference_boards {
                        app.reference_layers.layers.pop();
                        app.doc
                            .layers
                            .push(document::Layer::new_folder("Character references".into()));
                        let folder = app.doc.layers.len() - 1;
                        for board in 0..2 {
                            let id = app.reference_layers.layers[board].id;
                            app.reference_layers.layers[board].name = if board == 0 {
                                "Hair references"
                            } else {
                                "Boots references"
                            }
                            .into();
                            let count = if board == 0 { 12 } else { 6 };
                            for i in 1..count {
                                let mut rgba = vec![0; 64 * 48 * 4];
                                for (p, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                                    let shade = if (p / 64 + p % 64 + i * 3) % 17 < 4 {
                                        40
                                    } else {
                                        120
                                    };
                                    pixel.copy_from_slice(&[shade, 80 + i as u8 * 8, 160, 255]);
                                }
                                app.insert_reference_image(
                                    id,
                                    format!("Study {}", i + 1),
                                    reference_layers::Image {
                                        width: 64,
                                        height: 48,
                                        rgba,
                                    },
                                )
                                .unwrap();
                            }
                            app.arrange_reference_images(id);
                            let root = &mut app.reference_layers.layers[board];
                            let mut images = std::mem::take(&mut root.images);
                            for (i, image) in std::iter::once(&mut *root)
                                .chain(images.iter_mut())
                                .enumerate()
                            {
                                image.center = [
                                    70.0 + (i % 3) as f32 * 80.0,
                                    80.0 + board as f32 * 380.0 + (i / 3) as f32 * 75.0,
                                ];
                                image.scale = 1.0;
                            }
                            root.images = images;
                            app.place_reference(id, folder, 1);
                        }
                        app.select_reference(app.reference_layers.layers[0].id);
                    }
                    if capture_preview {
                        // Paint a mark on a layer the preview leaves out.
                        app.doc.layers.push(document::Layer::raster(
                            "Private notes".into(),
                            app.doc.width,
                            app.doc.height,
                        ));
                        let (w, notes) = (app.doc.width, app.doc.layers.len() - 1);
                        app.doc.layers[notes].pixels.writer().write_region(
                            w,
                            [w / 2 - 60, 40, w / 2 + 60, 80],
                            &[220, 30, 30, 255].repeat(120 * 40),
                        );
                        app.preview.hidden.insert(app.doc.layers[notes].id);
                        app.preview.open = true;
                        app.preview.settings.show_pointer = true;
                        app.preview.settings.background = preview_window::Background::Green;
                        let main = app.capture.as_ref().unwrap().path.clone();
                        let stem = main.file_stem().unwrap_or_default().to_string_lossy();
                        app.preview.capture =
                            Some(main.with_file_name(format!("{stem}-preview.png")));
                        app.changed();
                    }
                }
                if capture_rope {
                    app.toggle_rope_stabilizer();
                    app.brush.rope_distance = Some(80.0);
                    app.tool_panel_override = Some(true);
                }
                if capture_tip_cursor {
                    if let Some(index) = app.brushes.iter().position(|b| b.name == "Flat bristle") {
                        app.choose_brush(index);
                    }
                    app.brush.size = 160.0;
                    app.brush.angle = 30.0;
                    app.capture.as_mut().unwrap().pointer = Some(egui::pos2(700.0, 420.0));
                }
                if capture_eraser {
                    app.select_tool(tools::Tool::Eraser);
                    app.set_eraser_own(true);
                    app.set_show_transparency(true);
                }
                if capture_fill {
                    app.doc.active = 0;
                    app.color = [231, 142, 92];
                    app.brush.opacity = 1.0;
                    app.symmetry.mode = symmetry::Mode::Off;
                    app.doc
                        .fill_request(
                            fill::Request {
                                point: [512, 384],
                                color: app.color,
                                opacity: 1.0,
                                tolerance: app.fill_tolerance,
                                source: fill::Source::Active,
                                options: app.fill_options,
                            },
                            None,
                        )
                        .map_err(std::io::Error::other)?;
                    app.select_tool(tools::Tool::Fill);
                    app.tool_panel_override = Some(true);
                    app.inspector_override = Some(false);
                    app.status = "Soft edge fill · original blue linework preserved".into();
                }
                if capture_tabs {
                    app.path = Some(PathBuf::from("Portrait.dudel"));
                    if capture_project_memory {
                        app.insert_reference(
                            "Parked color study".into(),
                            reference_layers::Image {
                                width: 256,
                                height: 256,
                                rgba: [50, 120, 170, 255].repeat(256 * 256),
                            },
                        );
                        app.reference_layers.active = None;
                        app.performance.open = true;
                    }
                    app.modified = false;
                    for (name, modified) in [("Background study", true), ("", false)] {
                        app.prepare_tab_for_open();
                        app.apply_action(document_actions::Action::New(800, 600, 300.0));
                        if !name.is_empty() {
                            app.path = Some(PathBuf::from(format!("{name}.dudel")));
                            let w = app.doc.width;
                            app.doc.layers[0].pixels.writer().write_region(
                                w,
                                [100, 100, 700, 500],
                                &[90, 140, 110, 255].repeat(600 * 400),
                            );
                            app.changed();
                            app.modified = modified;
                        }
                    }
                    app.request_tab(1);
                }
                app.capture.as_mut().unwrap().popup = capture_popup;
                app.status = format!("Renderer: {}", display_settings::active_description(cc));
                if capture_adjustment {
                    app.doc.active = 0;
                    app.start_adjustment(adjustments::Kind::HueSaturation);
                    app.adjustment.as_mut().unwrap().settings =
                        adjustments::Settings::HueSaturation {
                            hue: 35.0,
                            saturation: 20.0,
                            lightness: 0.0,
                        };
                }
                if capture_layer_mask {
                    let (width, height) = (app.doc.width, app.doc.height);
                    let bytes: Vec<u8> = (0..width * height)
                        .map(|i| {
                            let x = i % width;
                            (255.0
                                * ((x as f32 - width as f32 / 2.0 + 48.0) / 96.0).clamp(0.0, 1.0))
                            .round() as u8
                        })
                        .collect();
                    app.doc.version = 2;
                    app.doc.active = 0;
                    app.doc.layers[0].mask = Some(layer_mask::LayerMask {
                        id: crate::document::LayerId::new(),
                        enabled: true,
                        linked: true,
                        coverage: layer_mask::Mask::from_coverage(width, height, &bytes, None)
                            .expect("capture mask geometry"),
                    });
                }
                if capture_layer_effects {
                    // A masked layer with effects, their window open and the
                    // mask shown on the canvas.
                    let mask = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.doc.layers[0].effects = ["Drop Shadow", "Stroke"]
                        .into_iter()
                        .filter_map(layer_effects::Effect::preset)
                        .map(layer_effects::LayerEffect::new)
                        .collect();
                    app.mask_focus = Some((app.doc.layers[0].id, mask));
                    app.mask_view = true;
                    app.effects_window_open = true;
                }
                if capture_adjustment_layer {
                    // A curves layer over the artwork with its window open.
                    let (width, height) = (app.doc.width, app.doc.height);
                    let mut layer =
                        crate::document::Layer::raster("Curves 1".into(), width, height);
                    layer.adjustment =
                        Some(adjustment::Adjustment::new(adjustment::Settings::Curves {
                            channels: vec![(0, vec![[0, 0], [70, 40], [190, 225], [255, 255]])],
                            bezier: false,
                        }));
                    app.doc.layers.push(layer);
                    app.doc.active = app.doc.layers.len() - 1;
                    app.adjustment_window_open = true;
                }
                if capture_mask_feedback {
                    let id = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.mask_focus = Some((app.doc.layers[0].id, id));
                    app.mask_colors[0] = 96;
                    app.color = [80, 132, 187];
                    app.status = "Editing mask: black conceals, white reveals".into();
                }
                if capture_mask_stroke {
                    let mask_id = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.mask_focus = Some((app.doc.layers[0].id, mask_id));
                    let settings = stroke_buffer::Settings {
                        target: paint_target::Target::Mask(mask_id),
                        brush: brush::Brush {
                            size: 48.0,
                            hardness: 0.8,
                            opacity: 0.7,
                            flow: 1.0,
                            pressure_size: false,
                            taper_out: 20.0,
                            ..Default::default()
                        },
                        color: [0; 3],
                        erase: false,
                        layer: 0,
                        symmetry: Default::default(),
                    };
                    let id = app
                        .render_queue
                        .begin_layer(std::sync::Arc::new(settings), app.doc.layers[0].id)
                        .unwrap();
                    for step in 0..64 {
                        app.render_queue
                            .dab(
                                id,
                                stroke::Sample {
                                    x: 650.5,
                                    y: 285.5 + step as f32 * 2.0,
                                    pressure: 1.0,
                                    distance: step as f64 * 2.0,
                                    ..Default::default()
                                },
                            )
                            .unwrap();
                    }
                    app.render_queue.end(id).unwrap();
                    app.status = "Mask stroke rendering fixture".into();
                }
                if capture_mask_fill {
                    let id = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.mask_focus = Some((app.doc.layers[0].id, id));
                    app.select_tool(tools::Tool::Fill);
                    app.color = [0; 3];
                    app.brush.opacity = 0.6;
                    app.fill_canvas(650, 300);
                }
                if capture_mask_lasso {
                    let id = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.mask_focus = Some((app.doc.layers[0].id, id));
                    app.select_tool(tools::Tool::Lasso);
                    app.lasso.erase = false;
                    app.color = [0; 3];
                    app.brush.opacity = 0.7;
                    let points = [[560.0, 230.0], [720.0, 350.0], [560.0, 530.0]];
                    for (i, point) in points.into_iter().enumerate() {
                        app.update_selection_drag(
                            point,
                            [point[0] as usize, point[1] as usize],
                            i == 0,
                            i == 2,
                            true,
                        );
                    }
                }
                if capture_mask_paste {
                    let id = app.doc.layers[0].mask.as_ref().unwrap().id;
                    app.mask_focus = Some((app.doc.layers[0].id, id));
                    let (width, height) = (320, 240);
                    let rgba = (0..width * height)
                        .flat_map(|i| {
                            let value = match (i % width) * 3 / width {
                                0 => 255,
                                1 => 0,
                                _ => 128,
                            };
                            [value, value, value, 192]
                        })
                        .collect();
                    app.capture_mask_paste(clipboard::Raster {
                        width,
                        height,
                        rgba,
                    })
                    .expect("capture mask paste");
                }
                if capture_canvas_size {
                    app.open_canvas_size();
                    app.canvas_resize = Some(canvas_size::Settings {
                        dpi: None,
                        width: 1280,
                        height: 960,
                        anchor: [1, 1],
                    });
                }
                if capture_editor {
                    app.tool_panel_override = Some(true);
                    app.brush_panel_page =
                        match std::env::var("DUDEL_CAPTURE_BRUSH_PAGE").as_deref() {
                            Ok("tip") => tool_panel::BrushPage::Tip,
                            Ok("texture") => tool_panel::BrushPage::Texture,
                            Ok("more") => tool_panel::BrushPage::More,
                            _ => tool_panel::BrushPage::Brush,
                        };
                }
                if capture_studio {
                    app.brush_studio = true;
                    app.brush_studio_section = brush_studio::Section::Stabilizer;
                    app.brush.stabilization = 12.0;
                }
                if capture_settings_recovery {
                    app.restore_preferences_source(preference_recovery::CAPTURE_SOURCE);
                }
                if capture_new_canvas {
                    app.open_new_canvas();
                    if capture_new_canvas_invalid {
                        app.new_canvas_form.width = "4096".into();
                        app.new_canvas_form.height = "4096".into();
                    }
                }
                if capture_long_toolbar {
                    app.brush.name =
                        "Detailed illustration brush with pressure, texture and custom blending"
                            .repeat(2);
                    app.brush.opacity = 0.63;
                }
                if capture_extensions {
                    app.extension_manager = true;
                    if let Ok(package) = serde_json::from_str(include_str!(
                        "../plugins/example-studio-kit.dudelplugin"
                    )) {
                        app.extensions.push(extensions::Installed {
                            package,
                            enabled: true,
                        });
                    }
                }
                if capture_settings {
                    app.settings_open = true;
                }
                if capture_references {
                    app.reference_capture_fixture(&cc.egui_ctx);
                }
                if capture_affine || capture_mask_transform || capture_mask_target_transform {
                    app.doc.active = 0;
                    if capture_mask_target_transform {
                        app.mask_focus = Some((
                            app.doc.layers[0].id,
                            app.doc.layers[0].mask.as_ref().unwrap().id,
                        ));
                    }
                    if capture_mirror_pair {
                        app.doc = document::Document::new(640, 480);
                        let mut writer = app.doc.layers[0].pixels.writer();
                        for y in 170..250 {
                            for x in 140..500 {
                                let cx = if x < 320 { 200.0 } else { 440.0 };
                                let dx = x as f64 - cx;
                                let dy = y as f64 - 220.0;
                                let ellipse = (dx / 54.0).powi(2) + (dy / 24.0).powi(2);
                                let pixel = if (174..180).contains(&y) && dx.abs() < 48.0 {
                                    Some([60, 42, 38, 255])
                                } else if ellipse <= 1.0 {
                                    Some(if ellipse > 0.83 {
                                        [50, 40, 45, 255]
                                    } else if dx * dx + dy * dy < 60.0 {
                                        [25, 30, 35, 255]
                                    } else if dx * dx + dy * dy < 240.0 {
                                        if x < 320 {
                                            [55, 145, 115, 255]
                                        } else {
                                            [65, 125, 185, 255]
                                        }
                                    } else {
                                        [245, 240, 235, 255]
                                    })
                                } else {
                                    None
                                };
                                if let Some(pixel) = pixel {
                                    writer.rgba_mut(y * 640 + x).copy_from_slice(&pixel);
                                }
                            }
                        }
                        app.doc.selection = Some(selection::Selection {
                            left: 135,
                            top: 165,
                            right: 265,
                            bottom: 255,
                        });
                        app.tool_panel_override = Some(true);
                        app.inspector_override = Some(false);
                    } else if capture_affine {
                        app.doc.selection = Some(selection::Selection {
                            left: 304,
                            top: 176,
                            right: 720,
                            bottom: 592,
                        });
                    }
                    app.affine_settings = affine::Settings {
                        scale_percent: [120.0, 80.0],
                        angle_degrees: 25.0,
                        ..affine::Settings::default()
                    };
                    if capture_mirror_pair {
                        app.affine_settings.mirror = Some(affine::Mirror {
                            center: [320., 240.],
                            angle_degrees: 0.,
                        });
                        app.affine_settings.angle_degrees = 10.0;
                        app.affine_settings.translation = [-12., 0.];
                    }
                    app.select_tool(tools::Tool::Move);
                    app.start_affine_preview();
                }
                if native_session {
                    let session = native_session::Session::configure(
                        &mut app,
                        display_settings::active_description(cc),
                        sustained_session,
                    )
                    .map_err(std::io::Error::other)?;
                    #[cfg(feature = "allocation-profile")]
                    let session = session.with_gpu_device(
                        cc.wgpu_render_state
                            .as_ref()
                            .map(|state| state.device.clone()),
                    );
                    app.capture.as_mut().unwrap().session = Some(session);
                }
                Ok(Box::new(app))
            } else {
                let mut app = Dudel::load_preferences(cc.storage);
                app.settings_store = settings_handle;
                if let Some(issue) = settings_issue {
                    app.protect_settings_container(issue.path, issue.reason);
                }
                app.display_settings = display;
                app.display_active = display_settings::active_description(cc);
                app.experimental_indexed_storage = experimental_indexed_storage;
                #[cfg(any(windows, target_os = "linux"))]
                app.input.attach(cc);
                #[cfg(windows)]
                match clipboard_shortcuts::Backend::attach(cc) {
                    Ok(backend) => app.clipboard_shortcuts = Some(backend),
                    Err(error) => app.status = format!("Image paste shortcut unavailable: {error}"),
                }
                Ok(Box::new(app))
            }
        })
    };
    let title = if native_session {
        "Dudel — Isolated native diagnostic"
    } else {
        "Dudel — Drawing Studio"
    };
    let mut result = eframe::run_native(title, display.native_options(base()), creator(display));
    if result.is_err() && display.renderer != display_settings::Renderer::OpenGl {
        // The chosen GPU API could not start on this PC: open with OpenGL and
        // remember that, so the app never fails to start over a setting.
        eprintln!(
            "{:?} renderer failed ({result:?}); falling back to OpenGL",
            display.renderer
        );
        let fallback = display_settings::DisplaySettings {
            renderer: display_settings::Renderer::OpenGl,
            ..display
        };
        if !isolated {
            let _ = fallback.save();
        }
        result = eframe::run_native(title, fallback.native_options(base()), creator(fallback));
    }
    if native_session && result.is_ok() && capture.as_ref().is_some_and(|p| !p.is_file()) {
        eprintln!("Native diagnostic did not produce a verified report");
        std::process::exit(1);
    }
    if let Some(session) = &mut settings_session
        && let Err(error) = session.finish()
    {
        rfd::MessageDialog::new()
            .set_title("Dudel settings could not be saved")
            .set_description(&error)
            .set_level(rfd::MessageLevel::Error)
            .show();
        if result.is_ok() {
            result = Err(startup_error(error));
        }
    }
    result
}

#[cfg(test)]
mod edit_order_tests {
    use super::*;
    pub(crate) fn pending_taper() -> (Dudel, Document) {
        let mut doc = Document::new(33, 35);
        doc.add_layer();
        doc.active = 0;
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 8, 7, 0]);
        let settings = stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Brush {
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
        let sample = stroke::Sample {
            x: 7.5,
            y: 9.5,
            pressure: 1.0,
            ..Default::default()
        };
        let mut expected = doc.clone();
        let mut reference = stroke_buffer::StrokeBuffer::default();
        reference.paint(&mut expected, sample, &settings);
        reference.finish_tail(&mut expected, &settings);
        let mut app = Dudel {
            doc,
            ..Default::default()
        };
        app.history.begin_stroke_checkpoint(&app.doc);
        app.stroking = true;
        app.stroke_settings = Some(settings.clone());
        app.stroke_buffer = Some(Default::default());
        app.stroke_buffer
            .as_mut()
            .unwrap()
            .paint(&mut app.doc, sample, &settings);
        assert_ne!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        (app, expected)
    }
    #[test]
    fn selection_changes_settle_taper_before_their_own_history_action() {
        let (mut app, settled) = pending_taper();
        app.select_all();
        assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
        assert!(!app.stroking && !app.history.has_pending_stroke());
        assert!(app.doc.selection.is_some());
        app.undo();
        assert!(app.doc.selection.is_none());
        assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [9, 8, 7, 0]);
        assert_eq!(app.doc.layers[0].pixels.rgba(9 * 33 + 7), [0; 4]);
        app.redo();
        assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
        app.redo();
        assert!(app.doc.selection.is_some());
    }
    #[test]
    fn transform_and_clear_use_settled_pixels_as_their_undo_source() {
        for clear in [false, true] {
            let (mut app, settled) = pending_taper();
            let mut expected = settled.clone();
            if clear {
                expected.clear_selected().unwrap();
                app.clear_selection_pixels();
            } else {
                expected
                    .transform_active(transform::Transform::FlipHorizontal)
                    .unwrap();
                app.transform_active(transform::Transform::FlipHorizontal);
            }
            assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
            app.undo();
            assert_eq!(app.doc.layers[0].pixels, settled.layers[0].pixels);
            app.redo();
            assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
            assert!(!app.history.has_pending_stroke());
        }
    }
}
