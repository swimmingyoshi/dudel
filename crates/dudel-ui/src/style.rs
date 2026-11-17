//! Interface styles: where the panels go, how icons are drawn, how dense the
//! controls are and which colors they use.
//!
//! The Classic style is the interface as it was before styles existed: its
//! colors come from the chosen theme and nothing here changes how it looks.
use egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle, vec2};
use serde::{Deserialize, Serialize};

/// Where the panels go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Layout {
    /// Tool rail, tool settings on the left, color and layers on the right.
    #[default]
    Classic,
    /// Grouped tool rail, one dock of tabbed groups on the right, status bar.
    Slate,
    /// The canvas fills the window; tools sit on floating islands.
    Float,
    /// Brush drawer on the left, color and layers on the right, timeline below.
    Atelier,
}

impl Layout {
    pub const ALL: [Layout; 4] = [
        Layout::Classic,
        Layout::Slate,
        Layout::Float,
        Layout::Atelier,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Layout::Classic => "Classic",
            Layout::Slate => "Slate",
            Layout::Float => "Float",
            Layout::Atelier => "Atelier",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Layout::Classic => "The familiar Dudel layout with your theme's colors",
            Layout::Slate => {
                "Desktop studio: grouped tools, tabbed docks and compact fields you drag to change"
            }
            Layout::Float => {
                "Canvas first: tools on floating islands, sliders on a side rail, panels as popovers"
            }
            Layout::Atelier => {
                "Painting and animation: a brush drawer, a color tray and a timeline below"
            }
        }
    }
}

/// How icons are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IconLook {
    /// The original painter-drawn icons.
    #[default]
    Classic,
    /// Thin outlines with square ends.
    Line,
    /// Rounded outlines with a light tint inside.
    Duotone,
    /// Filled shapes with details cut out.
    Solid,
}

impl IconLook {
    pub const ALL: [IconLook; 4] = [
        IconLook::Classic,
        IconLook::Line,
        IconLook::Duotone,
        IconLook::Solid,
    ];
    pub fn name(self) -> &'static str {
        match self {
            IconLook::Classic => "Classic",
            IconLook::Line => "Line",
            IconLook::Duotone => "Duotone",
            IconLook::Solid => "Solid",
        }
    }
}

/// How large controls and the gaps between them are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Regular,
    /// Sized for fingers and pens on a screen.
    Touch,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Compact, Density::Regular, Density::Touch];
    pub fn name(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Regular => "Regular",
            Density::Touch => "Touch",
        }
    }
    /// Height of a row of controls, in points.
    pub fn row(self) -> f32 {
        match self {
            Density::Compact => 26.0,
            Density::Regular => 32.0,
            Density::Touch => 44.0,
        }
    }
}

/// Atelier's workspaces: which panels sit beside the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    Draw,
    #[default]
    Paint,
    Animate,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Draw, Mode::Paint, Mode::Animate];
    pub fn name(self) -> &'static str {
        match self {
            Mode::Draw => "Draw",
            Mode::Paint => "Paint",
            Mode::Animate => "Animate",
        }
    }
}

/// The colors of a style. Every surface and state takes its color from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Palette {
    pub dark: bool,
    /// Title bar and the gaps between panels.
    pub base: [u8; 3],
    /// Docks, rails and bars.
    pub panel: [u8; 3],
    /// Popovers, menus, dialogs and buttons.
    pub raised: [u8; 3],
    /// Fields, wells and slider tracks.
    pub sunken: [u8; 3],
    /// Separators and panel edges.
    pub line: [u8; 3],
    pub text: [u8; 3],
    /// Labels and hints.
    pub muted: [u8; 3],
    /// The active tool, focus and the chosen tab.
    pub accent: [u8; 3],
    /// Text and icons drawn on a solid accent fill.
    pub accent_text: [u8; 3],
    /// The workspace around the artwork.
    pub canvas: [u8; 3],
}

