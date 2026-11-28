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

/// Every Dudel dialog shares one look: a title row with a close button,
/// the content, then the buttons at the bottom right with the main action
/// highlighted and Cancel last. The close button and Esc both cancel. The
/// title row moves a dialog made with [`modal`].
pub(crate) fn dialog_title(ui: &mut egui::Ui, title: &str) -> bool {
    let mut close = false;
    drag_handle(ui);
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = close_button(ui).clicked();
        });
    });
    ui.add_space(4.0);
    close
}

/// A small heading over a group in a dialog: quieter than the content, so
/// it orders a window without competing with it.
pub(crate) fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(11.0)
            .extra_letter_spacing(0.8)
            .color(ui.visuals().weak_text_color()),
    );
}

/// A key or key combination drawn as a key cap.
pub(crate) fn key_cap(ui: &mut egui::Ui, keys: &str) -> egui::Response {
    let visuals = ui.visuals();
    egui::Frame::new()
        .fill(visuals.faint_bg_color.gamma_multiply(2.0))
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(keys).size(12.5)))
        .response
}

/// Numbered steps in a row: the finished ones and the current one filled,
/// joined by a line, the current name strong.
pub(crate) fn steps(ui: &mut egui::Ui, names: &[&str], current: usize) {
    let accent = ui.visuals().selection.bg_fill;
    let weak = ui.visuals().weak_text_color();
    let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
    ui.horizontal(|ui| {
        for (index, name) in names.iter().enumerate() {
            if index > 0 {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::hover());
                let color = if index <= current { accent } else { line };
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    egui::Stroke::new(2.0_f32, color),
                );
            }
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            let reached = index <= current;
            if reached {
                ui.painter().circle_filled(rect.center(), 10.0, accent);
            } else {
                ui.painter()
                    .circle_stroke(rect.center(), 9.0, egui::Stroke::new(1.5_f32, line));
            }
            let number = if index < current {
                "✓".to_string()
            } else {
                (index + 1).to_string()
            };
            let on_accent = ui.visuals().selection.stroke.color;
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                number,
                egui::FontId::proportional(12.0),
                if reached { on_accent } else { weak },
            );
            let text = egui::RichText::new(*name);
            ui.label(if index == current {
                text.strong()
            } else {
                text.color(weak)
            });
        }
    });
}

/// Dudel's icon at `size` points.
pub(crate) fn app_mark(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let id = egui::Id::new("dudel app mark");
    let texture = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id));
    let texture = texture.or_else(|| {
        let image = image::load_from_memory(include_bytes!("app_icon/dudel.png"))
            .ok()?
            .to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ui
            .ctx()
            .load_texture("dudel app mark", image, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    });
    match texture {
        Some(texture) => ui.add(egui::Image::new((texture.id(), egui::vec2(size, size)))),
        None => {
            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover())
                .1
        }
    }
}

/// How far a dialog has been dragged, and the frame it was last shown in.
type Moved = (egui::Vec2, u64);

/// Whether the dialog `id` is opening: it was not shown in the last frame.
/// Every dialog opens where it belongs, wherever it was dragged before.
fn opening(ctx: &egui::Context, last: Option<u64>) -> bool {
    last.is_none_or(|last| ctx.cumulative_frame_nr() > last + 1)
}

/// A modal dialog in the middle of the window that its title row can drag
/// out of the way. A plain `egui::Modal` cannot be moved.
pub(crate) fn modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    modal_at(ctx, id, egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
}

/// [`modal`] that opens `offset` away from the window's `align` corner.
pub(crate) fn modal_at(
    ctx: &egui::Context,
    id: egui::Id,
    align: egui::Align2,
    offset: egui::Vec2,
) -> egui::Modal {
    let key = id.with("dialog moved");
    let (mut moved, last) = ctx
        .data(|d| d.get_temp::<Moved>(key))
        .map_or((egui::Vec2::ZERO, None), |(moved, last)| {
            (moved, Some(last))
        });
    if opening(ctx, last) {
        moved = egui::Vec2::ZERO;
    } else if moved != egui::Vec2::ZERO
        && let Some(rect) = ctx.memory(|m| m.area_rect(id))
    {
        // Kept inside the window, so dragging back answers at once.
        let window = ctx.content_rect();
        let base = align.align_size_within_rect(rect.size(), window).min + offset;
        let room = (window.max - rect.size()).max(window.min);
        moved = (base + moved).clamp(window.min, room) - base;
    }
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(key, (moved, frame)));
    egui::Modal::new(id).area(egui::Modal::default_area(id).anchor(align, offset + moved))
}

