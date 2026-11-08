//! The Text tool's options bar: font family and style, size, color, alignment
//! and formatting. Changing any of them with text selected edits that text.
use crate::{
    Dudel, egui,
    layer_effects::{Effect, Kind, LayerEffect, Paint, Position},
    text_layer::{Format, Letters, TextLayer},
    text_render::{self, Align},
};
use std::collections::HashMap;

const PREVIEW_SIZE: [usize; 2] = [300, 28];
/// Font previews kept before they are all dropped and redrawn on demand.
const PREVIEW_LIMIT: usize = 256;
/// Font previews drawn per frame, so that scrolling the list stays smooth.
const PREVIEWS_PER_FRAME: usize = 2;
/// Wheel travel that moves one font along.
const FONT_WHEEL_STEP: f32 = 40.0;
/// Weights from this one up count as bold.
const BOLD: u16 = 600;
const LARGEST_OUTLINE: f64 = 100.0;

/// What new text looks like; remembered between launches.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Style {
    pub(crate) font: String,
    pub(crate) size: f64,
    pub(crate) align: Align,
    pub(crate) line_spacing: f64,
    pub(crate) vertical: bool,
    pub(crate) rightward: bool,
    pub(crate) letters: Letters,
    pub(crate) format: Format,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            font: String::new(),
            size: 48.0,
            align: Align::Left,
            line_spacing: 1.0,
            vertical: false,
            // Vertical text starts out left to right.
            rightward: true,
            letters: Default::default(),
            format: Default::default(),
        }
    }
}

impl Style {
    pub(crate) fn of(text: &TextLayer) -> Self {
        Self {
            font: text.font.clone(),
            size: text.size,
            align: text.align,
            line_spacing: text.line_spacing,
            vertical: text.vertical,
            rightward: text.rightward,
            letters: text.letters,
            format: text.format.clone(),
        }
    }
    pub(crate) fn apply(&self, text: &mut TextLayer) {
        text.font.clone_from(&self.font);
        text.size = self.size;
        text.align = self.align;
        text.line_spacing = self.line_spacing;
        text.format = self.format.clone();
        text.set_vertical(self.vertical);
        text.set_rightward(self.vertical && self.rightward);
        if self.vertical {
            text.set_letters(self.letters);
        }
    }
    /// Make a saved style safe to use.
    pub(crate) fn sanitize(&mut self) {
        let fallback = Self::default();
        if self.font.len() > 256 {
            self.font.clear();
        }
        if !(1.0..=8192.0).contains(&self.size) {
            self.size = fallback.size;
        }
        if !(0.5..=5.0).contains(&self.line_spacing) {
            self.line_spacing = fallback.line_spacing;
        }
        if !(-0.5..=2.0).contains(&self.format.tracking) {
            self.format.tracking = 0.0;
        }
    }
    /// The weight and slant this style shows, faux included.
    fn looks(&self) -> (bool, bool) {
        let face = text_render::family_of(&self.font).map(|(family, i)| &family.faces[i]);
        (
            self.format.faux_bold || face.is_some_and(|f| f.weight >= BOLD),
            self.format.faux_italic || face.is_some_and(|f| f.italic),
        )
    }
    /// Use the face of `family` closest to `bold` and `italic`, faking what it
    /// doesn't have.
    fn choose(&mut self, family: &text_render::Family, bold: bool, italic: bool) {
        let face = family.closest(if bold { 700 } else { 400 }, italic);
        self.font.clone_from(&face.name);
        self.format.faux_bold = bold && face.weight < BOLD;
        self.format.faux_italic = italic && !face.italic;
    }
    /// Turn bold or italic on or off, with a real face when the family has one.
    fn set_look(&mut self, bold: bool, italic: bool) {
        match text_render::family_of(&self.font) {
            Some((family, _)) => self.choose(family, bold, italic),
            None => {
                self.format.faux_bold = bold;
                self.format.faux_italic = italic;
            }
        }
    }
}

