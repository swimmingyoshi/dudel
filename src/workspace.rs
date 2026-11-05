use crate::{
    Dudel,
    brush::Brush,
    color_wheel::color32,
    egui,
    icons::{self, Icon},
    tools::Tool,
};

#[cfg(test)]
pub fn theme(ctx: &egui::Context) {
    crate::themes::apply(ctx, &crate::themes::Theme::default());
}

pub(crate) fn setup_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = egui::Color32::from_rgb(35, 38, 46);
    // Menus, dropdowns and dialogs sit visibly above the panels: a lighter
    // fill and a clear edge instead of the panel color.
    style.visuals.window_fill = egui::Color32::from_rgb(52, 58, 71);
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(96, 106, 126));
    // Brighter text than egui's dark defaults so labels and menu entries read
    // clearly on the dark theme.
    style.visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_gray(200);
    style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_gray(222);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(26, 28, 34);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(42, 46, 56);
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(57, 105, 153);
    style.visuals.selection.stroke.color = egui::Color32::from_rgb(140, 190, 240);
    style.visuals.widgets.noninteractive.bg_stroke.color = egui::Color32::from_rgb(52, 56, 68);
    style.visuals.window_corner_radius = egui::CornerRadius::same(8);
    for widgets in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widgets.corner_radius = egui::CornerRadius::same(5);
    }
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 7.0);
    style.spacing.interact_size.y = 32.0;
    // Number boxes beside sliders keep one width as their value changes
    // ("99%" -> "100%"), so dragging a slider never shifts the layout.
    style.spacing.interact_size.x = 72.0;
    style.spacing.slider_width = 140.0;
    // Wider sidebar edge grab so a pen can find it.
    style.interaction.resize_grab_radius_side = 8.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    // egui's 9 pt default made hints and notes hard to read beside 15 pt text.
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.5));
    ctx.set_style(style);
}

/// Width of the label column beside dialog sliders.
const SLIDER_LABEL_WIDTH: f32 = 92.0;

/// A slider with its label on the left in a fixed column, so slider rows in
/// dialogs line up and read label-first like the brush and guide panels.
pub(crate) fn labeled_slider(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    slider: egui::Slider,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(SLIDER_LABEL_WIDTH, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(SLIDER_LABEL_WIDTH);
                ui.add(egui::Label::new(label).truncate());
            },
        );
        ui.add(slider)
    })
    .inner
}

const TOOL_RAIL_WIDTH: f32 = 58.0;
/// Narrowest the canvas may become while a sidebar is dragged wider.
const MIN_CANVAS_WIDTH: f32 = 360.0;
const TOOL_PANEL: &str = "tool property";
const INSPECTOR: &str = "inspector";
const NATIVE_PEN_LABEL: &str = if cfg!(windows) {
    "Use Windows pen history"
} else {
    "Use Wayland tablet input"
};

fn sidebar_width(ctx: &egui::Context, id: &str) -> Option<f32> {
    egui::containers::panel::PanelState::load(ctx, egui::Id::new(id)).map(|s| s.rect.width())
}

/// Lay out sidebar contents in a child bounded to the panel, then reserve exactly
/// the panel's width. egui paints and clips a side panel at its set width but
/// hands the canvas the space right of the content, so one row wider than the
/// panel left an unpainted strip between them. Overflow now stays clipped inside.
fn bounded(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    add(&mut child);
    let bottom = child.min_rect().bottom().clamp(rect.top(), rect.bottom());
    ui.advance_cursor_after_rect(egui::Rect::from_x_y_ranges(
        rect.x_range(),
        rect.top()..=bottom,
    ));
}

/// Width range for a sidebar whose upper bound leaves the canvas at least
/// `MIN_CANVAS_WIDTH` beside the rail and the other visible sidebar.
fn sidebar_range(
    ctx: &egui::Context,
    min: f32,
    max: f32,
    other: Option<f32>,
) -> std::ops::RangeInclusive<f32> {
    let room =
        ctx.content_rect().width() - TOOL_RAIL_WIDTH - other.unwrap_or(0.0) - MIN_CANVAS_WIDTH;
    min..=room.clamp(min, max)
}

/// How far past its narrowest width a sidebar edge must be dragged to close it.
const COLLAPSE_DRAG: f32 = 120.0;

/// egui saves a panel's content width as the panel width, so content that
/// overflows or underfills drags it away from where the user left it. Keep
/// the dragged width ourselves and hand it back to egui every frame.
/// Returns true when a collapsible panel's edge is dragged well past its
/// narrowest width, asking to close it; its last width is kept for reopening.
fn pin_sidebar_width(
    ctx: &egui::Context,
    id: &str,
    left: bool,
    collapsible: bool,
    width: &mut Option<f32>,
    range: &std::ops::RangeInclusive<f32>,
) -> bool {
    let id = egui::Id::new(id);
    let Some(state) = egui::containers::panel::PanelState::load(ctx, id) else {
        return false; // First frame: egui starts from the default width.
    };
    if let Some(resize) = ctx.read_response(id.with("__resize"))
        && resize.dragged()
        && let Some(pointer) = resize.interact_pointer_pos()
    {
        let start = id.with("width before drag");
        if resize.drag_started() {
            ctx.data_mut(|d| d.insert_temp(start, *width));
        }
        let requested = if left {
            pointer.x - state.rect.left()
        } else {
            state.rect.right() - pointer.x
        };
        if collapsible && requested < range.start() - COLLAPSE_DRAG {
            // Reopen where the panel was before this drag, not at the
            // narrow widths passed on the way to closing it.
            if let Some(before) = ctx.data(|d| d.get_temp::<Option<f32>>(start)) {
                *width = before;
            }
            return true;
        }
        *width = Some(requested);
    }
    let pinned = width
        .get_or_insert(state.rect.width())
        .clamp(*range.start(), *range.end());
    let mut rect = state.rect;
    if left {
        rect.set_width(pinned);
    } else {
        rect.min.x = rect.max.x - pinned;
    }
    ctx.data_mut(|d| d.insert_persisted(id, egui::containers::panel::PanelState { rect }));
    false
}

/// Tab toggles panels while no control has focus; F6 enters control navigation.
/// Once a control has focus, egui owns Tab/Shift+Tab and other keyboard input.
/// Returns true on a plain, nonrepeated panel-toggle press.
pub(crate) fn take_panel_toggle(ctx: &egui::Context, raw_input: &mut egui::RawInput) -> bool {
    if ctx.wants_keyboard_input() {
        return false;
    }
    let mut toggle = false;
    let mut enter_controls = false;
    raw_input.events.retain(|event| match event {
        egui::Event::Key {
            key: egui::Key::F6,
            pressed,
            repeat,
            modifiers,
            ..
        } if modifiers.is_none() => {
            enter_controls |= *pressed && !*repeat;
            false
        }
        egui::Event::Key {
            key: egui::Key::Tab,
            pressed,
            repeat,
            modifiers,
            ..
        } => {
            toggle |= *pressed && !*repeat && modifiers.is_none();
            false
        }
        _ => true,
    });
    if enter_controls {
        // Let egui choose the first enabled control during begin_pass. Supply a
        // complete pair so its key state never retains a synthetic Tab press.
        raw_input
            .events
            .extend([true, false].map(|pressed| egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }));
    }
    toggle
}

/// True while the pointer hovers or drags a sidebar's resize edge, whose grab
/// area overlaps the canvas by a few points.
pub(crate) fn sidebar_edge_active(ctx: &egui::Context) -> bool {
    [TOOL_PANEL, INSPECTOR].into_iter().any(|id| {
        ctx.read_response(egui::Id::new(id).with("__resize"))
            .is_some_and(|r| r.hovered() || r.dragged())
    })
}

/// Five-point star, filled when on; drawn as triangles so the concave
/// outline fills correctly.
fn paint_star(painter: &egui::Painter, center: egui::Pos2, radius: f32, on: bool, hovered: bool) {
    let points: Vec<egui::Pos2> = (0..10)
        .map(|k| {
            let r = if k % 2 == 0 { radius } else { radius * 0.45 };
            let a = std::f32::consts::PI * k as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
            center + egui::vec2(a.cos(), a.sin()) * r
        })
        .collect();
    let gold = egui::Color32::from_rgb(242, 196, 76);
    let outline = if on || hovered {
        gold
    } else {
        egui::Color32::from_gray(120)
    };
    if on {
        for k in 0..10 {
            painter.add(egui::Shape::convex_polygon(
                vec![center, points[k], points[(k + 1) % 10]],
                gold,
                egui::Stroke::NONE,
            ));
        }
    }
    painter.add(egui::Shape::closed_line(
        points,
        egui::Stroke::new(1.2_f32, outline),
    ));
}

pub(crate) fn section_title(title: &str) -> egui::RichText {
    // Theme-relative so titles stay legible on light themes.
    egui::RichText::new(title).size(12.0).weak()
}

pub(crate) fn section(title: &str) -> egui::CollapsingHeader {
    egui::CollapsingHeader::new(section_title(title)).default_open(true)
}

/// Horizontal divider grip between two stacked panel areas. Returns the
/// vertical drag distance this frame.
/// What a split handle did this frame.
pub(crate) enum SplitDrag {
    None,
    By(f32),
    /// Double-click returns the area to its automatic size.
    Reset,
}

pub(crate) fn split_handle(ui: &mut egui::Ui) -> SplitDrag {
    let (handle, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 10.0),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_text("Drag to resize · double-click to reset");
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let color = if response.hovered() || response.dragged() {
        egui::Color32::from_gray(140)
    } else {
        egui::Color32::from_gray(80)
    };
    ui.painter().line_segment(
        [
            handle.center() - egui::vec2(18.0, 0.0),
            handle.center() + egui::vec2(18.0, 0.0),
        ],
        egui::Stroke::new(3.0_f32, color),
    );
    if response.double_clicked() {
        SplitDrag::Reset
    } else if response.dragged() && response.drag_delta().y != 0.0 {
        SplitDrag::By(response.drag_delta().y)
    } else {
        SplitDrag::None
    }
}

fn bounded_menu(
    ui: &mut egui::Ui,
    label: &str,
    contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    ui.menu_button(label, |ui| {
        let pass = ui.ctx().cumulative_pass_nr();
        let id = ui.make_persistent_id("command menu last visible pass");
        let opened = ui.data_mut(|data| {
            let previous = data.get_temp::<u64>(id);
            data.insert_temp(id, pass);
            previous.is_none_or(|previous| pass.saturating_sub(previous) > 1)
        });
        // Use all the room below the menu bar before scrolling: a popup sizes
        // itself to the scroll area's minimum, so a small minimum made every
        // long menu scroll even on a tall window.
        let room =
            (ui.ctx().content_rect().bottom() - ui.next_widget_position().y - 12.0).max(120.0);
        let scroll = egui::ScrollArea::vertical()
            .max_height(room)
            .min_scrolled_height(room);
        let scroll = if opened {
            scroll.vertical_scroll_offset(0.0)
        } else {
            scroll
        };
        scroll.show(ui, contents);
    })
    .response
}

fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let clicked = ui
        .add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
        .clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Overlapping main/secondary chips; clicking swaps them.
