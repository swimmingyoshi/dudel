use crate::{
    brush::Brush,
    document::Document,
    stroke::{Sample, Stroke},
};
use eframe::egui;

/// What the test pad's empty paper looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paper {
    #[default]
    White,
    Gray,
    Dark,
}
impl Paper {
    const ALL: [Paper; 3] = [Paper::White, Paper::Gray, Paper::Dark];
    fn name(self) -> &'static str {
        match self {
            Paper::White => "White",
            Paper::Gray => "Gray",
            Paper::Dark => "Dark",
        }
    }
    fn color(self) -> egui::Color32 {
        match self {
            Paper::White => egui::Color32::from_gray(250),
            Paper::Gray => egui::Color32::from_gray(128),
            Paper::Dark => egui::Color32::from_gray(28),
        }
    }
}

/// Test strokes kept for Undo on the pad.
const UNDO_STEPS: usize = 12;
/// Largest pad, in pixels.
const MAX_SIDE: usize = 4096;

/// A drawing surface for trying the current brush, separate from the
/// project. It is drawn at true scale: one pad pixel per screen point.
pub struct Scratchpad {
    document: Document,
    stroke: Stroke,
    last: Option<Sample>,
    texture: Option<egui::TextureHandle>,
    dirty: bool,
    buffer: crate::stroke_buffer::StrokeBuffer,
    settings: Option<crate::stroke_buffer::Settings>,
    undo: Vec<Document>,
    paper: Paper,
    /// The egui pass whose pen input was already handled, so a pad shown in
    /// two places never paints a packet twice.
    handled_pass: Option<u64>,
}

