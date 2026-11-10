//! The Adjustment window: settings of the selected adjustment layer. Changes
//! apply live; a drag becomes one undo step when the pointer lifts, and while
//! the render queue is busy the gesture is held and sent as one edit.
use crate::{
    Dudel,
    adjustment::{Adjustment, LevelsChannel, Settings},
    egui,
};
use egui::{Color32, Pos2, Sense, Stroke, pos2, vec2};

/// Counts of the image below the selected levels layer, read in the
/// background and kept until the document changes.
#[derive(Default)]
pub(crate) struct Histogram {
    job: Option<(HistogramKey, crate::background::Handle<HistogramResult>)>,
    done: Option<(HistogramKey, Counts)>,
}
/// The layer and the document revision counted.
type HistogramKey = (crate::document::LayerId, u64);
type Counts = [[u32; 256]; 3];
type HistogramResult = Result<Counts, String>;

impl Histogram {
    /// Most pixels counted; the rest of the image is skipped evenly.
    const SAMPLES: usize = 1 << 18;

    /// The counts for `target`, starting a count when the document changed.
    /// Until it finishes, the previous counts for the same layer stand in.
    fn poll(
        &mut self,
        doc: &crate::document::Document,
        target: crate::document::LayerId,
        revision: u64,
        ctx: &egui::Context,
    ) -> Option<Counts> {
        let key = (target, revision);
        if self.job.as_ref().is_some_and(|(_, job)| job.is_finished())
            && let Some((counted, job)) = self.job.take()
            && let Ok(Ok(counts)) = job.join()
        {
            self.done = Some((counted, counts));
        }
        if self.job.is_none()
            && self.done.as_ref().is_none_or(|(done, _)| *done != key)
            && let Some(index) = doc.layers.iter().position(|l| l.id == target)
        {
            let doc = doc.clone();
            self.job = Some((
                key,
                crate::background::spawn(move || {
                    doc.histogram_below(index, Self::SAMPLES, &|| false)
                }),
            ));
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.done
            .as_ref()
            .filter(|((layer, _), _)| *layer == target)
            .map(|(_, counts)| *counts)
    }
}

/// An adjustment change being dragged.
pub(crate) struct Edit {
    target: crate::document::LayerId,
    original: Adjustment,
    modified: bool,
    /// The gesture's latest settings while the render queue is busy.
    queued: Option<Settings>,
}

impl Dudel {
    /// The Adjustment window, shown while an adjustment layer is selected.
    pub(crate) fn adjustment_window(&mut self, ctx: &egui::Context) {
        if !self.render_queue.busy() {
            self.adjustment_sent = None;
        }
        let selected = self.input_target().and_then(|target| {
            let layer = self.doc.layers.iter().find(|l| l.id == target)?;
            Some((target, layer.name.clone(), layer.adjustment.clone()?))
        });
        let Some((target, name, adjustment)) = selected.filter(|_| self.adjustment_window_open)
        else {
            self.finish_adjustment_edit();
            return;
        };
        let mut open = true;
        egui::Window::new("Adjustment")
            .open(&mut open)
            .resizable(false)
            .min_width(300.0)
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.spacing_mut().slider_width = 150.0;
                ui.strong(format!("{name} · {}", adjustment.name()));
                let base = self
                    .adjustment_edit
                    .as_ref()
                    .filter(|edit| edit.target == target)
                    .and_then(|edit| edit.queued.clone())
                    .or_else(|| {
                        self.adjustment_sent
                            .as_ref()
                            .filter(|(sent, _)| *sent == target)
                            .map(|(_, settings)| settings.clone())
                    })
                    .unwrap_or_else(|| adjustment.settings().clone());
                let histogram = if matches!(base, Settings::Levels { .. }) {
                    self.levels_histogram
                        .poll(&self.doc, target, self.revision, ui.ctx())
                } else {
                    None
                };
                let mut settings = base.clone();
                settings_ui(ui, &mut settings, histogram.as_ref());
                if settings != base {
                    self.set_adjustment_settings(target, settings);
                }
            });
        self.adjustment_window_open = open;
        if !ctx.input(|i| i.pointer.any_down()) {
            self.finish_adjustment_edit();
        }
    }

    /// Apply changed settings: live under one gesture, or held for one
    /// queued edit while the render queue is busy.
    pub(crate) fn set_adjustment_settings(
        &mut self,
        target: crate::document::LayerId,
        settings: Settings,
    ) {
        let adjustment = match Adjustment::try_from(settings.clone()) {
            Ok(adjustment) => adjustment,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.adjustment_edit.as_ref().is_some_and(|edit| {
            edit.target != target || edit.queued.is_some() != self.render_queue.busy()
        }) {
            self.finish_adjustment_edit();
        }
        let Some(index) = self.doc.layers.iter().position(|l| l.id == target) else {
            return;
        };
        let Some(current) = self.doc.layers[index].adjustment.clone() else {
            return;
        };
        if self.render_queue.busy() {
            match &mut self.adjustment_edit {
                Some(edit) => edit.queued = Some(settings),
                None => {
                    self.adjustment_edit = Some(Edit {
                        target,
                        original: current,
                        modified: self.modified,
                        queued: Some(settings),
                    });
                }
            }
            return;
        }
        if self.adjustment_edit.is_none() {
            self.finish_layer_rename(true);
            self.close_input_gestures();
            self.history.begin_gesture_checkpoint(&self.doc);
            self.adjustment_edit = Some(Edit {
                target,
                original: current,
                modified: self.modified,
                queued: None,
            });
        }
        self.doc.layers[index].adjustment = Some(adjustment);
        self.changed();
    }

    pub(crate) fn finish_adjustment_edit(&mut self) {
        let Some(edit) = self.adjustment_edit.take() else {
            return;
        };
        if let Some(queued) = edit.queued {
            if queued != *edit.original.settings() {
                self.set_canvas_property(
                    edit.target,
                    crate::document_edits::Property::Adjustment(Adjustment::new(queued.clone())),
                );
                self.adjustment_sent = Some((edit.target, queued));
            }
            return;
        }
        let unchanged = self
            .doc
            .layers
            .iter()
            .find(|l| l.id == edit.target)
            .is_none_or(|l| l.adjustment.as_ref() == Some(&edit.original));
        if unchanged {
            self.history.discard_pending_checkpoint(&self.doc);
            self.modified = edit.modified;
        } else {
            self.history.settle(&self.doc);
        }
    }
}

