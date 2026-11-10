//! Tool shortcut taps commit; holds return to the prior permanent tool.
//! At most one entry per tool key is retained. Space/R/Alt remain independent.
use crate::{Dudel, egui, tools::Tool};

const HOLD_SECONDS: f64 = 0.5;
struct Held {
    tool: Tool,
    since: f64,
    /// Pressed while this tool was already the permanent one.
    again: bool,
}
pub struct ToolShift {
    base: Tool,
    held: Vec<Held>,
    blocked: [bool; Tool::ALL.len()],
}
impl Default for ToolShift {
    fn default() -> Self {
        Self {
            base: Tool::Brush,
            held: Vec::new(),
            blocked: [false; Tool::ALL.len()],
        }
    }
}
impl ToolShift {
    fn index(tool: Tool) -> usize {
        Tool::ALL
            .iter()
            .position(|candidate| *candidate == tool)
            .unwrap()
    }
    fn current(&self) -> Tool {
        self.held.last().map_or(self.base, |held| held.tool)
    }
    fn permanent(&mut self, tool: Tool) {
        for held in &self.held {
            self.blocked[Self::index(held.tool)] = true;
        }
        self.held.clear();
        self.base = tool;
    }
    fn press(&mut self, tool: Tool, now: f64) {
        if !self.blocked[Self::index(tool)] && !self.held.iter().any(|held| held.tool == tool) {
            let again = self.held.is_empty() && self.base == tool;
            self.held.push(Held {
                tool,
                since: now,
                again,
            });
        }
    }
    /// Returns true for a tap on the tool that was already current.
    fn release(&mut self, tool: Tool, now: f64) -> bool {
        self.blocked[Self::index(tool)] = false;
        if let Some(index) = self.held.iter().position(|held| held.tool == tool) {
            let held = self.held.remove(index);
            if (now - held.since).max(0.0) <= HOLD_SECONDS {
                self.base = held.tool;
                return held.again;
            }
        }
        false
    }
    fn cancel(&mut self, down: [bool; Tool::ALL.len()]) {
        self.held.clear();
        self.blocked = down;
    }
}
impl Dudel {
    pub(crate) fn select_tool(&mut self, tool: Tool) {
        // Choosing a tool returns from reference editing to the artwork.
        self.deselect_reference();
        self.tool_shift.permanent(tool);
        self.tool = tool;
        // A scripted tool lasts until another tool is chosen.
        self.scripted_tool = None;
        self.sync_eraser_slot();
    }
    /// While the Eraser tool is active, its tip (Eraser tip mode) or just its
    /// size (Brush shape mode) sits in the brush slot, so size keys, sliders
    /// and the cursor edit the eraser and the pen keeps its own size.
    pub(crate) fn sync_eraser_slot(&mut self) {
        use crate::tools::EraserSwap;
        // Never exchange under a stroke; the next frame catches up.
        if self.stroking {
            return;
        }
        // A pen flipped to its eraser end counts as the Eraser, so size keys
        // and sliders resize what the pen is about to erase with.
        let erasing = self.tool == Tool::Eraser || self.pen_flipped();
        let want = erasing.then_some(if self.eraser_own {
            EraserSwap::Tip
        } else {
            EraserSwap::Size
        });
        if want != self.eraser_swap {
            self.unswap_eraser();
            if let Some(kind) = want {
                self.exchange_eraser(kind);
                self.eraser_swap = Some(kind);
            }
        }
    }
    /// The pen hovers or touches with its eraser end while a paint tool is active.
    pub(crate) fn pen_flipped(&self) -> bool {
        self.tool == Tool::Brush && self.input.eraser_in_range()
    }
    fn exchange_eraser(&mut self, kind: crate::tools::EraserSwap) {
        match kind {
            crate::tools::EraserSwap::Tip => {
                std::mem::swap(&mut self.brush, &mut self.eraser_brush)
            }
            crate::tools::EraserSwap::Size => {
                std::mem::swap(&mut self.brush.size, &mut self.eraser_brush.size)
            }
        }
    }
    /// Put the paint brush and eraser back in their own slots.
    pub(crate) fn unswap_eraser(&mut self) {
        if let Some(kind) = self.eraser_swap.take() {
            self.exchange_eraser(kind);
        }
    }
    /// The paint brush as it is when the Eraser tool is not active.
    pub(crate) fn paint_brush(&self) -> std::borrow::Cow<'_, crate::brush::Brush> {
        use std::borrow::Cow;
        match self.eraser_swap {
            None => Cow::Borrowed(&self.brush),
            Some(crate::tools::EraserSwap::Tip) => Cow::Borrowed(&self.eraser_brush),
            Some(crate::tools::EraserSwap::Size) => Cow::Owned(crate::brush::Brush {
                size: self.eraser_brush.size,
                ..self.brush.clone()
            }),
        }
    }
    /// The stored eraser tip, whose size is the eraser size in both modes.
    pub(crate) fn eraser_tip(&self) -> std::borrow::Cow<'_, crate::brush::Brush> {
        use std::borrow::Cow;
        match self.eraser_swap {
            None => Cow::Borrowed(&self.eraser_brush),
            Some(crate::tools::EraserSwap::Tip) => Cow::Borrowed(&self.brush),
            Some(crate::tools::EraserSwap::Size) => Cow::Owned(crate::brush::Brush {
                size: self.brush.size,
                ..self.eraser_brush.clone()
            }),
        }
    }
    /// What erasing paints with now: the eraser tip, or the brush's shape at
    /// the eraser size. Used for the Eraser tool and the pen's eraser end.
    pub(crate) fn eraser_for_stroke(&self) -> std::borrow::Cow<'_, crate::brush::Brush> {
        use std::borrow::Cow;
        if self.eraser_swap.is_some() {
            return Cow::Borrowed(&self.brush);
        }
        if self.eraser_own {
            Cow::Borrowed(&self.eraser_brush)
        } else {
            Cow::Owned(crate::brush::Brush {
                size: self.eraser_brush.size,
                ..self.brush.clone()
            })
        }
    }
    /// Switch between erasing with the brush shape and the eraser tip.
    pub(crate) fn set_eraser_own(&mut self, own: bool) {
        self.eraser_own = own;
        self.sync_eraser_slot();
        self.status = if own {
            "Eraser uses its own tip"
        } else {
            "Eraser uses the current brush's shape"
        }
        .into();
    }
    pub(crate) fn update_tool_shift(&mut self, ctx: &egui::Context) {
        if self.tool_shift.current() != self.tool {
            self.tool_shift.permanent(self.tool);
        }
        let (now, focused, events, down) = ctx.input(|input| {
            (
                input.time,
                input.focused,
                input.events.clone(),
                Tool::ALL.map(|tool| {
                    self.shortcuts
                        .effective(crate::commands::Action::Tool(tool))
                        .iter()
                        .any(|c| input.key_down(c.key))
                }),
            )
        });
        let opening_new = events.iter().any(|event| {
            self.shortcuts
                .event_matches(crate::commands::Action::New, event)
                || self
                    .shortcuts
                    .event_matches(crate::commands::Action::Search, event)
        });
        let enabled = focused
            && !opening_new
            && !ctx.wants_keyboard_input()
            && !egui::Popup::is_any_open(ctx)
            && self.renaming.is_none()
            && self.unsaved_action.is_none()
            && !self.new_canvas
            && !self.command_search.open
            && self.shortcut_editor.capture.is_none()
            && !self.shortcut_editor.block_frame
            && !self.input_settings
            && !self.recovery_window;
        let enabled = enabled
            && self.canvas_resize.is_none()
            && self.canvas_edit_ticket.is_none()
            && self.adjustment.is_none()
            && self.adjustment_ticket.is_none();
        if !enabled {
            self.tool_shift.cancel(down);
        }
        for event in events {
            if let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            {
                for tool in Tool::ALL {
                    if !self
                        .shortcuts
                        .effective(crate::commands::Action::Tool(tool))
                        .iter()
                        .any(|c| c.key == key && (!pressed || c.matches(key, modifiers)))
                    {
                        continue;
                    }
                    if !pressed {
                        if self.tool_shift.release(tool, now) && enabled {
                            // Tapping the current tool's key steps through its variants.
                            match tool {
                                Tool::Brush => self.select_brush_group(self.brush_group.next()),
                                Tool::Fill => {
                                    self.gradient.active = !self.gradient.active;
                                    self.status = if self.gradient.active {
                                        "Fill tool: gradient"
                                    } else {
                                        "Fill tool: bucket"
                                    }
                                    .into();
                                }
                                Tool::Lasso => {
                                    self.lasso.erase = !self.lasso.erase;
                                    self.status = if self.lasso.erase {
                                        "Lasso erases"
                                    } else {
                                        "Lasso fills"
                                    }
                                    .into();
                                }
                                _ => {}
                            }
                        }
                    } else if enabled && !repeat {
                        self.tool_shift.press(tool, now);
                        self.shortcuts
                            .consume(ctx, crate::commands::Action::Tool(tool));
                    }
                }
            }
        }
        self.tool = self.tool_shift.current();
        self.sync_eraser_slot();
    }

    /// Deliver cancellation with the old tool before ending its input lifetime.
    /// Completed contacts before Cancel retain their FIFO order and old settings.
    pub(crate) fn canvas_after_tool_change(
        &mut self,
        ctx: &egui::Context,
        packets: &[crate::input::Packet],
        previous: Tool,
    ) {
        let next = self.tool;
        if next != previous
            && packets
                .iter()
                .any(|packet| matches!(packet, crate::input::Packet::Cancel))
        {
            self.tool = previous;
            self.canvas(ctx, packets);
            self.close_input_gestures();
            self.tool = next;
        } else {
            if next != previous {
                self.close_input_gestures();
            }
            self.canvas(ctx, packets);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(tool: Tool, pressed: bool, repeat: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key: tool.key(),
            physical_key: None,
            pressed,
            repeat,
            modifiers,
        }
    }
    fn frame(
        app: &mut Dudel,
        ctx: &egui::Context,
        time: f64,
        focused: bool,
        events: Vec<egui::Event>,
        packets: &[crate::input::Packet],
    ) {
        let raw = egui::RawInput {
            time: Some(time),
            focused,
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            let previous = app.tool;
            app.update_tool_shift(ctx);
            if app.unsaved_action.is_none() {
                app.shortcuts(ctx);
            }
            app.canvas_after_tool_change(ctx, packets, previous);
        });
    }
    fn app() -> Dudel {
        Dudel {
            doc: crate::document::Document::new(64, 64),
            ..Default::default()
        }
    }

    fn tap(app: &mut Dudel, ctx: &egui::Context, tool: Tool, time: f64) {
        let none = egui::Modifiers::NONE;
        frame(
            app,
            ctx,
            time,
            true,
            vec![key(tool, true, false, none)],
            &[],
        );
        frame(
            app,
            ctx,
            time + 0.1,
            true,
            vec![key(tool, false, false, none)],
            &[],
        );
    }
    #[test]
    fn tapping_the_current_tool_cycles_brush_groups_and_lasso_mode_but_holds_do_not() {
        use crate::brush::BrushGroup;
        let ctx = egui::Context::default();
        let mut app = app();
        app.brushes = crate::workspace::default_brushes();
        app.choose_brush(0);
        assert_eq!(app.brush_group, BrushGroup::Pen);
        // Tweaks survive a round trip through the other groups.
        app.brush.size = 77.0;
        tap(&mut app, &ctx, Tool::Brush, 0.0);
        assert_eq!(app.brush_group, BrushGroup::Brush);
        assert_eq!(app.brush.group(), BrushGroup::Brush);
        for (time, group) in [(1.0, BrushGroup::Airbrush), (2.0, BrushGroup::Pencil)] {
            tap(&mut app, &ctx, Tool::Brush, time);
            assert_eq!(app.brush_group, group);
            assert_eq!(app.brush.group(), group);
        }
        tap(&mut app, &ctx, Tool::Brush, 3.0);
        assert_eq!(app.brush_group, BrushGroup::Pen);
        assert_eq!(app.brush.size, 77.0);
        // From another tool, the first tap only returns to the brush.
        tap(&mut app, &ctx, Tool::Lasso, 4.0);
        assert!(app.tool == Tool::Lasso && !app.lasso.erase);
        tap(&mut app, &ctx, Tool::Brush, 5.0);
        assert!(app.tool == Tool::Brush);
        assert_eq!(app.brush_group, BrushGroup::Pen);
        tap(&mut app, &ctx, Tool::Lasso, 6.0);
        tap(&mut app, &ctx, Tool::Lasso, 7.0);
        assert!(app.tool == Tool::Lasso && app.lasso.erase);
        // A long hold of the current tool's key is not a tap.
        let none = egui::Modifiers::NONE;
        frame(
            &mut app,
            &ctx,
            8.0,
            true,
            vec![key(Tool::Lasso, true, false, none)],
            &[],
        );
        frame(
            &mut app,
            &ctx,
            9.0,
            true,
            vec![key(Tool::Lasso, false, false, none)],
            &[],
        );
        assert!(app.lasso.erase);
    }
    #[test]
    fn eraser_tip_keeps_its_own_settings_and_the_paint_brush_is_untouched() {
        let mut editor = app();
        editor.brush.size = 12.0;
        editor.select_tool(Tool::Eraser);
        // Brush-shape mode erases with the brush's shape at the eraser size.
        assert_eq!(editor.brush.size, 30.0);
        assert_eq!(editor.paint_brush().size, 12.0);
        editor.set_eraser_own(true);
        assert_eq!(editor.brush.name, "Eraser");
        editor.brush.size = 80.0;
        editor.select_tool(Tool::Brush);
        assert_eq!(editor.brush.size, 12.0, "paint brush restored");
        assert_eq!(editor.eraser_brush.size, 80.0, "eraser tip kept");
        // Preferences saved mid-swap still separate the two.
        editor.select_tool(Tool::Eraser);
        let saved =
            crate::preferences::Preferences::decode(&editor.encode_preferences().unwrap()).unwrap();
        assert_eq!(saved.current_brush.size, 12.0);
        assert!(saved.tools.eraser_own);
        assert_eq!(saved.tools.eraser.as_ref().unwrap().size, 80.0);
        let mut restored = app();
        restored.apply_preferences(saved);
        assert_eq!(restored.brush.size, 12.0);
        restored.select_tool(Tool::Eraser);
        assert_eq!(restored.brush.size, 80.0);
        // Back in brush-shape mode the eraser is the brush's shape at the one
        // shared eraser size; the pen keeps its own size underneath.
        editor.set_eraser_own(false);
        assert_eq!(editor.brush.size, 80.0);
        assert_ne!(editor.brush.name, "Eraser");
        assert_eq!(editor.paint_brush().size, 12.0);
    }
    #[test]
    fn brush_shape_eraser_keeps_its_own_size_and_presets_keep_the_pen_size() {
        let mut editor = app();
        editor.brushes = crate::workspace::default_brushes();
        editor.choose_brush(0);
        editor.brush.size = 12.0;
        editor.eraser_brush.size = 60.0;
        // Brush shape mode: the eraser uses the brush shape at its own size.
        assert!(!editor.eraser_own);
        assert_eq!(editor.eraser_for_stroke().size, 60.0);
        assert_eq!(editor.eraser_for_stroke().name, editor.brush.name);
        editor.select_tool(Tool::Eraser);
        assert_eq!(editor.brush.size, 60.0, "size controls edit the eraser");
        editor.brush.size = 90.0;
        // Choosing or saving a preset while erasing never mixes the sizes.
        assert!(!editor.brush_preset_modified() || editor.paint_brush().size == 12.0);
        editor.choose_brush(1);
        assert_eq!(editor.brush.size, 90.0, "the eraser keeps its size");
        let preset = editor.brushes[1].size;
        assert_eq!(editor.paint_brush().size, preset);
        let count = editor.brushes.len();
        editor.save_new_brush_preset();
        assert_eq!(editor.brushes.len(), count + 1);
        assert_eq!(editor.brushes[count].size, preset);
        editor.select_tool(Tool::Brush);
        assert_eq!(editor.brush.size, preset, "the pen gets its own size back");
        assert_eq!(editor.eraser_brush.size, 90.0);
        // The pen's eraser end, seen from the Brush tool, uses the eraser size.
        assert_eq!(editor.eraser_for_stroke().size, 90.0);
        let saved =
            crate::preferences::Preferences::decode(&editor.encode_preferences().unwrap()).unwrap();
        assert_eq!(saved.tools.eraser.unwrap().size, 90.0);
        assert_eq!(saved.current_brush.size, preset);
    }
    #[test]
    fn size_keys_resize_the_eraser_while_the_pen_is_flipped_then_the_pen_again() {
        let ctx = egui::Context::default();
        let mut editor = app();
        editor.brush.size = 20.0;
        editor.eraser_brush.size = 40.0;
        let press = |editor: &mut Dudel, ctx: &egui::Context, key: egui::Key, time: f64| {
            let event = |pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            frame(
                editor,
                ctx,
                time,
                true,
                vec![event(true), event(false)],
                &[],
            );
        };
        // Flip the pen: the next frame puts the eraser size in the brush slot.
        editor.input.test_eraser_in_range = true;
        frame(&mut editor, &ctx, 0.0, true, vec![], &[]);
        assert_eq!(editor.tool_label().0, "Eraser");
        press(&mut editor, &ctx, egui::Key::CloseBracket, 0.1);
        assert!(
            editor.brush.size > 40.0,
            "eraser grew: {}",
            editor.brush.size
        );
        assert_eq!(editor.paint_brush().size, 20.0, "pen untouched");
        let eraser = editor.brush.size;
        // Flip back: the same key now resizes the pen.
        editor.input.test_eraser_in_range = false;
        frame(&mut editor, &ctx, 0.2, true, vec![], &[]);
        assert_eq!(editor.brush.size, 20.0);
        press(&mut editor, &ctx, egui::Key::OpenBracket, 0.3);
        assert!(editor.brush.size < 20.0);
        assert_eq!(editor.eraser_tip().size, eraser, "eraser untouched");
        // Changing paint groups while flipped keeps both sizes in place.
        editor.brushes = crate::workspace::default_brushes();
        editor.input.test_eraser_in_range = true;
        frame(&mut editor, &ctx, 0.4, true, vec![], &[]);
        let pen = editor.paint_brush().size;
        editor.select_brush_group(editor.brush_group.next());
        editor.select_brush_group(editor.brush_group.next().next().next());
        assert_eq!(editor.brush.size, eraser, "still the eraser in the slot");
        editor.input.test_eraser_in_range = false;
        frame(&mut editor, &ctx, 0.5, true, vec![], &[]);
        assert_eq!(
            editor.brush.size, pen,
            "the pen's size survived the group round trip"
        );
    }
    #[test]
    fn built_in_presets_cover_every_group_and_validate() {
        let brushes = crate::workspace::default_brushes();
        for group in crate::brush::BrushGroup::ALL {
            assert!(brushes.iter().filter(|b| b.group() == group).count() >= 2);
        }
        for brush in &brushes {
            brush.validate().unwrap();
            assert!(brush.group.is_some(), "{}", brush.name);
        }
        // Presets saved before groups existed are sorted by name and tags.
        let legacy = |name: &str, tags: &str| crate::brush::Brush {
            name: name.into(),
            tags: tags.into(),
            ..Default::default()
        };
        assert_eq!(
            legacy("HB", "sketch").group(),
            crate::brush::BrushGroup::Pencil
        );
        assert_eq!(legacy("Liner", "").group(), crate::brush::BrushGroup::Pen);
        assert_eq!(
            legacy("Oil", "paint").group(),
            crate::brush::BrushGroup::Brush
        );
        assert_eq!(
            legacy("Spray can", "").group(),
            crate::brush::BrushGroup::Airbrush
        );
    }
    #[test]
    fn keyboard_events_ignore_repeat_chords_and_release_with_changed_modifiers() {
        let ctx = egui::Context::default();
        let mut app = app();
        frame(
            &mut app,
            &ctx,
            0.0,
            true,
            vec![key(Tool::Eraser, true, false, egui::Modifiers::COMMAND)],
            &[],
        );
        assert!(app.tool == Tool::Brush);
        frame(
            &mut app,
            &ctx,
            0.1,
            true,
            vec![key(Tool::Eraser, false, false, egui::Modifiers::COMMAND)],
            &[],
        );
        frame(
            &mut app,
            &ctx,
            0.2,
            true,
            vec![key(Tool::Eraser, true, false, egui::Modifiers::NONE)],
            &[],
        );
        assert!(app.tool == Tool::Eraser);
        frame(
            &mut app,
            &ctx,
            0.8,
            true,
            vec![key(Tool::Eraser, true, true, egui::Modifiers::NONE)],
            &[],
        );
        frame(
            &mut app,
            &ctx,
            0.9,
            true,
            vec![key(Tool::Eraser, false, false, egui::Modifiers::SHIFT)],
            &[],
        );
        assert!(app.tool == Tool::Brush);
        frame(
            &mut app,
            &ctx,
            1.0,
            true,
            vec![
                key(Tool::Fill, true, false, egui::Modifiers::NONE),
                key(Tool::Fill, false, false, egui::Modifiers::NONE),
            ],
            &[],
        );
        assert!(app.tool == Tool::Fill);
        assert!(!app.history.can_undo());
    }

    #[test]
    fn text_edit_focus_blocks_tool_shortcuts_and_cancels_a_held_tool() {
        let ctx = egui::Context::default();
        let mut app = app();
        frame(
            &mut app,
            &ctx,
            0.0,
            true,
            vec![key(Tool::Eraser, true, false, egui::Modifiers::NONE)],
            &[],
        );
        let mut text = String::new();
        let id = egui::Id::new("tool-shift-text");
        let raw = egui::RawInput {
            time: Some(0.7),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(id))
                    .request_focus();
            });
        });
        assert!(ctx.wants_keyboard_input());
        let raw = egui::RawInput {
            time: Some(0.8),
            events: vec![
                key(Tool::Fill, true, false, egui::Modifiers::NONE),
                egui::Event::Text("g".into()),
            ],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            app.update_tool_shift(ctx);
            app.shortcuts(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(id));
            });
        });
        assert!(app.tool == Tool::Brush);
        assert_eq!(text, "g");
        ctx.memory_mut(|memory| memory.surrender_focus(id));
        frame(
            &mut app,
            &ctx,
            1.0,
            true,
            vec![
                key(Tool::Fill, false, false, egui::Modifiers::NONE),
                key(Tool::Eraser, false, false, egui::Modifiers::NONE),
            ],
            &[],
        );
        assert!(app.tool == Tool::Brush);
    }

    #[test]
    fn rename_modal_and_focus_loss_cancel_holds_without_resurrecting_keys() {
        let ctx = egui::Context::default();
        let mut app = app();
        frame(
            &mut app,
            &ctx,
            0.0,
            true,
            vec![key(Tool::Eraser, true, false, egui::Modifiers::NONE)],
            &[],
        );
        app.renaming = Some(0);
        frame(&mut app, &ctx, 0.7, true, vec![], &[]);
        assert!(app.tool == Tool::Brush);
        app.renaming = None;
        frame(
            &mut app,
            &ctx,
            0.8,
            true,
            vec![key(Tool::Eraser, true, true, egui::Modifiers::NONE)],
            &[],
        );
        assert!(app.tool == Tool::Brush);
        frame(
            &mut app,
            &ctx,
            0.9,
            true,
            vec![key(Tool::Eraser, false, false, egui::Modifiers::NONE)],
            &[],
        );
        frame(
            &mut app,
            &ctx,
            1.0,
            true,
            vec![key(Tool::Picker, true, false, egui::Modifiers::NONE)],
            &[],
        );
        assert!(app.tool == Tool::Picker);
        frame(
            &mut app,
            &ctx,
            1.1,
            false,
            vec![egui::Event::WindowFocused(false)],
            &[crate::input::Packet::Cancel],
        );
        assert!(app.tool == Tool::Brush);
        frame(
            &mut app,
            &ctx,
            1.8,
            true,
            vec![key(Tool::Picker, false, false, egui::Modifiers::NONE)],
            &[],
        );
        assert!(app.tool == Tool::Brush);
        app.new_canvas = true;
        frame(
            &mut app,
            &ctx,
            2.0,
            true,
            vec![key(Tool::Fill, true, false, egui::Modifiers::NONE)],
            &[],
        );
        assert!(app.tool == Tool::Brush);
        app.new_canvas = false;
        frame(
            &mut app,
            &ctx,
            2.1,
            true,
            vec![key(Tool::Fill, false, false, egui::Modifiers::NONE)],
            &[],
        );
        assert!(app.tool == Tool::Brush);
    }

    #[test]
    fn focus_restore_delivers_completed_tap_and_cancels_only_the_open_contact() {
        use crate::{
            input::Packet,
            stroke::Sample,
            stroke_buffer::{Settings, StrokeBuffer},
        };
        let ctx = egui::Context::default();
        let mut app = app();
        app.doc.layers[0].pixels = vec![120; 64 * 64 * 4].into();
        app.brush.size = 13.0;
        app.eraser_brush.size = 13.0;
        app.brush.pressure_size = false;
        app.brush.flow = 1.0;
        app.brush.taper_out = 0.0;
        frame(&mut app, &ctx, 0.0, true, vec![], &[]);
        let view = app.view(app.canvas_area);
        let screen = |x| {
            let position = view.to_screen(egui::pos2(x, 20.5));
            Sample {
                x: position.x,
                y: position.y,
                pressure: 1.0,
                ..Default::default()
            }
        };
        let first = screen(20.5);
        let second = screen(40.5);
        let at = view.to_doc(egui::pos2(first.x, first.y));
        let mut expected = app.doc.clone();
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: app.brush.clone(),
            color: app.color,
            erase: true,
            layer: 0,
            symmetry: app.symmetry,
        };
        let mut buffer = StrokeBuffer::default();
        buffer.paint(
            &mut expected,
            Sample {
                x: at.x,
                y: at.y,
                pressure: 1.0,
                ..Default::default()
            },
            &settings,
        );
        buffer.finish_tail(&mut expected, &settings);
        buffer.reclaim_empty_tiles(&mut expected);
        frame(
            &mut app,
            &ctx,
            0.1,
            true,
            vec![key(Tool::Eraser, true, false, egui::Modifiers::NONE)],
            &[Packet::Start(first)],
        );
        assert!(app.stroking);
        frame(
            &mut app,
            &ctx,
            0.9,
            false,
            vec![egui::Event::WindowFocused(false)],
            &[Packet::End(first), Packet::Start(second), Packet::Cancel],
        );
        assert!(app.tool == Tool::Brush && !app.stroking);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        app.undo();
        assert_eq!(app.doc.layers[0].pixels, vec![120; 64 * 64 * 4]);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn every_tool_taps_permanently_and_long_holds_restore() {
        for tool in Tool::ALL {
            for duration in [0.0, 0.5, 0.501, 2.0] {
                let mut shift = ToolShift::default();
                shift.permanent(Tool::Move);
                shift.press(tool, 1.0);
                assert!(shift.current() == tool);
                shift.press(tool, 1.2); // Duplicate press cannot restart the timer.
                shift.release(tool, 1.0 + duration);
                assert!(shift.current() == if duration <= 0.5 { tool } else { Tool::Move });
                assert!(shift.held.is_empty());
            }
        }
    }
    #[test]
    fn overlapping_keys_and_explicit_selection_keep_a_permanent_base() {
        let mut shift = ToolShift::default();
        shift.press(Tool::Eraser, 0.0);
        shift.press(Tool::Picker, 0.1);
        shift.release(Tool::Picker, 0.8);
        assert!(shift.current() == Tool::Eraser);
        shift.release(Tool::Eraser, 0.9);
        assert!(shift.current() == Tool::Brush);
        shift.press(Tool::Eraser, 0.0);
        shift.press(Tool::Picker, 0.7);
        shift.release(Tool::Eraser, 0.8); // Releasing an older hold leaves Picker active.
        assert!(shift.current() == Tool::Picker);
        shift.release(Tool::Picker, 0.9); // Tap commits Picker.
        assert!(shift.current() == Tool::Picker);
        shift.press(Tool::Eraser, 1.0);
        shift.permanent(Tool::Fill);
        shift.press(Tool::Eraser, 2.0); // Held key remains blocked until release.
        assert!(shift.current() == Tool::Fill);
        shift.release(Tool::Eraser, 3.0);
        assert!(shift.current() == Tool::Fill);
        shift.press(Tool::Eraser, 4.0);
        shift.release(Tool::Eraser, 5.0);
        assert!(shift.current() == Tool::Fill);
    }
}
