//! Searchable commands use the same application actions as menus and shortcuts.
use crate::{
    Dudel,
    commands::{Action, COMMANDS},
    egui,
};

#[derive(Default)]
pub(crate) struct Search {
    pub open: bool,
    pub query: String,
    selected: usize,
    focus: bool,
}

fn results(query: &str, bindings: &crate::shortcuts::Bindings) -> Vec<usize> {
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, command)| {
            let text = format!(
                "{} {} {}",
                command.label,
                command.aliases,
                bindings.label(command.action)
            )
            .to_lowercase();
            words
                .iter()
                .all(|word| text.contains(word))
                .then_some(index)
        })
        .collect()
}

impl Dudel {
    pub(crate) fn open_command_search(&mut self) {
        self.close_input_gestures();
        self.command_search = Search {
            open: true,
            focus: true,
            ..Default::default()
        };
    }

    /// Run before ordinary shortcuts, including while the query has text focus.
    pub(crate) fn command_search_shortcut(&mut self, ctx: &egui::Context) -> bool {
        if self.command_search.open {
            return true;
        }
        if self.new_canvas
            || self.unsaved_action.is_some()
            || self.dialogs.open()
            || self.renaming.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
            || self.canvas_resize.is_some()
            || self.canvas_edit_ticket.is_some()
            || egui::Popup::is_any_open(ctx)
        {
            return false;
        }
        if ctx.wants_keyboard_input() && !ctx.input(|i| i.events.iter().any(|event| {
            self.shortcuts.event_matches(Action::Search, event)
                && matches!(event, egui::Event::Key { modifiers, .. } if modifiers.command || modifiers.ctrl)
        })) { return false; }
        let open = self.shortcuts.consume(ctx, Action::Search);
        if open {
            self.open_command_search();
        }
        open
    }

    pub(crate) fn command_search_ui(&mut self, ctx: &egui::Context) {
        if !self.command_search.open {
            return;
        }
        let mut run = None;
        let mut close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let reset_scroll = self.command_search.focus;
        let mut query_id = None;
        let response = egui::Modal::new(egui::Id::new("command search")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(220.0, 500.0));
            ui.horizontal(|ui| {
                ui.heading("Find a command");
                if ui.small_button("Close").clicked() {
                    close = true;
                }
            });
            let (up, down, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            });
            let query = ui.add(
                egui::TextEdit::singleline(&mut self.command_search.query)
                    .id_salt("command query")
                    .hint_text("Search commands, tools or shortcuts…")
                    .char_limit(256)
                    .desired_width(f32::INFINITY),
            );
            query_id = Some(query.id);
            if self.command_search.focus {
                query.request_focus();
                self.command_search.focus = false;
            }
            if query.changed() {
                self.command_search.selected = 0;
            }
            let matches = results(&self.command_search.query, &self.shortcuts);
            self.command_search.selected = self
                .command_search
                .selected
                .min(matches.len().saturating_sub(1));
            if !matches.is_empty() {
                if down {
                    self.command_search.selected =
                        (self.command_search.selected + 1) % matches.len();
                }
                if up {
                    self.command_search.selected =
                        (self.command_search.selected + matches.len() - 1) % matches.len();
                }
                if enter
                    && self
                        .command_disabled(COMMANDS[matches[self.command_search.selected]].action)
                        .is_none()
                {
                    run = Some(matches[self.command_search.selected]);
                }
            }
            egui::ScrollArea::vertical()
                .id_salt("command results")
                .max_height((ctx.content_rect().height() - 210.0).clamp(80.0, 360.0))
                .show(ui, |ui| {
                    if matches.is_empty() {
                        ui.label("No matching commands. Try a tool name or action.");
                    }
                    for (row, &index) in matches.iter().enumerate() {
                        let command = &COMMANDS[index];
                        let reason = self.command_disabled(command.action);
                        let selected = self.command_search.selected == row;
                        let response = ui.add_enabled(
                            reason.is_none(),
                            egui::Button::new(command.label)
                                .selected(selected)
                                .right_text(self.shortcuts.label(command.action))
                                .min_size(egui::vec2(ui.available_width(), 28.0)),
                        );
                        if selected && (up || down || query.changed() || reset_scroll) {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() {
                            run = Some(index);
                        }
                        if let Some(reason) = reason {
                            response.on_disabled_hover_text(reason);
                            if selected {
                                ui.small(reason);
                            }
                        }
                    }
                });
            ui.small("Up / Down to choose · Enter to run · Esc to close");
            if run.is_some() || close {
                query.surrender_focus();
            }
        });
        if response.should_close() || close {
            self.command_search.open = false;
            if let Some(id) = query_id {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
        }
        if let Some(index) = run
            && self.command_disabled(COMMANDS[index].action).is_none()
        {
            self.command_search.open = false;
            self.execute_command(COMMANDS[index].action);
        }
    }
}
