//! Brush creation: a live preview that redraws whenever a setting changes,
//! the settings split into sections shared by the side panel and the Brush
//! Studio window, and the window itself.
use crate::{Dudel, brush::Brush, document::Document, egui, stroke};

/// Ink the previews draw with, on white paper, so opacity and flow read as
/// they would on a canvas.
const INK: [u8; 3] = [30, 34, 44];

/// A render in flight.
struct RenderJob {
    key: u64,
    /// The size the preview asked for, and the size actually rendered
    /// (half of it after a slow render).
    requested: [usize; 2],
    rendered: [usize; 2],
    /// Returns straight RGBA pixels and the milliseconds they took.
    worker: std::thread::JoinHandle<(Vec<u8>, f64)>,
}

/// A preview image rendered on a worker from the settings it was asked for.
/// Only one render runs at a time; when settings change mid-render the
/// newest settings are rendered next, so dragging a slider never stalls.
#[derive(Default)]
pub struct LivePreview {
    shown: Option<(u64, [usize; 2])>,
    texture: Option<egui::TextureHandle>,
    job: Option<RenderJob>,
    /// Milliseconds the last render took, for the performance gate.
    pub last_ms: f64,
}
/// Renders slower than this switch the preview to half resolution, so heavy
/// brushes (large tips with tight spacing) still update smoothly.
const SLOW_MS: f64 = 30.0;

impl LivePreview {
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }
    /// Draw the preview filling the available width at `height` points.
    pub fn show(&mut self, ui: &mut egui::Ui, brush: &Brush, height: f32, defer: bool) {
        let width = ui.available_width().max(80.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let scale = ui.ctx().pixels_per_point();
        let size = [
            ((width * scale).round() as usize).clamp(64, 2048),
            ((height * scale).round() as usize).clamp(32, 1024),
        ];
        self.update_when_idle(ui.ctx(), brush, size, defer);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, egui::Color32::WHITE);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        let tip = height.min(width * 0.3);
        painter.line_segment(
            [
                egui::pos2(rect.left() + tip, rect.top() + 6.0),
                egui::pos2(rect.left() + tip, rect.bottom() - 6.0),
            ],
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(215)),
        );
    }

    /// Start, collect or skip renders so the shown image follows `brush`.
    #[cfg(test)]
    pub fn update(&mut self, ctx: &egui::Context, brush: &Brush, size: [usize; 2]) {
        self.update_when_idle(ctx, brush, size, false);
    }
    fn update_when_idle(
        &mut self,
        ctx: &egui::Context,
        brush: &Brush,
        size: [usize; 2],
        defer: bool,
    ) {
        let key = crate::brush_previews::fingerprint(brush);
        if self
            .job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let RenderJob {
                key: job_key,
                requested,
                rendered,
                worker,
            } = self.job.take().unwrap();
            if let Ok((pixels, ms)) = worker.join()
                && job_key == key
                && requested == size
            {
                self.last_ms = ms;
                let image = egui::ColorImage::from_rgba_unmultiplied(rendered, &pixels);
                match &mut self.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture = Some(ctx.load_texture(
                            "brush live preview",
                            image,
                            egui::TextureOptions::LINEAR,
                        ))
                    }
                }
                self.shown = Some((job_key, requested));
            }
        }
        if self.shown == Some((key, size)) {
            return;
        }
        if self.job.is_none() && !defer {
            let brush = brush.clone();
            let render_size = if self.last_ms > SLOW_MS {
                [(size[0] / 2).max(32), (size[1] / 2).max(16)]
            } else {
                size
            };
            let handle = std::thread::Builder::new()
                .name("brush-preview".into())
                .spawn(move || {
                    let started = std::time::Instant::now();
                    let pixels = render(&brush, render_size);
                    (pixels, started.elapsed().as_secs_f64() * 1000.0)
                })
                .ok();
            if let Some(worker) = handle {
                self.job = Some(RenderJob {
                    key,
                    requested: size,
                    rendered: render_size,
                    worker,
                });
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// A tip close-up on the left and a pressure-ramped S-curve on the right,
/// over white paper. Straight RGBA.
pub fn render(brush: &Brush, [width, height]: [usize; 2]) -> Vec<u8> {
    let mut doc = Document::new(width, height);
    let settings = |brush: Brush| crate::stroke_buffer::Settings {
        target: crate::paint_target::Target::Raster,
        brush,
        color: INK,
        erase: false,
        layer: 0,
        symmetry: Default::default(),
    };
    let tip_side = height.min(width * 3 / 10) as f32;
    // Tip: one full-pressure dab scaled to fill its box, shape over size.
    let mut tip = brush.clone();
    tip.size = tip_side * 0.72;
    tip.scatter = 0.0;
    tip.stabilization = 0.0;
    tip.stabilizer_mode = crate::brush::StabilizerMode::Smooth;
    let tip_settings = settings(tip.clone());
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    let dab = stroke::Sample {
        x: tip_side / 2.0,
        y: height as f32 / 2.0,
        pressure: 1.0,
        random: Some(0.5),
        ..Default::default()
    };
    buffer.paint(&mut doc, dab, &tip_settings);
    buffer.finish_tail(&mut doc, &tip_settings);
    // Stroke: real size up to most of the height, pressure rising then falling.
    let mut sample_brush = brush.clone();
    sample_brush.size = brush.size.min(height as f32 * 0.55);
    sample_brush.stabilization = 0.0;
    sample_brush.stabilizer_mode = crate::brush::StabilizerMode::Smooth;
    let stroke_settings = settings(sample_brush.clone());
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    let mut path = stroke::Stroke::default();
    let left = tip_side + 12.0 + sample_brush.size / 2.0;
    let right = width as f32 - 10.0 - sample_brush.size / 2.0;
    let steps = 120;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let sample = stroke::Sample {
            x: left + t * (right - left).max(1.0),
            y: height as f32 / 2.0 - (t * std::f32::consts::TAU).sin() * height as f32 * 0.22,
            pressure: (t * std::f32::consts::PI).sin().max(0.05),
            time: Some(i as f64 * 0.008),
            ..Default::default()
        };
        let dabs = if i == steps {
            path.finish(sample, &sample_brush)
        } else {
            path.sample(sample, &sample_brush)
        };
        for dab in dabs {
            buffer.paint(&mut doc, dab, &stroke_settings);
        }
    }
    buffer.finish_tail(&mut doc, &stroke_settings);
    // Over white paper, opaque, so the texture shows exactly what paints.
    doc.composite()
}

/// Brush settings grouped for the side panel and the Brush Studio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    SizeOpacity,
    Pressure,
    Stabilizer,
    Dynamics,
    Tip,
    Scatter,
    Grain,
    Tags,
    TestPad,
}

