//! The animation timeline: frames as cells along the bottom of the window,
//! playback, frame holds, onion skin and the frame commands. The model is in
//! `animation`.

use crate::animation::{self, Animation, Frame, Playback};
use crate::document::Document;
use crate::document_edits::{Edit, Property, Target};
use crate::{Dudel, egui};

/// Width of one frame period in the strip.
const PERIOD_WIDTH: f32 = 22.0;
const CELL_HEIGHT: f32 = 46.0;

impl Dudel {
    /// The frames of the current project's animation, if it has one.
    pub(crate) fn animation_frames(&self) -> Option<(Animation, Vec<Frame>)> {
        let animation = self.doc.animation?;
        let frames = animation::frames(&self.doc, &animation);
        Some((animation, frames))
    }

    /// The frame on screen: the playing one, else the one holding the
    /// selected layer, else the first.
    pub(crate) fn current_frame(
        &self,
        doc: &Document,
        animation: &Animation,
        frames: &[Frame],
    ) -> Option<usize> {
        if frames.is_empty() {
            return None;
        }
        if let Some((started, start)) = self.animation_play {
            let seconds = started.elapsed().as_secs_f64();
            return Some(animation::frame_at(frames, animation, start, seconds));
        }
        Some(animation::frame_of(frames, doc.active).unwrap_or(0))
    }

    /// What the canvas shows of `base` when it is an animation: one frame,
    /// the background and foreground, and the onion skin while not playing.
    /// Also returns which frame and onion skin that is; when it differs from
    /// `animation_shown`, the whole canvas is redrawn.
    pub(crate) fn animation_display(
        &self,
        base: &Document,
    ) -> (Option<Document>, Option<(Option<usize>, bool)>) {
        let Some(animation) = base.animation else {
            return (None, None);
        };
        let frames = animation::frames(base, &animation);
        let frame = self.current_frame(base, &animation, &frames);
        let onion = self.animation_play.is_none()
            && animation.onion_opacity > 0.0
            && (animation.onion_before > 0 || animation.onion_after > 0);
        (
            Some(animation::frame_view(
                base, &animation, &frames, frame, onion,
            )),
            Some((frame, onion)),
        )
    }

    /// Turn the timeline on (with default settings) or off. Layers stay as
    /// they are either way.
    pub(crate) fn toggle_animation(&mut self) {
        self.animation_play = None;
        let next = if self.doc.animation.is_some() {
            None
        } else {
            Some(Animation::default())
        };
        self.dispatch_document_edit(
            Edit::Animation(next),
            if next.is_some() {
                "Animation on · each top-level layer or folder is a frame"
            } else {
                "Animation off · the layers stay as they are"
            },
            "Animation unchanged",
        );
    }

    fn set_animation(&mut self, animation: Animation) {
        if self.doc.animation != Some(animation) {
            self.dispatch_document_edit(
                Edit::Animation(Some(animation)),
                "Animation settings changed",
                "Animation unchanged",
            );
        }
    }

    pub(crate) fn toggle_playback(&mut self) {
        if let Some(stopped) = self.stop_playback() {
            self.status = format!("Stopped on frame {}", stopped + 1);
            return;
        }
        let Some((animation, frames)) = self.animation_frames() else {
            return;
        };
        if frames.len() < 2 {
            self.status = "Add a second frame to play the animation".into();
            return;
        }
        let start = animation::frame_of(&frames, self.doc.active).unwrap_or(0);
        // A finished Once animation plays again from the start.
        let start = if animation.playback == Playback::Once && start + 1 == frames.len() {
            0
        } else {
            start
        };
        self.close_input_gestures();
        self.animation_play = Some((std::time::Instant::now(), start));
        self.status = format!("Playing at {} fps", animation.fps);
    }

    /// Stop playing and select the frame it stopped on.
    pub(crate) fn stop_playback(&mut self) -> Option<usize> {
        let (animation, frames) = self.animation_frames()?;
        let frame = self.current_frame(&self.doc, &animation, &frames)?;
        self.animation_play.take()?;
        self.select_frame(&frames, frame);
        Some(frame)
    }

    fn select_frame(&mut self, frames: &[Frame], frame: usize) {
        if let Some(frame) = frames.get(frame)
            && animation::frame_of(frames, self.doc.active)
                != Some(frames.iter().position(|f| f == frame).unwrap())
        {
            self.select_layer_target(frame.entry);
        }
    }

    /// Step to the previous or next frame, wrapping around.
    pub(crate) fn step_frame(&mut self, forward: bool) {
        self.animation_play = None;
        let Some((_, frames)) = self.animation_frames() else {
            return;
        };
        if frames.is_empty() {
            return;
        }
        let current = animation::frame_of(&frames, self.doc.active).unwrap_or(0);
        let next = if forward {
            (current + 1) % frames.len()
        } else {
            (current + frames.len() - 1) % frames.len()
        };
        self.select_frame(&frames, next);
    }