pub(crate) fn color_chips(
    ui: &mut egui::Ui,
    main: &mut [u8; 3],
    secondary: &mut [u8; 3],
    side: f32,
) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let chip = egui::vec2(side * 0.62, side * 0.62);
    let back = egui::Rect::from_min_size(rect.right_bottom() - chip, chip);
    let front = egui::Rect::from_min_size(rect.min, chip);
    for (area, color) in [(back, *secondary), (front, *main)] {
        ui.painter().rect_filled(area, 4.0, color32(color));
        ui.painter().rect_stroke(
            area,
            4.0,
            egui::Stroke::new(1.5_f32, egui::Color32::from_gray(150)),
            egui::StrokeKind::Outside,
        );
    }
    if response
        .on_hover_text("Main / secondary color · click or press X to swap")
        .clicked()
    {
        std::mem::swap(main, secondary);
    }
}

impl Dudel {
    fn about_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("About Dudel")
            .open(&mut self.about_window)
            .default_width(330.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Dudel");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A native illustration application.");
                ui.separator();
                ui.label(format!("License: {}", env!("CARGO_PKG_LICENSE")));
                ui.hyperlink_to("GNU General Public License v3", "https://www.gnu.org/licenses/gpl-3.0.html");
                ui.hyperlink_to("Source and build instructions", env!("CARGO_PKG_REPOSITORY"));
                ui.label("Free to build from source. Official prebuilt downloads may be sold.");
                ui.small("The download package includes license notices and matching-source information.");
            });
    }
    fn menus(&mut self, ui: &mut egui::Ui) {
        let file_menu = bounded_menu(ui, "File", |ui| {
            self.command_menu_item(ui, crate::commands::Action::New);
            self.command_menu_item(ui, crate::commands::Action::Open);
            bounded_menu(ui, "Open recent", |ui| {
                if self.recent.is_empty() {
                    ui.label("No recent projects");
                }
                for path in self.recent.clone() {
                    let label = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    if ui
                        .button(label)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        self.open_path(path);
                        ui.close();
                    }
                }
            });
            ui.separator();
            self.command_menu_item(ui, crate::commands::Action::Save);
            self.command_menu_item(ui, crate::commands::Action::SaveAs);
            ui.separator();
            bounded_menu(ui, "Import", |ui| {
                if menu_item(ui, "PNG as canvas…", "", true) {
                    self.import_png();
                }
                if menu_item(ui, "CSP raster canvas…", "", true) {
                    self.import_clip();
                }
                if menu_item(ui, "Photoshop document (.psd, .psb)…", "", true) {
                    self.import_psd();
                }
                if menu_item(ui, "Images as layers…", "", self.doc.has_room(1, 1)) {
                    self.import_png_layer();
                }
                ui.separator();
                if menu_item(ui, "Project package (.zip)…", "", true) {
                    self.import_package();
                }
            });
            bounded_menu(ui, "Export", |ui| {
                self.command_menu_item(ui, crate::commands::Action::ExportPng(false));
                self.command_menu_item(ui, crate::commands::Action::ExportPng(true));
                self.command_menu_item(ui, crate::commands::Action::ExportPsd);
                self.command_menu_item(ui, crate::commands::Action::ExportLive2d);
                self.command_menu_item(ui, crate::commands::Action::ExportSvg);
                ui.separator();
                if menu_item(ui, "Export project package (.zip)…", "", true) {
                    self.export_package();
                }
            });
            ui.separator();
            if menu_item(ui, "Minimize window", "", true) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            let fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
            if menu_item(
                ui,
                if maximized {
                    "Restore window"
                } else {
                    "Maximize window"
                },
                "",
                !fullscreen,
            ) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            ui.separator();
            if menu_item(ui, "Exit", "Alt+F4", true) {
                self.request_replace(crate::document_actions::Action::Close);
            }
        });
        self.open_capture_popup(&file_menu, crate::capture::Popup::FileMenu);
        bounded_menu(ui, "Edit", |ui| {
            self.command_menu_item(ui, crate::commands::Action::Search);
            ui.separator();
            self.command_menu_item(ui, crate::commands::Action::Undo);
            self.command_menu_item(ui, crate::commands::Action::Redo);
            ui.separator();
            let transforming = self.affine_preview.is_some();
            self.command_menu_item(ui, crate::commands::Action::Transform);
            if transforming && menu_item(ui, "Cancel transform", "Esc", true) {
                self.cancel_affine_preview();
            }
            let mask_focused = !self.input_stroke_target().is_raster();
            if menu_item(
                ui,
                if mask_focused { "Clear mask" } else { "Clear" },
                "Delete",
                true,
            ) {
                self.clear_selection_pixels();
            }
            ui.separator();
            if menu_item(
                ui,
                if mask_focused {
                    "Copy mask"
                } else {
                    "Copy pixels"
                },
                "Ctrl+C",
                true,
            ) {
                self.copy_canvas(false);
            }
            if menu_item(
                ui,
                if mask_focused {
                    "Cut mask"
                } else {
                    "Cut pixels"
                },
                "Ctrl+X",
                true,
            ) {
                self.copy_canvas(true);
            }
            if menu_item(
                ui,
                if mask_focused {
                    "Paste into mask"
                } else {
                    "Paste image as layer"
                },
                "Ctrl+V",
                mask_focused || self.doc.has_room(1, 1),
            ) {
                self.paste_canvas();
            }
            if menu_item(ui, "Paste as reference image", "", true) {
                self.paste_as_reference();
            }
        });
        bounded_menu(ui, "Canvas", |ui| {
            ui.label(
                egui::RichText::new(
                    "These change the artwork. To turn the view, use View or the Navigator.",
                )
                .small()
                .weak(),
            );
            if menu_item(ui, "Canvas size…", "", true) {
                self.open_canvas_size();
            }
            if menu_item(
                ui,
                "Crop to selection bounds",
                "",
                self.doc.selection.is_some(),
            ) {
                self.dispatch_canvas_size(crate::document_edits::Edit::CropCanvas);
            }
            ui.separator();
            if menu_item(ui, "Rotate image 90° clockwise", "", true) {
                self.rotate_canvas(1);
            }
            if menu_item(ui, "Rotate image 90° counter-clockwise", "", true) {
                self.rotate_canvas(3);
            }
            if menu_item(ui, "Rotate image 180°", "", true) {
                self.rotate_canvas(2);
            }
        });
        bounded_menu(ui, "Layer", |ui| {
            self.command_menu_item(ui, crate::commands::Action::NewLayer);
            self.command_menu_item(ui, crate::commands::Action::NewFolder);
            ui.add_enabled_ui(self.doc.has_room(1, 1), |ui| {
                ui.menu_button("New adjustment layer", |ui| {
                    for settings in crate::adjustment::presets() {
                        if ui.button(settings.name()).clicked() {
                            self.create_adjustment_layer(settings);
                            self.adjustment_window_open = true;
                            ui.close();
                        }
                    }
                });
            });
            self.command_menu_item(ui, crate::commands::Action::NewVector);
            let vector = self
                .input_layer()
                .and_then(|i| self.doc.layers.get(i))
                .is_some_and(|l| l.vector.is_some());
            if menu_item(ui, "Recolor vector strokes", "", vector) {
                self.restyle_vector(true);
            }
            if menu_item(ui, "Apply current brush to vector strokes", "", vector) {
                self.restyle_vector(false);
            }
            if menu_item(ui, "Rasterize layer", "", vector) {
                self.rasterize_layer();
            }
            self.command_menu_item(ui, crate::commands::Action::Effects);
            self.command_menu_item(ui, crate::commands::Action::Group);
            self.command_menu_item(ui, crate::commands::Action::Ungroup);
            ui.separator();
            self.command_menu_item(ui, crate::commands::Action::Duplicate);
            self.command_menu_item(ui, crate::commands::Action::Merge);
            if menu_item(
                ui,
                if self.multiple_layers() {
                    "Delete selected layers"
                } else {
                    "Delete"
                },
                "",
                self.doc.layers.len() > 1,
            ) {
                self.delete_layer();
            }
            ui.separator();
            if menu_item(
                ui,
                "Add reference image…",
                "",
                self.reference_layers.layers.len() < crate::reference_layers::MAX_REFERENCES,
            ) {
                self.add_reference_layer();
            }
            ui.separator();
            ui.label(
                egui::RichText::new("Transforms use the selection, or the whole layer")
                    .small()
                    .weak(),
            );
            self.command_menu_item(ui, crate::commands::Action::FlipArtwork);
            if menu_item(ui, "Flip vertically", "", true) {
                self.transform_active(crate::transform::Transform::FlipVertical);
            }
            if menu_item(ui, "Rotate 180°", "", true) {
                self.transform_active(crate::transform::Transform::RotateHalfTurn);
            }
        });
        bounded_menu(ui, "Select", |ui| {
            let masked = !self.input_stroke_target().is_raster();
            if ui.add_enabled(masked || !self.input_is_folder(), egui::Button::new(if masked {"From mask coverage"} else {"From artwork coverage"}))
                .on_hover_text(if masked {"Use raw mask coverage, including when the mask is disabled"} else {"Use artwork alpha and its enabled mask; layer opacity and folder styling are excluded"})
                .clicked() {
                self.select_target_coverage();
                ui.close();
            }
            self.command_menu_item(ui, crate::commands::Action::SelectAll);
            self.command_menu_item(ui, crate::commands::Action::Deselect);
            self.command_menu_item(ui, crate::commands::Action::InvertSelection);
            if menu_item(
                ui,
                if masked { "Clear mask" } else { "Clear pixels" },
                "Delete",
                true,
            ) {
                self.clear_selection_pixels();
            }
        });
        bounded_menu(ui, "Filter", |ui| {
            for kind in [
                crate::adjustments::Kind::BrightnessContrast,
                crate::adjustments::Kind::HueSaturation,
                crate::adjustments::Kind::Levels,
            ] {
                if ui.button(format!("{}\u{2026}", kind.name())).clicked() {
                    self.start_adjustment(kind);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Invert").clicked() {
                self.filter(include_str!("../plugins/invert.rhai"));
                ui.close();
            }
            if ui.button("Grayscale").clicked() {
                self.filter(include_str!("../plugins/grayscale.rhai"));
                ui.close();
            }
            self.extension_filters_ui(ui);
            ui.separator();
            ui.menu_button("Run filter from file", |ui| {
                if ui.button("Rhai script (.rhai)…").clicked() {
                    let dialog = rfd::AsyncFileDialog::new().add_filter("Rhai filter", &["rhai"]);
                    self.ask_open_file(dialog, |app, path| {
                        match crate::read_bounded(&path, 32768) {
                            Ok(source) => app.filter(&source),
                            Err(e) => app.status = e,
                        }
                    });
                    ui.close();
                }
                if ui.button("WebAssembly module (.wasm)…").clicked() {
                    let dialog = rfd::AsyncFileDialog::new().add_filter("WASM filter", &["wasm"]);
                    self.ask_open_file(dialog, |app, path| match crate::wasm_filters::read_module(
                        &path,
                    ) {
                        Ok(bytes) => app.start_filter(crate::plugins::FilterProgram::Wasm(bytes)),
                        Err(error) => app.status = error,
                    });
                    ui.close();
                }
            });
            if ui.button("Manage extensions…").clicked() {
                self.extension_manager = true;
                ui.close();
            }
            ui.separator();
            self.extension_contributions_ui(ui, None);
        });
        let view_menu = bounded_menu(ui, "View", |ui| {
            ui.menu_button("Theme", |ui| self.theme_menu(ui));
            if ui.button("Timelapse...").clicked() {
                self.timelapse.open = true;
                ui.close();
            }
            ui.separator();
            ui.menu_button(
                if self.preview.open {
                    "Preview window ✔"
                } else {
                    "Preview window"
                },
                |ui| {
                    ui.checkbox(&mut self.preview.open, "Open preview window")
                        .on_hover_text("A separate window with only the artwork, for another monitor, viewers or OBS");
                    ui.separator();
                    self.preview_menu(ui);
                },
            );
            self.command_menu_item(ui, crate::commands::Action::Settings);
            self.command_menu_item(ui, crate::commands::Action::Panels);
            if menu_item(ui, "Automatic panel layout", "", true) {
                self.tool_panel_override = None;
                self.inspector_override = None;
                self.show_panels = true;
            }
            ui.separator();
            self.command_menu_item(ui, crate::commands::Action::ZoomIn);
            self.command_menu_item(ui, crate::commands::Action::ZoomOut);
            self.command_menu_item(ui, crate::commands::Action::Fit);
            self.command_menu_item(ui, crate::commands::Action::ActualSize);
            ui.separator();
            self.command_menu_item(ui, crate::commands::Action::RotateLeft);
            self.command_menu_item(ui, crate::commands::Action::RotateRight);
            self.command_menu_item(ui, crate::commands::Action::ResetRotation);
            self.command_menu_item(ui, crate::commands::Action::FlipView);
            self.command_menu_item(ui, crate::commands::Action::Transparency);
            ui.menu_button("Symmetry", |ui| self.symmetry_ui(ui));
            ui.menu_button("Grid and guides", |ui| self.guides_ui(ui));
            ui.separator();
            let mut tool_panel = self.tool_panel_visible(ui.ctx());
            if ui
                .checkbox(&mut tool_panel, "Tool property panel")
                .changed()
            {
                self.tool_panel_override = Some(tool_panel);
            }
            let mut inspector = self.inspector_visible(ui.ctx());
            if ui
                .checkbox(&mut inspector, "Navigator, color and layers")
                .changed()
            {
                self.inspector_override = Some(inspector);
            }
            ui.separator();
            if menu_item(ui, "Tablet input…", "", true) {
                self.input_settings = true;
            }
            if menu_item(ui, "References…", "", true) {
                self.references.open = true;
            }
            if menu_item(
                ui,
                "Recover artwork…",
                "",
                !self.recovery_candidates.is_empty(),
            ) {
                self.recovery_window = true;
            }
        });
        self.open_capture_popup(&view_menu, crate::capture::Popup::ViewMenu);
        bounded_menu(ui, "Help", |ui| {
            self.command_menu_item(ui, crate::commands::Action::Shortcuts);
            if menu_item(ui, "About Dudel", "", true) {
                self.about_window = true;
            }
        });
    }

    pub(crate) fn opacity_slider(&mut self, ui: &mut egui::Ui) {
        ui.label("Opacity");
        let mut percent = (self.brush.opacity * 100.0).round();
        if ui
            .add(
                egui::Slider::new(&mut percent, 0.0..=100.0)
                    .suffix("%")
                    .integer(),
            )
            .changed()
        {
            self.brush.opacity = percent / 100.0;
        }
    }

    fn brush_size_control(&mut self, ui: &mut egui::Ui, direct: bool) {
        ui.label("Size");
        if direct {
            ui.add(
                egui::DragValue::new(&mut self.brush.size)
                    .range(1.0..=300.0)
                    .suffix(" px")
                    .max_decimals(0),
            );
        } else {
            ui.add(
                egui::Slider::new(&mut self.brush.size, 1.0..=300.0)
                    .logarithmic(true)
                    .suffix(" px")
                    .max_decimals(0),
            )
            .on_hover_text("[ and ] change size");
        }
    }

    fn options_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // A previous wider/DPI pass can leave this child wider than the
            // viewport. Responsive choices must still respect visible bounds.
            let bar_width = ui
                .available_width()
                .min((ui.ctx().content_rect().width() - 16.0).max(0.0));
            let narrow = bar_width < 1050.0;
            let tiny = bar_width < 700.0;
            // Sliders share whatever width remains so the bar never clips.
            ui.spacing_mut().slider_width = ((bar_width - 780.0) / 3.0).clamp(48.0, 150.0);
            if self.capture.as_ref().is_some_and(|capture|capture.frames == 5) {
                println!("Capture toolbar: width={bar_width} content={:?} pixels_per_point={} zoom={} narrow={narrow} tiny={tiny}",ui.ctx().content_rect(),ui.ctx().pixels_per_point(),ui.ctx().zoom_factor());
            }
            let panel_open = self.show_panels && self.tool_panel_visible(ui.ctx());
            let (tool_name, tool_icon) = self.tool_label();
            if icons::button(ui, tool_icon, 30.0, panel_open, true)
                .on_hover_text("Show or hide the tool property panel")
                .clicked()
            {
                self.show_panels = true;
                self.tool_panel_override = Some(!panel_open);
            }
            let target_label = if self.input_stroke_target().is_raster() {
                tool_name
            } else {
                "Mask"
            };
            if self.mask_controls_available()
                && self.input_properties().is_some_and(|p| p.mask.is_some())
            {
                ui.menu_button(egui::RichText::new(target_label).strong(), |ui| {
                    ui.label("Painting target");
                    if ui
                        .selectable_label(self.input_stroke_target().is_raster(), "Edit artwork")
                        .clicked()
                    {
                        self.focus_current_mask(false);
                        ui.close();
                    }
                    if ui
                        .selectable_label(!self.input_stroke_target().is_raster(), "Edit mask")
                        .clicked()
                    {
                        self.focus_current_mask(true);
                        ui.close();
                    }
                    ui.separator();
                    ui.label("Black conceals; white reveals.");
                })
                .response
                .on_hover_text("Choose artwork or mask as the painting target");
            } else {
                ui.label(egui::RichText::new(target_label).strong());
            }
            ui.separator();
            if self.affine_preview.is_some() {
                ui.label(egui::RichText::new("Free transform").strong());
                if ui.button("Apply").on_hover_text("Enter").clicked() {
                    self.finish_affine_preview();
                }
                if ui.button("Cancel").on_hover_text("Esc").clicked() {
                    self.cancel_affine_preview();
                }
                ui.menu_button("Mirror", |ui| self.mirror_pair_ui(ui));
                ui.label(
                    egui::RichText::new("Shift keeps proportions · Alt scales from the center")
                        .weak(),
                );
                return;
            }
            if self.tool == Tool::Select && self.polygon_selection_points().is_some() {
                self.polygon_selection_controls(ui);
                return;
            }
            match self.tool {
                Tool::Pointer => {
                    ui.checkbox(&mut self.grid.visible, "Grid");
                    ui.checkbox(&mut self.show_rulers, "Rulers")
                        .on_hover_text("Drag from a ruler onto the canvas to add a guide line");
                    ui.checkbox(&mut self.guides_locked, "Lock guides")
                        .on_hover_text(
                            "Drawing tools cannot move locked guides; the Pointer tool still can",
                        );
                    ui.menu_button("Grid and guides", |ui| {
                        ui.set_max_width(260.0);
                        self.guides_ui(ui);
                    });
                }
                Tool::Brush | Tool::Eraser => {
                    self.brush_picker_ui(ui, (bar_width * 0.16).clamp(90.0, 192.0));
                    self.quick_color_ui(ui);
                    if !tiny {
                        self.brush_size_control(ui, narrow);
                    }
                    if narrow && !tiny {
                        let mut percent = (self.brush.opacity * 100.0).round();
                        if ui
                            .add(
                                egui::DragValue::new(&mut percent)
                                    .range(0.0..=100.0)
                                    .suffix("%"),
                            )
                            .on_hover_text("Brush opacity")
                            .changed()
                        {
                            self.brush.opacity = percent / 100.0;
                        }
                    } else if !tiny {
                        self.opacity_slider(ui);
                    }
                    // Measure the inline group after allocating the primary
                    // controls. Names, target labels and retained parent widths
                    // must not push snapping/symmetry outside the viewport.
                    let symmetry = self.active_symmetry();
                    let symmetry_label = if symmetry.active() {
                        format!("Symmetry: {}", symmetry.mode.name())
                    } else {
                        "Symmetry".to_owned()
                    };
                    let text_width = |text: &str, style: egui::TextStyle| {
                        ui.painter().layout_no_wrap(text.to_owned(), style.resolve(ui.style()), ui.visuals().text_color()).size().x
                    };
                    // The follow-mode button is sized for its longest label, so
                    // switching Smooth/Rope never changes the bar's width.
                    let follow_width = text_width("Smooth", egui::TextStyle::Button)
                        .max(text_width("Rope", egui::TextStyle::Button));
                    let inline_width = text_width("Stabilizer", egui::TextStyle::Body)
                        + follow_width
                        + 2.0 * ui.spacing().button_padding.x
                        + ui.spacing().item_spacing.x
                        + ui.spacing().slider_width + ui.spacing().interact_size.x
                        + text_width("Snap", egui::TextStyle::Button)
                        + text_width("Symmetry", egui::TextStyle::Button)
                        + 4.0 * ui.spacing().button_padding.x + 18.0
                        + 8.0 * ui.spacing().item_spacing.x + 16.0;
                    let remaining = ui.available_width().min(ui.ctx().content_rect().right() - 8.0 - ui.next_widget_position().x);
                    let compact = tiny || remaining < inline_width;
                    let mut stroke_options = |ui: &mut egui::Ui| {
                        if tiny {
                            ui.horizontal(|ui| self.brush_size_control(ui, false));
                            ui.horizontal(|ui| self.opacity_slider(ui));
                        }
                        let mut stabilizer = |ui: &mut egui::Ui| {
                            ui.label("Stabilizer");
                            ui.add(
                            egui::Slider::new(&mut self.brush.stabilization, 0.0..=50.0)
                                .max_decimals(0),
                        )
                        .on_hover_text("Smooths hand jitter; higher values follow the pen more slowly");
                        };
                        if compact {ui.horizontal(stabilizer);} else {stabilizer(ui);}
                        let rope = self.brush.stabilizer_mode == crate::brush::StabilizerMode::Rope;
                        // Follow mode, Rope distance and speed live in one place so
                        // the inline bar keeps a fixed set of controls in every mode.
                        let mut follow_options = |ui: &mut egui::Ui| {
                            self.stabilizer_mode_ui(ui);
                            self.rope_distance_ui(ui);
                            ui.checkbox(&mut self.brush.stabilization_by_speed, "Less smoothing on fast strokes");
                        };
                        if compact {
                            // Already inside the Stroke options popup: show them inline.
                            follow_options(ui);
                        } else {
                            let label = if rope { "Rope" } else { "Smooth" };
                            let width = follow_width + 2.0 * ui.spacing().button_padding.x;
                            let menu = egui::Button::new(label).selected(rope).min_size(egui::vec2(width, 0.0));
                            egui::containers::menu::MenuButton::from_button(menu)
                                .config(egui::containers::menu::MenuConfig::default().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                                .ui(ui, |ui| {
                                    ui.set_width(260.0);
                                    follow_options(ui);
                                })
                                .0
                                .on_hover_text("How the line follows the pen. Rope trails it on a string and stays still until pulled; its distance is set here.");
                        }
                        ui.separator();
                        if ui
                            .add(egui::Button::new("Snap").selected(self.snapping))
                            .on_hover_text(
                                "Strokes follow rulers and the guide lines they start near",
                            )
                            .clicked()
                        {
                            self.snapping = !self.snapping;
                        }
                        if compact {
                            egui::CollapsingHeader::new(&symmetry_label).id_salt("stroke symmetry settings").show(ui,|ui|self.symmetry_ui(ui));
                        } else {
                            icons::mark(
                            ui,
                            Icon::Symmetry,
                            18.0,
                            if symmetry.active() {
                                ui.visuals().selection.stroke.color
                            } else {
                                ui.visuals().text_color()
                            },
                        );
                        // A fixed label: a mode name would widen the bar, and choosing
                        // a mode could then collapse it and close this popup.
                        let menu = egui::Button::new("Symmetry").selected(symmetry.active());
                        let content = |ui: &mut egui::Ui| {
                                ui.set_width(290.0);
                                let height=(ui.ctx().content_rect().height()-40.0).clamp(120.0,560.0);
                                egui::ScrollArea::vertical().max_height(height).show(ui, |ui| self.symmetry_ui(ui));
                        };
                        let response = egui::containers::menu::MenuButton::from_button(menu).config(egui::containers::menu::MenuConfig::default().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)).ui(ui, content);
                        self.open_capture_popup(&response.0, crate::capture::Popup::Symmetry);
                        response.0
                            .on_hover_text(format!("{symmetry_label}\nDraw mirrored or radial copies of every stroke"));
                        }
                    };
                    if compact {
                        let response = egui::containers::menu::MenuButton::new("Stroke options")
                            .config(
                                egui::containers::menu::MenuConfig::default()
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                            )
                            .ui(ui, |ui| {
                                ui.set_width(300.0);
                                let height=(ui.ctx().content_rect().height()-32.0).clamp(120.0,320.0);
                                egui::ScrollArea::vertical().max_height(height).min_scrolled_height(height).show(ui,stroke_options);
                            }).0;
                        self.open_capture_popup(&response, crate::capture::Popup::StrokeOptions);
                    } else {
                        stroke_options(ui);
                    }
                }
                Tool::Lasso => {
                    self.lasso_mode_ui(ui);
                    self.quick_color_ui(ui);
                    self.opacity_slider(ui);
                    ui.label(egui::RichText::new("Draw a shape; lift to apply").weak());
                }
                Tool::Fill => {
                    self.quick_color_ui(ui);
                    egui::containers::menu::MenuButton::new("Fill options")
                    .config(egui::containers::menu::MenuConfig::default().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
                    .ui(ui, |ui| {
                        ui.set_width(280.0);
                        self.fill_source_ui(ui);
                        self.fill_edge_options_ui(ui);
                        if tiny {
                            ui.label("Tolerance");
                            ui.add(egui::Slider::new(&mut self.fill_tolerance, 0..=255));
                            self.opacity_slider(ui);
                        }
                    });
                    if !tiny {
                        ui.label("Tolerance");
                        ui.add(egui::Slider::new(&mut self.fill_tolerance, 0..=255));
                        self.opacity_slider(ui);
                    }
                }
                Tool::Select => {
                    self.selection_transform_ui(ui);
                }
                tool => {
                    ui.label(egui::RichText::new(tool.target_hint(!self.input_stroke_target().is_raster())).weak());
                }
            }
        });
    }

    fn tool_rail(&mut self, ui: &mut egui::Ui) {
        // Tools, color chips and history controls must all fit short displays.
        let side =
            ((ui.available_height() - 80.0) / (Tool::ALL.len() as f32 + 6.0)).clamp(26.0, 42.0);
        let reveal_state = (self.tool, ui.available_height().round() as i32);
        let reveal_id = ui.make_persistent_id("tool rail current tool");
        let reveal = ui.ctx().data_mut(|data| {
            let previous = data.get_temp::<(Tool, i32)>(reveal_id);
            data.insert_temp(reveal_id, reveal_state);
            previous != Some(reveal_state)
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("tool rail scroll")
            .max_height((ui.available_height() - 2.0 * (side + 4.0) - 12.0).max(side))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for tool in Tool::ALL {
                        if tool == Tool::Brush {
                            for group in crate::brush::BrushGroup::ALL {
                                let active = self.tool == Tool::Brush && self.brush_group == group;
                                let response = icons::button(ui, group.icon(), side, active, true)
                                    .on_hover_text(format!(
                                        "{} · {} (tap again to cycle groups)
{}",
                                        group.name(),
                                        self.shortcuts
                                            .label(crate::commands::Action::Tool(Tool::Brush)),
                                        group.hint()
                                    ));
                                if reveal && active {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    self.select_brush_group(group);
                                    self.reveal_tool_settings(ui.ctx());
                                }
                            }
                            continue;
                        }
                        let icon = if tool == Tool::Lasso && self.lasso.erase {
                            Icon::LassoErase
                        } else {
                            tool.icon()
                        };
                        let response = icons::button(ui, icon, side, self.tool == tool, true)
                            .on_hover_text(format!(
                                "{} · {}
{}\nTap the shortcut to select; hold > 0.5 s to use temporarily",
                                tool.name(),
                                self.shortcuts.label(crate::commands::Action::Tool(tool)),
                                tool.target_hint(!self.input_stroke_target().is_raster())
                            ));
                        if reveal && self.tool == tool {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            self.select_tool(tool);
                            self.reveal_tool_settings(ui.ctx());
                        }
                    }
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);
                    self.extension_tools_ui(ui, side);
                    self.paint_color_chips(ui, side);
                })
            });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            if icons::button(
                ui,
                Icon::Redo,
                side,
                false,
                self.history.can_redo() || self.render_queue.busy(),
            )
            .on_hover_text(self.command_hint(crate::commands::Action::Redo, "Redo"))
            .clicked()
            {
                self.redo();
            }
            if icons::button(
                ui,
                Icon::Undo,
                side,
                false,
                self.history.can_undo() || self.render_queue.busy(),
            )
            .on_hover_text(self.command_hint(crate::commands::Action::Undo, "Undo"))
            .clicked()
            {
                self.undo();
            }
        });
    }

    /// Make library brush `index` current and remember it as recently used.
    pub(crate) fn choose_brush(&mut self, index: usize) {
        let Some(brush) = self.brushes.get(index) else {
            return;
        };
        let brush = brush.clone();
        // A preset chosen while erasing keeps the eraser's own size.
        self.unswap_eraser();
        self.selected_brush = index;
        self.brush = brush.clone();
        self.sync_eraser_slot();
        let name = brush.name.clone();
        let group = brush.group();
        self.brush_group = group;
        self.group_presets[group.index()] = name.clone();
        self.recent_brushes.retain(|recent| *recent != name);
        self.recent_brushes.insert(0, name);
        self.recent_brushes.truncate(5);
    }
    /// Switch the Brush tool to a paint group, restoring that group's live
    /// brush (with unsaved tweaks) or its last chosen preset.
    pub(crate) fn select_brush_group(&mut self, group: crate::brush::BrushGroup) {
        if self.tool != Tool::Brush {
            self.select_tool(Tool::Brush);
        }
        if self.brush_group == group {
            return;
        }
        // Stash the pen brush itself, never an eraser size held in its slot.
        self.unswap_eraser();
        let leaving = self.brush_group.index();
        self.group_live[leaving] = Some((self.selected_brush, self.brush.clone()));
        self.brush_group = group;
        if let Some((index, live)) = self.group_live[group.index()].take()
            && self
                .brushes
                .get(index)
                .is_some_and(|saved| saved.name == live.name && saved.group() == group)
        {
            self.selected_brush = index;
            self.brush = live;
            self.group_presets[group.index()] = self.brush.name.clone();
        } else {
            let remembered = &self.group_presets[group.index()];
            let index = self
                .brushes
                .iter()
                .position(|b| &b.name == remembered && b.group() == group)
                .or_else(|| self.brushes.iter().position(|b| b.group() == group));
            match index {
                Some(index) => self.choose_brush(index),
                None => {
                    self.sync_eraser_slot();
                    self.status = format!(
                        "No {} presets yet · Library › Add built-in brushes",
                        group.name().to_lowercase()
                    );
                    return;
                }
            }
        }
        self.sync_eraser_slot();
        self.scratchpad.reset_stroke();
        self.status = format!("{} · {}", group.name(), self.brush.name);
    }
    /// Returns true when a preset is chosen, including the current preset.
    pub(crate) fn brush_library_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut chosen = false;
        let compact = ui.available_height() < 230.0;
        ui.add(
            egui::TextEdit::singleline(&mut self.brush_search)
                .hint_text("Search names or tags")
                .desired_width(f32::INFINITY),
        );
        self.brush_previews.truncate(self.brushes.len());
        // Recently used brushes, one click away.
        if !compact && self.brush_search.is_empty() && !self.recent_brushes.is_empty() {
            let mut choose = None;
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Recent").small().weak());
                for name in &self.recent_brushes {
                    if let Some(index) = self.brushes.iter().position(|b| &b.name == name)
                        && index != self.selected_brush
                        && ui.small_button(name).clicked()
                    {
                        choose = Some(index);
                    }
                }
            });
            if let Some(index) = choose {
                self.choose_brush(index);
                chosen = true;
            }
        }
        let mut matches = 0;
        egui::ScrollArea::vertical()
            .id_salt("brush list")
            .auto_shrink([false, false])
            .max_height((ui.available_height() - if compact { 30.0 } else { 44.0 }).max(24.0))
            .show(ui, |ui| {
                let width = ui.available_width();
                // Searching spans every group; browsing shows the active one.
                let group = self.brush_search.trim().is_empty().then_some(self.brush_group);
                for index in 0..self.brushes.len() {
                    if !self.brushes[index].matches_search(&self.brush_search)
                        || (self.brush_favorites_only && !self.brushes[index].favorite)
                        || group.is_some_and(|g| self.brushes[index].group() != g)
                    {
                        continue;
                    }
                    matches += 1;
                    let selected = self.selected_brush == index;
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(width, if compact { 48.0 } else { 60.0 }),
                        egui::Sense::click(),
                    );
                    // Offscreen rows reserve scroll space without rendering strokes,
                    // hashing embedded tips, laying out text or uploading textures.
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let visuals = ui.visuals();
                    let fill = if selected {
                        visuals.selection.bg_fill.gamma_multiply(0.85)
                    } else if response.hovered() {
                        visuals.widgets.hovered.weak_bg_fill
                    } else {
                        visuals.extreme_bg_color.gamma_multiply(0.6)
                    };
                    let text = if selected {
                        visuals.selection.stroke.color
                    } else {
                        visuals.text_color()
                    };
                    ui.painter()
                        .rect_filled(rect.shrink2(egui::vec2(0.0, 1.0)), 6.0, fill);
                    let name = egui::WidgetText::from(self.brushes[index].name.as_str())
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Truncate),
                            (width - 44.0).max(1.0),
                            egui::FontId::proportional(13.0),
                        );
                    ui.painter()
                        .galley(rect.left_top() + egui::vec2(10.0, 6.0), name, text);
                    let texture =
                        self.brush_previews
                            .texture(ui.ctx(), index, &self.brushes[index]);
                    let aspect =
                        crate::brush_previews::WIDTH as f32 / crate::brush_previews::HEIGHT as f32;
                    let room = egui::Rect::from_min_max(
                        rect.left_top() + egui::vec2(6.0, 22.0),
                        rect.right_bottom() - egui::vec2(6.0, 3.0),
                    );
                    let size = egui::vec2(
                        room.width().min(room.height() * aspect),
                        room.height().min(room.width() / aspect),
                    );
                    ui.painter().image(
                        texture,
                        egui::Rect::from_center_size(room.center(), size),
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        text,
                    );
                    let tags = if self.brushes[index].tags.is_empty() {
                        "Choose this brush; use Brush settings to edit it".to_owned()
                    } else {
                        format!("Tags: {}", self.brushes[index].tags)
                    };
                    // Star toggles a favourite without choosing the brush.
                    let star = egui::Rect::from_center_size(
                        rect.right_top() + egui::vec2(-16.0, 13.0),
                        egui::vec2(20.0, 20.0),
                    );
                    let star_response = ui
                        .interact(star, response.id.with("favorite"), egui::Sense::click())
                        .on_hover_text(if self.brushes[index].favorite {
                            "Remove from favourites"
                        } else {
                            "Add to favourites"
                        });
                    paint_star(
                        ui.painter(),
                        star.center(),
                        7.0,
                        self.brushes[index].favorite,
                        star_response.hovered(),
                    );
                    if star_response.clicked() {
                        self.toggle_brush_favorite(index);
                    }
                    let response =
                        response.on_hover_text(format!("{}\n{tags}", self.brushes[index].name));
                    if !star_response.clicked() && (response.clicked() || response.double_clicked())
                    {
                        self.choose_brush(index);
                        chosen = true;
                    }
                    response.context_menu(|ui| {
                        if ui.button("Duplicate preset").clicked() {
                            self.duplicate_brush_preset(index);
                            ui.close();
                        }
                    });
                }
                if matches == 0 {
                    ui.label(if self.brush_favorites_only {
                        "No favourites yet. Star a brush to keep it here.".to_owned()
                    } else if let Some(group) = group {
                        format!(
                            "No {} presets. Library › Add built-in brushes, or set a preset's group in Brush settings.",
                            group.name().to_lowercase()
                        )
                    } else {
                        "No matching brushes. Try another name or tag.".to_owned()
                    });
                }
            });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(egui::Button::new("Favourites").selected(self.brush_favorites_only))
                .on_hover_text("Show only starred brushes")
                .clicked()
            {
                self.brush_favorites_only = !self.brush_favorites_only;
            }
            ui.menu_button("Library", |ui| {
                if ui.button("Import brush…").clicked() {
                    self.brush_file(true);
                    ui.close();
                }
                if ui.button("Import pack…").clicked() {
                    self.brush_pack_file(true);
                    ui.close();
                }
                if ui.button("Export library…").clicked() {
                    self.brush_pack_file(false);
                    ui.close();
                }
                ui.separator();
                if ui
                    .button("Add built-in brushes")
                    .on_hover_text(
                        "Adds any built-in preset missing from your library; nothing is replaced",
                    )
                    .clicked()
                {
                    let added = add_missing_defaults(&mut self.brushes);
                    self.status = format!("Added {added} built-in brushes");
                    ui.close();
                }
            });
        });
        chosen
    }

    fn brush_picker_ui(&mut self, ui: &mut egui::Ui, width: f32) {
        let name = self.brush.name.clone();
        let response = ui
            .allocate_ui_with_layout(
                egui::vec2(width, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    egui::containers::menu::MenuButton::from_button(
                        egui::Button::new(&name).truncate(),
                    )
                    .config(
                        egui::containers::menu::MenuConfig::default()
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                    )
                    .ui(ui, |ui| {
                        ui.set_width(280.0);
                        // Reserve the settings action before the library takes the
                        // remaining space. Keep the whole popup inside short displays.
                        let height = (ui.ctx().content_rect().height() - 24.0).clamp(120.0, 440.0);
                        ui.set_max_height(height - 48.0);
                        if self.tool == Tool::Eraser {
                            self.eraser_mode_ui(ui);
                            ui.separator();
                            if self.eraser_own {
                                self.eraser_tip_ui(ui);
                                return;
                            }
                        }
                        let selected = self.selected_brush;
                        let chosen = ui
                            .push_id("quick brush library", |ui| self.brush_library_ui(ui))
                            .inner;
                        if chosen || self.selected_brush != selected {
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Brush settings…").clicked() {
                            self.show_brush_settings();
                            ui.close();
                        }
                    })
                },
            )
            .inner
            .0;
        self.open_capture_popup(&response, crate::capture::Popup::Brushes);
        response.on_hover_text(format!("{name}\nChoose a brush or open brush settings"));
    }

    fn quick_color_ui(&mut self, ui: &mut egui::Ui) {
        let color = self.paint_color();
        let masked = !self.input_stroke_target().is_raster();
        let label = if masked { "Coverage" } else { "Color" };
        let [r, g, b] = color.map(u32::from);
        let text = if 299 * r + 587 * g + 114 * b > 150_000 {
            egui::Color32::BLACK
        } else {
            egui::Color32::WHITE
        };
        let button = egui::Button::new(egui::RichText::new(label).color(text)).fill(color32(color));
        let response = egui::containers::menu::MenuButton::from_button(button)
            .config(
                egui::containers::menu::MenuConfig::default()
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
            )
            .ui(ui, |ui| {
                ui.set_width(260.0);
                let height = (ui.ctx().content_rect().height() - 24.0).clamp(120.0, 420.0);
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .show(ui, |ui| self.palette_ui(ui));
            })
            .0;
        self.open_capture_popup(&response, crate::capture::Popup::Color);
        response.on_hover_text(if masked {
            "Choose mask coverage; black conceals and white reveals"
        } else {
            "Choose drawing color · wheel, hex, recent colors and swatches"
        });
    }

    fn status_message_ui(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width().max(0.0);
        // Left to right inside whatever layout the bar uses, so the
        // recording indicator always sits before the message.
        ui.allocate_ui_with_layout(
            egui::vec2(width, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                self.import_report_button(ui);
                // Recording stays visible beside the status, never instead of it.
                if let Some(label) = self.timelapse_status() {
                    let recording = label == "Recording timelapse";
                    let response = ui
                        .add(egui::Button::new(if recording { "   REC" } else { "Paused" }).small())
                        .on_hover_text(format!("{label} · open timelapse controls"));
                    if recording {
                        ui.painter().circle_filled(
                            egui::pos2(response.rect.left() + 9.0, response.rect.center().y),
                            4.0,
                            egui::Color32::from_rgb(220, 60, 60),
                        );
                    }
                    if response.clicked() {
                        self.timelapse.open = true;
                    }
                }
                // Background work replaces the message in place, so the bar
                // and canvas never change size while it runs.
                let busy = self.save_progress_ui(ui)
                    | self.load_progress_ui(ui)
                    | self.filter_progress_ui(ui);
                if busy {
                    return;
                }
                if self.effects_waiting {
                    // Fixed size: the theme's touch-sized interact height
                    // would grow the status row and shrink the canvas.
                    ui.add(egui::Spinner::new().size(16.0));
                    ui.weak("Drawing layer effects…")
                        .on_hover_text("The previous drawing shows until the new one is ready");
                }
                if self.history.over_budget() {
                    let text = format!(
                        "Large edit: history is above its {} MiB budget",
                        crate::document::History::BUDGET_MIB
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&text).color(ui.visuals().warn_fg_color),
                        )
                        .truncate(),
                    )
                    .on_hover_text(text);
                } else {
                    ui.add(egui::Label::new(&self.status).truncate())
                        .on_hover_text(&self.status);
                }
            },
        );
    }

    fn status_view_buttons(&mut self, ui: &mut egui::Ui) {
        if icons::button(ui, Icon::Flip, 26.0, self.mirror, true)
            .on_hover_text(self.command_hint(
                crate::commands::Action::FlipView,
                "Flip canvas horizontally (view only)",
            ))
            .clicked()
        {
            self.toggle_flip();
        }
        if icons::button(ui, Icon::Transparency, 26.0, self.show_transparency, true)
            .on_hover_text(self.command_hint(
                crate::commands::Action::Transparency,
                "Show transparency as a checkerboard (view only; exports unchanged)",
            ))
            .clicked()
        {
            self.set_show_transparency(!self.show_transparency);
        }
    }

    /// Shows that unsaved changes have a crash-recovery backup, distinct from
    /// the project file, and briefly highlights each newly written backup.
    fn recovery_cue_ui(&self, ui: &mut egui::Ui) {
        let Some(saved) = self.recovery_saved.filter(|_| self.modified) else {
            return;
        };
        let age = saved.elapsed().as_secs();
        let when = if age < 60 {
            "just now".to_owned()
        } else {
            format!("{} min ago", age / 60)
        };
        let text = egui::RichText::new(format!("Backup saved {when}"));
        let fresh = age < 3;
        let text = if fresh {
            text.color(ui.visuals().selection.bg_fill)
        } else {
            text.weak()
        };
        let pending = if self.revision != self.recovery_revision {
            " Newer changes will be backed up shortly."
        } else {
            ""
        };
        ui.label(text).on_hover_text(format!(
            "Unsaved changes are backed up for crash recovery.{pending}\n\
             This backup is not your project file: use Save to keep your work."
        ));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(if fresh { 1 } else { 30 }));
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
        if width < 1100.0 {
            // Narrow windows give the message its own row; complete control
            // groups wrap rather than clipping the last buttons off-screen.
            self.status_message_ui(ui);
            ui.horizontal_wrapped(|ui| {
                let slider = if width < 700.0 { 50.0 } else { 80.0 };
                self.zoom_controls(ui, slider);
                self.rotation_controls(ui, slider);
                self.status_view_buttons(ui);
                ui.weak(format!("{} × {} px", self.doc.width, self.doc.height));
                self.recovery_cue_ui(ui);
            });
        } else {
            // Allocate real controls from the right, then use exactly the
            // remaining width for the message. No guessed reservation widths.
            // A fixed row height: a bare right-to-left layout would claim all
            // the height the bottom panel offers.
            let row = egui::vec2(ui.available_width(), 28.0);
            ui.allocate_ui_with_layout(
                row,
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if icons::button(ui, Icon::Transparency, 26.0, self.show_transparency, true)
                        .on_hover_text(self.command_hint(
                            crate::commands::Action::Transparency,
                            "Show transparency as a checkerboard",
                        ))
                        .clicked()
                    {
                        self.set_show_transparency(!self.show_transparency);
                    }
                    if icons::button(ui, Icon::Flip, 26.0, self.mirror, true)
                        .on_hover_text(self.command_hint(
                            crate::commands::Action::FlipView,
                            "Flip canvas horizontally (view only)",
                        ))
                        .clicked()
                    {
                        self.toggle_flip();
                    }
                    let slider = if width >= 1400.0 { 110.0 } else { 70.0 };
                    self.rotation_controls(ui, slider);
                    ui.separator();
                    self.zoom_controls(ui, slider);
                    ui.separator();
                    ui.weak(format!("{} × {} px", self.doc.width, self.doc.height));
                    self.recovery_cue_ui(ui);
                    if self.pressure_seen && width >= 1400.0 {
                        ui.weak(format!("Pen {:.0}%", self.pressure.unwrap_or(0.0) * 100.0));
                    }
                    self.status_message_ui(ui);
                },
            );
        }
    }

    pub(crate) fn inspector_visible(&self, ctx: &egui::Context) -> bool {
        self.inspector_override
            .unwrap_or(ctx.content_rect().width() >= 1000.0)
    }
    /// A tool-rail click shows that tool's settings, whether they were closed by
    /// dragging or hidden by the automatic layout on a smaller workspace.
    /// Keyboard tool switches leave the panel as it is.
    pub(crate) fn reveal_tool_settings(&mut self, ctx: &egui::Context) {
        if !self.tool_panel_visible(ctx) {
            self.tool_panel_override = Some(true);
        }
    }
    pub(crate) fn tool_panel_visible(&self, ctx: &egui::Context) -> bool {
        self.tool_panel_override.unwrap_or_else(|| {
            // Keep at least 65% of a compact workspace for drawing. Brush
            // selection lives in the options bar; advanced tuning is opt-in.
            // Other tools still expose their contextual settings by default.
            let minimum = if matches!(self.tool, Tool::Brush | Tool::Eraser)
                && self.affine_preview.is_none()
            {
                2000.0
            } else {
                1100.0
            };
            ctx.content_rect().width() >= minimum
        })
    }

    pub(crate) fn document_status_ui(&self, ui: &mut egui::Ui) {
        let remaining = ui.available_width();
        ui.allocate_ui_with_layout(
            egui::vec2(remaining, ui.spacing().interact_size.y),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(if self.modified {
                    egui::RichText::new("Unsaved changes").strong()
                } else {
                    egui::RichText::new("Saved").weak()
                });
                let name = self
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or("Untitled".into());
                ui.add(egui::Label::new(egui::RichText::new(&name).strong()).truncate())
                    .on_hover_text(name);
            },
        );
    }

    pub(crate) fn workspace(&mut self, ctx: &egui::Context) {
        self.window_resize_ui(ctx);
        self.navigator_paint = None;
        if self.show_panels {
            self.refresh_thumbnails(ctx);
        }
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            let compact = ctx.content_rect().width() < 900.0;
            let brand = |ui: &mut egui::Ui| {
                ui.label(
                    egui::RichText::new("DUDEL")
                        .strong()
                        .color(ui.visuals().text_color()),
                );
                ui.separator();
            };
            if compact {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.horizontal(|ui| {
                    brand(ui);
                    self.window_title_ui(ui);
                });
            }
            egui::MenuBar::new().ui(ui, |ui| {
                if compact {
                    ui.spacing_mut().button_padding.x = 5.0;
                    ui.spacing_mut().item_spacing.x = 4.0;
                } else {
                    brand(ui);
                }
                self.menus(ui);
                if !compact {
                    self.window_title_ui(ui);
                }
            });
            ui.add_space(2.0);
            self.options_bar(ui);
            ui.add_space(2.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        if self.show_panels {
            egui::SidePanel::left("tools")
                .exact_width(TOOL_RAIL_WIDTH)
                .resizable(false)
                .show(ctx, |ui| self.tool_rail(ui));
            let tool_panel = self.tool_panel_visible(ctx);
            let inspector = self.inspector_visible(ctx);
            if tool_panel {
                let other = inspector
                    .then(|| self.sidebar_widths[1].or(sidebar_width(ctx, INSPECTOR)))
                    .flatten();
                let range = sidebar_range(ctx, 260.0, 480.0, other);
                if pin_sidebar_width(
                    ctx,
                    TOOL_PANEL,
                    true,
                    true,
                    &mut self.sidebar_widths[0],
                    &range,
                ) {
                    self.tool_panel_override = Some(false);
                    self.status = "Tool settings closed · click a tool to open them again".into();
                }
                egui::SidePanel::left(TOOL_PANEL)
                    .default_width(280.0)
                    .width_range(range)
                    .show(ctx, |ui| {
                        bounded(ui, |ui| {
                            // Color sits under the tool settings in its own
                            // resizable area, beside the canvas being painted.
                            let total = ui.available_height();
                            // Tool settings scroll, so color may take all but 180 px.
                            let minimum = 120.0_f32.min(total.max(0.0));
                            let height = self
                                .color_height
                                .unwrap_or((total * 0.35).clamp(210.0, 270.0))
                                .clamp(minimum, (total - 180.0).max(minimum));
                            egui::TopBottomPanel::bottom("color area")
                                .exact_height(height)
                                .frame(egui::Frame::NONE)
                                .show_inside(ui, |ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                                    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
                                    ui.spacing_mut().interact_size.y = 26.0;
                                    match split_handle(ui) {
                                        SplitDrag::Reset => self.color_height = None,
                                        SplitDrag::By(delta) => {
                                            self.color_height = Some(height - delta)
                                        }
                                        SplitDrag::None => {}
                                    }
                                    ui.label(section_title(
                                        if self.input_stroke_target().is_raster() {
                                            "COLOR"
                                        } else {
                                            "MASK COVERAGE"
                                        },
                                    ));
                                    self.compact_palette_ui(ui);
                                });
                            if self.tool == Tool::Fill || self.affine_preview.is_some() {
                                egui::ScrollArea::vertical()
                                    .id_salt(if self.affine_preview.is_some() {
                                        "transform tool properties"
                                    } else {
                                        "fill tool properties"
                                    })
                                    .max_height(ui.available_height().max(0.0))
                                    .min_scrolled_height(0.0)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| self.tool_panel_ui(ui));
                            } else {
                                self.tool_panel_ui(ui);
                            }
                        })
                    });
            }
            if inspector {
                let other = tool_panel
                    .then(|| self.sidebar_widths[0].or(sidebar_width(ctx, TOOL_PANEL)))
                    .flatten();
                let range = sidebar_range(ctx, 240.0, 520.0, other);
                pin_sidebar_width(
                    ctx,
                    INSPECTOR,
                    false,
                    false,
                    &mut self.sidebar_widths[1],
                    &range,
                );
                egui::SidePanel::right(INSPECTOR)
                    .default_width(290.0)
                    .width_range(range)
                    .show(ctx, |ui| {
                        bounded(ui, |ui| {
                            // Layers keep a fixed, resizable area so they never
                            // scroll out of reach behind the navigator and color.
                            let total = ui.available_height();
                            if total < 360.0 {
                                // Leave room for search/filter controls and at least one layer row.
                                let inline_controls = total >= 300.0;
                                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                                ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
                                ui.spacing_mut().interact_size.y = 24.0;
                                ui.horizontal(|ui| {
                                    ui.menu_button("Navigator", |ui| {
                                        ui.set_width(240.0);
                                        let room = (ui.ctx().content_rect().height() - 80.0)
                                            .clamp(140.0, 360.0);
                                        self.navigator_ui(ui, room);
                                    });
                                    if !tool_panel {
                                        ui.menu_button("Color", |ui| {
                                            ui.set_width(240.0);
                                            let room = (ui.ctx().content_rect().height() - 80.0)
                                                .max(100.0);
                                            egui::ScrollArea::vertical()
                                                .max_height(room)
                                                .show(ui, |ui| self.palette_ui(ui));
                                        });
                                    }
                                    if !inline_controls {
                                        ui.menu_button("Layer", |ui| {
                                            ui.set_width(280.0);
                                            self.layer_controls_ui(ui);
                                        })
                                        .response
                                        .on_hover_text(
                                            "Layer actions, blend, opacity and mask settings",
                                        );
                                    }
                                });
                                ui.horizontal(|ui| {
                                    if total < 200.0 {
                                        ui.menu_button(section_title("LAYERS"), |ui| {
                                            ui.set_width(280.0);
                                            self.layer_navigation_ui(ui);
                                        }).response.on_hover_text("Search layers, filter types and reveal the active layer");
                                    } else {
                                        ui.label(section_title("LAYERS"));
                                    }
                                    self.layer_guide_controls(ui);
                                });
                                self.layers_ui(ui, inline_controls, total >= 200.0);
                                return;
                            }
                            let minimum = (if self.mask_controls_available() {
                                300.0_f32
                            } else {
                                250.0_f32
                            })
                            .min(total.max(0.0));
                            let height = self
                                .layers_height
                                .unwrap_or_else(|| {
                                    if self.reference_layers.active.is_some() {
                                        return total * 0.7;
                                    }
                                    if tool_panel {
                                        // Color is in the left panel: layers
                                        // take the room under the navigator.
                                        return (total - 400.0).max(total * 0.5);
                                    }
                                    let rows = self.doc.layers.len().min(8) as f32;
                                    (150.0 + rows * 50.0).min(total * 0.5)
                                })
                                .clamp(minimum, (total - 140.0).max(minimum));
                            egui::TopBottomPanel::bottom("layers area")
                                .exact_height(height)
                                .frame(egui::Frame::NONE)
                                .show_inside(ui, |ui| {
                                    match split_handle(ui) {
                                        SplitDrag::Reset => self.layers_height = None,
                                        SplitDrag::By(delta) => {
                                            self.layers_height = Some(height - delta)
                                        }
                                        SplitDrag::None => {}
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(section_title("LAYERS"));
                                        self.layer_guide_controls(ui);
                                    });
                                    self.layers_ui(ui, true, true);
                                });
                            let viewport_height = ui.available_height();
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let content_top = ui.cursor().top();
                                ui.add_space(6.0);
                                section("NAVIGATOR").show(ui, |ui| {
                                    let height =
                                        viewport_height - (ui.cursor().top() - content_top);
                                    self.navigator_ui(ui, height);
                                });
                                // Color lives in the left panel; keep it
                                // reachable here while that panel is closed.
                                if !tool_panel {
                                    ui.separator();
                                    section(if self.input_stroke_target().is_raster() {
                                        "COLOR"
                                    } else {
                                        "MASK COVERAGE"
                                    })
                                    .show(ui, |ui| self.palette_ui(ui));
                                }
                            });
                        })
                    });
            }
        }
        self.shortcuts_ui(ctx);
        self.about_ui(ctx);
        self.extension_manager_ui(ctx);
        self.import_report_ui(ctx);
        self.extension_panels_ui(ctx);
        self.settings_ui(ctx);
        self.references_ui(ctx);
        let mut input_settings = self.input_settings;
        egui::Window::new("Tablet input")
            .open(&mut input_settings)
            .show(ctx, |ui| {
                ui.label(if self.pressure_seen {
                    "Pressure capability confirmed this session"
                } else {
                    "Draw a stroke to detect pressure"
                });
                ui.add(egui::ProgressBar::new(self.pressure.unwrap_or(0.0)).show_percentage());
                ui.label("Pressure controls are available in Brush Editor.");
                if let Some(mut enabled) = self.input.native_enabled()
                    && ui
                        .checkbox(&mut enabled, NATIVE_PEN_LABEL)
                        .changed()
                {
                    self.input.set_native_enabled(enabled);
                }
                ui.separator();
                let diagnostics = &self.input.diagnostics;
                ui.label(if diagnostics.status.is_empty() {
                    "egui input fallback"
                } else {
                    &diagnostics.status
                });
                ui.label(format!("Received packets: {}", diagnostics.packets));
                if let Some(sample) = diagnostics.last {
                    ui.label(if sample.time.is_some() {
                        "Sample timing: native monotonic clock"
                    } else {
                        "Sample timing: unavailable in fallback input"
                    });
                    if let Some([x, y]) = sample.tilt {
                        ui.label(format!("Tilt: {x:.0}°, {y:.0}°"));
                    }
                    if let Some(twist) = sample.twist {
                        ui.label(format!("Rotation: {twist:.0}°"));
                    }
                    ui.label(format!(
                        "Eraser end: {} · Barrel button: {}",
                        sample.eraser, sample.barrel
                    ));
                }
                if ui.button("Export recent input…").clicked() {
                    let dialog = rfd::AsyncFileDialog::new()
                        .add_filter("Input recording", &["json"])
                        .set_file_name("dudel-input.json");
                    self.ask_save_file(dialog, |app, path| {
                        app.status = match app.input.diagnostics.export(&path) {
                            Ok(()) => "Exported recent input packets".into(),
                            Err(error) => format!("Input export: {error}"),
                        };
                    });
                }
                ui.small("Recordings contain up to 4,096 recent packets and screen coordinates.");
                ui.separator();
                if ui.checkbox(&mut self.replay.enabled, "Capture painted strokes for replay").changed() {
                    self.replay.clear();
                    self.replay.status.clear();
                }
                ui.small("Captures one active-layer stroke, its brush/view settings and starting/result pixels. Up to 65,536 commands and 32MiB of layer snapshots; off by default.");
                if !self.replay.status.is_empty() { ui.label(&self.replay.status); }
                if ui.add_enabled(self.replay.ready(), egui::Button::new("Export last stroke replay…")).clicked() {
                    let dialog = rfd::AsyncFileDialog::new().add_filter("Stroke replay", &["dudelreplay"]).set_file_name("stroke.dudelreplay");
                    self.ask_save_file(dialog, |app, path| {
                        app.status = match app.replay.export(&path) {
                            Ok(()) => "Exported reproducible active-layer stroke".into(),
                            Err(error) => format!("Stroke export: {error}"),
                        };
                    });
                }
            });
        self.input_settings = input_settings;
        self.new_canvas_ui(ctx);
        self.canvas_size_ui(ctx);
        self.adjustment_ui(ctx);
        self.command_search_ui(ctx);
        self.shortcut_capture_ui(ctx);
    }
}

