//! App side of `material`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::material`, re-exported
//! here so `crate::material::…` paths keep working.
use crate::{
    Dudel,
    document::{Document, LayerId},
    egui,
};
pub(crate) use dudel_core::material::*;

// ---------------------------------------------------------------------------
// Dialog

const PREVIEW: [usize; 2] = [348, 110];

pub(crate) struct MaterialForm {
    target: LayerId,
    layer_name: String,
    /// None is a plain layer.
    settings: Option<Settings>,
    /// Remembered per kind while the dialog is open, so trying another kind
    /// and coming back keeps the changes.
    kept: Settings,
    paper: Option<crate::paper::Paper>,
    preview: Option<(Option<Settings>, egui::TextureHandle)>,
}

/// A sample stroke on the material, painted and dried by the real engine.
fn preview_image(
    settings: Option<Settings>,
    paper: Option<crate::paper::Paper>,
) -> egui::ColorImage {
    let [w, h] = PREVIEW;
    let mut doc = Document::new(w, h);
    doc.paper = paper;
    doc.layers[0].material = settings.map(Material::new);
    let kind = settings.map(|s| s.kind);
    let brush = match kind {
        Some(Kind::Watercolor) => crate::brush::Brush {
            size: 34.0,
            opacity: 0.9,
            flow: 0.1,
            hardness: 0.7,
            spacing: 0.06,
            pressure_size: false,
            blend: crate::brush::Blend::Wet,
            wetness: 0.9,
            paper_response: 0.2,
            ..Default::default()
        },
        Some(Kind::Paint) => crate::brush::Brush {
            size: 30.0,
            hardness: 0.9,
            spacing: 0.05,
            thickness: 0.8,
            pressure_size: true,
            ..Default::default()
        },
        _ => crate::brush::Brush {
            size: 26.0,
            hardness: 0.7,
            flow: 0.6,
            spacing: 0.05,
            paper_response: 0.0,
            pressure_size: false,
            pressure_opacity: true,
            ..Default::default()
        },
    };
    let color = match kind {
        Some(Kind::Watercolor) => [40, 90, 170],
        Some(Kind::Paint) => [196, 84, 40],
        _ => [60, 52, 64],
    };
    let settings = crate::stroke_buffer::Settings {
        target: crate::paint_target::Target::Raster,
        brush: brush.clone(),
        color,
        secondary: color,
        erase: false,
        layer: 0,
        symmetry: Default::default(),
    };
    let mut stroke = crate::stroke::Stroke::default();
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    let steps = 40;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let sample = crate::stroke::Sample {
            x: 30.0 + t * (w as f32 - 60.0),
            y: h as f32 / 2.0 + (t * std::f32::consts::TAU).sin() * 18.0,
            pressure: 0.35 + 0.65 * (t * std::f32::consts::PI).sin(),
            pressure_known: true,
            ..Default::default()
        };
        let dabs = if i == steps {
            stroke.finish(sample, &brush)
        } else {
            stroke.sample(sample, &brush)
        };
        for dab in dabs {
            buffer.paint(&mut doc, dab, &settings);
        }
    }
    buffer.finish_tail(&mut doc, &settings);
    crate::wet::settle(&mut doc);
    let mut rgba = paper_backdrop(paper);
    let painted = crate::layer_effects::expand(&doc, false, &|| false)
        .map(|doc| doc.composite_transparent())
        .unwrap_or_default();
    for (i, p) in painted.chunks_exact(4).enumerate() {
        let a = f32::from(p[3]) / 255.0;
        for c in 0..3 {
            let under = f32::from(rgba[i * 4 + c]);
            rgba[i * 4 + c] = (f32::from(p[c]) * a + under * (1.0 - a)).round() as u8;
        }
    }
    egui::ColorImage::from_rgba_unmultiplied(PREVIEW, &rgba)
}

/// The paper the preview is painted on, lightly shaded by its height.
fn paper_backdrop(paper: Option<crate::paper::Paper>) -> Vec<u8> {
    let [w, h] = PREVIEW;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let tone = paper.map_or(250.0, |p| {
                236.0 + 16.0 * p.height(x, y) * p.roughness.max(0.3)
            });
            let tone = tone.min(255.0) as u8;
            rgba.extend_from_slice(&[tone, tone, tone.saturating_sub(4), 255]);
        }
    }
    rgba
}

impl Dudel {
    /// Layer › Material…
    pub(crate) fn open_material_dialog(&mut self) {
        let Some(layer) = self.doc.layers.get(self.doc.active) else {
            return;
        };
        if !layer.takes_material() {
            self.status = "Materials belong to raster layers".into();
            return;
        }
        let settings = layer.material.as_ref().map(|m| m.settings);
        self.material_form = Some(MaterialForm {
            target: layer.id,
            layer_name: layer.name.clone(),
            settings,
            kept: settings.unwrap_or(Settings::new(Kind::Watercolor)),
            paper: self.doc.paper,
            preview: None,
        });
    }

