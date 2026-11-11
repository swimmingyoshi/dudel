//! The Fill tool's Gradient mode: drag a line, and the active layer (or the
//! selection) fills with a gradient along it, as one undo step. Masks take
//! the gradient's gray values, so a mask can fade artwork out.
use crate::{
    document::{Document, LayerId},
    fill::check_cancel,
    paint_target::Target,
    psd_vector::Gradient,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// How the color changes around the dragged line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    /// Bands across the line, from its start to its end.
    #[default]
    Linear,
    /// Circles around the start; the line is the radius.
    Radial,
    /// A sweep around the start, beginning along the line.
    Angle,
    /// Linear, mirrored on both sides of the start.
    Reflected,
    /// Squares around the start, turned with the line.
    Diamond,
}
impl Style {
    pub const ALL: [Style; 5] = [
        Style::Linear,
        Style::Radial,
        Style::Angle,
        Style::Reflected,
        Style::Diamond,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Style::Linear => "Linear",
            Style::Radial => "Radial",
            Style::Angle => "Angle",
            Style::Reflected => "Reflected",
            Style::Diamond => "Diamond",
        }
    }
}

/// Where the gradient's colors come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Colors {
    /// The main color fading into the secondary color.
    #[default]
    MainToSecondary,
    /// The main color fading out to transparency.
    MainToTransparent,
    /// Stops edited in the tool panel.
    Custom,
}
impl Colors {
    pub const ALL: [Colors; 3] = [
        Colors::MainToSecondary,
        Colors::MainToTransparent,
        Colors::Custom,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Colors::MainToSecondary => "Main to secondary color",
            Colors::MainToTransparent => "Main color to transparent",
            Colors::Custom => "Custom",
        }
    }
}

/// Gradient choices that carry between sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// The Fill tool paints gradients instead of filling areas.
    pub active: bool,
    pub style: Style,
    pub colors: Colors,
    pub reverse: bool,
    /// The stops used with `Colors::Custom`.
    pub custom: Gradient,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            active: false,
            style: Style::Linear,
            colors: Colors::MainToSecondary,
            reverse: false,
            custom: Gradient::between([255, 196, 92], [96, 72, 200], 0.0),
        }
    }
}
impl Options {
    /// The color ramp for these options and the current colors.
    pub fn ramp(&self, main: [u8; 3], secondary: [u8; 3]) -> Gradient {
        match self.colors {
            Colors::MainToSecondary => Gradient::between(main, secondary, 0.0),
            Colors::MainToTransparent => {
                let mut ramp = Gradient::between(main, main, 0.0);
                ramp.set_opacity_stops(&[(0.0, 1.0), (1.0, 0.0)]);
                ramp
            }
            Colors::Custom => self.custom.clone(),
        }
    }
}

/// A gradient to paint: captured when the drag starts, sent as one edit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub target: LayerId,
    #[serde(default, skip_serializing_if = "Target::is_raster")]
    pub paint_target: Target,
    /// The dragged line in canvas pixels.
    pub start: [f64; 2],
    pub end: [f64; 2],
    pub style: Style,
    pub reverse: bool,
    pub ramp: Gradient,
    pub opacity: f32,
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .start
            .iter()
            .chain(&self.end)
            .any(|v| !v.is_finite() || v.abs() > 1.0e9)
        {
            return Err("Invalid gradient line".into());
        }
        if length(self.start, self.end) < 1.0 {
            return Err("Drag a longer line to draw a gradient".into());
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err("Gradient opacity must be between zero and one".into());
        }
        Ok(())
    }
}

