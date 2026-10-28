//! Left "Tool property" sidebar: the brush library and every brush setting
//! for Brush/Eraser, and the relevant options for the other tools.
use crate::{
    Dudel,
    brush::Tip,
    egui, icons, symmetry,
    tools::Tool,
    workspace::{SplitDrag, section, section_title, split_handle},
};

/// Stable pages keep the panel from growing as settings are opened.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BrushPage {
    #[default]
    Brush,
    Tip,
    Texture,
    More,
}

impl Dudel {
    /// Name and icon of the current tool, naming the paint group or lasso mode.
    pub(crate) fn tool_label(&self) -> (&'static str, icons::Icon) {
        match self.tool {
            Tool::Brush if self.pen_flipped() => ("Eraser", icons::Icon::Eraser),
            Tool::Brush => (self.brush_group.name(), self.brush_group.icon()),
            Tool::Lasso if self.lasso.erase => ("Lasso erase", icons::Icon::LassoErase),
            Tool::Lasso => ("Lasso fill", icons::Icon::LassoFill),
            tool => (tool.name(), tool.icon()),
        }
    }
    pub(crate) fn toggle_rope_stabilizer(&mut self) {
        use crate::brush::StabilizerMode;
        self.brush.stabilizer_mode = if self.brush.stabilizer_mode == StabilizerMode::Rope {
            StabilizerMode::Smooth
        } else {
            // A rope needs some length to do anything.
            if self.brush.stabilization < 1.0 {
                self.brush.stabilization = 10.0;
            }
            if self.brush.rope_distance.is_none() {
                self.brush.rope_distance = Some(self.brush.stabilization * 2.0);
            }
            StabilizerMode::Rope
        };
        self.status = match self.brush.stabilizer_mode {
            StabilizerMode::Rope => "Rope stabilizer: the line follows the pen on a string",
            StabilizerMode::Smooth => "Smooth stabilizer",
        }
        .into();
    }
    pub(crate) fn rope_distance_ui(&mut self, ui: &mut egui::Ui) {
        if self.brush.stabilizer_mode != crate::brush::StabilizerMode::Rope {
            return;
        }
        let mut distance = self
            .brush
            .rope_distance
            .unwrap_or(self.brush.stabilization * 2.0);
        let before = distance;
        let saved = self.saved_value(|b| b.rope_distance.unwrap_or(b.stabilization * 2.0));
        marked(ui, "Distance", &mut distance, saved, |ui, v| {
            ui.add(
                egui::Slider::new(v, 0.0..=200.0)
                    .clamping(egui::SliderClamping::Edits)
                    .suffix(" px")
                    .max_decimals(0),
            )
        })
        .on_hover_text("Rope distance on screen, independent of smoothing amount and canvas zoom");
        if distance != before {
            self.brush.rope_distance = Some(distance);
        }
    }
    pub(crate) fn stabilizer_mode_ui(&mut self, ui: &mut egui::Ui) {
        use crate::brush::StabilizerMode;
        ui.horizontal(|ui| {
            ui.label("Follow");
            ui.selectable_value(
                &mut self.brush.stabilizer_mode,
                StabilizerMode::Smooth,
                "Smooth",
            )
            .on_hover_text("Eases toward the pen and catches up when you pause");
            if ui
                .selectable_label(self.brush.stabilizer_mode == StabilizerMode::Rope, "Rope")
                .on_hover_text("Trails the pen on a string and holds still until pulled")
                .clicked()
                && self.brush.stabilizer_mode != StabilizerMode::Rope
            {
                self.toggle_rope_stabilizer();
            }
        });
    }
    /// Brush-shape / own-tip switch shared by the tool panel and quick bar.
    pub(crate) fn eraser_mode_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!self.eraser_own, "Brush shape")
                .on_hover_text("Erase with the current brush preset's shape and texture at the eraser's own size")
                .clicked()
            {
                self.set_eraser_own(false);
            }
            if ui
                .selectable_label(self.eraser_own, "Eraser tip")
                .on_hover_text("Erase with a separate round eraser that keeps its own size")
                .clicked()
            {
                self.set_eraser_own(true);
            }
        });
    }
    /// Settings for the separate eraser tip, which is in the brush slot now.
    pub(crate) fn eraser_tip_ui(&mut self, ui: &mut egui::Ui) {
        let brush = &mut self.brush;
        labeled(ui, "Size", |ui| {
            ui.add(
                egui::Slider::new(&mut brush.size, 1.0..=300.0)
                    .suffix(" px")
                    .logarithmic(true),
            )
        });
        let mut percent = (brush.opacity * 100.0).round();
        if labeled(ui, "Strength", |ui| {
            ui.add(
                egui::Slider::new(&mut percent, 0.0..=100.0)
                    .suffix("%")
                    .integer(),
            )
        })
        .changed()
        {
            brush.opacity = percent / 100.0;
        }
        let mut hardness = (brush.hardness * 100.0).round();
        if labeled(ui, "Hardness", |ui| {
            ui.add(
                egui::Slider::new(&mut hardness, 0.0..=100.0)
                    .suffix("%")
                    .integer(),
            )
        })
        .changed()
        {
            brush.hardness = hardness / 100.0;
        }
        ui.horizontal(|ui| {
            ui.label("Tip");
            ui.selectable_value(&mut brush.tip, crate::brush::Tip::Round, "Round");
            ui.selectable_value(&mut brush.tip, crate::brush::Tip::Square, "Square");
        });
        ui.horizontal(|ui| {
            ui.label("Pressure");
            ui.checkbox(&mut brush.pressure_size, "Size");
            ui.checkbox(&mut brush.pressure_opacity, "Strength");
        });
        if brush.tip == crate::brush::Tip::Image {
            brush.tip = crate::brush::Tip::Round;
        }
        if ui.small_button("Reset eraser").clicked() {
            *brush = crate::tools::default_eraser();
        }
        hint(
            ui,
            "The eraser keeps its own size and settings, separate from your brushes. A pen's eraser end uses it too.",
        );
    }
    /// Fill/Erase switch shared by the tool panel and the quick bar.
    pub(crate) fn lasso_mode_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.lasso.erase, false, "Fill")
                .on_hover_text("Fill the shape with the current color");
            ui.selectable_value(&mut self.lasso.erase, true, "Erase")
                .on_hover_text("Erase inside the shape · tap L to switch");
        });
    }
    /// Show the tool property panel with brush settings.
    pub(crate) fn show_brush_settings(&mut self) {
        self.tool_panel_override = Some(true);
        self.show_panels = true;
        if !matches!(self.tool, Tool::Brush | Tool::Eraser) {
            self.select_tool(Tool::Brush);
        }
    }

    pub(crate) fn tool_panel_ui(&mut self, ui: &mut egui::Ui) {
        // Fill the panel so its edge line and content agree with its width.
        ui.set_min_width(ui.available_width());
        // Labeled sliders keep their track narrow enough to fit the panel;
        // the panel width is pinned, so this cannot feed back into it.
        ui.spacing_mut().slider_width = (ui.available_width() - 250.0).clamp(40.0, 140.0);
        ui.add_space(6.0);
        // An open free transform owns the panel whatever tool is active.
        if self.affine_preview.is_some() {
            ui.horizontal(|ui| {
                icons::mark(ui, icons::Icon::Move, 18.0, ui.visuals().text_color());
                ui.label(section_title("FREE TRANSFORM"));
            });
            self.transform_panel_ui(ui);
            return;
        }
        ui.horizontal(|ui| {
            if self.tool != Tool::Brush {
                icons::mark(ui, self.tool.icon(), 18.0, ui.visuals().text_color());
            }
            if self.tool == Tool::Brush {
                icons::mark(ui, self.brush_group.icon(), 18.0, ui.visuals().text_color());
            }
            ui.label(section_title(match self.tool {
                Tool::Brush => match self.brush_group {
                    crate::brush::BrushGroup::Pencil => "PENCIL",
                    crate::brush::BrushGroup::Pen => "PEN",
                    crate::brush::BrushGroup::Brush => "BRUSH",
                    crate::brush::BrushGroup::Airbrush => "AIRBRUSH",
                },
                Tool::Eraser => "ERASER",
                Tool::Fill => "FILL",
                Tool::Picker => "COLOR PICKER",
                Tool::Select => "SELECTION",
                Tool::Lasso if self.lasso.erase => "LASSO ERASER",
                Tool::Lasso => "LASSO FILL",
                Tool::Move => "MOVE",
                Tool::Pointer => "CANVAS",
            }));
        });
        match self.tool {
            Tool::Eraser => {
                self.eraser_mode_ui(ui);
                if self.eraser_own {
                    self.eraser_tip_ui(ui);
                } else {
                    self.brush_panel_ui(ui);
                }
            }
            Tool::Brush => self.brush_panel_ui(ui),
            Tool::Fill => {
                ui.label("Reference");
                self.fill_source_ui(ui);
                ui.label("Tolerance");
                ui.add(egui::Slider::new(&mut self.fill_tolerance, 0..=255));
                self.fill_edge_options_ui(ui);
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
                hint(
                    ui,
                    if self.input_stroke_target().is_raster() {
                        "Paints the selected layer using the chosen reference to find boundaries. Visible layers use their composite; a specific layer uses its raw pixels, including when hidden. Higher tolerance includes more similar colors. A selection limits the fill."
                    } else {
                        "Fills mask coverage: black conceals and white reveals. Active mask uses grayscale boundaries; Visible layers uses the artwork composite. A specific layer uses its raw pixels. Opacity and selection coverage limit the change; transparency lock does not affect masks."
                    },
                );
            }
            Tool::Picker => hint(
                ui,
                if self.input_stroke_target().is_raster() {
                    "Click to sample the visible artwork. Hold Alt with any tool to pick temporarily."
                } else {
                    "Click to sample raw mask coverage, including disabled masks. Hold Alt with any tool to pick temporarily."
                },
            ),
            Tool::Lasso => {
                self.lasso_mode_ui(ui);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Shape");
                    for shape in crate::lasso_paint::Shape::ALL {
                        ui.selectable_value(&mut self.lasso.shape, shape, shape.name());
                    }
                });
                if self.lasso.shape != crate::lasso_paint::Shape::Freehand {
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.lasso.constrain, "Constrain proportions")
                            .on_hover_text("Squares and circles");
                        ui.checkbox(&mut self.lasso.from_center, "From center");
                    });
                }
                self.opacity_slider(ui);
                hint(
                    ui,
                    if self.input_stroke_target().is_raster() {
                        "Draw around an area and lift to apply. Uses uniform color or erasing, limited by the current selection and layer locks. Brush texture, flow and pressure do not affect the interior."
                    } else {
                        "Draw around an area and lift to edit mask coverage. Black conceals; white and lasso erasing reveal. Opacity and selection coverage limit the change. Raster transparency lock, brush texture, flow and pressure do not affect the interior."
                    },
                );
                if self.render_queue.has_background_edit()
                    && ui.button("Cancel background edit").clicked()
                {
                    self.render_queue.cancel_background_edit();
                }
            }
            Tool::Select => {
                egui::ScrollArea::vertical()
                    .id_salt("selection controls")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.selection_options_ui(ui);
                        hint(
                            ui,
                            "Drag a rectangle or lasso, place polygon vertices, or click a color with Magic wand. Add, Subtract and Intersect combine with the previous selection. Feather softens selection coverage for painting and editing.",
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Select all").clicked() {
                                self.select_all();
                            }
                            if ui
                                .add_enabled(
                                    self.doc.selection.is_some()
                                        || self.doc.selection_mask.is_some()
                                        || self.render_queue.busy(),
                                    egui::Button::new("Deselect"),
                                )
                                .clicked()
                            {
                                self.set_selection(None);
                            }
                            if ui.button("Clear pixels").clicked() {
                                self.clear_selection_pixels();
                            }
                        });
                    });
            }
            Tool::Move => {
                hint(
                    ui,
                    if self.input_stroke_target().is_raster() {
                        "Drag to move the selection, or the whole selected layer when nothing is selected. Each drag is one undo step."
                    } else {
                        "Drag selected coverage, or the whole mask when nothing is selected. Each drag is one undo step."
                    },
                );
                if ui
                    .button("Free transform…")
                    .on_hover_text("Scale, rotate and flip with handles · Ctrl+T from any tool")
                    .clicked()
                {
                    self.toggle_free_transform();
                }
            }
            Tool::Pointer => self.pointer_panel_ui(ui),
        }
    }

    /// Free transform: numeric fields, flips and Apply / Cancel.
    fn transform_panel_ui(&mut self, ui: &mut egui::Ui) {
        self.mirror_pair_ui(ui);
        ui.horizontal_wrapped(|ui| {
            for mode in crate::affine_preview::Mode::ALL {
                let enabled = mode != crate::affine_preview::Mode::ScaleRotate
                    || self.affine_settings.warp.is_none();
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(mode.name()).selected(self.transform_mode == mode),
                    )
                    .on_hover_text(mode.hint())
                    .on_disabled_hover_text("Reset to return to scale and rotate")
                    .clicked()
                {
                    self.set_transform_mode(mode);
                }
            }
        });
        if self.transform_mode == crate::affine_preview::Mode::Mesh {
            let [mut columns, mut rows] = self.mesh_grid;
            let max = crate::affine::MESH_MAX as u8;
            let mut changed = false;
            labeled(ui, "Columns", |ui| {
                changed |= ui.add(egui::Slider::new(&mut columns, 2..=max)).changed();
            });
            labeled(ui, "Rows", |ui| {
                changed |= ui.add(egui::Slider::new(&mut rows, 2..=max)).changed();
            });
            if changed {
                self.set_mesh_grid(columns, rows);
            }
        }
        if self.affine_settings.warp.is_some() {
            hint(ui, self.transform_mode.hint());
            self.transform_quality_ui(ui);
            self.transform_actions_ui(ui);
            return;
        }
        let settings = &mut self.affine_settings;
        for (axis, label) in [(0, "Width"), (1, "Height")] {
            labeled(ui, label, |ui| {
                ui.add(
                    egui::DragValue::new(&mut settings.scale_percent[axis])
                        .range(0.1..=10000.0)
                        .speed(0.5)
                        .suffix("%"),
                )
            });
        }
        labeled(ui, "Angle", |ui| {
            ui.add(
                egui::DragValue::new(&mut settings.angle_degrees)
                    .range(-180.0..=180.0)
                    .speed(0.5)
                    .suffix("°"),
            )
        });
        for (axis, label) in [(0, "Move X"), (1, "Move Y")] {
            labeled(ui, label, |ui| {
                ui.add(
                    egui::DragValue::new(&mut settings.translation[axis])
                        .range(-1000000.0..=1000000.0)
                        .suffix(" px"),
                )
            });
        }
        ui.horizontal_wrapped(|ui| {
            for (axis, label) in [(0, "Flip horizontal"), (1, "Flip vertical")] {
                if ui
                    .add(egui::Button::new(label).selected(settings.flip[axis]))
                    .clicked()
                {
                    settings.flip[axis] = !settings.flip[axis];
                }
            }
        });
        self.transform_quality_ui(ui);
        self.transform_actions_ui(ui);
        hint(
            ui,
            "Drag inside to move, a corner to scale, a side to stretch, or outside to rotate. Shift keeps proportions, snaps rotation to 15° and moves along one axis; Alt scales from the center. Moves snap to guides, the grid and the canvas edges. Arrow keys nudge by 1px (Shift 10px).",
        );
    }

    /// Special rulers: create, list and remove them.
    fn transform_quality_ui(&mut self, ui: &mut egui::Ui) {
        let settings = &mut self.affine_settings;
        labeled(ui, "Quality", |ui| {
            use crate::affine::Sampling;
            egui::ComboBox::from_id_salt("transform quality")
                .selected_text(match settings.sampling {
                    Sampling::Bilinear => "Smooth",
                    Sampling::Nearest => "Pixel",
                    Sampling::Bicubic => "Bicubic",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.sampling, Sampling::Bilinear, "Smooth");
                    ui.selectable_value(&mut settings.sampling, Sampling::Nearest, "Pixel");
                    ui.selectable_value(&mut settings.sampling, Sampling::Bicubic, "Bicubic");
                });
        });
    }
    fn transform_actions_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Apply").on_hover_text("Enter").clicked() {
                self.finish_affine_preview();
            }
            if ui.button("Cancel").on_hover_text("Esc").clicked() {
                self.cancel_affine_preview();
            }
            if ui.button("Reset").clicked() {
                self.affine_settings = crate::affine::Settings {
                    sampling: self.affine_settings.sampling,
                    mirror: self.affine_settings.mirror,
                    ..crate::affine::Settings::default()
                };
                self.transform_mode = crate::affine_preview::Mode::ScaleRotate;
            }
        });
    }
    fn rulers_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.snapping, "Snap strokes")
            .on_hover_text("Brush and eraser strokes follow rulers and nearby guide lines");
        ui.label(
            egui::RichText::new("Add a ruler, then drag on the canvas")
                .small()
                .weak(),
        );
        ui.horizontal_wrapped(|ui| {
            for kind in crate::rulers::Kind::ALL {
                let selected = self.ruler_create == Some(kind);
                if ui
                    .add(egui::Button::new(kind.name()).selected(selected))
                    .on_hover_text(kind.hint())
                    .clicked()
                {
                    self.ruler_create = (!selected).then_some(kind);
                }
            }
        });
        let layer = self
            .input_layer()
            .and_then(|index| self.doc.layers.get(index))
            .filter(|layer| !layer.folder)
            .map(|layer| layer.id);
        let mut remove = None;
        let mut owner_change = None;
        for index in 0..self.rulers.len() {
            let owner = self.ruler_owner(index);
            let orphaned = owner.is_some_and(|id| !self.doc.layers.iter().any(|l| l.id == id));
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} ruler", self.rulers[index].kind().name()));
                if orphaned {
                    ui.label(egui::RichText::new("(layer deleted)").weak());
                } else {
                    let mut own = owner.is_some();
                    let response = ui
                        .add_enabled(
                            owner.is_some() || layer.is_some(),
                            egui::Checkbox::new(&mut own, "This layer only"),
                        )
                        .on_hover_text(
                            "Show and snap only while drawing on the layer it belongs to",
                        );
                    if response.changed() {
                        owner_change = Some((index, if own { layer } else { None }));
                    }
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some((index, owner)) = owner_change {
            self.ruler_owners.resize(self.rulers.len(), None);
            self.ruler_owners[index] = owner;
        }
        if let Some(index) = remove {
            self.remove_ruler(index);
        }
        if self.rulers.len() > 1 && ui.button("Remove all rulers").clicked() {
            self.rulers.clear();
            self.ruler_owners.clear();
            self.ruler_drag = None;
        }
    }

    /// Pointer tool: canvas aids in one place, so they stay out of the way
    /// while drawing.
    fn pointer_panel_ui(&mut self, ui: &mut egui::Ui) {
        hint(
            ui,
            "Drag guide lines and the symmetry center, even when guides are locked. Drag empty canvas to pan. Drawing tools never move locked guides.",
        );
        egui::ScrollArea::vertical()
            .id_salt("canvas settings")
            .auto_shrink([false, false])
            .show(ui, |ui| self.canvas_settings_ui(ui));
    }
    fn canvas_settings_ui(&mut self, ui: &mut egui::Ui) {
        section("Rulers").show(ui, |ui| self.rulers_ui(ui));
        section("Grid and guides").show(ui, |ui| self.guides_ui(ui));
        section("Symmetry").show(ui, |ui| self.symmetry_ui(ui));
        section("View").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Fit canvas").on_hover_text("Ctrl+0").clicked() {
                    self.fit_view();
                }
                if ui.button("100%").on_hover_text("Ctrl+1").clicked() {
                    self.actual_size();
                }
                if ui.button("Reset rotation").on_hover_text("5").clicked() {
                    self.reset_rotation();
                }
                if ui
                    .add(egui::Button::new("Flip view").selected(self.mirror))
                    .on_hover_text("H · the artwork is not changed")
                    .clicked()
                {
                    self.toggle_flip();
                }
            });
        });
    }

    fn brush_panel_ui(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
        ui.spacing_mut().interact_size.y = 26.0;
        let total = ui.available_height();
        // Brush settings scroll, so the library may take all but 120 px of them.
        let minimum = 60.0_f32.min(total * 0.42);
        let library = self
            .library_height
            .unwrap_or((total * 0.28).min(190.0))
            .clamp(minimum, (total - 120.0).max(minimum));
        egui::TopBottomPanel::top("brush library area")
            .exact_height(library)
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| self.brush_library_ui(ui));
        match split_handle(ui) {
            SplitDrag::Reset => self.library_height = None,
            SplitDrag::By(delta) => self.library_height = Some(library + delta),
            SplitDrag::None => {}
        }
        self.brush_settings_ui(ui);
    }

    fn brush_settings_ui(&mut self, ui: &mut egui::Ui) {
        let modified = self.brush_preset_modified();
        // Right to left so the name field takes exactly the width the button
        // leaves; a guessed width overflows and feeds back into the panel width.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if ui
                    .small_button("Studio")
                    .on_hover_text("Open Brush Studio: full editor and test pad")
                    .clicked()
                {
                    self.brush_studio_section = match self.brush_panel_page {
                        BrushPage::Brush => crate::brush_studio::Section::SizeOpacity,
                        BrushPage::Tip => crate::brush_studio::Section::Tip,
                        BrushPage::Texture => crate::brush_studio::Section::Grain,
                        BrushPage::More => self.brush_panel_section,
                    };
                    self.brush_studio = true;
                }
                ui.menu_button("Preset", |ui| {
                    if ui
                        .add_enabled(modified, egui::Button::new("Update preset…"))
                        .on_hover_text("Save these settings over the selected library preset")
                        .clicked()
                    {
                        self.update_brush_preset();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.brushes.len() < 512,
                            egui::Button::new("Save as new preset"),
                        )
                        .clicked()
                    {
                        self.save_new_brush_preset();
                        ui.close();
                    }
                    if ui
                        .add_enabled(modified, egui::Button::new("Reset changes"))
                        .clicked()
                    {
                        self.revert_brush_preset();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Export brush…").clicked() {
                        self.brush_file(false);
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.brushes.len() > 1, egui::Button::new("Delete preset…"))
                        .clicked()
                    {
                        self.delete_brush_preset();
                        ui.close();
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.brush.name)
                        .desired_width(f32::INFINITY)
                        .char_limit(128)
                        .text_color(if modified {
                            ui.visuals().warn_fg_color
                        } else {
                            ui.visuals().text_color()
                        }),
                )
                .on_hover_text(if modified {
                    "Modified preset: use Preset > Update preset to keep changes"
                } else {
                    "Preset name"
                });
            },
        );
        self.brush_live_preview.show(
            ui,
            &self.brush,
            36.0,
            self.stroking || self.render_queue.busy() || self.performance.defer_optional(),
        );
        ui.horizontal(|ui| {
            for (page, label) in [
                (BrushPage::Brush, "Basic"),
                (BrushPage::Tip, "Tip"),
                (BrushPage::Texture, "Texture"),
                (BrushPage::More, "Advanced"),
            ] {
                ui.selectable_value(&mut self.brush_panel_page, page, label);
            }
        });
        if self.brush_panel_page == BrushPage::More {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("brush panel details")
                    .selected_text(self.brush_panel_section.title())
                    .width((ui.available_width() - 100.0).max(80.0))
                    .show_ui(ui, |ui| {
                        for section in crate::brush_studio::Section::STUDIO {
                            ui.selectable_value(
                                &mut self.brush_panel_section,
                                section,
                                section.title(),
                            );
                        }
                    });
                ui.menu_button("Symmetry", |ui| self.symmetry_ui(ui));
            });
        }
        egui::ScrollArea::vertical()
            .id_salt((
                "brush settings page",
                self.brush_panel_page as u8,
                self.brush_panel_section.title(),
            ))
            .auto_shrink([false, false])
            .show(ui, |ui| match self.brush_panel_page {
                BrushPage::Brush => {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.spacing_mut().interact_size.y = 24.0;
                    self.brush_quick_controls_ui(ui);
                }
                BrushPage::Tip => self.brush_section_ui(ui, crate::brush_studio::Section::Tip),
                BrushPage::Texture => {
                    self.brush_section_ui(ui, crate::brush_studio::Section::Grain)
                }
                BrushPage::More => self.brush_section_ui(ui, self.brush_panel_section),
            });
    }

    fn brush_flow_hardness_ui(&mut self, ui: &mut egui::Ui) {
        let saved = self.saved_value(|b| b.flow);
        percent_marked(ui, "Flow", &mut self.brush.flow, 0.0, saved);
        let saved = self.saved_value(|b| b.hardness);
        percent_marked(ui, "Hardness", &mut self.brush.hardness, 0.0, saved);
    }

    fn brush_size_opacity_ui(&mut self, ui: &mut egui::Ui) {
        let saved = self.saved_value(|b| b.size);
        marked(ui, "Size", &mut self.brush.size, saved, |ui, v| {
            ui.add(
                egui::Slider::new(v, 1.0..=300.0)
                    .logarithmic(true)
                    .suffix(" px")
                    .max_decimals(0),
            )
        });
        let saved = self.saved_value(|b| b.opacity);
        percent_marked(ui, "Opacity", &mut self.brush.opacity, 0.0, saved);
    }

    fn brush_quick_controls_ui(&mut self, ui: &mut egui::Ui) {
        self.brush_size_opacity_ui(ui);
        let saved = self.saved_value(|b| b.stabilization);
        marked(
            ui,
            "Stabilizer",
            &mut self.brush.stabilization,
            saved,
            |ui, v| ui.add(egui::Slider::new(v, 0.0..=50.0).max_decimals(0)),
        );
        self.rope_distance_ui(ui);
        self.stabilizer_mode_ui(ui);
        egui::CollapsingHeader::new("Paint & pressure")
            .id_salt("basic paint and pressure")
            .show(ui, |ui| {
                self.brush_flow_hardness_ui(ui);
                ui.horizontal(|ui| {
                    ui.label("Pressure");
                    ui.checkbox(&mut self.brush.pressure_size, "Size")
                        .on_hover_text("Pen pressure changes brush size");
                    ui.checkbox(&mut self.brush.pressure_opacity, "Opacity")
                        .on_hover_text("Pen pressure changes opacity");
                });
            });
    }

    /// A value of the selected library preset, to mark settings the current
    /// brush has changed from it.
    fn saved_value(&self, value: impl Fn(&crate::brush::Brush) -> f32) -> Option<f32> {
        self.brushes.get(self.selected_brush).map(value)
    }

    /// One group of brush settings; the side panel and the Brush Studio
    /// both draw sections through this.
    pub(crate) fn brush_section_ui(
        &mut self,
        ui: &mut egui::Ui,
        section: crate::brush_studio::Section,
    ) {
        use crate::brush_studio::Section;
        match section {
            Section::SizeOpacity => {
                self.brush_size_opacity_ui(ui);
                self.brush_flow_hardness_ui(ui);
                ui.horizontal(|ui| {
                    ui.label("Softness");
                    ui.selectable_value(
                        &mut self.brush.softness_profile,
                        crate::brush::SoftnessProfile::LegacyLinear,
                        "Classic",
                    );
                    ui.selectable_value(
                        &mut self.brush.softness_profile,
                        crate::brush::SoftnessProfile::Eased,
                        "Smooth",
                    );
                });
                let saved = self.saved_value(|b| b.spacing);
                percent_marked(ui, "Spacing", &mut self.brush.spacing, 0.02, saved);
                ui.checkbox(
                    &mut self.brush.adjust_flow_by_spacing,
                    "Keep flow consistent when spacing changes",
                );
                let saved = self.saved_value(|b| b.taper_in);
                marked(ui, "Taper in", &mut self.brush.taper_in, saved, |ui, v| {
                    ui.add(egui::Slider::new(v, 0.0..=300.0).suffix(" px"))
                });
                let saved = self.saved_value(|b| b.taper_out);
                marked(
                    ui,
                    "Taper out",
                    &mut self.brush.taper_out,
                    saved,
                    |ui, v| ui.add(egui::Slider::new(v, 0.0..=300.0).suffix(" px")),
                );
            }
            Section::Pressure => {
                ui.checkbox(&mut self.brush.pressure_size, "Pressure changes size");
                ui.checkbox(&mut self.brush.pressure_opacity, "Pressure changes opacity");
                crate::pressure_editor::ui(ui, &mut self.brush, self.pressure);
            }
            Section::Stabilizer => {
                self.stabilizer_mode_ui(ui);
                ui.checkbox(
                    &mut self.brush.stabilization_by_speed,
                    "Reduce stabilization on fast strokes",
                );
                let saved = self.saved_value(|b| b.stabilization);
                marked(
                    ui,
                    "Amount",
                    &mut self.brush.stabilization,
                    saved,
                    |ui, v| ui.add(egui::Slider::new(v, 0.0..=50.0).max_decimals(0)),
                );
                self.rope_distance_ui(ui);
                hint(
                    ui,
                    if self.brush.stabilizer_mode == crate::brush::StabilizerMode::Rope {
                        "Rope: Distance sets the string length on screen; Amount controls follow smoothing. It stays still until the pen pulls the string taut, then follows with a slight delay, and ends where it is when you lift."
                    } else {
                        "Smooths hand jitter. Higher values follow the pen more slowly; the line glides onto the pen when you pause or lift."
                    },
                );
            }
            Section::Dynamics => crate::dynamics::editor(ui, &mut self.brush.dynamics),
            Section::Tip => {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.brush.tip, Tip::Round, "Round");
                    ui.selectable_value(&mut self.brush.tip, Tip::Square, "Square");
                    if ui
                        .add_enabled(
                            self.brush.image.is_some(),
                            egui::Button::selectable(self.brush.tip == Tip::Image, "Image"),
                        )
                        .on_disabled_hover_text("Import a PNG tip first")
                        .clicked()
                    {
                        self.brush.tip = Tip::Image;
                    }
                });
                let saved = self.saved_value(|b| b.aspect);
                marked(ui, "Aspect", &mut self.brush.aspect, saved, |ui, v| {
                    ui.add(egui::Slider::new(v, 0.05..=1.0))
                });
                let saved = self.saved_value(|b| b.angle);
                marked(ui, "Angle", &mut self.brush.angle, saved, |ui, v| {
                    ui.add(egui::Slider::new(v, -180.0..=180.0).suffix("°"))
                });
                ui.horizontal_wrapped(|ui| {
                    use crate::brush::AngleSource;
                    ui.label("Turns with");
                    let source = &mut self.brush.angle_source;
                    ui.selectable_value(source, AngleSource::Fixed, "Nothing")
                        .on_hover_text("The tip keeps the angle above");
                    ui.selectable_value(source, AngleSource::Direction, "Stroke")
                        .on_hover_text("Follows the stroke direction, like a calligraphy pen");
                    ui.selectable_value(source, AngleSource::Tilt, "Pen tilt")
                        .on_hover_text(
                            "Points where the pen leans; follows the stroke when the tablet reports no tilt",
                        );
                });
                let saved = self.saved_value(|b| b.angle_jitter);
                marked(
                    ui,
                    "Jitter",
                    &mut self.brush.angle_jitter,
                    saved,
                    |ui, v| {
                        ui.add(
                            egui::Slider::new(v, 0.0..=180.0)
                                .suffix("°")
                                .max_decimals(0),
                        )
                    },
                )
                .on_hover_text("Turns each dab randomly by up to this much");
                if ui.button("Import PNG tip…").clicked() {
                    self.import_tip();
                }
                hint(
                    ui,
                    "Dark marks paint; white or transparent areas do not. Tips are saved inside the preset.",
                );
            }
            Section::Scatter => {
                let saved = self.saved_value(|b| b.scatter);
                marked(ui, "Scatter", &mut self.brush.scatter, saved, |ui, v| {
                    ui.add(egui::Slider::new(v, 0.0..=2.0))
                });
                labeled(ui, "Seed", |ui| {
                    ui.add(egui::DragValue::new(&mut self.brush.scatter_seed))
                });
                hint(
                    ui,
                    "Scatter is a multiple of the tip size. The seed picks a different repeatable pattern.",
                );
            }
            Section::Grain => {
                let saved = self.saved_value(|b| b.grain_amount);
                percent_marked(ui, "Amount", &mut self.brush.grain_amount, 0.0, saved);
                let saved = self.saved_value(|b| b.grain_scale);
                marked(ui, "Scale", &mut self.brush.grain_scale, saved, |ui, v| {
                    ui.add(egui::Slider::new(v, 0.25..=16.0).suffix(" px"))
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Import PNG grain…").clicked() {
                        self.import_grain();
                    }
                    if ui
                        .add_enabled(
                            self.brush.grain.is_some(),
                            egui::Button::new("Built-in grain"),
                        )
                        .clicked()
                    {
                        self.brush.grain = None;
                    }
                });
            }
            Section::Tags => {
                labeled(ui, "Name", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.brush.name)
                            .desired_width(f32::INFINITY)
                            .char_limit(128),
                    )
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.brush.tags)
                        .hint_text("ink, sketch, texture")
                        .char_limit(327),
                );
                labeled(ui, "Group", |ui| {
                    let current = self.brush.group();
                    egui::ComboBox::from_id_salt("brush group")
                        .selected_text(current.name())
                        .show_ui(ui, |ui| {
                            for group in crate::brush::BrushGroup::ALL {
                                if ui.selectable_label(current == group, group.name()).clicked() {
                                    self.brush.group = Some(group);
                                    self.brush_group = group;
                                }
                            }
                        })
                        .response
                        .on_hover_text("Which paint tool lists this preset. Update the preset to keep the change.")
                });
                hint(
                    ui,
                    "Up to 8 tags, separated by commas. Library search matches names and tags.",
                );
            }
            Section::TestPad => {
                let color = self.paint_color();
                self.scratchpad
                    .ui(ui, &self.brush, color, &self.frame_packets);
            }
        }
    }

    /// Symmetry for strokes on `layer`: the layer's own, else the global one.
    pub(crate) fn symmetry_for(&self, layer: Option<usize>) -> symmetry::Symmetry {
        layer
            .and_then(|index| self.doc.layers.get(index))
            .and_then(|layer| self.layer_symmetry.get(&layer.id))
            .copied()
            .unwrap_or(self.symmetry)
    }
    /// Symmetry for the layer the next stroke paints.
    pub(crate) fn active_symmetry(&self) -> symmetry::Symmetry {
        if let Some(index) = self.input_layer() {
            return self.symmetry_for(Some(index));
        }
        self.input_layer_id()
            .and_then(|id| self.layer_symmetry.get(&id))
            .copied()
            .unwrap_or(self.symmetry)
    }
    fn input_layer_id(&self) -> Option<crate::document::LayerId> {
        self.input_target()
    }
    /// The settings the symmetry controls edit: the active layer's own, or
    /// the global ones.
    pub(crate) fn active_symmetry_mut(&mut self) -> &mut symmetry::Symmetry {
        match self.input_layer_id() {
            Some(id) if self.layer_symmetry.contains_key(&id) => {
                self.layer_symmetry.get_mut(&id).unwrap()
            }
            _ => &mut self.symmetry,
        }
    }

    /// Direct layer controls with independent access to global symmetry.
    pub(crate) fn symmetry_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(index) = self.input_layer() {
            self.layer_guides_ui(ui, index);
        } else {
            self.global_symmetry_ui(ui);
        }
    }
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// Fixed-width label column so sliders line up.
fn labeled<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    labeled_marked(ui, label, false, add).0
}