    /// Screenshot fixture: a watercolor wash, dry media and thick paint on
    /// cold-pressed paper, with the thick paint layer's material open.
    pub(crate) fn material_capture_fixture(&mut self) {
        let (w, h) = (1400usize, 900usize);
        let mut doc = Document::new(w, h);
        doc.paper = Some(crate::paper::Paper::presets()[1].1);
        doc.layers[0].name = "Paper tone".into();
        let stroke = |doc: &mut Document,
                      brush: &crate::brush::Brush,
                      color: [u8; 3],
                      from: [f32; 2],
                      to: [f32; 2],
                      wave: f32| {
            let settings = crate::stroke_buffer::Settings {
                target: crate::paint_target::Target::Raster,
                brush: brush.clone(),
                color,
                secondary: color,
                erase: false,
                layer: doc.active,
                symmetry: Default::default(),
            };
            let mut path = crate::stroke::Stroke::default();
            let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
            let steps = 90;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let sample = crate::stroke::Sample {
                    x: from[0] + (to[0] - from[0]) * t,
                    y: from[1] + (to[1] - from[1]) * t + (t * std::f32::consts::TAU).sin() * wave,
                    pressure: 0.45 + 0.55 * (t * std::f32::consts::PI).sin(),
                    pressure_known: true,
                    ..Default::default()
                };
                let dabs = if i == steps {
                    path.finish(sample, brush)
                } else {
                    path.sample(sample, brush)
                };
                for dab in dabs {
                    buffer.paint(doc, dab, &settings);
                }
            }
            buffer.finish_tail(doc, &settings);
        };
        // Watercolor washes.
        doc.add_layer();
        doc.layers[1].name = "Watercolor 1".into();
        doc.layers[1].material = Some(Material::new(Settings::new(Kind::Watercolor)));
        let wash = crate::brush::Brush {
            size: 90.0,
            opacity: 0.9,
            flow: 0.07,
            hardness: 0.7,
            spacing: 0.06,
            pressure_size: false,
            blend: crate::brush::Blend::Wet,
            wetness: 0.9,
            granulation: 0.4,
            paper_response: 0.2,
            ..Default::default()
        };
        for (i, color) in [[46, 98, 170], [52, 120, 150], [120, 70, 150]]
            .into_iter()
            .enumerate()
        {
            let y = 140.0 + i as f32 * 70.0;
            stroke(&mut doc, &wash, color, [90.0, y], [620.0, y + 20.0], 16.0);
            crate::wet::settle(&mut doc);
        }
        // Yellow into wet blue: the pigments make green.
        let mixing = crate::brush::Brush {
            size: 60.0,
            flow: 0.18,
            ..wash.clone()
        };
        stroke(
            &mut doc,
            &mixing,
            [24, 50, 190],
            [110.0, 395.0],
            [600.0, 395.0],
            0.0,
        );
        stroke(
            &mut doc,
            &mixing,
            [250, 215, 30],
            [110.0, 425.0],
            [600.0, 395.0],
            0.0,
        );
        crate::wet::settle(&mut doc);
        // Dry media.
        doc.add_layer();
        doc.layers[2].name = "Dry media 1".into();
        doc.layers[2].material = Some(Material::new(Settings::new(Kind::Dry)));
        let chalk = crate::brush::Brush {
            size: 34.0,
            hardness: 0.6,
            flow: 0.5,
            spacing: 0.05,
            paper_response: 0.0,
            pressure_size: false,
            pressure_opacity: true,
            ..Default::default()
        };
        for (i, color) in [[40, 36, 44], [180, 60, 60]].into_iter().enumerate() {
            let y = 560.0 + i as f32 * 90.0;
            stroke(&mut doc, &chalk, color, [90.0, y], [620.0, y - 30.0], 22.0);
        }
        // Thick paint.
        doc.add_layer();
        doc.layers[3].name = "Thick paint 1".into();
        doc.layers[3].material = Some(Material::new(Settings::new(Kind::Paint)));
        let knife = crate::brush::Brush {
            size: 70.0,
            hardness: 0.92,
            spacing: 0.04,
            thickness: 0.85,
            pressure_size: true,
            ..Default::default()
        };
        for (i, color) in [
            [214, 92, 44],
            [236, 186, 60],
            [64, 128, 104],
            [230, 226, 214],
        ]
        .into_iter()
        .enumerate()
        {
            let y = 170.0 + i as f32 * 150.0;
            stroke(
                &mut doc,
                &knife,
                color,
                [760.0, y],
                [1300.0, y + 60.0],
                30.0,
            );
        }
        // A knife dragged down through the paint carries it and its height.
        let knife = crate::brush::Brush {
            size: 50.0,
            hardness: 0.9,
            opacity: 0.9,
            spacing: 0.04,
            pressure_size: false,
            blend: crate::brush::Blend::Smudge,
            ..Default::default()
        };
        stroke(
            &mut doc,
            &knife,
            [0, 0, 0],
            [1180.0, 150.0],
            [1180.0, 760.0],
            0.0,
        );
        doc.active = 3;
        self.doc = doc;
        self.changed();
        self.open_material_dialog();
    }

    /// "New material layer" with one entry per kind.
    pub(crate) fn new_material_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("New material layer", |ui| {
            for kind in Kind::ALL {
                if ui
                    .button(kind.name())
                    .on_hover_text(kind.describe())
                    .clicked()
                {
                    self.new_material_layer(kind);
                    ui.close();
                }
            }
        });
    }

    /// A new raster layer holding `kind`, above the selected entry and
    /// selected afterwards.
    pub(crate) fn new_material_layer(&mut self, kind: Kind) {
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let Some(active) = self.input_target() else {
            self.status = "Selected layer no longer exists".into();
            return;
        };
        let id = LayerId::new();
        let edit = crate::document_edits::Edit::NewMaterial {
            id,
            settings: Settings::new(kind),
        };
        if !self.render_queue.busy() {
            self.dispatch_document_edit(edit, "New material layer", "No layer created");
            return;
        }
        let context = self
            .pending_layers
            .clone()
            .unwrap_or_else(|| crate::pending_layers::PendingLayers::from_document(&self.doc));
        let projected = match context.with_new_layer(active, id) {
            Ok(projected) => projected,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if self.dispatch_document_edit(edit, "New material layer", "No layer created") {
            self.pending_layers = Some(projected);
            self.logical_layer = Some(id);
        }
    }

    pub(crate) fn material_dialog(&mut self, ctx: &egui::Context) {
        let Some(form) = &mut self.material_form else {
            if self.material_preview.take().is_some() {
                self.dirty = true;
            }
            return;
        };
        let (mut apply, mut cancel) = (false, false);
        let canvas = self.canvas_area;
        let mut modal = egui::Modal::new(egui::Id::new("material dialog"))
            .backdrop_color(egui::Color32::TRANSPARENT);
        if canvas.width() >= 640.0 && canvas.height() >= 360.0 {
            modal = modal.area(
                egui::Modal::default_area(egui::Id::new("material dialog")).anchor(
                    egui::Align2::RIGHT_TOP,
                    canvas.right_top().to_vec2() - ctx.content_rect().right_top().to_vec2()
                        + egui::vec2(-12.0, 12.0),
                ),
            );
        }
        let response = modal.show(ctx, |ui| {
            ui.set_width(PREVIEW[0] as f32 + 16.0);
            cancel |= crate::dialogs::dialog_title(ui, "Layer material");
            ui.label(egui::RichText::new(&form.layer_name).weak());
            if form
                .preview
                .as_ref()
                .is_none_or(|(shown, _)| *shown != form.settings)
            {
                let image = preview_image(form.settings, form.paper);
                match &mut form.preview {
                    Some((shown, texture)) => {
                        texture.set(image, egui::TextureOptions::LINEAR);
                        *shown = form.settings;
                    }
                    None => {
                        form.preview = Some((
                            form.settings,
                            ctx.load_texture("material preview", image, egui::TextureOptions::LINEAR),
                        ));
                    }
                }
            }
            if let Some((_, texture)) = &form.preview {
                ui.image((texture.id(), egui::vec2(PREVIEW[0] as f32, PREVIEW[1] as f32)));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(form.settings.is_none(), "Plain")
                    .on_hover_text("An ordinary layer: paint is only color")
                    .clicked()
                    && let Some(settings) = form.settings.take()
                {
                    form.kept = settings;
                }
                for kind in Kind::ALL {
                    let chosen = form.settings.is_some_and(|s| s.kind == kind);
                    if ui
                        .selectable_label(chosen, kind.name())
                        .on_hover_text(kind.describe())
                        .clicked()
                        && !chosen
                    {
                        let mut settings = form.settings.unwrap_or(form.kept);
                        settings.kind = kind;
                        form.settings = Some(settings);
                    }
                }
            });
            if let Some(settings) = &mut form.settings {
                ui.label(egui::RichText::new(settings.kind.describe()).small());
                ui.add_space(4.0);
                let slider = |ui: &mut egui::Ui, label: &str, value: &mut f32, hint: &str| {
                    crate::workspace::labeled_slider(ui, label, egui::Slider::new(value, 0.0..=1.0))
                        .on_hover_text(hint);
                };
                match settings.kind {
                    Kind::Watercolor => {
                        slider(ui, "Edge darkening", &mut settings.edges, "How much paint gathers at the edges of a wash as it dries");
                        slider(ui, "Blooms", &mut settings.blooms, "How far fresh water pushes paint that is drying");
                    }
                    Kind::Dry => {
                        slider(ui, "Tooth", &mut settings.tooth, "How much the paper's tooth catches every stroke on this layer");
                    }
                    Kind::Paint => {
                        crate::workspace::labeled_slider(
                            ui,
                            "Light from",
                            egui::Slider::new(&mut settings.light, -180.0..=180.0)
                                .suffix("°")
                                .integer(),
                        )
                        .on_hover_text("Where the light comes from: 0° the right, 90° the top, 135° the upper left");
                        slider(ui, "Depth", &mut settings.depth, "How deep the paint looks");
                        slider(ui, "Shine", &mut settings.shine, "How glossy the ridges are");
                    }
                }
            }
            ui.small(
                "The material is saved with the layer. Watercolor and dry media change how new \
                 strokes behave; thick paint's light shows on the canvas as you change it.",
            );
            crate::dialogs::dialog_buttons(ui, |ui| {
                apply = ui.add(crate::workspace::primary_button(ui, "Apply")).clicked();
                cancel |= ui.button("Cancel").clicked();
            });
        });
        // Thick paint's light is tried on the canvas itself while the dialog
        // is open; the other settings only change strokes to come.
        let preview = self.material_form.as_ref().and_then(|form| {
            form.settings
                .filter(|s| s.kind == Kind::Paint)
                .map(|s| (form.target, s))
        });
        if preview != self.material_preview {
            self.material_preview = preview;
            self.dirty = true;
        }
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if apply {
            self.material_preview = None;
            let Some(form) = self.material_form.take() else {
                return;
            };
            match self.doc.layers.iter().position(|l| l.id == form.target) {
                Some(index) => {
                    if index == self.doc.active || self.select_layer_target(index) {
                        self.dispatch_document_edit(
                            crate::document_edits::Edit::Material(form.settings),
                            "Material changed",
                            "The layer already has this material",
                        );
                    }
                }
                None => self.status = "The layer is gone".into(),
            }
        } else if cancel || (response.should_close() && escape) {
            self.material_form = None;
            self.material_preview = None;
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dudel, document::Document, egui};

    fn paint(doc: &mut Document, brush: &crate::brush::Brush, color: [u8; 3], y: f32, erase: bool) {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color,
            secondary: color,
            erase,
            layer: doc.active,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=20 {
            let sample = crate::stroke::Sample {
                x: 20.0 + i as f32 * 6.0,
                y,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 20 {
                path.finish(sample, brush)
            } else {
                path.sample(sample, brush)
            };
            for dab in dabs {
                buffer.paint(doc, dab, &settings);
            }
        }
        buffer.finish_tail(doc, &settings);
    }

    fn thick() -> crate::brush::Brush {
        crate::brush::Brush {
            size: 24.0,
            hardness: 0.9,
            spacing: 0.08,
            thickness: 0.8,
            pressure_size: false,
            ..Default::default()
        }
    }

    #[test]
    fn the_dialog_tries_a_light_on_the_canvas_without_changing_the_document() {
        let mut app = Dudel {
            doc: Document::new(160, 60),
            ..Default::default()
        };
        app.doc.layers[0].material = Some(Material::new(Settings::new(Kind::Paint)));
        paint(&mut app.doc, &thick(), [200, 80, 40], 30.0, false);
        let saved = app.doc.layers[0].material.as_ref().unwrap().settings;
        let ctx = egui::Context::default();
        let frame = |app: &mut Dudel| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.material_dialog(ctx));
        };
        app.open_material_dialog();
        let form = app.material_form.as_mut().unwrap();
        form.settings.as_mut().unwrap().light = -45.0;
        app.dirty = false;
        frame(&mut app);
        let (layer, tried) = app.material_preview.expect("the light is being tried");
        assert_eq!((layer, tried.light), (app.doc.layers[0].id, -45.0));
        assert!(app.dirty, "the canvas redraws with it");
        assert_eq!(app.doc.layers[0].material.as_ref().unwrap().settings, saved);
        // Closing without Apply puts the canvas back.
        app.material_form = None;
        frame(&mut app);
        assert!(app.material_preview.is_none());
    }
}

#[cfg(test)]
mod look_app {

    #[test]
    #[ignore = "writes a picture for a person to look at"]
    fn knife_picture() {
        let mut app = crate::Dudel::default();
        app.material_capture_fixture();
        let doc = &app.doc;
        let expanded = crate::layer_effects::expand(doc, false, &|| false).unwrap();
        let rgba = expanded.composite_display_region([700, 100, 1400, 900], false);
        let path = std::env::var("DUDEL_LOOK").unwrap();
        image::save_buffer(path, &rgba, 700, 800, image::ExtendedColorType::Rgba8).unwrap();
    }
}