pub fn default_brushes() -> Vec<Brush> {
    use crate::brush::BrushGroup;
    vec![
        Brush {
            tags: "ink".into(),
            group: Some(BrushGroup::Pen),
            ..Brush::default()
        },
        Brush {
            name: "G-pen".into(),
            group: Some(BrushGroup::Pen),
            tags: "ink, lineart, comic".into(),
            size: 14.0,
            hardness: 0.92,
            spacing: 0.06,
            stabilization: 3.0,
            ..Brush::default()
        },
        Brush {
            name: "Pencil".into(),
            group: Some(BrushGroup::Pencil),
            tags: "sketch, pencil, texture".into(),
            size: 7.0,
            opacity: 0.9,
            hardness: 0.55,
            spacing: 0.1,
            pressure_opacity: true,
            grain_amount: 0.55,
            ..Brush::default()
        },
        Brush {
            name: "Fine liner".into(),
            group: Some(BrushGroup::Pen),
            tags: "ink, detail".into(),
            size: 5.0,
            hardness: 1.0,
            ..Brush::default()
        },
        Brush {
            name: "Chisel marker".into(),
            group: Some(BrushGroup::Pen),
            tags: "lettering, calligraphy".into(),
            size: 26.0,
            hardness: 0.8,
            spacing: 0.05,
            aspect: 0.3,
            angle: 45.0,
            pressure_size: false,
            ..Brush::default()
        },
        Brush {
            name: "Soft airbrush".into(),
            group: Some(BrushGroup::Airbrush),
            tags: "soft, shading, blending".into(),
            size: 80.0,
            opacity: 0.12,
            hardness: 0.0,
            ..Brush::default()
        },
        Brush {
            name: "Dry gouache".into(),
            group: Some(BrushGroup::Brush),
            tags: "paint, dry, opaque, texture".into(),
            size: 38.0,
            flow: 0.8,
            hardness: 0.85,
            spacing: 0.08,
            grain: Some(paper_grain()),
            grain_amount: 0.55,
            grain_scale: 10.0,
            pressure_opacity: true,
            angle_source: crate::brush::AngleSource::Direction,
            aspect: 0.7,
            ..Brush::default()
        },
        Brush {
            name: "Charcoal".into(),
            group: Some(BrushGroup::Pencil),
            tags: "sketch, charcoal, texture, shading".into(),
            size: 28.0,
            opacity: 0.75,
            flow: 0.45,
            hardness: 0.5,
            spacing: 0.12,
            grain: Some(paper_grain()),
            grain_amount: 0.8,
            grain_scale: 6.0,
            pressure_opacity: true,
            aspect: 0.55,
            angle: 25.0,
            ..Brush::default()
        },
        Brush {
            name: "Round wash".into(),
            group: Some(BrushGroup::Brush),
            tags: "paint, wash, glazing, soft".into(),
            size: 64.0,
            opacity: 0.3,
            flow: 0.14,
            hardness: 0.2,
            spacing: 0.07,
            pressure_opacity: true,
            softness_profile: crate::brush::SoftnessProfile::Eased,
            ..Brush::default()
        },
        Brush {
            name: "Flat bristle".into(),
            group: Some(BrushGroup::Brush),
            tags: "paint, flat, bristle, texture".into(),
            size: 40.0,
            flow: 0.65,
            spacing: 0.06,
            aspect: 0.4,
            tip: crate::brush::Tip::Image,
            image: Some(bristle_tip()),
            angle_source: crate::brush::AngleSource::Direction,
            pressure_opacity: true,
            ..Brush::default()
        },
        Brush {
            name: "Mechanical pencil".into(),
            group: Some(BrushGroup::Pencil),
            tags: "sketch, pencil, detail".into(),
            size: 3.0,
            opacity: 0.85,
            hardness: 0.75,
            spacing: 0.08,
            pressure_opacity: true,
            grain: Some(paper_grain()),
            grain_amount: 0.35,
            grain_scale: 2.0,
            ..Brush::default()
        },
        Brush {
            name: "Soft 6B".into(),
            group: Some(BrushGroup::Pencil),
            tags: "sketch, pencil, shading, texture".into(),
            size: 14.0,
            opacity: 0.8,
            flow: 0.6,
            hardness: 0.35,
            spacing: 0.08,
            pressure_opacity: true,
            pressure_gamma: 1.3,
            grain: Some(paper_grain()),
            grain_amount: 0.75,
            grain_scale: 4.0,
            ..Brush::default()
        },
        Brush {
            name: "Maru pen".into(),
            group: Some(BrushGroup::Pen),
            tags: "ink, lineart, detail".into(),
            size: 4.0,
            hardness: 1.0,
            spacing: 0.04,
            stabilization: 4.0,
            taper_in: 6.0,
            taper_out: 10.0,
            ..Brush::default()
        },
        Brush {
            name: "Brush pen".into(),
            group: Some(BrushGroup::Pen),
            tags: "ink, lineart, expressive".into(),
            size: 20.0,
            hardness: 0.95,
            spacing: 0.04,
            pressure_gamma: 1.6,
            stabilization: 3.0,
            taper_in: 18.0,
            taper_out: 36.0,
            ..Brush::default()
        },
        Brush {
            name: "Hard airbrush".into(),
            group: Some(BrushGroup::Airbrush),
            tags: "shading, cel".into(),
            size: 48.0,
            opacity: 0.25,
            flow: 0.5,
            hardness: 0.55,
            spacing: 0.06,
            pressure_size: false,
            pressure_opacity: true,
            ..Brush::default()
        },
        Brush {
            name: "Spray".into(),
            group: Some(BrushGroup::Airbrush),
            tags: "texture, spray, speckle".into(),
            size: 6.0,
            opacity: 0.7,
            flow: 0.6,
            hardness: 0.6,
            spacing: 0.05,
            scatter: 2.0,
            pressure_size: false,
            pressure_opacity: true,
            ..Brush::default()
        },
        Brush {
            name: "Blending wash".into(),
            group: Some(BrushGroup::Brush),
            tags: "paint, blending, soft".into(),
            size: 90.0,
            opacity: 0.18,
            flow: 0.1,
            hardness: 0.0,
            spacing: 0.06,
            pressure_opacity: true,
            softness_profile: crate::brush::SoftnessProfile::Eased,
            ..Brush::default()
        },
    ]
}

