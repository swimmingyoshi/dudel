//! Keeps one widget with a broken position from closing Dudel.
//!
//! egui finds the widget under the pointer by comparing widget rectangles
//! with `==`. A rectangle holding NaN never equals itself, so once a widget is
//! laid out at NaN (a size divided by zero somewhere), the next pointer move
//! over it panics inside egui (`hit_test.rs`, "called `Option::unwrap()` on a
//! `None` value") and the window closes. This was seen while painting on
//! 2026-11-26.
//!
//! After each frame, [`repair`] gives every such widget a finite rectangle
//! before egui hit-tests the next frame, and writes one problem report per
//! widget naming it, so the cause can be found and fixed where it starts.
use std::collections::HashSet;
use std::sync::Mutex;

use eframe::egui;

/// Widgets already reported in this run.
static REPORTED: Mutex<Option<HashSet<egui::Id>>> = Mutex::new(None);

/// Whether `rect` is safe for egui's hit test: no NaN anywhere. (Infinite
/// sides are fine; `Rect::EVERYTHING` and `Rect::NOTHING` use them.)
fn sound(rect: egui::Rect) -> bool {
    !rect.any_nan()
}

/// Replace NaN widget rectangles of this frame with finite ones. Returns how
/// many widgets were repaired.
pub(crate) fn repair(ctx: &egui::Context) -> usize {
    let broken: Vec<(egui::WidgetRect, Option<egui::WidgetInfo>)> = ctx.viewport(|viewport| {
        let widgets = &viewport.this_pass.widgets;
        widgets
            .layers()
            .flat_map(|(_, rects)| rects.iter())
            .filter(|w| !sound(w.rect) || !sound(w.interact_rect))
            .map(|w| (*w, widgets.info(w.id).cloned()))
            .collect()
    });
    for (widget, info) in &broken {
        // What the pointer can still reach, or nothing if that is broken too.
        let rect = if sound(widget.interact_rect) {
            widget.interact_rect
        } else {
            egui::Rect::NOTHING
        };
        // Registering the same id again replaces its rectangles ("last
        // wins" in `WidgetRects::insert`) and keeps its sense.
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("widget guard").with(widget.id),
            egui::UiBuilder::new()
                .layer_id(widget.layer_id)
                .max_rect(rect),
        );
        ui.interact(rect, widget.id, egui::Sense::hover());
        report(widget, info.as_ref());
    }
    broken.len()
}

fn report(widget: &egui::WidgetRect, info: Option<&egui::WidgetInfo>) {
    let Ok(mut reported) = REPORTED.lock() else {
        return;
    };
    if !reported.get_or_insert_with(HashSet::new).insert(widget.id) {
        return;
    }
    let described = info.map_or_else(
        || "unknown".to_owned(),
        |info| format!("{:?} {:?}", info.typ, info.label.as_deref().unwrap_or("")),
    );
    // Tests break widgets on purpose; their reports would land in the
    // artist's real folder.
    if cfg!(test) {
        return;
    }
    crate::crash_report::note(&format!(
        "Dudel problem report (not fatal)\nVersion: {}\nA widget was laid out at an invalid position \
         and was repaired before it could close Dudel.\nWidget: {described}\nId: {:?}\nLayer: {:?}\n\
         Rect: {:?}\nInteract rect: {:?}\nSense: {:?}\n\nBacktrace (where it was found, not where \
         it was made):\n{}",
        env!("CARGO_PKG_VERSION"),
        widget.id,
        widget.layer_id,
        widget.rect,
        widget.interact_rect,
        widget.sense,
        std::backtrace::Backtrace::force_capture(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, pos: egui::Pos2, broken: bool) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            events: vec![egui::Event::PointerMoved(pos)],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // A click widget and a drag widget under the pointer, as a
                // button and a slider laid out after a picture sized 0 x
                // infinity. Their interact rectangles stay finite (clipping
                // drops NaN); their rectangles do not.
                let rect = if broken {
                    egui::Rect::from_min_size(egui::pos2(f32::NAN, 145.0), egui::vec2(10.0, 10.0))
                } else {
                    egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(400.0, 300.0))
                };
                ui.interact(rect, egui::Id::new("drag"), egui::Sense::drag());
                ui.interact(rect, egui::Id::new("click"), egui::Sense::click());
            });
            if broken {
                assert!(repair(ctx) > 0);
            }
        });
    }

    /// Without `repair` (comment out its call above), egui 0.33.3 panics in
    /// `hit_test` on the second frame, as in the 2026-11-26 report.
    #[test]
    fn a_widget_laid_out_at_nan_cannot_close_dudel() {
        let ctx = egui::Context::default();
        for _ in 0..2 {
            frame(&ctx, egui::pos2(150.0, 150.0), false);
        }
        for _ in 0..3 {
            frame(&ctx, egui::pos2(150.0, 150.0), true);
        }
        let broken = ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, rects)| rects.iter())
                .filter(|w| w.rect.any_nan() || w.interact_rect.any_nan())
                .count()
        });
        assert_eq!(broken, 0);
    }

    #[test]
    fn sound_rectangles_are_left_alone() {
        assert!(sound(egui::Rect::EVERYTHING) && sound(egui::Rect::NOTHING));
        assert!(!sound(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(f32::NAN, 1.0)
        )));
    }
}
