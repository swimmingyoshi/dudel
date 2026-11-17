//! The pen quick menu: the pen's side button (or a right-click) on the canvas
//! opens a ring around the tip with the starred brushes on its outer band,
//! recent colors on the middle one and a few chosen commands in the four
//! corners around an open middle. Tap anything, or hold the side button,
//! slide onto it and let go. A press outside, Esc or the side button again
//! closes it.
//!
//! The ring is workspace UI, never artwork: its commands are the ordinary
//! command list's and its brushes the library's favorites, so it keeps no
//! second copy of either. While it is open the canvas takes no pen input, and
//! a press that closes it is swallowed until it lifts, so it never paints.
//!
//! It is drawn as thin bands with the middle left open, so the canvas under
//! the tip stays in view; every item takes its whole slice of a band, so the
//! targets are large for a pen.
use crate::{Dudel, commands, egui};
use std::f32::consts::{FRAC_PI_4, PI, TAU};

/// Command slots in the corners of the middle band.
pub(crate) const ACTION_SLOTS: usize = 4;
/// Brushes on the outer band and colors on the middle one, at most.
const MOST_BRUSHES: usize = 10;
const MOST_COLORS: usize = 8;
/// The bands, from the tip outwards: the open middle, then the commands,
/// the colors and the brushes, with a narrow gap between bands.
const OPEN: f32 = 22.0;
const ACTIONS_OUT: f32 = 58.0;
const COLORS_IN: f32 = 61.0;
const COLORS_OUT: f32 = 85.0;
const BRUSHES_IN: f32 = 88.0;
const BRUSHES_OUT: f32 = 138.0;
/// The middle of the color band, where the swatches sit.
pub(crate) const COLOR_RING: f32 = (COLORS_IN + COLORS_OUT) / 2.0;
const SWATCH: f32 = 9.0;
/// The ring's reach from the tip, past which a press closes it.
const REACH: f32 = BRUSHES_OUT + 6.0;
/// Where each command slot sits, as a turn from the top, clockwise: the
/// first two at the upper corners, the others below them.
const ACTION_TURNS: [f32; ACTION_SLOTS] =
    [-FRAC_PI_4, FRAC_PI_4, -3.0 * FRAC_PI_4, 3.0 * FRAC_PI_4];
/// How far a held side button must travel before letting go picks.
const SLIDE: f32 = 12.0;

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
    /// Where the press that opened it was, which may be off the middle near
    /// the canvas's edges: letting go there picks nothing.
    opened_at: egui::Pos2,
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

/// Something on the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Action(usize),
    Color(usize),
    Brush(usize),
}

/// The turn of `offset` from the top, clockwise, in 0..TAU.
fn turn_of(offset: egui::Vec2) -> f32 {
    offset.x.atan2(-offset.y).rem_euclid(TAU)
}

/// How far apart two turns are, the short way round.
fn apart(a: f32, b: f32) -> f32 {
    ((a - b + PI).rem_euclid(TAU) - PI).abs()
}

/// The point `radius` out from `center` at `turn` from the top, clockwise.
fn at_turn(center: egui::Pos2, radius: f32, turn: f32) -> egui::Pos2 {
    center + radius * egui::vec2(turn.sin(), -turn.cos())
}

/// Which of `count` even slices, the first centered at the top, holds `turn`.
fn slice(turn: f32, count: usize) -> usize {
    let width = TAU / count.max(1) as f32;
    ((turn / width).round() as usize) % count.max(1)
}

/// The item under `offset` from the ring's middle, if any. Gaps between
/// bands belong to the nearer band, so no tap falls between targets.
fn slot_at(offset: egui::Vec2, actions: usize, colors: usize, brushes: usize) -> Option<Slot> {
    let distance = offset.length();
    let turn = turn_of(offset);
    let gap = (COLORS_IN - ACTIONS_OUT) / 2.0;
    if (OPEN..ACTIONS_OUT + gap).contains(&distance) {
        let slot = (0..ACTION_SLOTS).find(|&s| apart(turn, ACTION_TURNS[s]) <= FRAC_PI_4)?;
        return (slot < actions).then_some(Slot::Action(slot));
    }
    if (ACTIONS_OUT + gap..BRUSHES_IN - gap).contains(&distance) {
        return (colors > 0).then(|| Slot::Color(slice(turn, colors)));
    }
    if (BRUSHES_IN - gap..=BRUSHES_OUT + 2.0).contains(&distance) {
        return (brushes > 0).then(|| Slot::Brush(slice(turn, brushes)));
    }
    None
}