/// Makes the row about to be added drag the dialog it is in. Call it before
/// the row, so the row's own buttons stay on top.
pub(crate) fn drag_handle(ui: &mut egui::Ui) {
    let id = ui.layer_id().id;
    let height = ui
        .text_style_height(&egui::TextStyle::Heading)
        .max(ui.spacing().interact_size.y)
        .max(22.0);
    let mut row = ui.available_rect_before_wrap();
    row.max.y = row.min.y + height;
    let response = ui
        .interact(row, id.with("dialog drag"), egui::Sense::DRAG)
        .on_hover_cursor(egui::CursorIcon::Grab);
    let delta = response.drag_delta();
    if delta != egui::Vec2::ZERO {
        ui.data_mut(|d| {
            // Only dialogs made with `modal` have an entry.
            let key = id.with("dialog moved");
            if let Some((moved, frame)) = d.get_temp::<Moved>(key) {
                d.insert_temp(key, (moved + delta, frame));
            }
        });
        ui.ctx().request_repaint();
    }
}

/// `window`, opening in the middle of the app and movable from there.
/// (`Window::anchor` would hold it in place.)
pub(crate) fn centered<'a>(
    ctx: &egui::Context,
    id: &str,
    window: egui::Window<'a>,
) -> egui::Window<'a> {
    let id = egui::Id::new(id);
    let key = id.with("window shown");
    let last = ctx.data(|d| d.get_temp::<u64>(key));
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(key, frame));
    let window = window.id(id).pivot(egui::Align2::CENTER_CENTER);
    if opening(ctx, last) {
        window.current_pos(ctx.content_rect().center())
    } else {
        window
    }
}

fn close_button(ui: &mut egui::Ui) -> egui::Response {
    // Like a window's close box, it is clicked, not reached with Tab.
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::CLICK);
    let visuals = ui.style().interact(&response);
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, visuals.weak_bg_fill);
    }
    let cross = rect.shrink(7.0);
    let stroke = egui::Stroke::new(1.5_f32, visuals.fg_stroke.color);
    ui.painter()
        .line_segment([cross.left_top(), cross.right_bottom()], stroke);
    ui.painter()
        .line_segment([cross.right_top(), cross.left_bottom()], stroke);
    response.on_hover_text("Close (Esc)")
}

/// Room left between a dialog and the window's top and bottom edges.
const DIALOG_MARGIN: f32 = 24.0;

/// The height a dialog's scrolling body may take so the whole dialog fits in
/// the window. The rest of the dialog (title, buttons, frame) keeps the room
/// it took when last shown, measured by [`measure_dialog`]; `guess` stands
/// in until then. A fixed guess alone was too small for larger interface
/// sizes, and the buttons ended up below the window.
pub(crate) fn dialog_body_room(ctx: &egui::Context, id: egui::Id, guess: f32) -> f32 {
    let rest = ctx
        .data(|d| d.get_temp::<f32>(id.with("dialog rest")))
        .unwrap_or(guess);
    (ctx.content_rect().height() - DIALOG_MARGIN - rest).max(80.0)
}

/// Remember how much of a dialog `whole` tall is not its scrolling body
/// `body` tall, for [`dialog_body_room`].
pub(crate) fn measure_dialog(ctx: &egui::Context, id: egui::Id, whole: f32, body: f32) {
    let id = id.with("dialog rest");
    let rest = whole - body;
    let known = ctx.data(|d| d.get_temp::<f32>(id));
    if known.is_none_or(|known| (known - rest).abs() > 0.5) {
        ctx.data_mut(|d| d.insert_temp(id, rest));
        ctx.request_repaint();
    }
}

/// The button row at the bottom of a dialog, at the right: add the main
/// action first and Cancel last, so Tab visits them in reading order.
pub(crate) fn dialog_buttons<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    dialog_buttons_beside(ui, |_| {}, add)
}