fn length(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Steps in the color lookup: finer than 8-bit color can show.
const STEPS: usize = 1024;

/// Where `point` falls on the gradient, 0 at the start and 1 at the end.
pub(crate) fn position(style: Style, start: [f64; 2], end: [f64; 2], point: [f64; 2]) -> f64 {
    let d = [end[0] - start[0], end[1] - start[1]];
    let len = d[0].hypot(d[1]);
    if len <= 0.0 {
        return 0.0;
    }
    let v = [point[0] - start[0], point[1] - start[1]];
    let along = (v[0] * d[0] + v[1] * d[1]) / len;
    let across = (d[0] * v[1] - d[1] * v[0]) / len;
    let t = match style {
        Style::Linear => along / len,
        Style::Reflected => (along / len).abs(),
        Style::Radial => v[0].hypot(v[1]) / len,
        Style::Diamond => along.abs().max(across.abs()) / len,
        Style::Angle => {
            across.atan2(along).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU
        }
    };
    t.clamp(0.0, 1.0)
}

/// The pixels a gradient may change: the selection's bounds, else the canvas.
fn region(doc: &Document) -> [usize; 4] {
    let mut bounds = [0, 0, doc.width, doc.height];
    if let Some(s) = doc.selection {
        bounds = [s.left, s.top, s.right, s.bottom];
    }
    if let Some(s) = doc.selection_mask.as_ref().and_then(|m| m.bounds()) {
        bounds = [
            bounds[0].max(s.left),
            bounds[1].max(s.top),
            bounds[2].min(s.right),
            bounds[3].min(s.bottom),
        ];
    }
    bounds
}

/// Paint the gradient onto the active layer or its mask. Returns whether any
/// pixel changed.
pub(crate) fn apply(
    doc: &mut Document,
    settings: &Settings,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    check_cancel(cancel)?;
    settings.validate()?;
    if doc.layers[doc.active].id != settings.target {
        return Err("Captured gradient target is no longer active".into());
    }
    if let Some(blocker) = settings.paint_target.blocker(doc, doc.active, false) {
        return Err(blocker.into());
    }
    let layer = &doc.layers[doc.active];
    if settings.paint_target.is_raster() && (layer.vector.is_some() || layer.text.is_some()) {
        return Err(
            "Gradients paint on raster layers — add a new layer, or rasterize this one".into(),
        );
    }
    if settings.opacity == 0.0 {
        return Ok(false);
    }
    let [left, top, right, bottom] = region(doc);
    if left >= right || top >= bottom {
        return Ok(false);
    }
    let mut ramp = settings.ramp.clone();
    ramp.set_reverse(false);
    let lookup: Vec<[u8; 4]> = (0..STEPS)
        .map(|i| ramp.sample(i as f64 / (STEPS - 1) as f64))
        .collect();
    let step = |x: usize, y: usize| {
        let mut t = position(
            settings.style,
            settings.start,
            settings.end,
            [x as f64 + 0.5, y as f64 + 0.5],
        );
        if settings.reverse {
            t = 1.0 - t;
        }
        (t * (STEPS - 1) as f64).round() as usize
    };
    if let Target::Mask(id) = settings.paint_target {
        let grays: Vec<u8> = lookup
            .iter()
            .map(|c| Target::Mask(id).params([c[0], c[1], c[2]], false, false).0[0])
            .collect();
        let gray = |x: usize, y: usize| {
            let index = step(x, y);
            (
                grays[index],
                f32::from(lookup[index][3]) / 255.0 * settings.opacity,
            )
        };
        return Ok(
            match crate::mask_fill::paint_each(
                doc,
                doc.active,
                id,
                [left, top, right, bottom],
                cancel,
                gray,
            )? {
                Some(coverage) => {
                    doc.layers[doc.active].mask.as_mut().unwrap().coverage = coverage;
                    true
                }
                None => false,
            },
        );
    }
    let selection = doc.selection;
    let selected = doc.selection_mask.clone();
    let alpha_locked = doc.layers[doc.active].alpha_locked;
    let original = doc.layers[doc.active].pixels.clone();
    let width = doc.width;
    let mut writer = doc.layers[doc.active].pixels.writer();
    let mut changed = false;
    for y in top..bottom {
        check_cancel(cancel)?;
        for x in left..right {
            if selection.is_some_and(|s| !s.contains(x, y)) {
                continue;
            }
            let coverage = selected
                .as_ref()
                .map_or(1.0, |m| f32::from(m.at(x, y)) / 255.0);
            if coverage == 0.0 {
                continue;
            }
            let color = lookup[step(x, y)];
            let alpha = f32::from(color[3]) / 255.0 * settings.opacity * coverage;
            let index = y * width + x;
            let before = original.rgba(index);
            let mut after = before;
            crate::document::paint_pixel(
                &mut after,
                alpha,
                [color[0], color[1], color[2]],
                false,
                alpha_locked,
            );
            if before != after {
                writer.rgba_mut(index).copy_from_slice(&after);
                changed = true;
            }
        }
    }
    writer.reclaim_region(width, [left, top, right, bottom]);
    check_cancel(cancel)?;
    Ok(changed)
}

impl crate::Dudel {
    /// The secondary color, or the secondary mask coverage while a mask is
    /// the painting target.
    pub(crate) fn secondary_paint_color(&self) -> [u8; 3] {
        if self.input_stroke_target().is_raster() {
            self.secondary
        } else {
            [self.mask_colors[1]; 3]
        }
    }

    /// Pointer input while the Fill tool is in Gradient mode: press to start
    /// the line, drag to aim it (Shift snaps to 15°), release to paint.
    pub(crate) fn gradient_packet(
        &mut self,
        point: [f64; 2],
        start: bool,
        end: bool,
        can_start: bool,
        shift: bool,
    ) {
        use crate::selection_ui::Drag;
        if start && can_start {
            if self.canvas_edit_ticket.is_some() {
                self.status = "Wait for the canvas bounds change to finish".into();
                return;
            }
            let paint_target = self.input_stroke_target();
            if let Some(blocker) = self.stroke_target_blocker(paint_target, false) {
                self.status = blocker.into();
                return;
            }
            let Some(target) = self.input_target() else {
                return;
            };
            let layer = self.input_layer().and_then(|i| self.doc.layers.get(i));
            if paint_target.is_raster()
                && layer.is_some_and(|l| l.vector.is_some() || l.text.is_some())
            {
                self.status =
                    "Gradients paint on raster layers — add a new layer, or rasterize this one"
                        .into();
                return;
            }
            self.selection_drag = Some(Drag::Gradient(Box::new(Settings {
                target,
                paint_target,
                start: point,
                end: point,
                style: self.gradient.style,
                reverse: self.gradient.reverse,
                ramp: self
                    .gradient
                    .ramp(self.paint_color(), self.secondary_paint_color()),
                opacity: self.brush.opacity,
            })));
        }
        let Some(Drag::Gradient(settings)) = &mut self.selection_drag else {
            return;
        };
        let mut point = point;
        if shift {
            let (dx, dy) = (point[0] - settings.start[0], point[1] - settings.start[1]);
            let step = 15f64.to_radians();
            let angle = (dy.atan2(dx) / step).round() * step;
            let length = dx.hypot(dy);
            point = [
                settings.start[0] + angle.cos() * length,
                settings.start[1] + angle.sin() * length,
            ];
        }
        settings.end = point;
        if !end {
            return;
        }
        let Some(Drag::Gradient(settings)) = self.selection_drag.take() else {
            return;
        };
        if length(settings.start, settings.end) < 1.0 {
            self.status = "Drag a line to draw a gradient".into();
            return;
        }
        let colored = settings.paint_target.is_raster() && self.gradient.colors != Colors::Custom;
        if self.dispatch_document_edit(
            crate::document_edits::Edit::Gradient(settings),
            "Gradient applied",
            "Gradient made no changes",
        ) && colored
        {
            self.note_recent_color();
        }
    }

    /// The line being dragged, with its ends marked.
    pub(crate) fn paint_gradient_line(
        &self,
        painter: &crate::egui::Painter,
        view: crate::view::View,
    ) {
        use crate::egui::{Color32, Stroke, pos2};
        let Some(crate::selection_ui::Drag::Gradient(settings)) = &self.selection_drag else {
            return;
        };
        let at = |p: [f64; 2]| view.to_screen(pos2(p[0] as f32, p[1] as f32));
        let (a, b) = (at(settings.start), at(settings.end));
        painter.line_segment([a, b], Stroke::new(3.0_f32, Color32::BLACK));
        painter.line_segment([a, b], Stroke::new(1.0_f32, Color32::WHITE));
        for (point, filled) in [(a, true), (b, false)] {
            painter.circle_filled(point, 5.0, Color32::BLACK);
            painter.circle(
                point,
                3.5,
                if filled {
                    Color32::WHITE
                } else {
                    Color32::BLACK
                },
                Stroke::new(1.5_f32, Color32::WHITE),
            );
        }
    }

    /// Bucket or Gradient, at the top of the Fill tool's options.
    pub(crate) fn fill_mode_ui(&mut self, ui: &mut crate::egui::Ui) {
        let key = self
            .shortcuts
            .label(crate::commands::Action::Tool(crate::tools::Tool::Fill));
        ui.horizontal(|ui| {
            if ui
                .selectable_label(!self.gradient.active, "Bucket")
                .on_hover_text(format!(
                    "Fill an area of similar color · tap {key} to switch"
                ))
                .clicked()
            {
                self.gradient.active = false;
            }
            if ui
                .selectable_label(self.gradient.active, "Gradient")
                .on_hover_text(format!("Drag to paint a gradient · tap {key} to switch"))
                .clicked()
            {
                self.gradient.active = true;
            }
        });
    }

    /// The Gradient mode's options, in the tool panel or the options bar's
    /// menu (which has its own opacity slider beside it).
    pub(crate) fn gradient_options_ui(&mut self, ui: &mut crate::egui::Ui, opacity: bool) {
        use crate::egui;
        let ramp = self
            .gradient
            .ramp(self.paint_color(), self.secondary_paint_color());
        let reverse = self.gradient.reverse;
        crate::effects_panel::gradient_bar(ui, |t| ramp.sample(if reverse { 1.0 - t } else { t }));
        ui.label("Shape");
        ui.horizontal_wrapped(|ui| {
            for style in Style::ALL {
                ui.selectable_value(&mut self.gradient.style, style, style.name());
            }
        });
        ui.label("Colors");
        egui::ComboBox::from_id_salt("gradient colors")
            .selected_text(self.gradient.colors.name())
            .width(ui.available_width().min(240.0))
            .show_ui(ui, |ui| {
                for colors in Colors::ALL {
                    ui.selectable_value(&mut self.gradient.colors, colors, colors.name());
                }
            });
        if self.gradient.colors == Colors::Custom {
            crate::effects_panel::gradient_stops_editor(ui, &mut self.gradient.custom);
        }
        ui.checkbox(&mut self.gradient.reverse, "Reverse");
        if opacity {
            ui.label("Opacity");
            let mut percent = (self.brush.opacity * 100.0).round();
            if ui
                .add(
                    egui::Slider::new(&mut percent, 0.0..=100.0)
                        .suffix("%")
                        .integer(),
                )
                .changed()
            {
                self.brush.opacity = percent / 100.0;
            }
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(if self.input_stroke_target().is_raster() {
                "Drag across the canvas: the gradient starts where you press and ends where \
                 you release. Shift snaps the angle. A selection limits it; Lock \
                 transparency recolors only existing paint."
            } else {
                "Drag across the canvas to fade the mask: the colors' brightness becomes \
                 mask coverage, so black conceals and white reveals. Shift snaps the angle."
            })
            .small()
            .weak(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(doc: &Document, style: Style) -> Settings {
        Settings {
            target: doc.layers[doc.active].id,
            paint_target: Target::Raster,
            start: [0.0, 0.0],
            end: [10.0, 0.0],
            style,
            reverse: false,
            ramp: Gradient::between([0, 0, 0], [255, 255, 255], 0.0),
            opacity: 1.0,
        }
    }

    #[test]
    fn positions_follow_each_style() {
        let (a, b) = ([0.0, 0.0], [10.0, 0.0]);
        assert_eq!(position(Style::Linear, a, b, [5.0, 3.0]), 0.5);
        assert_eq!(position(Style::Linear, a, b, [-5.0, 0.0]), 0.0);
        assert_eq!(position(Style::Linear, a, b, [20.0, 0.0]), 1.0);
        assert_eq!(position(Style::Reflected, a, b, [-5.0, 0.0]), 0.5);
        assert_eq!(position(Style::Radial, a, b, [0.0, 5.0]), 0.5);
        assert_eq!(position(Style::Diamond, a, b, [3.0, -5.0]), 0.5);
        assert_eq!(position(Style::Angle, a, b, [5.0, 0.0]), 0.0);
        assert!((position(Style::Angle, a, b, [-5.0, 0.0]) - 0.5).abs() < 1e-9);
        // A turned line turns the bands with it.
        assert!((position(Style::Linear, a, [0.0, 10.0], [7.0, 5.0]) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn linear_gradient_runs_from_start_to_end_across_the_layer() {
        let mut doc = Document::new(10, 2);
        let settings = settings(&doc, Style::Linear);
        assert!(apply(&mut doc, &settings, None).unwrap());
        let px = |x: usize| doc.layers[0].pixels.rgba(x);
        assert!(px(0)[0] < 20 && px(9)[0] > 235, "{:?} {:?}", px(0), px(9));
        assert!(px(0)[0] < px(5)[0] && px(5)[0] < px(9)[0]);
        assert!((0..10).all(|x| px(x)[3] == 255));
        // Reversed, the ends swap.
        let mut reversed = Document::new(10, 2);
        let mut settings = settings.clone();
        settings.target = reversed.layers[0].id;
        settings.reverse = true;
        apply(&mut reversed, &settings, None).unwrap();
        assert!(reversed.layers[0].pixels.rgba(0)[0] > 235);
    }

    #[test]
    fn selections_alpha_lock_and_transparent_ends_are_respected() {
        let mut doc = Document::new(10, 2);
        doc.selection = Some(crate::selection::Selection {
            left: 2,
            top: 0,
            right: 4,
            bottom: 2,
        });
        let linear = settings(&doc, Style::Linear);
        apply(&mut doc, &linear, None).unwrap();
        for x in 0..10 {
            let painted = doc.layers[0].pixels.rgba(x)[3] != 0;
            assert_eq!(painted, (2..4).contains(&x), "pixel {x}");
        }
        // Alpha lock recolors only what is already there.
        let mut locked = Document::new(10, 1);
        locked.layers[0].alpha_locked = true;
        locked.layers[0]
            .pixels
            .writer()
            .rgba_mut(3)
            .copy_from_slice(&[9, 9, 9, 255]);
        let linear = settings(&locked, Style::Linear);
        apply(&mut locked, &linear, None).unwrap();
        assert_eq!(locked.layers[0].pixels.rgba(0)[3], 0);
        assert_ne!(locked.layers[0].pixels.rgba(3), [9, 9, 9, 255]);
        // Main color to transparent leaves the far end clear.
        let mut fade = Document::new(10, 1);
        let mut settings = settings(&fade, Style::Linear);
        settings.ramp = Options {
            colors: Colors::MainToTransparent,
            ..Default::default()
        }
        .ramp([200, 0, 0], [0, 0, 0]);
        apply(&mut fade, &settings, None).unwrap();
        assert!(fade.layers[0].pixels.rgba(0)[3] > 230);
        assert!(fade.layers[0].pixels.rgba(9)[3] < 25);
    }

    #[test]
    fn short_lines_bad_values_and_stale_targets_are_refused() {
        let mut doc = Document::new(8, 8);
        let mut short = settings(&doc, Style::Linear);
        short.end = [0.5, 0.0];
        assert!(apply(&mut doc, &short, None).is_err());
        let mut bad = settings(&doc, Style::Linear);
        bad.opacity = f32::NAN;
        assert!(bad.validate().is_err());
        let mut stale = settings(&doc, Style::Linear);
        stale.target = LayerId::new();
        assert!(apply(&mut doc, &stale, None).is_err());
        assert!((0..64).all(|i| doc.layers[0].pixels.rgba(i)[3] == 0));
        let mut locked = Document::new(8, 8);
        locked.layers[0].locked = true;
        let linear = settings(&locked, Style::Linear);
        assert!(apply(&mut locked, &linear, None).is_err());
    }

    #[test]
    fn masks_take_the_gradient_as_gray_coverage() {
        let mut doc = Document::new(10, 1);
        let id = LayerId::new();
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id,
            enabled: true,
            linked: true,
            coverage: crate::layer_mask::Mask::new(10, 1, 255).unwrap(),
        });
        let mut settings = settings(&doc, Style::Linear);
        settings.paint_target = Target::Mask(id);
        assert!(apply(&mut doc, &settings, None).unwrap());
        let mask = &doc.layers[0].mask.as_ref().unwrap().coverage;
        assert!(mask.at(0, 0) < 20 && mask.at(9, 0) > 235);
    }
}
