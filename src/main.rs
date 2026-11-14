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
mod blend_brush;
mod brush;
mod brush_edge;
mod brush_formula;
mod brush_graph;
mod brush_library;
mod brush_pack;
mod brush_previews;
mod brush_share;
mod brush_studio;
mod canvas;
mod canvas_size;
mod canvas_textures;
mod capture;
mod capture_fixtures;
mod cli;
mod clip_attributes;
mod clip_blocks;
mod clip_container;
mod clip_document;
mod clip_metadata;
mod clip_preview;
mod clip_raster;
mod clip_text;
mod clip_timeline;
mod clip_vector;
mod clipboard;
#[cfg(windows)]
mod clipboard_files;
#[cfg(windows)]
mod clipboard_shortcuts;
mod color_wheel;
mod composite_cursor;
mod dab_bands;
mod dialogs;
mod display_settings;
mod display_workers;
mod document;
mod document_actions;
mod document_edits;
mod dynamics;
mod effects_panel;
mod export_jobs;
mod export_options;
mod extension_api;
mod extensions;
mod file_preview;
mod fill;
mod filters;
mod folders;
#[cfg(feature = "allocation-profile")]
mod gpu_profile;
mod gradient_tool;
mod graph_editor;
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
mod install;
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
mod pattern;
mod psd_export;
mod psd_import;
mod psd_vector;
mod quick_menu;
#[cfg(target_os = "linux")]
use wayland_pen as native_pen;
mod background;
mod command_search;
mod commands;
mod crash_report;
mod editing_feedback;
mod layer_batch;
mod layer_menu;
mod layer_navigation;
mod material;
mod native_session;
mod navigator;
mod new_canvas;
mod paint_target;
mod palette;
mod palette_assist;
mod paper;
mod paper_view;
mod parallel;
mod pending_layers;
mod performance;
mod pigment;
mod pixels;
mod plugins;
mod preference_recovery;
mod preferences;
mod prefix_cache;
mod pressure_editor;
mod preview;
mod preview_window;
mod project_package;
mod quick_resize;
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
mod scripted_tool;
mod selection;
mod selection_contour;
mod selection_mask;
mod selection_ui;
mod settings_store;
mod settings_ui;
mod setup_ui;
mod sha256;
mod shortcut_editor;
mod shortcuts;
mod snapshot_memory;
mod stage_timer;
mod storage;
mod stroke;
mod stroke_buffer;
mod stroke_profile;
mod svg_export;
mod symmetry;
mod tabs;
mod text_layer;
mod text_options;
mod text_render;
mod text_tool;
mod themes;
mod thumbnails;
mod tile_project;
mod timelapse;
mod tip_cursor;
mod tip_import;
mod tool_panel;
mod tool_shift;
mod tools;
#[cfg(windows)]
mod touch_keyboard;
mod transform;
mod update;
mod vector_layer;
mod vector_pick;
mod view;
mod wand;
mod wasm_filters;
#[cfg(target_os = "linux")]
mod wayland_pen;
mod welcome;
mod wet;
mod window_chrome;
#[cfg(test)]
mod workflow_tests;
mod workspace;
mod workspace_layouts;
use tools::Tool;

use brush::Brush;
use document::{Document, History};
use eframe::egui;
use std::path::PathBuf;

/// Layer panel previews by layer identity.
type Thumbnails = std::collections::HashMap<document::LayerId, egui::TextureHandle>;

