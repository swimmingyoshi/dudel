//! Dockable palettes, as in Clip Studio Paint and Photoshop. In the Slate,
//! Float and Atelier styles every panel is a palette: a tab that can be
//! dragged to another group, split off beside the canvas, pulled out to float
//! over it, closed, and opened again from the Window menu. The canvas is the
//! one tab that always stays docked.
//!
//! Each style starts from its own arrangement and remembers the artist's
//! changes to it; named workspaces save the arrangement too. Lock palettes
//! (Window menu) stops tabs from being dragged by accident with a pen.
use crate::{
    Dudel, egui, input,
    ui::style::{Active, Layout},
};
use egui_dock::{AllowedSplits, DockArea, DockState, Node, NodeIndex, SurfaceIndex, TabViewer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A palette: one tab of the dock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Palette {
    /// The drawing; always docked, never closed.
    Canvas,
    Tools,
    ToolProperty,
    Brushes,
    Color,
    Navigator,
    Layers,
    LayerProperty,
    Timeline,
}

impl Palette {
    /// Every palette the Window menu lists, in its order.
    pub(crate) const MENU: [Palette; 8] = [
        Palette::Tools,
        Palette::ToolProperty,
        Palette::Brushes,
        Palette::Color,
        Palette::Navigator,
        Palette::Layers,
        Palette::LayerProperty,
        Palette::Timeline,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Palette::Canvas => "Canvas",
            Palette::Tools => "Tools",
            Palette::ToolProperty => "Tool property",
            Palette::Brushes => "Brushes",
            Palette::Color => "Color",
            Palette::Navigator => "Navigator",
            Palette::Layers => "Layers",
            Palette::LayerProperty => "Layer property",
            Palette::Timeline => "Timeline",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Palette::Canvas => "",
            Palette::Tools => "Every tool; right-click or hold one for its variants",
            Palette::ToolProperty => "Size, opacity and every setting of the current tool",
            Palette::Brushes => "The brush library",
            Palette::Color => "Color wheel, sliders and swatches",
            Palette::Navigator => "The whole canvas, zoom and rotation",
            Palette::Layers => "The layer stack",
            Palette::LayerProperty => "Blend mode, opacity, locks and mask of the current layer",
            Palette::Timeline => "Animation frames",
        }
    }
}

/// The arrangement a style starts from.
pub(crate) fn preset(layout: Layout) -> DockState<Palette> {
    if layout == Layout::Float {
        // Float keeps the canvas whole, outside the dock; palettes float over
        // it when opened.
        return DockState::new(Vec::new());
    }
    let mut state = DockState::new(vec![Palette::Canvas]);
    let tree = state.main_surface_mut();
    match layout {
        Layout::Slate => {
            // As in Clip Studio Paint: the tools as a vertical bar at the left
            // edge, as in Classic, with brushes and the tool property beside
            // it; color and layers down the right. `COLUMNS` gives the
            // columns their widths once the window's size is known.
            let [rest, _] = tree.split_left(NodeIndex::root(), 0.05, vec![Palette::Tools]);
            let [canvas, brushes] = tree.split_left(rest, 0.2, vec![Palette::Brushes]);
            tree.split_below(brushes, 0.38, vec![Palette::ToolProperty]);
            let [_, column] =
                tree.split_right(canvas, 0.72, vec![Palette::Color, Palette::Navigator]);
            tree.split_below(column, 0.4, vec![Palette::Layers, Palette::LayerProperty]);
        }
        Layout::Atelier => {
            // Brushes and the tool grid on the left; tool property and color
            // above the layers on the right.
            let [canvas, drawer] = tree.split_left(NodeIndex::root(), 0.19, vec![Palette::Brushes]);
            tree.split_below(drawer, 0.66, vec![Palette::Tools, Palette::Navigator]);
            let [_, tray] =
                tree.split_right(canvas, 0.71, vec![Palette::ToolProperty, Palette::Color]);
            tree.split_below(tray, 0.48, vec![Palette::Layers, Palette::LayerProperty]);
        }
        Layout::Float | Layout::Classic => {}
    }
    state
}

fn in_palette_id() -> egui::Id {
    egui::Id::new("dudel drawing palettes")
}

/// Whether `ui` is inside a palette: Classic's fixed panels and separate
/// windows such as Brush Studio are not.
pub(crate) fn in_palette(ui: &egui::Ui) -> bool {
    ui.data(|d| d.get_temp::<bool>(in_palette_id()))
        .unwrap_or(false)
}

/// The narrowest a palette group may be made, wide enough for a bar of tool
/// buttons. egui_dock's own limit, 175 points, kept the tools from being a bar.
const NARROWEST: f32 = 56.0;

/// Columns of a starting arrangement with a width in points rather than a
/// share of the window: a palette, and the width of the column holding it.
fn columns(layout: Layout) -> &'static [(Palette, f32)] {
    match layout {
        Layout::Slate => &[
            // Classic's rail (58) and room for the tab's name and close box.
            (Palette::Tools, 72.0),
            (Palette::Brushes, 280.0),
            (Palette::Color, 300.0),
        ],
        _ => &[],
    }
}

/// Give each of `layout`'s columns its width, from the sizes the dock was
/// last laid out at. Returns whether anything changed: a column's width
/// depends on those around it, so this runs until it settles.
fn fit_columns(layout: Layout, state: &mut DockState<Palette>) -> bool {
    let mut changed = false;
    for &(palette, width) in columns(layout) {
        let Some((surface, mut node, _)) = state.find_tab(&palette) else {
            continue;
        };
        if surface != egui_dock::SurfaceIndex::main() {
            continue;
        }
        let tree = state.main_surface_mut();
        // The nearest side-by-side split above the palette is its column's edge.
        while let Some(parent) = node.parent() {
            if let Node::Horizontal(split) = &mut tree[parent] {
                let total = split.rect().width();
                if total.is_finite() && total > width * 1.5 {
                    let share = width / total;
                    let fraction = if node == parent.left() {
                        share
                    } else {
                        1.0 - share
                    };
                    if (split.fraction - fraction).abs() * total > 0.5 {
                        split.fraction = fraction;
                        changed = true;
                    }
                }
                break;
            }
            node = parent;
        }
    }
    changed
}

