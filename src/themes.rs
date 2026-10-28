//! Portable UI palettes. Artwork and export colors are independent of the UI.
use crate::{Dudel, egui};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub panel: [u8; 3],
    pub window: [u8; 3],
    pub text: [u8; 3],
    pub accent: [u8; 3],
    /// The empty workspace outside the artwork, never part of exported pixels.
    #[serde(default = "default_canvas_background")]
    pub canvas_background: [u8; 3],
}
fn default_canvas_background() -> [u8; 3] {
    [28, 31, 39]
}

pub struct Editor {
    original: Theme,
    draft: Theme,
    // A live workspace preview must not restyle an active picker or text edit.
    style: Option<std::sync::Arc<egui::Style>>,
}
impl Default for Theme {
    fn default() -> Self {
        Self::presets().remove(0)
    }
}
impl Theme {
    pub fn presets() -> Vec<Self> {
        vec![
            Self {
                name: "Dudel Dark".into(),
                dark: true,
                panel: [35, 38, 46],
                window: [52, 58, 71],
                text: [222, 226, 234],
                accent: [57, 105, 153],
                canvas_background: default_canvas_background(),
            },
            Self {
                name: "Paper".into(),
                dark: false,
                panel: [208, 205, 198],
                window: [222, 218, 209],
                text: [43, 43, 42],
                accent: [143, 169, 189],
                canvas_background: default_canvas_background(),
            },
            Self {
                name: "Graphite".into(),
                dark: true,
                panel: [36, 36, 36],
                window: [52, 52, 52],
                text: [225, 225, 225],
                accent: [94, 101, 108],
                canvas_background: default_canvas_background(),
            },
        ]
    }
    /// Upgrade the exact shipped Paper palette without changing custom palettes.
    pub fn upgrade_builtin(&mut self) {
        let old = Self {
            name: "Paper".into(),
            dark: false,
            panel: [236, 235, 230],
            window: [250, 249, 245],
            text: [35, 38, 43],
            accent: [165, 200, 230],
            canvas_background: default_canvas_background(),
        };
        if *self == old {
            *self = Self::presets().remove(1);
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Theme needs a name of at most 80 bytes".into());
        }
        self.validate_colors()
    }
    fn validate_colors(&self) -> Result<(), String> {
        let luminance = |rgb: [u8; 3]| -> f32 {
            rgb.into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(c, w)| {
                    let v = c as f32 / 255.0;
                    w * if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                })
                .sum()
        };
        for bg in [self.panel, self.window, self.accent] {
            let (a, b) = (luminance(self.text), luminance(bg));
            if (a.max(b) + 0.05) / (a.min(b) + 0.05) < 3.0 {
                return Err(
                    "Theme text needs at least 3:1 contrast on panels, windows and selection"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

pub fn apply(ctx: &egui::Context, theme: &Theme) {
    let id = egui::Id::new("applied dudel theme");
    if ctx.data(|d| d.get_temp::<Theme>(id)).as_ref() == Some(theme) {
        return;
    }
    let numeric_color_space = ctx.style().visuals.numeric_color_space;
    crate::workspace::setup_theme(ctx);
    let mut style = (*ctx.style()).clone();
    if !theme.dark {
        style.visuals = egui::Visuals::light();
        // egui's near-white fields and buttons otherwise keep a muted palette
        // glaring. Derive all control surfaces from the chosen panel color.
        let surface = |offset: i16| {
            let [r, g, b] = theme
                .panel
                .map(|v| (i16::from(v) + offset).clamp(0, 255) as u8);
            egui::Color32::from_rgb(r, g, b)
        };
        style.visuals.extreme_bg_color = surface(-12);
        style.visuals.faint_bg_color = surface(-5);
        style.visuals.widgets.noninteractive.bg_fill = surface(0);
        style.visuals.widgets.inactive.bg_fill = surface(-8);
        style.visuals.widgets.inactive.weak_bg_fill = surface(-8);
        style.visuals.widgets.hovered.bg_fill = surface(8);
        style.visuals.widgets.hovered.weak_bg_fill = surface(8);
        style.visuals.widgets.active.bg_fill = surface(-18);
        style.visuals.widgets.active.weak_bg_fill = surface(-18);
        style.visuals.widgets.open.bg_fill = surface(4);
        style.visuals.widgets.open.weak_bg_fill = surface(4);
    }
    let rgb = |c: [u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
    style.visuals.panel_fill = rgb(theme.panel);
    style.visuals.window_fill = rgb(theme.window);
    style.visuals.selection.bg_fill = rgb(theme.accent);
    style.visuals.selection.stroke.color = rgb(theme.text);
    style.visuals.override_text_color = Some(rgb(theme.text));
    // Muted text (section titles, hints) is a solid blend toward the panel,
    // so it keeps contrast on light and dark themes alike.
    let muted: [u8; 3] = std::array::from_fn(|i| {
        ((theme.text[i] as u16 * 7 + theme.panel[i] as u16 * 3) / 10) as u8
    });
    style.visuals.weak_text_color = Some(rgb(muted));
    style.visuals.numeric_color_space = numeric_color_space;
    ctx.set_style(style);
    ctx.data_mut(|d| d.insert_temp(id, theme.clone()));
}

fn write_theme(path: &std::path::Path, theme: &Theme) -> Result<(), String> {
    theme.validate()?;
    let bytes = serde_json::to_vec_pretty(theme).map_err(|e| e.to_string())?;
    crate::storage::atomic_write(path, &bytes)
}

fn draft_preview(ui: &mut egui::Ui, theme: &Theme) {
    ui.separator();
    ui.label("Draft preview");
    let rgb = |c: [u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
    // Keep this sample visible even for invalid intermediate palettes.
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 62.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 4.0, rgb(theme.canvas_background));
    let inner = rect.shrink(8.0);
    let width = inner.width() / 3.0;
    for (index, (label, color)) in [
        ("Panel", theme.panel),
        ("Window", theme.window),
        ("Selection", theme.accent),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = egui::Rect::from_min_size(
            inner.min + egui::vec2(index as f32 * width, 0.0),
            egui::vec2(width - 3.0, inner.height()),
        );
        ui.painter().rect_filled(cell, 2.0, rgb(color));
        ui.painter_at(cell).text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.0),
            rgb(theme.text),
        );
    }
}

fn theme_form(ui: &mut egui::Ui, editor: &mut Editor) {
    ui.label("Customize your workspace. Changes preview live.");
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.add(
            egui::TextEdit::singleline(&mut editor.draft.name)
                .char_limit(80)
                .desired_width(240.0),
        );
    });
    egui::ComboBox::from_id_salt("theme editor preset")
        .selected_text("Start from a preset")
        .show_ui(ui, |ui| {
            for theme in Theme::presets() {
                if ui
                    .selectable_label(editor.draft == theme, &theme.name)
                    .clicked()
                {
                    editor.draft = theme;
                }
            }
        });
    ui.checkbox(&mut editor.draft.dark, "Dark controls");
    ui.separator();
    egui::Grid::new("theme palette")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            for (label, color) in [
                ("Panels", &mut editor.draft.panel),
                ("Windows and menus", &mut editor.draft.window),
                ("Text", &mut editor.draft.text),
                ("Selection", &mut editor.draft.accent),
                ("Outside canvas", &mut editor.draft.canvas_background),
            ] {
                ui.label(label);
                let response = ui
                    .push_id(("theme color", label), |ui| {
                        ui.color_edit_button_srgb(color)
                    })
                    .inner;
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(("theme picker", label)), response.rect)
                });
                let _ = response;
                ui.end_row();
            }
        });
    ui.small("Outside canvas changes the empty area around your artwork.");
}

