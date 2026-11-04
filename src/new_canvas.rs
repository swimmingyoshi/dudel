//! New-document form; submission always goes through the shared replacement guard.
use crate::{Dudel, document_actions::Action, egui};
use serde::{Deserialize, Serialize};

/// Most saved presets and remembered recent sizes.
const MAX_PRESETS: usize = 12;
const MAX_RECENT: usize = 5;

const BUILT_IN: [(&str, usize, usize, Option<f64>); 7] = [
    ("Sketch", 1024, 1024, None),
    ("Square", 2048, 2048, None),
    ("Portrait", 1600, 2000, None),
    ("Landscape", 2000, 1600, None),
    ("Full HD", 1920, 1080, None),
    ("A4 · 300 dpi", 2480, 3508, Some(300.0)),
    ("4K", 3840, 2160, None),
];

/// A canvas size the artist saved or created recently.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Size {
    #[serde(default)]
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub dpi: f64,
}
impl Size {
    fn valid(&self) -> bool {
        (1..=crate::document::MAX_SIDE).contains(&self.width)
            && (1..=crate::document::MAX_SIDE).contains(&self.height)
            && self.width * self.height <= crate::document::MAX_PIXELS
            && crate::document::validate_dpi(self.dpi).is_ok()
            && self.name.len() <= 40
            && !self.name.chars().any(char::is_control)
    }
    fn label(&self) -> String {
        format!("{} × {}", self.width, self.height)
    }
}

/// New canvas choices that carry between sessions.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Prefs {
    #[serde(default)]
    pub presets: Vec<Size>,
    #[serde(default)]
    pub recent: Vec<Size>,
}
impl Prefs {
    /// Drops entries an edited or older settings file could make invalid.
    pub(crate) fn sanitized(mut self) -> Self {
        self.presets
            .retain(|p| p.valid() && !p.name.trim().is_empty());
        self.presets.truncate(MAX_PRESETS);
        self.recent.retain(Size::valid);
        self.recent.truncate(MAX_RECENT);
        self
    }
    fn remember(&mut self, width: usize, height: usize, dpi: f64) {
        self.recent
            .retain(|r| (r.width, r.height, r.dpi) != (width, height, dpi));
        self.recent.insert(
            0,
            Size {
                name: String::new(),
                width,
                height,
                dpi,
            },
        );
        self.recent.truncate(MAX_RECENT);
    }
}

pub(crate) struct Form {
    pub width: String,
    pub height: String,
    pub dpi: String,
    pub feedback: Option<String>,
    focus_width: bool,
    pub prefs: Prefs,
    /// The name being typed for a new preset, while saving one.
    saving: Option<String>,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            width: "2048".into(),
            height: "2048".into(),
            dpi: "300".into(),
            feedback: None,
            focus_width: true,
            prefs: Prefs::default(),
            saving: None,
        }
    }
}
impl Form {
    pub(crate) fn resolution(&self) -> Result<f64, String> {
        let dpi = self
            .dpi
            .trim()
            .parse::<f64>()
            .map_err(|_| "Enter a resolution from 1 to 12,000 DPI".to_owned())?;
        crate::document::validate_dpi(dpi)?;
        Ok(dpi)
    }
    pub(crate) fn dimensions(&self) -> Result<[usize; 2], &'static str> {
        let parse = |value: &str| {
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Enter a whole number from 1 to 16384 for each dimension.");
            }
            value
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=crate::document::MAX_SIDE).contains(n))
                .ok_or("Each dimension must be from 1 to 16384 pixels.")
        };
        let (width, height) = (parse(&self.width)?, parse(&self.height)?);
        if width * height > crate::document::MAX_PIXELS {
            return Err("Use no more than 100 megapixels in total (for example 10000 × 10000).");
        }
        Ok([width, height])
    }
    fn is(&self, size: &Size) -> bool {
        self.dimensions() == Ok([size.width, size.height]) && self.resolution() == Ok(size.dpi)
    }
    fn set(&mut self, width: usize, height: usize, dpi: Option<f64>) {
        self.width = width.to_string();
        self.height = height.to_string();
        if let Some(dpi) = dpi {
            self.dpi = dpi.to_string();
        }
        self.feedback = None;
    }
    /// Save the current fields under `name`, replacing a preset of that name.
    fn save_preset(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Name the preset".into());
        }
        let [width, height] = self.dimensions()?;
        let size = Size {
            name: name.to_owned(),
            width,
            height,
            dpi: self.resolution()?,
        };
        if !size.valid() {
            return Err("Use a shorter name".into());
        }
        let presets = &mut self.prefs.presets;
        if let Some(existing) = presets.iter_mut().find(|p| p.name == size.name) {
            *existing = size;
        } else if presets.len() >= MAX_PRESETS {
            return Err(format!("At most {MAX_PRESETS} presets; remove one first"));
        } else {
            presets.push(size);
        }
        Ok(())
    }
}

