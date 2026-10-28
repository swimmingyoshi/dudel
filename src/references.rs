use crate::{Dudel, storage};
use eframe::egui;
use std::path::Path;

pub struct Reference {
    name: String,
    texture: egui::TextureHandle,
}
pub struct References {
    pub open: bool,
    images: Vec<Reference>,
    active: usize,
    fit: bool,
    zoom: f32,
}
impl Default for References {
    fn default() -> Self {
        Self {
            open: false,
            images: vec![],
            active: 0,
            fit: true,
            zoom: 1.0,
        }
    }
}

impl References {
    pub(crate) fn visit_textures(&self, mut visit: impl FnMut(&egui::TextureHandle)) {
        for reference in &self.images {
            visit(&reference.texture);
        }
    }
}

fn decode(path: &Path) -> Result<egui::ColorImage, String> {
    // Reuse bounded PNG decoding, then retain only a display-sized reference.
    let mut doc = storage::import_png(path)?;
    let rgba = image::RgbaImage::from_raw(
        doc.width as u32,
        doc.height as u32,
        std::mem::take(&mut doc.layers[0].pixels).into_vec(),
    )
    .ok_or("Invalid reference pixels")?;
    let factor = (1536.0 / doc.width.max(doc.height) as f32).min(1.0);
    let resized = image::imageops::resize(
        &rgba,
        (doc.width as f32 * factor).round().max(1.0) as u32,
        (doc.height as f32 * factor).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    );
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [resized.width() as usize, resized.height() as usize],
        resized.as_raw(),
    ))
}
impl Dudel {
    pub(crate) fn reference_capture_fixture(&mut self, ctx: &egui::Context) {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [self.doc.width, self.doc.height],
            &self.doc.composite(),
        );
        let texture = ctx.load_texture("capture reference", image, egui::TextureOptions::LINEAR);
        self.references.images.push(Reference {
            name: "Color study.png".into(),
            texture,
        });
        self.references.open = true;
    }
    pub(crate) fn references_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.references.open;
        egui::Window::new("References").open(&mut open).default_width(360.0).default_height(440.0).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(self.references.images.len() < 4, egui::Button::new("+ Add PNG…")).clicked() {
                    let ctx = ctx.clone();
                    self.ask_open_file(rfd::AsyncFileDialog::new().add_filter("PNG reference", &["png"]), move |app, path| {
                        match decode(&path) {
                            Ok(image) => {
                                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                                let texture = ctx.load_texture(format!("reference-{name}"), image, egui::TextureOptions::LINEAR);
                                app.references.images.push(Reference { name, texture });
                                app.references.active = app.references.images.len() - 1;
                                app.references.fit = true;
                                app.status = "Reference added · artwork unchanged".into();
                            }
                            Err(error) => app.status = format!("Reference: {error}"),
                        }
                    });
                }
                if ui.add_enabled(!self.references.images.is_empty(), egui::Button::new("Remove")).clicked() {
                    self.references.images.remove(self.references.active);
                    self.references.active = self.references.active.min(self.references.images.len().saturating_sub(1));
                }
            });
            if self.references.images.is_empty() { ui.label("Add a visual reference to keep beside your painting. Up to four PNGs per session; references are separate from the project."); return; }
            egui::ComboBox::from_id_salt("reference chooser").selected_text(&self.references.images[self.references.active].name).show_ui(ui, |ui| {
                for (index, image) in self.references.images.iter().enumerate() {
                    if ui.selectable_value(&mut self.references.active, index, &image.name).changed() { self.references.fit = true; }
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.references.fit, "Fit");
                if ui.add(egui::Slider::new(&mut self.references.zoom, 0.1..=4.0).text("Zoom")).changed() { self.references.fit = false; }
            });
            let texture = &self.references.images[self.references.active].texture;
            let natural = texture.size_vec2();
            let scale = if self.references.fit {
                (ui.available_width() / natural.x).min(ui.available_height().max(100.0) / natural.y).max(0.01)
            } else { self.references.zoom };
            egui::ScrollArea::both().id_salt(("reference image", self.references.active)).show(ui, |ui| {
                ui.add(egui::Image::new(texture).fit_to_exact_size(natural * scale));
            });
        });
        self.references.open = open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_decode_bounds_display_size_and_preserves_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference.png");
        let pixels = vec![128; 2048 * 4];
        image::save_buffer_with_format(
            &path,
            &pixels,
            2048,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let decoded = decode(&path).unwrap();
        assert_eq!(decoded.size, [1536, 1]);
        assert_eq!(decoded.pixels[0].a(), 128);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::write(&path, b"not a PNG").unwrap();
        assert!(decode(&path).is_err());
    }
}
