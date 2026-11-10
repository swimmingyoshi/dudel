//! The Export image window: choose a format and its settings, see what the
//! conversion does to the picture, then choose where to write it. Export
//! again repeats the last export of the project, to the same file.
//!
//! Settings are preferences, never part of the artwork; the last export is
//! remembered per open project, for the session.
use crate::{Dudel, egui};
use std::path::PathBuf;

/// Longest side of the preview, in pixels.
const PREVIEW: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Format {
    #[default]
    Png,
    Jpeg,
    Webp,
    Bmp,
    Gif,
}

impl Format {
    pub(crate) const ALL: [Self; 5] = [Self::Png, Self::Jpeg, Self::Webp, Self::Bmp, Self::Gif];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Webp => "WebP",
            Self::Bmp => "BMP",
            Self::Gif => "GIF",
        }
    }
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
            Self::Gif => "gif",
        }
    }
    pub(crate) fn image_format(self) -> image::ImageFormat {
        match self {
            Self::Png => image::ImageFormat::Png,
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::Webp => image::ImageFormat::WebP,
            Self::Bmp => image::ImageFormat::Bmp,
            Self::Gif => image::ImageFormat::Gif,
        }
    }
    /// The format a file name asks for, when it is one of these.
    pub(crate) fn of_path(path: &std::path::Path) -> Option<Self> {
        let format = image::ImageFormat::from_path(path).ok()?;
        Self::ALL.into_iter().find(|f| f.image_format() == format)
    }
    /// Whether the format can keep see-through areas.
    pub(crate) fn keeps_transparency(self) -> bool {
        !matches!(self, Self::Jpeg | Self::Bmp)
    }
    /// What the format does to a picture, in a sentence or two.
    fn description(self) -> &'static str {
        match self {
            Self::Png => "Lossless, and keeps the document's resolution (DPI).",
            Self::Jpeg => {
                "Lossy and small. Lower quality makes a smaller file with blurrier \
                 edges. JPEG can't be see-through."
            }
            Self::Webp => "Lossless, usually smaller than PNG.",
            Self::Bmp => "Uncompressed, so files are large. BMP can't be see-through.",
            Self::Gif => {
                "At most 256 colors, so gradients and soft shading band. A pixel is \
                 either fully see-through or not at all."
            }
        }
    }
}

/// How the next image export is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ExportSettings {
    pub(crate) format: Format,
    /// Keep see-through areas, where the format can.
    pub(crate) transparent: bool,
    /// JPEG quality, 1 to 100.
    pub(crate) quality: u8,
    /// What see-through areas become when flattened: None is white paper.
    pub(crate) background: Option<[u8; 3]>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: Format::Png,
            transparent: false,
            quality: 92,
            background: None,
        }
    }
}

impl ExportSettings {
    /// Make saved settings safe to use.
    pub(crate) fn sanitized(mut self) -> Self {
        self.quality = self.quality.clamp(1, 100);
        self
    }
    /// Whether the picture keeps see-through areas.
    pub(crate) fn keeps_transparency(&self) -> bool {
        self.transparent && self.format.keeps_transparency()
    }
    /// What happens to see-through areas, when they don't stay.
    fn flattening(&self) -> String {
        match self.background {
            None => "See-through areas become white.".into(),
            Some(_) => "See-through areas become the background color.".into(),
        }
    }
}

/// The project's last image export, which Export again repeats.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LastExport {
    pub(crate) path: PathBuf,
    pub(crate) settings: ExportSettings,
}

/// The open Export image window.
#[derive(Default)]
pub(crate) struct ExportWindow {
    pub(crate) open: bool,
    /// The picture reduced to preview size, straight RGBA, and its size.
    source: Option<(Vec<u8>, [usize; 2])>,
    /// The preview as the settings convert it, and the settings it shows.
    preview: Option<(ExportSettings, egui::TextureHandle)>,
}