/// Small embedded textures keep presets portable and deterministic.
fn paper_grain() -> crate::brush::TipImage {
    let alpha: Vec<u8> = (0..32 * 32)
        .map(|i| {
            let n = (i as u32).wrapping_mul(747796405).wrapping_add(2891336453);
            48 + ((n ^ (n >> 13)) % 208) as u8
        })
        .collect();
    crate::brush::TipImage {
        width: 32,
        height: 32,
        alpha: alpha.into(),
    }
}
fn bristle_tip() -> crate::brush::TipImage {
    let alpha: Vec<u8> = (0..16 * 16)
        .map(|i| {
            let (x, y) = (i % 16, i / 16);
            let edge = if y == 0 || y == 15 { 100 } else { 255 };
            if x % 3 == 0 { edge / 4 } else { edge }
        })
        .collect();
    crate::brush::TipImage {
        width: 16,
        height: 16,
        alpha: alpha.into(),
    }
}

/// Add built-in presets whose names are missing from a library.
pub fn add_missing_defaults(library: &mut Vec<Brush>) -> usize {
    let mut added = 0;
    for preset in default_brushes() {
        if library.len() < 512 && !library.iter().any(|b| b.name == preset.name) {
            library.push(preset);
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    #[test]
    fn overflowing_sidebar_content_leaves_no_gap_before_the_canvas() {
        // Content twice the panel's width: unbounded, egui would start the
        // central area at the content edge, beyond the painted panel.
        for bound in [false, true] {
            let ctx = egui::Context::default();
            let mut gap = 0.0;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    let wide = |ui: &mut egui::Ui| {
                        ui.horizontal(|ui| {
                            ui.allocate_exact_size(egui::vec2(600.0, 20.0), egui::Sense::hover());
                        });
                    };
                    let panel =
                        egui::SidePanel::left("overflow")
                            .exact_width(300.0)
                            .show(ctx, |ui| {
                                if bound {
                                    super::bounded(ui, wide);
                                } else {
                                    wide(ui);
                                }
                            });
                    let central = egui::CentralPanel::default().show(ctx, |_| {});
                    // The panel paints and clips at its set width.
                    let painted = panel.response.rect.left() + 300.0;
                    gap = central.response.rect.left() - painted;
                });
            }
            if bound {
                assert!(gap.abs() < 1.0, "gap {gap}");
            } else {
                assert!(
                    gap > 100.0,
                    "unbounded overflow no longer reproduces: {gap}"
                );
            }
        }
    }
    #[test]
    fn compact_toolbar_respects_viewport_when_parent_retains_a_wider_layout() {
        let ctx = egui::Context::default();
        let mut app = Dudel::default();
        assert!(app.create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll));
        app.flush_render_queue();
        fn bounds(shape: &egui::Shape) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Stroke options" => Some(
                    egui::Rect::from_min_size(text.pos, text.galley.size())
                        .expand2(egui::vec2(10.0, 7.0)),
                ),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(bounds),
                _ => None,
            }
        }
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 420.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    super::theme(ctx);
                    egui::TopBottomPanel::top("wide toolbar fixture").show(ctx, |ui| {
                        ui.set_min_width(1000.0);
                        app.options_bar(ui);
                    });
                },
            );
            let rect = output
                .shapes
                .iter()
                .find_map(|s| bounds(&s.shape))
                .expect("Stroke options");
            assert!(
                ctx.content_rect().contains_rect(rect),
                "Button {rect:?} exceeds viewport {:?}",
                ctx.content_rect()
            );
        }
    }
    use super::*;
    #[test]
    fn large_brush_library_uploads_only_visible_previews() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            brushes: vec![Brush::default(); 512],
            ..Default::default()
        };
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 360.0),
                )),
                ..Default::default()
            },
            |ctx| {
                theme(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.brush_library_ui(ui);
                });
            },
        );
        assert!(
            output.textures_delta.set.len() < 12,
            "offscreen previews must not be uploaded: {}",
            output.textures_delta.set.len()
        );
        app.brushes[511].name = "Only last brush".into();
        app.brush_search = "Only last brush".into();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 360.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.brush_library_ui(ui);
                });
            },
        );
        assert!(
            output.textures_delta.set.len() < 4,
            "searching a late entry must not upload placeholder textures: {}",
            output.textures_delta.set.len()
        );
    }

    #[test]
    fn compact_drawing_workspace_preserves_canvas_and_explicit_panel_choice() {
        for width in [1280.0, 1600.0, 1920.0, 2560.0] {
            let ctx = egui::Context::default();
            let mut app = Dudel::default();
            let frame = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.0),
                )),
                ..Default::default()
            };
            for _ in 0..3 {
                let _ = ctx.run(frame(), |ctx| {
                    theme(ctx);
                    app.workspace(ctx);
                    app.canvas(ctx, &[]);
                });
            }
            assert!(
                app.canvas_area.width() >= width * 0.65,
                "workspace {width}: canvas {}",
                app.canvas_area.width()
            );
            assert_eq!(app.tool_panel_visible(&ctx), width >= 2000.0);
            app.tool = Tool::Fill;
            assert!(app.tool_panel_visible(&ctx));
            app.tool_panel_override = Some(false);
            assert!(!app.tool_panel_visible(&ctx));
            app.show_brush_settings();
            assert!(app.tool_panel_visible(&ctx));
        }
    }

    #[test]
    fn default_presets_are_valid_and_restoring_adds_only_missing_names() {
        let defaults = default_brushes();
        for brush in &defaults {
            brush.validate().unwrap();
        }
        let mut library = vec![defaults[0].clone()];
        assert_eq!(add_missing_defaults(&mut library), defaults.len() - 1);
        assert_eq!(add_missing_defaults(&mut library), 0);
    }
    #[test]
    fn tool_panel_keeps_the_dragged_width_for_every_tool() {
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            ..Default::default()
        };
        for tool in Tool::ALL {
            for width in [260.0, 420.0] {
                let ctx = egui::Context::default();
                let mut app = Dudel {
                    tool,
                    tool_panel_override: Some(true),
                    inspector_override: Some(true),
                    sidebar_widths: [Some(width), None],
                    ..Default::default()
                };
                for _ in 0..30 {
                    let _ = ctx.run(raw(), |ctx| {
                        theme(ctx);
                        app.workspace(ctx);
                        app.canvas(ctx, &[]);
                    });
                }
                // egui saves the content width, so this fails if content
                // overflows or underfills the panel.
                assert_eq!(
                    sidebar_width(&ctx, TOOL_PANEL),
                    Some(width),
                    "{}",
                    tool.name()
                );
                assert_eq!(app.sidebar_widths[0], Some(width));
            }
        }
    }
    #[test]
    fn f6_enters_controls_and_preserves_navigation_activation_and_text_input() {
        fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
            [true, false]
                .map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                })
                .to_vec()
        }
        fn frame(
            ctx: &egui::Context,
            events: Vec<egui::Event>,
            activated: &mut usize,
            text: &mut String,
        ) -> (bool, Vec<egui::Id>) {
            let mut raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let toggled = take_panel_toggle(ctx, &mut raw);
            let mut ids = Vec::new();
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let first = ui.button("First control");
                    if first.clicked() {
                        *activated += 1;
                    }
                    ids.push(first.id);
                    ids.push(ui.button("Second control").id);
                    ids.push(ui.text_edit_singleline(text).id);
                });
            });
            (toggled, ids)
        }
        let ctx = egui::Context::default();
        let mut activated = 0;
        let mut text = String::new();
        let (_, ids) = frame(&ctx, vec![], &mut activated, &mut text);
        assert!(ctx.memory(|m| m.focused()).is_none());
        let (toggled, _) = frame(
            &ctx,
            key(egui::Key::F6, egui::Modifiers::NONE),
            &mut activated,
            &mut text,
        );
        assert!(!toggled);
        assert_eq!(ctx.memory(|m| m.focused()), Some(ids[0]));
        assert!(!ctx.input(|i| i.key_down(egui::Key::Tab)));
        frame(
            &ctx,
            key(egui::Key::Tab, egui::Modifiers::NONE),
            &mut activated,
            &mut text,
        );
        assert_eq!(ctx.memory(|m| m.focused()), Some(ids[1]));
        frame(
            &ctx,
            key(egui::Key::Tab, egui::Modifiers::SHIFT),
            &mut activated,
            &mut text,
        );
        // egui applies backward traversal at the start of the following pass.
        frame(&ctx, vec![], &mut activated, &mut text);
        assert_eq!(ctx.memory(|m| m.focused()), Some(ids[0]));
        frame(
            &ctx,
            key(egui::Key::Space, egui::Modifiers::NONE),
            &mut activated,
            &mut text,
        );
        assert_eq!(activated, 1);
        for _ in 0..2 {
            frame(
                &ctx,
                key(egui::Key::Tab, egui::Modifiers::NONE),
                &mut activated,
                &mut text,
            );
        }
        assert_eq!(ctx.memory(|m| m.focused()), Some(ids[2]));
        frame(
            &ctx,
            vec![egui::Event::Text("typed in control".into())],
            &mut activated,
            &mut text,
        );
        assert_eq!(text, "typed in control");
        frame(
            &ctx,
            key(egui::Key::Escape, egui::Modifiers::NONE),
            &mut activated,
            &mut text,
        );
        assert!(ctx.memory(|m| m.focused()).is_none());
        let (toggled, _) = frame(
            &ctx,
            key(egui::Key::Tab, egui::Modifiers::NONE),
            &mut activated,
            &mut text,
        );
        assert!(toggled);
        assert!(ctx.memory(|m| m.focused()).is_none());
    }
    #[test]
    fn tab_toggles_panels_without_moving_keyboard_focus() {
        let ctx = egui::Context::default();
        let key = |key, pressed, repeat, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        };
        let mut input = egui::RawInput {
            events: vec![
                key(egui::Key::Tab, true, false, egui::Modifiers::NONE),
                key(egui::Key::Tab, true, true, egui::Modifiers::NONE),
                key(egui::Key::Tab, true, false, egui::Modifiers::SHIFT),
                key(egui::Key::Tab, false, false, egui::Modifiers::NONE),
                key(egui::Key::A, true, false, egui::Modifiers::NONE),
            ],
            ..Default::default()
        };
        assert!(take_panel_toggle(&ctx, &mut input));
        assert_eq!(input.events.len(), 1);
        assert!(!take_panel_toggle(&ctx, &mut input));
    }
    #[test]
    fn layer_symmetry_replaces_the_shared_symmetry_on_its_layer_only() {
        use crate::symmetry::Mode;
        let mut app = Dudel {
            doc: crate::document::Document::new(64, 64),
            ..Default::default()
        };
        app.doc.insert_layer_above().unwrap();
        app.symmetry.mode = Mode::LeftRight;
        let top = app.doc.active;
        let id = app.doc.layers[top].id;
        app.layer_symmetry.insert(id, app.symmetry);
        app.active_symmetry_mut().mode = Mode::Radial;
        app.active_symmetry_mut().angle = 30.0;
        assert_eq!(app.symmetry.mode, Mode::LeftRight);
        assert_eq!(app.symmetry_for(Some(top)).mode, Mode::Radial);
        assert_eq!(app.symmetry_for(Some(1 - top)).mode, Mode::LeftRight);
        assert_eq!(app.active_symmetry().angle, 30.0);
        app.doc.active = 1 - top;
        assert_eq!(app.active_symmetry().mode, Mode::LeftRight);
        assert!(app.guides_locked, "guides start locked");
    }
    #[test]
    fn slider_rows_keep_their_width_as_values_change() {
        let ctx = egui::Context::default();
        let width = |value: f32, suffix: &str| {
            let mut value = value;
            let mut width = 0.0;
            let _ = ctx.run(Default::default(), |ctx| {
                theme(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    width = ui
                        .add(
                            egui::Slider::new(&mut value, 0.0..=300.0)
                                .suffix(suffix)
                                .max_decimals(1),
                        )
                        .rect
                        .width();
                });
            });
            width
        };
        for suffix in [" px", "%", "°"] {
            let widths: Vec<f32> = [0.0, 9.0, 10.0, 99.5, 100.0, 300.0]
                .iter()
                .map(|value| width(*value, suffix))
                .collect();
            assert!(
                widths.windows(2).all(|w| w[0] == w[1]),
                "{suffix}: {widths:?}"
            );
        }
    }
    #[test]
    fn recent_brushes_and_favourites() {
        let mut app = Dudel::default();
        assert!(app.brushes.len() >= 6);
        for index in [1, 2, 1, 3, 4, 5, 0] {
            app.choose_brush(index);
        }
        let names: Vec<&str> = app.recent_brushes.iter().map(String::as_str).collect();
        let expected: Vec<&str> = [0, 5, 4, 3, 1]
            .iter()
            .map(|i| app.brushes[*i].name.as_str())
            .collect();
        assert_eq!(names, expected);
        assert_eq!(app.selected_brush, 0);
        // Stars persist with the brush and older brushes load unstarred.
        app.brushes[2].favorite = true;
        let json = serde_json::to_string(&app.brushes[2]).unwrap();
        assert!(serde_json::from_str::<Brush>(&json).unwrap().favorite);
        let legacy = r#"{"name":"Old","size":10.0,"opacity":1.0,"hardness":0.8,"spacing":0.1,"pressure_size":true,"pressure_opacity":false}"#;
        assert!(!serde_json::from_str::<Brush>(legacy).unwrap().favorite);
    }
}