const CHANNELS: [&str; 4] = ["RGB", "Red", "Green", "Blue"];
const IDENTITY: LevelsChannel = LevelsChannel {
    input: [0.0, 255.0],
    output: [0.0, 255.0],
    gamma: 1.0,
};

/// Which channel a levels or curves editor shows, remembered per window.
fn channel_picker(ui: &mut egui::Ui) -> usize {
    picker(ui, "Channel", &CHANNELS)
}

/// One of `names`, remembered per window under `label`.
fn picker(ui: &mut egui::Ui, label: &str, names: &[&str]) -> usize {
    let id = ui.id().with(("adjustment picker", label));
    let mut index = ui
        .data(|d| d.get_temp::<usize>(id))
        .unwrap_or(0)
        .min(names.len() - 1);
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        for (i, name) in names.iter().enumerate() {
            ui.selectable_value(&mut index, i, *name);
        }
    });
    ui.data_mut(|d| d.insert_temp(id, index));
    index
}

fn color_button(ui: &mut egui::Ui, label: &str, color: &mut [f32; 3]) {
    let before = color.map(|v| v.round().clamp(0.0, 255.0) as u8);
    let mut rgb = before;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
    });
    if rgb != before {
        *color = rgb.map(f32::from);
    }
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    crate::workspace::labeled_slider(ui, label, egui::Slider::new(value, range).max_decimals(2));
}