/// The canvas lives outside the dock: Float.
fn canvas_outside(layout: Layout) -> bool {
    layout == Layout::Float
}

/// A usable arrangement: no palette twice, and the canvas exactly where the
/// style wants it (docked in the main area, or outside the dock for Float).
fn valid(layout: Layout, state: &DockState<Palette>) -> bool {
    let mut seen = std::collections::HashSet::new();
    let unique = state.iter_all_tabs().all(|(_, tab)| seen.insert(*tab));
    let canvas = if canvas_outside(layout) {
        // Float's main area stays empty: whatever docked there would cover
        // the canvas.
        state.main_surface().num_tabs() == 0
    } else {
        state.find_main_surface_tab(&Palette::Canvas).is_some()
    };
    unique && canvas && layout != Layout::Classic
}

/// An arrangement as text. RON, not JSON: an area not laid out yet has an
/// infinite rectangle, which JSON cannot hold.
pub(crate) fn encode(state: &DockState<Palette>) -> Option<String> {
    ron::to_string(state).ok()
}

/// A saved arrangement of `layout`, if it reads and is usable.
pub(crate) fn decode(layout: Layout, text: &str) -> Option<DockState<Palette>> {
    const LIMIT: usize = 256 * 1024;
    if text.len() > LIMIT {
        return None;
    }
    ron::from_str(text)
        .ok()
        .filter(|state| valid(layout, state))
}

/// What the dock remembers between sessions: each style's arrangement and
/// the lock.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct DockPrefs {
    #[serde(default)]
    pub locked: bool,
    /// Each style with its arrangement, encoded by `encode`.
    #[serde(default)]
    pub layouts: Vec<(Layout, String)>,
    /// The `PRESETS` the arrangements were saved under.
    #[serde(default)]
    pub presets: u32,
    /// Where each floating palette was left, as left, top, width and height
    /// in points; egui_dock doesn't keep it.
    #[serde(default)]
    pub windows: Vec<(Layout, Palette, [f32; 4])>,
}

/// The smallest a remembered floating palette comes back, so that one left
/// squeezed to its tab can't stay that way.
const SMALLEST_WINDOW: egui::Vec2 = egui::vec2(200.0, 160.0);

/// Raised when the starting arrangements change, so arrangements saved
/// before give way to the new ones once.
const PRESETS: u32 = 2;

/// The dock of the session: one arrangement per style.
#[derive(Default)]
pub(crate) struct Docks {
    states: HashMap<Layout, DockState<Palette>>,
    pub(crate) locked: bool,
    /// Changes asked for while the dock is being drawn.
    pending: Vec<Change>,
    drawing: bool,
    /// Palettes just pulled out to float, placed when the dock is next drawn
    /// and the canvas's real size is known.
    unplaced: Vec<(Layout, Palette)>,
    /// The dock's area last frame: palettes are placed once it holds still,
    /// not while the window is still opening or being resized.
    last_area: Option<egui::Rect>,
    /// The dock's area held still since last frame.
    settled: bool,
    /// Where each floating palette was last seen, to put it back there.
    left: HashMap<(Layout, Palette), egui::Rect>,
    /// Styles whose starting arrangement hasn't had its column widths set.
    unfitted: HashSet<Layout>,
    /// While the dock is drawn its arrangement is out of the map: which
    /// palettes are in it, and which are on screen.
    drawn: Option<(Vec<Palette>, Vec<Palette>)>,
}

/// The palettes of an arrangement, and those on screen (each group's chosen tab).
fn listing(state: &DockState<Palette>) -> (Vec<Palette>, Vec<Palette>) {
    let present = state.iter_all_tabs().map(|(_, tab)| *tab).collect();
    let shown = state
        .iter_leaves()
        .filter_map(|(_, leaf)| leaf.tabs().get(leaf.active.0).copied())
        .collect();
    (present, shown)
}

#[derive(Clone, Copy)]
enum Change {
    Show(Palette),
    Hide(Palette),
    Reset,
}

impl Docks {
    /// Restore saved arrangements, keeping only usable ones.
    pub(crate) fn restore(&mut self, prefs: Option<DockPrefs>) {
        let prefs = prefs.unwrap_or_default();
        self.locked = prefs.locked;
        let current = prefs.presets >= PRESETS;
        self.states = prefs
            .layouts
            .into_iter()
            .filter(|_| current)
            .filter_map(|(layout, text)| Some((layout, decode(layout, &text)?)))
            .collect();
        self.left = prefs
            .windows
            .into_iter()
            .filter(|(_, _, rect)| rect.iter().all(|v| v.is_finite()))
            .map(|(layout, palette, [x, y, w, h])| {
                let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                ((layout, palette), rect)
            })
            .collect();
        // egui forgets where windows were, so floating palettes are put back
        // where they were left once the window's size is known.
        self.unplaced.clear();
        let layouts: Vec<Layout> = self.states.keys().copied().collect();
        for layout in layouts {
            self.queue_floating(layout);
        }
    }

