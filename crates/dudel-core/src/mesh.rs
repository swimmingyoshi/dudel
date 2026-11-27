//! 3D models to draw from and to preview textures on: what is loaded from a
//! GLB or OBJ file. Pictures of them come from `mesh_render`.
use std::{path::Path, sync::Arc};

pub const EXTENSIONS: [&str; 2] = ["glb", "obj"];
pub const MAX_FILE: u64 = 512 << 20;
pub const MAX_TRIANGLES: usize = 2_000_000;
const MAX_TEXTURE: u32 = 8192;

/// A picture on a model, with halved copies for showing it smaller.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    levels: Vec<([usize; 2], Vec<u8>)>,
}

impl Texture {
    /// `rgba` holds straight-alpha pixels, the top row first.
    pub fn new(size: [usize; 2], rgba: Vec<u8>) -> Option<Self> {
        if size[0] == 0 || size[1] == 0 || rgba.len() != size[0] * size[1] * 4 {
            return None;
        }
        let mut levels = vec![(size, rgba)];
        while let Some(([w, h], last)) = levels.last()
            && (*w > 1 || *h > 1)
            && levels.len() < 12
        {
            let next = [(w / 2).max(1), (h / 2).max(1)];
            let mut half = Vec::with_capacity(next[0] * next[1] * 4);
            for y in 0..next[1] {
                for x in 0..next[0] {
                    let (x0, y0) = (x * w / next[0], y * h / next[1]);
                    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
                    for c in 0..4 {
                        let at = |x: usize, y: usize| last[(y * w + x) * 4 + c] as u32;
                        half.push(
                            ((at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1) + 2) / 4) as u8,
                        );
                    }
                }
            }
            levels.push((next, half));
        }
        Some(Self { levels })
    }
    pub fn size(&self) -> [usize; 2] {
        self.levels[0].0
    }
    /// The full-size pixel at `x`, `y`, straight RGBA.
    pub fn texel(&self, x: usize, y: usize) -> [u8; 4] {
        let ([w, _], data) = &self.levels[0];
        data[(y * w + x) * 4..][..4].try_into().unwrap()
    }
    /// The level where one pixel covers about `texels` square texels.
    pub(crate) fn level(&self, texels: f32) -> usize {
        if !texels.is_finite() || texels <= 1.0 {
            return 0;
        }
        ((texels.log2() * 0.5).round() as usize).min(self.levels.len() - 1)
    }
    /// The color at `uv`, repeating outside 0 to 1. `sharp` keeps whole
    /// texels, as pixel art needs.
    pub(crate) fn sample(&self, uv: [f32; 2], level: usize, sharp: bool) -> [f32; 4] {
        let ([w, h], data) = &self.levels[level];
        let texel = |x: usize, y: usize| {
            let at = (y * w + x) * 4;
            [0, 1, 2, 3].map(|c| data[at + c] as f32)
        };
        let wrap = |v: f32, n: usize| (v.rem_euclid(n as f32) as usize).min(n - 1);
        let (x, y) = (uv[0] * *w as f32, uv[1] * *h as f32);
        if !x.is_finite() || !y.is_finite() {
            return texel(0, 0);
        }
        if sharp {
            return texel(wrap(x, *w), wrap(y, *h));
        }
        let (x, y) = (x - 0.5, y - 0.5);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (x0, y0) = (wrap(x.floor(), *w), wrap(y.floor(), *h));
        let (x1, y1) = ((x0 + 1) % w, (y0 + 1) % h);
        let (a, b, c, d) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
        [0, 1, 2, 3].map(|i| {
            (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// Multiplies the texture, or is the color without one.
    pub color: [u8; 3],
    pub texture: Option<Arc<Texture>>,
}

impl Material {
    pub fn plain(name: &str) -> Self {
        Self {
            name: name.into(),
            color: [205, 205, 205],
            texture: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub at: [f32; 3],
    pub normal: [f32; 3],
    /// The top left of the texture is 0, 0.
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub triangles: Vec<[u32; 3]>,
    /// One material index for each triangle.
    pub material: Vec<u16>,
    pub materials: Vec<Material>,
    /// Whether any part of the model has texture coordinates.
    pub has_uv: bool,
}

pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    a.map(|v| v * k)
}
pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Length 1, whatever the size: a tiny face's normal must not vanish.
pub(crate) fn unit(a: [f32; 3]) -> [f32; 3] {
    let most = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if most > 0.0 && most.is_finite() {
        let a = scale(a, 1.0 / most);
        let length = dot(a, a).sqrt();
        if length.is_finite() && length > 0.0 {
            return scale(a, 1.0 / length);
        }
    }
    [0.0, 1.0, 0.0]
}

impl Mesh {
    pub fn load(path: &Path) -> Result<Self, String> {
        let name = crate::image_paste::file_name(path);
        let size = std::fs::metadata(path)
            .map_err(|e| format!("{name}: {e}"))?
            .len();
        if size > MAX_FILE {
            return Err(format!("{name} is larger than 512 MB"));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
        match crate::image_paste::extension(path).as_deref() {
            Some("glb") => crate::mesh_import::glb(&bytes),
            Some("obj") => crate::mesh_import::obj(&bytes, path.parent()),
            Some("gltf") => Err("export it as one .glb file instead of .gltf".to_owned()),
            Some("fbx") => Err("FBX is not supported yet; export it as .glb or .obj".to_owned()),
            _ => Err("not a GLB or OBJ model".to_owned()),
        }
        .and_then(Mesh::checked)
        .map_err(|e| format!("{name}: {e}"))
    }

    pub(crate) fn checked(mut self) -> Result<Self, String> {
        if self.triangles.is_empty() {
            return Err("it has no surfaces".into());
        }
        if self.triangles.len() > MAX_TRIANGLES {
            return Err(format!("it has more than {MAX_TRIANGLES} triangles"));
        }
        if self.materials.is_empty() {
            self.materials.push(Material::plain("Surface"));
        }
        let (vertices, materials) = (self.vertices.len(), self.materials.len());
        if self.material.len() != self.triangles.len()
            || self
                .triangles
                .iter()
                .flatten()
                .any(|i| *i as usize >= vertices)
            || self.material.iter().any(|m| *m as usize >= materials)
        {
            return Err("its surfaces refer to missing data".into());
        }
        for vertex in &mut self.vertices {
            if !vertex.at.iter().all(|v| v.is_finite() && v.abs() < 1e15) {
                return Err("it has a point without a usable position".into());
            }
            if !vertex.uv.iter().all(|v| v.is_finite()) {
                vertex.uv = [0.0; 2];
            }
        }
        // Centered and at one size, so the camera never meets extreme numbers.
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for index in self.triangles.iter().flatten() {
            let at = self.vertices[*index as usize].at;
            for axis in 0..3 {
                low[axis] = low[axis].min(at[axis]);
                high[axis] = high[axis].max(at[axis]);
            }
        }
        let center = scale(add(low, high), 0.5);
        let radius = self
            .triangles
            .iter()
            .flatten()
            .map(|i| sub(self.vertices[*i as usize].at, center))
            .map(|away| dot(away, away).sqrt())
            .fold(0.0, f32::max);
        if radius <= 1e-12 {
            return Err("it has no size".into());
        }
        for vertex in &mut self.vertices {
            vertex.at = scale(sub(vertex.at, center), 1.0 / radius);
            vertex.normal = unit(vertex.normal);
        }
        Ok(self)
    }

    /// The edges of the texture layout, in texture coordinates, each once.
    /// Every triangle is moved into the square its middle is in.
    pub fn uv_edges(&self, material: Option<u16>) -> Vec<[[f32; 2]; 2]> {
        let mut seen = std::collections::HashSet::new();
        let mut edges = Vec::new();
        for (triangle, of) in self.triangles.iter().zip(&self.material) {
            if material.is_some_and(|m| m != *of) {
                continue;
            }
            let uv = triangle.map(|i| self.vertices[i as usize].uv);
            let tile = [0, 1].map(|c| ((uv[0][c] + uv[1][c] + uv[2][c]) / 3.0).floor());
            let uv = uv.map(|p| [p[0] - tile[0], p[1] - tile[1]]);
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let key = |p: [f32; 2]| p.map(|v| (v * 65536.0).round() as i64);
                let (ka, kb) = (key(uv[a]), key(uv[b]));
                if ka != kb && seen.insert(if ka < kb { (ka, kb) } else { (kb, ka) }) {
                    edges.push([uv[a], uv[b]]);
                }
            }
        }
        edges
    }
}

/// Decode a model's picture from a file's bytes, within the size limits.
pub(crate) fn decode_texture(bytes: &[u8]) -> Result<Arc<Texture>, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE);
    limits.max_image_height = Some(MAX_TEXTURE);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| format!("a texture could not be read: {e}"))?
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Texture::new(size, image.into_raw())
        .map(Arc::new)
        .ok_or_else(|| "a texture is empty".into())
}

/// A ball of `rings` × `rings` × 2 triangles with its layout, for tests and
/// for showing the window before a model is opened.
pub fn sphere(rings: usize) -> Mesh {
    let rings = rings.clamp(3, 900);
    let mut mesh = Mesh {
        materials: vec![Material::plain("Ball")],
        has_uv: true,
        ..Mesh::default()
    };
    for y in 0..=rings {
        for x in 0..=rings {
            let uv = [x as f32 / rings as f32, y as f32 / rings as f32];
            let (around, down) = (uv[0] * std::f32::consts::TAU, uv[1] * std::f32::consts::PI);
            let at = [
                around.sin() * down.sin(),
                down.cos(),
                around.cos() * down.sin(),
            ];
            mesh.vertices.push(Vertex { at, normal: at, uv });
        }
    }
    let row = rings as u32 + 1;
    for y in 0..rings as u32 {
        for x in 0..rings as u32 {
            let a = y * row + x;
            mesh.triangles.push([a, a + row, a + 1]);
            mesh.triangles.push([a + 1, a + row, a + row + 1]);
        }
    }
    mesh.material = vec![0; mesh.triangles.len()];
    mesh.checked().expect("a ball is a usable model")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaded_models_are_centered_one_size_and_never_refer_outside() {
        let mesh = sphere(8);
        let far = mesh
            .vertices
            .iter()
            .map(|v| dot(v.at, v.at).sqrt())
            .fold(0.0, f32::max);
        assert!((far - 1.0).abs() < 0.001);
        let mut broken = mesh.clone();
        broken.triangles[0][0] = 1_000_000;
        assert!(broken.checked().is_err());
        let mut broken = mesh.clone();
        broken.vertices[3].at[0] = f32::NAN;
        assert!(broken.checked().is_err());
        let mut broken = mesh.clone();
        broken.material[0] = 9;
        assert!(broken.checked().is_err());
        assert!(Mesh::default().checked().is_err());
    }

    #[test]
    fn layout_edges_come_once_and_inside_the_square() {
        let mut mesh = sphere(4);
        let edges = mesh.uv_edges(None);
        assert_eq!(edges.len(), 4 * 5 * 2 + 4 * 4);
        for vertex in &mut mesh.vertices {
            vertex.uv[0] += 3.0;
        }
        for edge in mesh.uv_edges(Some(0)) {
            assert!(edge.iter().all(|p| (-0.01..=1.01).contains(&p[0])));
        }
        assert!(mesh.uv_edges(Some(1)).is_empty());
    }

    #[test]
    fn textures_repeat_and_shrink() {
        let texture = Texture::new([2, 1], vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert_eq!(texture.sample([0.25, 0.5], 0, true)[0], 255.0);
        assert_eq!(texture.sample([1.25, -3.5], 0, true)[0], 255.0);
        assert_eq!(texture.sample([0.5, 0.5], 0, false)[0], 127.5);
        let level = texture.level(4.0);
        assert_eq!(texture.sample([0.9, 0.5], level, true)[0], 128.0);
        assert!(Texture::new([2, 2], vec![0; 4]).is_none());
    }

    #[test]
    fn other_formats_say_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        for (name, says) in [("a.fbx", ".glb"), ("a.gltf", ".glb"), ("a.txt", "GLB")] {
            let path = dir.path().join(name);
            std::fs::write(&path, "x").unwrap();
            let error = Mesh::load(&path).unwrap_err();
            assert!(error.starts_with(name) && error.contains(says), "{error}");
        }
    }
}