/// Straight RGBA flattened onto `background`, where it isn't kept.
pub(crate) fn flatten(rgba: &mut [u8], background: [u8; 3]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        for channel in 0..3 {
            let over = u32::from(pixel[channel]) * alpha;
            let under = u32::from(background[channel]) * (255 - alpha);
            pixel[channel] = ((over + under + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
}

/// The picture as `settings` will write it: flattened where transparency
/// isn't kept, and through the format's own encoder for the lossy ones, so
/// that JPEG blur and GIF banding show as they will.
fn converted(rgba: &[u8], size: [usize; 2], settings: &ExportSettings) -> Vec<u8> {
    let mut rgba = rgba.to_vec();
    if !settings.keeps_transparency() {
        flatten(&mut rgba, settings.background.unwrap_or([255; 3]));
    }
    if !matches!(settings.format, Format::Jpeg | Format::Gif) {
        return rgba;
    }
    let [width, height] = size.map(|v| v as u32);
    let Some(image) = image::RgbaImage::from_raw(width, height, rgba.clone()) else {
        return rgba;
    };
    let mut bytes = std::io::Cursor::new(Vec::new());
    let written = match settings.format {
        Format::Jpeg => {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, settings.quality)
                .encode_image(&image::DynamicImage::ImageRgba8(image).to_rgb8())
        }
        _ => image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, settings.format.image_format()),
    };
    written
        .ok()
        .and_then(|()| image::load_from_memory(bytes.get_ref()).ok())
        .map_or(rgba, |decoded| decoded.into_rgba8().into_raw())
}

impl Dudel {
    /// File > Export image: open the window, with transparency kept when asked.
    pub(crate) fn open_export_window(&mut self, transparent: bool) {
        if transparent {
            self.export_settings.transparent = true;
            if !self.export_settings.format.keeps_transparency() {
                self.export_settings.format = Format::Png;
            }
        }
        self.export_window = ExportWindow {
            open: true,
            ..Default::default()
        };
    }

    /// The picture reduced to at most `PREVIEW` pixels on its longest side,
    /// one sample per preview pixel.
    fn export_preview_source(&self) -> (Vec<u8>, [usize; 2]) {
        let [width, height] = [self.doc.width.max(1), self.doc.height.max(1)];
        let scale = (PREVIEW as f64 / width.max(height) as f64).min(1.0);
        let size = [width, height].map(|v| ((v as f64 * scale).round() as usize).max(1));
        let mut rgba = Vec::with_capacity(size[0] * size[1] * 4);
        for y in 0..size[1] {
            let source_y = ((y as f64 + 0.5) / scale) as usize;
            for x in 0..size[0] {
                let source_x = ((x as f64 + 0.5) / scale) as usize;
                let index = source_y.min(height - 1) * width + source_x.min(width - 1);
                rgba.extend(self.doc.composite_display_pixel(index, true));
            }
        }
        (rgba, size)
    }

    /// The Export image window, while it is open.
    pub(crate) fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export_window.open {
            return;
        }
        if self.export_window.source.is_none() {
            self.export_window.source = Some(self.export_preview_source());
        }
        let mut open = true;
        let mut settings = self.export_settings;
        let mut export = false;
        let mut cancel = false;
        egui::Window::new("Export image")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::ComboBox::from_label("Format")
                    .selected_text(settings.format.name())
                    .show_ui(ui, |ui| {
                        for format in Format::ALL {
                            ui.selectable_value(&mut settings.format, format, format.name());
                        }
                    });
                ui.label(settings.format.description());
                ui.add_space(4.0);
                if settings.format.keeps_transparency() {
                    ui.checkbox(&mut settings.transparent, "Keep transparency")
                        .on_hover_text("Leave see-through areas see-through");
                }
                if !settings.keeps_transparency() {
                    ui.horizontal(|ui| {
                        ui.label("Background");
                        let mut white = settings.background.is_none();
                        if ui.radio_value(&mut white, true, "White").clicked() {
                            settings.background = None;
                        }
                        if ui.radio_value(&mut white, false, "Color").clicked()
                            && settings.background.is_none()
                        {
                            settings.background = Some(self.color);
                        }
                        if let Some(color) = &mut settings.background {
                            ui.color_edit_button_srgb(color);
                        }
                    });
                    ui.weak(settings.flattening());
                }
                if settings.format == Format::Jpeg {
                    ui.add(egui::Slider::new(&mut settings.quality, 1..=100).text("Quality"));
                }
                ui.add_space(4.0);
                self.export_preview_ui(ui, &settings);
                ui.weak(format!(
                    "{} × {} px at {} DPI",
                    self.doc.width, self.doc.height, self.doc.dpi
                ));
                crate::dialogs::dialog_buttons(ui, |ui| {
                    export = ui
                        .add(crate::workspace::primary_button(ui, "Export…"))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        self.export_settings = settings.sanitized();
        if export {
            self.export_window.open = false;
            self.choose_export_file();
        } else if cancel || !open {
            self.export_window.open = false;
        }
    }

    /// The preview of the picture as the settings write it.
    fn export_preview_ui(&mut self, ui: &mut egui::Ui, settings: &ExportSettings) {
        let Some((rgba, size)) = &self.export_window.source else {
            return;
        };
        let stale = self
            .export_window
            .preview
            .as_ref()
            .is_none_or(|(shown, _)| shown != settings);
        if stale {
            let pixels = converted(rgba, *size, settings);
            let image = egui::ColorImage::from_rgba_unmultiplied(*size, &pixels);
            let texture =
                ui.ctx()
                    .load_texture("export preview", image, egui::TextureOptions::LINEAR);
            self.export_window.preview = Some((*settings, texture));
        }
        if let Some((_, texture)) = &self.export_window.preview {
            let shown = texture.size_vec2();
            let (rect, _) = ui.allocate_exact_size(shown, egui::Sense::hover());
            // A checkerboard behind what stays see-through.
            let tile = 8.0;
            let painter = ui.painter_at(rect);
            for row in 0..(shown.y / tile).ceil() as usize {
                for column in 0..(shown.x / tile).ceil() as usize {
                    let light = (row + column) % 2 == 0;
                    let min = rect.min + egui::vec2(column as f32 * tile, row as f32 * tile);
                    painter.rect_filled(
                        egui::Rect::from_min_size(min, egui::vec2(tile, tile)),
                        0.0,
                        egui::Color32::from_gray(if light { 235 } else { 200 }),
                    );
                }
            }
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    /// Ask where to write, then export there with the current settings.
    fn choose_export_file(&mut self) {
        if self.adjustment.is_some() {
            self.status = "Apply or cancel the adjustment before exporting".into();
            return;
        }
        if self.export_job.is_some() || self.queued_export.is_some() {
            self.status = "An image export is already running".into();
            return;
        }
        let settings = self.export_settings;
        let format = settings.format;
        let name = self
            .path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map_or("Painting".into(), |s| s.to_string_lossy().into_owned());
        let extensions: &[&str] = if format == Format::Jpeg {
            &["jpg", "jpeg"]
        } else {
            &[format.extension()]
        };
        let dialog = self.dialog_in(
            crate::locations::Kind::Exports,
            rfd::AsyncFileDialog::new()
                .add_filter(format.name(), extensions)
                .set_file_name(format!("{name}.{}", format.extension())),
        );
        self.ask_save_file(dialog, move |app, path| app.export_image(path, settings));
    }

    /// Export the picture to `path` with `settings`, and remember it for
    /// Export again. A file name for another of the formats uses that one.
    pub(crate) fn export_image(&mut self, path: PathBuf, mut settings: ExportSettings) {
        match Format::of_path(&path) {
            Some(format) => settings.format = format,
            None => {
                self.status = "Choose PNG, JPEG, WebP, BMP or GIF for image export".into();
                return;
            }
        }
        if self.start_image_export(path.clone(), settings) {
            self.last_export = Some(LastExport { path, settings });
        }
    }

    /// File > Export again: the project's last image export, once more.
    pub(crate) fn export_again(&mut self) {
        match self.last_export.clone() {
            Some(last) => self.export_image(last.path, last.settings),
            None => self.open_export_window(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattening_puts_see_through_pixels_on_the_background() {
        let mut rgba = vec![200, 100, 0, 255, 0, 0, 0, 0, 255, 255, 255, 128];
        flatten(&mut rgba, [0, 0, 255]);
        assert_eq!(rgba[..4], [200, 100, 0, 255], "opaque stays");
        assert_eq!(rgba[4..8], [0, 0, 255, 255], "clear becomes the background");
        assert_eq!(rgba[8..], [128, 128, 255, 255], "half and half");
    }

    #[test]
    fn previews_show_what_each_format_does() {
        // A see-through left half and a smooth red ramp on the right.
        let size = [64, 8];
        let mut rgba = Vec::new();
        for _ in 0..size[1] {
            for x in 0..size[0] {
                let alpha = if x < 32 { 0 } else { 255 };
                rgba.extend([(x * 4) as u8, 0, 0, alpha]);
            }
        }
        let settings = |format, transparent| ExportSettings {
            format,
            transparent,
            ..Default::default()
        };
        let png = converted(&rgba, size, &settings(Format::Png, true));
        assert_eq!(png, rgba, "PNG with transparency is the picture itself");
        let flat = converted(&rgba, size, &settings(Format::Png, false));
        assert_eq!(flat[..4], [255, 255, 255, 255], "white paper");
        let mut blue = settings(Format::Bmp, true);
        blue.background = Some([0, 0, 255]);
        let bmp = converted(&rgba, size, &blue);
        assert_eq!(bmp[..4], [0, 0, 255, 255], "BMP can't keep transparency");
        let mut rough = settings(Format::Jpeg, false);
        rough.quality = 5;
        let jpeg = converted(&rgba, size, &rough);
        assert_eq!(jpeg.len(), rgba.len());
        assert_ne!(jpeg, flat, "low quality JPEG changes the picture");
    }

    #[test]
    fn saved_settings_are_made_safe() {
        let settings: ExportSettings =
            serde_json::from_str(r#"{"format":"jpeg","quality":0}"#).unwrap();
        assert_eq!(settings.sanitized().quality, 1);
        assert_eq!(settings.format, Format::Jpeg);
        let unknown: ExportSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(unknown, ExportSettings::default());
    }
}