/// Controls for an adjustment's settings.
pub(crate) fn settings_ui(ui: &mut egui::Ui, settings: &mut Settings, histogram: Option<&Counts>) {
    match settings {
        Settings::Invert => {
            ui.label("Inverts the colors below. There is nothing to set.");
        }
        Settings::Posterize { levels } => {
            crate::workspace::labeled_slider(ui, "Levels", egui::Slider::new(levels, 2..=255));
        }
        Settings::Threshold { level } => {
            crate::workspace::labeled_slider(ui, "Threshold", egui::Slider::new(level, 1..=255));
        }
        Settings::Levels { channels } => {
            channels.resize(4, None);
            let index = channel_picker(ui);
            let before = channels[index].unwrap_or(IDENTITY);
            let mut value = before;
            match histogram {
                Some(counts) => histogram_graph(ui, counts, index, value.input),
                None => {
                    ui.small("Reading the image below…");
                }
            }
            slider(ui, "Input black", &mut value.input[0], 0.0..=254.0);
            slider(ui, "Input white", &mut value.input[1], 1.0..=255.0);
            crate::workspace::labeled_slider(
                ui,
                "Midtones",
                egui::Slider::new(&mut value.gamma, 0.1..=9.99).logarithmic(true),
            );
            slider(ui, "Output black", &mut value.output[0], 0.0..=255.0);
            slider(ui, "Output white", &mut value.output[1], 0.0..=255.0);
            value.input[1] = value.input[1].max(value.input[0] + 1.0);
            if value != before {
                channels[index] = Some(value);
            }
        }
        Settings::Curves { channels, bezier } => {
            let index = channel_picker(ui) as u8;
            let at = channels.iter().position(|(c, _)| *c == index);
            let mut points = at.map_or(vec![[0, 0], [255, 255]], |at| channels[at].1.clone());
            if curve_editor(ui, &mut points, *bezier) {
                match at {
                    Some(at) => channels[at].1 = points,
                    None => channels.push((index, points)),
                }
            }
            ui.small("Click to add a point, drag to move it, double-click to remove it.");
        }
        Settings::BrightnessContrast {
            brightness,
            contrast,
            legacy,
        } => {
            slider(ui, "Brightness", brightness, -150.0..=150.0);
            slider(
                ui,
                "Contrast",
                contrast,
                if *legacy {
                    -100.0..=100.0
                } else {
                    -50.0..=100.0
                },
            );
            ui.checkbox(legacy, "Use legacy");
        }
        Settings::Exposure {
            exposure,
            offset,
            gamma,
            ..
        } => {
            slider(ui, "Exposure", exposure, -5.0..=5.0);
            slider(ui, "Offset", offset, -0.5..=0.5);
            crate::workspace::labeled_slider(
                ui,
                "Gamma",
                egui::Slider::new(gamma, 0.01..=9.99).logarithmic(true),
            );
        }
        Settings::HueSaturationLuminosity {
            hue,
            saturation,
            luminosity,
        } => {
            crate::workspace::labeled_slider(ui, "Hue", egui::Slider::new(hue, -180..=180));
            crate::workspace::labeled_slider(
                ui,
                "Saturation",
                egui::Slider::new(saturation, -100..=100),
            );
            crate::workspace::labeled_slider(
                ui,
                "Luminosity",
                egui::Slider::new(luminosity, -100..=100),
            );
        }
        Settings::HueSaturation {
            colorize,
            master,
            ranges,
        } => {
            let mut on = colorize.is_some();
            if ui.checkbox(&mut on, "Colorize").changed() {
                *colorize = on.then_some([0.0, 25.0, 0.0]);
            }
            match colorize {
                Some(tint) => {
                    slider(ui, "Hue", &mut tint[0], 0.0..=360.0);
                    slider(ui, "Saturation", &mut tint[1], 0.0..=100.0);
                    slider(ui, "Lightness", &mut tint[2], -100.0..=100.0);
                }
                None => hue_ranges_ui(ui, master, ranges),
            }
        }
        Settings::GradientMap { stops, reverse } => gradient_map_ui(ui, stops, reverse),
        Settings::ColorBalance {
            tones,
            preserve_luminosity,
        } => {
            let tone = picker(ui, "Tone", &["Shadows", "Midtones", "Highlights"]);
            let shifts = &mut tones[tone];
            slider(ui, "Cyan – Red", &mut shifts[0], -100.0..=100.0);
            slider(ui, "Magenta – Green", &mut shifts[1], -100.0..=100.0);
            slider(ui, "Yellow – Blue", &mut shifts[2], -100.0..=100.0);
            ui.checkbox(preserve_luminosity, "Preserve luminosity");
        }
        Settings::Vibrance {
            vibrance,
            saturation,
        } => {
            slider(ui, "Vibrance", vibrance, -100.0..=100.0);
            slider(ui, "Saturation", saturation, -100.0..=100.0);
        }
        Settings::BlackWhite { weights, tint } => {
            let names = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];
            for (weight, name) in weights.iter_mut().zip(names) {
                slider(ui, name, weight, -200.0..=300.0);
            }
            let mut on = tint.is_some();
            if ui.checkbox(&mut on, "Tint").changed() {
                *tint = on.then_some([225.0, 211.0, 179.0]);
            }
            if let Some(tint) = tint {
                color_button(ui, "Tint color", tint);
            }
            if ui.small_button("Reset weights").clicked() {
                *weights = crate::adjustment::BLACK_WHITE_DEFAULT;
            }
        }
        Settings::PhotoFilter {
            color,
            density,
            preserve_luminosity,
        } => {
            color_button(ui, "Filter color", color);
            slider(ui, "Density", density, 0.0..=100.0);
            ui.checkbox(preserve_luminosity, "Preserve luminosity");
        }
        Settings::ChannelMixer { rows, monochrome } => {
            ui.checkbox(monochrome, "Monochrome");
            let row = if *monochrome {
                0
            } else {
                picker(ui, "Output", &["Red", "Green", "Blue"])
            };
            let values = &mut rows[row];
            slider(ui, "Red", &mut values[0], -200.0..=200.0);
            slider(ui, "Green", &mut values[1], -200.0..=200.0);
            slider(ui, "Blue", &mut values[2], -200.0..=200.0);
            slider(ui, "Constant", &mut values[3], -200.0..=200.0);
        }
        Settings::SelectiveColor { colors, absolute } => {
            let group = picker(
                ui,
                "Colors",
                &[
                    "Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas", "Whites",
                    "Neutrals", "Blacks",
                ],
            );
            let inks = &mut colors[group];
            for (ink, name) in inks.iter_mut().zip(["Cyan", "Magenta", "Yellow", "Black"]) {
                slider(ui, name, ink, -100.0..=100.0);
            }
            ui.horizontal(|ui| {
                ui.selectable_value(absolute, false, "Relative");
                ui.selectable_value(absolute, true, "Absolute");
            });
        }
    }
}

