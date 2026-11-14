//! What each `--capture-*` screenshot mode sets up before its first frame.
//!
//! A new mode is a name in `MODES` and a block in `prepare`. The blocks run
//! in the order written and several modes build on another's (a mask fixture
//! starts from the layer mask one), so a later block may overwrite an
//! earlier block's status line or tool.

use crate::{
    Dudel, adjustment, adjustments, affine, brush, brush_studio, canvas_size, capture, clipboard,
    commands, display_settings, document, document_actions, export_options, extensions, fill,
    gradient_tool, layer_effects, layer_mask, new_canvas, paint_target, paper, preference_recovery,
    preview_window, reference_layers, rulers, selection, selection_ui, shortcuts, stroke,
    stroke_buffer, symmetry, themes, tool_panel, tools,
};
use eframe::egui;
use std::path::PathBuf;

/// Every screenshot mode. Each runs in an isolated window with default
/// settings and takes the image to write as its first argument.
pub(crate) const MODES: &[&str] = &[
    "--capture-adjustment",
    "--capture-adjustment-layer",
    "--capture-affine",
    "--capture-brush-editor",
    "--capture-brush-graph",
    "--capture-brush-group",
    "--capture-brush-import",
    "--capture-brush-picker",
    "--capture-brush-studio",
    "--capture-canvas-size",
    "--capture-color-brush",
    "--capture-color-picker",
    "--capture-command-search",
    "--capture-eraser",
    "--capture-export",
    "--capture-extensions",
    "--capture-file-menu",
    "--capture-fill",
    "--capture-filter",
    "--capture-gradient",
    "--capture-install",
    "--capture-large-canvas",
    "--capture-large-canvas-detail",
    "--capture-layer-batch",
    "--capture-layer-effects",
    "--capture-layer-guides",
    "--capture-layer-mask",
    "--capture-layer-mask-fill",
    "--capture-layer-mask-lasso",
    "--capture-layer-mask-paste",
    "--capture-layer-mask-stroke",
    "--capture-layer-mask-transform",
    "--capture-long-toolbar",
    "--capture-long-toolbar-brushes",
    "--capture-long-toolbar-options",
    "--capture-mask-coverage",
    "--capture-mask-target-transform",
    "--capture-material",
    "--capture-mirror-pair",
    "--capture-new-canvas",
    "--capture-new-canvas-invalid",
    "--capture-new-canvas-presets",
    "--capture-paper",
    "--capture-preview",
    "--capture-project-memory",
    "--capture-quick-menu",
    "--capture-reference-boards",
    "--capture-reference-layers",
    "--capture-references",
    "--capture-rope",
    "--capture-settings",
    "--capture-settings-recovery",
    "--capture-shortcut-conflict",
    "--capture-shortcuts",
    "--capture-smudge",
    "--capture-symmetry",
    "--capture-tabs",
    "--capture-theme-editor",
    "--capture-timelapse",
    "--capture-tip-cursor",
    "--capture-ui",
    "--capture-updates",
    "--capture-view-menu",
    "--capture-welcome",
    "--capture-workflow",
];

