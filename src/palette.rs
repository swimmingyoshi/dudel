use crate::{Dudel, read_bounded, storage};
use serde::{Deserialize, Serialize};

pub fn defaults() -> Vec<[u8; 3]> {
    vec![
        [38, 44, 59],
        [255, 255, 255],
        [211, 78, 91],
        [235, 170, 79],
        [82, 166, 137],
        [78, 132, 204],
        [159, 111, 193],
    ]
}
pub fn validate(colors: &[[u8; 3]]) -> Result<(), String> {
    if colors.len() > 64 {
        return Err("A palette supports up to 64 swatches".into());
    }
    let mut unique = std::collections::HashSet::new();
    if colors.iter().any(|color| !unique.insert(*color)) {
        return Err("Palette contains duplicate swatches".into());
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct PaletteFile {
    format: String,
    version: u32,
    colors: Vec<[u8; 3]>,
}
impl PaletteFile {
    fn decode(source: &str) -> Result<Vec<[u8; 3]>, String> {
        let palette: Self = serde_json::from_str(source).map_err(|error| error.to_string())?;
        if palette.format != "dudel-palette" || palette.version != 1 {
            return Err("Unsupported palette format/version".into());
        }
        validate(&palette.colors)?;
        Ok(palette.colors)
    }
}
impl Dudel {
    pub(crate) fn palette_ui(&mut self, ui: &mut eframe::egui::Ui) {
        if !self.input_stroke_target().is_raster() {
            self.mask_palette_ui(ui);
            return;
        }
        use crate::color_wheel;
        color_wheel::sync(&mut self.hsv, self.color);
        let size = ui.available_width().clamp(140.0, 200.0);
        ui.vertical_centered(|ui| {
            if color_wheel::wheel(ui, &mut self.hsv, size) {
                self.color = color_wheel::hsv_to_rgb(self.hsv);
            }
        });
        self.palette_current_color_ui(ui);
        self.palette_swatches_ui(ui);
    }

    pub(crate) fn compact_palette_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use crate::color_wheel::{self, color32};
        use eframe::egui;
        if !self.input_stroke_target().is_raster() {
            egui::ScrollArea::vertical()
                .id_salt("compact mask palette")
                .show(ui, |ui| self.mask_palette_ui(ui));
            return;
        }
        color_wheel::sync(&mut self.hsv, self.color);
        let size = (ui.available_height() - 78.0)
            .clamp(80.0, 168.0)
            .min(ui.available_width());
        ui.vertical_centered(|ui| {
            if color_wheel::wheel(ui, &mut self.hsv, size) {
                self.color = color_wheel::hsv_to_rgb(self.hsv);
            }
        });
        self.palette_current_color_ui(ui);
        ui.horizontal(|ui| {
            ui.menu_button("Swatches", |ui| {
                ui.set_width(240.0);
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| self.palette_swatches_ui(ui));
            });
            let count = ((ui.available_width() + 4.0) / 22.0).floor().max(0.0) as usize;
            for color in self.palette.iter().copied().take(count.min(8)) {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::click());
                ui.painter().rect_filled(rect, 3.0, color32(color));
                if response
                    .on_hover_text(format!("#{}", color_wheel::hex(color)))
                    .clicked()
                {
                    self.color = color;
                }
            }
        });
    }

    fn palette_current_color_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use crate::color_wheel;
        use eframe::egui;
        ui.horizontal(|ui| {
            crate::workspace::color_chips(ui, &mut self.color, &mut self.secondary, 40.0);
            ui.label("#");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.hex_input)
                    .desired_width(64.0)
                    .char_limit(7)
                    .font(egui::TextStyle::Monospace),
            );
            if edit.lost_focus() {
                match color_wheel::parse_hex(&self.hex_input) {
                    Some(color) => self.color = color,
                    None => self.status = "Enter a color as RRGGBB or RGB hex".into(),
                }
            }
            if !edit.has_focus() {
                self.hex_input = color_wheel::hex(self.color);
            }
            ui.color_edit_button_srgb(&mut self.color)
                .on_hover_text("Exact RGB values");
        });
    }

    fn palette_swatches_ui(&mut self, ui: &mut eframe::egui::Ui) {
        use crate::color_wheel::{self, color32};
        use eframe::egui;
        if !self.recent_colors.is_empty() {
            ui.label(egui::RichText::new("Recent").small().weak());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                for color in self.recent_colors.clone() {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 3.0, color32(color));
                    if response
                        .on_hover_text(format!("#{}", color_wheel::hex(color)))
                        .clicked()
                    {
                        self.color = color;
                    }
                }
            });
        }
        ui.label(egui::RichText::new("Swatches").small().weak());
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for (index, color) in self.palette.iter().copied().enumerate() {
                let selected = self.color == color;
                let response = ui
                    .add(
                        egui::Button::new("  ")
                            .min_size(egui::vec2(32.0, 32.0))
                            .fill(egui::Color32::from_rgb(color[0], color[1], color[2]))
                            .stroke(egui::Stroke::new(
                                if selected { 2.0_f32 } else { 1.0_f32 },
                                if selected {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_gray(70)
                                },
                            )),
                    )
                    .on_hover_text(format!(
                        "#{:02X}{:02X}{:02X} · right-click to remove",
                        color[0], color[1], color[2]
                    ));
                if response.clicked() {
                    self.color = color;
                }
                response.context_menu(|ui| {
                    if ui.button("Remove swatch").clicked() {
                        remove = Some(index);
                        ui.close();
                    }
                });
            }
        });
        if let Some(index) = remove {
            self.palette.remove(index);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.palette.len() < 64 && !self.palette.contains(&self.color),
                    egui::Button::new("+ Save color"),
                )
                .clicked()
            {
                self.palette.push(self.color);
            }
            ui.menu_button("Palette", |ui| {
                if ui.button("Import palette…").clicked() {
                    ui.close();
                    self.palette_file(true);
                }
                if ui.button("Export palette…").clicked() {
                    ui.close();
                    self.palette_file(false);
                }
                ui.separator();
                self.palette_extract_menu_ui(ui);
            });
        });
        self.palette_assist_ui(ui);
    }
    fn palette_file(&mut self, import: bool) {
        let dialog = self.dialog_in(
            crate::locations::Kind::Palettes,
            rfd::AsyncFileDialog::new().add_filter("Dudel palette", &["dudelpalette"]),
        );
        if import {
            self.ask_open_file(dialog, Self::import_palette);
        } else {
            let dialog = dialog.set_file_name("My colors.dudelpalette");
            self.ask_save_file(dialog, Self::export_palette);
        }
    }
    pub(crate) fn import_palette(&mut self, path: std::path::PathBuf) {
        match read_bounded(&path, 16 * 1024).and_then(|source| PaletteFile::decode(&source)) {
            Ok(colors) => {
                let new: Vec<_> = colors
                    .into_iter()
                    .filter(|color| !self.palette.contains(color))
                    .collect();
                if self.palette.len() + new.len() > 64 {
                    self.status = "Import exceeds the 64-swatch limit".into();
                    return;
                }
                let count = new.len();
                self.palette.extend(new);
                self.status = format!("Added {count} palette colors");
            }
            Err(error) => self.status = format!("Palette import: {error}"),
        }
    }
    fn export_palette(&mut self, path: std::path::PathBuf) {
        let file = PaletteFile {
            format: "dudel-palette".into(),
            version: 1,
            colors: self.palette.clone(),
        };
        self.status = match serde_json::to_vec_pretty(&file)
            .map_err(|error| error.to_string())
            .and_then(|data| storage::atomic_write(&path, &data))
        {
            Ok(()) => "Palette exported".into(),
            Err(error) => format!("Palette export: {error}"),
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_palettes_reject_duplicates_limits_and_unknown_versions() {
        let mut file = PaletteFile {
            format: "dudel-palette".into(),
            version: 1,
            colors: defaults(),
        };
        assert_eq!(
            PaletteFile::decode(&serde_json::to_string(&file).unwrap()).unwrap(),
            defaults()
        );
        file.colors.push(file.colors[0]);
        assert!(PaletteFile::decode(&serde_json::to_string(&file).unwrap()).is_err());
        file.colors = defaults();
        file.version = 2;
        assert!(PaletteFile::decode(&serde_json::to_string(&file).unwrap()).is_err());
        assert!(validate(&(0..65).map(|i| [i, 0, 0]).collect::<Vec<_>>()).is_err());
    }
}
