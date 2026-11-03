//! F resizes on the canvas: press F and move the pointer, and the brush
//! (or the reshape reach) follows a circle around a fixed center. A click,
//! Enter or F again keeps the size; Esc puts it back. Holding F and
//! letting go also keeps it.
use crate::{Dudel, egui, tools::Tool};
use egui::{Color32, Pos2, vec2};

/// Holding F at least this long makes letting go of it keep the size.
const HOLD: std::time::Duration = std::time::Duration::from_millis(350);

/// What F resizes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    /// The brush (or eraser) size: the circle is the tip.
    Brush,
    /// The Pointer's reshape reach: the circle's radius.
    Reach,
}

pub(crate) struct QuickResize {
    target: Target,
    /// The circle's center, in screen points.
    center: Pos2,
    original: f32,
    started: std::time::Instant,
}

impl QuickResize {
    /// The circle's radius in canvas pixels for `value`.
    fn radius(&self, value: f32) -> f32 {
        match self.target {
            Target::Brush => value * 0.5,
            Target::Reach => value,
        }
    }
}

impl Dudel {
    fn resize_target(&self) -> Option<Target> {
        match self.tool {
            Tool::Brush | Tool::Eraser => Some(Target::Brush),
            Tool::Pointer if self.line_drag == crate::vector_pick::LineDrag::Reshape => {
                Some(Target::Reach)
            }
            _ => None,
        }
    }
    fn resize_value(&self, target: Target) -> f32 {
        match target {
            Target::Brush => self.brush.size,
            Target::Reach => self.bend_reach,
        }
    }
    fn set_resize_value(&mut self, target: Target, value: f32) {
        match target {
            Target::Brush => self.brush.size = value.round().clamp(1.0, 300.0),
            Target::Reach => {
                self.bend_reach = value.round().clamp(2.0, crate::vector_pick::MAX_REACH)
            }
        }
    }
    /// F and the keys that end an F resize. Returns whether the keyboard
    /// belongs to the resize this frame.
    pub(crate) fn quick_resize_keys(&mut self, ctx: &egui::Context) -> bool {
        // Key repeats while F is held must neither start nor end a resize.
        let pressed = ctx.input_mut(|i| {
            let mut fresh = false;
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key: egui::Key::F,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } if modifiers.is_none() => {
                    fresh |= !*repeat;
                    false
                }
                _ => true,
            });
            fresh
        });
        let Some(resize) = &self.quick_resize else {
            if pressed && !self.stroking {
                self.start_quick_resize(ctx);
            }
            return self.quick_resize.is_some();
        };
        let held = resize.started.elapsed() >= HOLD;
        let (escape, enter, released) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.key_released(egui::Key::F),
            )
        });
        if escape {
            self.finish_quick_resize(false);
        } else if pressed || enter || (released && held) {
            self.finish_quick_resize(true);
        }
        true
    }
    fn start_quick_resize(&mut self, ctx: &egui::Context) {
        let Some(target) = self.resize_target() else {
            self.status = "F resizes the brush, or the reach while reshaping lines".into();
            return;
        };
        let value = self.resize_value(target);
        let pointer = ctx
            .input(|i| i.pointer.hover_pos())
            .filter(|p| self.canvas_area.contains(*p))
            .unwrap_or(self.canvas_area.center());
        let mut resize = QuickResize {
            target,
            center: pointer,
            original: value,
            started: std::time::Instant::now(),
        };
        // The pointer starts on the circle's edge, so nothing jumps.
        resize.center = pointer - vec2(resize.radius(value) * self.zoom, 0.0);
        self.quick_resize = Some(resize);
        self.status = "Move to resize · click, Enter or F keeps it · Esc cancels".into();
    }
    /// Follow the pointer: the circle's edge is where it is.
    pub(crate) fn update_quick_resize(&mut self, ctx: &egui::Context) {
        let Some(resize) = &self.quick_resize else {
            return;
        };
        if self.resize_target() != Some(resize.target) {
            self.finish_quick_resize(false);
            return;
        }
        let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let radius = (pointer - resize.center).length() / self.zoom.max(1e-3);
        let value = match resize.target {
            Target::Brush => radius * 2.0,
            Target::Reach => radius,
        };
        let target = resize.target;
        self.set_resize_value(target, value);
        ctx.request_repaint();
    }
    /// End the resize, keeping the new size or putting the old one back.
    pub(crate) fn finish_quick_resize(&mut self, keep: bool) {
        let Some(resize) = self.quick_resize.take() else {
            return;
        };
        if !keep {
            self.set_resize_value(resize.target, resize.original);
        }
        let value = self.resize_value(resize.target);
        self.status = match resize.target {
            Target::Brush => format!("Brush size {value:.0} px"),
            Target::Reach => format!("Reach {value:.0} px"),
        };
    }
    /// The circle being sized and its value.
    pub(crate) fn paint_quick_resize(&self, painter: &egui::Painter) {
        let Some(resize) = &self.quick_resize else {
            return;
        };
        let value = self.resize_value(resize.target);
        let radius = (resize.radius(value) * self.zoom).max(1.0);
        let accent = Color32::from_rgb(240, 160, 60);
        painter.circle_stroke(
            resize.center,
            radius,
            egui::Stroke::new(3.0_f32, Color32::from_black_alpha(110)),
        );
        painter.circle_stroke(resize.center, radius, egui::Stroke::new(1.5_f32, accent));
        painter.circle_filled(resize.center, 2.0, accent);
        let label = format!("{value:.0} px");
        let at = resize.center + vec2(0.0, -radius - 12.0);
        for (offset, color) in [
            (vec2(1.0, 1.0), Color32::BLACK),
            (vec2(0.0, 0.0), Color32::WHITE),
        ] {
            painter.text(
                at + offset,
                egui::Align2::CENTER_BOTTOM,
                &label,
                egui::FontId::proportional(14.0),
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame with these input events, through shortcuts and the canvas.
    fn frame(
        app: &mut Dudel,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        packets: &[crate::input::Packet],
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            app.shortcuts(ctx);
            app.canvas(ctx, packets);
        });
    }
    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn f_resizes_the_brush_and_the_reach_by_moving_the_pointer() {
        let mut app = Dudel {
            doc: crate::document::Document::new(96, 64),
            zoom: 4.0,
            auto_fit: false,
            ..Default::default()
        };
        app.select_tool(Tool::Brush);
        app.brush.size = 20.0;
        let ctx = egui::Context::default();
        let at = Pos2::new(400.0, 300.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)], &[]);
        // F, then 40 points right: the tip's radius grows by 10 canvas px.
        frame(&mut app, &ctx, vec![key(egui::Key::F, true)], &[]);
        assert!(app.quick_resize.is_some(), "{}", app.status);
        assert_eq!(app.brush.size, 20.0);
        let moved = at + vec2(40.0, 0.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(moved)], &[]);
        assert_eq!(app.brush.size, 40.0);
        // A quick tap's release keeps resizing; Esc puts the size back.
        frame(&mut app, &ctx, vec![key(egui::Key::F, false)], &[]);
        assert!(app.quick_resize.is_some());
        frame(&mut app, &ctx, vec![key(egui::Key::Escape, true)], &[]);
        assert!(app.quick_resize.is_none());
        assert_eq!(app.brush.size, 20.0);
        // A click keeps the size and paints nothing.
        frame(&mut app, &ctx, vec![key(egui::Key::F, true)], &[]);
        frame(&mut app, &ctx, vec![key(egui::Key::F, false)], &[]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(moved + vec2(20.0, 0.0))],
            &[],
        );
        let sample = crate::stroke::Sample {
            x: moved.x,
            y: moved.y,
            pressure: 1.0,
            pressure_known: true,
            ..Default::default()
        };
        frame(
            &mut app,
            &ctx,
            vec![],
            &[crate::input::Packet::Start(sample)],
        );
        frame(&mut app, &ctx, vec![], &[crate::input::Packet::End(sample)]);
        app.flush_render_queue();
        assert!(app.quick_resize.is_none());
        assert_eq!(app.brush.size, 30.0);
        let pixels = &app.doc.layers[app.doc.active].pixels;
        assert!((0..96 * 64).all(|i| pixels.rgba(i)[3] == 0));
        // With the Pointer reshaping lines, F sizes the reach (a radius).
        app.select_tool(Tool::Pointer);
        app.line_drag = crate::vector_pick::LineDrag::Reshape;
        app.bend_reach = 10.0;
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)], &[]);
        frame(&mut app, &ctx, vec![key(egui::Key::F, true)], &[]);
        assert!(app.quick_resize.is_some(), "{}", app.status);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(at + vec2(40.0, 0.0))],
            &[],
        );
        frame(&mut app, &ctx, vec![key(egui::Key::Enter, true)], &[]);
        assert!(app.quick_resize.is_none());
        assert_eq!(app.bend_reach, 20.0);
    }
}
