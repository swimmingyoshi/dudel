//! The Updates and Install windows, and restarting Dudel into another copy
//! of itself once the open projects have been dealt with.
use crate::{
    Dudel,
    background::{self, Handle},
    egui,
    update::{self, Release},
};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

/// The program to start once this one has closed.
static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// After the window has closed and settings are saved: start the copy the
/// artist asked to restart into, if any.
pub(crate) fn relaunch_if_requested() {
    let program = RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(program) = program {
        let _ = std::process::Command::new(program).spawn();
    }
}

#[derive(Default)]
enum Updates {
    #[default]
    Idle,
    Checking(Handle<Result<Option<Release>, String>>),
    Current,
    Available(Release),
    Installing {
        release: Release,
        job: Handle<Result<(), String>>,
        received: Arc<AtomicU64>,
    },
    Installed(String),
    Failed(String),
}

#[derive(Default)]
pub(crate) struct Setup {
    pub updates_open: bool,
    updates: Updates,
    pub install_open: bool,
    /// This is a portable Windows copy that can be installed.
    pub can_install: bool,
    #[cfg(windows)]
    options: crate::install::Options,
    install_job: Option<Handle<Result<PathBuf, String>>>,
    installed: Option<PathBuf>,
    install_error: Option<String>,
    /// Restart into this program, asked for in this frame.
    relaunch: Option<(PathBuf, u64)>,
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_048_576.0)
}

fn stopped<T>(result: std::thread::Result<Result<T, String>>) -> Result<T, String> {
    result.unwrap_or_else(|_| Err("The task stopped unexpectedly".into()))
}

impl Dudel {
    /// Help > Check for updates: the only thing that makes Dudel connect
    /// to the internet.
    pub(crate) fn check_for_updates(&mut self, ctx: &egui::Context) {
        self.setup.updates_open = true;
        if matches!(
            self.setup.updates,
            Updates::Checking(_) | Updates::Installing { .. } | Updates::Installed(_)
        ) {
            return;
        }
        let ctx = ctx.clone();
        self.setup.updates = Updates::Checking(background::spawn(move || {
            let result = update::check();
            ctx.request_repaint();
            result
        }));
    }

    fn install_update(&mut self, ctx: &egui::Context, release: Release) {
        let Some(folder) = crate::install::packaged_folder() else {
            return;
        };
        let received = Arc::new(AtomicU64::new(0));
        let (ctx, progress, wanted) = (ctx.clone(), received.clone(), release.clone());
        let job = background::spawn(move || {
            let result = update::download_and_install(&wanted, &folder, &progress);
            ctx.request_repaint();
            result
        });
        self.setup.updates = Updates::Installing {
            release,
            job,
            received,
        };
    }