impl Section {
    pub(crate) const STUDIO: [Section; 9] = [
        Section::SizeOpacity,
        Section::Pressure,
        Section::Dynamics,
        Section::Tip,
        Section::Stabilizer,
        Section::Scatter,
        Section::Grain,
        Section::Tags,
        Section::TestPad,
    ];
    pub(crate) fn title(self) -> &'static str {
        match self {
            Section::SizeOpacity => "Size and opacity",
            Section::Pressure => "Pen pressure",
            Section::Stabilizer => "Stabilizer",
            Section::Dynamics => "Dynamics",
            Section::Tip => "Tip shape",
            Section::Scatter => "Scatter",
            Section::Grain => "Paper grain",
            Section::Tags => "Name and tags",
            Section::TestPad => "Test pad",
        }
    }
}

impl Dudel {
    /// The Brush Studio: sections on the left, a large live preview and the
    /// chosen section's settings on the right.
    pub(crate) fn brush_studio_ui(&mut self, ctx: &egui::Context) {
        if !self.brush_studio {
            return;
        }
        let mut open = true;
        let bounds = ctx.content_rect().shrink(8.0);
        let maximum = (bounds.size() - egui::vec2(16.0, 40.0)).max(egui::vec2(200.0, 140.0));
        egui::Window::new("Brush Studio")
            .id(egui::Id::new("brush studio"))
            .open(&mut open)
            .default_size(egui::vec2(760.0, 560.0).min(maximum))
            .min_size(egui::vec2(300.0, 240.0).min(maximum))
            .max_size(maximum)
            .constrain_to(bounds)
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&self.brush.name).strong()).truncate());
                ui.separator();
                let compact = ui.available_width() < 650.0 || ui.available_height() < 440.0;
                // Reserve the action row and state line outside the settings scroll.
                let footer = ui.spacing().interact_size.y
                    + ui.text_style_height(&egui::TextStyle::Small)
                    + 4.0 * ui.spacing().item_spacing.y
                    + 8.0;
                let height = (ui.available_height() - footer).max(32.0);
                let (body_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), height),
                    egui::Sense::hover(),
                );
                let mut body = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("brush studio body")
                        .max_rect(body_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                body.set_clip_rect(body_rect.intersect(ui.clip_rect()));
                {
                    let ui = &mut body;
                    if compact {
                        egui::ComboBox::from_id_salt("brush studio section")
                            .selected_text(self.brush_studio_section.title())
                            .width(ui.available_width())
                            .height((bounds.height() - 100.0).max(60.0))
                            .show_ui(ui, |ui| {
                                for section in Section::STUDIO {
                                    ui.selectable_value(
                                        &mut self.brush_studio_section,
                                        section,
                                        section.title(),
                                    );
                                }
                            });
                        self.brush_studio_settings(ui, true);
                    } else {
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(150.0);
                                egui::ScrollArea::vertical()
                                    .id_salt("brush studio sections")
                                    .max_height(height)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        for section in Section::STUDIO {
                                            ui.selectable_value(
                                                &mut self.brush_studio_section,
                                                section,
                                                section.title(),
                                            );
                                        }
                                    });
                            });
                            ui.separator();
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width());
                                self.brush_studio_settings(ui, false);
                            });
                        });
                    }
                }
                ui.separator();
                let modified = self.brush_preset_modified();
                let valid = self.brush.validate().is_ok();
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.brushes.len() < 512 && valid,
                            egui::Button::new("Save as new"),
                        )
                        .on_hover_text("Add these settings as a new library preset")
                        .on_disabled_hover_text(if self.brushes.len() >= 512 {
                            "The library has 512 presets. Delete one before adding another."
                        } else {
                            "Correct the brush settings shown below before saving."
                        })
                        .clicked()
                    {
                        self.save_new_brush_preset();
                    }
                    if ui
                        .add_enabled(modified && valid, egui::Button::new("Update preset…"))
                        .on_hover_text("Replace the selected preset's saved settings")
                        .clicked()
                    {
                        self.update_brush_preset();
                    }
                    if ui
                        .add_enabled(modified, egui::Button::new("Revert"))
                        .on_hover_text("Restore the selected preset's saved settings")
                        .clicked()
                    {
                        self.revert_brush_preset();
                    }
                });
                if let Err(error) = self.brush.validate() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&error)
                                .small()
                                .color(ui.visuals().error_fg_color),
                        )
                        .truncate(),
                    )
                    .on_hover_text(error);
                } else {
                    ui.small(if self.brush_preset_modified() {
                        "Unsaved preset changes"
                    } else {
                        "Saved preset"
                    });
                }
            });
        self.brush_studio = open;
    }
    fn brush_studio_settings(&mut self, ui: &mut egui::Ui, compact: bool) {
        let preview_height = (ui.available_height() * if compact { 0.2 } else { 0.3 })
            .clamp(32.0, if compact { 64.0 } else { 150.0 });
        self.brush_studio_preview.show(
            ui,
            &self.brush,
            preview_height,
            self.stroking || self.render_queue.busy() || self.performance.defer_optional(),
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("brush studio settings")
            .max_height(ui.available_height().max(1.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.brush_section_ui(ui, self.brush_studio_section);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drawing_defers_new_preview_jobs_until_idle() {
        let ctx = egui::Context::default();
        let mut preview = LivePreview::default();
        let brush = Brush::default();
        preview.update_when_idle(&ctx, &brush, [100, 40], true);
        assert!(preview.job.is_none() && preview.texture.is_none());
        preview.update_when_idle(&ctx, &brush, [100, 40], false);
        assert!(preview.job.is_some());
    }
    #[test]
    fn live_preview_follows_the_newest_settings_and_halves_after_slow_renders() {
        let ctx = egui::Context::default();
        let mut preview = LivePreview::default();
        let settle = |preview: &mut LivePreview, brush: &Brush| {
            for _ in 0..500 {
                preview.update(&ctx, brush, [200, 60]);
                let key = crate::brush_previews::fingerprint(brush);
                if preview.shown == Some((key, [200, 60])) && preview.job.is_none() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            panic!("preview never caught up");
        };
        let first = Brush::default();
        let newer = Brush {
            size: 5.0,
            ..first.clone()
        };
        preview.update(&ctx, &first, [200, 60]);
        // Settings change while the first render runs: the newest wins.
        preview.update(&ctx, &newer, [200, 60]);
        settle(&mut preview, &newer);
        // After a slow render, the next one renders at half resolution but
        // still counts as current for the size that was asked for.
        preview.last_ms = 100.0;
        let heavy = Brush { size: 9.0, ..first };
        preview.update(&ctx, &heavy, [200, 60]);
        assert_eq!(
            preview.job.as_ref().map(|job| job.rendered),
            Some([100, 30])
        );
        settle(&mut preview, &heavy);
        assert_eq!(preview.texture.as_ref().unwrap().size(), [100, 30]);
    }
    #[test]
    fn preview_shows_tip_and_stroke_and_follows_settings() {
        let size = [360, 90];
        let brush = Brush::default();
        let pixels = render(&brush, size);
        assert_eq!(pixels.len(), 360 * 90 * 4);
        let inked = |pixels: &[u8], from: usize, to: usize| {
            (0..90).any(|y| (from..to).any(|x| pixels[(y * 360 + x) * 4] < 128))
        };
        // Ink in the tip box and in the stroke area, paper elsewhere.
        assert!(inked(&pixels, 0, 90));
        assert!(inked(&pixels, 120, 340));
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
        // Lower opacity paints lighter; the preview follows the setting.
        let faint = render(
            &Brush {
                opacity: 0.2,
                ..brush.clone()
            },
            size,
        );
        let darkness = |pixels: &[u8]| {
            pixels
                .chunks_exact(4)
                .map(|p| 255 - p[0] as u32)
                .sum::<u32>()
        };
        assert!(darkness(&faint) < darkness(&pixels) / 2);
        // Very large tips stay within the preview and still render quickly.
        let started = std::time::Instant::now();
        let big = render(
            &Brush {
                size: 300.0,
                hardness: 0.0,
                ..brush
            },
            size,
        );
        assert!(inked(&big, 0, 90));
        assert!(started.elapsed().as_millis() < 2000);
    }
}
