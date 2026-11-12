//! The welcome window: how to start, the few keys worth knowing first, and
//! where to find help. It opens at startup until the artist turns it off.
use crate::{Dudel, commands::Action, egui, tools::Tool};

pub(crate) const GUIDE: &str = concat!(
    env!("CARGO_PKG_REPOSITORY"),
    "/blob/master/docs/QUICK_START.md"
);
pub(crate) const RELEASES: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases");

impl Dudel {
    /// The first keys to learn, using the artist's own shortcuts.
    fn welcome_basics(&self) -> Vec<(String, &'static str)> {
        // The first binding only: "Ctrl+K / Ctrl+Shift+P" crowds the list.
        let key = |action: Action, fallback: &str| {
            let label = self.shortcuts.label(action);
            match label.split(" / ").next() {
                Some(first) if !first.is_empty() => first.to_owned(),
                _ => fallback.to_owned(),
            }
        };
        vec![
            (
                key(Action::Tool(Tool::Brush), "B"),
                "Paint tools; press again for Pencil, Pen, Brush, Airbrush",
            ),
            (
                key(Action::Tool(Tool::Eraser), "E"),
                "Eraser; hold it, and release to return",
            ),
            (
                format!(
                    "{} / {}",
                    key(Action::SmallerBrush, "["),
                    key(Action::LargerBrush, "]")
                ),
                "Smaller / larger brush",
            ),
            ("Space + drag".into(), "Pan; scroll to zoom"),
            (key(Action::Undo, "Ctrl+Z"), "Undo"),
            ("Pen side button".into(), "Quick menu of brushes and colors"),
            (key(Action::Search, "Ctrl+K"), "Find any command"),
        ]
    }

    pub(crate) fn welcome_ui(&mut self, ctx: &egui::Context) {
        if !self.welcome_open {
            return;
        }
        let basics = self.welcome_basics();
        let mut open = true;
        let mut close = false;
        let mut command = None;
        let mut chosen = None;
        egui::Window::new("Welcome to Dudel")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(560.0)
            .max_width(560.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Version {} · a drawing app made for pen tablets",
                    env!("CARGO_PKG_VERSION")
                ));
                ui.add_space(8.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(200.0);
                        ui.strong("Start");
                        let wide = egui::vec2(200.0, 0.0);
                        if ui
                            .add(crate::workspace::primary_button(ui, "New canvas…").min_size(wide))
                            .clicked()
                        {
                            command = Some(Action::New);
                        }
                        if ui.add(egui::Button::new("Open…").min_size(wide)).clicked() {
                            command = Some(Action::Open);
                        }
                        if ui
                            .add(egui::Button::new("Draw on this blank canvas").min_size(wide))
                            .clicked()
                        {
                            close = true;
                        }
                        if !self.recent.is_empty() {
                            ui.add_space(6.0);
                            ui.weak("Recent");
                            for path in self.recent.iter().take(5) {
                                let name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| path.display().to_string());
                                if ui
                                    .add(egui::Button::new(name).truncate().min_size(wide))
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    chosen = Some(path.clone());
                                }
                            }
                        }
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.strong("Basics");
                        for (keys, what) in &basics {
                            ui.horizontal_top(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(120.0, 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(120.0);
                                        ui.label(egui::RichText::new(keys).strong());
                                    },
                                );
                                ui.add(egui::Label::new(*what).wrap());
                            });
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to("Quick start guide", GUIDE);
                    ui.label("·");
                    if ui
                        .link("Check your pen")
                        .on_hover_text("Pressure test and tablet input diagnostics")
                        .clicked()
                    {
                        self.input_settings = true;
                        close = true;
                    }
                    ui.label("·");
                    if ui.link("All shortcuts").clicked() {
                        command = Some(Action::Shortcuts);
                    }
                    if self.setup.can_install {
                        ui.label("·");
                        if ui
                            .link("Install on this PC")
                            .on_hover_text(
                                "Adds this portable Dudel to the Start menu and opens .dudel projects with it",
                            )
                            .clicked()
                        {
                            self.setup.install_open = true;
                            close = true;
                        }
                    }
                });
                ui.checkbox(&mut self.show_welcome, "Show this window when Dudel starts");
            });
        if !open || close || command.is_some() || chosen.is_some() {
            self.welcome_open = false;
        }
        if let Some(action) = command {
            self.execute_command(action);
        }
        if let Some(path) = chosen {
            self.open_path(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Dudel, egui};

    #[test]
    fn welcome_starts_closed_outside_a_real_launch_and_stays_until_answered() {
        assert!(!Dudel::default().welcome_open);
        let mut app = Dudel {
            welcome_open: true,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.welcome_ui(ctx));
        assert!(app.welcome_open);
        assert_eq!(app.welcome_basics().len(), 7);
    }
}