impl Palette {
    pub fn slate() -> Self {
        Self {
            dark: true,
            base: [22, 24, 28],
            panel: [30, 33, 38],
            raised: [41, 45, 52],
            sunken: [18, 20, 23],
            line: [48, 53, 61],
            text: [227, 229, 232],
            muted: [138, 145, 155],
            accent: [224, 161, 66],
            accent_text: [27, 20, 5],
            canvas: [14, 15, 18],
        }
    }
    pub fn float() -> Self {
        Self {
            dark: true,
            base: [29, 31, 36],
            panel: [34, 37, 43],
            raised: [43, 46, 53],
            sunken: [23, 25, 29],
            line: [56, 60, 69],
            text: [243, 244, 246],
            muted: [165, 171, 181],
            accent: [62, 199, 179],
            accent_text: [13, 39, 35],
            canvas: [43, 46, 52],
        }
    }
    pub fn atelier() -> Self {
        Self {
            dark: false,
            base: [230, 233, 229],
            panel: [246, 247, 244],
            raised: [255, 255, 255],
            sunken: [233, 236, 232],
            line: [211, 216, 210],
            text: [28, 34, 32],
            muted: [102, 112, 107],
            accent: [45, 63, 168],
            accent_text: [255, 255, 255],
            canvas: [217, 221, 215],
        }
    }
    /// A style's colors made from a theme's few: its panels, windows, text,
    /// selection and the color around the canvas. The rest is derived, and
    /// the accent is lifted toward the text until it reads as a mark on the
    /// panels (Graphite's grey selection would otherwise vanish there).
    pub fn from_theme(
        dark: bool,
        panel: [u8; 3],
        window: [u8; 3],
        text: [u8; 3],
        accent: [u8; 3],
        canvas: [u8; 3],
    ) -> Self {
        let black = [0, 0, 0];
        let accent = (0..=10)
            .map(|step| mix(accent, text, step as f32 / 10.0))
            .find(|c| contrast(*c, panel) >= 3.0)
            .unwrap_or(text);
        Self {
            dark,
            base: mix(panel, black, if dark { 0.25 } else { 0.08 }),
            panel,
            raised: window,
            sunken: mix(panel, black, if dark { 0.3 } else { 0.06 }),
            line: mix(panel, text, if dark { 0.16 } else { 0.2 }),
            text,
            muted: mix(text, panel, 0.3),
            accent,
            accent_text: if contrast(accent, [255, 255, 255]) >= contrast(accent, [20, 20, 20]) {
                [255, 255, 255]
            } else {
                [20, 20, 20]
            },
            canvas,
        }
    }
    /// The named colors, for editing them one by one.
    pub fn entries(&mut self) -> [(&'static str, &mut [u8; 3]); 10] {
        [
            ("Title bar", &mut self.base),
            ("Panels", &mut self.panel),
            ("Menus and buttons", &mut self.raised),
            ("Fields", &mut self.sunken),
            ("Lines", &mut self.line),
            ("Text", &mut self.text),
            ("Labels", &mut self.muted),
            ("Accent", &mut self.accent),
            ("Text on accent", &mut self.accent_text),
            ("Around the canvas", &mut self.canvas),
        ]
    }
}

/// Everything about how the interface looks, saved with the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interface {
    pub layout: Layout,
    pub icons: IconLook,
    pub density: Density,
    /// Corner radius of buttons and fields, in points.
    pub radius: u8,
    /// The style's own colors; None uses the chosen theme's colors.
    pub palette: Option<Palette>,
    /// Float: the slider rail sits on the right, for left-handed drawing.
    pub rails_right: bool,
    /// Atelier: which workspace is shown.
    pub mode: Mode,
}

impl Default for Interface {
    fn default() -> Self {
        Self::preset(Layout::Classic)
    }
}

/// The corner radius the Classic style has always used.
pub const CLASSIC_RADIUS: u8 = 5;