    pub(crate) fn prefs(&self) -> DockPrefs {
        let order = |layout: &Layout| Layout::ALL.iter().position(|l| l == layout);
        let mut layouts: Vec<_> = self
            .states
            .iter()
            .filter_map(|(layout, state)| Some((*layout, encode(state)?)))
            .collect();
        layouts.sort_by_key(|(layout, _)| order(layout));
        let mut windows: Vec<_> = self
            .left
            .iter()
            .map(|(&(layout, palette), rect)| {
                (
                    layout,
                    palette,
                    [rect.left(), rect.top(), rect.width(), rect.height()],
                )
            })
            .collect();
        windows.sort_by_key(|(layout, palette, _)| (order(layout), *palette as usize));
        DockPrefs {
            locked: self.locked,
            layouts,
            presets: PRESETS,
            windows,
        }
    }

    /// Every style back to the arrangement it starts with.
    pub(crate) fn reset_all(&mut self) {
        self.states.clear();
        self.unplaced.clear();
        self.left.clear();
        self.locked = false;
    }

    /// Place every floating window of `layout` again, by its first palette.
    fn queue_floating(&mut self, layout: Layout) {
        let Some(state) = self.states.get(&layout) else {
            return;
        };
        let mut windows: Vec<(SurfaceIndex, Palette)> = Vec::new();
        for ((surface, _), tab) in state.iter_all_tabs() {
            if surface != SurfaceIndex::main() && !windows.iter().any(|(s, _)| *s == surface) {
                windows.push((surface, *tab));
            }
        }
        for (_, palette) in windows {
            if !self.unplaced.contains(&(layout, palette)) {
                self.unplaced.push((layout, palette));
            }
        }
    }

    /// Note where the floating palettes of `layout` are, as egui drew them:
    /// the window's corner and the size of what is inside its frame, as
    /// `set_position` and `set_size` take them.
    fn remember_windows(
        &mut self,
        ctx: &egui::Context,
        layout: Layout,
        state: &DockState<Palette>,
    ) {
        let frame = egui::Frame::window(&ctx.style()).total_margin().sum();
        for ((surface, _), tab) in state.iter_all_tabs() {
            // A window still waiting for its place may show another's rect.
            if surface == SurfaceIndex::main() || self.unplaced.contains(&(layout, *tab)) {
                continue;
            }
            if let Some(rect) = window_rect(ctx, surface)
                && rect.is_finite()
                && rect.is_positive()
            {
                let inside = egui::Rect::from_min_size(rect.min, rect.size() - frame);
                self.left.insert((layout, *tab), inside);
            }
        }
    }

    /// The arrangement of `layout`, as saved in a workspace.
    pub(crate) fn arrangement(&self, layout: Layout) -> Option<DockState<Palette>> {
        self.states.get(&layout).cloned()
    }

    /// Put back an arrangement from a workspace.
    pub(crate) fn set_arrangement(&mut self, layout: Layout, state: DockState<Palette>) {
        if valid(layout, &state) {
            self.states.insert(layout, state);
            self.queue_floating(layout);
        }
    }

    fn state(&mut self, layout: Layout) -> &mut DockState<Palette> {
        self.states.entry(layout).or_insert_with(|| {
            self.unfitted.insert(layout);
            preset(layout)
        })
    }

    fn visible(&self, layout: Layout, palette: Palette) -> bool {
        if let Some((_, shown)) = &self.drawn {
            return shown.contains(&palette);
        }
        let check = |state: &DockState<Palette>| {
            state
                .find_tab(&palette)
                .is_some_and(|(surface, node, tab)| {
                    state[surface][node]
                        .get_leaf()
                        .is_some_and(|leaf| leaf.active == tab)
                })
        };
        match self.states.get(&layout) {
            Some(state) => check(state),
            None => check(&preset(layout)),
        }
    }

    fn present(&self, layout: Layout, palette: Palette) -> bool {
        if let Some((present, _)) = &self.drawn {
            return present.contains(&palette);
        }
        match self.states.get(&layout) {
            Some(state) => state.find_tab(&palette).is_some(),
            None => preset(layout).find_tab(&palette).is_some(),
        }
    }

    /// Where the floating palettes of `layout` are.
    #[cfg(test)]
    pub(crate) fn floating_rects(&mut self, layout: Layout) -> Vec<egui::Rect> {
        let state = self.state(layout);
        let windows: Vec<SurfaceIndex> = state
            .iter_all_tabs()
            .map(|((surface, _), _)| surface)
            .filter(|s| *s != SurfaceIndex::main())
            .collect();
        windows
            .into_iter()
            .filter_map(|s| state.get_window_state(s).map(|w| w.rect()))
            .collect()
    }

    /// Place palettes that just started floating, or came back from saved
    /// preferences or a workspace: where they were left, or else side by
    /// side from the right, below Float's islands, in the order they were
    /// opened.
    fn place_floating(&mut self, layout: Layout, area: egui::Rect) {
        let settled = self.last_area == Some(area);
        self.last_area = Some(area);
        self.settled = settled;
        if !settled || !self.unplaced.iter().any(|(at, _)| *at == layout) {
            return;
        }
        let mut placed = 0;
        let (unplaced, others) = std::mem::take(&mut self.unplaced)
            .into_iter()
            .partition::<Vec<_>, _>(|(at, _)| *at == layout);
        // Other styles' palettes wait until their style is shown.
        self.unplaced = others;
        self.state(layout);
        let Some(state) = self.states.get_mut(&layout) else {
            return;
        };
        for (_, palette) in unplaced {
            let Some((surface, _, _)) = state.find_tab(&palette) else {
                continue;
            };
            let Some(window) = state.get_window_state_mut(surface) else {
                continue;
            };
            if let Some(rect) = self.left.get(&(layout, palette)) {
                let size = rect.size().max(SMALLEST_WINDOW).min(area.size());
                let at = egui::pos2(
                    rect.left().clamp(area.left(), area.right() - size.x),
                    rect.top().clamp(area.top(), area.bottom() - size.y),
                );
                window.set_position(at).set_size(size);
                continue;
            }
            let width = 320.0;
            let height = (area.height() * 0.62).clamp(220.0, 640.0);
            let column = (placed % 3) as f32;
            let row = (placed / 3) as f32;
            window
                .set_position(egui::pos2(
                    (area.right() - 16.0 - (width + 12.0) * (column + 1.0)).max(area.left() + 16.0),
                    area.top() + 132.0 + 32.0 * row,
                ))
                .set_size(egui::vec2(width, height));
            placed += 1;
        }
    }

