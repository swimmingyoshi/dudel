use serde::{Deserialize, Serialize};

/// Layer blend modes, saved by variant name. Formulas follow the W3C
/// Compositing and Blending definitions and the common painting-app set.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    LinearBurn,
    Subtract,
    DarkerColor,
    LinearDodge,
    LighterColor,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// Normal, but each pixel is fully shown or hidden at random in proportion
    /// to its alpha (Photoshop's Dissolve).
    Dissolve,
    /// Color dodge with the source weighted by its alpha and applied at full
    /// strength, so soft edges glow (CSP's Color Dodge (Glow)).
    GlowDodge,
    /// Add with the source weighted by its alpha and applied at full strength
    /// (CSP's Add (Glow)).
    AddGlow,
    /// Folders only: contents blend straight into the layers below instead of
    /// being combined first (Photoshop's group default, CSP's Through).
    PassThrough,
}

impl BlendMode {
    /// Menu order, in groups: normal, darken, lighten, contrast, comparative
    /// and component modes.
    pub const GROUPS: [&'static [Self]; 6] = [
        &[Self::Normal, Self::Dissolve],
        &[
            Self::Darken,
            Self::Multiply,
            Self::ColorBurn,
            Self::LinearBurn,
            Self::Subtract,
            Self::DarkerColor,
        ],
        &[
            Self::Lighten,
            Self::Screen,
            Self::ColorDodge,
            Self::GlowDodge,
            Self::LinearDodge,
            Self::AddGlow,
            Self::LighterColor,
        ],
        &[
            Self::Overlay,
            Self::SoftLight,
            Self::HardLight,
            Self::VividLight,
            Self::LinearLight,
            Self::PinLight,
            Self::HardMix,
        ],
        &[Self::Difference, Self::Exclusion, Self::Divide],
        &[Self::Hue, Self::Saturation, Self::Color, Self::Luminosity],
    ];
    /// Every mode a raster layer can use, in menu order. Folders may also use
    /// `PassThrough`.
    pub const ALL: [Self; 29] = [
        Self::Normal,
        Self::Dissolve,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::Subtract,
        Self::DarkerColor,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::GlowDodge,
        Self::LinearDodge,
        Self::AddGlow,
        Self::LighterColor,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::HardMix,
        Self::Difference,
        Self::Exclusion,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::Darken => "Darken",
            Self::Lighten => "Lighten",
            Self::ColorDodge => "Color dodge",
            Self::ColorBurn => "Color burn",
            Self::HardLight => "Hard light",
            Self::SoftLight => "Soft light",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::LinearBurn => "Linear burn",
            Self::Subtract => "Subtract",
            Self::DarkerColor => "Darker color",
            Self::LinearDodge => "Add (linear dodge)",
            Self::LighterColor => "Lighter color",
            Self::VividLight => "Vivid light",
            Self::LinearLight => "Linear light",
            Self::PinLight => "Pin light",
            Self::HardMix => "Hard mix",
            Self::Divide => "Divide",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
            Self::Dissolve => "Dissolve",
            Self::GlowDodge => "Color dodge (glow)",
            Self::AddGlow => "Add (glow)",
            Self::PassThrough => "Pass through",
        }
    }
    /// The separable mode a glow mode applies to its alpha-weighted source.
    pub fn glow_base(self) -> Option<Self> {
        match self {
            Self::GlowDodge => Some(Self::ColorDodge),
            Self::AddGlow => Some(Self::LinearDodge),
            _ => None,
        }
    }
    /// Menu order for this kind of entry: folders also offer Pass through.
    pub fn choices(folder: bool) -> impl Iterator<Item = Self> {
        folder
            .then_some(Self::PassThrough)
            .into_iter()
            .chain(Self::ALL)
    }
    /// The next (`step` > 0) or previous mode in menu order, stopping at the ends.
    pub fn stepped(self, step: i32, folder: bool) -> Self {
        let modes: Vec<Self> = Self::choices(folder).collect();
        let index = modes.iter().position(|mode| *mode == self).unwrap_or(0) as i32;
        modes[(index + step).clamp(0, modes.len() as i32 - 1) as usize]
    }
    /// Blend straight 0-255 colors: `source` over `backdrop`.
    pub fn pixel(self, backdrop: [f32; 3], source: [f32; 3]) -> [f32; 3] {
        let component = |set: fn([f32; 3], [f32; 3]) -> [f32; 3]| {
            let unit = |c: [f32; 3]| c.map(|v| v / 255.0);
            set(unit(backdrop), unit(source)).map(|v| (v * 255.0).clamp(0.0, 255.0))
        };
        match self {
            Self::Normal => source,
            Self::DarkerColor => {
                if lum(source) < lum(backdrop) {
                    source
                } else {
                    backdrop
                }
            }
            Self::LighterColor => {
                if lum(source) > lum(backdrop) {
                    source
                } else {
                    backdrop
                }
            }
            Self::Hue => component(|b, s| set_lum(set_sat(s, sat(b)), lum(b))),
            Self::Saturation => component(|b, s| set_lum(set_sat(b, sat(s)), lum(b))),
            Self::Color => component(|b, s| set_lum(s, lum(b))),
            Self::Luminosity => component(|b, s| set_lum(b, lum(s))),
            _ => [0, 1, 2].map(|c| self.channel(backdrop[c], source[c])),
        }
    }
    /// One channel of a separable mode, on straight 0-255 values. Component
    /// modes (hue, color, ...) need the whole pixel; see `pixel`.
    pub fn channel(self, backdrop: f32, source: f32) -> f32 {
        match self {
            Self::Normal
            | Self::Dissolve
            | Self::PassThrough
            | Self::DarkerColor
            | Self::LighterColor
            | Self::Hue
            | Self::Saturation
            | Self::Color
            | Self::Luminosity => source,
            Self::GlowDodge => Self::ColorDodge.channel(backdrop, source),
            Self::AddGlow => Self::LinearDodge.channel(backdrop, source),
            Self::Multiply => backdrop * source / 255.0,
            Self::Screen => 255.0 - (255.0 - backdrop) * (255.0 - source) / 255.0,
            Self::Darken => backdrop.min(source),
            Self::Lighten => backdrop.max(source),
            Self::Difference => (backdrop - source).abs(),
            Self::Exclusion => backdrop + source - 2.0 * backdrop * source / 255.0,
            Self::Overlay => Self::HardLight.channel(source, backdrop),
            Self::HardLight => {
                if source <= 127.5 {
                    2.0 * backdrop * source / 255.0
                } else {
                    255.0 - 2.0 * (255.0 - backdrop) * (255.0 - source) / 255.0
                }
            }
            Self::ColorDodge => {
                if backdrop == 0.0 {
                    0.0
                } else if source == 255.0 {
                    255.0
                } else {
                    (backdrop * 255.0 / (255.0 - source)).min(255.0)
                }
            }
            Self::ColorBurn => {
                if backdrop == 255.0 {
                    255.0
                } else if source == 0.0 {
                    0.0
                } else {
                    255.0 - ((255.0 - backdrop) * 255.0 / source).min(255.0)
                }
            }
            Self::SoftLight => {
                let b = backdrop / 255.0;
                let s = source / 255.0;
                let result = if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 {
                        ((16.0 * b - 12.0) * b + 4.0) * b
                    } else {
                        b.sqrt()
                    };
                    b + (2.0 * s - 1.0) * (d - b)
                };
                result * 255.0
            }
            Self::LinearBurn => (backdrop + source - 255.0).max(0.0),
            Self::Subtract => (backdrop - source).max(0.0),
            Self::LinearDodge => (backdrop + source).min(255.0),
            Self::VividLight => {
                if source <= 127.5 {
                    Self::ColorBurn.channel(backdrop, 2.0 * source)
                } else {
                    Self::ColorDodge.channel(backdrop, 2.0 * source - 255.0)
                }
            }
            Self::LinearLight => (backdrop + 2.0 * source - 255.0).clamp(0.0, 255.0),
            Self::PinLight => {
                if source <= 127.5 {
                    backdrop.min(2.0 * source)
                } else {
                    backdrop.max(2.0 * source - 255.0)
                }
            }
            Self::HardMix => {
                if backdrop + source >= 255.0 {
                    255.0
                } else {
                    0.0
                }
            }
            Self::Divide => {
                if source == 0.0 {
                    if backdrop == 0.0 { 0.0 } else { 255.0 }
                } else {
                    (backdrop * 255.0 / source).min(255.0)
                }
            }
        }
    }
}

