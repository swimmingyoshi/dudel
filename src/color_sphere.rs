//! The Color sphere: the current color on a lit ball, to pick its shades,
//! from the highlight to the core shadow, as a painter would see them.
//! Picking a shade keeps the ball's color; choosing a color anywhere else
//! (the wheel, a swatch, the eyedropper) makes it the ball's new color.
use crate::egui;

/// Where the light comes from: up and to the left, toward the viewer.
const LIGHT: [f32; 3] = [-0.72, -0.55, 0.42];
/// Light bounced back from below and to the right.
const BOUNCE: [f32; 3] = [0.6, 0.65, 0.1];

/// The ball's color, its side in pixels, and whether the light is plain.
type PictureKey = ([u8; 3], usize, bool);

#[derive(Default)]
pub(crate) struct Sphere {
    /// The ball's color, once one was seen.
    base: Option<[u8; 3]>,
    /// The last shade picked here, and where on the ball.
    picked: Option<([u8; 3], [f32; 2])>,
    /// Plain light instead of warm light with cool shadows.
    neutral: bool,
    texture: Option<(PictureKey, egui::TextureHandle)>,
}

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let c = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / length)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The shade of `base` at `at` on the ball, from -1 to 1 across it (y down).
/// Points outside the ball take the nearest point on its edge.
pub(crate) fn shade(base: [u8; 3], at: [f32; 2], neutral: bool) -> [u8; 3] {
    let [mut x, mut y] = at;
    let r2 = x * x + y * y;
    if r2 > 1.0 {
        let r = r2.sqrt();
        (x, y) = (x / r, y / r);
    }
    let z = (1.0 - x * x - y * y).max(0.0).sqrt();
    let normal = [x, y, z];
    let light = normalized(LIGHT);
    // A soft terminator: light wraps a little past the halfway line.
    let diffuse = ((dot(normal, light) + 0.08) / 1.08).clamp(0.0, 1.0);
    let bounce = dot(normal, normalized(BOUNCE)).max(0.0).powi(2) * 0.22;
    let half = normalized([light[0], light[1], light[2] + 1.0]);
    let specular = dot(normal, half).max(0.0).powf(48.0) * 0.55;
    let (light_color, shadow_color) = if neutral {
        ([1.0, 1.0, 1.0], [1.0, 1.0, 1.0])
    } else {
        // Warm light leaves cool shadows.
        ([1.0, 0.93, 0.8], [0.68, 0.8, 1.3])
    };
    let base = base.map(to_linear);
    let ambient = 0.06;
    let mut out = [0.0; 3];
    for c in 0..3 {
        let lit = base[c] * (light_color[c] * diffuse * 1.04 + shadow_color[c] * ambient);
        let bounced = base[c] * base[c].sqrt() * bounce * shadow_color[c];
        out[c] = lit + bounced + specular * light_color[c];
    }
    out.map(to_srgb)
}

fn picture(base: [u8; 3], side: usize, neutral: bool) -> egui::ColorImage {
    let mut pixels = Vec::with_capacity(side * side);
    let half = side as f32 / 2.0;
    for row in 0..side {
        for column in 0..side {
            let x = (column as f32 + 0.5 - half) / half;
            let y = (row as f32 + 0.5 - half) / half;
            let distance = (x * x + y * y).sqrt();
            // One pixel of soft edge.
            let coverage = ((1.0 - distance) * half + 0.5).clamp(0.0, 1.0);
            let [r, g, b] = shade(base, [x, y], neutral);
            pixels.push(egui::Color32::from_rgba_unmultiplied(
                r,
                g,
                b,
                (coverage * 255.0).round() as u8,
            ));
        }
    }
    egui::ColorImage {
        size: [side, side],
        source_size: egui::vec2(side as f32, side as f32),
        pixels,
    }
}

