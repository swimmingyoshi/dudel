//! The app's side of interface styles: applying one, choosing one and
//! customizing it in Settings or View › Theme.
use crate::{
    Dudel, egui,
    icons::{self, Icon},
    themes,
    ui::{
        style::{self, Active, Density, IconLook, Interface, Layout, Palette},
        widgets,
    },
};
use egui::{Color32, Rect, RichText, Sense, Stroke, vec2};

/// Apply the interface style on top of the theme, only when either changed.
///
/// The Classic style with its own spacing applies exactly what the theme
/// alone did before styles existed.
pub fn apply(ctx: &egui::Context, interface: &Interface, theme: &themes::Theme) {
    let id = egui::Id::new("applied dudel interface");
    let key = (interface.clone(), theme.clone());
    if ctx
        .data(|d| d.get_temp::<(Interface, themes::Theme)>(id))
        .as_ref()
        == Some(&key)
    {
        return;
    }
    // Every change starts again from the style egui began with, so nothing
    // one style set (touch-sized checkboxes, rounder menus) outlives it.
    let base_id = egui::Id::new("dudel base style");
    let base = ctx
        .data(|d| d.get_temp::<std::sync::Arc<egui::Style>>(base_id))
        .unwrap_or_else(|| {
            let base = ctx.style();
            ctx.data_mut(|d| d.insert_temp(base_id, base.clone()));
            base
        });
    ctx.set_style(base);
    themes::forget_applied(ctx);
    themes::apply(ctx, theme);
    let effective = Interface {
        palette: colors(interface, theme),
        ..interface.clone()
    };
    if effective.palette.is_some() || effective.custom_metrics() {
        let mut s = (*ctx.style()).clone();
        if let Some(palette) = &effective.palette {
            style::apply_palette(&mut s, palette);
        }
        style::apply_metrics(&mut s, effective.density, effective.radius);
        ctx.set_style(s);
    }
    Active::new(&effective, &ctx.style()).store(ctx);
    ctx.data_mut(|d| d.insert_temp(id, key));
}

/// A theme's colors as a whole style palette.
pub fn theme_palette(theme: &themes::Theme) -> Palette {
    Palette::from_theme(
        theme.dark,
        theme.panel,
        theme.window,
        theme.text,
        theme.accent,
        theme.canvas_background,
    )
}