/// Master hue, saturation and lightness, or those of one color range
/// (Photoshop's six), picked by name; ranges with changes are marked.
fn hue_ranges_ui(ui: &mut egui::Ui, master: &mut [f32; 3], ranges: &mut Vec<([f32; 4], [f32; 3])>) {
    use crate::adjustment::{HUE_RANGES, hue_range_slot};
    const NAMES: [&str; 6] = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas"];
    let changed: Vec<String> = std::iter::once("Master".to_string())
        .chain(NAMES.iter().enumerate().map(|(slot, name)| {
            let marked = ranges
                .iter()
                .any(|(range, change)| hue_range_slot(*range) == slot && *change != [0.0; 3]);
            if marked {
                format!("{name} •")
            } else {
                (*name).to_string()
            }
        }))
        .collect();
    let names: Vec<&str> = changed.iter().map(String::as_str).collect();
    let group = picker(ui, "Edit", &names);
    if group == 0 {
        slider(ui, "Hue", &mut master[0], -180.0..=180.0);
        slider(ui, "Saturation", &mut master[1], -100.0..=100.0);
        slider(ui, "Lightness", &mut master[2], -100.0..=100.0);
        return;
    }
    let slot = group - 1;
    let at = ranges
        .iter()
        .position(|(range, _)| hue_range_slot(*range) == slot);
    let before = at.map_or([0.0; 3], |at| ranges[at].1);
    let mut change = before;
    slider(ui, "Hue", &mut change[0], -180.0..=180.0);
    slider(ui, "Saturation", &mut change[1], -100.0..=100.0);
    slider(ui, "Lightness", &mut change[2], -100.0..=100.0);
    if change != before {
        match at {
            Some(at) => ranges[at].1 = change,
            None if ranges.len() < HUE_RANGES.len() => ranges.push((HUE_RANGES[slot], change)),
            None => {}
        }
    }
    if let Some(at) = at
        && ranges[at].0 != HUE_RANGES[slot]
    {
        let [from, .., to] = ranges[at].0;
        ui.small(format!("Hues {from:.0}°–{to:.0}°, from the source file."));
    }
}