impl Sphere {
    /// The ball, `size` points across. Returns a shade when one is picked.
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui, color: [u8; 3], size: f32) -> Option<[u8; 3]> {
        // A color chosen elsewhere becomes the ball's.
        if self.picked.is_none_or(|(picked, _)| picked != color) {
            self.base = Some(color);
            self.picked = None;
        }
        let base = self.base.unwrap_or(color);
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
        let side = ((size * ui.ctx().pixels_per_point()).round() as usize).clamp(16, 512);
        let key = (base, side, self.neutral);
        if self.texture.as_ref().is_none_or(|(k, _)| *k != key) {
            let image = picture(base, side, self.neutral);
            match &mut self.texture {
                Some((k, texture)) => {
                    texture.set(image, egui::TextureOptions::LINEAR);
                    *k = key;
                }
                None => {
                    let texture =
                        ui.ctx()
                            .load_texture("color sphere", image, egui::TextureOptions::LINEAR);
                    self.texture = Some((key, texture));
                }
            }
        }
        let painter = ui.painter_at(rect);
        if let Some((_, texture)) = &self.texture {
            let all = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
            painter.image(texture.id(), rect, all, egui::Color32::WHITE);
        }
        let mut chosen = None;
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let offset = (pointer - rect.center()) / (size / 2.0);
            let at = [offset.x, offset.y];
            let shade = shade(base, at, self.neutral);
            self.picked = Some((shade, at));
            chosen = Some(shade);
        }
        if let Some((shade, [x, y])) = self.picked {
            let mut at = egui::vec2(x, y);
            if at.length() > 1.0 {
                at = at.normalized();
            }
            let center = rect.center() + at * (size / 2.0);
            let dark = shade.iter().map(|c| *c as u32).sum::<u32>() > 380;
            let ring = if dark {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            };
            painter.circle_stroke(center, 5.0, egui::Stroke::new(1.5_f32, ring));
        }
        response.on_hover_text("Click or drag to pick a shade of this color");
        chosen
    }

    pub(crate) fn options_ui(&mut self, ui: &mut egui::Ui) {
        let mut warm = !self.neutral;
        if ui
            .checkbox(&mut warm, "Warm light, cool shadows")
            .on_hover_text("Shades lean warm in the light and cool in shadow, as in daylight")
            .changed()
        {
            self.neutral = !warm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brightness(c: [u8; 3]) -> u32 {
        c.iter().map(|v| *v as u32).sum()
    }

    #[test]
    fn the_light_side_is_brighter_and_shadows_lean_cool() {
        let base = [200, 80, 60];
        let lit = shade(base, [-0.35, -0.4], false);
        let core = shade(base, [0.5, 0.5], false);
        assert!(brightness(lit) > brightness(core) * 2, "{lit:?} {core:?}");
        // The highlight is near white; the shadow keeps the color's hue.
        let highlight = shade(base, [-0.72 * 0.5, -0.55 * 0.5], true);
        assert!(highlight.iter().all(|c| *c > 180), "{highlight:?}");
        assert!(core[0] > core[1], "{core:?}");
        // Cool shadows have relatively more blue than plain ones.
        let plain = shade(base, [0.5, 0.5], true);
        let blue = |c: [u8; 3]| c[2] as f32 / c[0].max(1) as f32;
        assert!(blue(core) > blue(plain), "{core:?} {plain:?}");
        // Outside the ball counts as its edge.
        assert_eq!(
            shade(base, [3.0, 0.0], false),
            shade(base, [1.0, 0.0], false)
        );
    }

    #[test]
    fn picking_keeps_the_ball_and_a_color_from_elsewhere_replaces_it() {
        let ctx = egui::Context::default();
        let mut sphere = Sphere::default();
        let run = |sphere: &mut Sphere, color: [u8; 3], click: Option<egui::Pos2>| {
            let mut input = egui::RawInput::default();
            if let Some(pos) = click {
                input.events = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ];
            }
            let mut picked = None;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    picked = sphere.ui(ui, color, 100.0);
                });
            });
            picked
        };
        let red = [200, 40, 40];
        run(&mut sphere, red, None);
        let at = egui::pos2(8.0 + 75.0, 8.0 + 75.0);
        let picked = run(&mut sphere, red, Some(at)).expect("a shade");
        assert_ne!(picked, red);
        run(&mut sphere, picked, None);
        assert_eq!(sphere.base, Some(red), "picking keeps the ball's color");
        run(&mut sphere, [10, 200, 10], None);
        assert_eq!(sphere.base, Some([10, 200, 10]));
    }
}
