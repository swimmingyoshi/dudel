//! Where the panels go in the Slate, Float and Atelier styles.
//!
//! Each layout arranges the same panel contents the Classic layout uses
//! (color, layers, brushes, tool settings, navigator, timeline), so every
//! feature is reachable in every style; only the chrome around them differs.
use crate::tools::GroupIcon;
use crate::{
    Dudel,
    brush::BrushGroup,
    commands::Action,
    egui,
    icons::{self, Icon},
    tools::Tool,
    ui::{
        style::{Active, Layout, Mode},
        widgets,
    },
    workspace::{SplitDrag, split_handle},
};
use egui::{Align, Align2, Margin, Pos2, RichText, Vec2, vec2};

/// A Float popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Popover {
    Brushes,
    Tool,
    Color,
    Layers,
    Navigator,
}

impl Popover {
    fn title(self) -> &'static str {
        match self {
            Popover::Brushes => "Brush library",
            Popover::Tool => "Tool settings",
            Popover::Color => "Color",
            Popover::Layers => "Layers",
            Popover::Navigator => "Navigator",
        }
    }
}

/// Panel choices of the styled layouts that last for the session.
#[derive(Default)]
pub(crate) struct LayoutState {
    /// Slate dock: the chosen tab of the top and middle groups.
    dock_tabs: [usize; 2],
    /// Slate dock: groups folded down to their tabs.
    dock_folded: [bool; 3],
    /// Slate dock: heights the artist dragged for the top and middle groups.
    dock_heights: [Option<f32>; 2],
    /// Float: the open popover.
    pub(crate) popover: Option<Popover>,
    /// Atelier: the chosen tab of the tray above the layers.
    tray_tab: usize,
    tray_height: Option<f32>,
}

/// A tool rail entry: a brush group or a tool.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Group(BrushGroup),
    Tool(Tool),
}

/// The rail in its groups: paint, erase, fill and sample, select and move,
/// text and pointer.
fn rail_sections() -> Vec<Vec<Item>> {
    vec![
        BrushGroup::ALL.into_iter().map(Item::Group).collect(),
        vec![Item::Tool(Tool::Eraser)],
        vec![Item::Tool(Tool::Fill), Item::Tool(Tool::Picker)],
        vec![
            Item::Tool(Tool::Select),
            Item::Tool(Tool::Lasso),
            Item::Tool(Tool::Move),
        ],
        vec![Item::Tool(Tool::Text), Item::Tool(Tool::Pointer)],
    ]
}

/// The four tabs a Slate or Atelier panel can show above the layers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Color,
    Navigator,
    Brushes,
    Tool,
}

fn atelier_tabs(mode: Mode) -> [Tab; 2] {
    match mode {
        Mode::Draw => [Tab::Tool, Tab::Navigator],
        Mode::Paint => [Tab::Color, Tab::Tool],
        Mode::Animate => [Tab::Color, Tab::Navigator],
    }
}

impl Dudel {
    /// Controls show as scrub fields instead of sliders.
    pub(crate) fn scrub_fields(&self) -> bool {
        self.interface.layout != Layout::Classic
    }

    /// Panels of the styled layouts. The Classic layout keeps its own code.
    pub(crate) fn styled_panels(&mut self, ctx: &egui::Context) {
        match self.interface.layout {
            Layout::Classic => {}
            Layout::Slate => self.slate(ctx),
            Layout::Float => self.float_bars(ctx),
            Layout::Atelier => self.atelier(ctx),
        }
    }

    /// Whether the current tool's settings are on screen in a styled layout.
    pub(crate) fn styled_tool_settings_open(&self) -> bool {
        match self.interface.layout {
            Layout::Slate => self.ui_layout.dock_tabs[1] == 1 && !self.ui_layout.dock_folded[1],
            Layout::Float => self.ui_layout.popover == Some(Popover::Tool),
            Layout::Atelier => atelier_tabs(self.interface.mode)
                .get(self.ui_layout.tray_tab)
                .is_some_and(|tab| *tab == Tab::Tool),
            Layout::Classic => false,
        }
    }