    fn apply(&mut self, layout: Layout, change: Change) {
        if self.drawing {
            self.pending.push(change);
            return;
        }
        match change {
            Change::Reset => {
                self.states.insert(layout, preset(layout));
                self.unfitted.insert(layout);
            }
            Change::Hide(palette) => {
                let state = self.state(layout);
                if palette != Palette::Canvas
                    && let Some(found) = state.find_tab(&palette)
                {
                    state.remove_tab(found);
                }
            }
            Change::Show(palette) => {
                let state = self.state(layout);
                if let Some((surface, node, tab)) = state.find_tab(&palette) {
                    state.set_active_tab((surface, node, tab));
                    state.set_focused_node_and_surface((surface, node));
                    return;
                }
                // Back into the group it starts in, if one of its partners is
                // still docked; otherwise it floats beside the canvas.
                let partners: Vec<Palette> = preset(layout)
                    .iter_all_tabs()
                    .filter(|(_, tab)| **tab != palette)
                    .filter(|(at, _)| {
                        preset(layout)
                            .find_tab(&palette)
                            .is_some_and(|(s, n, _)| (s, n) == *at)
                    })
                    .map(|(_, tab)| *tab)
                    .collect();
                let home = partners.iter().find_map(|p| state.find_tab(p));
                if let Some((surface, node, _)) = home
                    && let Some(leaf) = state[surface][node].get_leaf_mut()
                {
                    leaf.append_tab(palette);
                    let last = leaf.tabs().len() - 1;
                    leaf.set_active_tab(last);
                    return;
                }
                // The timeline docks under the canvas, as a strip.
                if palette == Palette::Timeline
                    && let Some((canvas_node, _)) = state.find_main_surface_tab(&Palette::Canvas)
                {
                    state
                        .main_surface_mut()
                        .split_below(canvas_node, 0.8, vec![palette]);
                    return;
                }
                state.add_window(vec![palette]);
                self.unplaced.push((layout, palette));
            }
        }
    }
}

impl Dudel {
    /// The styles built from palettes; Classic keeps its fixed panels.
    pub(crate) fn docked(&self) -> bool {
        self.interface.layout != Layout::Classic
    }

    /// The palette is on screen: docked or floating, and its group's chosen tab.
    pub(crate) fn palette_visible(&self, palette: Palette) -> bool {
        self.docked() && self.ui_layout.docks.visible(self.interface.layout, palette)
    }

    /// The palette is somewhere in the arrangement, chosen or not.
    pub(crate) fn palette_present(&self, palette: Palette) -> bool {
        self.docked() && self.ui_layout.docks.present(self.interface.layout, palette)
    }

    pub(crate) fn show_palette(&mut self, palette: Palette) {
        self.show_panels = true;
        self.ui_layout
            .docks
            .apply(self.interface.layout, Change::Show(palette));
    }

    pub(crate) fn hide_palette(&mut self, palette: Palette) {
        self.ui_layout
            .docks
            .apply(self.interface.layout, Change::Hide(palette));
    }

    pub(crate) fn toggle_palette(&mut self, palette: Palette) {
        if self.palette_visible(palette) {
            self.hide_palette(palette);
        } else {
            self.show_palette(palette);
        }
    }

    /// Back to the style's own arrangement.
    pub(crate) fn reset_palettes(&mut self) {
        self.ui_layout
            .docks
            .apply(self.interface.layout, Change::Reset);
        self.status = format!(
            "Palettes back to the {} arrangement",
            self.interface.layout.name()
        );
    }

    /// The Window menu: every palette with a check, the lock and the reset.
    pub(crate) fn palettes_menu(&mut self, ui: &mut egui::Ui) {
        for palette in Palette::MENU {
            let mut shown = self.palette_present(palette);
            if ui
                .checkbox(&mut shown, palette.name())
                .on_hover_text(palette.hint())
                .clicked()
            {
                if shown {
                    self.show_palette(palette);
                } else {
                    self.hide_palette(palette);
                }
                ui.close();
            }
        }
        ui.separator();
        ui.checkbox(&mut self.ui_layout.docks.locked, "Lock palettes")
            .on_hover_text("Keep tabs where they are: no dragging, closing or floating");
        if ui
            .button(format!(
                "Reset to the {} arrangement",
                self.interface.layout.name()
            ))
            .clicked()
        {
            self.reset_palettes();
            ui.close();
        }
    }