/// A labeled row whose label is highlighted when `changed`; the highlighted
/// label is clickable and reports the click so the caller can reset.
fn labeled_marked<R>(
    ui: &mut egui::Ui,
    label: &str,
    changed: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 166.0).max(60.0);
        let sense = if changed {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(64.0, 20.0), sense);
        let color = if changed {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            color,
        );
        let reset = changed
            && response
                .on_hover_text("Changed from the saved preset. Click to put it back.")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
        (add(ui), reset)
    })
    .inner
}

/// A labeled value row marked when it differs from the saved preset value;
/// clicking the marked label restores that value.
fn marked(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    saved: Option<f32>,
    add: impl FnOnce(&mut egui::Ui, &mut f32) -> egui::Response,
) -> egui::Response {
    let changed = saved.is_some_and(|saved| (saved - *value).abs() > 1e-4);
    let (response, reset) = labeled_marked(ui, label, changed, |ui| add(ui, value));
    if reset && let Some(saved) = saved {
        *value = saved;
    }
    response
}

fn percent_marked(ui: &mut egui::Ui, label: &str, value: &mut f32, min: f32, saved: Option<f32>) {
    marked(ui, label, value, saved, |ui, value| {
        let mut percent = (*value * 100.0).round();
        let response = ui.add(
            egui::Slider::new(&mut percent, (min * 100.0).round()..=100.0)
                .suffix("%")
                .integer(),
        );
        if response.changed() {
            *value = (percent / 100.0).max(min);
        }
        response
    });
}