/// `label` drawn in the font `font`, or None when the font can't draw it.
pub(crate) fn preview_image(label: &str, font: &str) -> Option<egui::ColorImage> {
    let found = text_render::find_font(font)?;
    if !text_render::missing_characters(label, &found).is_empty() {
        return None;
    }
    let text = TextLayer {
        string: label.into(),
        font: font.into(),
        size: 18.0,
        color: [255, 255, 255],
        align: Align::Left,
        vertical: false,
        rightward: false,
        letters: Default::default(),
        format: Default::default(),
        wrap: None,
        box_height: None,
        line_spacing: 1.0,
        start: [2.0, 2.0],
        along: [1.0, 0.0],
        down: [0.0, 1.0],
        csp: None,
    };
    let pixels = text.render(PREVIEW_SIZE, &|| false).ok()?;
    let bytes: Vec<u8> = (0..PREVIEW_SIZE[0] * PREVIEW_SIZE[1])
        .flat_map(|index| pixels.rgba(index))
        .collect();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        PREVIEW_SIZE,
        &bytes,
    ))
}

/// Previews of the fonts shown so far.
#[derive(Default)]
pub(crate) struct FontList {
    search: String,
    previews: HashMap<String, Option<egui::TextureHandle>>,
    /// Wheel travel over the font button not yet spent on a step.
    wheel: f32,
    /// The first row the open list showed last.
    #[cfg(test)]
    pub(crate) first_row: usize,
}

impl Dudel {
    /// The text the options bar shows and changes: the open session, else a
    /// selected text layer, else what new text will look like.
    pub(crate) fn shown_text_style(&self) -> Style {
        if let Some(edit) = &self.text_edit {
            return Style::of(&edit.draft);
        }
        self.selected_text()
            .map_or_else(|| self.text_style.clone(), |(_, text)| Style::of(text))
    }

    pub(crate) fn selected_text(&self) -> Option<(usize, &TextLayer)> {
        let index = self.input_layer()?;
        Some((index, self.doc.layers.get(index)?.text.as_ref()?))
    }

    pub(crate) fn set_text_style(&mut self, style: Style) {
        self.text_style = style.clone();
        self.edit_open_text(|text| style.apply(text));
        self.text_style = style;
    }

    /// Change the open text, opening the selected text first.
    fn edit_open_text(&mut self, change: impl FnOnce(&mut TextLayer)) {
        if self.text_edit.is_none()
            && let Some((index, _)) = self.selected_text()
        {
            self.begin_text_edit(index);
        }
        if let Some(edit) = &mut self.text_edit {
            change(&mut edit.draft);
            edit.focus = true;
        }
        self.draw_text_draft();
    }

    /// The layer the outline controls change: the open text, else the
    /// selected text.
    fn outline_layer(&self) -> Option<usize> {
        match &self.text_edit {
            Some(edit) => self.doc.layers.iter().position(|l| l.id == edit.target),
            None => self.selected_text().map(|(index, _)| index),
        }
    }

    /// The outline's width and color: the text layer's Stroke effect.
    pub(crate) fn text_outline(&self) -> Option<(f64, [u8; 3])> {
        let layer = &self.doc.layers[self.outline_layer()?];
        layer
            .effects
            .iter()
            .find_map(|e| match (&e.effect.kind, &e.effect.paint) {
                (Kind::Stroke { size, .. }, Paint::Color(color)) if e.enabled => {
                    Some((*size, *color))
                }
                _ => None,
            })
    }

    /// Give the text an outline, change it or take it away. The outline is the
    /// layer's Stroke effect, so Layer Effects shows and edits it too. Changes
    /// join the editing session's undo step.
    pub(crate) fn set_text_outline(&mut self, outline: Option<(f64, [u8; 3])>) {
        self.edit_open_text(|_| {});
        let Some(index) = self
            .text_edit
            .as_ref()
            .and_then(|edit| self.doc.layers.iter().position(|l| l.id == edit.target))
        else {
            return;
        };
        let effects = &mut self.doc.layers[index].effects;
        let stroke = effects
            .iter()
            .position(|e| matches!(e.effect.kind, Kind::Stroke { .. }));
        match (outline, stroke) {
            (None, None) => return,
            (None, Some(at)) => {
                effects.remove(at);
            }
            (Some((size, color)), at) => {
                let size = size.clamp(1.0, LARGEST_OUTLINE);
                let kind = Kind::Stroke {
                    size,
                    position: Position::Outside,
                };
                match at {
                    Some(at) => {
                        let entry = &mut effects[at];
                        entry.enabled = true;
                        entry.effect.kind = kind;
                        entry.effect.paint = Paint::Color(color);
                    }
                    None if effects.len() < crate::document::MAX_EFFECTS => {
                        let Some(mut effect) = Effect::preset("Stroke") else {
                            return;
                        };
                        effect.kind = kind;
                        effect.paint = Paint::Color(color);
                        effects.push(LayerEffect::new(effect));
                    }
                    None => {
                        self.status = "Too many layer effects for an outline".into();
                        return;
                    }
                }
            }
        }
        self.changed();
    }

