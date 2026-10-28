use eframe::egui::{self, Color32, Pos2, Sense, Shape, Stroke, pos2, vec2};

/// HSV over stored sRGB values, matching the color pickers artists know.
/// Hue, saturation and value are all in 0..=1.
pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [u8; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|channel| ((channel + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

pub fn rgb_to_hsv(rgb: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    [hue, saturation, max]
}

/// Bring the edited HSV state in line with a color chosen elsewhere (swatch,
/// eyedropper, hex). Grays keep the previous hue so the wheel does not jump.
pub fn sync(hsv: &mut [f32; 3], rgb: [u8; 3]) {
    if hsv_to_rgb(*hsv) == rgb {
        return;
    }
    let next = rgb_to_hsv(rgb);
    if next[2] == 0.0 {
        hsv[2] = 0.0;
    } else if next[1] == 0.0 {
        hsv[1] = 0.0;
        hsv[2] = next[2];
    } else {
        *hsv = next;
    }
}

pub fn color32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn hex(rgb: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    let digits = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => digits.to_owned(),
        _ => return None,
    };
    let value = u32::from_str_radix(&digits, 16).ok()?;
    Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Ring,
    Square,
}

/// Hue ring around a saturation/value square. Returns true when changed.
pub fn wheel(ui: &mut egui::Ui, hsv: &mut [f32; 3], size: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let center = rect.center();
    let outer = size / 2.0 - 1.0;
    let ring = (size * 0.1).clamp(12.0, 22.0);
    let inner = outer - ring;
    let half = (inner - 6.0) / std::f32::consts::SQRT_2;
    let square = egui::Rect::from_center_size(center, vec2(half * 2.0, half * 2.0));
    let id = response.id;
    let mut changed = false;

    if let Some(pointer) = response.interact_pointer_pos()
        && (response.drag_started() || response.clicked() || response.is_pointer_button_down_on())
    {
        {
            let grab = ui
                .data(|d| d.get_temp::<Grab>(id))
                .filter(|_| !ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary)));
            let grab = grab.or_else(|| {
                let distance = (pointer - center).length();
                if distance >= inner - 3.0 && distance <= outer + 3.0 {
                    Some(Grab::Ring)
                } else if square.expand(6.0).contains(pointer) {
                    Some(Grab::Square)
                } else {
                    None
                }
            });
            if let Some(grab) = grab {
                ui.data_mut(|d| d.insert_temp(id, grab));
                let before = *hsv;
                match grab {
                    Grab::Ring => {
                        let v = pointer - center;
                        // Red at the top, hues increasing clockwise.
                        hsv[0] = (v.x.atan2(-v.y) / std::f32::consts::TAU).rem_euclid(1.0);
                    }
                    Grab::Square => {
                        hsv[1] = ((pointer.x - square.left()) / square.width()).clamp(0.0, 1.0);
                        hsv[2] =
                            1.0 - ((pointer.y - square.top()) / square.height()).clamp(0.0, 1.0);
                    }
                }
                changed = *hsv != before;
            }
        }
    }
    // Clicks do not necessarily emit drag_stopped. A grab belongs to one
    // pointer contact, otherwise a square click can trap later hue-ring clicks.
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.data_mut(|d| d.remove::<Grab>(id));
    }

    let painter = ui.painter_at(rect.expand(2.0));
    // Hue ring.
    let segments = 120;
    let mut mesh = egui::Mesh::default();
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let angle = t * std::f32::consts::TAU;
        let direction = vec2(angle.sin(), -angle.cos());
        let color = color32(hsv_to_rgb([t, 1.0, 1.0]));
        mesh.colored_vertex(center + direction * outer, color);
        mesh.colored_vertex(center + direction * inner, color);
        if i > 0 {
            let base = (i as u32 - 1) * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    painter.add(Shape::mesh(mesh));
    // Saturation (x) / value (y) square for the current hue.
    let steps = 16;
    let mut mesh = egui::Mesh::default();
    for row in 0..=steps {
        for column in 0..=steps {
            let s = column as f32 / steps as f32;
            let v = 1.0 - row as f32 / steps as f32;
            mesh.colored_vertex(
                pos2(
                    square.left() + s * square.width(),
                    square.top() + (1.0 - v) * square.height(),
                ),
                color32(hsv_to_rgb([hsv[0], s, v])),
            );
            if row > 0 && column > 0 {
                let a = ((row - 1) * (steps + 1) + column - 1) as u32;
                let b = a + 1;
                let c = a + steps as u32 + 1;
                let d = c + 1;
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(b, d, c);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
    // Markers with a two-tone outline so they read on any color.
    let marker = |painter: &egui::Painter, at: Pos2, radius: f32| {
        painter.circle_stroke(at, radius + 1.0, Stroke::new(1.5_f32, Color32::BLACK));
        painter.circle_stroke(at, radius, Stroke::new(2.0_f32, Color32::WHITE));
    };
    let angle = hsv[0] * std::f32::consts::TAU;
    marker(
        &painter,
        center + vec2(angle.sin(), -angle.cos()) * (inner + ring / 2.0),
        ring / 2.0 - 2.0,
    );
    marker(
        &painter,
        pos2(
            square.left() + hsv[1] * square.width(),
            square.top() + (1.0 - hsv[2]) * square.height(),
        ),
        5.0,
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separate_clicks_switch_between_square_and_hue_ring() {
        let ctx = egui::Context::default();
        let mut hsv = [0.0, 0.5, 0.5];
        let mut rect = egui::Rect::NOTHING;
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>, hsv: &mut [f32; 3]| {
            time += 0.05;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rect = egui::Rect::from_min_size(
                            ui.next_widget_position(),
                            vec2(200.0, 200.0),
                        );
                        wheel(ui, hsv, 200.0);
                    });
                },
            );
            rect
        };
        for _ in 0..4 {
            frame(vec![], &mut hsv);
        }
        let rect = frame(vec![], &mut hsv);
        let mut click = |pos: Pos2, hsv: &mut [f32; 3]| {
            for pressed in [true, false] {
                frame(
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            pressed,
                            button: egui::PointerButton::Primary,
                            modifiers: Default::default(),
                        },
                    ],
                    hsv,
                );
            }
        };
        click(rect.center(), &mut hsv);
        let square_sv = [hsv[1], hsv[2]];
        click(rect.center() + vec2(90.0, 0.0), &mut hsv);
        assert!(
            (hsv[0] - 0.25).abs() < 0.01,
            "a previous square click retained input mode: {hsv:?}"
        );
        assert_eq!([hsv[1], hsv[2]], square_sv);
        click(rect.center() + vec2(-20.0, -20.0), &mut hsv);
        assert!(
            (hsv[0] - 0.25).abs() < 0.01,
            "a previous ring click retained input mode"
        );
        assert_ne!([hsv[1], hsv[2]], square_sv);
    }
    #[test]
    fn hsv_round_trips_every_primary_and_a_range_of_mixed_colors() {
        for rgb in [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
            [12, 200, 140],
            [38, 44, 59],
            [128, 128, 128],
            [0, 0, 0],
            [255, 255, 255],
        ] {
            assert_eq!(hsv_to_rgb(rgb_to_hsv(rgb)), rgb, "{rgb:?}");
        }
        assert_eq!(hsv_to_rgb([0.0, 1.0, 1.0]), [255, 0, 0]);
        assert_eq!(hsv_to_rgb([1.0 / 3.0, 1.0, 1.0]), [0, 255, 0]);
    }
    #[test]
    fn sync_keeps_hue_for_grays_and_hex_parses_short_and_long_forms() {
        let mut hsv = [0.6, 0.8, 0.9];
        sync(&mut hsv, [128, 128, 128]);
        assert_eq!(hsv[0], 0.6);
        assert_eq!(hsv[1], 0.0);
        sync(&mut hsv, [0, 0, 0]);
        assert_eq!(hsv[0], 0.6);
        assert_eq!(parse_hex("#FfA"), Some([255, 255, 170]));
        assert_eq!(parse_hex("1a2B3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(parse_hex("12345"), None);
        assert_eq!(hex([1, 171, 255]), "01ABFF");
    }
}
