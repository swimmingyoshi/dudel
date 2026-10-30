//! New-document form; submission always goes through the shared replacement guard.
use crate::{Dudel, document_actions::Action, egui};

pub(crate) fn opening_shortcut(event: &egui::Event) -> bool {
    matches!(event,egui::Event::Key{key:egui::Key::N,pressed:true,repeat:false,modifiers,..} if modifiers.command && !modifiers.shift)
}

pub(crate) struct Form {
    pub width: String,
    pub height: String,
    pub dpi: String,
    pub feedback: Option<String>,
    focus_width: bool,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            width: "2048".into(),
            height: "2048".into(),
            dpi: "300".into(),
            feedback: None,
            focus_width: true,
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
}

impl Dudel {
    pub(crate) fn open_new_canvas(&mut self) {
        if self.unsaved_action.is_some() || self.pending_action.is_some() {
            return;
        }
        self.new_canvas = true;
        self.new_canvas_form.feedback = None;
        self.new_canvas_form.focus_width = true;
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
                    ui.horizontal_wrapped(|ui| {
                        for (label, width, height) in [
                            ("Sketch", 1024, 1024),
                            ("Square", 2048, 2048),
                            ("Portrait", 1600, 2000),
                            ("Landscape", 2000, 1600),
                            ("Full HD", 1920, 1080),
                            ("A4 · 300 dpi", 2480, 3508),
                            ("4K", 3840, 2160),
                        ] {
                            let selected = self.new_canvas_form.dimensions() == Ok([width, height])
                                && (!label.starts_with("A4") || self.new_canvas_form.resolution() == Ok(300.0));
                            if ui.selectable_label(selected, label)
                                .on_hover_text(format!("{width} × {height} px"))
                                .clicked()
                            {
                                changed_option = true;
                                self.new_canvas_form.width = width.to_string();
                                self.new_canvas_form.height = height.to_string();
                                if label.starts_with("A4") { self.new_canvas_form.dpi = "300".into(); }
                                self.new_canvas_form.feedback = None;
                            }
                        }
                        if ui.small_button("Swap W/H").on_hover_text("Switch between landscape and portrait").clicked() {
                            changed_option = true;
                            std::mem::swap(&mut self.new_canvas_form.width, &mut self.new_canvas_form.height);
                        }
                    });
                    ui.add_space(4.0);
                    for (label, id, value) in [
                        (
                            "Width (px)",
                            "new canvas width",
                            &mut self.new_canvas_form.width,
                        ),
                        (
                            "Height (px)",
                            "new canvas height",
                            &mut self.new_canvas_form.height,
                        ),
                    ] {
                        ui.horizontal(|ui| {
                            ui.add_sized([88.0, 32.0], egui::Label::new(label));
                            let edit = ui.add(
                                egui::TextEdit::singleline(value)
                                    .id(egui::Id::new(id))
                                    .char_limit(16)
                                    .desired_width(120.0),
                            );
                            if id == "new canvas width" && self.new_canvas_form.focus_width {
                                edit.request_focus();
                                self.new_canvas_form.focus_width = false;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.add_sized([88.0,32.0], egui::Label::new("DPI"));
                        ui.add(egui::TextEdit::singleline(&mut self.new_canvas_form.dpi).char_limit(16).desired_width(120.0));
                    });
                    if let Err(error) = self.new_canvas_form.resolution() { ui.colored_label(ui.visuals().error_fg_color, error); }
                    if let Ok([width, height]) = self.new_canvas_form.dimensions() {
                        if let Ok(dpi) = self.new_canvas_form.resolution() {
                            ui.small(format!("Print size: {:.2} \u{00d7} {:.2} cm", width as f64 / dpi * 2.54, height as f64 / dpi * 2.54));
                        }
                        let (area, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 148.0), egui::Sense::hover());
                        let scale = ((area.width() - 84.0) / width as f32).min(106.0 / height as f32);
                        let canvas = egui::Rect::from_center_size(area.center() - egui::vec2(0.0, 8.0), egui::vec2(width as f32, height as f32) * scale);
                        let painter = ui.painter();
                        painter.rect_filled(area, 8.0, ui.visuals().faint_bg_color);
                        painter.rect_filled(canvas.translate(egui::vec2(3.0, 3.0)), 2.0, egui::Color32::from_black_alpha(45));
                        painter.rect_filled(canvas, 2.0, egui::Color32::WHITE);
                        painter.rect_stroke(canvas, 2.0, egui::Stroke::new(1.0_f32, ui.visuals().selection.stroke.color), egui::StrokeKind::Outside);
                        painter.text(egui::pos2(canvas.center().x, canvas.bottom() + 10.0), egui::Align2::CENTER_TOP, format!("{width} px"), egui::FontId::proportional(12.0), ui.visuals().text_color());
                        painter.text(egui::pos2(canvas.right() + 8.0, canvas.center().y), egui::Align2::LEFT_CENTER, format!("{height}\npx"), egui::FontId::proportional(12.0), ui.visuals().text_color());
                        ui.add_space(4.0);
                        ui.small(format!("{:.1} MP · {:.0} MiB per fully painted layer", width as f64 * height as f64 / 1_000_000.0, (width * height * 4) as f64 / (1024.0 * 1024.0)))
                            .on_hover_text("Empty areas use sparse storage. Undo, masks, display textures and timelapse use additional memory.");
                    }
                    ui.small("Up to 16,384 px per side and 100 megapixels in total.");
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
        if (create || (enter && !changed_option))
            && !self.tab_open_pending()
            && let Ok([width, height]) = self.new_canvas_form.dimensions()
            && let Ok(dpi) = self.new_canvas_form.resolution()
        {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            });
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