/// The palette a style is drawn with: its own colors, or the theme's made
/// into a whole palette, so that Slate, Float and Atelier keep their raised
/// islands, wells and accent marks in any theme. Classic with the theme's
/// colors draws exactly as the theme alone does.
fn colors(interface: &Interface, theme: &themes::Theme) -> Option<Palette> {
    interface
        .palette
        .or_else(|| (interface.layout != Layout::Classic).then(|| theme_palette(theme)))
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// A small picture of a style's layout in its colors.
fn preset_card(
    ui: &mut egui::Ui,
    layout: Layout,
    chosen: bool,
    theme: &themes::Theme,
) -> egui::Response {
    let size = vec2(104.0, 84.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let palette = Interface::preset(layout)
        .palette
        .unwrap_or_else(|| theme_palette(theme));
    let p = ui.painter();
    // Drawn on a 128 by 74 grid, scaled to the card.
    let k = size.x / 128.0;
    let screen = Rect::from_min_size(rect.min, vec2(128.0, 74.0) * k);
    let pt = |x: f32, y: f32| screen.min + vec2(x, y) * k;
    p.rect_filled(screen, 6.0, rgb(palette.canvas));
    let fill = |r: Rect, c: [u8; 3]| p.rect_filled(r, 1.5, rgb(c));
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pt(x, y), vec2(w, h) * k);
    let page = |r: Rect| p.rect_filled(r, 0.0, Color32::from_gray(248));
    let accent = rgb(palette.accent);
    match layout {
        Layout::Classic => {
            fill(at(0.0, 0.0, 128.0, 9.0), palette.panel);
            fill(at(0.0, 9.0, 7.0, 65.0), palette.panel);
            fill(at(8.0, 10.0, 24.0, 63.0), palette.panel);
            fill(at(96.0, 10.0, 32.0, 63.0), palette.panel);
            page(at(46.0, 20.0, 36.0, 44.0));
            p.circle_filled(pt(3.5, 15.0), 2.5 * k, accent);
        }
        Layout::Slate => {
            fill(at(0.0, 0.0, 128.0, 6.0), palette.base);
            fill(at(0.0, 6.0, 128.0, 8.0), palette.panel);
            fill(at(0.0, 14.0, 7.0, 55.0), palette.panel);
            fill(at(92.0, 14.0, 36.0, 55.0), palette.panel);
            for y in [15.0, 34.0, 50.0] {
                fill(at(92.0, y, 36.0, 3.0), palette.base);
                p.rect_filled(at(94.0, y + 2.0, 8.0, 1.0), 0.0, accent);
            }
            fill(at(0.0, 69.0, 128.0, 5.0), palette.panel);
            page(at(34.0, 20.0, 40.0, 44.0));
            p.rect_filled(at(1.5, 17.0, 4.0, 4.0), 1.0, accent);
            for x in [18.0, 32.0, 46.0] {
                p.rect_filled(at(x, 12.0, 10.0, 1.0), 0.0, accent.gamma_multiply(0.8));
            }
        }
        Layout::Float => {
            fill(at(0.0, 0.0, 128.0, 5.0), palette.base);
            page(at(34.0, 9.0, 60.0, 62.0));
            p.rect_filled(at(4.0, 9.0, 34.0, 8.0), 4.0, rgb(palette.raised));
            p.rect_filled(at(78.0, 9.0, 46.0, 8.0), 4.0, rgb(palette.raised));
            p.circle_filled(at(80.0, 9.0, 8.0, 8.0).center(), 2.6 * k, accent);
            p.rect_filled(at(4.0, 26.0, 8.0, 30.0), 4.0, rgb(palette.raised));
            p.rect_filled(at(38.0, 63.0, 52.0, 8.0), 4.0, rgb(palette.raised));
        }
        Layout::Atelier => {
            fill(at(0.0, 0.0, 128.0, 9.0), palette.panel);
            fill(at(0.0, 9.0, 30.0, 49.0), palette.panel);
            fill(at(98.0, 9.0, 30.0, 49.0), palette.panel);
            fill(at(0.0, 58.0, 128.0, 16.0), palette.panel);
            for y in [14.0, 22.0, 30.0] {
                p.line_segment(
                    [pt(4.0, y + 2.0), pt(26.0, y)],
                    Stroke::new(
                        2.0_f32,
                        accent.gamma_multiply(if y == 14.0 { 1.0 } else { 0.5 }),
                    ),
                );
            }
            for x in 0..8 {
                p.rect_filled(
                    at(6.0 + x as f32 * 14.0, 61.0, 11.0, 9.0),
                    1.0,
                    Color32::from_gray(245),
                );
            }
            p.rect_stroke(
                at(48.0, 61.0, 11.0, 9.0),
                1.0,
                Stroke::new(1.5_f32, accent),
                egui::StrokeKind::Outside,
            );
            page(at(40.0, 14.0, 48.0, 38.0));
        }
    }
    let ring = if chosen {
        Stroke::new(2.5_f32, ui.visuals().selection.stroke.color)
    } else if response.hovered() {
        Stroke::new(1.5_f32, ui.visuals().widgets.hovered.bg_stroke.color)
    } else {
        Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color)
    };
    ui.painter()
        .rect_stroke(screen, 6.0, ring, egui::StrokeKind::Outside);
    let label = RichText::new(layout.name());
    let label = if chosen { label.strong() } else { label };
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 10.0),
        egui::Align2::CENTER_CENTER,
        label.text(),
        egui::FontId::proportional(14.0),
        if chosen {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        },
    );
    response.on_hover_text(layout.hint())
}

impl Dudel {
    /// The color around the artwork: the style's, or the theme's.
    pub(crate) fn canvas_background(&self) -> [u8; 3] {
        self.interface
            .palette
            .map_or(self.theme.canvas_background, |p| p.canvas)
    }

    /// Switch to another style. Theme colors chosen for one of the palette
    /// styles carry over to another; otherwise the style brings its own.
    pub(crate) fn choose_interface(&mut self, layout: Layout) {
        let theme_colors =
            self.interface.layout != Layout::Classic && self.interface.palette.is_none();
        self.interface.choose_preset(layout);
        if theme_colors {
            self.interface.palette = None;
        }
        self.status = format!("Interface style: {} · {}", layout.name(), layout.hint());
    }

