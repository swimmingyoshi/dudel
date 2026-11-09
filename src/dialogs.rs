//! File dialogs and confirmations that keep the window running.
//! A blocking native dialog stalls the event loop until it closes, and a
//! Wayland compositor then offers to kill the "unresponsive" window; rfd's
//! async dialogs are polled each frame instead. Confirmations are drawn in
//! the app, since rfd's Linux message boxes need zenity installed. While a
//! question is open the app takes no other input, so nothing it was asked
//! about can change before the answer runs.
use crate::Dudel;
use eframe::egui;
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

type Reply = Box<dyn FnOnce(&mut Dudel)>;

#[derive(Default)]
pub(crate) struct Dialogs {
    native: Option<Pin<Box<dyn Future<Output = Reply>>>>,
    confirm: Option<Confirm>,
}
impl Dialogs {
    pub(crate) fn open(&self) -> bool {
        self.native.is_some() || self.confirm.is_some()
    }
    pub(crate) fn native_open(&self) -> bool {
        self.native.is_some()
    }
}

struct Confirm {
    title: String,
    text: String,
    action: &'static str,
    then: Reply,
}

/// Repaints when a dialog answers, so the reply runs without other input.
struct Repaint(egui::Context);
impl Wake for Repaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }
}

impl Dudel {
    fn ask<T: 'static>(
        &mut self,
        answer: impl Future<Output = T> + 'static,
        then: impl FnOnce(&mut Dudel, T) + 'static,
    ) {
        // Input is held while a question is open, so this only guards
        // programmatic requests.
        if self.dialogs.open() {
            return;
        }
        self.dialogs.native = Some(Box::pin(async move {
            let answer = answer.await;
            Box::new(move |app: &mut Dudel| then(app, answer)) as Reply
        }));
    }
    /// Ask for a file to read; `then` runs only when one is chosen.
    pub(crate) fn ask_open_file(
        &mut self,
        dialog: rfd::AsyncFileDialog,
        then: impl FnOnce(&mut Dudel, PathBuf) + 'static,
    ) {
        self.ask(dialog.pick_file(), |app, file| {
            if let Some(file) = file {
                then(app, file.path().to_owned());
            }
        });
    }
    /// Ask for multiple files to read without blocking the event loop.
    pub(crate) fn ask_open_files(
        &mut self,
        dialog: rfd::AsyncFileDialog,
        then: impl FnOnce(&mut Dudel, Vec<PathBuf>) + 'static,
    ) {
        self.ask(dialog.pick_files(), |app, files| {
            if let Some(files) = files {
                then(
                    app,
                    files
                        .into_iter()
                        .map(|file| file.path().to_owned())
                        .collect(),
                );
            }
        });
    }
    /// Ask where to write a file; `then` runs only when a path is chosen.
    pub(crate) fn ask_save_file(
        &mut self,
        dialog: rfd::AsyncFileDialog,
        then: impl FnOnce(&mut Dudel, PathBuf) + 'static,
    ) {
        self.ask(dialog.save_file(), |app, file| {
            if let Some(file) = file {
                then(app, file.path().to_owned());
            }
        });
    }
    /// Ask for a folder; `then` runs only when one is chosen.
    pub(crate) fn ask_folder(
        &mut self,
        dialog: rfd::AsyncFileDialog,
        then: impl FnOnce(&mut Dudel, PathBuf) + 'static,
    ) {
        self.ask(dialog.pick_folder(), |app, folder| {
            if let Some(folder) = folder {
                then(app, folder.path().to_owned());
            }
        });
    }
    /// Ask before an action; `then` runs only when it is confirmed.
    pub(crate) fn confirm(
        &mut self,
        title: &str,
        text: impl Into<String>,
        action: &'static str,
        then: impl FnOnce(&mut Dudel) + 'static,
    ) {
        if self.dialogs.open() {
            return;
        }
        self.dialogs.confirm = Some(Confirm {
            title: title.into(),
            text: text.into(),
            action,
            then: Box::new(then),
        });
    }
    /// Answer the open question with yes, as its first button does.
    #[cfg(test)]
    pub(crate) fn accept_confirmation(&mut self) {
        if let Some(confirm) = self.dialogs.confirm.take() {
            (confirm.then)(self);
        }
    }
    /// Drop open questions unanswered, as closing the window does.
    pub(crate) fn dismiss_dialogs(&mut self) {
        self.dialogs = Dialogs::default();
    }
    /// Poll a native dialog and run its reply once it answers. Runs at the end
    /// of a frame so a dialog requested during the frame opens right away.
    pub(crate) fn poll_dialogs(&mut self, ctx: &egui::Context) {
        let Some(answer) = &mut self.dialogs.native else {
            return;
        };
        let waker = Waker::from(Arc::new(Repaint(ctx.clone())));
        if let Poll::Ready(reply) = answer.as_mut().poll(&mut Context::from_waker(&waker)) {
            self.dialogs.native = None;
            reply(self);
            ctx.request_repaint();
        }
    }
    pub(crate) fn dialogs_ui(&mut self, ctx: &egui::Context) {
        if self.dialogs.native.is_some() {
            // The system dialog may open behind the window or on another
            // screen; say why the window is not taking input.
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dialog shade"),
            ));
            let visuals = ctx.style().visuals.clone();
            painter.rect_filled(screen, 0.0, visuals.extreme_bg_color.gamma_multiply(0.6));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Finish in the open file dialog to continue",
                egui::FontId::proportional(16.0),
                visuals.strong_text_color(),
            );
        }
        let Some(confirm) = &self.dialogs.confirm else {
            return;
        };
        let mut answer = None;
        let response = egui::Modal::new(egui::Id::new("confirm dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(&confirm.title);
            ui.add_space(8.0);
            ui.label(&confirm.text);
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui
                    .add(crate::workspace::primary_button(ui, confirm.action))
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        if answer.is_none() && response.should_close() {
            answer = Some(false);
        }
        if let Some(confirmed) = answer
            && let Some(confirm) = self.dialogs.confirm.take()
            && confirmed
        {
            (confirm.then)(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_question_runs_its_reply_once_answered_and_holds_others() {
        let ctx = egui::Context::default();
        let mut app = Dudel::default();
        app.ask(std::future::ready(7), |app, answer| {
            app.status = format!("answer {answer}");
        });
        assert!(app.dialogs.open());
        app.ask(std::future::ready(8), |app, _| app.status = "second".into());
        app.confirm("Title", "Text", "Do it", |app| {
            app.status = "confirmed".into()
        });
        assert!(app.dialogs.confirm.is_none());
        app.poll_dialogs(&ctx);
        assert_eq!(app.status, "answer 7");
        assert!(!app.dialogs.open());
    }
    #[test]
    fn confirmations_run_only_when_confirmed_and_dismissal_drops_them() {
        let mut app = Dudel::default();
        app.confirm("Title", "Text", "Do it", |app| {
            app.status = "confirmed".into()
        });
        assert!(app.dialogs.open());
        app.dismiss_dialogs();
        assert!(!app.dialogs.open());
        app.confirm("Title", "Text", "Do it", |app| {
            app.status = "confirmed".into()
        });
        let confirm = app.dialogs.confirm.take().unwrap();
        (confirm.then)(&mut app);
        assert_eq!(app.status, "confirmed");
    }
}
