//! App side of `paper`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::paper`, re-exported
//! here so `crate::paper::…` paths keep working.
use crate::{Dudel, egui};
pub(crate) use dudel_core::paper::*;

/// The Canvas > Paper dialog's state.
pub(crate) struct PaperForm {
    paper: Option<Paper>,
    preview: Option<(Option<Paper>, egui::TextureHandle)>,
}

const PREVIEW: [usize; 2] = [320, 110];

/// The surface lit from the upper left, with a soft and a firm stroke.
fn preview_image(paper: Option<Paper>) -> egui::ColorImage {
    let [w, h] = PREVIEW;
    let mut doc = crate::document::Document::new(w, h);
    doc.paper = paper;
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let shade = paper.map_or(244.0, |p| {
                let slope = p.height(x, y) - p.height(x + 1, y + 1);
                244.0 + slope * p.roughness * 70.0 - p.roughness * 6.0
            });
            let v = shade.clamp(200.0, 255.0) as u8;
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v.saturating_sub(4), 255]);
        }
    }
    for (row, brush) in [
        crate::brush::Brush {
            size: 26.0,
            hardness: 0.3,
            opacity: 0.9,
            flow: 0.35,
            spacing: 0.08,
            pressure_size: false,
            pressure_opacity: true,
            ..Default::default()
        },
        crate::brush::Brush {
            size: 14.0,
            hardness: 0.9,
            spacing: 0.06,
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [38, 44, 59],
            secondary: [38, 44, 59],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut stroke = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=60 {
            let t = i as f32 / 60.0;
            let sample = crate::stroke::Sample {
                x: 20.0 + t * (w as f32 - 40.0),
                y: 34.0 + row as f32 * 44.0 + (t * std::f32::consts::TAU).sin() * 8.0,
                pressure: 0.15 + 0.85 * t,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 60 {
                stroke.finish(sample, &brush)
            } else {
                stroke.sample(sample, &brush)
            };
            for dab in dabs {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
    }
    for (i, p) in doc.layers[0]
        .pixels
        .region(w, [0, 0, w, h])
        .chunks_exact(4)
        .enumerate()
    {
        let a = f32::from(p[3]) / 255.0;
        for c in 0..3 {
            let under = f32::from(rgba[i * 4 + c]);
            rgba[i * 4 + c] = (f32::from(p[c]) * a + under * (1.0 - a)).round() as u8;
        }
    }
    egui::ColorImage::from_rgba_unmultiplied(PREVIEW, &rgba)
}

impl Dudel {
    /// Canvas > Paper…
    pub(crate) fn open_paper_dialog(&mut self) {
        self.paper_form = Some(PaperForm {
            paper: self.doc.paper,
            preview: None,
        });
    }

    pub(crate) fn paper_dialog(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.paper_form else {
            return;
        };
        let (mut apply, mut cancel) = (false, false);
        let response = crate::dialogs::modal(ctx, egui::Id::new("paper dialog")).show(ctx, |ui| {
            ui.set_width(PREVIEW[0] as f32 + 16.0);
            cancel |= crate::dialogs::dialog_title(ui, "Paper");
            if form
                .preview
                .as_ref()
                .is_none_or(|(shown, _)| *shown != form.paper)
            {
                let image = preview_image(form.paper);
                match &mut form.preview {
                    Some((shown, texture)) => {
                        texture.set(image, egui::TextureOptions::NEAREST);
                        *shown = form.paper;
                    }
                    None => {
                        form.preview = Some((
                            form.paper,
                            ctx.load_texture("paper preview", image, egui::TextureOptions::NEAREST),
                        ));
                    }
                }
            }
            if let Some((_, texture)) = &form.preview {
                ui.image((
                    texture.id(),
                    egui::vec2(PREVIEW[0] as f32, PREVIEW[1] as f32),
                ));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(form.paper.is_none(), "Smooth")
                    .on_hover_text("No paper texture: brushes paint as on a plain canvas")
                    .clicked()
                {
                    form.paper = None;
                }
                for (name, preset) in Paper::presets() {
                    if ui
                        .selectable_label(form.paper == Some(preset), name)
                        .clicked()
                    {
                        form.paper = Some(preset);
                    }
                }
            });
            if let Some(paper) = &mut form.paper {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Surface");
                    for pattern in Pattern::ALL {
                        ui.selectable_value(&mut paper.pattern, pattern, pattern.name());
                    }
                });
                crate::workspace::labeled_slider(
                    ui,
                    "Roughness",
                    egui::Slider::new(&mut paper.roughness, 0.0..=1.0),
                )
                .on_hover_text(
                    "How deep the valleys are: rougher paper breaks light strokes up more",
                );
                crate::workspace::labeled_slider(
                    ui,
                    "Grain size",
                    egui::Slider::new(&mut paper.scale, 1.0..=64.0)
                        .logarithmic(true)
                        .suffix(" px"),
                );
                crate::workspace::labeled_slider(
                    ui,
                    "Absorbency",
                    egui::Slider::new(&mut paper.absorbency, 0.0..=1.0),
                )
                .on_hover_text(
                    "Absorbent paper draws paint into its valleys, so strokes cover more evenly",
                );
                if matches!(paper.pattern, Pattern::Weave | Pattern::Fibers) {
                    crate::workspace::labeled_slider(
                        ui,
                        "Direction",
                        egui::Slider::new(&mut paper.direction, -90.0..=90.0)
                            .suffix("°")
                            .integer(),
                    );
                }
            }
            ui.small(
                "Paper changes how brushes lay paint down from now on; paint already on the \
                 canvas stays as it is. Each brush's Paper texture setting (Brush settings › \
                 Texture) says how much it feels the paper. The paper is saved with the project.",
            );
            crate::dialogs::dialog_buttons(ui, |ui| {
                apply = ui
                    .add(crate::workspace::primary_button(ui, "Apply"))
                    .clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        if apply {
            let paper = self.paper_form.take().and_then(|form| form.paper);
            self.dispatch_document_edit(
                crate::document_edits::Edit::Paper(paper),
                "Paper changed",
                "The canvas already has this paper",
            );
        } else if cancel || response.should_close() {
            self.paper_form = None;
        }
    }
}

/// `DUDEL_PAPER_SHEET=out.png cargo test paper_sheet -- --ignored` draws a
/// soft and a firm stroke on smooth canvas and on every built-in paper.
#[cfg(test)]
#[test]
#[ignore = "writes a picture for review"]
fn paper_sheet() {
    let Ok(path) = std::env::var("DUDEL_PAPER_SHEET") else {
        return;
    };
    let papers: Vec<Option<Paper>> = std::iter::once(None)
        .chain(Paper::presets().into_iter().map(|(_, p)| Some(p)))
        .collect();
    let [w, h] = PREVIEW;
    let mut sheet = Vec::with_capacity(w * h * 4 * papers.len());
    for paper in &papers {
        sheet.extend(preview_image(*paper).as_raw());
    }
    image::save_buffer(
        path,
        &sheet,
        w as u32,
        (h * papers.len()) as u32,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
}