/// Sets `app` up for `mode`, one of `MODES`. `size` is the window's size.
pub(crate) fn prepare(
    app: &mut Dudel,
    mode: &str,
    cc: &eframe::CreationContext<'_>,
    size: [f32; 2],
) -> std::io::Result<()> {
    let capture_large_detail = mode == "--capture-large-canvas-detail";
    let capture_large = mode == "--capture-large-canvas" || capture_large_detail;
    let capture_editor = mode == "--capture-brush-editor";
    let capture_studio = mode == "--capture-brush-studio";
    let capture_settings_recovery = mode == "--capture-settings-recovery";
    let capture_new_canvas_invalid = mode == "--capture-new-canvas-invalid";
    let capture_new_canvas_presets = mode == "--capture-new-canvas-presets";
    let capture_new_canvas = matches!(
        mode,
        "--capture-new-canvas" | "--capture-new-canvas-invalid" | "--capture-new-canvas-presets"
    );
    let capture_long_toolbar = matches!(
        mode,
        "--capture-long-toolbar"
            | "--capture-long-toolbar-options"
            | "--capture-long-toolbar-brushes"
    );
    let capture_extensions = mode == "--capture-extensions";
    let capture_settings = mode == "--capture-settings";
    let capture_shortcuts = mode == "--capture-shortcuts";
    let capture_welcome = mode == "--capture-welcome";
    let capture_updates = mode == "--capture-updates";
    let capture_install = mode == "--capture-install" || capture_updates;
    let capture_gradient = mode == "--capture-gradient";
    let capture_filter = mode == "--capture-filter";
    let capture_smudge = mode == "--capture-smudge";
    let capture_brush_graph = mode == "--capture-brush-graph";
    let capture_brush_import = mode == "--capture-brush-import";
    let capture_color_brush = mode == "--capture-color-brush";
    let capture_brush_group = mode == "--capture-brush-group";
    let capture_paper = mode == "--capture-paper";
    let capture_material = mode == "--capture-material";
    let capture_shortcut_conflict = mode == "--capture-shortcut-conflict";
    let capture_command_search = mode == "--capture-command-search";
    let capture_workflow = mode == "--capture-workflow";
    let capture_layer_batch = mode == "--capture-layer-batch";
    let capture_preview = mode == "--capture-preview";
    let capture_project_memory = mode == "--capture-project-memory";
    let capture_tabs = mode == "--capture-tabs" || capture_project_memory;
    let capture_rope = mode == "--capture-rope";
    let capture_layer_guides = mode == "--capture-layer-guides";
    let capture_eraser = mode == "--capture-eraser";
    let capture_fill = mode == "--capture-fill";
    let capture_tip_cursor = mode == "--capture-tip-cursor";
    let capture_reference_boards = mode == "--capture-reference-boards" || capture_preview;
    let capture_reference_layers = mode == "--capture-reference-layers" || capture_reference_boards;
    let capture_theme_editor = mode == "--capture-theme-editor";
    let capture_timelapse = mode == "--capture-timelapse";
    let capture_references = mode == "--capture-references";
    let capture_mirror_pair = mode == "--capture-mirror-pair";
    let capture_affine = mode == "--capture-affine" || capture_mirror_pair;
    let capture_adjustment = mode == "--capture-adjustment";
    let capture_mask_stroke = mode == "--capture-layer-mask-stroke";
    let capture_mask_fill = mode == "--capture-layer-mask-fill";
    let capture_mask_lasso = mode == "--capture-layer-mask-lasso";
    let capture_mask_paste = mode == "--capture-layer-mask-paste";
    let capture_mask_transform = mode == "--capture-layer-mask-transform";
    let capture_mask_target_transform = mode == "--capture-mask-target-transform";
    let capture_mask_feedback = mode == "--capture-mask-coverage";
    let capture_layer_effects = mode == "--capture-layer-effects";
    let capture_adjustment_layer = mode == "--capture-adjustment-layer";
    let capture_layer_mask = mode == "--capture-layer-mask"
        || capture_layer_effects
        || capture_adjustment_layer
        || capture_mask_transform
        || capture_mask_target_transform
        || capture_mask_feedback
        || capture_mask_stroke
        || capture_mask_fill
        || capture_mask_lasso
        || capture_mask_paste;
    let capture_canvas_size = mode == "--capture-canvas-size";
    let capture_export = mode == "--capture-export";
    let capture_quick_menu = mode == "--capture-quick-menu";
    let capture_popup = match mode {
        "--capture-file-menu" => Some(capture::Popup::FileMenu),
        "--capture-view-menu" => Some(capture::Popup::ViewMenu),
        "--capture-brush-picker" => Some(capture::Popup::Brushes),
        "--capture-color-picker" => Some(capture::Popup::Color),
        "--capture-mask-coverage" => Some(capture::Popup::Color),
        "--capture-long-toolbar-options" => Some(capture::Popup::StrokeOptions),
        "--capture-long-toolbar-brushes" => Some(capture::Popup::Brushes),
        "--capture-symmetry" => Some(capture::Popup::Symmetry),
        "--capture-layer-guides" => Some(capture::Popup::Guides),
        _ => None,
    };
    if capture_command_search {
        app.open_command_search();
        app.command_search.query = "mirror".into();
    }
    if capture_workflow {
        app.inspector_override = Some(true);
        app.tool_panel_override = Some(false);
        app.doc.layers[app.doc.active].locked = true;
        app.layer_navigation.query = "highlights".into();
        app.doc.selection = Some(selection::Selection {
            left: 200,
            top: 200,
            right: 700,
            bottom: 600,
        });
    }
    if capture_layer_batch {
        app.inspector_override = Some(true);
        app.tool_panel_override = Some(false);
        let roots: Vec<_> = app
            .doc
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.depth == 0)
            .map(|(i, l)| (i, l.id))
            .collect();
        app.layer_selection.ids = roots.iter().map(|(_, id)| *id).collect();
        if let Some((index, _)) = roots.last() {
            app.doc.active = *index;
        }
    }
    if capture_large {
        // DUDEL_CAPTURE_SIDE checks larger canvases (default 4096).
        let side = std::env::var("DUDEL_CAPTURE_SIDE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|side| side * side <= document::MAX_PIXELS && *side <= document::MAX_SIDE)
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
            "Layer symmetry and guide controls · global settings remain independent".into();
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
            app.preview.capture = Some(main.with_file_name(format!("{stem}-preview.png")));
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
        app.adjustment.as_mut().unwrap().settings = adjustments::Settings::HueSaturation {
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
                (255.0 * ((x as f32 - width as f32 / 2.0 + 48.0) / 96.0).clamp(0.0, 1.0)).round()
                    as u8
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
        app.doc.layers[0].effects = [
            "Drop Shadow",
            "Stroke",
            "Gradient Overlay",
            "Pattern Overlay",
        ]
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
        let mut layer = crate::document::Layer::raster("Curves 1".into(), width, height);
        // DUDEL_CAPTURE_ADJUSTMENT=levels, hue or gradient shows those.
        let presets = adjustment::presets();
        let settings = match std::env::var("DUDEL_CAPTURE_ADJUSTMENT").as_deref() {
            Ok("levels") => presets[0].clone(),
            Ok("hue") => adjustment::Settings::HueSaturation {
                colorize: None,
                master: [0.0; 3],
                ranges: vec![(adjustment::HUE_RANGES[4], [30.0, 20.0, 0.0])],
            },
            Ok("gradient") => adjustment::Settings::GradientMap {
                stops: vec![
                    (0.0, [40.0, 20.0, 90.0], 0.5),
                    (0.55, [220.0, 90.0, 60.0], 0.5),
                    (1.0, [255.0, 240.0, 180.0], 0.5),
                ],
                reverse: false,
            },
            _ => adjustment::Settings::Curves {
                channels: vec![(0, vec![[0, 0], [70, 40], [190, 225], [255, 255]])],
                bezier: false,
            },
        };
        layer.name = format!("{} 1", settings.name());
        layer.adjustment = Some(adjustment::Adjustment::new(settings));
        // DUDEL_CAPTURE_CLIPPED shows the clipping group's rows.
        layer.clipped = std::env::var_os("DUDEL_CAPTURE_CLIPPED").is_some();
        app.adjustment_window_open = !layer.clipped;
        app.doc.layers.push(layer);
        app.doc.active = app.doc.layers.len() - 1;
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
            secondary: [0; 3],
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
    if capture_quick_menu {
        for index in [0, 1, 2, 3, 5, 6, 8] {
            if let Some(brush) = app.brushes.get_mut(index) {
                brush.favorite = true;
            }
        }
        app.recent_colors = vec![
            [32, 36, 48],
            [200, 60, 60],
            [240, 170, 60],
            [70, 140, 90],
            [70, 110, 200],
            [235, 225, 210],
        ];
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
        app.open_quick_menu(screen.center() - egui::vec2(60.0, 0.0), screen);
    }
    if capture_export {
        app.open_export_window(false);
        app.export_settings.format = match std::env::var("DUDEL_CAPTURE_EXPORT_FORMAT").as_deref() {
            Ok("jpeg") => export_options::Format::Jpeg,
            Ok("gif") => export_options::Format::Gif,
            _ => export_options::Format::Png,
        };
        app.export_settings.quality = 20;
        app.export_settings.background = Some([40, 60, 110]);
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
        app.brush_panel_page = match std::env::var("DUDEL_CAPTURE_BRUSH_PAGE").as_deref() {
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
        if capture_new_canvas_presets {
            let size = |name: &str, width, height, dpi| new_canvas::Size {
                name: name.into(),
                width,
                height,
                dpi,
            };
            app.new_canvas_form.prefs = new_canvas::Prefs {
                presets: vec![
                    size("Comic page", 2894, 4093, 350.0),
                    size("Stream overlay", 1920, 1080, 72.0),
                    size("Icon", 512, 512, 72.0),
                ],
                recent: vec![size("", 3000, 4200, 300.0), size("", 1920, 1080, 72.0)],
            };
        }
    }
    if capture_long_toolbar {
        app.brush.name =
            "Detailed illustration brush with pressure, texture and custom blending".repeat(2);
        app.brush.opacity = 0.63;
    }
    if capture_extensions {
        app.extension_manager = true;
        if let Ok(package) =
            serde_json::from_str(include_str!("../plugins/example-studio-kit.dudelplugin"))
        {
            app.extensions.push(extensions::Installed {
                package,
                enabled: true,
            });
        }
    }
    if capture_settings {
        app.settings_open = true;
    }
    if capture_filter {
        // The Gaussian blur dialog over the sample artwork.
        app.doc.active = app.doc.layers.len() - 1;
        app.start_adjustment(adjustments::Kind::GaussianBlur);
    }
    if capture_gradient {
        // The Fill tool's Gradient mode mid-drag, with its options.
        app.select_tool(tools::Tool::Fill);
        app.gradient.active = true;
        app.gradient.style = gradient_tool::Style::Radial;
        app.tool_panel_override = Some(true);
        if let Some(target) = app.input_target() {
            app.selection_drag = Some(selection_ui::Drag::Gradient(Box::new(
                gradient_tool::Settings {
                    target,
                    paint_target: Default::default(),
                    start: [300.0, 260.0],
                    end: [620.0, 470.0],
                    style: app.gradient.style,
                    reverse: false,
                    ramp: app.gradient.ramp(app.color, app.secondary),
                    opacity: 1.0,
                },
            )));
        }
    }
    if capture_smudge {
        app.smudge_capture_fixture();
    }
    if capture_brush_graph {
        app.brush_graph_capture_fixture();
    }
    if capture_brush_import {
        app.brush_import_capture_fixture();
    }
    if capture_color_brush {
        app.color_brush_capture_fixture();
    }
    if capture_brush_group {
        app.graph_group_capture_fixture();
    }
    if capture_paper {
        app.doc.paper = Some(paper::Paper::presets()[1].1);
        app.open_paper_dialog();
    }
    if capture_material {
        app.material_capture_fixture();
    }
    if capture_install {
        app.setup_capture_fixture(capture_updates);
    }
    if capture_welcome {
        app.welcome_open = true;
        app.recent = ["Portrait study.dudel", "Comic page 12.dudel"]
            .map(PathBuf::from)
            .to_vec();
    }
    if capture_shortcuts || capture_shortcut_conflict {
        app.settings_open = true;
        app.settings_section = 4;
        app.shortcut_editor.filter = "layer".into();
        if capture_shortcut_conflict {
            app.shortcut_editor.capture = Some(commands::Action::NewLayer);
            app.shortcut_editor.candidate =
                Some(shortcuts::Chord::new(egui::Key::Z, egui::Modifiers::CTRL));
        }
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
    Ok(())
}