impl Dudel {
    pub(crate) fn theme_for_preferences(&self) -> &Theme {
        self.theme_editor
            .as_ref()
            .map_or(&self.theme, |e| &e.original)
    }
    pub(crate) fn begin_theme_editor(&mut self) {
        if self.theme_editor.is_none() {
            self.theme_editor = Some(Editor {
                original: self.theme.clone(),
                draft: self.theme.clone(),
                style: None,
            });
        }
    }
    fn select_theme(&mut self, theme: Theme) {
        if let Some(editor) = &mut self.theme_editor {
            editor.draft = theme.clone();
        }
        self.theme = theme;
    }
    fn cancel_theme_edit(&mut self) {
        if let Some(editor) = self.theme_editor.take() {
            self.theme = editor.original;
        }
    }
    fn apply_theme_edit(&mut self) -> Result<(), String> {
        if let Some(editor) = &self.theme_editor {
            editor.draft.validate()?;
        }
        if let Some(editor) = self.theme_editor.take() {
            self.theme = editor.draft;
        }
        Ok(())
    }
    pub(crate) fn theme_editor_ui(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.theme_editor else {
            return;
        };
        let readable_theme = self.theme.clone();
        let previous_draft = editor.draft.clone();
        let workspace_style = ctx.style();
        let editor_style = editor
            .style
            .get_or_insert_with(|| workspace_style.clone())
            .clone();
        ctx.set_style(editor_style);
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        let mut save = false;
        egui::Window::new("Theme editor")
            .open(&mut open)
            .default_width(430.0)
            .resizable(false)
            .collapsible(false)
            .vscroll(false)
            .show(ctx, |ui| {
                draft_preview(ui, &editor.draft);
                // Only the form scrolls. Actions stay visible at small window sizes
                // and high display scaling, even when validation adds a message.
                egui::ScrollArea::vertical()
                    .id_salt("theme editor form")
                    .max_height((ctx.content_rect().height() - 320.0).clamp(60.0, 400.0))
                    .show(ui, |ui| {
                        theme_form(ui, editor);
                    });
                let validation = editor.draft.validate();
                ui.separator();
                ui.scope(|ui| {
                    // Keep the form and its popup anchors stationary when contrast
                    // feedback appears or disappears during a held drag.
                    ui.set_min_height(60.0);
                    if let Err(error) = &validation {
                        if editor.draft.validate_colors().is_err() {
                            ui.horizontal_wrapped(|ui| {
                                ui.small("Text contrast is too low.");
                                if ui.small_button("Restore readable colors").clicked() {
                                    editor.draft.panel = readable_theme.panel;
                                    editor.draft.window = readable_theme.window;
                                    editor.draft.text = readable_theme.text;
                                    editor.draft.accent = readable_theme.accent;
                                }
                            });
                            ui.small("Apply / Save need readable text. Color controls and draft preview still work.");
                        } else {
                            ui.label(error);
                            ui.small("Give your theme a name before applying or saving.");
                        }
                    } else {
                        ui.small("Workspace preview is live. Apply keeps it; Save theme writes a file.");
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        editor.draft = editor.original.clone();
                    }
                    if ui
                        .add_enabled(validation.is_ok(), egui::Button::new("Apply theme"))
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui
                        .add_enabled(validation.is_ok(), egui::Button::new("Save theme..."))
                        .clicked()
                    {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        let editor_style = ctx.style();
        editor.style = Some(editor_style.clone());
        if workspace_style.visuals.numeric_color_space == editor_style.visuals.numeric_color_space {
            ctx.set_style(workspace_style);
        } else {
            let mut workspace_style = (*workspace_style).clone();
            workspace_style.visuals.numeric_color_space = editor_style.visuals.numeric_color_space;
            ctx.set_style(workspace_style);
        }
        if editor.draft != previous_draft {
            // The pinned sample is drawn before the form. Invalid drafts need
            // another frame too, even when the workspace palette stays unchanged.
            ctx.request_repaint();
        }
        if save {
            let draft = self.theme_editor.as_ref().unwrap().draft.clone();
            self.export_theme_file(&draft);
        }
        if !open || cancel {
            self.cancel_theme_edit();
            ctx.request_repaint();
        } else if apply {
            if let Err(error) = self.apply_theme_edit() {
                self.status = error;
            }
            ctx.request_repaint();
        } else if let Some(editor) = &self.theme_editor
            && editor.draft.validate_colors().is_ok()
            && self.theme != editor.draft
        {
            self.theme = editor.draft.clone();
            ctx.request_repaint();
        }
    }
    pub(crate) fn theme_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Edit theme...").clicked() {
            self.begin_theme_editor();
            ui.close();
        }
        ui.separator();
        for theme in Theme::presets() {
            if ui
                .selectable_label(self.theme == theme, &theme.name)
                .clicked()
            {
                self.select_theme(theme);
                ui.ctx().request_repaint();
            }
        }
        let saved = self.saved_themes();
        if !saved.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("Themes folder").small().weak());
            for theme in saved {
                if ui
                    .selectable_label(self.theme == theme, &theme.name)
                    .clicked()
                {
                    self.select_theme(theme);
                    ui.ctx().request_repaint();
                }
            }
        }
        ui.separator();
        if ui.button("Import theme...").clicked() {
            let dialog = self.dialog_in(
                crate::locations::Kind::Themes,
                rfd::AsyncFileDialog::new().add_filter("Dudel theme", &["dudeltheme"]),
            );
            self.ask_open_file(dialog, Self::import_theme_file);
        }
        if ui.button("Export theme...").clicked() {
            let theme = self.theme.clone();
            self.export_theme_file(&theme);
        }
    }
    pub(crate) fn import_theme_file(&mut self, path: std::path::PathBuf) {
        let result = (|| {
            let source = crate::read_bounded(&path, 4096)?;
            let theme: Theme = serde_json::from_str(&source).map_err(|e| e.to_string())?;
            theme.validate()?;
            Ok::<_, String>(theme)
        })();
        match result {
            Ok(theme) => self.select_theme(theme),
            Err(e) => self.status = format!("Theme: {e}"),
        }
    }
    /// Valid themes in the Themes folder, rescanned at most every two seconds.
    fn saved_themes(&mut self) -> Vec<Theme> {
        let fresh = self
            .theme_library
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() < std::time::Duration::from_secs(2));
        if !fresh {
            let folder = self.locations.get(crate::locations::Kind::Themes);
            self.theme_library = Some((std::time::Instant::now(), scan_themes(&folder)));
        }
        self.theme_library.as_ref().unwrap().1.clone()
    }
    fn export_theme_file(&mut self, theme: &Theme) {
        let name: String = theme
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let name = if name.trim().is_empty() {
            "my-theme".into()
        } else {
            name
        };
        let dialog = self.dialog_in(
            crate::locations::Kind::Themes,
            rfd::AsyncFileDialog::new()
                .add_filter("Dudel theme", &["dudeltheme"])
                .set_file_name(format!("{}.dudeltheme", name.trim())),
        );
        let theme = theme.clone();
        self.ask_save_file(dialog, move |app, path| {
            app.status = match write_theme(&path, &theme) {
                Ok(()) => "Theme saved".into(),
                Err(e) => format!("Theme: {e}"),
            };
        });
    }
}

