//! The canvas's paper, shown on the display. Paper is a height field that
//! brushes feel (`paper`); without this the canvas looks blank until paint
//! lands on it. Valleys are shaded and peaks lightened over the artwork, as
//! the paper's grain shows through real paint. Only the display shows it:
//! exports, thumbnails and color picks never include it.
//!
//! Tiles of the overlay are made from the same height function strokes use,
//! at document resolution, so the shading lines up with the grain paint
//! catches. Only visible tiles are made, a few per frame, and kept in a small
//! cache; zoomed far out the grain is too fine to see and nothing is drawn.
use crate::{egui, paper::Paper, view::View};
use std::collections::HashMap;

/// Document pixels per overlay tile side.
const SIDE: usize = 256;
/// Tiles kept; the least recently shown go first.
const MAX_TILES: usize = 160;
/// Tiles made per frame, so panning never stalls.
const PER_FRAME: usize = 6;

pub(crate) struct PaperView {
    /// Show the paper on the canvas (View › Paper texture).
    pub(crate) show: bool,
    paper: Option<Paper>,
    tiles: HashMap<(usize, usize), (egui::TextureHandle, u64)>,
    frame: u64,
}

impl Default for PaperView {
    fn default() -> Self {
        Self {
            show: true,
            paper: None,
            tiles: HashMap::new(),
            frame: 0,
        }
    }
}

/// How strongly the paper shows at a zoom: fading in from 20% to 70%.
fn strength(zoom: f32) -> f32 {
    ((zoom - 0.2) / 0.5).clamp(0.0, 1.0)
}

/// The overlay over one tile: dark where the paper dips, light on peaks.
fn image(paper: &Paper, rect: [usize; 4]) -> egui::ColorImage {
    let [left, top, right, bottom] = rect;
    let relief = 0.35 + 0.65 * paper.roughness;
    let mut pixels = Vec::with_capacity((right - left) * (bottom - top));
    for y in top..bottom {
        for x in left..right {
            let d = (0.5 - paper.height(x, y)) * 2.0 * relief;
            pixels.push(if d > 0.0 {
                egui::Color32::from_rgba_unmultiplied(48, 42, 34, (d * 40.0).min(255.0) as u8)
            } else {
                egui::Color32::from_rgba_unmultiplied(255, 255, 250, (-d * 24.0).min(255.0) as u8)
            });
        }
    }
    egui::ColorImage::new([right - left, bottom - top], pixels)
}

impl PaperView {
    /// Tile rectangles of a `size` document visible in `area`.
    fn visible(size: [usize; 2], area: egui::Rect, view: View) -> Vec<(usize, usize)> {
        let corners = [
            area.left_top(),
            area.right_top(),
            area.right_bottom(),
            area.left_bottom(),
        ]
        .map(|p| view.to_doc(p));
        let (mut lo, mut hi) = (corners[0], corners[0]);
        for c in corners {
            lo = lo.min(c);
            hi = hi.max(c);
        }
        let columns = size[0].div_ceil(SIDE);
        let rows = size[1].div_ceil(SIDE);
        let column = |v: f32| ((v.max(0.0) as usize) / SIDE).min(columns.saturating_sub(1));
        let row = |v: f32| ((v.max(0.0) as usize) / SIDE).min(rows.saturating_sub(1));
        if hi.x < 0.0 || hi.y < 0.0 || lo.x > size[0] as f32 || lo.y > size[1] as f32 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for ty in row(lo.y)..=row(hi.y) {
            for tx in column(lo.x)..=column(hi.x) {
                out.push((tx, ty));
            }
        }
        out
    }

    /// Make the visible tiles still missing, a few per frame. Call once per
    /// frame before drawing.
    pub(crate) fn prepare(
        &mut self,
        ctx: &egui::Context,
        paper: Option<Paper>,
        size: [usize; 2],
        area: egui::Rect,
        view: View,
    ) {
        if self.paper != paper {
            self.tiles.clear();
            self.paper = paper;
        }
        let Some(paper) = paper.filter(|_| self.show && strength(view.zoom) > 0.0) else {
            return;
        };
        self.frame += 1;
        let mut visible = Self::visible(size, area, view);
        // Nearest the middle of the view first.
        let middle = view.to_doc(area.center());
        visible.sort_by(|a, b| {
            let distance = |(x, y): (usize, usize)| {
                let cx = (x * SIDE + SIDE / 2) as f32 - middle.x;
                let cy = (y * SIDE + SIDE / 2) as f32 - middle.y;
                cx * cx + cy * cy
            };
            distance(*a).total_cmp(&distance(*b))
        });
        let mut made = 0;
        let mut missing = false;
        for key in visible {
            if let Some(tile) = self.tiles.get_mut(&key) {
                tile.1 = self.frame;
                continue;
            }
            if made == PER_FRAME {
                missing = true;
                continue;
            }
            let rect = [
                key.0 * SIDE,
                key.1 * SIDE,
                ((key.0 + 1) * SIDE).min(size[0]),
                ((key.1 + 1) * SIDE).min(size[1]),
            ];
            let options = egui::TextureOptions {
                magnification: egui::TextureFilter::Linear,
                minification: egui::TextureFilter::Linear,
                wrap_mode: egui::TextureWrapMode::ClampToEdge,
                mipmap_mode: Some(egui::TextureFilter::Linear),
            };
            let texture = ctx.load_texture(
                format!("paper {} {}", key.0, key.1),
                image(&paper, rect),
                options,
            );
            self.tiles.insert(key, (texture, self.frame));
            made += 1;
        }
        if self.tiles.len() > MAX_TILES {
            let mut ages: Vec<((usize, usize), u64)> = self
                .tiles
                .iter()
                .map(|(k, (_, used))| (*k, *used))
                .collect();
            ages.sort_by_key(|(_, used)| *used);
            for (key, _) in ages.into_iter().take(self.tiles.len() - MAX_TILES) {
                self.tiles.remove(&key);
            }
        }
        if missing {
            ctx.request_repaint();
        }
    }