impl Default for Scratchpad {
    fn default() -> Self {
        Self {
            document: Document::new(512, 160),
            stroke: Stroke::default(),
            last: None,
            texture: None,
            dirty: true,
            buffer: Default::default(),
            settings: None,
            undo: Vec::new(),
            paper: Paper::White,
            handled_pass: None,
        }
    }
}
impl Scratchpad {
    pub fn reset_stroke(&mut self) {
        if let Some(settings) = &self.settings {
            self.dirty |= !self
                .buffer
                .finish_tail(&mut self.document, settings)
                .region
                .is_empty();
        }
        self.stroke.reset();
        self.last = None;
        self.buffer = Default::default();
        self.settings = None;
    }
    fn paint(&mut self, dabs: Vec<Sample>, settings: &crate::stroke_buffer::Settings) {
        if !dabs.is_empty() {
            for dab in dabs {
                self.buffer.paint(&mut self.document, dab, settings);
            }
            self.dirty = true;
        }
    }
    /// Whether anything is drawn on the pad.
    #[cfg(test)]
    pub fn marked(&self) -> bool {
        let [w, h] = [self.document.width, self.document.height];
        self.document.layers[0]
            .pixels
            .region(w, [0, 0, w, h])
            .chunks_exact(4)
            .any(|p| p[3] > 0)
    }
    fn remember(&mut self) {
        self.undo.push(self.document.clone());
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
    }
    /// Match the pad to `size`, keeping the marks that still fit.
    fn fit(&mut self, [width, height]: [usize; 2]) {
        let (width, height) = (width.clamp(64, MAX_SIDE), height.clamp(32, MAX_SIDE));
        if self.last.is_some() || [width, height] == [self.document.width, self.document.height] {
            return;
        }
        let mut document = Document::new(width, height);
        {
            let old = &self.document;
            let mut writer = document.layers[0].pixels.writer();
            for y in 0..height.min(old.height) {
                for x in 0..width.min(old.width) {
                    let p = old.layers[0].pixels.rgba(y * old.width + x);
                    if p[3] > 0 {
                        writer.rgba_mut(y * width + x).copy_from_slice(&p);
                    }
                }
            }
        }
        self.document = document;
        // Undo steps were for the old size.
        self.undo.clear();
        self.dirty = true;
    }
    /// Bands of the main and secondary colors, for smudge and blur brushes.
    fn stripes(&mut self, main: [u8; 3], secondary: [u8; 3]) {
        self.remember();
        let (width, height) = (self.document.width, self.document.height);
        let mut writer = self.document.layers[0].pixels.writer();
        for y in 0..height {
            for x in 0..width {
                let c = if x / 24 % 2 == 0 { main } else { secondary };
                writer
                    .rgba_mut(y * width + x)
                    .copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        self.dirty = true;
    }
    /// Paint a few sample strokes, rising and falling in pressure, for
    /// screenshots.
    pub fn demo(&mut self, brush: &Brush, color: [u8; 3], secondary: [u8; 3], size: [usize; 2]) {
        self.fit(size);
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color,
            secondary,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let (w, h) = (self.document.width as f32, self.document.height as f32);
        for row in 0..3 {
            let mut stroke = Stroke::default();
            let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
            let steps = 90;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let sample = Sample {
                    x: w * (0.05 + 0.9 * t),
                    y: h * (0.22 + 0.28 * row as f32)
                        + (t * std::f32::consts::TAU * 1.5).sin() * h * 0.08,
                    pressure: (t * std::f32::consts::PI).sin().max(0.05),
                    time: Some(f64::from(i) * 0.01),
                    pressure_known: true,
                    ..Default::default()
                };
                let dabs = if i == steps {
                    stroke.finish(sample, brush)
                } else {
                    stroke.sample(sample, brush)
                };
                for dab in dabs {
                    buffer.paint(&mut self.document, dab, &settings);
                }
            }
            buffer.finish_tail(&mut self.document, &settings);
        }
        self.dirty = true;
    }
    /// The test pad with its toolbar. `height` is the pad's height in points;
    /// without one it keeps a 16 : 5 shape. With `hide`, the toolbar offers
    /// to hide the pad; returns whether that was clicked.
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        brush: &Brush,
        color: [u8; 3],
        secondary: [u8; 3],
        packets: &[crate::input::Packet],
        height: Option<f32>,
        hide: bool,
    ) -> bool {
        let mut hidden = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong("Test pad");
            if ui
                .button("Clear")
                .on_hover_text("Clear the test pad")
                .clicked()
            {
                self.reset_stroke();
                self.remember();
                self.document = Document::new(self.document.width, self.document.height);
                self.dirty = true;
            }
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                .on_hover_text("Take back the last test stroke")
                .clicked()
            {
                self.reset_stroke();
                if let Some(document) = self.undo.pop() {
                    self.document = document;
                    self.dirty = true;
                }
            }
            if ui
                .button("Stripes")
                .on_hover_text("Fill with bands of your two colors, to try smudge and blur")
                .clicked()
            {
                self.reset_stroke();
                self.stripes(color, secondary);
            }
            ui.separator();
            for paper in Paper::ALL {
                ui.selectable_value(&mut self.paper, paper, paper.name())
                    .on_hover_text("Paper under the test strokes");
            }
            if hide {
                ui.separator();
                hidden = ui
                    .button("Hide")
                    .on_hover_text("Give the settings the room; Show test pad brings it back")
                    .clicked();
            }
        });
        let width = ui.available_width().max(64.0);
        let height = height
            .map_or(width * 160.0 / 512.0, |h| {
                // The toolbar above and the hint below share the height.
                h - ui.spacing().interact_size.y
                    - ui.text_style_height(&egui::TextStyle::Small)
                    - 3.0 * ui.spacing().item_spacing.y
                    - 2.0
            })
            .max(48.0);
        let (area, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::drag());
        self.fit([
            area.width().round() as usize,
            area.height().round() as usize,
        ]);
        // Pad pixels per screen point; 1 unless the pad hit its size limit.
        let scale = self.document.width as f32 / area.width();
        let pass = ui.ctx().cumulative_pass_nr();
        let packets = if self.handled_pass == Some(pass) {
            &[][..]
        } else {
            self.handled_pass = Some(pass);
            packets
        };
        for packet in packets {
            let (sample, start, end) = match packet {
                crate::input::Packet::Start(s) => (*s, true, false),
                crate::input::Packet::Move(s) => (*s, false, false),
                crate::input::Packet::End(s) => (*s, false, true),
                crate::input::Packet::Cancel => {
                    if self.last.is_some() {
                        std::mem::take(&mut self.buffer).restore(&mut self.document);
                        self.undo.pop();
                        self.dirty = true;
                    }
                    self.reset_stroke();
                    continue;
                }
            };
            let pos = egui::pos2(sample.x, sample.y);
            if start {
                self.reset_stroke();
                if !area.contains(pos) || ui.ctx().layer_id_at(pos) != Some(response.layer_id) {
                    continue;
                }
                self.remember();
                self.last = Some(sample);
            }
            if self.last.is_none() {
                continue;
            }
            let settings = self
                .settings
                .take()
                .unwrap_or_else(|| crate::stroke_buffer::Settings {
                    target: crate::paint_target::Target::Raster,
                    brush: brush.clone(),
                    color,
                    secondary,
                    erase: false,
                    layer: 0,
                    symmetry: Default::default(),
                });
            let local = Sample {
                x: (pos.x - area.min.x) * scale,
                y: (pos.y - area.min.y) * scale,
                ..sample
            };
            let dabs = if end {
                self.stroke
                    .finish_at_zoom(local, &settings.brush, 1.0 / scale)
            } else {
                self.stroke
                    .sample_at_zoom(local, &settings.brush, 1.0 / scale)
            };
            self.paint(dabs, &settings);
            self.last = Some(local);
            if end {
                let finished = self.buffer.finish_tail(&mut self.document, &settings);
                self.dirty |= !finished.region.is_empty();
                self.reset_stroke();
            } else {
                self.settings = Some(settings);
            }
        }
        if self.dirty || self.texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.document.width, self.document.height],
                &self.document.composite_transparent(),
            );
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "brush scratchpad",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            self.dirty = false;
        }
        ui.painter().rect_filled(area, 2.0, self.paper.color());
        if let Some(texture) = &self.texture {
            ui.painter().image(
                texture.id(),
                area,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        ui.label(
            egui::RichText::new(
                "Draw here to try the brush as you change it. Test strokes never touch your project.",
            )
            .small()
            .weak(),
        );
        hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratchpad_finalizes_end_taper_and_cancellation_preserves_previous_marks() {
        let ctx = egui::Context::default();
        let mut pad = Scratchpad::default();
        let brush = Brush {
            size: 40.0,
            hardness: 1.0,
            opacity: 0.5,
            pressure_size: false,
            taper_out: 20.0,
            ..Default::default()
        };
        let draw = |pad: &mut Scratchpad, packets: &[crate::input::Packet]| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        pad.ui(ui, &brush, [10, 20, 30], [10, 20, 30], packets, None, false)
                    });
                },
            );
        };
        draw(&mut pad, &[]);
        let sample = Sample {
            x: 80.0,
            y: 80.0,
            pressure: 1.0,
            ..Default::default()
        };
        draw(&mut pad, &[crate::input::Packet::Start(sample)]);
        let preview = pad.document.layers[0].pixels.clone();
        // Brush changes/UI teardown commit using the captured stroke settings.
        pad.reset_stroke();
        let final_pixels = pad.document.layers[0].pixels.clone();
        assert_ne!(preview, final_pixels);
        assert_eq!(pad.buffer.payload_capacity_bytes(), 0);
        draw(
            &mut pad,
            &[crate::input::Packet::Start(Sample { x: 150.0, ..sample })],
        );
        assert_ne!(pad.document.layers[0].pixels, final_pixels);
        draw(&mut pad, &[crate::input::Packet::Cancel]);
        assert_eq!(pad.document.layers[0].pixels, final_pixels);
        draw(
            &mut pad,
            &[
                crate::input::Packet::Start(sample),
                crate::input::Packet::End(sample),
            ],
        );
        assert_ne!(pad.document.layers[0].pixels, final_pixels);
        assert!(pad.settings.is_none());
        assert!(pad.last.is_none());
        assert_eq!(pad.buffer.payload_capacity_bytes(), 0);
    }
    #[test]
    fn scratchpad_handles_a_complete_tap_in_one_update_and_new_stroke_coverage() {
        let ctx = egui::Context::default();
        let mut pad = Scratchpad::default();
        let brush = Brush {
            opacity: 0.5,
            pressure_size: false,
            ..Default::default()
        };
        let mut draw = |packets: &[crate::input::Packet]| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        pad.ui(ui, &brush, [0, 0, 0], [0, 0, 0], packets, None, false)
                    });
                },
            );
            let [w, h] = [pad.document.width, pad.document.height];
            pad.document.layers[0]
                .pixels
                .region(w, [0, 0, w, h])
                .chunks_exact(4)
                .map(|p| p[3])
                .max()
                .unwrap()
        };
        draw(&[]);
        let sample = Sample {
            x: 80.0,
            y: 80.0,
            pressure: 1.0,
            time: Some(1.0),
            ..Default::default()
        };
        let tap = [
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ];
        let first = draw(&tap);
        assert_eq!(first, 128);
        assert_eq!(draw(&tap), 192);
        assert!(pad.last.is_none());
        assert_eq!(pad.buffer.payload_capacity_bytes(), 0);
    }
}