/// An arc of `radius` from turn `from` to `to`, as a line to stroke.
fn arc(center: egui::Pos2, radius: f32, from: f32, to: f32) -> Vec<egui::Pos2> {
    let steps = (((to - from).abs() / (TAU / 120.0)).ceil() as usize).max(2);
    (0..=steps)
        .map(|i| at_turn(center, radius, from + (to - from) * i as f32 / steps as f32))
        .collect()
}

/// Fill the part of the band between `inner` and `outer` that runs from turn
/// `from` to `to`. A very wide stroke doesn't come out solid in egui, so this
/// is a mesh of triangles, with hairlines along its curved edges to smooth
/// them.
fn fill_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    (inner, outer): (f32, f32),
    (from, to): (f32, f32),
    color: egui::Color32,
) {
    let inside = arc(center, inner + 0.5, from, to);
    let outside = arc(center, outer - 0.5, from, to);
    let mut mesh = egui::Mesh::default();
    for (a, b) in inside.iter().zip(&outside) {
        mesh.colored_vertex(*a, color);
        mesh.colored_vertex(*b, color);
    }
    for i in 0..inside.len() as u32 - 1 {
        let at = i * 2;
        mesh.add_triangle(at, at + 1, at + 2);
        mesh.add_triangle(at + 1, at + 3, at + 2);
    }
    painter.add(mesh);
    let edge = egui::Stroke::new(1.0_f32, color);
    painter.add(egui::epaint::PathShape::line(inside, edge));
    painter.add(egui::epaint::PathShape::line(outside, edge));
}

/// Fill the slice of the band between `inner` and `outer` that runs from
/// turn `from` to `to`, leaving a hairline at its ends.
fn fill_slice(
    painter: &egui::Painter,
    center: egui::Pos2,
    (inner, outer): (f32, f32),
    (from, to): (f32, f32),
    color: egui::Color32,
) {
    let inset = if to - from >= TAU - 0.001 {
        0.0
    } else {
        1.5 / ((inner + outer) / 2.0)
    };
    fill_arc(
        painter,
        center,
        (inner + 1.0, outer - 1.0),
        (from + inset, to - inset),
        color,
    );
}

/// Draw a band: its fill and the lines between its slices at `bounds`
/// (turns). The narrow gaps between bands show the canvas, which keeps them
/// apart without outlines.
fn band(
    painter: &egui::Painter,
    center: egui::Pos2,
    (inner, outer): (f32, f32),
    bounds: impl IntoIterator<Item = f32>,
    visuals: &egui::Visuals,
) {
    let [r, g, b, _] = visuals.window_fill.to_opaque().to_array();
    let fill = egui::Color32::from_rgba_unmultiplied(r, g, b, 244);
    fill_arc(painter, center, (inner, outer), (0.0, TAU), fill);
    let line = egui::Stroke::new(1.0_f32, visuals.window_stroke.color.gamma_multiply(0.5));
    for turn in bounds {
        painter.line_segment(
            [
                at_turn(center, inner + 3.0, turn),
                at_turn(center, outer - 3.0, turn),
            ],
            line,
        );
    }
}

/// `text` laid out in at most two rows of `width`, ending in … if longer.
fn label(
    painter: &egui::Painter,
    text: &str,
    size: f32,
    width: f32,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        egui::FontId::proportional(size),
        color,
        width,
    );
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    painter.layout_job(job)
}

/// Paint `galley`, laid out centered, with its middle at `at`.
fn put(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, at: egui::Pos2) {
    let top_left = at - galley.rect.center().to_vec2();
    painter.galley(top_left, galley, egui::Color32::PLACEHOLDER);
}