    /// The overlay's shapes over the canvas tiles.
    pub(crate) fn shapes(
        &self,
        size: [usize; 2],
        area: egui::Rect,
        view: View,
    ) -> Vec<egui::Shape> {
        let alpha = strength(view.zoom);
        if !self.show || self.paper.is_none() || alpha <= 0.0 {
            return Vec::new();
        }
        let tint = egui::Color32::from_white_alpha((alpha * 255.0) as u8);
        Self::visible(size, area, view)
            .into_iter()
            .filter_map(|key| {
                let (texture, _) = self.tiles.get(&key)?;
                let left = (key.0 * SIDE) as f32;
                let top = (key.1 * SIDE) as f32;
                let right = ((key.0 + 1) * SIDE).min(size[0]) as f32;
                let bottom = ((key.1 + 1) * SIDE).min(size[1]) as f32;
                let corners = [
                    egui::pos2(left, top),
                    egui::pos2(right, top),
                    egui::pos2(right, bottom),
                    egui::pos2(left, bottom),
                ];
                let uvs = [
                    egui::pos2(0.0, 0.0),
                    egui::pos2(1.0, 0.0),
                    egui::pos2(1.0, 1.0),
                    egui::pos2(0.0, 1.0),
                ];
                let mut mesh = egui::Mesh::with_texture(texture.id());
                for (corner, uv) in corners.into_iter().zip(uvs) {
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: view.to_screen(corner),
                        uv,
                        color: tint,
                    });
                }
                mesh.indices = vec![0, 1, 2, 0, 2, 3];
                Some(egui::Shape::mesh(mesh))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(zoom: f32) -> View {
        View {
            origin: egui::pos2(400.0, 300.0),
            zoom,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(1000.0, 600.0),
        }
    }

    #[test]
    fn only_visible_tiles_are_made_a_few_at_a_time_and_follow_the_paper() {
        let ctx = egui::Context::default();
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let paper = Some(Paper::presets()[2].1);
        let mut shown = PaperView::default();
        let size = [2000, 1200];
        // At 100% the view shows a 800 x 600 window around the middle.
        let visible = PaperView::visible(size, area, view(1.0));
        assert!(visible.len() <= 4 * 4, "{visible:?}");
        shown.prepare(&ctx, paper, size, area, view(1.0));
        assert_eq!(shown.tiles.len(), PER_FRAME.min(visible.len()));
        for _ in 0..4 {
            shown.prepare(&ctx, paper, size, area, view(1.0));
        }
        assert_eq!(shown.tiles.len(), visible.len());
        assert_eq!(shown.shapes(size, area, view(1.0)).len(), visible.len());
        // Zoomed far out nothing is drawn or made.
        assert!(shown.shapes(size, area, view(0.1)).is_empty());
        // Another paper starts over; no paper shows nothing.
        shown.prepare(&ctx, Some(Paper::presets()[3].1), size, area, view(1.0));
        assert_eq!(shown.tiles.len(), PER_FRAME.min(visible.len()));
        shown.prepare(&ctx, None, size, area, view(1.0));
        assert!(shown.shapes(size, area, view(1.0)).is_empty());
    }

    #[test]
    fn the_overlay_shades_valleys_and_lightens_peaks() {
        let paper = Paper::presets()[2].1;
        let tile = image(&paper, [0, 0, 64, 64]);
        let mut dark = 0;
        let mut light = 0;
        for (i, p) in tile.pixels.iter().enumerate() {
            let h = paper.height(i % 64, i / 64);
            if h < 0.3 {
                assert!(p.r() < 60 && p.a() > 0);
                dark += 1;
            } else if h > 0.7 {
                assert!(p.r() > 0 && p.a() > 0);
                light += 1;
            }
        }
        assert!(dark > 0 && light > 0);
    }
}