/// Up to 64 valid `.dudeltheme` files in `folder`, sorted by name; invalid
/// files are skipped rather than reported in a menu.
pub(crate) fn scan_themes(folder: &std::path::Path) -> Vec<Theme> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut themes: Vec<Theme> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dudeltheme"))
        })
        .take(256)
        .filter_map(|path| {
            let source = crate::read_bounded(&path, 4096).ok()?;
            let theme: Theme = serde_json::from_str(&source).ok()?;
            theme.validate().ok()?;
            Some(theme)
        })
        .collect();
    themes.sort_by_cached_key(|theme| theme.name.to_lowercase());
    themes.truncate(64);
    themes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muted_text_stays_legible_on_every_preset() {
        let luminance = |c: egui::Color32| {
            let channel = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        for theme in Theme::presets() {
            let ctx = egui::Context::default();
            apply(&ctx, &theme);
            let visuals = ctx.style().visuals.clone();
            let (a, b) = (
                luminance(visuals.weak_text_color()),
                luminance(visuals.panel_fill),
            );
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(
                ratio >= 3.0,
                "{}: muted text contrast {ratio:.2}",
                theme.name
            );
        }
    }
    #[test]
    fn live_theme_changes_preserve_picker_color_space_and_editor_style() {
        let ctx = egui::Context::default();
        let mut app = Dudel::default();
        apply(&ctx, &app.theme);
        ctx.all_styles_mut(|s| {
            s.visuals.numeric_color_space = egui::style::NumericColorSpace::Linear
        });
        app.begin_theme_editor();
        let _ = ctx.run(Default::default(), |ctx| app.theme_editor_ui(ctx));
        let editor_style = app.theme_editor.as_ref().unwrap().style.clone().unwrap();
        app.theme_editor.as_mut().unwrap().draft = Theme::presets().remove(1);
        let _ = ctx.run(Default::default(), |ctx| app.theme_editor_ui(ctx));
        apply(&ctx, &app.theme);
        assert_eq!(
            ctx.style().visuals.numeric_color_space,
            egui::style::NumericColorSpace::Linear
        );
        let _ = ctx.run(Default::default(), |ctx| app.theme_editor_ui(ctx));
        let current = app.theme_editor.as_ref().unwrap().style.as_ref().unwrap();
        assert_eq!(
            current.visuals.override_text_color,
            editor_style.visuals.override_text_color
        );
        assert_eq!(
            current.visuals.window_fill,
            editor_style.visuals.window_fill
        );
        assert_eq!(
            ctx.style().visuals.window_fill,
            egui::Color32::from_rgb(222, 218, 209),
            "editor styling must not leak into the workspace"
        );
    }
    #[test]
    fn text_picker_then_other_picker_remain_interactive_across_preview_validation_changes() {
        struct Harness {
            app: Dudel,
            ctx: egui::Context,
            output: egui::FullOutput,
            time: f64,
        }
        impl Harness {
            fn frame(&mut self, events: Vec<egui::Event>) {
                self.time += 0.05;
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                };
                eframe::App::raw_input_hook(&mut self.app, &self.ctx, &mut raw);
                self.output = self.ctx.run(raw, |ctx| {
                    eframe::App::update(&mut self.app, ctx, &mut eframe::Frame::_new_kittest());
                });
            }
            fn settle(&mut self) {
                for _ in 0..4 {
                    self.frame(vec![]);
                }
            }
            fn click(&mut self, pos: egui::Pos2) {
                for pressed in [true, false] {
                    self.frame(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            pressed,
                            button: egui::PointerButton::Primary,
                            modifiers: Default::default(),
                        },
                    ]);
                }
                self.settle();
            }
            fn open(&mut self, name: &str) {
                let rect = self
                    .ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("theme picker", name))))
                    .unwrap();
                self.click(rect.center());
            }
            fn square(&self) -> egui::Rect {
                self.output
                    .shapes
                    .iter()
                    .find_map(|s| {
                        if let egui::Shape::Mesh(mesh) = &s.shape {
                            let rect = mesh.calc_bounds();
                            if (rect.width() - 275.0).abs() < 1.0
                                && (rect.height() - 275.0).abs() < 1.0
                            {
                                return Some(rect);
                            }
                        }
                        None
                    })
                    .expect("open picker saturation/value square")
            }
            fn pick(&mut self, x: f32, y: f32) {
                let rect = self.square();
                self.click(rect.min + rect.size() * egui::vec2(x, y));
            }
            fn drag(&mut self, text: bool) {
                let color = |app: &Dudel| {
                    let draft = &app.theme_editor.as_ref().unwrap().draft;
                    if text { draft.text } else { draft.panel }
                };
                let rect = self.square();
                let pos = rect.min + rect.size() * egui::vec2(0.1, 0.1);
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        pressed: true,
                        button: egui::PointerButton::Primary,
                        modifiers: Default::default(),
                    },
                ]);
                let first = color(&self.app);
                let pos = rect.min + rect.size() * egui::vec2(0.8, 0.7);
                self.frame(vec![egui::Event::PointerMoved(pos)]);
                assert_ne!(color(&self.app), first, "held picker drag stopped updating");
                let second = color(&self.app);
                let pos = rect.min + rect.size() * egui::vec2(0.4, 0.4);
                self.frame(vec![egui::Event::PointerMoved(pos)]);
                assert_ne!(
                    color(&self.app),
                    second,
                    "continued picker drag stopped updating"
                );
                self.frame(vec![egui::Event::PointerButton {
                    pos,
                    pressed: false,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                }]);
                self.settle();
            }
            fn escape(&mut self) {
                self.frame(vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }]);
                self.frame(vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: Default::default(),
                }]);
                self.settle();
                assert!(
                    !egui::Popup::is_any_open(&self.ctx),
                    "picker retained popup input capture"
                );
            }
        }
        for theme in Theme::presets() {
            let mut app = Dudel {
                theme,
                ..Default::default()
            };
            app.begin_theme_editor();
            let mut h = Harness {
                app,
                ctx: Default::default(),
                output: Default::default(),
                time: 0.0,
            };
            h.settle();
            h.open("Text");
            h.drag(true);
            h.pick(0.5, 0.5);
            let first = h.app.theme_editor.as_ref().unwrap().draft.text;
            h.pick(0.2, 0.8);
            assert_ne!(
                h.app.theme_editor.as_ref().unwrap().draft.text,
                first,
                "second text change stuck"
            );
            h.escape();
            h.open("Panels");
            h.drag(false);
            h.pick(0.3, 0.4);
            let first = h.app.theme_editor.as_ref().unwrap().draft.panel;
            h.pick(0.6, 0.6);
            assert_ne!(
                h.app.theme_editor.as_ref().unwrap().draft.panel,
                first,
                "second panel change stuck after editing text"
            );
            h.escape();
            for _ in 0..3 {
                h.open("Text");
                h.drag(true);
                h.escape();
                h.open("Panels");
                h.drag(false);
                h.escape();
            }
            h.app.cancel_theme_edit();
            h.settle();
            let color_button = h
                .output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "Color" => {
                        Some(t.galley.rect.translate(t.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("drawing color button");
            h.click(color_button);
            assert!(egui::Popup::is_any_open(&h.ctx));
            let wheel_square = h
                .output
                .shapes
                .iter()
                // The inspector may also paint a wheel underneath the popup.
                .rev()
                .find_map(|s| match &s.shape {
                    egui::Shape::Mesh(m) if m.vertices.len() == 17 * 17 => Some(m.calc_bounds()),
                    _ => None,
                })
                .expect("drawing color wheel square");
            h.click(wheel_square.min + wheel_square.size() * egui::vec2(0.3, 0.3));
            let first = h.app.color;
            h.click(wheel_square.min + wheel_square.size() * egui::vec2(0.7, 0.7));
            assert_ne!(
                h.app.color,
                first,
                "drawing color wheel stuck after theme editing: rect={wheel_square:?} popup={} layer={:?} hsv={:?}",
                egui::Popup::is_any_open(&h.ctx),
                h.ctx.layer_id_at(wheel_square.center()),
                h.app.hsv
            );
            h.escape();
            assert!(!h.app.modified);
        }
    }
    #[test]
    fn editor_footer_and_draft_preview_remain_visible_with_invalid_colors_at_small_sizes() {
        fn visible(shapes: &[egui::epaint::ClippedShape], name: &str, screen: egui::Rect) -> bool {
            shapes.iter().any(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == name => {
                    let rect = t.galley.rect.translate(t.pos.to_vec2());
                    screen.contains_rect(rect) && s.clip_rect.expand(0.5).contains_rect(rect)
                }
                _ => false,
            })
        }
        for size in [[960.0, 640.0], [640.0, 400.0]] {
            let mut app = Dudel::default();
            let original = app.theme.clone();
            app.begin_theme_editor();
            let draft = &mut app.theme_editor.as_mut().unwrap().draft;
            draft.text = draft.panel;
            draft.canvas_background = [180, 90, 80];
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
            let mut output = None;
            for _ in 0..4 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ctx| {
                        apply(ctx, &app.theme);
                        app.theme_editor_ui(ctx);
                    },
                ));
            }
            let output = output.unwrap();
            for label in [
                "Draft preview",
                "Panel",
                "Window",
                "Selection",
                "Apply theme",
                "Save theme...",
                "Reset",
                "Cancel",
            ] {
                assert!(
                    visible(&output.shapes, label, screen),
                    "{label} is clipped at {size:?}"
                );
            }
            assert_eq!(
                app.theme, original,
                "unsafe workspace preview must stay readable"
            );
            app.theme_editor.as_mut().unwrap().draft.text = original.text;
            app.theme_editor.as_mut().unwrap().draft.name.clear();
            let _ = ctx.run(Default::default(), |ctx| app.theme_editor_ui(ctx));
            assert_eq!(
                app.theme.canvas_background,
                [180, 90, 80],
                "editing the name must not pause color preview"
            );
            app.cancel_theme_edit();
            assert_eq!(app.theme, original);
        }
    }

    #[test]
    fn saving_a_theme_uses_the_draft_and_preserves_existing_files_on_validation_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("study.dudeltheme");
        std::fs::write(dir.path().join("broken.dudeltheme"), b"{").unwrap();
        let mut app = Dudel::default();
        app.begin_theme_editor();
        let draft = &mut app.theme_editor.as_mut().unwrap().draft;
        draft.canvas_background = [100, 90, 80];
        write_theme(&path, draft).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(serde_json::from_slice::<Theme>(&bytes).unwrap(), *draft);
        // Saved files show up in the theme menu; damaged ones are skipped.
        assert_eq!(scan_themes(dir.path()), vec![draft.clone()]);
        draft.text = draft.panel;
        assert!(write_theme(&path, draft).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(
            app.theme_editor.is_some(),
            "saving a file must leave the editor open"
        );
    }
    #[test]
    fn legacy_palettes_load_and_canvas_color_survives_theme_exchange() {
        let mut source = serde_json::to_value(Theme::default()).unwrap();
        source.as_object_mut().unwrap().remove("canvas_background");
        let mut theme: Theme = serde_json::from_value(source).unwrap();
        assert_eq!(theme.canvas_background, [28, 31, 39]);
        theme.canvas_background = [122, 115, 108];
        let restored: Theme =
            serde_json::from_str(&serde_json::to_string(&theme).unwrap()).unwrap();
        assert_eq!(restored, theme);
    }
    #[test]
    fn editor_previews_apply_and_cancel_without_saving_drafts_or_editing_artwork() {
        let mut app = Dudel::default();
        let original = app.theme.clone();
        let artwork = app.doc.clone();
        let ctx = egui::Context::default();
        app.begin_theme_editor();
        app.theme_editor.as_mut().unwrap().draft.canvas_background = [100, 110, 120];
        let _ = ctx.run(Default::default(), |ctx| app.theme_editor_ui(ctx));
        assert_eq!(app.theme.canvas_background, [100, 110, 120]);
        let saved =
            crate::preferences::Preferences::decode(&app.encode_preferences().unwrap()).unwrap();
        assert_eq!(
            saved.theme, original,
            "an unconfirmed preview must not persist"
        );
        app.cancel_theme_edit();
        assert_eq!(app.theme, original);
        app.begin_theme_editor();
        let editor = app.theme_editor.as_mut().unwrap();
        editor.draft.canvas_background = [70, 80, 90];
        editor.draft.text = editor.draft.panel;
        assert!(app.apply_theme_edit().is_err());
        assert!(app.theme_editor.is_some());
        app.theme_editor.as_mut().unwrap().draft.text = original.text;
        app.apply_theme_edit().unwrap();
        let saved =
            crate::preferences::Preferences::decode(&app.encode_preferences().unwrap()).unwrap();
        assert_eq!(saved.theme.canvas_background, [70, 80, 90]);
        assert!(crate::document::same_snapshot(&artwork, &app.doc));
        assert!(!app.modified);
    }
    #[test]
    fn palettes_roundtrip_and_reject_unreadable_custom_colors() {
        for theme in Theme::presets() {
            theme.validate().unwrap();
            assert_eq!(
                serde_json::from_str::<Theme>(&serde_json::to_string(&theme).unwrap()).unwrap(),
                theme
            );
        }
        let mut theme = Theme::default();
        theme.text = theme.panel;
        assert!(theme.validate().is_err());
    }
}