// Non-separable helpers on 0-1 colors, as in the W3C blending specification.
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 && l - n > 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 && x - l > 0.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}
fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if x > n {
        c.map(|v| (v - n) * s / (x - n))
    } else {
        [0.0; 3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separable_modes_match_known_normalized_fixtures_and_edge_cases() {
        for (mode, expected) in [
            (BlendMode::Overlay, 0.375),
            (BlendMode::Darken, 0.25),
            (BlendMode::Lighten, 0.75),
            (BlendMode::ColorDodge, 1.0),
            (BlendMode::ColorBurn, 0.0),
            (BlendMode::HardLight, 0.625),
            (BlendMode::SoftLight, 0.375),
            (BlendMode::Difference, 0.5),
            (BlendMode::Exclusion, 0.625),
            (BlendMode::LinearBurn, 0.0),
            (BlendMode::Subtract, 0.0),
            (BlendMode::LinearDodge, 1.0),
            (BlendMode::LinearLight, 0.75),
            (BlendMode::PinLight, 0.5),
            (BlendMode::HardMix, 1.0),
            (BlendMode::Divide, 1.0 / 3.0),
            (BlendMode::VividLight, 0.5),
        ] {
            assert!(
                (mode.channel(63.75, 191.25) / 255.0 - expected).abs() < 0.0001,
                "{}: {}",
                mode.name(),
                mode.channel(63.75, 191.25) / 255.0
            );
        }
        assert_eq!(BlendMode::ColorDodge.channel(0.0, 255.0), 0.0);
        assert_eq!(BlendMode::ColorBurn.channel(255.0, 0.0), 255.0);
        assert_eq!(BlendMode::Divide.channel(0.0, 0.0), 0.0);
        assert_eq!(BlendMode::Divide.channel(10.0, 0.0), 255.0);
        for mode in BlendMode::ALL {
            for b in [0.0, 63.75, 127.5, 191.25, 255.0] {
                for s in [0.0, 63.75, 127.5, 191.25, 255.0] {
                    let value = mode.channel(b, s);
                    assert!(value.is_finite() && (0.0..=255.0).contains(&value));
                    let pixel = mode.pixel([b, s, 127.5], [s, 255.0 - b, b]);
                    assert!(
                        pixel
                            .iter()
                            .all(|v| v.is_finite() && (0.0..=255.0).contains(v)),
                        "{}",
                        mode.name()
                    );
                }
            }
        }
    }

    #[test]
    fn component_modes_take_hue_saturation_and_luminosity_from_the_right_layer() {
        let red = [255.0, 0.0, 0.0];
        let gray = [128.0, 128.0, 128.0];
        let blue = [0.0, 0.0, 255.0];
        let lum255 = |c: [f32; 3]| lum(c.map(|v| v / 255.0)) * 255.0;
        // Color over gray keeps the gray's brightness with the red hue.
        let colored = BlendMode::Color.pixel(gray, red);
        assert!((lum255(colored) - lum255(gray)).abs() < 0.6);
        assert!(colored[0] > colored[1] && colored[1] == colored[2]);
        // Luminosity over red takes the gray's brightness, keeping red's hue.
        let lit = BlendMode::Luminosity.pixel(red, gray);
        assert!((lum255(lit) - lum255(gray)).abs() < 0.6);
        assert!(lit[0] > lit[1]);
        // Saturation from gray removes color; hue from blue over red is blue.
        let flat = BlendMode::Saturation.pixel(red, gray);
        assert!((flat[0] - flat[1]).abs() < 0.6 && (flat[1] - flat[2]).abs() < 0.6);
        let hue = BlendMode::Hue.pixel(red, blue);
        assert!(hue[2] > hue[0] && hue[2] > hue[1]);
        // Darker/lighter color pick a whole pixel by luminosity.
        assert_eq!(BlendMode::DarkerColor.pixel(red, blue), blue);
        assert_eq!(BlendMode::LighterColor.pixel(red, blue), red);
        // Separable modes in `pixel` agree with `channel`.
        let mixed = BlendMode::Multiply.pixel([200.0, 100.0, 50.0], [128.0, 255.0, 0.0]);
        assert_eq!(
            mixed,
            [0, 1, 2]
                .map(|c| BlendMode::Multiply
                    .channel([200.0, 100.0, 50.0][c], [128.0, 255.0, 0.0][c]))
        );
    }

    #[test]
    fn menu_groups_list_every_mode_once_and_stepping_stops_at_the_ends() {
        let grouped: Vec<_> = BlendMode::GROUPS.iter().flat_map(|g| g.iter()).collect();
        assert_eq!(grouped.len(), BlendMode::ALL.len());
        assert!(grouped.iter().zip(&BlendMode::ALL).all(|(a, b)| **a == *b));
        assert_eq!(BlendMode::Normal.stepped(-1, false), BlendMode::Normal);
        assert_eq!(BlendMode::Normal.stepped(1, false), BlendMode::Dissolve);
        assert_eq!(
            BlendMode::Luminosity.stepped(1, false),
            BlendMode::Luminosity
        );
        assert_eq!(BlendMode::Screen.stepped(-1, false), BlendMode::Lighten);
        // Only folders offer Pass through, first in their list.
        assert!(!BlendMode::ALL.contains(&BlendMode::PassThrough));
        assert_eq!(BlendMode::Normal.stepped(-1, true), BlendMode::PassThrough);
        assert_eq!(BlendMode::PassThrough.stepped(1, true), BlendMode::Normal);
        assert_eq!(BlendMode::PassThrough.stepped(0, false), BlendMode::Normal);
    }
}
