//! The pen quick menu: the pen's side button (or a right-click) on the canvas
//! opens a ring under the tip with the starred brushes around the outside,
//! recent colors inside them and a few chosen commands in the middle. Picking
//! anything, a press outside, Esc or the side button again closes it.
//!
//! The ring is workspace UI, never artwork: its commands are the ordinary
//! command list's and its brushes the library's favorites, so it keeps no
//! second copy of either. While it is open the canvas takes no pen input, and
//! a press that closes it is swallowed until it lifts, so it never paints.
use crate::{Dudel, commands, egui};

/// Command slots in the middle of the ring.
pub(crate) const ACTION_SLOTS: usize = 4;
/// Brushes on the outer ring and colors on the inner one, at most.
const MOST_BRUSHES: usize = 10;
const MOST_COLORS: usize = 8;
/// Where the rings run, from the tip, and the size of what sits on them.
const BRUSH_RING: f32 = 138.0;
const COLOR_RING: f32 = 86.0;
const BRUSH_SIZE: egui::Vec2 = egui::vec2(96.0, 32.0);
const COLOR_SIZE: f32 = 28.0;
const ACTION_SIZE: egui::Vec2 = egui::vec2(58.0, 30.0);
/// How far out the brushes run: further for more of them, so that the
/// pills at the top and bottom never overlap.
fn brush_ring(count: usize) -> f32 {
    BRUSH_RING.max(count as f32 * (BRUSH_SIZE.x + 8.0) / std::f32::consts::TAU)
}

/// The ring's reach from the tip for `count` brushes.
fn reach(count: usize) -> f32 {
    brush_ring(count) + BRUSH_SIZE.x / 2.0 + 6.0
}

/// The commands in the middle of the ring, by command id.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct QuickMenuPrefs {
    pub(crate) actions: Vec<String>,
}

impl Default for QuickMenuPrefs {
    fn default() -> Self {
        Self {
            actions: ["edit.undo", "edit.redo", "tool.eraser", "view.flip"]
                .map(String::from)
                .to_vec(),
        }
    }
}

impl QuickMenuPrefs {
    /// Make saved slots safe: known commands only, at most `ACTION_SLOTS`.
    pub(crate) fn sanitized(mut self) -> Self {
        self.actions
            .retain(|id| commands::COMMANDS.iter().any(|c| c.id == id));
        self.actions.truncate(ACTION_SLOTS);
        self
    }
    fn commands(&self) -> Vec<&'static commands::Command> {
        self.actions
            .iter()
            .filter_map(|id| commands::COMMANDS.iter().find(|c| c.id == id))
            .collect()
    }
}

/// The open ring.
#[derive(Clone, Copy, Debug)]
pub(crate) struct QuickMenu {
    /// Its middle on screen.
    pub(crate) center: egui::Pos2,
    /// The project tab it opened over; another tab closes it.
    tab: usize,
    /// Closed by a press, which the canvas mustn't see until it lifts.
    closing: bool,
}

/// A command's name, short enough for a slot in the middle of the ring.
fn short(command: &commands::Command) -> &'static str {
    match command.id {
        "view.flip" => "Flip",
        "edit.transform" => "Transform",
        "selection.none" => "Deselect",
        "selection.all" => "Select all",
        "layer.new" => "New layer",
        _ => command
            .label
            .split([' ', '…', '/'])
            .find(|word| !word.is_empty())
            .unwrap_or(command.label),
    }
}

impl Dudel {
    /// The ring's reach from the tip, past which a press closes it.
    fn quick_menu_reach(&self) -> f32 {
        let favorites = self.brushes.iter().filter(|b| b.favorite).count();
        reach(favorites.min(MOST_BRUSHES))
    }

    /// Whether the canvas leaves pen input to the quick menu.
    pub(crate) fn quick_menu_blocks_canvas(&self) -> bool {
        self.quick_menu.is_some()
    }

    /// Why the ring can't open now, if it can't: it would get in the way.
    fn quick_menu_unavailable(&self) -> bool {
        self.stroking
            || self.text_edit.is_some()
            || self.text_drag.is_some()
            || self.affine_preview.is_some()
            || self.move_drag.is_some()
            || self.selection_drag.is_some()
    }