impl Interface {
    /// A style as designed, with its layout, icons, density and colors.
    pub fn preset(layout: Layout) -> Self {
        let (icons, density, radius, palette) = match layout {
            Layout::Classic => (IconLook::Classic, Density::Regular, CLASSIC_RADIUS, None),
            Layout::Slate => (IconLook::Line, Density::Compact, 4, Some(Palette::slate())),
            Layout::Float => (
                IconLook::Duotone,
                Density::Touch,
                10,
                Some(Palette::float()),
            ),
            Layout::Atelier => (
                IconLook::Solid,
                Density::Regular,
                8,
                Some(Palette::atelier()),
            ),
        };
        Self {
            layout,
            icons,
            density,
            radius,
            palette,
            rails_right: false,
            mode: Mode::default(),
        }
    }
    /// The preset this matches, ignoring handedness and the Atelier mode, or
    /// None when something was customized.
    pub fn matching_preset(&self) -> Option<Layout> {
        Layout::ALL.into_iter().find(|&layout| {
            let preset = Self::preset(layout);
            preset.layout == self.layout
                && preset.icons == self.icons
                && preset.density == self.density
                && preset.radius == self.radius
                && preset.palette == self.palette
        })
    }
    /// Switch to another preset, keeping handedness and the Atelier mode.
    pub fn choose_preset(&mut self, layout: Layout) {
        *self = Self {
            rails_right: self.rails_right,
            mode: self.mode,
            ..Self::preset(layout)
        };
    }
    pub fn sanitize(&mut self) {
        self.radius = self.radius.min(16);
    }
    /// Spacing and corners differ from the Classic ones.
    pub fn custom_metrics(&self) -> bool {
        self.density != Density::Regular || self.radius != CLASSIC_RADIUS
    }
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// `a` moved toward `b` by `t` (0 keeps `a`, 1 gives `b`).
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// Relative luminance of an sRGB color, as WCAG defines it.
fn luminance(c: [u8; 3]) -> f32 {
    c.into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(c, weight)| {
            let v = c as f32 / 255.0;
            weight
                * if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

/// WCAG contrast ratio between two colors, from 1 to 21.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Replace the style's colors with the palette's.
pub fn apply_palette(style: &mut egui::Style, p: &Palette) {
    let numeric_color_space = style.visuals.numeric_color_space;
    style.visuals = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let v = &mut style.visuals;
    v.numeric_color_space = numeric_color_space;
    v.override_text_color = Some(rgb(p.text));
    v.weak_text_color = Some(rgb(p.muted));
    v.panel_fill = rgb(p.panel);
    v.window_fill = rgb(p.raised);
    v.window_stroke = Stroke::new(1.0_f32, rgb(p.line));
    v.extreme_bg_color = rgb(p.sunken);
    v.text_edit_bg_color = Some(rgb(p.sunken));
    v.code_bg_color = rgb(p.sunken);
    v.faint_bg_color = rgb(mix(p.panel, p.raised, 0.5));
    v.hyperlink_color = rgb(p.accent);
    // Chosen things get a tint of the accent with the accent itself on top,
    // so the accent also reads as a mark on a plain panel.
    v.selection.bg_fill = rgb(mix(p.panel, p.accent, if p.dark { 0.26 } else { 0.16 }));
    v.selection.stroke = Stroke::new(1.0_f32, rgb(p.accent));
    let well = mix(p.line, p.text, if p.dark { 0.18 } else { 0.08 });
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = rgb(p.panel);
    w.noninteractive.weak_bg_fill = rgb(p.panel);
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, rgb(p.line));
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, rgb(p.text));
    w.inactive.bg_fill = rgb(well);
    w.inactive.weak_bg_fill = rgb(p.raised);
    w.inactive.bg_stroke = if p.dark {
        Stroke::NONE
    } else {
        Stroke::new(1.0_f32, rgb(p.line))
    };
    w.inactive.fg_stroke = Stroke::new(1.0_f32, rgb(p.text));
    w.hovered.weak_bg_fill = rgb(mix(p.raised, p.text, 0.08));
    w.hovered.bg_fill = rgb(mix(well, p.text, 0.12));
    w.hovered.bg_stroke = Stroke::new(1.0_f32, rgb(mix(p.line, p.text, 0.3)));
    w.hovered.fg_stroke = Stroke::new(1.5_f32, rgb(p.text));
    w.active.weak_bg_fill = rgb(mix(p.raised, p.accent, 0.3));
    w.active.bg_fill = rgb(p.accent);
    w.active.bg_stroke = Stroke::new(1.0_f32, rgb(p.accent));
    w.active.fg_stroke = Stroke::new(2.0_f32, rgb(p.text));
    w.open.weak_bg_fill = rgb(mix(p.raised, p.text, 0.05));
    w.open.bg_fill = rgb(mix(well, p.text, 0.05));
    w.open.bg_stroke = Stroke::new(1.0_f32, rgb(p.line));
    w.open.fg_stroke = Stroke::new(1.0_f32, rgb(p.text));
}

/// Set control sizes, gaps, text sizes and corner radius.
pub fn apply_metrics(style: &mut egui::Style, density: Density, radius: u8) {
    let (spacing, padding, body, button, small) = match density {
        Density::Compact => (vec2(6.0, 5.0), vec2(7.0, 4.0), 14.0, 13.0, 12.0),
        Density::Regular => (vec2(8.0, 8.0), vec2(10.0, 7.0), 15.0, 14.0, 12.5),
        Density::Touch => (vec2(10.0, 10.0), vec2(14.0, 10.0), 16.0, 15.0, 13.5),
    };
    style.spacing.item_spacing = spacing;
    style.spacing.button_padding = padding;
    style.spacing.interact_size.y = density.row();
    if density == Density::Touch {
        style.spacing.icon_width = 20.0;
        style.spacing.icon_width_inner = 12.0;
        style.interaction.resize_grab_radius_side = 12.0;
    }
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(body));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(button));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(small));
    let r = CornerRadius::same(radius);
    for widgets in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widgets.corner_radius = r;
    }
    style.visuals.window_corner_radius = CornerRadius::same(radius.saturating_add(3));
    style.visuals.menu_corner_radius = CornerRadius::same(radius.saturating_add(2));
}