/// The map as a bar (shadows on the left), reverse, and its color stops.
fn gradient_map_ui(ui: &mut egui::Ui, stops: &mut Vec<(f32, [f32; 3], f32)>, reverse: &mut bool) {
    use crate::adjustment::ramp;
    let to_u8 = |c: [f32; 3]| c.map(|v| v.round().clamp(0.0, 255.0) as u8);
    let flipped = *reverse;
    crate::effects_panel::gradient_bar(ui, |t| {
        let t = if flipped { 1.0 - t } else { t };
        let [r, g, b] = to_u8(ramp(stops, t as f32));
        [r, g, b, 255]
    });
    ui.checkbox(reverse, "Reverse");
    ui.small("Dark tones take the left colors, light tones the right.");
    let mut colors: Vec<(f64, [u8; 3])> = stops
        .iter()
        .map(|(at, color, _)| (f64::from(*at), to_u8(*color)))
        .collect();
    crate::effects_panel::color_stops(ui, &mut colors, |t| to_u8(ramp(stops, t as f32)));
    colors.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Segment midpoints stay with the stop at the same place in the list.
    let updated: Vec<(f32, [f32; 3], f32)> = colors
        .iter()
        .enumerate()
        .map(|(i, (at, color))| {
            (
                (*at as f32).clamp(0.0, 1.0),
                color.map(f32::from),
                stops.get(i).map_or(0.5, |s| s.2),
            )
        })
        .collect();
    // Unchanged stops keep their exact values (colors are not rounded).
    let same = updated.len() == stops.len()
        && updated
            .iter()
            .zip(stops.iter())
            .all(|(a, b)| a.0 == b.0 && to_u8(a.1) == to_u8(b.1));
    if !same {
        *stops = updated;
    }
}

/// The counts of `channel` (0 for all three) as a graph, with the input
/// black and white points marked.
fn histogram_graph(ui: &mut egui::Ui, counts: &Counts, channel: usize, input: [f32; 2]) {
    let bins: Vec<u32> = (0..256)
        .map(|v| match channel {
            0 => counts.iter().map(|c| c[v]).sum(),
            c => counts[c - 1][v],
        })
        .collect();
    // Pure black and white (often paper) would flatten everything else.
    let tallest = bins[1..255].iter().copied().max().unwrap_or(0).max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(256.0, 72.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let color = match channel {
        1 => Color32::from_rgb(220, 70, 70),
        2 => Color32::from_rgb(70, 180, 90),
        3 => Color32::from_rgb(80, 120, 230),
        _ => visuals.text_color(),
    };
    for (v, count) in bins.iter().enumerate() {
        let height = (*count as f32 / tallest).min(1.0) * rect.height();
        if height > 0.0 {
            let x = rect.left() + v as f32 + 0.5;
            painter.line_segment(
                [pos2(x, rect.bottom()), pos2(x, rect.bottom() - height)],
                Stroke::new(1.0_f32, color),
            );
        }
    }
    for at in input {
        let x = rect.left() + at.clamp(0.0, 255.0) + 0.5;
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, visuals.weak_text_color()),
        );
    }
}