    /// The dock filling the central area: palettes around the canvas, or for
    /// Float the canvas with palettes floating over it.
    pub(crate) fn dock_ui(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        packets: &[input::Packet],
    ) {
        let layout = self.interface.layout;
        let whole = ui.max_rect();
        if canvas_outside(layout) {
            self.canvas_tab_ui(ctx, ui, packets);
        }
        self.ui_layout.docks.place_floating(layout, whole);
        let empty = || DockState::new(Vec::new());
        let saved = std::mem::replace(self.ui_layout.docks.state(layout), empty());
        // Tab hides every palette; the arrangement waits unchanged.
        let (mut state, waiting) = if self.show_panels {
            (saved, None)
        } else if canvas_outside(layout) {
            (empty(), Some(saved))
        } else {
            (DockState::new(vec![Palette::Canvas]), Some(saved))
        };
        let before = state.clone();
        let locked = self.ui_layout.docks.locked;
        let mut style = egui_dock::Style::from_egui(ui.style().as_ref());
        let active = Active::get(ctx);
        if active.styled {
            style.tab.active.text_color = ui.visuals().strong_text_color();
            style.tab.focused.text_color = ui.visuals().strong_text_color();
            style.tab.hovered.text_color = ui.visuals().text_color();
            style.tab.active.outline_color = active.accent;
            style.tab.focused.outline_color = active.accent;
        }
        style.tab_bar.fill_tab_bar = true;
        style.separator.extra = NARROWEST;
        // A tab's right-click menu: Close, and Float to pull it out.
        state.translations.tab_context_menu = egui_dock::TabContextMenuTranslations {
            close_button: "Close".into(),
            eject_button: "Float".into(),
        };
        self.ui_layout.docks.drawing = true;
        self.ui_layout.docks.drawn = Some(listing(&state));
        ctx.data_mut(|d| d.insert_temp(in_palette_id(), true));
        let floating = state
            .iter_all_tabs()
            .filter(|((surface, _), _)| *surface != egui_dock::SurfaceIndex::main())
            .map(|(_, tab)| *tab)
            .collect();
        let mut viewer = Viewer {
            app: self,
            ctx,
            packets,
            floating,
            moves: Vec::new(),
        };
        let area = DockArea::new(&mut state)
            .id(egui::Id::new("dudel dock"))
            .style(style)
            .draggable_tabs(!locked)
            // Closing is in a tab's right-click menu and the Window menu, so
            // tabs stay uncluttered; a group or a floating palette has its X.
            .show_close_buttons(false)
            .tab_context_menus(!locked)
            .allowed_splits(if locked {
                AllowedSplits::None
            } else {
                AllowedSplits::All
            })
            .show_leaf_close_all_buttons(!locked)
            .show_leaf_collapse_buttons(false)
            .window_bounds(whole);
        if canvas_outside(layout) {
            // Over the canvas, which already took the space.
            ui.scope_builder(egui::UiBuilder::new().max_rect(whole), |ui| {
                area.show_inside(ui, &mut viewer);
            });
        } else {
            area.show_inside(ui, &mut viewer);
        }
        let moves = viewer.moves;
        self.ui_layout.docks.drawing = false;
        self.ui_layout.docks.drawn = None;
        ctx.data_mut(|d| d.remove_temp::<bool>(in_palette_id()));
        move_windows(ctx, &mut state, &moves, whole);
        // Float has no docking: a palette dropped on its canvas floats where
        // it was dropped, so dragging a palette by its tab moves it.
        if canvas_outside(layout) {
            let pointer = ctx.pointer_latest_pos().unwrap_or(whole.center());
            float_strays(&mut state, pointer, whole);
        }
        // A drop the style can't take snaps back.
        let mut state = if valid(layout, &state) || state.iter_all_tabs().next().is_none() {
            state
        } else {
            before
        };
        // A starting arrangement gets its column widths once the window's
        // size holds still, over a few frames, as each column's width
        // depends on the others.
        if waiting.is_none()
            && self.ui_layout.docks.settled
            && self.ui_layout.docks.unfitted.contains(&layout)
        {
            if fit_columns(layout, &mut state) {
                ctx.request_repaint();
            } else {
                self.ui_layout.docks.unfitted.remove(&layout);
            }
        }
        if waiting.is_none() {
            self.ui_layout.docks.remember_windows(ctx, layout, &state);
        }
        *self.ui_layout.docks.state(layout) = waiting.unwrap_or(state);
        for change in std::mem::take(&mut self.ui_layout.docks.pending) {
            self.ui_layout.docks.apply(layout, change);
        }
    }

    /// The canvas with the strip naming what a stroke edits.
    fn canvas_tab_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, packets: &[input::Packet]) {
        egui::TopBottomPanel::top("editing target")
            .show_inside(ui, |ui| self.editing_target_ui(ui));
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(
            rect,
            0.0,
            crate::color_wheel::color32(self.canvas_background()),
        );
        self.canvas_ui(ctx, ui, packets);
    }
}

struct Viewer<'a> {
    app: &'a mut Dudel,
    ctx: &'a egui::Context,
    packets: &'a [input::Packet],
    /// Palettes in floating windows, which get a grip to move them by.
    floating: Vec<Palette>,
    /// Floating palettes dragged by their grip this frame, and how far.
    moves: Vec<(Palette, egui::Vec2)>,
}

/// Height of the grip along the top of a floating palette.
const GRIP: f32 = 16.0;

/// Where a floating window was last drawn. egui_dock 0.18 never fills in
/// its own `WindowState::rect`, so this asks egui for the window's area,
/// which egui_dock names after the surface.
fn window_rect(ctx: &egui::Context, surface: egui_dock::SurfaceIndex) -> Option<egui::Rect> {
    let id = egui::Id::new(format!("window {surface:?}"));
    ctx.memory(|m| m.area_rect(id))
}