    /// The tool button in the options bar: show or hide the tool settings.
    pub(crate) fn toggle_styled_tool_settings(&mut self) {
        let open = self.styled_tool_settings_open();
        self.show_panels = true;
        match self.interface.layout {
            Layout::Slate => {
                if open {
                    self.ui_layout.dock_tabs[1] = 0;
                } else {
                    self.ui_layout.dock_tabs[1] = 1;
                    self.ui_layout.dock_folded[1] = false;
                }
            }
            Layout::Float => {
                self.ui_layout.popover = (!open).then_some(Popover::Tool);
            }
            Layout::Atelier => {
                let tabs = atelier_tabs(self.interface.mode);
                match tabs.iter().position(|t| *t == Tab::Tool) {
                    Some(index) if !open => self.ui_layout.tray_tab = index,
                    Some(_) => self.ui_layout.tray_tab = 1 - self.ui_layout.tray_tab.min(1),
                    None => {
                        self.interface.mode = Mode::Draw;
                        self.ui_layout.tray_tab = 0;
                    }
                }
            }
            Layout::Classic => {}
        }
    }

    // ------------------------------------------------------------ rail --

    fn item_state(&self, item: Item) -> (bool, Icon, &'static str, &'static str, bool) {
        let masked = !self.input_stroke_target().is_raster();
        match item {
            Item::Group(group) => (
                self.tool == Tool::Brush && self.brush_group == group,
                group.icon(),
                group.name(),
                group.hint(),
                true,
            ),
            Item::Tool(tool) => {
                let icon = if tool == Tool::Lasso && self.lasso.erase {
                    Icon::LassoErase
                } else if tool == Tool::Fill && self.gradient.active {
                    Icon::Gradient
                } else {
                    tool.icon()
                };
                (
                    self.tool == tool,
                    icon,
                    tool.name(),
                    tool.target_hint(masked),
                    matches!(tool, Tool::Eraser | Tool::Fill | Tool::Lasso | Tool::Select),
                )
            }
        }
    }

