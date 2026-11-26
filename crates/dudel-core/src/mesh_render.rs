//! Pictures of 3D models, drawn on the processor: bands of rows across the
//! cores, each with its own depth, so the result never depends on how many
//! cores there are.
use crate::mesh::{Mesh, Texture, add, cross, dot, scale, sub, unit};
use crate::rulers::Ruler;
use std::sync::{Arc, Mutex};

/// Output rows in one band, and triangles in one part.
const BAND: usize = 16;
const PART: usize = 4096;
const NEAR: f32 = 0.01;

/// Looks at `target` from `distance` away, turned by `yaw` around the model
/// and `pitch` above it, in radians. Models have a radius of 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: [f32; 3],
    /// The angle the picture's shorter side covers, in radians.
    pub field: f32,
    /// Parallel lines stay parallel.
    pub flat: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.3,
            distance: 3.6,
            target: [0.0; 3],
            field: 40f32.to_radians(),
            flat: false,
        }
    }
}

impl Camera {
    /// The directions to the right, up and back toward the viewer.
    pub fn axes(&self) -> [[f32; 3]; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let back = [sy * cp, sp, cy * cp];
        let right = [cy, 0.0, -sy];
        [right, cross(back, right), back]
    }
    /// Pixels for one unit at a distance of one (or anywhere, when flat).
    fn focal(&self, size: [usize; 2]) -> f32 {
        let half = size[0].min(size[1]) as f32 / 2.0;
        let focal = half / (self.field / 2.0).tan();
        if self.flat {
            focal / self.distance
        } else {
            focal
        }
    }
    /// Move the target so the model follows a drag of `by` pixels.
    pub fn pan(&mut self, by: [f32; 2], size: [usize; 2]) {
        let [right, up, _] = self.axes();
        let unit = if self.flat { 1.0 } else { self.distance } / self.focal(size);
        self.target = add(
            self.target,
            add(scale(right, -by[0] * unit), scale(up, by[1] * unit)),
        )
        .map(|v| v.clamp(-10.0, 10.0));
    }
    pub fn orbit(&mut self, by: [f32; 2]) {
        self.yaw = (self.yaw - by[0]).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + by[1]).clamp(-1.55, 1.55);
    }
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance / factor).clamp(0.05, 40.0);
    }
    /// The perspective ruler matching this view in a picture of `size`:
    /// where the model's left-right, front-back and up-down lines meet.
    pub fn ruler(&self, size: [usize; 2]) -> Option<Ruler> {
        if self.flat {
            return None;
        }
        // Farther than this, lines are parallel on any canvas.
        const FAR: f32 = 200_000.0;
        let axes = self.axes();
        let focal = self.focal(size);
        let middle = size.map(|s| s as f32 / 2.0);
        let meet = |direction: [f32; 3]| -> Option<[f32; 2]> {
            let [x, y, back] = axes.map(|axis| dot(axis, direction));
            let at = [middle[0] - focal * x / back, middle[1] + focal * y / back];
            at.iter().all(|v| v.abs() < FAR).then_some(at)
        };
        let (side, front) = (meet([1.0, 0.0, 0.0]), meet([0.0, 0.0, 1.0]));
        let vertical = meet([0.0, 1.0, 0.0]);
        match (side, front) {
            (Some(a), Some(b)) => Some(Ruler::Perspective {
                vanishing: [a, b, vertical.unwrap_or(middle)],
                three: vertical.is_some(),
            }),
            // Seen straight from a side: one point.
            (Some(center), None) | (None, Some(center)) => Some(Ruler::Focus { center }),
            (None, None) => vertical.map(|center| Ruler::Focus { center }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shading {
    #[default]
    Smooth,
    /// Two tones with a hard border, as in cel shading.
    Toon,
    /// The colors alone.
    Colors,
}

impl Shading {
    pub const ALL: [Shading; 3] = [Shading::Smooth, Shading::Toon, Shading::Colors];
    pub fn name(self) -> &'static str {
        match self {
            Shading::Smooth => "Smooth",
            Shading::Toon => "Two tones",
            Shading::Colors => "Colors only",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Look {
    pub shading: Shading,
    /// Where the light comes from, turned and raised like the camera.
    pub light: [f32; 2],
    /// Keep whole texels.
    pub sharp: bool,
    /// Show this in place of a material's own colors.
    pub paint: Option<Paint>,
    /// Plain light gray instead of the model's colors and textures.
    pub clay: bool,
}

#[derive(Clone, Debug)]
pub struct Paint {
    pub texture: Arc<Texture>,
    /// Only on this material; on all without one.
    pub material: Option<u16>,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            shading: Shading::Smooth,
            light: [0.2, 0.8],
            sharp: false,
            paint: None,
            clay: false,
        }
    }
}

#[derive(Clone, Copy)]
struct Corner {
    /// Right, up, and how far in front of the camera.
    view: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

fn between(a: &Corner, b: &Corner, t: f32) -> Corner {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    Corner {
        view: [0, 1, 2].map(|i| mix(a.view[i], b.view[i])),
        normal: [0, 1, 2].map(|i| mix(a.normal[i], b.normal[i])),
        uv: [0, 1].map(|i| mix(a.uv[i], b.uv[i])),
    }
}

/// A triangle on the picture, its values already divided by distance.
struct Piece {
    at: [[f32; 2]; 3],
    /// Larger is nearer, and straight across the picture.
    depth: [f32; 3],
    over: [f32; 3],
    uv: [[f32; 2]; 3],
    normal: [[f32; 3]; 3],
    area: f32,
    material: u16,
    level: u8,
}

struct Scene<'a> {
    mesh: &'a Mesh,
    look: &'a Look,
    camera: Camera,
    axes: [[f32; 3]; 3],
    focal: f32,
    /// In samples, not output pixels.
    size: [usize; 2],
    light: [f32; 3],
    samples: usize,
}

impl Scene<'_> {
    fn texture(&self, material: u16) -> Option<&Texture> {
        match &self.look.paint {
            Some(paint) if paint.material.is_none_or(|m| m == material) => Some(&paint.texture),
            _ if self.look.clay => None,
            _ => self.mesh.materials[material as usize].texture.as_deref(),
        }
    }
    fn color(&self, material: u16) -> [f32; 3] {
        match &self.look.paint {
            Some(paint) if paint.material.is_none_or(|m| m == material) => [255.0; 3],
            _ if self.look.clay => [205.0; 3],
            _ => self.mesh.materials[material as usize].color.map(f32::from),
        }
    }
    fn pieces(&self, range: std::ops::Range<usize>) -> Vec<Piece> {
        let mut out = Vec::new();
        for (triangle, &material) in self.mesh.triangles[range.clone()]
            .iter()
            .zip(&self.mesh.material[range])
        {
            let corner = triangle.map(|i| {
                let v = &self.mesh.vertices[i as usize];
                let at = sub(v.at, self.camera.target);
                Corner {
                    view: [
                        dot(self.axes[0], at),
                        dot(self.axes[1], at),
                        self.camera.distance - dot(self.axes[2], at),
                    ],
                    normal: v.normal,
                    uv: v.uv,
                }
            });
            let inside = corner.map(|c| self.camera.flat || c.view[2] >= NEAR);
            if inside == [true; 3] {
                self.push(&mut out, corner, material);
            } else if inside != [false; 3] {
                // Cut away what is behind the camera: 3 or 4 corners remain.
                let mut kept = Vec::with_capacity(4);
                for i in 0..3 {
                    let next = (i + 1) % 3;
                    let (a, b) = (&corner[i], &corner[next]);
                    if inside[i] {
                        kept.push(*a);
                    }
                    if inside[i] != inside[next] {
                        let mut cut = between(a, b, (NEAR - a.view[2]) / (b.view[2] - a.view[2]));
                        cut.view[2] = NEAR;
                        kept.push(cut);
                    }
                }
                for i in 1..kept.len() - 1 {
                    self.push(&mut out, [kept[0], kept[i], kept[i + 1]], material);
                }
            }
        }
        out
    }
    fn push(&self, out: &mut Vec<Piece>, corner: [Corner; 3], material: u16) {
        let flat = self.camera.flat;
        let over = corner.map(|c| if flat { 1.0 } else { 1.0 / c.view[2] });
        let depth = if flat {
            corner.map(|c| -c.view[2])
        } else {
            over
        };
        let at = [0, 1, 2].map(|i| {
            [
                self.size[0] as f32 / 2.0 + corner[i].view[0] * self.focal * over[i],
                self.size[1] as f32 / 2.0 - corner[i].view[1] * self.focal * over[i],
            ]
        });
        let [a, b, c] = at;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if !area.is_finite() || area.abs() < 1e-9 {
            return;
        }
        for axis in 0..2 {
            let low = a[axis].min(b[axis]).min(c[axis]);
            let high = a[axis].max(b[axis]).max(c[axis]);
            if high < 0.0 || low > self.size[axis] as f32 {
                return;
            }
        }
        let level = self.texture(material).map_or(0, |texture| {
            let [w, h] = texture.size().map(|v| v as f32);
            let [p, q, r] = corner.map(|c| c.uv);
            let uv = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
            texture.level(uv.abs() * w * h / area.abs())
        });
        out.push(Piece {
            at,
            depth,
            over,
            uv: [0, 1, 2].map(|i| corner[i].uv.map(|v| v * over[i])),
            normal: [0, 1, 2].map(|i| scale(corner[i].normal, over[i])),
            area,
            material,
            level: level as u8,
        });
    }
    fn shade(&self, piece: &Piece, weight: [f32; 3]) -> Option<[u8; 3]> {
        let blend = |value: [f32; 3]| dot(weight, value);
        let mut color = self.color(piece.material);
        if let Some(texture) = self.texture(piece.material) {
            let over = 1.0 / blend(piece.over);
            let uv = [0, 1].map(|c| blend([piece.uv[0][c], piece.uv[1][c], piece.uv[2][c]]) * over);
            let texel = texture.sample(uv, piece.level as usize, self.look.sharp);
            if texel[3] < 128.0 {
                return None;
            }
            color = [0, 1, 2].map(|c| color[c] * texel[c] / 255.0);
        }
        if self.look.shading != Shading::Colors {
            let mut normal = unit(
                [0, 1, 2]
                    .map(|c| blend([piece.normal[0][c], piece.normal[1][c], piece.normal[2][c]])),
            );
            // Both sides of a surface are lit, each as the side seen. Screen
            // rows go down, so a front face turns the other way there.
            if piece.area > 0.0 {
                normal = scale(normal, -1.0);
            }
            let lit = dot(normal, self.light);
            let light = match self.look.shading {
                Shading::Toon if lit > 0.1 => 1.0,
                Shading::Toon => 0.62,
                _ => {
                    let half = (lit * 0.5 + 0.5).clamp(0.0, 1.0);
                    0.38 + 0.62 * half * half
                }
            };
            color = scale(color, light);
        }
        Some(color.map(|v| v.round().clamp(0.0, 255.0) as u8))
    }
    /// Draw one band of output rows from the pieces that reach it.
    fn band(&self, index: usize, pieces: &[Vec<Piece>], reach: &[Vec<u32>], out: &mut [u8]) {
        let width = self.size[0];
        let top = index * BAND * self.samples;
        let rows = out.len() / 4 * self.samples * self.samples / width;
        let mut depth = vec![f32::NEG_INFINITY; width * rows];
        let mut color = vec![[0u8; 4]; width * rows];
        for (pieces, reach) in pieces.iter().zip(reach) {
            for &piece in reach {
                let piece = &pieces[piece as usize];
                self.fill(piece, top, rows, &mut depth, &mut color);
            }
        }
        let n = self.samples;
        if n == 1 {
            for (out, color) in out.chunks_exact_mut(4).zip(&color) {
                out.copy_from_slice(color);
            }
            return;
        }
        let across = width / n;
        let all = (n * n) as u32;
        for (at, out) in out.chunks_exact_mut(4).enumerate() {
            let (x, y) = (at % across, at / across);
            let (mut sum, mut covered) = ([0u32; 3], 0u32);
            for sy in 0..n {
                let row = (y * n + sy) * width + x * n;
                for c in &color[row..row + n] {
                    if c[3] != 0 {
                        covered += 1;
                        for i in 0..3 {
                            sum[i] += c[i] as u32;
                        }
                    }
                }
            }
            if covered > 0 {
                for (out, sum) in out.iter_mut().zip(sum) {
                    *out = (sum as f32 / covered as f32).round() as u8;
                }
                out[3] = ((covered * 255 + all / 2) / all) as u8;
            }
        }
    }
    fn fill(
        &self,
        piece: &Piece,
        top: usize,
        rows: usize,
        depth: &mut [f32],
        color: &mut [[u8; 4]],
    ) {
        let width = self.size[0];
        let [a, b, c] = piece.at;
        let low = |axis: usize| a[axis].min(b[axis]).min(c[axis]);
        let high = |axis: usize| a[axis].max(b[axis]).max(c[axis]);
        // Sample centers within [low, high]; `as` saturates huge numbers.
        let x0 = (low(0) - 0.5).ceil().max(0.0) as usize;
        let x1 = ((high(0) + 0.5).floor().max(0.0) as usize).min(width);
        let y0 = ((low(1) - 0.5).ceil().max(0.0) as usize).max(top);
        let y1 = ((high(1) + 0.5).floor().max(0.0) as usize).min(top + rows);
        if x0 >= x1 {
            return;
        }
        let inverse = 1.0 / piece.area;
        // Each corner's weight at a point: base + slope · x, for one row.
        let edge = |p: [f32; 2], q: [f32; 2], y: f32| {
            let slope = -(q[1] - p[1]) * inverse;
            let base = (q[0] - p[0]) * (y - p[1]) * inverse - slope * p[0];
            (base, slope)
        };
        for y in y0..y1 {
            let row_y = y as f32 + 0.5;
            let edges = [edge(b, c, row_y), edge(c, a, row_y), edge(a, b, row_y)];
            // The part of the row inside all three edges, one sample wider
            // for rounding; the weights decide.
            let (mut from, mut to) = (x0 as f32, x1 as f32);
            for (base, slope) in edges {
                if slope > 0.0 {
                    from = from.max(-base / slope - 1.5);
                } else if slope < 0.0 {
                    to = to.min(-base / slope + 1.5);
                }
            }
            let (start, end) = (from.max(0.0) as usize, (to.max(0.0) as usize).min(x1));
            let row = (y - top) * width;
            for x in start..end {
                let at = x as f32 + 0.5;
                let weight = edges.map(|(base, slope)| base + slope * at);
                if weight.iter().any(|w| *w < -1e-5) {
                    continue;
                }
                let near = dot(weight, piece.depth);
                if near > depth[row + x]
                    && let Some([r, g, b]) = self.shade(piece, weight)
                {
                    depth[row + x] = near;
                    color[row + x] = [r, g, b, 255];
                }
            }
        }
    }
}

/// The model in a picture of `size`, transparent around it, in straight
/// alpha. `samples` is 1, or 2 for smooth edges.
pub fn render(
    mesh: &Mesh,
    camera: &Camera,
    look: &Look,
    size: [usize; 2],
    samples: usize,
    check: &(dyn Fn() -> Result<(), String> + Sync),
) -> Result<Vec<u8>, String> {
    if size[0] == 0 || size[1] == 0 {
        return Ok(Vec::new());
    }
    let samples = samples.clamp(1, 4);
    let full = size.map(|s| s * samples);
    let scene = Scene {
        mesh,
        look,
        camera: *camera,
        axes: camera.axes(),
        focal: camera.focal(full),
        size: full,
        light: Camera {
            yaw: look.light[0],
            pitch: look.light[1],
            ..*camera
        }
        .axes()[2],
        samples,
    };
    check()?;
    let bands = size[1].div_ceil(BAND);
    let tall = (BAND * samples) as f32;
    let parts = mesh.triangles.len().div_ceil(PART);
    let failed = |_| "Drawing the model stopped unexpectedly".to_owned();
    let mut pieces = Vec::with_capacity(parts);
    // For each band, the pieces of every part that reach it.
    let mut reach: Vec<Vec<Vec<u32>>> = vec![Vec::with_capacity(parts); bands];
    for part in crate::parallel::run(parts, |part| {
        let start = part * PART;
        let pieces = scene.pieces(start..(start + PART).min(mesh.triangles.len()));
        let mut reach = vec![Vec::new(); bands];
        for (index, piece) in pieces.iter().enumerate() {
            let [a, b, c] = piece.at.map(|p| p[1]);
            let first = (a.min(b).min(c) / tall).floor().max(0.0) as usize;
            let last = ((a.max(b).max(c) / tall).floor().max(0.0) as usize).min(bands - 1);
            for band in reach.iter_mut().take(last + 1).skip(first) {
                band.push(index as u32);
            }
        }
        (pieces, reach)
    }) {
        let (part, reached) = part.map_err(failed)?;
        pieces.push(part);
        for (all, reached) in reach.iter_mut().zip(reached) {
            all.push(reached);
        }
    }
    check()?;
    let mut out = vec![0u8; size[0] * size[1] * 4];
    let rows: Vec<_> = out.chunks_mut(size[0] * BAND * 4).map(Mutex::new).collect();
    for band in crate::parallel::run(bands, |band| {
        check()?;
        let mut out = rows[band].lock().unwrap_or_else(|e| e.into_inner());
        scene.band(band, &pieces, &reach[band], &mut out);
        Ok::<(), String>(())
    }) {
        band.map_err(failed)??;
    }
    drop(rows);
    Ok(out)
}

/// The texture layout as thin lines on a transparent picture of `size`.
pub fn draw_layout(edges: &[[[f32; 2]; 2]], size: [usize; 2], color: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; size[0] * size[1] * 4];
    let (w, h) = (size[0] as f32, size[1] as f32);
    for [a, b] in edges {
        let (a, b) = ([a[0] * w, a[1] * h], [b[0] * w, b[1] * h]);
        let longest = (b[0] - a[0]).abs().max((b[1] - a[1]).abs());
        // A line far outside the picture still takes bounded time.
        let steps = longest.ceil().clamp(1.0, 65536.0) as usize;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = a[0] + (b[0] - a[0]) * t;
            let y = a[1] + (b[1] - a[1]) * t;
            // The right and bottom borders of the square stay in view.
            let x = if x == w { w - 1.0 } else { x.floor() };
            let y = if y == h { h - 1.0 } else { y.floor() };
            if x >= 0.0 && y >= 0.0 && x < w && y < h {
                let at = (y as usize * size[0] + x as usize) * 4;
                out[at..at + 4].copy_from_slice(&color);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Material, Vertex, sphere};

    fn draw(
        mesh: &Mesh,
        camera: &Camera,
        look: &Look,
        size: [usize; 2],
        samples: usize,
    ) -> Vec<u8> {
        render(mesh, camera, look, size, samples, &|| Ok(())).unwrap()
    }
    fn at(image: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        image[(y * width + x) * 4..][..4].try_into().unwrap()
    }
    fn front() -> Camera {
        Camera {
            yaw: 0.0,
            pitch: 0.0,
            ..Camera::default()
        }
    }
    fn colors() -> Look {
        Look {
            shading: Shading::Colors,
            sharp: true,
            ..Look::default()
        }
    }

    #[test]
    fn a_ball_fills_a_round_shape_without_holes_and_is_lit_from_the_light() {
        let (mesh, camera) = (sphere(24), Camera::default());
        let look = Look {
            light: [camera.yaw, 1.2],
            ..Look::default()
        };
        let image = draw(&mesh, &camera, &look, [200, 160], 1);
        let first = (0..200).find(|x| at(&image, 200, *x, 80)[3] > 0).unwrap();
        let radius = 100 - first;
        assert!((50..=75).contains(&radius), "{radius}");
        for y in 0..160 {
            for x in 0..200 {
                let away = (x as f32 + 0.5 - 100.0).hypot(y as f32 + 0.5 - 80.0);
                let alpha = at(&image, 200, x, y)[3];
                if away < radius as f32 - 3.0 {
                    assert_eq!(alpha, 255, "hole at {x} {y}");
                } else if away > radius as f32 + 3.0 {
                    assert_eq!(alpha, 0, "spill at {x} {y}");
                }
            }
        }
        let up = at(&image, 200, 100, 80 - radius + 8);
        let down = at(&image, 200, 100, 80 + radius - 8);
        assert!(up[0] > down[0] + 40, "{up:?} {down:?}");
        // Smooth edges have partial pixels.
        let smooth = draw(&mesh, &camera, &look, [200, 160], 2);
        assert!(smooth.chunks(4).any(|p| p[3] > 0 && p[3] < 255));
    }

    /// Two squares facing the front: a small near one, then a large far one.
    fn squares() -> Mesh {
        let mut mesh = Mesh::default();
        for (z, size, color) in [(0.3, 0.3, [255, 0, 0]), (-0.3, 0.8, [0, 0, 255])] {
            let base = mesh.vertices.len() as u32;
            for [x, y] in [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0f32]] {
                mesh.vertices.push(Vertex {
                    at: [x * size, y * size, z],
                    normal: [0.0, 0.0, 1.0],
                    uv: [x * 0.5 + 0.5, 0.5 - y * 0.5],
                });
            }
            mesh.triangles.push([base, base + 2, base + 1]);
            mesh.triangles.push([base, base + 3, base + 2]);
            mesh.material.extend([mesh.materials.len() as u16; 2]);
            mesh.materials.push(Material {
                color,
                ..Material::plain("square")
            });
        }
        mesh
    }

    #[test]
    fn nearer_surfaces_cover_farther_ones_in_both_projections() {
        for flat in [false, true] {
            let mut camera = Camera { flat, ..front() };
            let image = draw(&squares(), &camera, &colors(), [64, 64], 1);
            assert_eq!(at(&image, 64, 32, 32), [255, 0, 0, 255]);
            assert_eq!(at(&image, 64, 32, 16), [0, 0, 255, 255]);
            // From behind the order changes, and backs are drawn.
            camera.yaw = std::f32::consts::PI;
            let image = draw(&squares(), &camera, &colors(), [64, 64], 1);
            assert_eq!(at(&image, 64, 32, 32), [0, 0, 255, 255]);
        }
    }

    #[test]
    fn the_seen_side_is_lit_from_either_side() {
        for yaw in [0.0, std::f32::consts::PI] {
            let camera = Camera { yaw, ..front() };
            let look = Look {
                light: [yaw, 0.0],
                clay: true,
                ..Look::default()
            };
            let image = draw(&squares(), &camera, &look, [64, 64], 1);
            assert!(at(&image, 64, 32, 32)[0] > 195);
        }
    }

    #[test]
    fn a_painted_texture_shows_upright_and_only_on_its_material() {
        // Red and green over blue and white.
        let texels = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255; 4],
        ];
        let look = Look {
            paint: Some(Paint {
                texture: Arc::new(Texture::new([2, 2], texels.concat()).unwrap()),
                material: Some(1),
            }),
            ..colors()
        };
        let image = draw(&squares(), &front(), &look, [64, 64], 1);
        assert_eq!(at(&image, 64, 16, 16), [255, 0, 0, 255]);
        assert_eq!(at(&image, 64, 47, 16), [0, 255, 0, 255]);
        assert_eq!(at(&image, 64, 16, 47), [0, 0, 255, 255]);
        assert_eq!(at(&image, 64, 47, 47), [255; 4]);
        assert_eq!(at(&image, 64, 32, 32), [255, 0, 0, 255], "other material");
    }

    #[test]
    fn a_camera_inside_the_model_cuts_instead_of_breaking() {
        let mesh = sphere(16);
        let camera = Camera {
            distance: 0.05,
            ..Camera::default()
        };
        let look = Look::default();
        let image = draw(&mesh, &camera, &look, [80, 60], 2);
        assert!(image.chunks(4).all(|p| p[3] == 255));
        assert!(render(&mesh, &camera, &look, [80, 60], 1, &|| Err("stop".into())).is_err());
        assert!(draw(&mesh, &camera, &look, [0, 5], 1).is_empty());
        assert_eq!(draw(&mesh, &camera, &look, [1, 1], 2).len(), 4);
    }

    #[test]
    fn rulers_meet_where_the_models_lines_do() {
        let size = [800, 600];
        let camera = Camera::default();
        let Some(Ruler::Perspective { vanishing, three }) = camera.ruler(size) else {
            panic!("a perspective ruler");
        };
        assert!(three);
        // A level horizon; looking down, it is above the middle.
        assert!((vanishing[0][1] - vanishing[1][1]).abs() < 0.5);
        assert!(vanishing[0][1] < 300.0 && vanishing[2][1] > 300.0);
        // A line along an axis runs toward that axis's point.
        let axes = camera.axes();
        let focal = camera.focal(size);
        let project = |p: [f32; 3]| {
            let z = camera.distance - dot(axes[2], p);
            [
                400.0 + dot(axes[0], p) * focal / z,
                300.0 - dot(axes[1], p) * focal / z,
            ]
        };
        for (meet, axis) in vanishing.iter().zip([0, 2, 1]) {
            let mut direction = [0.0; 3];
            direction[axis] = 0.5;
            let (a, b) = (project([0.1; 3]), project(add([0.1; 3], direction)));
            let side = (b[0] - a[0]) * (meet[1] - a[1]) - (b[1] - a[1]) * (meet[0] - a[0]);
            let length = (meet[0] - a[0]).hypot(meet[1] - a[1]) * (b[0] - a[0]).hypot(b[1] - a[1]);
            assert!((side / length).abs() < 1e-3, "axis {axis}");
        }
        let level = Camera {
            pitch: 0.0,
            ..camera
        };
        assert!(matches!(
            level.ruler(size),
            Some(Ruler::Perspective { three: false, .. })
        ));
        assert!(matches!(front().ruler(size), Some(Ruler::Focus { .. })));
        let flat = Camera {
            flat: true,
            ..camera
        };
        assert_eq!(flat.ruler(size), None);
    }

    #[test]
    fn the_layout_is_drawn_inside_the_picture() {
        let edges = [[[0.0, 0.5], [1.0, 0.5]], [[-5.0, 0.0], [9.0, 9.0]]];
        let image = draw_layout(&edges, [16, 16], [1, 2, 3, 255]);
        assert_eq!(at(&image, 16, 3, 8), [1, 2, 3, 255]);
        assert_eq!(at(&image, 16, 15, 8), [1, 2, 3, 255]);
        assert_eq!(at(&image, 16, 3, 2), [0; 4]);
    }

    /// Run by hand: `cargo test --release -p dudel-core -- mesh_render
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn cost_of_a_large_model() {
        for rings in [224, 700] {
            let mesh = sphere(rings);
            let start = std::time::Instant::now();
            let texture = Texture::new([2048, 2048], vec![180; 2048 * 2048 * 4]).unwrap();
            let built = start.elapsed().as_secs_f64() * 1e3;
            println!("2048 px texture: {built:.1} ms");
            let look = Look {
                paint: Some(Paint {
                    texture: Arc::new(texture),
                    material: None,
                }),
                ..Look::default()
            };
            for (size, samples) in [
                ([800, 800], 1),
                ([800, 800], 2),
                ([1600, 1200], 1),
                ([1600, 1200], 2),
                ([4000, 4000], 2),
            ] {
                let mut times: Vec<f64> = (0..12)
                    .map(|_| {
                        let start = std::time::Instant::now();
                        std::hint::black_box(draw(&mesh, &Camera::default(), &look, size, samples));
                        start.elapsed().as_secs_f64() * 1e3
                    })
                    .collect();
                times.sort_by(f64::total_cmp);
                println!(
                    "{} triangles, {size:?} x{samples}: median {:.1} ms, worst {:.1} ms",
                    mesh.triangles.len(),
                    times[6],
                    times[11]
                );
            }
        }
    }
}