/// Move the windows whose grip was dragged, keeping them inside `bounds`.
fn move_windows(
    ctx: &egui::Context,
    state: &mut DockState<Palette>,
    moves: &[(Palette, egui::Vec2)],
    bounds: egui::Rect,
) {
    for &(palette, delta) in moves {
        let Some((surface, _, _)) = state.find_tab(&palette) else {
            continue;
        };
        let Some(rect) = window_rect(ctx, surface) else {
            continue;
        };
        let Some(window) = state.get_window_state_mut(surface) else {
            continue;
        };
        let mut to = rect.translate(delta);
        // Keep the grip reachable: never past the bounds' edges.
        to = to.translate(egui::vec2(
            (bounds.left() - to.left()).max(0.0) - (to.right() - bounds.right()).max(0.0),
            (bounds.top() - to.top()).max(0.0) - (to.top() + GRIP * 2.0 - bounds.bottom()).max(0.0),
        ));
        window.set_position(to.min);
    }
}

/// Palettes dropped into Float's main area (where they would cover the
/// canvas) become a floating window at the drop point instead.
fn float_strays(state: &mut DockState<Palette>, pointer: egui::Pos2, bounds: egui::Rect) {
    let strays: Vec<Palette> = state
        .iter_all_tabs()
        .filter(|((surface, _), _)| *surface == egui_dock::SurfaceIndex::main())
        .map(|(_, tab)| *tab)
        .filter(|tab| *tab != Palette::Canvas)
        .collect();
    if strays.is_empty() {
        return;
    }
    for palette in &strays {
        if let Some(found) = state.find_tab(palette) {
            state.remove_tab(found);
        }
    }
    let size = egui::vec2(320.0, (bounds.height() * 0.62).clamp(220.0, 640.0));
    let at = egui::pos2(
        (pointer.x - 40.0).clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left())),
        (pointer.y - 12.0).clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
    );
    let surface = state.add_window(strays);
    if let Some(window) = state.get_window_state_mut(surface) {
        window.set_position(at).set_size(size);
    }
}

impl TabViewer for Viewer<'_> {
    type Tab = Palette;

    fn title(&mut self, tab: &mut Palette) -> egui::WidgetText {
        match tab {
            Palette::Canvas => {
                let (name, modified) = self.app.tab_title(self.app.tabs.active);
                if modified {
                    format!("{name} •")
                } else {
                    name
                }
                .into()
            }
            Palette::Color => self.app.color_tab_name().into(),
            _ => tab.name().into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Palette) {
        if *tab == Palette::Canvas {
            self.app.canvas_tab_ui(self.ctx, ui, self.packets);
            return;
        }
        // Room between a palette's edges and its contents, and scroll bars
        // that stay in view, beside the contents rather than over them,
        // whenever there is more to scroll to.
        let mut scroll = egui::style::ScrollStyle::solid();
        scroll.bar_width = 8.0;
        ui.spacing_mut().scroll = scroll;
        // A floating palette's tab fills its bar, and dragging the tab pulls
        // the palette out; the grip below it moves the whole window.
        if self.floating.contains(tab) {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), GRIP), egui::Sense::drag());
            let color = if response.hovered() || response.dragged() {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            ui.painter().rect_filled(
                egui::Rect::from_center_size(rect.center(), egui::vec2(40.0, 4.0)),
                2.0,
                color,
            );
            let response = response
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag to move this palette");
            if response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                let delta = response.drag_delta();
                if delta != egui::Vec2::ZERO {
                    self.moves.push((*tab, delta));
                }
            }
        }
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                self.palette_ui(ui, *tab);
            });
    }

    fn id(&mut self, tab: &mut Palette) -> egui::Id {
        egui::Id::new(("palette", *tab))
    }

    fn is_closeable(&self, tab: &Palette) -> bool {
        *tab != Palette::Canvas
    }

    fn allowed_in_windows(&self, tab: &mut Palette) -> bool {
        *tab != Palette::Canvas
    }

    fn clear_background(&self, tab: &Palette) -> bool {
        *tab != Palette::Canvas
    }

    fn scroll_bars(&self, _tab: &Palette) -> [bool; 2] {
        [false, false]
    }

    fn on_tab_button(&mut self, tab: &mut Palette, response: &egui::Response) {
        if *tab != Palette::Canvas {
            response.clone().on_hover_text(tab.hint());
        }
    }
}

impl Viewer<'_> {
    fn palette_ui(&mut self, ui: &mut egui::Ui, tab: Palette) {
        let app = &mut *self.app;
        match tab {
            Palette::Canvas => {}
            Palette::Tools => app.tools_palette(ui),
            Palette::ToolProperty => app.tool_property_palette(ui),
            Palette::Brushes => {
                app.brush_groups_row(ui);
                app.brush_library_ui(ui);
            }
            Palette::Color => {
                if ui.available_height() < 300.0 {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.min(26.0);
                    app.compact_palette_ui(ui);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("color palette")
                        .show(ui, |ui| app.palette_ui(ui));
                }
            }
            Palette::Navigator => {
                let room = ui.available_height();
                egui::ScrollArea::vertical()
                    .id_salt("navigator palette")
                    .show(ui, |ui| app.navigator_ui(ui, room));
            }
            Palette::Layers => {
                ui.horizontal(|ui| app.layer_guide_controls(ui));
                // With Layer property open, its controls live there.
                let inline = !app.palette_visible(Palette::LayerProperty);
                app.layers_ui(ui, inline, true);
            }
            Palette::LayerProperty => {
                egui::ScrollArea::vertical()
                    .id_salt("layer property")
                    .auto_shrink([false, false])
                    .show(ui, |ui| app.layer_controls_ui(ui));
            }
            Palette::Timeline => app.timeline_palette(ui),
        }
    }
}