    /// Open Settings › Interface, where a style is customized.
    pub(crate) fn open_interface_settings(&mut self) {
        self.settings_open = true;
        self.settings_section = crate::settings_ui::page::INTERFACE;
        self.settings_search.clear();
    }

    /// The styles, the one in use marked.
    pub(crate) fn style_choices(&mut self, ui: &mut egui::Ui) {
        for layout in Layout::ALL {
            if ui
                .add(egui::Button::new(layout.name()).selected(self.interface.layout == layout))
                .on_hover_text(layout.hint())
                .clicked()
                && self.interface.layout != layout
            {
                self.choose_interface(layout);
                ui.close();
            }
        }
    }

    /// The style's own colors, when it has them, as one more choice beside
    /// the themes. Marked while in use.
    pub(crate) fn own_colors_choice(&mut self, ui: &mut egui::Ui) {
        let own = Interface::preset(self.interface.layout).palette;
        let label = match (self.interface.palette, own) {
            (Some(current), Some(own)) if current == own => {
                format!("{} colors", self.interface.layout.name())
            }
            (Some(_), _) => "Custom colors".to_owned(),
            (None, Some(_)) => format!("{} colors", self.interface.layout.name()),
            (None, None) => return,
        };
        let using = self.interface.palette.is_some();
        if ui
            .add(egui::Button::new(label).selected(using))
            .on_hover_text("The colors this style was designed with")
            .clicked()
            && !using
        {
            self.interface.palette = own;
        }
    }