struct Dudel {
    shortcuts: shortcuts::Bindings,
    shortcut_editor: shortcut_editor::Editor,
    command_search: command_search::Search,
    layer_navigation: layer_navigation::Navigation,
    layer_selection: layer_batch::Selection,
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
    palette_assist: palette_assist::Assist,
    tool: Tool,
    /// Lasso fill/erase mode and outline shape.
    lasso: lasso_paint::Options,
    gradient: gradient_tool::Options,
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
    layer_thumbnails: Thumbnails,
    mask_thumbnails: Thumbnails,
    layer_reveal: Option<(document::LayerId, paint_target::Target, [i32; 2])>,
    thumbnail_revision: Option<u64>,
    thumbnail_timer: std::time::Instant,
    /// Pixel identity each thumbnail was drawn from, by layer.
    thumbnail_stamps: std::collections::HashMap<
        document::LayerId,
        (bool, pixels::RegionStamp, layer_mask::LayerStamp),
    >,
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
    text_edit: Option<text_tool::Editing>,
    text_drag: Option<text_tool::Drag>,
    text_style: text_options::Style,
    /// The pen quick menu, while open, and the commands it offers.
    quick_menu: Option<quick_menu::QuickMenu>,
    quick_menu_prefs: quick_menu::QuickMenuPrefs,
    /// How the next image export is written; a preference.
    export_settings: export_options::ExportSettings,
    export_window: export_options::ExportWindow,
    /// The project's last image export, which Export again repeats.
    last_export: Option<export_options::LastExport>,
    text_extents: text_tool::Extents,
    text_drawer: text_tool::Drawer,
    /// The Pointer or Text tool is dragging a text layer, through the Move
    /// tool's path.
    text_moving: bool,
    /// Where that press began, until the drag is told from a click.
    text_press: Option<[f32; 2]>,
    /// The text layer last pressed with the Pointer, and when.
    text_click: Option<(document::LayerId, f64)>,
    /// The artist agreed that the open transform may rasterize text.
    text_transform_confirmed: bool,
    held_transform: Option<affine_preview::HeldTransform>,
    font_list: text_options::FontList,
    adjustment_edit: Option<adjustment_panel::Edit>,
    /// Adjustment settings last sent through the busy render queue.
    adjustment_sent: Option<(document::LayerId, adjustment::Settings)>,
    adjustment_window_open: bool,
    /// An extension's scripted tool, drawing with the Brush or Eraser.
    scripted_tool: Option<scripted_tool::Active>,
    /// Its script's state during a stroke.
    script_stroke: Option<scripted_tool::ScriptedStroke>,
    /// The stroke being drawn on a vector layer, as it will be kept.
    vector_record: Option<vector_layer::Stroke>,
    /// The vector eraser while it is dragged.
    vector_erase: Option<vector_layer::Erase>,
    /// What the eraser takes from vector layers.
    vector_erase_mode: vector_layer::EraseMode,
    /// Lines of a vector layer picked with the Pointer tool.
    vector_pick: vector_pick::Pick,
    /// The Pointer's drag on empty canvas lassoes vector lines (else pans).
    pointer_lasso: bool,
    /// What the Pointer's drag on a vector line does.
    line_drag: vector_pick::LineDrag,
    /// How far along a line a reshape reaches, in canvas pixels.
    bend_reach: f32,
    /// F resizing the brush or reach, while it lasts.
    quick_resize: Option<quick_resize::QuickResize>,
    /// Counts shown in the Levels settings of the Adjustment window.
    levels_histogram: adjustment_panel::Histogram,
    /// The effects list last sent through the busy render queue, which the
    /// window builds on until the queue has published it.
    effects_sent: Option<(document::LayerId, effects_panel::Style)>,
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
    /// The right panel shows the navigator instead of color (its tabs).
    inspector_navigator: bool,
    /// Dragged widths of the tool property panel and the inspector.
    sidebar_widths: [Option<f32>; 2],
    /// Brush Studio's node editor: view, selection and its own undo steps.
    graph_editor: graph_editor::GraphEditor,
    /// Brush Studio opens as a tab beside the projects rather than a window;
    /// whether that tab is the one showing; whether the Studio was open last
    /// frame, to bring the tab forward when it opens.
    studio_as_tab: bool,
    studio_tab_active: bool,
    studio_was_open: bool,
    /// The Studio's test pad strip is hidden to give the settings room.
    studio_pad_hidden: bool,
    /// Sharing brushes: the open Export and Import dialogs, and the author
    /// and license last used, offered again next time.
    /// Wet paint drying on the open project.
    drying: wet::Drying,
    /// The open Canvas > Paper dialog.
    paper_form: Option<paper::PaperForm>,
    material_form: Option<material::MaterialForm>,
    /// A thick paint light being tried in the Material dialog: shown on
    /// the canvas, not yet in the document.
    material_preview: Option<(document::LayerId, material::Settings)>,
    paper_view: paper_view::PaperView,
    brush_export: Option<brush_share::ExportForm>,
    brush_import: Option<brush_share::PendingImport>,
    brush_author: String,
    brush_license: String,
    /// Named panel arrangements, and the name being typed for a new one.
    workspaces: Vec<workspace_layouts::Workspace>,
    workspace_name: String,
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
    /// Settings window search text; not saved.
    settings_search: String,
    /// Layer effect drawings still being made in the background.
    effects_waiting: bool,
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
    /// The welcome window is showing.
    welcome_open: bool,
    /// Open the welcome window when Dudel starts (a preference).
    show_welcome: bool,
    /// The Updates and Install windows.
    setup: setup_ui::Setup,
}