/// Starts naming a preset; true when clicked.
fn save_preset_button(ui: &mut egui::Ui, form: &mut Form) -> bool {
    if form.saving.is_some() {
        return false;
    }
    let clicked = ui
        .add_enabled(
            form.dimensions().is_ok() && form.resolution().is_ok(),
            egui::Button::new("Save as preset…"),
        )
        .on_hover_text("Keep this size and DPI for next time")
        .clicked();
    if clicked {
        form.saving = Some(String::new());
    }
    clicked
}

/// A small heading over each group of choices.
fn group(ui: &mut egui::Ui, title: &str) {
    ui.add_space(2.0);
    ui.weak(title);
}

impl Dudel {
    pub(crate) fn open_new_canvas(&mut self) {
        if self.unsaved_action.is_some() || self.pending_action.is_some() {
            return;
        }
        self.new_canvas = true;
        self.new_canvas_form.feedback = None;
        self.new_canvas_form.focus_width = true;
        self.new_canvas_form.saving = None;
    }
    pub(crate) fn new_canvas_ui(&mut self, ctx: &egui::Context) {
        if !self.new_canvas || self.unsaved_action.is_some() || self.pending_action.is_some() {
            return;
        }
        let enter=ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{key:egui::Key::Enter,pressed:true,repeat:false,modifiers,..} if modifiers.is_none())));
        let (mut create, mut cancel) = (false, false);
        let mut changed_option = false;
        let response = egui::Modal::new(egui::Id::new("new canvas form")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.0).clamp(240.0, 440.0));
            ui.heading("New canvas");
            ui.label("Choose a starting size, then make it yours.");
            egui::ScrollArea::vertical()
                .id_salt("new canvas fields")
                .max_height((ctx.content_rect().height() - 144.0).max(80.0))
                .show(ui, |ui| {
                    let form = &mut self.new_canvas_form;
                    group(ui, "Presets");
                    ui.horizontal_wrapped(|ui| {
                        for (label, width, height, dpi) in BUILT_IN {
                            let selected = form.dimensions() == Ok([width, height])
                                && dpi.is_none_or(|dpi| form.resolution() == Ok(dpi));
                            if ui.selectable_label(selected, label)
                                .on_hover_text(format!("{width} × {height} px"))
                                .clicked()
                            {
                                changed_option = true;
                                form.set(width, height, dpi);
                            }
                        }
                        // Here rather than beside the fields, so Tab runs from
                        // Width to Height to DPI without stopping on it.
                        if ui.small_button("Swap W/H").on_hover_text("Switch between landscape and portrait").clicked() {
                            changed_option = true;
                            std::mem::swap(&mut form.width, &mut form.height);
                        }
                        // Without saved presets the button needs no row of its own.
                        if form.prefs.presets.is_empty() {
                            changed_option |= save_preset_button(ui, form);
                        }
                    });
                    let mut remove = None;
                    if !form.prefs.presets.is_empty() {
                    group(ui, "Your presets");
                    ui.horizontal_wrapped(|ui| {
                        for (index, preset) in form.prefs.presets.clone().into_iter().enumerate() {
                            let response = ui
                                .push_id(("preset", index), |ui| ui.selectable_label(form.is(&preset), &preset.name))
                                .inner
                                .on_hover_text(format!("{} px · {} DPI · right-click to remove", preset.label(), preset.dpi));
                            if response.clicked() {
                                changed_option = true;
                                form.set(preset.width, preset.height, Some(preset.dpi));
                            }
                            response.context_menu(|ui| {
                                if ui.button("Remove preset").clicked() {
                                    remove = Some(index);
                                    ui.close();
                                }
                            });
                        }
                        changed_option |= save_preset_button(ui, form);
                    });
                    }
                    if let Some(index) = remove {
                        form.prefs.presets.remove(index);
                    }
                    if let Some(mut name) = form.saving.take() {
                        let mut keep = true;
                        ui.horizontal(|ui| {
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut name)
                                    .id(egui::Id::new("new canvas preset name"))
                                    .hint_text("Preset name")
                                    .char_limit(40)
                                    .desired_width(160.0),
                            );
                            if !edit.has_focus() && name.is_empty() {
                                edit.request_focus();
                            }
                            let submit = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if ui.button("Save").clicked() || submit {
                                changed_option = true;
                                match form.save_preset(&name) {
                                    Ok(()) => keep = false,
                                    Err(error) => form.feedback = Some(error),
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                changed_option = true;
                                keep = false;
                            }
                        });
                        if keep {
                            form.saving = Some(name);
                        }
                    }
                    if !form.prefs.recent.is_empty() {
                        group(ui, "Recent");
                        ui.horizontal_wrapped(|ui| {
                            for (index, recent) in form.prefs.recent.clone().into_iter().enumerate() {
                                if ui
                                    .push_id(("recent", index), |ui| ui.selectable_label(form.is(&recent), recent.label()))
                                    .inner
                                    .on_hover_text(format!("{} DPI", recent.dpi))
                                    .clicked()
                                {
                                    changed_option = true;
                                    form.set(recent.width, recent.height, Some(recent.dpi));
                                }
                            }
                        });
                    }
                    ui.add_space(6.0);
                    for (label, id, height) in [
                        ("Width (px)", "new canvas width", false),
                        ("Height (px)", "new canvas height", true),
                    ] {
                        ui.horizontal(|ui| {
                            ui.add_sized([88.0, 32.0], egui::Label::new(label));
                            let value = if height { &mut form.height } else { &mut form.width };
                            let edit = ui.add(
                                egui::TextEdit::singleline(value)
                                    .id(egui::Id::new(id))
                                    .char_limit(16)
                                    .desired_width(120.0),
                            );
                            if !height && form.focus_width {
                                edit.request_focus();
                                form.focus_width = false;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.add_sized([88.0,32.0], egui::Label::new("DPI"));
                        ui.add(egui::TextEdit::singleline(&mut form.dpi).char_limit(16).desired_width(120.0));
                        if let (Ok([width, height]), Ok(dpi)) = (form.dimensions(), form.resolution()) {
                            let (w, h) = (width as f64 / dpi, height as f64 / dpi);
                            // Not selectable, so Tab goes from DPI straight on.
                            ui.add(egui::Label::new(egui::RichText::new(format!("{:.1} × {:.1} cm", w * 2.54, h * 2.54)).weak()).selectable(false))
                                .on_hover_text(format!("Print size: {w:.2} × {h:.2} in"));
                        }
                    });
                    if let Err(error) = form.resolution() { ui.colored_label(ui.visuals().error_fg_color, error); }
                    if let Ok([width, height]) = form.dimensions() {
                        let (area, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 128.0), egui::Sense::hover());
                        let scale = ((area.width() - 84.0) / width as f32).min(88.0 / height as f32);
                        let canvas = egui::Rect::from_center_size(area.center() - egui::vec2(0.0, 8.0), egui::vec2(width as f32, height as f32) * scale);
                        let painter = ui.painter();
                        painter.rect_filled(area, 8.0, ui.visuals().faint_bg_color);
                        painter.rect_filled(canvas.translate(egui::vec2(3.0, 3.0)), 2.0, egui::Color32::from_black_alpha(45));
                        painter.rect_filled(canvas, 2.0, egui::Color32::WHITE);
                        painter.rect_stroke(canvas, 2.0, egui::Stroke::new(1.0_f32, ui.visuals().selection.stroke.color), egui::StrokeKind::Outside);
                        painter.text(egui::pos2(canvas.center().x, canvas.bottom() + 10.0), egui::Align2::CENTER_TOP, format!("{width} px"), egui::FontId::proportional(12.0), ui.visuals().text_color());
                        painter.text(egui::pos2(canvas.right() + 8.0, canvas.center().y), egui::Align2::LEFT_CENTER, format!("{height}\npx"), egui::FontId::proportional(12.0), ui.visuals().text_color());
                        ui.add_space(4.0);
                        ui.weak(format!("{:.1} MP · {:.0} MiB per fully painted layer", width as f64 * height as f64 / 1_000_000.0, (width * height * 4) as f64 / (1024.0 * 1024.0)))
                            .on_hover_text("Empty areas use sparse storage. Undo, masks, display textures and timelapse use additional memory.");
                    }
                    ui.weak("Up to 16,384 px per side and 100 megapixels in total.");
                    ui.add_space(4.0);
                    changed_option |= ui.checkbox(&mut self.timelapse.prefs.record_new, "Record a timelapse")
                        .on_hover_text("Saves a frame as the artwork changes, for speed-paint playback and GIF export in View > Timelapse. Frames go to the Timelapses folder (View > Settings > Storage & folders).")
                        .clicked();
                    if let Some(error) = &self.new_canvas_form.feedback {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
            let dimensions = self.new_canvas_form.dimensions();
            if let Err(error) = dimensions {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                create = ui
                    .add_enabled(
                        dimensions.is_ok() && self.new_canvas_form.resolution().is_ok() && !self.tab_open_pending(),
                        egui::Button::new("Create canvas"),
                    )
                    .clicked();
                let button = ui.button("Cancel").on_hover_text("Esc");
                cancel = button.clicked() || (enter && button.has_focus());
            });
        });
        if cancel || response.should_close() {
            self.cancel_pending_tab_change();
            self.new_canvas = false;
            return;
        }
        // Enter in the preset name field saves the preset, not the canvas.
        let naming = self.new_canvas_form.saving.is_some();
        if (create || (enter && !changed_option && !naming))
            && !self.tab_open_pending()
            && let Ok([width, height]) = self.new_canvas_form.dimensions()
            && let Ok(dpi) = self.new_canvas_form.resolution()
        {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            });
            self.new_canvas_form.prefs.remember(width, height, dpi);
            self.new_canvas_form.saving = None;
            // A new canvas gets its own tab unless the current one is untouched.
            self.open_in_tab(move |app| {
                app.new_canvas_form.focus_width = true;
                app.new_canvas_form.feedback = None;
                app.request_replace(Action::New(width, height, dpi));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolution_accepts_print_dpi_and_rejects_invalid_values() {
        assert_eq!(Form::default().resolution(), Ok(300.0));
        for value in ["0", "-1", "NaN", "inf", "12001", ""] {
            assert!(
                Form {
                    dpi: value.into(),
                    ..Default::default()
                }
                .resolution()
                .is_err()
            );
        }
        assert_eq!(
            Form {
                dpi: " 600.5 ".into(),
                ..Default::default()
            }
            .resolution(),
            Ok(600.5)
        );
    }
    #[test]
    fn dimensions_accept_supported_bounds_and_reject_invalid_input_without_overflow() {
        for (width, height) in [
            ("1", "4096"),
            ("2048", "2048"),
            ("4096", "1024"),
            (" 32 ", "16"),
        ] {
            let form = Form {
                width: width.into(),
                height: height.into(),
                ..Default::default()
            };
            assert!(form.dimensions().is_ok());
        }
        for width in [
            "",
            "0",
            "-1",
            "1.5",
            "+32",
            "16385",
            "1e3",
            "184467440737095516160",
            "\u{0661}",
        ] {
            let form = Form {
                width: width.into(),
                height: "1".into(),
                ..Default::default()
            };
            assert!(form.dimensions().is_err());
        }
        assert!(
            Form {
                width: "10001".into(),
                height: "10000".into(),
                ..Default::default()
            }
            .dimensions()
            .is_err()
        );
        // 4K artwork, CSP's 10000 x 10000 and a long webtoon strip fit.
        for (width, height) in [
            ("4096", "4096"),
            ("3840", "2160"),
            ("10000", "10000"),
            ("800", "16384"),
        ] {
            let form = Form {
                width: width.into(),
                height: height.into(),
                ..Default::default()
            };
            assert!(form.dimensions().is_ok(), "{width} x {height}");
        }
    }
    #[test]
    fn opening_shortcut_owns_other_keys_in_the_same_input_frame() {
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 8),
            ..Default::default()
        };
        app.new_layer();
        app.flush_render_queue();
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[23, 84, 163, 255]);
        let original = serde_json::to_value(&app.doc).unwrap();
        let tool = app.tool;
        let ctx = egui::Context::default();
        let event = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    event(egui::Key::N, egui::Modifiers::COMMAND),
                    event(egui::Key::Z, egui::Modifiers::COMMAND),
                    event(egui::Key::Delete, Default::default()),
                    event(egui::Key::E, Default::default()),
                ],
                ..Default::default()
            },
            |ctx| {
                app.update_tool_shift(ctx);
                app.shortcuts(ctx);
            },
        );
        assert!(app.new_canvas && app.tool == tool);
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), original);
    }
    #[test]
    fn dialog_reserves_tab_and_canvas_shortcuts_even_before_an_editor_has_focus() {
        let mut app = Dudel::default();
        let ctx = egui::Context::default();
        app.open_new_canvas();
        let before = serde_json::to_value(&app.doc).unwrap();
        let panels = app.show_panels;
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let mut raw = egui::RawInput {
            events: vec![key(egui::Key::Tab, Default::default())],
            ..Default::default()
        };
        eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
        assert_eq!(raw.events.len(), 1);
        assert_eq!(app.show_panels, panels);
        for event in [
            key(egui::Key::Delete, Default::default()),
            key(
                egui::Key::N,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            ),
            key(egui::Key::A, egui::Modifiers::COMMAND),
        ] {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![event],
                    ..Default::default()
                },
                |ctx| app.shortcuts(ctx),
            );
        }
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), before);
        assert!(!app.history.can_undo());
    }
}