impl Dudel {
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
        let room = area.shrink(REACH);
        let center = if room.is_positive() {
            room.clamp(at)
        } else {
            area.center()
        };
        self.quick_menu = Some(QuickMenu {
            center,
            opened_at: at,
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
                (Some(menu), _) if at.distance(menu.center) > REACH => {
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
        let colors: Vec<[u8; 3]> = self
            .recent_colors
            .iter()
            .copied()
            .take(MOST_COLORS)
            .collect();
        let actions = self.quick_menu_prefs.commands();
        let disabled: Vec<Option<&'static str>> = actions
            .iter()
            .map(|command| self.command_disabled(command.action))
            .collect();
        let hit = |at: egui::Pos2| {
            slot_at(at - center, actions.len(), colors.len(), favorites.len())
                .filter(|slot| !matches!(slot, Slot::Action(i) if disabled[*i].is_some()))
        };
        // Holding the side button, sliding onto an item and letting go picks
        // it, as long as the pen has moved from where it opened the ring.
        let released = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    ..
                } if pos.distance(menu.opened_at) > SLIDE => Some(*pos),
                _ => None,
            })
        });
        let mut chosen = released.and_then(hit);
        egui::Area::new(egui::Id::new("pen quick menu"))
            .order(egui::Order::Foreground)
            // At once under the pen, rather than fading in.
            .fade_in(false)
            .fixed_pos(center - egui::Vec2::splat(REACH))
            .constrain(false)
            .show(ctx, |ui| {
                let (_, response) =
                    ui.allocate_exact_size(egui::Vec2::splat(REACH * 2.0), egui::Sense::click());
                let visuals = ui.visuals().clone();
                let painter = ui.painter().clone();
                // The slot under the pen, by its position: hovering, or
                // sliding with the side button held.
                let pointed = ui
                    .input(|i| i.pointer.hover_pos())
                    .filter(|_| response.hovered() || response.is_pointer_button_down_on())
                    .and_then(hit)
                    .or(chosen);
                let hovered_fill = visuals.widgets.hovered.weak_bg_fill;
                let selected_fill = visuals.selection.bg_fill.gamma_multiply(0.55);
                let text = visuals.text_color();
                let strong = visuals.strong_text_color();
                // A faint shadow, so the ring reads on any artwork.
                painter.circle_stroke(
                    center,
                    BRUSHES_OUT + 2.5,
                    egui::Stroke::new(5.0_f32, egui::Color32::from_black_alpha(28)),
                );

                // Commands: four corners around the open middle.
                band(
                    &painter,
                    center,
                    (OPEN, ACTIONS_OUT),
                    (0..ACTION_SLOTS).map(|i| i as f32 * TAU / 4.0),
                    &visuals,
                );
                for (slot, command) in actions.iter().enumerate() {
                    let turn = ACTION_TURNS[slot];
                    let selected = match command.action {
                        commands::Action::FlipView => self.mirror,
                        commands::Action::Tool(tool) => self.tool == tool,
                        _ => false,
                    };
                    let fill = if pointed == Some(Slot::Action(slot)) {
                        Some(hovered_fill)
                    } else {
                        selected.then_some(selected_fill)
                    };
                    if let Some(fill) = fill {
                        fill_slice(
                            &painter,
                            center,
                            (OPEN, ACTIONS_OUT),
                            (turn - FRAC_PI_4, turn + FRAC_PI_4),
                            fill,
                        );
                    }
                    let color = if disabled[slot].is_some() {
                        visuals.weak_text_color()
                    } else if selected || pointed == Some(Slot::Action(slot)) {
                        strong
                    } else {
                        text
                    };
                    let galley = label(&painter, short(command), 12.0, 52.0, color);
                    put(
                        &painter,
                        galley,
                        at_turn(center, (OPEN + ACTIONS_OUT) / 2.0 + 1.0, turn),
                    );
                }

                // Recent colors, one slice each.
                if !colors.is_empty() {
                    let width = TAU / colors.len() as f32;
                    band(&painter, center, (COLORS_IN, COLORS_OUT), [], &visuals);
                    for (slot, &color) in colors.iter().enumerate() {
                        let turn = slot as f32 * width;
                        if pointed == Some(Slot::Color(slot)) {
                            fill_slice(
                                &painter,
                                center,
                                (COLORS_IN, COLORS_OUT),
                                (turn - width / 2.0, turn + width / 2.0),
                                hovered_fill,
                            );
                        }
                        let at = at_turn(center, COLOR_RING, turn);
                        painter.circle_filled(at, SWATCH, crate::color_wheel::color32(color));
                        let ring = if color == self.color {
                            egui::Stroke::new(2.0_f32, visuals.selection.stroke.color)
                        } else {
                            egui::Stroke::new(1.0_f32, visuals.window_stroke.color)
                        };
                        painter.circle_stroke(at, SWATCH + 1.0, ring);
                    }
                }

                // Starred brushes, one slice each, or a hint to star some.
                let brush_band = (BRUSHES_IN, BRUSHES_OUT);
                let middle = (BRUSHES_IN + BRUSHES_OUT) / 2.0;
                if favorites.is_empty() {
                    band(&painter, center, brush_band, [], &visuals);
                    let galley = label(
                        &painter,
                        "Star brushes in the library to put them here",
                        11.5,
                        120.0,
                        visuals.weak_text_color(),
                    );
                    put(&painter, galley, at_turn(center, middle, 0.0));
                } else {
                    let width = TAU / favorites.len() as f32;
                    band(
                        &painter,
                        center,
                        brush_band,
                        (0..favorites.len())
                            .filter(|_| favorites.len() > 1)
                            .map(|i| (i as f32 + 0.5) * width),
                        &visuals,
                    );
                    for (slot, &index) in favorites.iter().enumerate() {
                        let turn = slot as f32 * width;
                        let selected = index == self.selected_brush;
                        let pointed_here = pointed == Some(Slot::Brush(slot));
                        let fill = if pointed_here {
                            Some(hovered_fill)
                        } else {
                            selected.then_some(selected_fill)
                        };
                        if let Some(fill) = fill {
                            fill_slice(
                                &painter,
                                center,
                                brush_band,
                                (turn - width / 2.0, turn + width / 2.0),
                                fill,
                            );
                        }
                        let color = if selected || pointed_here {
                            strong
                        } else {
                            text
                        };
                        // Room across the slice, but never past the band at
                        // its sides.
                        let room = (middle * width * 0.8).clamp(56.0, 68.0);
                        let galley = label(&painter, &self.brushes[index].name, 12.5, room, color);
                        put(&painter, galley, at_turn(center, middle, turn));
                    }
                }

                // A dot where the middle is, to aim by.
                painter.circle_stroke(
                    center,
                    2.5,
                    egui::Stroke::new(1.0_f32, visuals.text_color().gamma_multiply(0.6)),
                );

                // The pointed item's full name or what it does.
                let tip = match pointed {
                    Some(Slot::Brush(slot)) => Some(self.brushes[favorites[slot]].name.clone()),
                    Some(Slot::Color(slot)) => {
                        Some(format!("Color #{}", crate::color_wheel::hex(colors[slot])))
                    }
                    Some(Slot::Action(slot)) => {
                        let command = actions[slot];
                        Some(self.command_hint(command.action, command.label))
                    }
                    None => None,
                };
                let response = match tip {
                    Some(tip) if response.hovered() => response.on_hover_text(tip),
                    _ => response,
                };
                if response.clicked()
                    && let Some(at) = response.interact_pointer_pos()
                {
                    chosen = hit(at);
                }
            });
        match chosen {
            Some(Slot::Brush(slot)) => self.choose_brush(favorites[slot]),
            Some(Slot::Color(slot)) => self.color = colors[slot],
            Some(Slot::Action(slot)) => self.execute_command(actions[slot].action),
            None => {}
        }
        if chosen.is_some() {
            self.close_quick_menu(true);
        }
    }

    /// Settings: which commands sit in the middle of the ring.
    pub(crate) fn quick_menu_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "The pen's side button or a right-click on the canvas opens a ring with your \
             starred brushes, recent colors and these commands. Tap one, or hold the \
             side button, slide onto one and let go.",
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

    #[test]
    fn every_point_of_a_band_belongs_to_its_slice() {
        let at = |radius: f32, degrees: f32| {
            at_turn(egui::Pos2::ZERO, radius, degrees.to_radians()).to_vec2()
        };
        // The open middle is the canvas's.
        assert_eq!(slot_at(at(10.0, 0.0), 4, 8, 10), None);
        // Commands: upper left and right first, as in reading order.
        assert_eq!(slot_at(at(40.0, -45.0), 4, 8, 10), Some(Slot::Action(0)));
        assert_eq!(slot_at(at(40.0, 10.0), 4, 8, 10), Some(Slot::Action(1)));
        assert_eq!(slot_at(at(40.0, 200.0), 4, 8, 10), Some(Slot::Action(2)));
        assert_eq!(slot_at(at(40.0, 100.0), 4, 8, 10), Some(Slot::Action(3)));
        assert_eq!(slot_at(at(40.0, 100.0), 2, 8, 10), None, "an empty corner");
        // Colors and brushes from the top, clockwise; gaps go to a band.
        assert_eq!(slot_at(at(COLOR_RING, 0.0), 4, 8, 10), Some(Slot::Color(0)));
        assert_eq!(slot_at(at(59.5, 44.0), 4, 8, 10), Some(Slot::Color(1)));
        assert_eq!(slot_at(at(86.0, 350.0), 4, 8, 10), Some(Slot::Color(0)));
        assert_eq!(slot_at(at(87.0, 350.0), 4, 8, 10), Some(Slot::Brush(0)));
        assert_eq!(slot_at(at(120.0, 179.0), 4, 8, 10), Some(Slot::Brush(5)));
        assert_eq!(slot_at(at(120.0, 179.0), 4, 0, 0), None);
        assert_eq!(slot_at(at(REACH, 0.0), 4, 8, 10), None, "past the ring");
    }
}
