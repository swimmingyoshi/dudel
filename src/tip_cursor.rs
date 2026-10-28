//! The brush cursor shows the tip's real shape: its outline for round and
//! square tips, the tip image itself for image tips. While drawing it sits
//! where the ink lands, at the size and angle pressure, direction or tilt
//! give the latest dab; erasing draws it dashed.
use crate::{Dudel, brush::Brush, egui, view::View};
use egui::{Color32, Pos2, Stroke, pos2, vec2};

/// Cached texture of the current image tip, keyed by its pixels.
#[derive(Default)]
pub(crate) struct TipTexture {
    key: u64,
    texture: Option<egui::TextureHandle>,
}

fn tip_key(brush: &Brush) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Some(image) = brush
        .image
        .as_ref()
        .filter(|_| brush.tip == crate::brush::Tip::Image)
    {
        (image.width, image.height).hash(&mut hasher);
        image.alpha.hash(&mut hasher);
    }
    hasher.finish()
}

/// Where and how to draw the tip.
pub(crate) struct Placement {
    pub center: Pos2,
    /// Size multiplier from pressure and dynamics.
    pub scale: f32,
    /// Extra clockwise degrees from direction, tilt or jitter.
    pub turn: f32,
    pub erasing: bool,
}

impl Dudel {
    /// The brush the cursor shows: the stroke being drawn, the eraser while
    /// the pen hovers flipped, otherwise the current brush.
    fn cursor_brush(&self) -> std::borrow::Cow<'_, Brush> {
        use std::borrow::Cow;
        if self.stroking
            && let Some(settings) = &self.stroke_settings
        {
            return Cow::Borrowed(&settings.brush);
        }
        if self.input.eraser_in_range() {
            return self.eraser_for_stroke();
        }
        Cow::Borrowed(&self.brush)
    }
    /// Keep the image tip's texture current; painting itself borrows immutably.
    pub(crate) fn prepare_tip_cursor(&mut self, ctx: &egui::Context) {
        let brush = self.cursor_brush();
        let brush = brush.as_ref();
        let Some(image) = brush
            .image
            .as_ref()
            .filter(|_| brush.tip == crate::brush::Tip::Image)
        else {
            self.tip_cursor.texture = None;
            return;
        };
        let key = tip_key(brush);
        if self.tip_cursor.texture.is_some() && self.tip_cursor.key == key {
            return;
        }
        // White with the tip's coverage as alpha, tinted when drawn.
        let pixels = image
            .alpha
            .iter()
            .map(|&a| Color32::from_white_alpha(a))
            .collect();
        let texture = ctx.load_texture(
            "brush tip cursor",
            egui::ColorImage::new([image.width, image.height], pixels),
            egui::TextureOptions::LINEAR,
        );
        self.tip_cursor = TipTexture {
            key,
            texture: Some(texture),
        };
    }
    /// The tip under the pointer, or at the latest dab while drawing.
    pub(crate) fn tip_placement(
        &self,
        pointer: Pos2,
        view: View,
    ) -> (Placement, std::borrow::Cow<'_, Brush>) {
        let erasing = self.tool == crate::tools::Tool::Eraser;
        if self.stroking
            && let Some(settings) = &self.stroke_settings
            && let Some(dab) = self.stroke.last_dab()
        {
            return (
                Placement {
                    center: view.to_screen(pos2(dab.x, dab.y)),
                    scale: settings.brush.size_factor(&dab),
                    turn: dab.rotation.unwrap_or(0.0),
                    erasing: settings.erase,
                },
                std::borrow::Cow::Borrowed(&settings.brush),
            );
        }
        (
            Placement {
                center: pointer,
                scale: 1.0,
                turn: 0.0,
                erasing: erasing || self.input.eraser_in_range(),
            },
            self.cursor_brush(),
        )
    }
    pub(crate) fn paint_tip_cursor(
        &self,
        painter: &egui::Painter,
        view: View,
        brush: &Brush,
        at: &Placement,
    ) {
        let (sin, cos) = at.turn.to_radians().sin_cos();
        let to_screen = |[x, y]: [f32; 2]| {
            let (x, y) = (x * at.scale, y * at.scale);
            at.center + view.offset_to_screen(vec2(x * cos - y * sin, x * sin + y * cos))
        };
        let outline: Vec<Pos2> = brush.outline().into_iter().map(to_screen).collect();
        let extent = outline
            .iter()
            .map(|p| (*p - at.center).length())
            .fold(0.0_f32, f32::max);
        // Too small to read as a shape: the crosshair alone marks the hotspot.
        if extent < 2.0 {
            return;
        }
        if let Some(texture) = &self.tip_cursor.texture
            && brush.tip == crate::brush::Tip::Image
            && outline.len() == 4
        {
            // The tip image itself is the cursor: a faint dark halo keeps it
            // readable on any artwork, then the paint color (gray to erase).
            let uv = [
                pos2(0.0, 0.0),
                pos2(1.0, 0.0),
                pos2(1.0, 1.0),
                pos2(0.0, 1.0),
            ];
            let quad = |grow: f32, color: Color32| {
                let mut mesh = egui::Mesh::with_texture(texture.id());
                for (point, uv) in outline.iter().zip(uv) {
                    let out = (*point - at.center).normalized() * grow;
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: *point + out,
                        uv,
                        color,
                    });
                }
                mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                egui::Shape::mesh(mesh)
            };
            let [r, g, b] = if at.erasing {
                [128, 128, 128]
            } else {
                self.paint_color()
            };
            painter.add(quad(1.5, Color32::from_black_alpha(70)));
            painter.add(quad(0.0, Color32::from_rgba_unmultiplied(r, g, b, 150)));
            return;
        }
        let mut closed = outline.clone();
        closed.push(outline[0]);
        let dark = Stroke::new(2.0_f32, Color32::from_black_alpha(110));
        let light = Stroke::new(1.0_f32, Color32::from_white_alpha(200));
        if at.erasing {
            for stroke in [dark, light] {
                painter.extend(egui::Shape::dashed_line(&closed, stroke, 5.0, 4.0));
            }
        } else {
            painter.add(egui::Shape::line(closed.clone(), dark));
            painter.add(egui::Shape::line(closed, light));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drawing_places_the_tip_on_the_latest_dab_with_its_live_size_and_turn() {
        let mut app = Dudel {
            doc: crate::document::Document::new(64, 64),
            ..Default::default()
        };
        app.brush.size = 20.0;
        app.brush.pressure_size = true;
        let view = View {
            origin: pos2(100.0, 100.0),
            zoom: 2.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(32.0, 32.0),
        };
        let (idle, brush) = app.tip_placement(pos2(10.0, 20.0), view);
        assert_eq!(
            (idle.center, idle.scale, brush.size),
            (pos2(10.0, 20.0), 1.0, 20.0)
        );
        let settings = crate::stroke_buffer::Settings {
            target: app.input_stroke_target(),
            brush: app.brush.clone(),
            color: [0; 3],
            erase: true,
            layer: 0,
            symmetry: Default::default(),
        };
        assert!(app.begin_canvas_stroke(settings));
        let sample = |x: f32, pressure: f32| crate::stroke::Sample {
            x,
            y: 32.0,
            pressure,
            pressure_known: true,
            ..Default::default()
        };
        for (x, pressure) in [(10.0, 0.2), (20.0, 0.4), (30.0, 0.5)] {
            let dabs = app.stroke.sample(sample(x, pressure), &app.brush.clone());
            app.deliver_canvas_dabs(dabs);
        }
        let (live, _) = app.tip_placement(pos2(0.0, 0.0), view);
        let dab = app.stroke.last_dab().unwrap();
        assert_eq!(live.center, view.to_screen(pos2(dab.x, dab.y)));
        assert!(
            live.scale < 0.9 && live.scale > 0.0,
            "pressure shrinks it: {}",
            live.scale
        );
        assert!(live.erasing);
        app.cancel_painted_stroke();
    }
}
