//! Shortcut editing captures keys before application or text-input dispatch.
use crate::{
    Dudel,
    commands::{Action, COMMANDS},
    egui,
    shortcuts::{Bindings, Chord},
};
use std::{io::Read, path::Path};

#[derive(Default)]
pub(crate) struct Editor {
    pub filter: String,
    pub capture: Option<Action>,
    pub candidate: Option<Chord>,
    pub capture_error: String,
    pub message: String,
    pub block_frame: bool,
}

impl Dudel {
    pub(crate) fn capture_shortcut_input(&mut self, raw: &mut egui::RawInput) -> bool {
        self.shortcut_editor.block_frame = self.shortcut_editor.capture.is_some();
        if !self.shortcut_editor.block_frame {
            return false;
        }
        #[cfg(windows)]
        if let Some(backend) = self.clipboard_shortcuts.as_mut() {
            backend.take_paste();
        }
        let mut cancel = false;
        raw.events.retain(|event| match event {
            egui::Event::Key {
                key,
                modifiers,
                pressed,
                repeat,
                ..
            } => {
                // Let egui release keys held before the dialog opened. Releases
                // cannot dispatch commands; block_frame also suspends tool taps.
                if !pressed {
                    return true;
                }
                if *pressed && !repeat {
                    if *key == egui::Key::Escape {
                        cancel = true;
                    } else {
                        self.shortcut_editor.capture_error.clear();
                        self.shortcut_editor.candidate = Some(Chord::new(*key, *modifiers));
                    }
                }
                false
            }
            egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => {
                self.shortcut_editor.candidate = None;
                self.shortcut_editor.capture_error =
                    "Clipboard shortcuts keep their standard behavior".into();
                false
            }
            egui::Event::Text(_) => false,
            _ => true,
        });
        if cancel {
            self.shortcut_editor.capture = None;
        }
        true
    }

    pub(crate) fn apply_shortcuts(&mut self, bindings: Bindings) {
        self.finish_quick_resize(true);
        self.close_input_gestures();
        self.shortcuts = bindings;
        self.tool_shift = Default::default();
    }

    pub(crate) fn import_shortcut_profile(&mut self, path: &Path) -> Result<(), String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(crate::shortcuts::MAX_PROFILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let bindings = Bindings::import(&bytes)?;
        self.apply_shortcuts(bindings);
        Ok(())
    }

    pub(crate) fn shortcut_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Choose a command, then press its new shortcut. Unbind a conflicting command before assigning its keys elsewhere.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Restore defaults…").clicked() {
                self.confirm(
                    "Restore shortcut defaults",
                    "Restore every command's default shortcuts?",
                    "Restore",
                    |app| {
                        app.apply_shortcuts(Default::default());
                        app.shortcut_editor.message = "Default shortcuts restored".into();
                    },
                );
            }
            if ui.button("Import profile…").clicked() {
                self.ask_open_file(
                    rfd::AsyncFileDialog::new()
                        .add_filter("Dudel shortcuts", &["dudelkeys", "json"]),
                    |app, path| {
                        app.shortcut_editor.message = match app.import_shortcut_profile(&path) {
                            Ok(()) => "Shortcut profile imported".into(),
                            Err(error) => error,
                        };
                    },
                );
            }
            if ui.button("Export profile…").clicked() {
                match self.shortcuts.export() {
                    Ok(bytes) => self.ask_save_file(
                        rfd::AsyncFileDialog::new()
                            .add_filter("Dudel shortcuts", &["dudelkeys"])
                            .set_file_name("My shortcuts.dudelkeys"),
                        move |app, path| {
                            app.shortcut_editor.message =
                                match crate::storage::atomic_write(&path, &bytes) {
                                    Ok(()) => "Shortcut profile exported".into(),
                                    Err(error) => error,
                                };
                        },
                    ),
                    Err(error) => self.shortcut_editor.message = error,
                }
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.shortcut_editor.filter)
                .id_salt("shortcut filter")
                .hint_text("Find a command or shortcut")
                .char_limit(256)
                .desired_width(f32::INFINITY),
        );
        if !self.shortcut_editor.message.is_empty() {
            ui.label(&self.shortcut_editor.message);
        }
        ui.small("Clipboard, navigation, Enter/Escape, F6/F11 and Alt gestures keep their standard behavior. Tool shortcuts still support tap and hold.");
        let words: Vec<_> = self
            .shortcut_editor
            .filter
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut count = 0;
        for command in COMMANDS {
            let binding = self.shortcuts.label(command.action);
            let searchable =
                format!("{} {} {binding}", command.label, command.aliases).to_lowercase();
            if !words.iter().all(|word| searchable.contains(word)) {
                continue;
            }
            count += 1;
            ui.push_id(command.id, |ui| {
                ui.separator();
                ui.label(command.label);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(if binding.is_empty() {
                            "Unassigned"
                        } else {
                            &binding
                        })
                        .on_hover_text("Assign a shortcut")
                        .clicked()
                    {
                        self.finish_quick_resize(true);
                        self.close_input_gestures();
                        self.shortcut_editor.capture = Some(command.action);
                        self.shortcut_editor.candidate = None;
                        self.shortcut_editor.capture_error.clear();
                    }
                    if ui
                        .add_enabled(!binding.is_empty(), egui::Button::new("Unbind"))
                        .clicked()
                    {
                        if let Ok(next) = self.shortcuts.changed(command.action, Some(vec![])) {
                            self.apply_shortcuts(next);
                        }
                        self.shortcut_editor.message.clear();
                    }
                    if ui
                        .add_enabled(
                            self.shortcuts.0.contains_key(command.id),
                            egui::Button::new("Default"),
                        )
                        .clicked()
                    {
                        match self.shortcuts.changed(command.action, None) {
                            Ok(next) => {
                                self.apply_shortcuts(next);
                                self.shortcut_editor.message.clear();
                            }
                            Err(error) => self.shortcut_editor.message = error,
                        }
                    }
                });
            });
        }
        if count == 0 {
            ui.label("No matching commands");
        }
    }

    pub(crate) fn shortcut_capture_ui(&mut self, ctx: &egui::Context) {
        let Some(action) = self.shortcut_editor.capture else {
            return;
        };
        let proposed = self
            .shortcut_editor
            .candidate
            .map(|c| self.shortcuts.changed(action, Some(vec![c])));
        let mut apply = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("assign shortcut")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(220.0, 400.0));
            cancel |= crate::dialogs::dialog_title(ui, "Assign shortcut");
            ui.label(crate::commands::get(action).label);
            ui.label(
                self.shortcut_editor
                    .candidate
                    .map_or_else(|| "Press the keys you want to use".into(), Chord::label),
            );
            if let Some(Err(error)) = &proposed {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            if !self.shortcut_editor.capture_error.is_empty() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    &self.shortcut_editor.capture_error,
                );
            }
            ui.small("Apply replaces this command's current shortcuts. Escape cancels.");
            crate::dialogs::dialog_buttons(ui, |ui| {
                apply = ui
                    .add_enabled(
                        matches!(proposed, Some(Ok(_))),
                        crate::workspace::primary_button(ui, "Apply shortcut"),
                    )
                    .clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if apply && let Some(Ok(next)) = proposed {
            self.apply_shortcuts(next);
            self.shortcut_editor.message = "Shortcut saved".into();
            self.shortcut_editor.capture = None;
        } else if cancel || response.should_close() {
            self.shortcut_editor.capture = None;
        }
    }
}