    /// One rail button. A click chooses it; a right-click or a long press
    /// opens its variants: presets for a brush group, modes for a tool.
    fn rail_button(&mut self, ui: &mut egui::Ui, item: Item, side: f32) {
        let (active, icon, name, hint, variants) = self.item_state(item);
        let action = match item {
            Item::Group(_) => Action::Tool(Tool::Brush),
            Item::Tool(tool) => Action::Tool(tool),
        };
        let response = icons::button(ui, icon, side, active, true).on_hover_text(format!(
            "{name} · {}\n{hint}\nRight-click or press and hold for {}",
            self.shortcuts.label(action),
            if matches!(item, Item::Group(_)) {
                "its presets"
            } else {
                "its options"
            }
        ));
        if variants {
            // A corner mark: this tool has variants.
            let r = response.rect.shrink(side * 0.1);
            let color = ui.visuals().weak_text_color();
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    r.right_bottom() - vec2(0.0, side * 0.16),
                    r.right_bottom(),
                    r.right_bottom() - vec2(side * 0.16, 0.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        if response.clicked() {
            match item {
                Item::Group(group) => self.select_brush_group(group),
                Item::Tool(tool) => self.select_tool(tool),
            }
        }
        response.context_menu(|ui| self.variants_ui(ui, item));
    }

    fn variants_ui(&mut self, ui: &mut egui::Ui, item: Item) {
        ui.set_width(290.0);
        let room = (ui.ctx().content_rect().height() - 80.0).clamp(160.0, 460.0);
        match item {
            Item::Group(group) => {
                if self.tool != Tool::Brush || self.brush_group != group {
                    self.select_brush_group(group);
                }
                ui.strong(group.name());
                ui.allocate_ui(vec2(ui.available_width(), room), |ui| {
                    if ui
                        .push_id("rail presets", |ui| self.brush_library_ui(ui))
                        .inner
                    {
                        ui.close();
                    }
                });
            }
            Item::Tool(Tool::Fill) => {
                for (gradient, label) in [(false, "Fill an area"), (true, "Gradient")] {
                    if ui
                        .selectable_label(
                            self.tool == Tool::Fill && self.gradient.active == gradient,
                            label,
                        )
                        .clicked()
                    {
                        self.select_tool(Tool::Fill);
                        self.gradient.active = gradient;
                        ui.close();
                    }
                }
            }
            Item::Tool(Tool::Lasso) => {
                for (erase, label) in [(false, "Lasso fill"), (true, "Lasso erase")] {
                    if ui
                        .selectable_label(
                            self.tool == Tool::Lasso && self.lasso.erase == erase,
                            label,
                        )
                        .clicked()
                    {
                        self.select_tool(Tool::Lasso);
                        self.lasso.erase = erase;
                        ui.close();
                    }
                }
            }
            Item::Tool(Tool::Eraser) => {
                if self.tool != Tool::Eraser {
                    self.select_tool(Tool::Eraser);
                }
                self.eraser_mode_ui(ui);
            }
            Item::Tool(tool) => {
                if self.tool != tool {
                    self.select_tool(tool);
                }
                ui.strong(tool.name());
                egui::ScrollArea::vertical()
                    .id_salt("rail tool options")
                    .max_height(room)
                    .show(ui, |ui| self.tool_panel_ui(ui));
            }
        }
    }

    fn undo_redo_buttons(&mut self, ui: &mut egui::Ui, side: f32) {
        let busy = self.render_queue.busy();
        if icons::button(ui, Icon::Undo, side, false, self.history.can_undo() || busy)
            .on_hover_text(self.command_hint(Action::Undo, "Undo"))
            .clicked()
        {
            self.undo();
        }
        if icons::button(ui, Icon::Redo, side, false, self.history.can_redo() || busy)
            .on_hover_text(self.command_hint(Action::Redo, "Redo"))
            .clicked()
        {
            self.redo();
        }
    }

    /// The vertical rail in groups, with undo and redo at the bottom.
    fn grouped_rail(&mut self, ui: &mut egui::Ui) {
        let side = ((ui.available_height() - 100.0) / 17.0).clamp(26.0, 36.0);
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("styled rail")
            .max_height((ui.available_height() - 2.0 * (side + 4.0) - 12.0).max(side))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (index, section) in rail_sections().into_iter().enumerate() {
                        if index > 0 {
                            ui.add(egui::Separator::default().spacing(8.0).shrink(10.0));
                        }
                        for item in section {
                            self.rail_button(ui, item, side);
                        }
                    }
                    ui.add(egui::Separator::default().spacing(8.0).shrink(10.0));
                    self.extension_tools_ui(ui, side);
                    self.paint_color_chips(ui, side);
                });
            });
        ui.with_layout(egui::Layout::bottom_up(Align::Center), |ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            // Bottom-up: redo is placed first so undo sits above it.
            let busy = self.render_queue.busy();
            if icons::button(ui, Icon::Redo, side, false, self.history.can_redo() || busy)
                .on_hover_text(self.command_hint(Action::Redo, "Redo"))
                .clicked()
            {
                self.redo();
            }
            if icons::button(ui, Icon::Undo, side, false, self.history.can_undo() || busy)
                .on_hover_text(self.command_hint(Action::Undo, "Undo"))
                .clicked()
            {
                self.undo();
            }
        });
    }

    fn command_search_button(&mut self, ui: &mut egui::Ui, width: f32) {
        let label = format!("Search commands   {}", self.shortcuts.label(Action::Search));
        let button = egui::Button::new(RichText::new(label).weak())
            .fill(ui.visuals().extreme_bg_color)
            .min_size(vec2(width, 0.0));
        if ui
            .add(button)
            .on_hover_text("Find any command by name")
            .clicked()
        {
            self.open_command_search();
        }
    }

    fn color_tab_name(&self) -> &'static str {
        if self.input_stroke_target().is_raster() {
            "Color"
        } else {
            "Mask"
        }
    }

    fn tab_name(&self, tab: Tab) -> &'static str {
        match tab {
            Tab::Color => self.color_tab_name(),
            Tab::Navigator => "Navigator",
            Tab::Brushes => "Brushes",
            Tab::Tool => "Settings",
        }
    }

    fn tab_contents(&mut self, ui: &mut egui::Ui, tab: Tab, salt: &str) {
        let room = ui.available_height();
        match tab {
            Tab::Color if room < 300.0 => {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.min(26.0);
                self.compact_palette_ui(ui);
            }
            Tab::Color => {
                egui::ScrollArea::vertical()
                    .id_salt((salt, "color"))
                    .show(ui, |ui| self.palette_ui(ui));
            }
            Tab::Navigator => {
                egui::ScrollArea::vertical()
                    .id_salt((salt, "navigator"))
                    .show(ui, |ui| self.navigator_ui(ui, room));
            }
            Tab::Brushes => {
                self.brush_library_ui(ui);
            }
            Tab::Tool => {
                egui::ScrollArea::vertical()
                    .id_salt((salt, "tool"))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.tool_panel_ui(ui));
            }
        }
    }

    fn layers_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            widgets::tab_strip(ui, &["Layers"], 0);
            self.layer_guide_controls(ui);
        });
        self.layers_ui(ui, true, true);
    }

    fn timeline_panel(&mut self, ctx: &egui::Context, offer: bool) {
        if !self.show_panels {
            return;
        }
        if self.doc.animation.is_some() {
            egui::TopBottomPanel::bottom("timeline")
                .resizable(false)
                .show(ctx, |ui| self.timeline_ui(ui));
        } else if offer {
            egui::TopBottomPanel::bottom("timeline").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icons::mark(ui, Icon::Onion, 22.0, ui.visuals().weak_text_color());
                    ui.label("This canvas has no frames yet.");
                    if ui.button("Turn on the animation timeline").clicked() {
                        self.execute_command(Action::AnimationTimeline);
                    }
                });
            });
        }
    }

    // ----------------------------------------------------------- Slate --

    fn slate(&mut self, ctx: &egui::Context) {
        let active = Active::get(ctx);
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(active.base)
                    .inner_margin(Margin::symmetric(8, 2)),
            )
            .show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.label(
                        RichText::new("DUDEL")
                            .monospace()
                            .strong()
                            .color(active.accent),
                    );
                    ui.add_space(6.0);
                    self.menus(ui);
                    ui.add_space(12.0);
                    self.window_title_ui(ui);
                });
            });
        egui::TopBottomPanel::top("slate context")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(Margin::symmetric(8, 4)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let search = if ui.available_width() > 1100.0 {
                        230.0
                    } else {
                        0.0
                    };
                    let height = ui.spacing().interact_size.y;
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width() - search, height),
                        egui::Layout::left_to_right(Align::Center),
                        |ui| self.options_bar(ui),
                    );
                    if search > 0.0 {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            self.command_search_button(ui, search - 12.0);
                        });
                    }
                });
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        self.timeline_panel(ctx, false);
        if !self.show_panels {
            return;
        }
        egui::SidePanel::left("tools")
            .exact_width(50.0)
            .resizable(false)
            .show(ctx, |ui| self.grouped_rail(ui));
        egui::SidePanel::right("slate dock")
            .default_width(300.0)
            .width_range(250.0..=520.0)
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(Margin::symmetric(6, 4)))
            .show(ctx, |ui| self.slate_dock(ui));
    }

    /// Three tabbed groups: color or navigator, brushes or tool settings, and
    /// layers. A click on the chosen tab folds its group.
    fn slate_dock(&mut self, ui: &mut egui::Ui) {
        if self.affine_preview.is_some() {
            self.ui_layout.dock_tabs[1] = 1;
            self.ui_layout.dock_folded[1] = false;
        }
        let total = ui.available_height();
        let folded_height = 30.0;
        let groups: [[Tab; 2]; 2] = [[Tab::Color, Tab::Navigator], [Tab::Brushes, Tab::Tool]];
        let defaults = [total * 0.34, total * 0.27];
        for (index, tabs) in groups.into_iter().enumerate() {
            let folded = self.ui_layout.dock_folded[index];
            let height = if folded {
                folded_height
            } else {
                self.ui_layout.dock_heights[index]
                    .unwrap_or(defaults[index])
                    .clamp(110.0, (total - 230.0).max(110.0))
            };
            let id = ["dock group one", "dock group two"][index];
            egui::TopBottomPanel::top(id)
                .exact_height(height)
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    if index > 0 {
                        self.dock_split(ui, index - 1, total);
                    }
                    let names = tabs.map(|t| self.tab_name(t));
                    let chosen = self.ui_layout.dock_tabs[index].min(1);
                    ui.horizontal(|ui| {
                        if let Some(clicked) = widgets::tab_strip(ui, &names, chosen) {
                            if clicked == chosen {
                                self.ui_layout.dock_folded[index] = !folded;
                            } else {
                                self.ui_layout.dock_tabs[index] = clicked;
                                self.ui_layout.dock_folded[index] = false;
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            let icon = if folded {
                                Icon::ChevronDown
                            } else {
                                Icon::ChevronUp
                            };
                            if icons::button(ui, icon, 22.0, false, true)
                                .on_hover_text(if folded { "Unfold" } else { "Fold" })
                                .clicked()
                            {
                                self.ui_layout.dock_folded[index] = !folded;
                            }
                        });
                    });
                    if !folded {
                        let tab = tabs[self.ui_layout.dock_tabs[index].min(1)];
                        self.tab_contents(ui, tab, id);
                    }
                });
        }
        self.dock_split(ui, 1, total);
        self.layers_section(ui);
    }

    fn dock_split(&mut self, ui: &mut egui::Ui, above: usize, total: f32) {
        if self.ui_layout.dock_folded[above] {
            ui.add_space(4.0);
            return;
        }
        match split_handle(ui) {
            SplitDrag::Reset => self.ui_layout.dock_heights[above] = None,
            SplitDrag::By(delta) => {
                let defaults = [total * 0.34, total * 0.27];
                let current = self.ui_layout.dock_heights[above].unwrap_or(defaults[above]);
                self.ui_layout.dock_heights[above] = Some(current + delta);
            }
            SplitDrag::None => {}
        }
    }

    // ----------------------------------------------------------- Float --

    /// Float's only bars: a slim title bar and the timeline.
    fn float_bars(&mut self, ctx: &egui::Context) {
        let active = Active::get(ctx);
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(active.base)
                    .inner_margin(Margin::symmetric(10, 2)),
            )
            .show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 28.0),
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.label(RichText::new("Dudel").strong().color(active.accent));
                        self.window_title_ui(ui);
                    },
                );
            });
        self.timeline_panel(ctx, false);
    }

    fn island(
        ctx: &egui::Context,
        id: &str,
        pos: Pos2,
        pivot: Align2,
        calm: bool,
        add: impl FnOnce(&mut egui::Ui),
    ) -> egui::Rect {
        let response = egui::Area::new(egui::Id::new(("float island", id)))
            .fixed_pos(pos)
            .pivot(pivot)
            .constrain(true)
            // While a stroke is drawn the islands fade and let the pen
            // through, so a stroke can pass under them.
            .interactable(!calm)
            .show(ctx, |ui| {
                if calm {
                    ui.multiply_opacity(0.18);
                }
                let fill = ui.visuals().window_fill;
                egui::Frame::new()
                    .fill(fill.gamma_multiply(0.96))
                    .stroke(ui.visuals().window_stroke)
                    .corner_radius(egui::CornerRadius::same(
                        (ui.spacing().interact_size.y / 2.0 + 6.0).min(30.0) as u8,
                    ))
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(Margin::same(6))
                    .show(ui, add);
            });
        response.response.rect
    }

    /// Float's islands, drawn over the canvas after it is laid out.
    pub(crate) fn float_overlays(&mut self, ctx: &egui::Context) {
        if self.interface.layout != Layout::Float || self.studio_tab_shown() {
            return;
        }
        if self.stroking {
            self.ui_layout.popover = None;
        }
        let area = if self.canvas_area.is_positive() {
            self.canvas_area
        } else {
            ctx.content_rect()
        };
        let side = ctx.style().spacing.interact_size.y.clamp(30.0, 46.0);
        let margin = 12.0;
        let calm = self.stroking;
        // Left island: menus, search, view and the selection tools.
        Self::island(
            ctx,
            "left",
            area.left_top() + vec2(margin, margin),
            Align2::LEFT_TOP,
            calm,
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let menu =
                        icons::button(ui, Icon::Menu, side, false, true).on_hover_text("Menus");
                    egui::Popup::menu(&menu).show(|ui| self.menus(ui));
                    if icons::button(ui, Icon::Search, side, false, true)
                        .on_hover_text(self.command_hint(Action::Search, "Search commands"))
                        .clicked()
                    {
                        self.open_command_search();
                    }
                    if icons::button(ui, Icon::Fit, side, false, true)
                        .on_hover_text(self.command_hint(Action::Fit, "Fit the canvas"))
                        .clicked()
                    {
                        self.execute_command(Action::Fit);
                    }
                    if self.doc.animation.is_some()
                        && icons::button(ui, Icon::Play, side, false, true)
                            .on_hover_text(self.command_hint(Action::PlayAnimation, "Play or stop"))
                            .clicked()
                    {
                        self.execute_command(Action::PlayAnimation);
                    }
                    ui.separator();
                    for item in [
                        Tool::Select,
                        Tool::Lasso,
                        Tool::Move,
                        Tool::Text,
                        Tool::Pointer,
                    ] {
                        self.rail_button(ui, Item::Tool(item), side);
                    }
                });
            },
        );
        if !self.show_panels {
            return;
        }
        // Right island: painting tools and the popovers.
        let right = Self::island(
            ctx,
            "right",
            area.right_top() + vec2(-margin, margin),
            Align2::RIGHT_TOP,
            calm,
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for group in BrushGroup::ALL {
                        self.rail_button(ui, Item::Group(group), side);
                    }
                    for tool in [Tool::Eraser, Tool::Fill, Tool::Picker] {
                        self.rail_button(ui, Item::Tool(tool), side);
                    }
                    ui.separator();
                    for (popover, icon, hint) in [
                        (Popover::Brushes, Icon::Grid, "Brush library"),
                        (Popover::Tool, Icon::Sliders, "Tool settings"),
                        (Popover::Layers, Icon::Layers, "Layers"),
                        (Popover::Navigator, Icon::ZoomIn, "Navigator"),
                    ] {
                        let open = self.ui_layout.popover == Some(popover);
                        if icons::button(ui, icon, side, open, true)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            self.ui_layout.popover = (!open).then_some(popover);
                        }
                    }
                    // The color dot opens the color popover.
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(side), egui::Sense::click());
                    let open = self.ui_layout.popover == Some(Popover::Color);
                    let ring = if open {
                        Active::get(ui.ctx()).accent
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.painter().circle(
                        rect.center(),
                        side * 0.32,
                        crate::color_wheel::color32(self.paint_color()),
                        egui::Stroke::new(2.5_f32, ring),
                    );
                    if response.on_hover_text(self.color_tab_name()).clicked() {
                        self.ui_layout.popover = (!open).then_some(Popover::Color);
                    }
                });
            },
        );
        // The popover hangs under the right island.
        if let Some(popover) = self.ui_layout.popover {
            let max_height = (area.bottom() - right.bottom() - 90.0).max(200.0);
            let mut keep = true;
            egui::Area::new(egui::Id::new("float popover"))
                .fixed_pos(egui::pos2(right.right(), right.bottom() + 8.0))
                .pivot(Align2::RIGHT_TOP)
                .constrain(true)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(330.0);
                            ui.set_max_height(max_height);
                            ui.horizontal(|ui| {
                                ui.strong(popover.title());
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Close").clicked() {
                                        keep = false;
                                    }
                                });
                            });
                            ui.separator();
                            match popover {
                                Popover::Brushes => {
                                    ui.horizontal_wrapped(|ui| {
                                        for group in BrushGroup::ALL {
                                            let chosen = self.tool == Tool::Brush
                                                && self.brush_group == group;
                                            if ui.selectable_label(chosen, group.name()).clicked() {
                                                self.select_brush_group(group);
                                            }
                                        }
                                    });
                                    ui.allocate_ui(vec2(330.0, max_height - 60.0), |ui| {
                                        self.brush_library_ui(ui);
                                    });
                                }
                                Popover::Tool => {
                                    egui::ScrollArea::vertical()
                                        .id_salt("float tool")
                                        .max_height(max_height - 40.0)
                                        .show(ui, |ui| self.tool_panel_ui(ui));
                                }
                                Popover::Color => {
                                    egui::ScrollArea::vertical()
                                        .id_salt("float color")
                                        .max_height(max_height - 40.0)
                                        .show(ui, |ui| self.palette_ui(ui));
                                }
                                Popover::Layers => {
                                    ui.allocate_ui(vec2(330.0, max_height - 40.0), |ui| {
                                        self.layers_section(ui);
                                    });
                                }
                                Popover::Navigator => {
                                    self.navigator_ui(ui, (max_height - 40.0).min(320.0));
                                }
                            }
                        });
                });
            if !keep {
                self.ui_layout.popover = None;
            }
        }
        // The side rail: brush size and opacity, undo and redo.
        let rail_x = if self.interface.rails_right {
            area.right() - margin
        } else {
            area.left() + margin
        };
        let rail_pivot = if self.interface.rails_right {
            Align2::RIGHT_CENTER
        } else {
            Align2::LEFT_CENTER
        };
        if matches!(
            self.tool,
            Tool::Brush | Tool::Eraser | Tool::Lasso | Tool::Fill
        ) {
            Self::island(
                ctx,
                "rail",
                egui::pos2(rail_x, area.center().y),
                rail_pivot,
                calm,
                |ui| {
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.spacing_mut().slider_width = (area.height() * 0.16).clamp(60.0, 160.0);
                        ui.label(RichText::new("SIZE").small().weak());
                        ui.add(
                            egui::Slider::new(&mut self.brush.size, 1.0..=300.0)
                                .vertical()
                                .logarithmic(true)
                                .show_value(false),
                        )
                        .on_hover_text(format!("{:.0} px · [ and ] change size", self.brush.size));
                        ui.label(RichText::new(format!("{:.0}", self.brush.size)).strong());
                        ui.add_space(4.0);
                        let mut percent = (self.brush.opacity * 100.0).round();
                        if ui
                            .add(
                                egui::Slider::new(&mut percent, 0.0..=100.0)
                                    .vertical()
                                    .show_value(false),
                            )
                            .on_hover_text(format!("Opacity {percent:.0}%"))
                            .changed()
                        {
                            self.brush.opacity = percent / 100.0;
                        }
                        ui.label(RichText::new(format!("{percent:.0}%")).strong());
                        ui.label(RichText::new("OPACITY").small().weak());
                        ui.add_space(4.0);
                        self.undo_redo_buttons(ui, side * 0.9);
                    });
                },
            );
        }
        // The context strip: the current tool's options.
        Self::island(
            ctx,
            "context",
            egui::pos2(area.center().x, area.bottom() - margin),
            Align2::CENTER_BOTTOM,
            calm,
            |ui| {
                ui.set_max_width((area.width() - 80.0).max(300.0));
                ui.horizontal(|ui| self.options_bar(ui));
            },
        );
        // The status line, bottom left, only while there is news.
        if !self.status.is_empty() {
            egui::Area::new(egui::Id::new("float status"))
                .fixed_pos(area.left_bottom() + vec2(margin, -margin))
                .pivot(Align2::LEFT_BOTTOM)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_max_width((area.width() * 0.22).clamp(160.0, 320.0));
                    egui::Frame::new()
                        .fill(ui.visuals().window_fill.gamma_multiply(0.85))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&self.status).small()).truncate(),
                            );
                        });
                });
        }
    }

    // --------------------------------------------------------- Atelier --

    fn atelier(&mut self, ctx: &egui::Context) {
        let active = Active::get(ctx);
        egui::TopBottomPanel::top("menu")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(Margin::symmetric(12, 4)))
            .show(ctx, |ui| {
                // One menu bar row holds everything: a menu bar placed beside
                // other widgets takes the rest of the row for itself.
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.label(
                        RichText::new("dudel")
                            .size(21.0)
                            .strong()
                            .color(active.accent),
                    );
                    self.menus(ui);
                    ui.add_space(8.0);
                    let mut mode = self.interface.mode;
                    let options: Vec<(Mode, &str)> =
                        Mode::ALL.iter().map(|m| (*m, m.name())).collect();
                    if widgets::segmented(ui, &mut mode, &options) {
                        self.interface.mode = mode;
                        self.ui_layout.tray_tab = 0;
                    }
                    ui.add_space(8.0);
                    self.undo_redo_buttons(ui, 28.0);
                    let export = egui::Button::new(
                        RichText::new("Export").strong().color(active.accent_text),
                    )
                    .fill(active.accent);
                    if ui
                        .add(export)
                        .on_hover_text(self.command_hint(Action::ExportPng(false), "Export image"))
                        .clicked()
                    {
                        self.execute_command(Action::ExportPng(false));
                    }
                    ui.add_space(8.0);
                    self.window_title_ui(ui);
                });
            });
        egui::TopBottomPanel::top("atelier options")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(Margin::symmetric(10, 3)))
            .show(ctx, |ui| self.options_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        self.timeline_panel(ctx, self.interface.mode == Mode::Animate);
        if !self.show_panels {
            return;
        }
        egui::SidePanel::left("atelier drawer")
            .default_width(250.0)
            .width_range(220.0..=420.0)
            .show(ctx, |ui| self.brush_drawer(ui));
        egui::SidePanel::right("atelier tray")
            .default_width(290.0)
            .width_range(240.0..=520.0)
            .show(ctx, |ui| self.atelier_tray(ui));
    }

    /// Brushes first, grouped by medium, with the other tools in a tray below.
    fn brush_drawer(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Brushes").strong().size(16.0));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&self.brush.name).small().weak()).truncate());
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for group in BrushGroup::ALL {
                let chosen = self.tool == Tool::Brush && self.brush_group == group;
                if ui
                    .selectable_label(chosen, group.name())
                    .on_hover_text(group.hint())
                    .clicked()
                {
                    self.select_brush_group(group);
                }
            }
        });
        let side = 36.0;
        let tray = 2.0 * (side + 4.0) + 24.0;
        let room = (ui.available_height() - tray).max(120.0);
        ui.allocate_ui(vec2(ui.available_width(), room), |ui| {
            self.brush_library_ui(ui);
        });
        ui.separator();
        let tools = [
            Tool::Eraser,
            Tool::Fill,
            Tool::Picker,
            Tool::Select,
            Tool::Lasso,
            Tool::Move,
            Tool::Text,
            Tool::Pointer,
        ];
        egui::Grid::new("atelier tool tray")
            .spacing(vec2(4.0, 4.0))
            .show(ui, |ui| {
                for (index, tool) in tools.into_iter().enumerate() {
                    self.rail_button(ui, Item::Tool(tool), side);
                    if index == 4 {
                        ui.end_row();
                    }
                }
                self.paint_color_chips(ui, side);
            });
    }

    /// Color, tool settings or the navigator above the layers, by workspace.
    fn atelier_tray(&mut self, ui: &mut egui::Ui) {
        let total = ui.available_height();
        let tabs = atelier_tabs(self.interface.mode);
        let tab = tabs[self.ui_layout.tray_tab.min(1)];
        let default = total * 0.48;
        let height = self
            .ui_layout
            .tray_height
            .unwrap_or(default)
            .clamp(140.0, (total - 220.0).max(140.0));
        egui::TopBottomPanel::top("atelier tray top")
            .exact_height(height)
            .frame(egui::Frame::NONE)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                let names = tabs.map(|t| self.tab_name(t));
                if let Some(clicked) =
                    widgets::tab_strip(ui, &names, self.ui_layout.tray_tab.min(1))
                {
                    self.ui_layout.tray_tab = clicked;
                }
                self.tab_contents(ui, tab, "atelier tray");
            });
        match split_handle(ui) {
            SplitDrag::Reset => self.ui_layout.tray_height = None,
            SplitDrag::By(delta) => self.ui_layout.tray_height = Some(height + delta),
            SplitDrag::None => {}
        }
        self.layers_section(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::Interface;

    /// Run a few frames of the whole workspace in `layout` at `size`.
    fn frames(app: &mut Dudel, size: Vec2, count: usize) -> Vec<egui::FullOutput> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        for _ in 0..count {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            };
            out.push(ctx.run(input, |ctx| {
                crate::interface::apply(ctx, &app.interface, &app.theme);
                app.workspace(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.canvas_area = ui.max_rect();
                });
                app.float_overlays(ctx);
            }));
        }
        out
    }

    #[test]
    fn every_layout_shows_its_panels_and_the_canvas_keeps_room() {
        for layout in Layout::ALL {
            for size in [vec2(1600.0, 1000.0), vec2(1100.0, 760.0)] {
                let mut app = Dudel {
                    interface: Interface::preset(layout),
                    ..Default::default()
                };
                frames(&mut app, size, 3);
                assert!(
                    app.canvas_area.width() >= 360.0 && app.canvas_area.height() >= 300.0,
                    "{layout:?} at {size:?} leaves {:?}",
                    app.canvas_area
                );
            }
        }
    }

    #[test]
    fn the_tool_button_opens_tool_settings_in_every_styled_layout() {
        for layout in [Layout::Slate, Layout::Float, Layout::Atelier] {
            for mode in Mode::ALL {
                let mut app = Dudel {
                    interface: Interface {
                        mode,
                        ..Interface::preset(layout)
                    },
                    ..Default::default()
                };
                if app.styled_tool_settings_open() {
                    app.toggle_styled_tool_settings();
                }
                assert!(!app.styled_tool_settings_open(), "{layout:?} {mode:?}");
                app.toggle_styled_tool_settings();
                assert!(app.styled_tool_settings_open(), "{layout:?} {mode:?}");
            }
        }
    }

    #[test]
    fn float_popovers_close_when_a_stroke_starts() {
        let mut app = Dudel {
            interface: Interface::preset(Layout::Float),
            ..Default::default()
        };
        app.ui_layout.popover = Some(Popover::Layers);
        frames(&mut app, vec2(1400.0, 900.0), 2);
        assert_eq!(app.ui_layout.popover, Some(Popover::Layers));
        app.stroking = true;
        frames(&mut app, vec2(1400.0, 900.0), 1);
        assert_eq!(app.ui_layout.popover, None);
    }

    #[test]
    fn the_rail_holds_every_tool_once() {
        let items: Vec<Item> = rail_sections().into_iter().flatten().collect();
        for tool in Tool::ALL {
            if tool == Tool::Brush {
                for group in BrushGroup::ALL {
                    assert_eq!(
                        items.iter().filter(|i| **i == Item::Group(group)).count(),
                        1
                    );
                }
            } else {
                assert_eq!(
                    items.iter().filter(|i| **i == Item::Tool(tool)).count(),
                    1,
                    "{}",
                    tool.name()
                );
            }
        }
    }
}