    fn new_frame(&mut self) {
        self.animation_play = None;
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.dispatch_document_edit(Edit::NewFrame, "New frame", "No frame added");
    }

    /// Copy the current frame (the whole folder for a folder frame) above it.
    fn duplicate_frame(&mut self) {
        self.animation_play = None;
        let Some((_, frames)) = self.animation_frames() else {
            return;
        };
        let Some(frame) = animation::frame_of(&frames, self.doc.active).and_then(|f| frames.get(f))
        else {
            self.status = "Select a frame to duplicate".into();
            return;
        };
        if self.doc.active != frame.entry && !self.select_layer_target(frame.entry) {
            return;
        }
        self.duplicate_layer();
    }

    fn set_hold(&mut self, entry: usize, hold: u16) {
        let Some(id) = self.doc.layers.get(entry).map(|layer| layer.id) else {
            return;
        };
        self.dispatch_document_edit(
            Edit::Property {
                target: Target::Identity(id),
                property: Property::Hold(hold.clamp(1, animation::MAX_HOLD)),
            },
            "Frame hold changed",
            "Frame hold unchanged",
        );
    }

    /// Keep the canvas moving while playing, and stop a Once animation at
    /// its end.
    fn tick_playback(&mut self, ctx: &egui::Context) {
        let Some((started, start)) = self.animation_play else {
            return;
        };
        let Some((animation, frames)) = self.animation_frames() else {
            self.animation_play = None;
            return;
        };
        if frames.len() < 2 {
            self.animation_play = None;
            return;
        }
        if animation.playback == Playback::Once {
            let offset: u32 = frames[..start.min(frames.len())]
                .iter()
                .map(|f| u32::from(f.hold))
                .sum();
            let periods = started.elapsed().as_secs_f64() * f64::from(animation.fps);
            if periods >= f64::from(animation::length(&frames).saturating_sub(offset)) {
                self.stop_playback();
                return;
            }
        }
        let period = animation::period(&animation);
        let into = started.elapsed().as_secs_f64() % period;
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(
            (period - into).max(0.001),
        ));
    }

    /// The timeline panel, while the project is an animation.
    pub(crate) fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        self.tick_playback(ui.ctx());
        let Some((mut animation, frames)) = self.animation_frames() else {
            return;
        };
        let current = self.current_frame(&self.doc, &animation, &frames);
        let playing = self.animation_play.is_some();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let play = if playing {
                "\u{23f8} Pause"
            } else {
                "\u{25b6} Play"
            };
            if ui
                .button(play)
                .on_hover_text("Play or pause the animation (Space in the timeline)")
                .clicked()
            {
                self.toggle_playback();
            }
            if ui
                .button("\u{25c0}")
                .on_hover_text("Previous frame")
                .clicked()
            {
                self.step_frame(false);
            }
            if ui.button("\u{25b6}").on_hover_text("Next frame").clicked() {
                self.step_frame(true);
            }
            ui.label(match current {
                Some(frame) => format!("Frame {} of {}", frame + 1, frames.len()),
                None => "No frames".into(),
            });
            ui.separator();
            let mut fps = animation.fps;
            ui.add(
                egui::DragValue::new(&mut fps)
                    .range(animation::FPS)
                    .suffix(" fps"),
            )
            .on_hover_text("Frames per second");
            animation.fps = fps;
            egui::ComboBox::from_id_salt("animation playback")
                .selected_text(animation.playback.name())
                .show_ui(ui, |ui| {
                    for mode in Playback::ALL {
                        ui.selectable_value(&mut animation.playback, mode, mode.name());
                    }
                });
            ui.menu_button("Onion skin", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Before");
                    ui.add(
                        egui::DragValue::new(&mut animation.onion_before)
                            .range(0..=animation::MAX_ONION),
                    );
                    ui.label("After");
                    ui.add(
                        egui::DragValue::new(&mut animation.onion_after)
                            .range(0..=animation::MAX_ONION),
                    );
                });
                ui.add(egui::Slider::new(&mut animation.onion_opacity, 0.0..=1.0).text("Opacity"));
                ui.checkbox(&mut animation.onion_tint, "Tint (earlier red, later green)");
            })
            .response
            .on_hover_text("Faint copies of the frames before and after, while drawing");
            ui.checkbox(&mut animation.background, "Background")
                .on_hover_text("The bottom layer or folder stays under every frame");
            ui.checkbox(&mut animation.foreground, "Foreground")
                .on_hover_text("The top layer or folder stays over every frame");
            ui.separator();
            if ui
                .button("+ Frame")
                .on_hover_text("A new empty frame after this one")
                .clicked()
            {
                self.new_frame();
            }
            if ui
                .button("Duplicate")
                .on_hover_text("Copy this frame after itself")
                .clicked()
            {
                self.duplicate_frame();
            }
            if ui
                .button("Export…")
                .on_hover_text("Animated GIF, PNG frames, a sprite sheet or an MP4 video")
                .clicked()
            {
                self.animation_play = None;
                self.animation_export.open = true;
            }
            if let Some(frame) = current.and_then(|f| frames.get(f)) {
                let mut hold = frame.hold;
                let response = ui
                    .add(
                        egui::DragValue::new(&mut hold)
                            .range(1..=animation::MAX_HOLD)
                            .prefix("Hold "),
                    )
                    .on_hover_text("How many frame periods this frame shows for");
                if response.changed() && hold != frame.hold {
                    self.set_hold(frame.entry, hold);
                }
            }
        });
        self.set_animation(animation);
        let (background, foreground) = animation::fixed(&self.doc, &animation);
        egui::ScrollArea::horizontal()
            .id_salt("timeline frames")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    if let Some(entry) = background {
                        self.fixed_cell(ui, entry, "BG");
                    }
                    let mut chosen = None;
                    for (index, frame) in frames.iter().enumerate() {
                        if self.frame_cell(ui, index, frame, current == Some(index)) {
                            chosen = Some(index);
                        }
                    }
                    if let Some(entry) = foreground {
                        self.fixed_cell(ui, entry, "FG");
                    }
                    if let Some(index) = chosen {
                        self.animation_play = None;
                        self.select_frame(&frames, index);
                    }
                });
            });
        ui.add_space(2.0);
    }

    /// A frame's cell: its thumbnail, number and hold. True when clicked.
    fn frame_cell(&self, ui: &mut egui::Ui, index: usize, frame: &Frame, current: bool) -> bool {
        let width = PERIOD_WIDTH * f32::from(frame.hold).min(12.0) + 18.0;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, CELL_HEIGHT), egui::Sense::click());
        let visuals = ui.visuals();
        let painter = ui.painter_at(rect);
        let fill = if current {
            visuals.selection.bg_fill
        } else {
            visuals.faint_bg_color
        };
        painter.rect_filled(rect, 4.0, fill);
        if let Some(texture) = self.layer_thumbnails.get(&self.doc.layers[frame.entry].id) {
            let side = CELL_HEIGHT - 16.0;
            let thumb =
                egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 3.0), egui::vec2(side, side));
            painter.image(
                texture.id(),
                thumb,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        painter.text(
            egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            if frame.hold > 1 {
                format!("{} ×{}", index + 1, frame.hold)
            } else {
                format!("{}", index + 1)
            },
            egui::FontId::proportional(11.0),
            visuals.text_color(),
        );
        response
            .on_hover_text(format!(
                "{} · frame {}, shows for {} frame period{}",
                self.doc.layers[frame.entry].name,
                index + 1,
                frame.hold,
                if frame.hold == 1 { "" } else { "s" }
            ))
            .clicked()
    }

    /// The background or foreground's cell.
    fn fixed_cell(&self, ui: &mut egui::Ui, entry: usize, label: &str) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(34.0, CELL_HEIGHT), egui::Sense::hover());
        let visuals = ui.visuals();
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0_f32, visuals.weak_text_color()),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            visuals.weak_text_color(),
        );
        response.on_hover_text(format!(
            "{} · shows with every frame",
            self.doc.layers[entry].name
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_added_on_top_level_and_play_and_stop_on_a_frame() {
        let mut app = Dudel::default();
        app.toggle_animation();
        app.flush_render_queue();
        assert!(app.doc.animation.is_some());
        // Group the first layer into a folder, so the frame is a folder.
        app.group_into_folder();
        app.flush_render_queue();
        assert!(app.doc.layers[app.doc.active].folder);
        app.new_frame();
        app.flush_render_queue();
        app.new_frame();
        app.flush_render_queue();
        let (_, frames) = app.animation_frames().unwrap();
        assert_eq!(frames.len(), 3, "the new layers are top-level frames");
        assert!(app.doc.layers.iter().filter(|l| l.depth == 0).count() == 3);
        assert_eq!(animation::frame_of(&frames, app.doc.active), Some(2));
        app.step_frame(true);
        app.flush_render_queue();
        let (_, frames) = app.animation_frames().unwrap();
        assert_eq!(
            animation::frame_of(&frames, app.doc.active),
            Some(0),
            "wraps"
        );
        app.set_hold(frames[0].entry, 4);
        app.flush_render_queue();
        assert_eq!(app.doc.layers[frames[0].entry].hold, 4);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[frames[0].entry].hold, 1);
        // Playing starts on the selected frame and stops on a frame.
        app.toggle_playback();
        assert!(app.animation_play.is_some());
        let stopped = app.stop_playback();
        assert_eq!(stopped, Some(0));
        assert!(app.animation_play.is_none());
        // The canvas shows one frame; the layers themselves stay visible.
        let shown = app.animation_display(&app.doc.clone()).0.unwrap();
        assert_eq!(
            shown
                .layers
                .iter()
                .filter(|l| l.depth == 0 && l.visible && l.opacity == 1.0)
                .count(),
            1
        );
        assert!(app.doc.layers.iter().all(|l| l.visible));
        app.toggle_animation();
        app.flush_render_queue();
        assert!(app.doc.animation.is_none());
    }
}