    /// Font, size, color, alignment and formatting, in the options bar.
    pub(crate) fn text_options_ui(&mut self, ui: &mut egui::Ui, narrow: bool) {
        text_render::warm_up();
        // Nothing may change the text while its last drawing comes in.
        if self.text_finishing() {
            ui.spinner();
            ui.label("Finishing text…");
            if ui.button("Keep editing").on_hover_text("Esc").clicked() {
                self.keep_editing_text();
            }
            return;
        }
        let original = self.shown_text_style();
        let mut style = original.clone();
        let family = text_render::family_of(&style.font);
        let label = match family {
            Some((family, _)) => family.name.as_str(),
            None if style.font.is_empty() => "Font",
            None => style.font.as_str(),
        };
        let label: String = label.chars().take(22).collect();
        let button = ui
            .menu_button(label, |ui| self.font_menu_ui(ui, &mut style))
            .response
            .on_hover_text("Font · scroll to step through fonts");
        if button.hovered() {
            self.step_font_with_wheel(ui, &mut style);
        }
        if let Some((family, current)) = family.filter(|(f, _)| f.faces.len() > 1) {
            ui.menu_button(family.faces[current].style.as_str(), |ui| {
                for face in &family.faces {
                    if ui
                        .selectable_label(face.name == style.font, &face.style)
                        .clicked()
                    {
                        style.font.clone_from(&face.name);
                        style.format.faux_bold = false;
                        style.format.faux_italic = false;
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Style");
        }
        ui.add(
            egui::DragValue::new(&mut style.size)
                .range(1.0..=8192.0)
                .speed(0.5)
                .max_decimals(1)
                .suffix(" px"),
        )
        .on_hover_text("Text size in canvas pixels");
        if !narrow {
            let dpi = self.doc.dpi;
            let mut points = style.size * 72.0 / dpi;
            if ui
                .add(
                    egui::DragValue::new(&mut points)
                        .range(72.0 / dpi..=8192.0 * 72.0 / dpi)
                        .speed(0.25)
                        .max_decimals(1)
                        .suffix(" pt"),
                )
                .on_hover_text("Text size in points at this document's DPI")
                .changed()
            {
                style.size = (points * dpi / 72.0).clamp(1.0, 8192.0);
            }
        }
        self.quick_color_ui(ui);
        for (align, name) in [
            (Align::Left, "Left"),
            (Align::Center, "Center"),
            (Align::Right, "Right"),
            (Align::Justify, "Justify"),
        ] {
            ui.selectable_value(&mut style.align, align, name);
        }
        let (mut bold, mut italic) = style.looks();
        let toggles = [
            (&mut bold, egui::RichText::new("B").strong(), "Bold"),
            (&mut italic, egui::RichText::new("I").italics(), "Italic"),
        ];
        let mut looks_changed = false;
        for (value, label, hint) in toggles {
            looks_changed |= ui.toggle_value(value, label).on_hover_text(hint).changed();
        }
        if looks_changed {
            style.set_look(bold, italic);
        }
        ui.toggle_value(
            &mut style.format.underline,
            egui::RichText::new("U").underline(),
        )
        .on_hover_text("Underline");
        if ui
            .toggle_value(&mut style.vertical, "Vertical")
            .on_hover_text("Lines run down the canvas")
            .changed()
            && style.vertical
        {
            // Turning vertical starts out left to right.
            style.rightward = true;
        }
        if style.vertical {
            let facing = [
                (
                    Letters::TurnedRight,
                    "Letters turned right",
                    "Read downward",
                ),
                (
                    Letters::TurnedLeft,
                    "Letters turned left",
                    "Read upward, like a book's spine",
                ),
                (
                    Letters::Upright,
                    "Letters upright",
                    "Stacked one under another",
                ),
            ];
            let name = facing
                .iter()
                .find(|(letters, ..)| *letters == style.letters)
                .map_or("Letters", |(_, name, _)| name);
            ui.menu_button(name, |ui| {
                for (letters, name, hint) in facing {
                    if ui
                        .selectable_value(&mut style.letters, letters, name)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Which way the letters face. Japanese stays upright unless turned left");
            ui.toggle_value(&mut style.rightward, "Left to right")
                .on_hover_text("The first line is on the left and new lines go right");
        }
        ui.menu_button("Format", |ui| self.format_menu_ui(ui, &mut style))
            .response
            .on_hover_text("Strikethrough, spacing, box and outline");
        if style != original {
            self.set_text_style(style);
        }
        // A pen display may have no keyboard to finish with.
        if self.text_edit.is_some() {
            if ui.button("Done").on_hover_text("Ctrl+Enter").clicked() {
                self.request_text_finish();
            } else if ui.button("Cancel").on_hover_text("Esc").clicked() {
                self.cancel_text_edit();
            }
            #[cfg(windows)]
            if ui
                .button("Touch keyboard")
                .on_hover_text("Show or hide the Windows touch keyboard")
                .clicked()
                && let Err(error) = crate::touch_keyboard::toggle()
            {
                self.status = error;
            }
        }
    }

    /// The rest of the formatting, which doesn't fit the bar.
    fn format_menu_ui(&mut self, ui: &mut egui::Ui, style: &mut Style) {
        ui.set_min_width(220.0);
        ui.checkbox(&mut style.format.strikethrough, "Strikethrough");
        ui.horizontal(|ui| {
            ui.label("Letter spacing");
            let mut percent = style.format.tracking * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .range(-50.0..=200.0)
                        .speed(0.5)
                        .max_decimals(1)
                        .suffix("%"),
                )
                .on_hover_text("Extra space after each letter, as a share of the text size")
                .changed()
            {
                style.format.tracking = percent / 100.0;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Line spacing");
            ui.add(
                egui::DragValue::new(&mut style.line_spacing)
                    .range(0.5..=5.0)
                    .speed(0.01)
                    .max_decimals(2)
                    .suffix("×"),
            )
            .on_hover_text("As a multiple of the font's line height");
        });
        ui.separator();
        let open = self
            .text_edit
            .as_ref()
            .map(|edit| &edit.draft)
            .or_else(|| self.selected_text().map(|(_, text)| text));
        let boxed = open.is_some_and(|text| text.wrap.is_some());
        let open = open.is_some();
        let mut wants_box = boxed;
        ui.add_enabled(open, egui::Checkbox::new(&mut wants_box, "Box"))
            .on_hover_text("Wrap the text in a box you can resize, or let it run as one line");
        if wants_box != boxed {
            self.edit_open_text(|text| text.set_box(wants_box));
        }
        let outline = self.text_outline();
        let mut outlined = outline.is_some();
        let (mut width, mut color) =
            outline.unwrap_or(((style.size * 0.08).max(2.0).round(), [255; 3]));
        let mut changed = false;
        ui.add_enabled_ui(open, |ui| {
            ui.horizontal(|ui| {
                changed |= ui
                    .checkbox(&mut outlined, "Outline")
                    .on_hover_text("An edge around the letters, from Layer Effects' Stroke")
                    .changed();
                ui.add_enabled_ui(outlined, |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut width)
                                .range(1.0..=LARGEST_OUTLINE)
                                .speed(0.25)
                                .suffix(" px"),
                        )
                        .changed();
                    changed |= ui.color_edit_button_srgb(&mut color).changed();
                });
            });
        });
        if changed {
            self.set_text_outline(outlined.then_some((width, color)));
        }
    }

    /// The wheel over the font button moves through the families, as it does
    /// over the blend box. Wheel up moves up the list.
    fn step_font_with_wheel(&mut self, ui: &mut egui::Ui, style: &mut Style) {
        let delta = ui.input_mut(|i| {
            let delta = i.raw_scroll_delta.y;
            i.raw_scroll_delta = egui::Vec2::ZERO;
            i.smooth_scroll_delta = egui::Vec2::ZERO;
            delta
        });
        self.font_list.wheel += delta;
        let steps = (self.font_list.wheel / FONT_WHEEL_STEP).trunc();
        self.font_list.wheel -= steps * FONT_WHEEL_STEP;
        let families = text_render::families();
        if steps == 0.0 || families.is_empty() {
            return;
        }
        let current = text_render::family_of(&style.font)
            .and_then(|(family, _)| families.iter().position(|f| f.name == family.name))
            .unwrap_or(0);
        let next = (current as i64 - steps as i64).clamp(0, families.len() as i64 - 1);
        let (bold, italic) = style.looks();
        style.choose(&families[next as usize], bold, italic);
    }

    fn font_menu_ui(&mut self, ui: &mut egui::Ui, style: &mut Style) {
        let families = text_render::families();
        if families.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading fonts…");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.font_list.search)
                .hint_text("Search fonts…")
                .desired_width(PREVIEW_SIZE[0] as f32),
        );
        let search = self.font_list.search.to_lowercase();
        let found: Vec<&text_render::Family> = families
            .iter()
            .filter(|family| {
                let name = family.name.to_lowercase();
                search.split_whitespace().all(|word| name.contains(word))
            })
            .collect();
        if self.font_list.previews.len() > PREVIEW_LIMIT {
            self.font_list.previews.clear();
        }
        let current = text_render::family_of(&style.font).map(|(family, _)| family.name.as_str());
        let mut budget = PREVIEWS_PER_FRAME;
        let height = PREVIEW_SIZE[1] as f32 + 4.0;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .auto_shrink([false, false])
            .show_rows(ui, height, found.len(), |ui, rows| {
                #[cfg(test)]
                {
                    self.font_list.first_row = rows.start;
                }
                for family in &found[rows] {
                    let name = &family.name;
                    let size = egui::vec2(PREVIEW_SIZE[0] as f32, height);
                    let (rect, row) = ui.allocate_exact_size(size, egui::Sense::click());
                    if !self.font_list.previews.contains_key(name) {
                        if budget == 0 {
                            ui.ctx().request_repaint();
                        } else {
                            budget -= 1;
                            let face = &family.closest(400, false).name;
                            let preview = preview_image(name, face).map(|image| {
                                ui.ctx().load_texture(
                                    format!("font preview {name}"),
                                    image,
                                    egui::TextureOptions::LINEAR,
                                )
                            });
                            self.font_list.previews.insert(name.clone(), preview);
                        }
                    }
                    let selected = current == Some(name.as_str());
                    if selected || row.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            2.0,
                            ui.visuals().widgets.hovered.weak_bg_fill,
                        );
                    }
                    let ink = ui.visuals().text_color();
                    match self.font_list.previews.get(name) {
                        Some(Some(texture)) => {
                            let image = egui::Rect::from_min_size(
                                rect.min + egui::vec2(0.0, 2.0),
                                egui::vec2(PREVIEW_SIZE[0] as f32, PREVIEW_SIZE[1] as f32),
                            );
                            let uv =
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                            ui.painter().image(texture.id(), image, uv, ink);
                        }
                        _ => {
                            ui.painter().text(
                                rect.left_center() + egui::vec2(2.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                name.as_str(),
                                egui::FontId::proportional(14.0),
                                ink,
                            );
                        }
                    }
                    let faces = family.faces.len();
                    let hint = if faces == 1 {
                        name.clone()
                    } else {
                        format!("{name} · {faces} styles")
                    };
                    if row.on_hover_text(hint).clicked() {
                        let (bold, italic) = style.looks();
                        style.choose(family, bold, italic);
                        ui.close();
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_render::TEST_FONT;

    fn app() -> Dudel {
        let mut app = Dudel {
            doc: crate::document::Document::new(300, 200),
            ..Default::default()
        };
        app.text_style.font = TEST_FONT.into();
        app.tool = crate::tools::Tool::Text;
        app
    }

    fn open(app: &mut Dudel, words: &str) {
        app.begin_new_text([20.0, 20.0], None, None);
        app.text_edit.as_mut().unwrap().draft.string = words.into();
        app.settle_text_draft();
    }

    #[test]
    fn fonts_without_a_family_fake_bold_and_italic() {
        let mut style = Style {
            font: TEST_FONT.into(),
            ..Default::default()
        };
        style.set_look(true, true);
        assert!(style.format.faux_bold && style.format.faux_italic);
        assert_eq!(style.looks(), (true, true));
        style.set_look(false, false);
        assert_eq!(style.looks(), (false, false));
    }

    #[test]
    fn families_use_their_real_bold_and_italic_faces() {
        // Looking a font up waits for the index, so the families are there.
        let _ = crate::text_render::find_font("Arial");
        assert!(!crate::text_render::families().is_empty());
        let Some((family, _)) = ["Arial", "Segoe UI", "DejaVu Sans"]
            .into_iter()
            .find_map(crate::text_render::family_of)
        else {
            return;
        };
        let mut style = Style {
            font: family.closest(400, false).name.clone(),
            ..Default::default()
        };
        style.set_look(true, false);
        if family.faces.iter().any(|f| f.weight >= BOLD && !f.italic) {
            assert!(!style.format.faux_bold, "a real bold face: {}", style.font);
            let (_, face) = crate::text_render::family_of(&style.font).unwrap();
            assert!(family.faces[face].weight >= BOLD);
        }
        assert!(style.looks().0);
        style.set_look(false, false);
        assert_eq!(style.looks(), (false, false));
        assert_eq!(
            crate::text_render::family_of(&style.font).unwrap().0.name,
            family.name
        );
    }

    #[test]
    fn the_outline_is_the_layers_stroke_effect_in_the_same_undo_step() {
        let mut app = app();
        open(&mut app, "Hemlo");
        app.set_text_outline(Some((4.0, [255, 255, 255])));
        assert_eq!(app.text_outline(), Some((4.0, [255, 255, 255])));
        app.set_text_outline(Some((6.0, [0, 0, 0])));
        let index = app.doc.active;
        assert_eq!(
            app.doc.layers[index].effects.len(),
            1,
            "one stroke, changed"
        );
        app.finish_text_edit();
        assert_eq!(app.text_outline(), Some((6.0, [0, 0, 0])));
        app.undo();
        assert_eq!(app.doc.layers.len(), 1, "text and outline undo together");
    }

    #[test]
    fn an_outline_alone_still_counts_as_a_change() {
        let mut app = app();
        open(&mut app, "Hemlo");
        app.finish_text_edit();
        let index = app.doc.active;
        app.begin_text_edit(index);
        app.set_text_outline(Some((4.0, [255, 255, 255])));
        app.finish_text_edit();
        assert!(app.text_outline().is_some());
        app.undo();
        assert!(app.text_outline().is_none(), "its own undo step");
        assert_eq!(app.doc.layers.len(), 2);
    }

    #[test]
    fn vertical_text_starts_out_left_to_right() {
        assert!(Style::default().rightward);
        let mut app = app();
        app.text_style.rightward = false;
        app.text_style.vertical = true;
        // As the bar does it: turning vertical on chooses left to right.
        let mut style = app.text_style.clone();
        style.vertical = false;
        app.text_style = style;
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut style = app.shown_text_style();
                let before = style.vertical;
                style.vertical = true;
                if style.vertical != before {
                    style.rightward = true;
                }
                app.set_text_style(style);
                let _ = ui;
            });
        });
        assert!(app.text_style.vertical && app.text_style.rightward);
        app.begin_new_text([60.0, 30.0], None, None);
        let edit = app.text_edit.as_ref().unwrap();
        assert_eq!(edit.draft.down, [1.0, 0.0], "lines go right");
    }

    #[test]
    fn the_box_toggle_turns_a_line_into_a_box_and_back() {
        let mut app = app();
        open(&mut app, "Hemlo");
        app.edit_open_text(|text| text.set_box(true));
        let draft = &app.text_edit.as_ref().unwrap().draft;
        assert!(draft.wrap.is_some() && draft.box_height.is_some());
        let carets = draft.carets().unwrap();
        assert!(carets.iter().all(|c| c.top == 0.0), "still one line");
        app.edit_open_text(|text| text.set_box(false));
        let draft = &app.text_edit.as_ref().unwrap().draft;
        assert!(draft.wrap.is_none() && draft.box_height.is_none());
    }

    #[test]
    fn formatting_from_the_bar_reaches_the_text_and_new_text() {
        let mut app = app();
        open(&mut app, "Hemlo");
        let mut style = app.shown_text_style();
        style.format.underline = true;
        style.format.tracking = 0.2;
        style.align = Align::Justify;
        app.set_text_style(style);
        app.finish_text_edit();
        let text = app.doc.layers[app.doc.active].text.as_ref().unwrap();
        assert!(text.format.underline);
        assert_eq!(text.format.tracking, 0.2);
        assert!(app.text_style.format.underline, "new text looks the same");
    }
}