    /// Close Dudel the ordinary way, asking about unsaved work, and start
    /// `program` afterwards.
    fn restart_into(&mut self, ctx: &egui::Context, program: PathBuf) {
        self.setup.relaunch = Some((program, ctx.cumulative_frame_nr()));
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Hand a requested restart over once closing is certain, and forget
    /// it when the artist cancels the close.
    fn track_relaunch(&mut self, frame: u64) {
        let Some((program, asked)) = &self.setup.relaunch else {
            return;
        };
        if self.close_ready {
            *RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(program.clone());
        } else if frame > asked + 2
            && self.unsaved_action.is_none()
            && self.pending_action.is_none()
            && !self.dialogs.open()
        {
            // The artist chose to stay: a later close is only a close.
            self.setup.relaunch = None;
        }
    }

    fn poll_setup(&mut self) {
        let setup = &mut self.setup;
        match &setup.updates {
            Updates::Checking(job) if job.is_finished() => {
                let Updates::Checking(job) = std::mem::take(&mut setup.updates) else {
                    unreachable!()
                };
                setup.updates = match stopped(job.join()) {
                    Ok(Some(release)) => Updates::Available(release),
                    Ok(None) => Updates::Current,
                    Err(error) => Updates::Failed(error),
                };
            }
            Updates::Installing { job, .. } if job.is_finished() => {
                let Updates::Installing { release, job, .. } = std::mem::take(&mut setup.updates)
                else {
                    unreachable!()
                };
                setup.updates = match stopped(job.join()) {
                    Ok(()) => Updates::Installed(release.version),
                    Err(error) => Updates::Failed(error),
                };
            }
            _ => {}
        }
        if setup.install_job.as_ref().is_some_and(Handle::is_finished) {
            match stopped(setup.install_job.take().expect("checked above").join()) {
                Ok(program) => {
                    setup.installed = Some(program);
                    setup.can_install = false;
                }
                Err(error) => setup.install_error = Some(error),
            }
        }
    }

    pub(crate) fn setup_ui(&mut self, ctx: &egui::Context) {
        self.poll_setup();
        self.updates_ui(ctx);
        #[cfg(windows)]
        self.install_ui(ctx);
        self.track_relaunch(ctx.cumulative_frame_nr());
    }

    fn updates_ui(&mut self, ctx: &egui::Context) {
        if !self.setup.updates_open {
            return;
        }
        let mut open = true;
        let mut check = false;
        let mut install = None;
        let mut restart = false;
        let mut close = false;
        egui::Window::new("Updates")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(420.0)
            .max_width(420.0)
            .show(ctx, |ui| {
                let have = env!("CARGO_PKG_VERSION");
                match &self.setup.updates {
                    Updates::Idle => {}
                    Updates::Checking(_) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Asking github.com for the newest release…");
                        });
                    }
                    Updates::Current => {
                        ui.label(format!("You have the newest version, {have}."));
                        ui.add_space(6.0);
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    }
                    Updates::Available(release) => {
                        ui.strong(format!("Dudel {} is available", release.version));
                        ui.label(format!("You have version {have}."));
                        if !release.notes.trim().is_empty() {
                            ui.add_space(6.0);
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(180.0)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        let notes: String =
                                            release.notes.chars().take(4000).collect();
                                        ui.add(egui::Label::new(notes).wrap());
                                    });
                            });
                        }
                        ui.add_space(6.0);
                        let package = release.package.as_ref().filter(|_| update::can_install());
                        if package.is_none() {
                            ui.label(if cfg!(windows) {
                                "This copy cannot replace itself. Download the new version from its release page."
                            } else {
                                "Download the new version from its release page and install it over this one."
                            });
                        }
                        ui.horizontal(|ui| {
                            if let Some(package) = package {
                                let label = format!(
                                    "Download and install ({})",
                                    megabytes(package.size)
                                );
                                if ui
                                    .add(crate::workspace::primary_button(ui, &label))
                                    .on_hover_text(
                                        "Downloads the new version, checks it against its published checksum and replaces this copy. Your projects, brushes and settings are not touched.",
                                    )
                                    .clicked()
                                {
                                    install = Some(release.clone());
                                }
                            }
                            ui.hyperlink_to("Release page", &release.page);
                        });
                    }
                    Updates::Installing {
                        release, received, ..
                    } => {
                        let size = release.package.as_ref().map_or(0, |package| package.size);
                        let got = received.load(Ordering::Relaxed);
                        let done = size > 0 && got >= size;
                        ui.label(format!("Dudel {}", release.version));
                        ui.add(
                            egui::ProgressBar::new(if size == 0 {
                                0.0
                            } else {
                                got as f32 / size as f32
                            })
                            .text(if done {
                                "Checking and installing…".to_owned()
                            } else {
                                format!(
                                    "Downloading… {} of {}",
                                    megabytes(got),
                                    megabytes(size)
                                )
                            }),
                        );
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    Updates::Installed(version) => {
                        ui.strong(format!("Dudel {version} is installed"));
                        ui.label("It starts the next time you open Dudel.");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(crate::workspace::primary_button(ui, "Restart now"))
                                .on_hover_text("Asks about unsaved changes first")
                                .clicked()
                            {
                                restart = true;
                            }
                            if ui.button("Later").clicked() {
                                close = true;
                            }
                        });
                    }
                    Updates::Failed(error) => {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                        ui.label("Nothing was changed.");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Try again").clicked() {
                                check = true;
                            }
                            ui.hyperlink_to("Releases page", crate::welcome::RELEASES);
                        });
                    }
                }
                ui.add_space(4.0);
                ui.weak("Dudel connects to the internet only when you ask it to here.");
            });
        if !open || close {
            self.setup.updates_open = false;
        }
        if check {
            self.setup.updates = Updates::Idle;
            self.check_for_updates(ctx);
        }
        if let Some(release) = install {
            self.install_update(ctx, release);
        }
        if restart && let Ok(program) = std::env::current_exe() {
            self.setup.updates_open = false;
            self.restart_into(ctx, program);
        }
    }

    #[cfg(windows)]
    fn install_ui(&mut self, ctx: &egui::Context) {
        if !self.setup.install_open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let mut close = false;
        let mut switch = None;
        egui::Window::new("Install Dudel")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .min_width(440.0)
            .max_width(440.0)
            .show(ctx, |ui| {
                let setup = &mut self.setup;
                if let Some(program) = &setup.installed {
                    ui.strong("Dudel is installed");
                    ui.label("You will find it in the Start menu. This folder is no longer needed and can be deleted.");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(crate::workspace::primary_button(
                                ui,
                                "Switch to the installed Dudel",
                            ))
                            .on_hover_text("Closes this copy, asking about unsaved changes first")
                            .clicked()
                        {
                            switch = Some(program.clone());
                        }
                        if ui.button("Stay here").clicked() {
                            close = true;
                        }
                    });
                    return;
                }
                ui.label("Copies this Dudel to your programs folder and adds it to the Start menu, for your Windows account only. No administrator rights are needed.");
                ui.add_space(6.0);
                let busy = setup.install_job.is_some();
                ui.add_enabled_ui(!busy, |ui| {
                    ui.checkbox(&mut setup.options.desktop, "Add a desktop shortcut");
                    ui.checkbox(
                        &mut setup.options.open_projects,
                        "Open .dudel projects with Dudel",
                    );
                    ui.checkbox(
                        &mut setup.options.thumbnails,
                        "Show artwork previews of projects in File Explorer",
                    );
                });
                ui.add_space(6.0);
                ui.weak("Your projects, brushes and settings stay where they are and are shared with this copy. To remove Dudel later, use Windows Settings > Apps.");
                if let Some(error) = &setup.install_error {
                    ui.add_space(4.0);
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                        ui.label("Installing…");
                    } else {
                        if ui
                            .add(crate::workspace::primary_button(ui, "Install"))
                            .clicked()
                        {
                            start = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if !open || close {
            self.setup.install_open = false;
        }
        if start {
            let (ctx, options) = (ctx.clone(), self.setup.options);
            self.setup.install_error = None;
            self.setup.install_job = Some(background::spawn(move || {
                let result = crate::install::install(options);
                ctx.request_repaint();
                result
            }));
        }
        if let Some(program) = switch {
            self.setup.install_open = false;
            self.restart_into(ctx, program);
        }
    }
}

impl Dudel {
    /// The Install or Updates window as the artist meets it, for screenshots.
    pub(crate) fn setup_capture_fixture(&mut self, updates: bool) {
        self.welcome_open = false;
        if updates {
            self.setup.updates_open = true;
            self.setup.updates = Updates::Available(Release {
                version: "9.9.0".into(),
                page: crate::welcome::RELEASES.into(),
                notes: "Painting essentials: gradients, filters, smudge and blur brushes.

- Canvas materials
- Brush graphs
- Installing and updating from inside Dudel"
                    .into(),
                package: Some(update::Package {
                    url: String::new(),
                    size: 17 << 20,
                    digest: None,
                    checksum_url: None,
                }),
            });
        } else {
            self.setup.can_install = true;
            self.setup.install_open = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_actions::{Action, UnsavedChoice};

    #[test]
    fn a_restart_waits_for_unsaved_work_and_is_forgotten_when_the_artist_stays() {
        let requested = || RELAUNCH.lock().unwrap().take();
        let mut app = Dudel {
            modified: true,
            ..Dudel::default()
        };
        // Restart asked: Dudel asks about the unsaved project first.
        app.setup.relaunch = Some(("new-dudel".into(), 10));
        app.request_replace(Action::Close);
        app.track_relaunch(20);
        assert!(app.setup.relaunch.is_some() && requested().is_none());
        // The artist cancels: a later close must not restart anything.
        app.answer_unsaved(UnsavedChoice::Cancel);
        app.track_relaunch(21);
        assert!(app.setup.relaunch.is_none());
        app.request_replace(Action::Close);
        app.answer_unsaved(UnsavedChoice::Discard);
        app.track_relaunch(22);
        assert!(app.close_ready && requested().is_none());
        // Asked again and confirmed: the new program starts after closing.
        let mut app = Dudel {
            modified: true,
            ..Dudel::default()
        };
        app.setup.relaunch = Some(("new-dudel".into(), 30));
        app.request_replace(Action::Close);
        app.answer_unsaved(UnsavedChoice::Discard);
        app.track_relaunch(31);
        assert_eq!(requested(), Some(PathBuf::from("new-dudel")));
    }
}