impl Default for Dudel {
    fn default() -> Self {
        Self {
            shortcuts: Default::default(),
            shortcut_editor: Default::default(),
            command_search: Default::default(),
            layer_navigation: Default::default(),
            layer_selection: Default::default(),
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
            palette_assist: Default::default(),
            tool: Tool::Brush,
            lasso: Default::default(),
            gradient: Default::default(),
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
            layer_thumbnails: Default::default(),
            mask_thumbnails: Default::default(),
            layer_reveal: None,
            thumbnail_revision: None,
            thumbnail_timer: std::time::Instant::now(),
            thumbnail_stamps: Default::default(),
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
            text_edit: None,
            text_drag: None,
            text_style: Default::default(),
            export_settings: Default::default(),
            quick_menu: None,
            quick_menu_prefs: Default::default(),
            export_window: Default::default(),
            last_export: None,
            text_extents: Default::default(),
            text_drawer: Default::default(),
            text_moving: false,
            text_press: None,
            text_click: None,
            text_transform_confirmed: false,
            held_transform: None,
            font_list: Default::default(),
            adjustment_edit: None,
            adjustment_sent: None,
            adjustment_window_open: false,
            scripted_tool: None,
            script_stroke: None,
            vector_record: None,
            vector_erase: None,
            vector_erase_mode: Default::default(),
            vector_pick: Default::default(),
            pointer_lasso: true,
            line_drag: Default::default(),
            bend_reach: 40.0,
            quick_resize: None,
            levels_histogram: Default::default(),
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
            inspector_navigator: false,
            sidebar_widths: [None; 2],
            graph_editor: Default::default(),
            studio_as_tab: true,
            studio_tab_active: false,
            studio_was_open: false,
            studio_pad_hidden: false,
            drying: Default::default(),
            paper_form: None,
            material_form: None,
            material_preview: None,
            paper_view: Default::default(),
            brush_export: None,
            brush_import: None,
            brush_author: String::new(),
            brush_license: String::new(),
            workspaces: Vec::new(),
            workspace_name: String::new(),
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
            settings_search: String::new(),
            effects_waiting: false,
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
            welcome_open: false,
            show_welcome: true,
            setup: Default::default(),
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
    /// Select everything outside the selection, keeping soft edges soft.
    fn invert_selection(&mut self) {
        if self.doc.selection.is_none() && self.doc.selection_mask.is_none() {
            self.status = "Nothing is selected to invert".into();
            return;
        }
        self.dispatch_document_edit(
            document_edits::Edit::InvertSelection,
            "Selection inverted",
            "Selection unchanged",
        );
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
        self.workspaces = p.workspaces;
        self.studio_as_tab = p.studio_as_tab;
        self.brush_author = p.brush_author.chars().take(100).collect();
        self.brush_license = p.brush_license.chars().take(100).collect();
        self.recent = p.recent;
        self.secondary = p.secondary;
        self.recent_colors = p.recent_colors;
        self.text_style = p.text;
        self.export_settings = p.export.sanitized();
        self.quick_menu_prefs = p.quick_menu.sanitized();
        self.show_welcome = p.welcome;
        self.set_show_transparency(p.show_transparency);
        self.lasso = p.tools.lasso;
        self.gradient = p.tools.gradient.clone();
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
        self.new_canvas_form.prefs = p.new_canvas.sanitized();
        self.shortcuts = p.shortcuts;
        if p.shortcuts_dropped > 0 {
            self.status = format!(
                "{} saved shortcut(s) clashed and went back to their defaults",
                p.shortcuts_dropped
            );
        }
        if p.graphs_dropped > 0 {
            self.status = format!(
                "{} saved brush graph(s) were damaged and were removed; those brushes paint with their plain settings",
                p.graphs_dropped
            );
        }
        let finite = |h: Option<f32>| h.filter(|h| h.is_finite() && *h > 0.0);
        self.library_height = finite(p.layout.library_height);
        self.color_height = finite(p.layout.color_height);
        self.navigator_height = finite(p.layout.navigator_height);
        self.inspector_navigator = p.layout.navigator_tab;
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
            text: self.text_style.clone(),
            export: self.export_settings,
            quick_menu: self.quick_menu_prefs.clone(),
            welcome: self.show_welcome,
            show_transparency: self.show_transparency,
            tools: preferences::ToolPrefs {
                lasso: self.lasso,
                gradient: self.gradient.clone(),
                brush_group: self.brush_group,
                group_presets: self.group_presets.clone(),
                eraser_own: self.eraser_own,
                eraser: eraser.validate().is_ok().then(|| eraser.clone()),
            },
            storage: self.locations.clone(),
            preview: self.preview.settings.clone(),
            tabs: preferences::TabPrefs { max: self.tabs.max },
            timelapse: self.timelapse.prefs.clone(),
            new_canvas: self.new_canvas_form.prefs.clone(),
            workspaces: self.workspaces.clone(),
            studio_as_tab: self.studio_as_tab,
            brush_author: self.brush_author.clone(),
            brush_license: self.brush_license.clone(),
            shortcuts: self.shortcuts.clone(),
            shortcuts_dropped: 0,
            graphs_dropped: 0,
            layout: preferences::LayoutPrefs {
                library_height: self.library_height,
                color_height: self.color_height,
                navigator_height: self.navigator_height,
                navigator_tab: self.inspector_navigator,
                layers_height: self.layers_height,
            },
        };
        serde_json::to_string(&p).map_err(|e| e.to_string())
    }
    fn export_psd(&mut self, effects: psd_export::Effects) {
        if self.adjustment.is_some() {
            self.status = "Apply or cancel the adjustment before exporting".into();
            return;
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            self.status = "An export is already running".into();
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
                .add_filter("Photoshop document", &["psd"])
                .set_file_name(format!("{name}.psd")),
        );
        self.ask_save_file(dialog, move |app, path| app.export_psd_to(path, effects));
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
        let worker = background::spawn(move || {
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
        #[cfg(any(windows, target_os = "linux"))]
        self.input.raw_input_hook(raw_input, ctx.pixels_per_point());
        if self.dialogs.native_open() {
            // Nothing in the window answers a system dialog.
            raw_input.events.clear();
            return;
        }
        if self.capture_shortcut_input(raw_input) {
            return;
        }
        // Ctrl+Tab and Ctrl+Shift+Tab step through open projects.
        raw_input.events.retain(|event| match event {
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed,
                modifiers,
                ..
            } if modifiers.command && !self.command_search.open => {
                if *pressed {
                    self.tabs.cycle += if modifiers.shift { -1 } else { 1 };
                }
                false
            }
            _ => true,
        });
        if !self.new_canvas
            && !self.dialogs.open()
            && !raw_input
                .events
                .iter()
                .any(|event| self.shortcuts.event_matches(commands::Action::New, event))
            && !self.command_search.open
            && workspace::take_panel_toggle(ctx, raw_input)
            && self
                .shortcuts
                .effective(commands::Action::Panels)
                .iter()
                .any(|c| c.key == egui::Key::Tab && !c.command && !c.shift && !c.alt)
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
            && self.text_edit.is_none()
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
        self.brush_share_ui(ctx);
        self.paper_dialog(ctx);
        self.material_dialog(ctx);
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
        self.trace_live_frame(frame_start);
    }
}

fn main() -> eframe::Result {
    // A plain launch is an artist's session: keep a report of any panic and,
    // if one closes the window, say so instead of vanishing. Command-line
    // and diagnostic modes report on stderr as before.
    // Projects to open (from File Explorer) are still a plain launch.
    let plain = std::env::args_os()
        .nth(1)
        .is_none_or(|first| !first.to_string_lossy().starts_with("--"));
    if !plain {
        return run();
    }
    install::startup();
    crash_report::install();
    stage_timer::start_live_from_env();
    match std::panic::catch_unwind(run) {
        Ok(result) => result,
        Err(_) => {
            crash_report::tell_artist();
            std::process::exit(101);
        }
    }
}

fn run() -> eframe::Result {
    // `dudel PROJECT...`, as File Explorer starts it for a .dudel file.
    let open_files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .take_while(|argument| !argument.to_string_lossy().starts_with("--"))
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect();
    let mut args = std::env::args().skip(1);
    let mode = args.next().filter(|mode| mode.starts_with("--"));
    if let Some(mode) = mode.as_deref()
        && cli::run(mode, &mut args)
    {
        return Ok(());
    }
    let experimental_indexed_storage = mode.as_deref() == Some("--experimental-indexed-storage");
    let sustained_session = mode.as_deref() == Some("--soak-native-drawing-session");
    let native_session = mode.as_deref() == Some("--soak-native-mask-session") || sustained_session;
    let capture = if native_session
        || mode
            .as_deref()
            .is_some_and(|mode| capture_fixtures::MODES.contains(&mode))
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
        let mode = mode.clone();
        let open_files = open_files.clone();
        let settings_handle = settings_session.as_ref().map(|s| s.handle.clone());
        let settings_issue = settings_session.as_ref().and_then(|s| s.issue.clone());
        Box::new(move |cc| {
            if let Some(path) = capture {
                cc.egui_ctx
                    .memory_mut(|memory| *memory = egui::Memory::default());
                cc.egui_ctx.set_zoom_factor(scale);
                let mut app = Dudel::for_capture(path);
                capture_fixtures::prepare(
                    &mut app,
                    mode.as_deref().unwrap_or_default(),
                    cc,
                    [width, height],
                )?;
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
                // Recovery asks first; the welcome window can wait for Help.
                app.welcome_open = app.show_welcome && !app.recovery_window;
                if !open_files.is_empty() {
                    app.pending_opens = open_files;
                    app.welcome_open = false;
                }
                app.setup.can_install = cfg!(windows) && !install::running_installed();
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
    setup_ui::relaunch_if_requested();
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
            secondary: [30, 40, 50],
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