impl Dudel {
    /// Brush groups as chips, above the library.
    pub(crate) fn brush_groups_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for group in crate::brush::BrushGroup::ALL {
                let chosen = self.tool == crate::tools::Tool::Brush && self.brush_group == group;
                if ui
                    .selectable_label(chosen, group.name())
                    .on_hover_text(group.hint())
                    .clicked()
                {
                    self.select_brush_group(group);
                }
            }
        });
    }

    fn timeline_palette(&mut self, ui: &mut egui::Ui) {
        if self.doc.animation.is_some() {
            self.timeline_ui(ui);
        } else {
            ui.horizontal(|ui| {
                ui.label("This canvas has no frames yet.");
                if ui.button("Turn on the animation timeline").clicked() {
                    self.execute_command(crate::commands::Action::AnimationTimeline);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_usable_and_saves_and_restores() {
        for layout in [Layout::Slate, Layout::Float, Layout::Atelier] {
            let state = preset(layout);
            assert!(valid(layout, &state), "{layout:?}");
            let mut docks = Docks::default();
            docks.states.insert(layout, state);
            docks.locked = true;
            let text = serde_json::to_string(&docks.prefs()).unwrap();
            let mut back = Docks::default();
            back.restore(serde_json::from_str(&text).ok());
            assert!(back.locked);
            assert!(back.states.contains_key(&layout));
        }
    }

    #[test]
    fn an_arrangement_without_the_canvas_is_not_restored() {
        let broken = encode(&DockState::new(vec![Palette::Layers])).unwrap();
        let mut docks = Docks::default();
        docks.restore(Some(DockPrefs {
            locked: false,
            layouts: vec![
                (Layout::Slate, broken),
                (Layout::Atelier, "not an arrangement".into()),
            ],
            presets: PRESETS,
            windows: Vec::new(),
        }));
        assert!(docks.states.is_empty());
    }

    #[test]
    fn arrangements_saved_before_the_presets_changed_start_fresh() {
        let mut docks = Docks::default();
        docks
            .states
            .insert(Layout::Atelier, preset(Layout::Atelier));
        let mut prefs = docks.prefs();
        prefs.presets = PRESETS - 1;
        prefs.locked = true;
        let mut back = Docks::default();
        back.restore(Some(prefs));
        assert!(back.states.is_empty());
        assert!(back.locked);
    }

    #[test]
    fn slate_keeps_tools_brushes_and_tool_property_down_the_left() {
        let state = preset(Layout::Slate);
        let under = |palette, ancestor: NodeIndex| {
            let (_, mut node, _) = state.find_tab(&palette).unwrap();
            loop {
                if node == ancestor {
                    return true;
                }
                match node.parent() {
                    Some(parent) => node = parent,
                    None => return false,
                }
            }
        };
        // The tools are a bar of their own at the left edge.
        let bar = NodeIndex::root().left();
        assert!(under(Palette::Tools, bar));
        // Brushes and the tool property make the next column, left of the canvas.
        let rest = NodeIndex::root().right();
        for palette in [Palette::Brushes, Palette::ToolProperty] {
            assert!(under(palette, rest.left()), "{palette:?}");
        }
        for palette in [Palette::Canvas, Palette::Color, Palette::Layers] {
            assert!(under(palette, rest.right()), "{palette:?}");
        }
        // Each in a group of its own, so all three are on screen at once.
        let shown = listing(&state).1;
        for palette in [Palette::Tools, Palette::Brushes, Palette::ToolProperty] {
            assert!(shown.contains(&palette), "{palette:?}");
        }
    }

    #[test]
    fn slate_columns_get_their_widths_in_points_once_laid_out() {
        let mut state = preset(Layout::Slate);
        let widths = |state: &DockState<Palette>| {
            columns(Layout::Slate)
                .iter()
                .map(|(palette, _)| {
                    let (_, node, _) = state.find_tab(palette).unwrap();
                    let mut column = node;
                    while let Some(parent) = column.parent() {
                        if matches!(state.main_surface()[parent], Node::Horizontal(_)) {
                            break;
                        }
                        column = parent;
                    }
                    state.main_surface()[column].rect().unwrap().width()
                })
                .collect::<Vec<_>>()
        };
        for screen in [1280.0, 1920.0] {
            let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(screen, 800.0));
            let ctx = egui::Context::default();
            let mut rounds = 0;
            loop {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(area),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let mut style = egui_dock::Style::from_egui(ui.style());
                            style.separator.extra = NARROWEST;
                            DockArea::new(&mut state)
                                .style(style)
                                .show_inside(ui, &mut Empty);
                        });
                    },
                );
                rounds += 1;
                if !fit_columns(Layout::Slate, &mut state) || rounds > 6 {
                    break;
                }
            }
            assert!(rounds <= 6, "settles at {screen}");
            for (width, (palette, wanted)) in widths(&state).into_iter().zip(columns(Layout::Slate))
            {
                assert!(
                    (width - wanted).abs() < 8.0,
                    "{palette:?} {width} at {screen}"
                );
            }
        }
    }

    #[test]
    fn a_palette_dropped_on_floats_canvas_floats_where_dropped_and_its_grip_moves_it() {
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        // As if a floating Color palette's tab were dropped on the canvas.
        let mut state: DockState<Palette> = DockState::new(vec![Palette::Color]);
        float_strays(&mut state, egui::pos2(600.0, 300.0), bounds);
        assert!(valid(Layout::Float, &state));
        let (surface, _, _) = state.find_tab(&Palette::Color).unwrap();
        assert_ne!(surface, egui_dock::SurfaceIndex::main());

        // Lay it out once, then drag its grip.
        let ctx = egui::Context::default();
        let frame = |state: &mut DockState<Palette>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        DockArea::new(state)
                            .window_bounds(bounds)
                            .show_inside(ui, &mut Empty);
                    });
                },
            );
            window_rect(&ctx, surface).unwrap()
        };
        frame(&mut state);
        let before = frame(&mut state);
        assert!((before.left() - 560.0).abs() < 1.0, "{before:?}");
        move_windows(
            &ctx,
            &mut state,
            &[(Palette::Color, egui::vec2(-200.0, -50.0))],
            bounds,
        );
        let after = frame(&mut state);
        assert!(
            (after.min - before.min - egui::vec2(-200.0, -50.0)).length() < 1.0,
            "{after:?}"
        );
        // Never pushed past the left edge.
        move_windows(
            &ctx,
            &mut state,
            &[(Palette::Color, egui::vec2(-2000.0, 0.0))],
            bounds,
        );
        let edge = frame(&mut state);
        assert!(edge.left() >= bounds.left() - 0.5, "{edge:?}");
    }

    #[test]
    fn floating_palettes_come_back_where_they_were_left() {
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut state = preset(Layout::Float);
        state.add_window(vec![Palette::Color]);
        let text = encode(&state).unwrap();
        let saved = |windows| DockPrefs {
            locked: false,
            layouts: vec![(Layout::Float, text.clone())],
            presets: PRESETS,
            windows,
        };
        // A restart: a new egui context that knows no window yet.
        let restart = |prefs: DockPrefs| {
            let mut docks = Docks::default();
            docks.restore(Some(prefs));
            let ctx = egui::Context::default();
            for _ in 0..4 {
                docks.place_floating(Layout::Float, bounds);
                let mut state = docks.states.remove(&Layout::Float).unwrap();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(bounds),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            DockArea::new(&mut state)
                                .window_bounds(bounds)
                                .show_inside(ui, &mut Empty);
                        });
                    },
                );
                docks.remember_windows(&ctx, Layout::Float, &state);
                docks.states.insert(Layout::Float, state);
            }
            let (surface, _, _) = docks.states[&Layout::Float]
                .find_tab(&Palette::Color)
                .unwrap();
            (window_rect(&ctx, surface).unwrap(), docks)
        };
        // Saved before positions were kept: placed like a new palette, not
        // squeezed into the top-left corner.
        let (rect, _) = restart(saved(Vec::new()));
        assert!(rect.left() > bounds.center().x, "{rect:?}");
        assert!(rect.height() >= 220.0, "{rect:?}");
        // Where it was left, at the size it had, and the same again after
        // another restart: the frame isn't added each time.
        let left = [100.0, 150.0, 300.0, 400.0];
        let (rect, docks) = restart(saved(vec![(Layout::Float, Palette::Color, left)]));
        assert!(
            (rect.min - egui::pos2(100.0, 150.0)).length() < 1.0,
            "{rect:?}"
        );
        let again = docks.prefs().windows;
        assert_eq!(again.len(), 1);
        for (got, wanted) in again[0].2.iter().zip(left) {
            assert!((got - wanted).abs() < 1.0, "{again:?}");
        }
        // One squeezed to its tab comes back at a usable size.
        let (rect, _) = restart(saved(vec![(
            Layout::Float,
            Palette::Color,
            [0.0, 0.0, 210.0, 24.0],
        )]));
        assert!(rect.height() >= SMALLEST_WINDOW.y, "{rect:?}");
    }

    struct Empty;
    impl TabViewer for Empty {
        type Tab = Palette;
        fn title(&mut self, tab: &mut Palette) -> egui::WidgetText {
            tab.name().into()
        }
        fn ui(&mut self, _ui: &mut egui::Ui, _tab: &mut Palette) {}
    }

    #[test]
    fn closed_palettes_come_back_to_their_group_or_float() {
        let mut docks = Docks::default();
        // Slate: Navigator shares a group with Color, so it returns there.
        assert!(docks.visible(Layout::Slate, Palette::Color));
        docks.apply(Layout::Slate, Change::Hide(Palette::Navigator));
        assert!(!docks.present(Layout::Slate, Palette::Navigator));
        docks.apply(Layout::Slate, Change::Show(Palette::Navigator));
        let state = docks.state(Layout::Slate);
        let (s, n, _) = state.find_tab(&Palette::Navigator).unwrap();
        assert_eq!(
            Some((s, n)),
            state.find_tab(&Palette::Color).map(|(s, n, _)| (s, n))
        );
        assert!(docks.visible(Layout::Slate, Palette::Navigator));
        // Float has no groups: palettes open as floating windows.
        docks.apply(Layout::Float, Change::Show(Palette::Layers));
        let state = docks.state(Layout::Float);
        let (surface, _, _) = state.find_tab(&Palette::Layers).unwrap();
        assert_ne!(surface, SurfaceIndex::main());
        // The canvas can't be closed.
        docks.apply(Layout::Slate, Change::Hide(Palette::Canvas));
        assert!(docks.present(Layout::Slate, Palette::Canvas));
    }

    #[test]
    fn a_group_shows_only_its_chosen_tab() {
        let (present, shown) = listing(&preset(Layout::Atelier));
        assert!(present.contains(&Palette::LayerProperty));
        // Layers is chosen in its group, so Layer property is behind it and
        // the Layers palette keeps the layer controls.
        assert!(shown.contains(&Palette::Layers) && !shown.contains(&Palette::LayerProperty));
        // Brushes stands on its own, so Tool property leaves its library out.
        assert!(shown.contains(&Palette::Brushes) && shown.contains(&Palette::ToolProperty));
    }

    #[test]
    fn float_never_docks_over_its_canvas() {
        let mut docked = DockState::new(vec![Palette::Layers]);
        assert!(!valid(Layout::Float, &docked));
        docked = DockState::new(Vec::new());
        docked.add_window(vec![Palette::Layers]);
        assert!(valid(Layout::Float, &docked));
        // Slate needs its canvas in the main area.
        assert!(!valid(Layout::Slate, &docked));
    }
}