    /// Settings › Workspace › Interface style.
    pub(crate) fn interface_settings_ui(&mut self, ui: &mut egui::Ui) {
        let current = self.interface.matching_preset();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
            for layout in Layout::ALL {
                if preset_card(ui, layout, current == Some(layout), &self.theme).clicked() {
                    self.choose_interface(layout);
                }
            }
        });
        if current.is_none() {
            ui.horizontal(|ui| {
                ui.label(format!("Customized from {}.", self.interface.layout.name()));
                if ui
                    .small_button(format!("Reset to {}", self.interface.layout.name()))
                    .clicked()
                {
                    // Everything back as designed, its colors included.
                    self.interface.choose_preset(self.interface.layout);
                }
            });
        }
        ui.add_space(6.0);
        egui::Grid::new("interface customize")
            .num_columns(2)
            .spacing(vec2(12.0, 8.0))
            .show(ui, |ui| {
                ui.label("Layout");
                let layouts: Vec<(Layout, &str)> =
                    Layout::ALL.iter().map(|l| (*l, l.name())).collect();
                widgets::segmented(ui, &mut self.interface.layout, &layouts);
                ui.end_row();
                ui.label("Icons");
                ui.vertical(|ui| {
                    let looks: Vec<(IconLook, &str)> =
                        IconLook::ALL.iter().map(|l| (*l, l.name())).collect();
                    widgets::segmented(ui, &mut self.interface.icons, &looks);
                    ui.horizontal(|ui| {
                        for icon in [
                            Icon::Pencil,
                            Icon::Pen,
                            Icon::Brush,
                            Icon::Airbrush,
                            Icon::Wet,
                            Icon::Blend,
                            Icon::Pixel,
                            Icon::Eraser,
                            Icon::Fill,
                            Icon::LassoFill,
                            Icon::Text,
                            Icon::Layers,
                            Icon::Pressure,
                            Icon::Snap,
                            Icon::Play,
                        ] {
                            icons::mark(ui, icon, 22.0, ui.visuals().text_color());
                        }
                    });
                });
                ui.end_row();
                ui.label("Density");
                let densities: Vec<(Density, &str)> =
                    Density::ALL.iter().map(|d| (*d, d.name())).collect();
                widgets::segmented(ui, &mut self.interface.density, &densities);
                ui.end_row();
                ui.label("Corners");
                ui.add(egui::Slider::new(&mut self.interface.radius, 0..=16).suffix(" pt"));
                ui.end_row();
                ui.label("Colors");
                let mut own = self.interface.palette.is_some();
                if widgets::segmented(ui, &mut own, &[(false, "Theme"), (true, "Style")]) {
                    self.interface.palette = own.then(|| {
                        Interface::preset(self.interface.layout)
                            .palette
                            .unwrap_or_else(Palette::slate)
                    });
                }
                ui.end_row();
            });
        match &mut self.interface.palette {
            Some(palette) => {
                ui.checkbox(&mut palette.dark, "Dark controls");
                egui::Grid::new("interface palette")
                    .num_columns(4)
                    .spacing(vec2(8.0, 6.0))
                    .show(ui, |ui| {
                        for (index, (name, color)) in palette.entries().into_iter().enumerate() {
                            ui.color_edit_button_srgb(color);
                            ui.label(name);
                            if index % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    ui.weak("Start from");
                    for (name, preset) in [
                        ("Slate", Palette::slate()),
                        ("Float", Palette::float()),
                        ("Atelier", Palette::atelier()),
                    ] {
                        if ui.small_button(name).clicked() {
                            *palette = preset;
                        }
                    }
                });
            }
            None => {
                ui.menu_button(format!("Theme: {}", self.theme.name), |ui| {
                    self.theme_choices(ui);
                    ui.separator();
                    self.theme_file_actions(ui);
                });
            }
        }
        if self.interface.layout == Layout::Float {
            ui.checkbox(
                &mut self.interface.rails_right,
                "Slider rail on the right, for drawing with the left hand",
            );
        }
        self.interface.sanitize();
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(interface: &Interface, theme: &themes::Theme) -> egui::Style {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            apply(ctx, interface, theme)
        });
        (*ctx.style()).clone()
    }

    #[test]
    fn classic_applies_exactly_what_the_theme_alone_did() {
        let theme = themes::Theme::default();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| themes::apply(ctx, &theme));
        // Compared as text: egui's style holds a NaN, which never equals itself.
        let alone = format!("{:?}", ctx.style());
        assert_eq!(format!("{:?}", run(&Interface::default(), &theme)), alone);
    }

    #[test]
    fn styles_take_their_palette_and_metrics() {
        let theme = themes::Theme::default();
        let slate = run(&Interface::preset(Layout::Slate), &theme);
        assert_eq!(slate.visuals.panel_fill, rgb(Palette::slate().panel));
        assert_eq!(slate.spacing.interact_size.y, Density::Compact.row());
        let atelier = run(&Interface::preset(Layout::Atelier), &theme);
        assert!(!atelier.visuals.dark_mode);
        assert_eq!(atelier.visuals.widgets.inactive.corner_radius.nw, 8);
        // Switching back restores the Classic look in the same context.
        let ctx = egui::Context::default();
        for interface in [Interface::preset(Layout::Float), Interface::default()] {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                apply(ctx, &interface, &theme)
            });
        }
        assert_eq!(
            format!("{:?}", ctx.style()),
            format!("{:?}", run(&Interface::default(), &theme))
        );
        assert_eq!(Active::get(&ctx).layout, Layout::Classic);
    }

    #[test]
    fn a_theme_draws_any_style_and_carries_between_palette_styles() {
        let mut app = Dudel::default();
        app.choose_interface(Layout::Slate);
        assert_eq!(app.interface.palette, Some(Palette::slate()));
        let paper = themes::Theme::presets().remove(1);
        app.use_theme(paper.clone());
        assert!(app.interface.palette.is_none());
        let style = run(&app.interface, &app.theme);
        assert_eq!(style.visuals.panel_fill, rgb(paper.panel));
        assert!(!style.visuals.dark_mode);
        assert_eq!(
            style.spacing.interact_size.y,
            Density::Compact.row(),
            "still Slate"
        );
        assert_eq!(app.canvas_background(), paper.canvas_background);
        // Paper stays when going to another palette style.
        app.choose_interface(Layout::Float);
        assert!(app.interface.palette.is_none(), "Paper carries over");
        // From Classic, a style brings its own colors.
        app.choose_interface(Layout::Classic);
        app.choose_interface(Layout::Atelier);
        assert_eq!(app.interface.palette, Some(Palette::atelier()));
    }

    #[test]
    fn the_canvas_uses_the_style_color_only_with_style_colors() {
        let mut app = Dudel::default();
        assert_eq!(app.canvas_background(), app.theme.canvas_background);
        app.interface = Interface::preset(Layout::Atelier);
        assert_eq!(app.canvas_background(), Palette::atelier().canvas);
    }
}