/// What the rest of the interface needs to know about the style in use,
/// kept in the egui context so drawing code can read it without the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Active {
    /// The style has its own palette (not the Classic theme colors).
    pub styled: bool,
    pub layout: Layout,
    pub icons: IconLook,
    pub density: Density,
    /// Fill and icon color of a chosen icon button.
    pub chosen_fill: Color32,
    pub chosen_ink: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    /// Panel and title bar colors, for chrome drawn by hand.
    pub base: Color32,
    pub line: Color32,
}

impl Active {
    /// Derive the active style from the egui style it was applied to.
    pub fn new(interface: &Interface, style: &egui::Style) -> Self {
        let v = &style.visuals;
        let (chosen_fill, chosen_ink, accent, accent_text, base, line) = match &interface.palette {
            None => (
                v.selection.bg_fill,
                Color32::WHITE,
                v.selection.bg_fill,
                Color32::WHITE,
                v.panel_fill,
                v.widgets.noninteractive.bg_stroke.color,
            ),
            Some(p) => {
                let solid = interface.icons == IconLook::Solid;
                (
                    if solid {
                        rgb(p.accent)
                    } else {
                        v.selection.bg_fill
                    },
                    if solid {
                        rgb(p.accent_text)
                    } else {
                        rgb(p.accent)
                    },
                    rgb(p.accent),
                    rgb(p.accent_text),
                    rgb(p.base),
                    rgb(p.line),
                )
            }
        };
        Self {
            styled: interface.palette.is_some(),
            layout: interface.layout,
            icons: interface.icons,
            density: interface.density,
            chosen_fill,
            chosen_ink,
            accent,
            accent_text,
            base,
            line,
        }
    }
    fn id() -> egui::Id {
        egui::Id::new("dudel active interface style")
    }
    pub fn store(self, ctx: &egui::Context) {
        ctx.data_mut(|d| d.insert_temp(Self::id(), self));
    }
    /// The style in use, or the Classic one before any was applied.
    pub fn get(ctx: &egui::Context) -> Self {
        ctx.data(|d| d.get_temp::<Self>(Self::id()))
            .unwrap_or_else(|| Self::new(&Interface::default(), &ctx.style()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_makes_a_whole_style_palette_with_a_visible_accent() {
        // Graphite: a grey selection close to its panels.
        let panel = [36, 36, 36];
        let p = Palette::from_theme(
            true,
            panel,
            [52, 52, 52],
            [225, 225, 225],
            [94, 101, 108],
            [28, 31, 39],
        );
        assert!(contrast(p.accent, panel) >= 3.0, "{:?}", p.accent);
        assert!(contrast(p.accent_text, p.accent) >= 3.0);
        assert!(contrast(p.muted, panel) >= 3.0, "labels stay readable");
        assert_eq!(
            (p.panel, p.raised, p.canvas),
            (panel, [52, 52, 52], [28, 31, 39])
        );
        // Paper: light, with dark text on its accent.
        let panel = [208, 205, 198];
        let p = Palette::from_theme(
            false,
            panel,
            [222, 218, 209],
            [43, 43, 42],
            [143, 169, 189],
            [28, 31, 39],
        );
        assert!(contrast(p.accent, panel) >= 3.0);
        assert!(contrast(p.accent_text, p.accent) >= 3.0);
        assert!(!p.dark && p.sunken != p.panel && p.line != p.panel);
    }

    #[test]
    fn presets_are_recognized_and_customizing_one_leaves_it() {
        for layout in Layout::ALL {
            let mut interface = Interface::preset(layout);
            assert_eq!(interface.matching_preset(), Some(layout));
            interface.rails_right = true;
            interface.mode = Mode::Animate;
            assert_eq!(interface.matching_preset(), Some(layout));
            interface.radius = 15;
            assert_eq!(interface.matching_preset(), None);
        }
        let classic = Interface::default();
        assert_eq!(classic.layout, Layout::Classic);
        assert!(classic.palette.is_none() && !classic.custom_metrics());
    }

    #[test]
    fn choosing_a_preset_keeps_handedness_and_mode() {
        let mut interface = Interface {
            rails_right: true,
            mode: Mode::Draw,
            ..Interface::preset(Layout::Slate)
        };
        interface.choose_preset(Layout::Float);
        assert_eq!(interface.layout, Layout::Float);
        assert!(interface.rails_right);
        assert_eq!(interface.mode, Mode::Draw);
    }

    #[test]
    fn saved_interfaces_load_with_missing_and_unknown_fields() {
        let interface: Interface = serde_json::from_str(r#"{"layout":"Slate"}"#).unwrap();
        assert_eq!(interface.layout, Layout::Slate);
        assert_eq!(interface.density, Density::Regular);
        let round = serde_json::to_string(&Interface::preset(Layout::Atelier)).unwrap();
        let back: Interface = serde_json::from_str(&round).unwrap();
        assert_eq!(back, Interface::preset(Layout::Atelier));
    }

    #[test]
    fn style_text_reads_on_its_panels() {
        fn luminance(c: [u8; 3]) -> f32 {
            c.into_iter()
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
        }
        let contrast = |a, b| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for p in [Palette::slate(), Palette::float(), Palette::atelier()] {
            for surface in [p.base, p.panel, p.raised, p.sunken] {
                assert!(contrast(p.text, surface) >= 7.0, "{p:?}");
                assert!(contrast(p.muted, surface) >= 3.0, "{p:?}");
            }
            assert!(contrast(p.accent_text, p.accent) >= 4.5, "{p:?}");
            let chosen = mix(p.panel, p.accent, if p.dark { 0.26 } else { 0.16 });
            assert!(contrast(p.accent, chosen) >= 3.0, "{p:?}");
        }
    }
}