/// A curves graph: input along the bottom, output up the side. Returns
/// whether the points changed.
fn curve_editor(ui: &mut egui::Ui, points: &mut Vec<[u8; 2]>, bezier: bool) -> bool {
    let side = ui.available_width().clamp(160.0, 256.0);
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let faint = Stroke::new(1.0_f32, visuals.weak_text_color().gamma_multiply(0.4));
    for quarter in 1..4 {
        let t = quarter as f32 / 4.0 * side;
        painter.line_segment(
            [
                pos2(rect.left() + t, rect.top()),
                pos2(rect.left() + t, rect.bottom()),
            ],
            faint,
        );
        painter.line_segment(
            [
                pos2(rect.left(), rect.top() + t),
                pos2(rect.right(), rect.top() + t),
            ],
            faint,
        );
    }
    painter.line_segment([rect.left_bottom(), rect.right_top()], faint);
    let to_screen = |x: f32, y: f32| {
        pos2(
            rect.left() + x / 255.0 * side,
            rect.bottom() - y / 255.0 * side,
        )
    };
    let from_screen = |p: Pos2| {
        [
            ((p.x - rect.left()) / side * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8,
            ((rect.bottom() - p.y) / side * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8,
        ]
    };
    let id = response.id.with("selected point");
    let mut selected = ui.data(|d| d.get_temp::<usize>(id));
    let near = |points: &[[u8; 2]], at: Pos2| {
        points
            .iter()
            .position(|p| to_screen(f32::from(p[0]), f32::from(p[1])).distance(at) < 9.0)
    };
    let mut changed = false;
    if let Some(at) = response.interact_pointer_pos() {
        if response.double_clicked() {
            if let Some(i) = near(points, at)
                && points.len() > 2
            {
                points.remove(i);
                selected = None;
                changed = true;
            }
        } else if response.drag_started() || response.clicked() {
            selected = near(points, at);
            let [x, _] = from_screen(at);
            if selected.is_none() && !points.iter().any(|p| p[0] == x) && points.len() < 64 {
                let curve = crate::adjustment::sample_curve(points, bezier);
                let y = curve[usize::from(x)].round().clamp(0.0, 255.0) as u8;
                let i = points.partition_point(|p| p[0] < x);
                points.insert(i, [x, y]);
                selected = Some(i);
                changed = true;
            }
        }
        if response.dragged()
            && let Some(i) = selected
            && i < points.len()
        {
            let [mut x, y] = from_screen(at);
            // Keep points in order, one level apart.
            let low = if i > 0 {
                points[i - 1][0].saturating_add(1)
            } else {
                0
            };
            let high = if i + 1 < points.len() {
                points[i + 1][0].saturating_sub(1)
            } else {
                255
            };
            x = x.clamp(low, high.max(low));
            if points[i] != [x, y] {
                points[i] = [x, y];
                changed = true;
            }
        }
    }
    if response.drag_stopped() {
        selected = None;
    }
    ui.data_mut(|d| match selected {
        Some(i) => d.insert_temp(id, i),
        None => d.remove::<usize>(id),
    });
    let curve = crate::adjustment::sample_curve(points, bezier);
    let line: Vec<Pos2> = curve
        .iter()
        .enumerate()
        .map(|(x, y)| to_screen(x as f32, *y))
        .collect();
    painter.add(egui::Shape::line(
        line,
        Stroke::new(2.0_f32, visuals.strong_text_color()),
    ));
    for (i, p) in points.iter().enumerate() {
        let at = to_screen(f32::from(p[0]), f32::from(p[1]));
        if selected == Some(i) {
            painter.circle_filled(at, 5.0, visuals.selection.bg_fill);
        } else {
            painter.circle(
                at,
                4.0,
                Color32::TRANSPARENT,
                Stroke::new(1.5_f32, visuals.strong_text_color()),
            );
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (Dudel, crate::document::LayerId) {
        let mut app = Dudel {
            doc: crate::document::Document::new(4, 4),
            ..Default::default()
        };
        app.create_adjustment_layer(Settings::Invert);
        crate::workflow_tests::finish_jobs(&mut app);
        let id = app.doc.layers[app.doc.active].id;
        (app, id)
    }

    #[test]
    fn new_adjustment_layers_are_named_selected_and_undo_in_one_step() {
        let (mut app, id) = app();
        assert_eq!(app.doc.layers.len(), 2);
        let layer = &app.doc.layers[1];
        assert_eq!((layer.id, layer.name.as_str()), (id, "Invert 1"));
        assert!(layer.adjustment.is_some());
        app.create_adjustment_layer(Settings::Invert);
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(app.doc.layers[2].name, "Invert 2");
        app.undo();
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(app.doc.layers.len(), 2);
    }

    #[test]
    fn settings_edits_apply_live_and_undo_as_one_step() {
        let (mut app, id) = app();
        for levels in [3, 5, 7] {
            app.set_adjustment_settings(id, Settings::Posterize { levels });
        }
        assert_eq!(
            app.doc.layers[1].adjustment.as_ref().unwrap().settings(),
            &Settings::Posterize { levels: 7 }
        );
        app.finish_adjustment_edit();
        app.undo();
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(
            app.doc.layers[1].adjustment.as_ref().unwrap().settings(),
            &Settings::Invert
        );
        // Damaged settings are refused before touching the document.
        app.set_adjustment_settings(
            id,
            Settings::Curves {
                channels: vec![(0, vec![[0, 0]])],
                bezier: false,
            },
        );
        assert_eq!(
            app.doc.layers[1].adjustment.as_ref().unwrap().settings(),
            &Settings::Invert
        );
    }

    #[test]
    fn busy_queue_sends_one_adjustment_edit_per_gesture() {
        let (mut app, id) = app();
        app.render_queue.redo().unwrap();
        assert!(app.render_queue.busy());
        for level in [100, 120] {
            app.set_adjustment_settings(id, Settings::Threshold { level });
        }
        assert!(matches!(
            app.doc.layers[1].adjustment.as_ref().unwrap().settings(),
            Settings::Invert
        ));
        app.finish_adjustment_edit();
        app.flush_render_queue();
        assert_eq!(
            app.doc.layers[1].adjustment.as_ref().unwrap().settings(),
            &Settings::Threshold { level: 120 }
        );
    }

    #[test]
    fn histogram_counts_what_shows_below_the_layer() {
        let mut doc = crate::document::Document::new(4, 4);
        let paint = |layer: &mut crate::document::Layer, rgba: [u8; 4]| {
            let mut writer = layer.pixels.writer();
            for i in 0..16 {
                writer.rgba_mut(i).copy_from_slice(&rgba);
            }
        };
        paint(&mut doc.layers[0], [255, 0, 0, 255]);
        // In a folder: a green layer, the levels layer, then the header.
        let mut green = doc.layers[0].clone();
        green.id = crate::document::LayerId::new();
        green.depth = 1;
        paint(&mut green, [0, 255, 0, 255]);
        let mut target = green.clone();
        target.id = crate::document::LayerId::new();
        let mut header = doc.layers[0].clone();
        header.id = crate::document::LayerId::new();
        header.folder = true;
        doc.layers.extend([green, target, header]);
        let counts = doc.histogram_below(2, 1 << 18, &|| false).unwrap();
        assert_eq!((counts[0][0], counts[1][255], counts[2][0]), (16, 16, 16));
        let counts = doc.histogram_below(1, 4, &|| false).unwrap();
        assert_eq!(counts[0][255], 4, "about four pixels sampled");
        // Nothing below: white paper.
        assert_eq!(doc.histogram_below(0, 16, &|| false).unwrap()[2][255], 16);
    }

    #[test]
    fn hue_ranges_are_named_by_their_hues() {
        use crate::adjustment::{HUE_RANGES, hue_range_slot};
        for (slot, range) in HUE_RANGES.iter().enumerate() {
            assert_eq!(hue_range_slot(*range), slot);
        }
        // A narrower range around orange-red is still reds.
        assert_eq!(hue_range_slot([340.0, 355.0, 10.0, 25.0]), 0);
        assert_eq!(hue_range_slot([200.0, 230.0, 250.0, 270.0]), 4);
    }
}
