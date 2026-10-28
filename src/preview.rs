use crate::{
    brush::Brush,
    document::Document,
    stroke::{Sample, Stroke},
};
use eframe::egui;

pub struct Scratchpad {
    document: Document,
    stroke: Stroke,
    last: Option<Sample>,
    texture: Option<egui::TextureHandle>,
    dirty: bool,
    buffer: crate::stroke_buffer::StrokeBuffer,
    settings: Option<crate::stroke_buffer::Settings>,
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
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        brush: &Brush,
        color: [u8; 3],
        packets: &[crate::input::Packet],
    ) {
        ui.horizontal(|ui| {
            ui.label("Scratchpad");
            if ui.button("Clear preview").clicked() {
                self.reset_stroke();
                self.document = Document::new(512, 160);
                self.dirty = true;
            }
        });
        let width = ui.available_width();
        let (area, response) = ui.allocate_exact_size(
            egui::vec2(width, width * 160.0 / 512.0),
            egui::Sense::drag(),
        );
        for packet in packets {
            let (sample, start, end) = match packet {
                crate::input::Packet::Start(s) => (*s, true, false),
                crate::input::Packet::Move(s) => (*s, false, false),
                crate::input::Packet::End(s) => (*s, false, true),
                crate::input::Packet::Cancel => {
                    if self.last.is_some() {
                        std::mem::take(&mut self.buffer).restore(&mut self.document);
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
                    erase: false,
                    layer: 0,
                    symmetry: Default::default(),
                });
            let local = Sample {
                x: (pos.x - area.min.x) * 512.0 / area.width(),
                y: (pos.y - area.min.y) * 160.0 / area.height(),
                ..sample
            };
            let dabs = if end {
                self.stroke
                    .finish_at_zoom(local, &settings.brush, area.width() / 512.0)
            } else {
                self.stroke
                    .sample_at_zoom(local, &settings.brush, area.width() / 512.0)
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
            let image =
                egui::ColorImage::from_rgba_unmultiplied([512, 160], &self.document.composite());
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
        if let Some(texture) = &self.texture {
            ui.painter().image(
                texture.id(),
                area,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        ui.label(egui::RichText::new("Draw here to test the current brush. Preview marks are separate from your project.").small().weak());
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
                    egui::CentralPanel::default()
                        .show(ctx, |ui| pad.ui(ui, &brush, [10, 20, 30], packets));
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
                    egui::CentralPanel::default()
                        .show(ctx, |ui| pad.ui(ui, &brush, [0, 0, 0], packets));
                },
            );
            pad.document.layers[0]
                .pixels
                .region(512, [0, 0, 512, 160])
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