    /// Open the ring at `at`, kept inside `area` so nothing hangs off it.
    pub(crate) fn open_quick_menu(&mut self, at: egui::Pos2, area: egui::Rect) {
        let room = area.shrink(self.quick_menu_reach());
        let center = if room.is_positive() {
            room.clamp(at)
        } else {
            area.center()
        };
        self.quick_menu = Some(QuickMenu {
            center,
            tab: self.tabs.active,
            closing: false,
        });
    }

    /// Close the ring. A press that closed it stays the ring's until it lifts.
    pub(crate) fn close_quick_menu(&mut self, by_press: bool) {
        match &mut self.quick_menu {
            Some(menu) if by_press => menu.closing = true,
            _ => self.quick_menu = None,
        }
    }

    /// The side button or a right-click on the canvas opens or closes the
    /// ring; a press outside it closes it. `canvas` is the canvas's layer.
    pub(crate) fn quick_menu_input(
        &mut self,
        ctx: &egui::Context,
        area: egui::Rect,
        canvas: egui::LayerId,
    ) {
        if let Some(menu) = self.quick_menu {
            if menu.tab != self.tabs.active {
                self.quick_menu = None;
                return;
            }
            // A closing press is over once nothing is held.
            if menu.closing {
                if !ctx.input(|i| i.pointer.any_down()) {
                    self.quick_menu = None;
                }
                return;
            }
        }
        let presses: Vec<(egui::PointerButton, egui::Pos2)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        ..
                    } => Some((*button, *pos)),
                    _ => None,
                })
                .collect()
        });
        for (button, at) in presses {
            let on_canvas = area.contains(at) && ctx.layer_id_at(at) == Some(canvas);
            match (self.quick_menu, button) {
                (Some(_), egui::PointerButton::Secondary) => self.close_quick_menu(true),
                (Some(menu), _) if at.distance(menu.center) > self.quick_menu_reach() => {
                    self.close_quick_menu(true);
                }
                (None, egui::PointerButton::Secondary)
                    if on_canvas && !self.quick_menu_unavailable() =>
                {
                    self.open_quick_menu(at, area);
                }
                _ => {}
            }
        }
    }

    /// The ring, while it is open.
    pub(crate) fn quick_menu_ui(&mut self, ctx: &egui::Context) {
        let Some(menu) = self.quick_menu.filter(|m| !m.closing) else {
            return;
        };
        let center = menu.center;
        let favorites: Vec<usize> = (0..self.brushes.len())
            .filter(|&i| self.brushes[i].favorite)
            .take(MOST_BRUSHES)
            .collect();
        let (ring, reach) = (brush_ring(favorites.len()), reach(favorites.len()));
        let colors: Vec<[u8; 3]> = self
            .recent_colors
            .iter()
            .copied()
            .take(MOST_COLORS)
            .collect();
        let actions = self.quick_menu_prefs.commands();
        let mut chosen_brush = None;
        let mut chosen_color = None;
        let mut chosen_action = None;
        egui::Area::new(egui::Id::new("pen quick menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(center - egui::vec2(reach, reach))
            .constrain(false)
            .show(ctx, |ui| {
                let (whole, _) =
                    ui.allocate_exact_size(egui::vec2(reach, reach) * 2.0, egui::Sense::hover());
                let visuals = ui.visuals().clone();
                let painter = ui.painter().clone();
                painter.circle_filled(center, reach - 4.0, visuals.window_fill);
                painter.circle_stroke(center, reach - 4.0, visuals.window_stroke);
                let around = |index: usize, count: usize, radius: f32| {
                    // From the top, clockwise.
                    let turn = std::f32::consts::TAU * index as f32 / count.max(1) as f32;
                    center + radius * egui::vec2(turn.sin(), -turn.cos())
                };
                if favorites.is_empty() {
                    ui.put(
                        egui::Rect::from_center_size(
                            center - egui::vec2(0.0, ring),
                            egui::vec2(220.0, 40.0),
                        ),
                        egui::Label::new(
                            egui::RichText::new("Star brushes in the library to put them here")
                                .small(),
                        ),
                    );
                }
                for (slot, &index) in favorites.iter().enumerate() {
                    let brush = &self.brushes[index];
                    let at = around(slot, favorites.len(), ring);
                    let selected = index == self.selected_brush;
                    if ui
                        .put(
                            egui::Rect::from_center_size(at, BRUSH_SIZE),
                            egui::Button::new(brush.name.as_str())
                                .selected(selected)
                                .corner_radius(16.0)
                                .wrap_mode(egui::TextWrapMode::Truncate),
                        )
                        .on_hover_text(&brush.name)
                        .clicked()
                    {
                        chosen_brush = Some(index);
                    }
                }
                for (slot, &color) in colors.iter().enumerate() {
                    let at = around(slot, colors.len(), COLOR_RING);
                    let rect = egui::Rect::from_center_size(at, egui::Vec2::splat(COLOR_SIZE));
                    let label = format!("Color #{}", crate::color_wheel::hex(color));
                    let response = ui
                        .interact(rect, ui.id().with(("color", slot)), egui::Sense::click())
                        .on_hover_text(&label);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label)
                    });
                    let ring = if color == self.color {
                        visuals.selection.stroke
                    } else if response.hovered() {
                        visuals.widgets.hovered.fg_stroke
                    } else {
                        visuals.widgets.inactive.bg_stroke
                    };
                    painter.circle_filled(at, COLOR_SIZE / 2.0, crate::color_wheel::color32(color));
                    painter.circle_stroke(at, COLOR_SIZE / 2.0 + 1.0, ring);
                    if response.clicked() {
                        chosen_color = Some(color);
                    }
                }
                // The commands in a small square in the middle.
                for (slot, command) in actions.iter().enumerate() {
                    let offset = egui::vec2(
                        if slot % 2 == 0 { -0.5 } else { 0.5 } * (ACTION_SIZE.x + 4.0),
                        if slot < 2 { -0.5 } else { 0.5 } * (ACTION_SIZE.y + 4.0),
                    );
                    let reason = self.command_disabled(command.action);
                    let rect = egui::Rect::from_center_size(center + offset, ACTION_SIZE);
                    let button = egui::Button::new(short(command)).selected(match command.action {
                        commands::Action::FlipView => self.mirror,
                        commands::Action::Tool(tool) => self.tool == tool,
                        _ => false,
                    });
                    let response = ui
                        .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.add_enabled_ui(reason.is_none(), |ui| ui.put(rect, button))
                                .inner
                        })
                        .inner
                        .on_hover_text(self.command_hint(command.action, command.label));
                    let response = match reason {
                        Some(reason) => response.on_disabled_hover_text(reason),
                        None => response,
                    };
                    if response.clicked() {
                        chosen_action = Some(command.action);
                    }
                }
                let _ = whole;
            });
        if let Some(index) = chosen_brush {
            self.choose_brush(index);
        }
        if let Some(color) = chosen_color {
            self.color = color;
        }
        if let Some(action) = chosen_action {
            self.execute_command(action);
        }
        if chosen_brush.is_some() || chosen_color.is_some() || chosen_action.is_some() {
            self.close_quick_menu(true);
        }
    }

    /// Settings: which commands sit in the middle of the ring.
    pub(crate) fn quick_menu_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "The pen's side button or a right-click on the canvas opens a ring with your \
             starred brushes, recent colors and these commands.",
        );
        let mut actions = self.quick_menu_prefs.actions.clone();
        actions.resize(ACTION_SLOTS, String::new());
        for (slot, id) in actions.iter_mut().enumerate() {
            let shown = commands::COMMANDS
                .iter()
                .find(|c| c.id == *id)
                .map_or("Nothing", |c| c.label);
            egui::ComboBox::from_id_salt(("quick menu slot", slot))
                .selected_text(shown)
                .width(240.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(id, String::new(), "Nothing");
                    for command in commands::COMMANDS {
                        ui.selectable_value(id, command.id.to_owned(), command.label);
                    }
                });
        }
        actions.retain(|id| !id.is_empty());
        if actions != self.quick_menu_prefs.actions {
            self.quick_menu_prefs.actions = actions;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_saved_slots_are_known_commands() {
        let defaults = QuickMenuPrefs::default().sanitized();
        assert_eq!(defaults.actions.len(), ACTION_SLOTS, "every default exists");
        let saved = QuickMenuPrefs {
            actions: [
                "edit.undo",
                "no.such",
                "edit.redo",
                "tool.brush",
                "tool.fill",
                "view.fit",
            ]
            .map(String::from)
            .to_vec(),
        }
        .sanitized();
        assert_eq!(
            saved.actions,
            ["edit.undo", "edit.redo", "tool.brush", "tool.fill"]
        );
    }

    #[test]
    fn slot_names_are_short() {
        for command in commands::COMMANDS {
            assert!(short(command).chars().count() <= 10, "{}", short(command));
        }
    }
}
