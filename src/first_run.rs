//! First-time setup: three short pages shown on the first start, before any
//! preferences are saved: where Dudel keeps art, how it looks, and the pen.
//! Every choice applies at once and can be changed later; Help › First-time
//! setup opens it again.
use crate::{
    Dudel, egui, locations,
    ui::{
        style::{Density, Layout},
        widgets,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Folder,
    Look,
    Pen,
}

impl Page {
    const ALL: [Page; 3] = [Page::Folder, Page::Look, Page::Pen];
    fn name(self) -> &'static str {
        match self {
            Page::Folder => "Your folder",
            Page::Look => "Look",
            Page::Pen => "Pen",
        }
    }
    fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap()
    }
}

#[derive(Default)]
pub(crate) struct FirstRun {
    pub open: bool,
    pub page: Page,
    /// Open the Install window when setup is done (portable Windows copies).
    pub install: bool,
    /// Height of the tallest page shown, in points.
    tallest: f32,
}

impl FirstRun {
    /// From the first page again.
    pub(crate) fn start(&mut self) {
        *self = Self {
            open: true,
            ..Self::default()
        };
    }
}

const WIDTH: f32 = 600.0;

impl Dudel {
    pub(crate) fn first_run_ui(&mut self, ctx: &egui::Context) {
        if !self.first_run.open {
            return;
        }
        let page = self.first_run.page;
        let mut skip = false;
        let mut finish = false;
        crate::dialogs::centered(ctx, "Set up Dudel", egui::Window::new("Set up Dudel"))
            .collapsible(false)
            .resizable(false)
            .min_width(WIDTH)
            .max_width(WIDTH)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                ui.add_space(2.0);
                let names = Page::ALL.map(Page::name);
                crate::dialogs::steps(ui, &names, page.index());
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(6.0);
                // A page is at least as tall as the tallest one shown, so
                // the buttons stay put when going back and forth.
                let height = ui
                    .vertical(|ui| {
                        ui.set_min_height(self.first_run.tallest);
                        match page {
                            Page::Folder => self.first_run_folder(ui),
                            Page::Look => self.first_run_look(ui),
                            Page::Pen => self.first_run_pen(ui),
                        }
                    })
                    .response
                    .rect
                    .height();
                self.first_run.tallest = self.first_run.tallest.max(height);
                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button("Skip setup")
                        .on_hover_text("Keep the defaults; Help › First-time setup comes back here")
                        .clicked()
                    {
                        skip = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let last = page == Page::Pen;
                        let next = if last { "Start drawing" } else { "Next" };
                        if ui.add(crate::workspace::primary_button(ui, next)).clicked() {
                            match Page::ALL.get(page.index() + 1) {
                                Some(following) => self.first_run.page = *following,
                                None => finish = true,
                            }
                        }
                        if page != Page::Folder && ui.button("Back").clicked() {
                            self.first_run.page = Page::ALL[page.index() - 1];
                        }
                    });
                });
            });
        if skip || finish {
            self.first_run.open = false;
            self.welcome_open = self.show_welcome || finish;
            if finish && self.first_run.install && self.setup.can_install {
                self.setup.install_open = true;
                self.welcome_open = false;
            }
            self.status = if finish {
                "Setup done. View › Theme and Settings change any of it later".into()
            } else {
                String::new()
            };
        }
    }

    fn first_run_folder(&mut self, ui: &mut egui::Ui) {
        ui.heading("Where should Dudel keep your art?");
        ui.add(
            egui::Label::new(
                "Projects, exports, brushes, palettes, themes and timelapses each get a \
                 folder inside this one. Folders are made when first used.",
            )
            .wrap(),
        );
        ui.add_space(8.0);
        let standard = self.locations.library.is_none();
        let mut choose = false;
        if ui.radio(standard, "In Documents (recommended)").clicked() {
            self.locations.library = None;
        }
        ui.indent("standard folder", |ui| {
            path_label(ui, &locations::default_library());
        });
        ui.add_space(4.0);
        // Choosing "another folder" asks for it; cancelling keeps Documents.
        if ui.radio(!standard, "In another folder…").clicked() && standard {
            choose = true;
        }
        let chosen = self.locations.library.clone();
        if let Some(folder) = &chosen {
            ui.indent("chosen folder", |ui| {
                ui.horizontal(|ui| {
                    path_label(ui, folder);
                    choose |= ui.button("Change…").clicked();
                });
            });
        }
        if choose {
            let dialog = rfd::AsyncFileDialog::new().set_title("Choose the Dudel folder");
            let dialog = match &chosen {
                Some(folder) if folder.is_dir() => dialog.set_directory(folder),
                _ => dialog,
            };
            self.ask_folder(dialog, |app, folder| {
                app.locations.library = Some(folder);
            });
        }
        ui.add_space(8.0);
        ui.weak("Change it later in Settings › Files › Storage & folders, where each kind can also go elsewhere.");
    }

    fn first_run_look(&mut self, ui: &mut egui::Ui) {
        ui.heading("Choose a look");
        ui.add(
            egui::Label::new(
                "Classic is Dudel's fixed layout. Slate, Float and Atelier are previews in \
                 which every panel is a palette you can move, group and float.",
            )
            .wrap(),
        );
        ui.add_space(8.0);
        let current = self.interface.layout;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 40.0 / 3.0;
            for layout in Layout::ALL {
                if crate::interface::preset_card(ui, layout, current == layout, &self.theme)
                    .clicked()
                    && layout != current
                {
                    self.choose_interface(layout);
                }
            }
        });
        ui.weak(self.interface.layout.hint());
        ui.add_space(8.0);
        ui.strong("Colors");
        ui.horizontal_wrapped(|ui| {
            self.own_colors_choice(ui);
            self.theme_choices(ui);
        });
        ui.add_space(4.0);
        ui.weak("View › Theme switches style and colors at any time.");
    }

    fn first_run_pen(&mut self, ui: &mut egui::Ui) {
        ui.heading("Your pen and screen");
        ui.add_space(4.0);
        egui::Grid::new("first run pen")
            .num_columns(2)
            .spacing(egui::vec2(12.0, 10.0))
            .show(ui, |ui| {
                ui.label("Control size");
                ui.vertical(|ui| {
                    let sizes: Vec<(Density, &str)> =
                        Density::ALL.iter().map(|d| (*d, d.name())).collect();
                    widgets::segmented(ui, &mut self.interface.density, &sizes);
                    ui.weak("Touch suits pen displays and tablets held close.");
                });
                ui.end_row();
                ui.label("Pressure");
                ui.vertical(|ui| {
                    if ui.button("Test your pen…").clicked() {
                        self.input_settings = true;
                    }
                    if cfg!(windows) {
                        ui.weak("Pressure needs Windows Ink turned on in the tablet driver.");
                    }
                });
                ui.end_row();
                if self.interface.layout == Layout::Float {
                    ui.label("Hand");
                    ui.checkbox(
                        &mut self.interface.rails_right,
                        "I draw with my left hand (slider rail on the right)",
                    );
                    ui.end_row();
                }
                if self.setup.can_install {
                    ui.label("Install");
                    ui.vertical(|ui| {
                        ui.checkbox(
                            &mut self.first_run.install,
                            "Add Dudel to the Start menu and open .dudel files with it",
                        );
                        ui.weak("Leave it off to keep this copy portable.");
                    });
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        ui.weak(format!(
            "Pen side button: a quick menu of brushes and colors. {} finds any command.",
            self.shortcuts
                .label(crate::commands::Action::Search)
                .split(" / ")
                .next()
                .filter(|k| !k.is_empty())
                .unwrap_or("Ctrl+K")
        ));
    }
}

fn path_label(ui: &mut egui::Ui, path: &std::path::Path) {
    let text = path.display().to_string();
    ui.add(egui::Label::new(egui::RichText::new(&text).monospace().small()).truncate())
        .on_hover_text(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locations::Kind;

    fn run(app: &mut Dudel) {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            crate::workspace::theme(ctx);
            app.first_run_ui(ctx);
        });
    }

    #[test]
    fn setup_stays_closed_unless_started_and_starts_on_the_folder_page() {
        let mut app = Dudel::default();
        assert!(!app.first_run.open);
        app.first_run.page = Page::Pen;
        app.first_run.install = true;
        app.first_run.start();
        assert!(app.first_run.open);
        assert_eq!(app.first_run.page, Page::Folder);
        assert!(!app.first_run.install);
        for page in Page::ALL {
            app.first_run.page = page;
            run(&mut app);
            assert!(app.first_run.open, "{page:?} stays until answered");
        }
        // The standard folder holds every kind of file.
        let library = app.locations.library();
        assert!(
            Kind::ALL
                .iter()
                .all(|k| app.locations.get(*k).starts_with(&library))
        );
    }

    #[test]
    fn pages_keep_the_tallest_height_without_growing() {
        let mut app = Dudel::default();
        app.first_run.start();
        let ctx = egui::Context::default();
        let frame = |app: &mut Dudel| {
            let _ = ctx.run(Default::default(), |ctx| {
                crate::workspace::theme(ctx);
                app.first_run_ui(ctx);
            });
        };
        app.first_run.page = Page::Look;
        frame(&mut app);
        let look = app.first_run.tallest;
        assert!(look > 100.0, "the Look page has height: {look}");
        app.first_run.page = Page::Folder;
        for _ in 0..20 {
            frame(&mut app);
        }
        assert_eq!(app.first_run.tallest, look, "no frame-by-frame growth");
    }
}
