//! Controls the styled layouts share.
use eframe::egui::{self, Color32, Rect, RichText, Sense, Stroke, vec2};
use std::ops::RangeInclusive;

/// Where `value` sits in `range`, 0 to 1, on a log scale when asked.
pub fn fraction(value: f32, range: &RangeInclusive<f32>, log: bool) -> f32 {
    let (lo, hi) = (*range.start(), *range.end());
    let t = if log && lo > 0.0 {
        (value.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln())
    } else {
        (value - lo) / (hi - lo)
    };
    if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// A compact number field: drag across it to change the value, click to
/// type one. A thin bar along the bottom shows where the value sits.
///
/// Returns the number's response; `changed()` reports edits.
pub fn scrub_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    suffix: &str,
    log: bool,
) -> egui::Response {
    let accent = super::style::Active::get(ui.ctx()).accent;
    let well = ui.visuals().extreme_bg_color;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let height = ui.spacing().interact_size.y;
    let inner = egui::Frame::new()
        .fill(well)
        .corner_radius(radius)
        .inner_margin(egui::Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.set_height(height);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(label).weak());
                // The number has no box of its own: the field is the box.
                let w = &mut ui.visuals_mut().widgets;
                for state in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                    state.weak_bg_fill = Color32::TRANSPARENT;
                    state.bg_fill = Color32::TRANSPARENT;
                    state.bg_stroke = Stroke::NONE;
                }
                ui.spacing_mut().button_padding.x = 2.0;
                ui.spacing_mut().interact_size.x = 20.0;
                // Large values move faster, so a log range feels even.
                let span = range.end() - range.start();
                let speed = if log {
                    (value.abs() * 0.012).max(0.05)
                } else {
                    span / 300.0
                };
                ui.add(
                    egui::DragValue::new(value)
                        .range(range.clone())
                        .speed(speed)
                        .suffix(suffix)
                        .max_decimals(if span <= 2.0 { 2 } else { 0 }),
                )
            })
            .inner
        });
    let rect = inner.response.rect;
    let t = fraction(*value, &range, log);
    let bar = Rect::from_min_size(
        rect.left_bottom() - vec2(0.0, 2.0),
        vec2(rect.width() * t, 2.0),
    );
    ui.painter()
        .rect_filled(bar, 0.0, accent.gamma_multiply(0.9));
    inner.inner
}

/// A row of flat tabs with the chosen one underlined in the accent.
/// Returns the index clicked, if any.
pub fn tab_strip(ui: &mut egui::Ui, tabs: &[&str], selected: usize) -> Option<usize> {
    let accent = super::style::Active::get(ui.ctx()).accent;
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (index, name) in tabs.iter().enumerate() {
            let chosen = index == selected;
            let text = RichText::new(name.to_uppercase()).size(11.5);
            let text = if chosen { text.strong() } else { text.weak() };
            let galley = egui::WidgetText::from(text).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );
            let size = vec2(galley.size().x + 18.0, 26.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            if response.hovered() && !chosen {
                ui.painter().rect_filled(
                    rect.shrink(2.0),
                    4.0,
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
            let color = if chosen {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            ui.painter()
                .galley(rect.center() - galley.size() / 2.0, galley, color);
            if chosen {
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        rect.left_bottom() - vec2(-6.0, 2.0),
                        rect.right_bottom() - vec2(6.0, 0.0),
                    ),
                    1.0,
                    accent,
                );
            }
            if response.clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// Choices in one rounded track; the chosen one is raised.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    let track = ui.visuals().extreme_bg_color;
    let raised = ui.visuals().window_fill;
    egui::Frame::new()
        .fill(track)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius.at_least(4))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for &(option, name) in options {
                    let chosen = *value == option;
                    let button = egui::Button::new(if chosen {
                        RichText::new(name).strong()
                    } else {
                        RichText::new(name).weak()
                    })
                    .fill(if chosen { raised } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .min_size(vec2(0.0, ui.spacing().interact_size.y - 6.0));
                    if ui.add(button).clicked() && !chosen {
                        *value = option;
                        changed = true;
                    }
                }
            });
        });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_follow_the_scale() {
        assert_eq!(fraction(50.0, &(0.0..=100.0), false), 0.5);
        assert_eq!(fraction(-5.0, &(0.0..=100.0), false), 0.0);
        let mid = fraction(10.0, &(1.0..=100.0), true);
        assert!((mid - 0.5).abs() < 1e-4);
        assert_eq!(fraction(5.0, &(5.0..=5.0), false), 0.0);
    }
}