/// [`dialog_buttons`] with a secondary action, such as Reset, at the left.
pub(crate) fn dialog_buttons_beside<R>(
    ui: &mut egui::Ui,
    left: impl FnOnce(&mut egui::Ui),
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.add_space(4.0);
    ui.separator();
    ui.horizontal(|ui| {
        left(ui);
        // Right-aligned by last frame's width; a changed width redraws the
        // frame before it is shown, so the row never jumps.
        let id = ui.id().with("dialog buttons width");
        let width = ui.ctx().data(|d| d.get_temp::<f32>(id));
        ui.add_space((ui.available_width() - width.unwrap_or(0.0)).max(0.0));
        let row = ui.scope(add);
        let measured = row.response.rect.width();
        if width.is_none_or(|w| (w - measured).abs() > 0.5) {
            ui.ctx().data_mut(|d| d.insert_temp(id, measured));
            // A dialog's first frame is an invisible sizing pass, and
            // discarding it would drop focus requests made while opening.
            if width.is_some() {
                ui.ctx().request_discard("dialog buttons resized");
            } else {
                ui.ctx().request_repaint();
            }
        }
        row.inner
    })
    .inner
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
        let response = modal(ctx, egui::Id::new("confirm dialog")).show(ctx, |ui| {
            ui.set_width(360.0);
            if dialog_title(ui, &confirm.title) {
                answer = Some(false);
            }
            ui.label(&confirm.text);
            ui.add_space(12.0);
            dialog_buttons(ui, |ui| {
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
    fn a_dialog_body_leaves_room_for_the_rest_of_the_dialog_as_measured() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            let id = egui::Id::new("test dialog");
            // Before the dialog was shown, the guess holds the rest's room.
            assert_eq!(
                dialog_body_room(ctx, id, 100.0),
                600.0 - DIALOG_MARGIN - 100.0
            );
            // Larger text made the title and buttons 200 tall.
            measure_dialog(ctx, id, 500.0, 300.0);
            assert_eq!(
                dialog_body_room(ctx, id, 100.0),
                600.0 - DIALOG_MARGIN - 200.0
            );
            // However short the window, the body keeps a usable height.
            measure_dialog(ctx, id, 1000.0, 300.0);
            assert_eq!(dialog_body_room(ctx, id, 100.0), 80.0);
        });
    }

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

    fn drag(ctx: &egui::Context, from: egui::Pos2, to: egui::Pos2, show: &dyn Fn(&egui::Context)) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let middle = from.lerp(to, 0.5);
        for events in [
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(middle)],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
            vec![],
        ] {
            frame(ctx, events, show);
        }
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, show: &dyn Fn(&egui::Context)) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| show(ctx));
    }

    #[test]
    fn a_dialog_is_moved_by_its_title_stays_in_the_window_and_reopens_centered() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("movable");
        let show = |ctx: &egui::Context| {
            modal(ctx, id).show(ctx, |ui| {
                ui.set_width(300.0);
                dialog_title(ui, "Title");
                ui.label("Body");
            });
        };
        let rect = || ctx.memory(|m| m.area_rect(id)).unwrap();
        for _ in 0..4 {
            frame(&ctx, vec![], &show);
        }
        let start = rect();
        assert!((start.center() - egui::pos2(500.0, 400.0)).length() < 2.0);
        let grip = start.min + egui::vec2(120.0, 16.0);
        drag(&ctx, grip, grip + egui::vec2(150.0, 100.0), &show);
        let moved = rect().min - start.min;
        assert!(
            (moved - egui::vec2(150.0, 100.0)).length() < 2.0,
            "{moved:?}"
        );
        // Past the edge it stops there, and comes straight back.
        let grip = rect().min + egui::vec2(120.0, 16.0);
        drag(&ctx, grip, egui::pos2(2000.0, 16.0), &show);
        assert!(rect().max.x <= 1000.5, "{:?}", rect());
        let before = rect();
        let grip = before.min + egui::vec2(120.0, 16.0);
        drag(&ctx, grip, grip - egui::vec2(60.0, 0.0), &show);
        assert!((before.min.x - rect().min.x - 60.0).abs() < 2.0);
        // Closed and opened again.
        for _ in 0..3 {
            frame(&ctx, vec![], &|_| {});
        }
        for _ in 0..3 {
            frame(&ctx, vec![], &show);
        }
        assert!((rect().min - start.min).length() < 2.0);
    }

    #[test]
    fn a_centered_window_opens_in_the_middle_and_can_be_moved() {
        let ctx = egui::Context::default();
        let show = |ctx: &egui::Context| {
            centered(ctx, "centered", egui::Window::new("Centered"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| ui.set_width(300.0));
        };
        let rect = || {
            ctx.memory(|m| m.area_rect(egui::Id::new("centered")))
                .unwrap()
        };
        for _ in 0..4 {
            frame(&ctx, vec![], &show);
        }
        let start = rect();
        assert!((start.center() - egui::pos2(500.0, 400.0)).length() < 2.0);
        let grip = start.min + egui::vec2(40.0, 14.0);
        drag(&ctx, grip, grip + egui::vec2(150.0, 100.0), &show);
        assert!((rect().min - start.min - egui::vec2(150.0, 100.0)).length() < 2.0);
        for _ in 0..3 {
            frame(&ctx, vec![], &|_| {});
        }
        for _ in 0..3 {
            frame(&ctx, vec![], &show);
        }
        assert!((rect().center() - egui::pos2(500.0, 400.0)).length() < 2.0);
    }
}
