//! Reading GLB (binary glTF 2.0) and OBJ files. Everything in a file is
//! checked before it is used: a damaged one gives an error, never a panic.
use crate::mesh::{MAX_TRIANGLES, Material, Mesh, Texture, Vertex, add, cross, dot, sub, unit};
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::Arc};

const MAX_NODES: usize = 100_000;
const MAX_VERTICES: usize = 3 * MAX_TRIANGLES;
const MAX_MATERIALS: usize = 10_000;
/// Extensions a file may require without changing the shapes read here.
const UNDERSTOOD: [&str; 6] = [
    "KHR_mesh_quantization",
    "KHR_materials_unlit",
    "KHR_materials_emissive_strength",
    "KHR_materials_specular",
    "KHR_materials_ior",
    "EXT_texture_webp",
];

/// Columns, as glTF stores them.
type Matrix = [[f32; 4]; 4];
const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for (column, out) in out.iter_mut().enumerate() {
        for (row, out) in out.iter_mut().enumerate() {
            *out = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    out
}
fn transform(m: &Matrix, p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}
/// Normals turn with the inverse transpose, up to a length.
fn transform_normal(m: &Matrix, n: [f32; 3]) -> [f32; 3] {
    let column = |i: usize| [m[i][0], m[i][1], m[i][2]];
    let (a, b, c) = (column(0), column(1), column(2));
    let (x, y, z) = (cross(b, c), cross(c, a), cross(a, b));
    // Mirroring turns the cross products around.
    let side = if dot(a, x) < 0.0 { -1.0 } else { 1.0 };
    unit([0, 1, 2].map(|r| side * (x[r] * n[0] + y[r] * n[1] + z[r] * n[2])))
}

fn number(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)?
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
}
fn floats<const N: usize>(value: &Value, key: &str) -> Option<[f32; N]> {
    let list = value.get(key)?.as_array()?;
    if list.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (out, value) in out.iter_mut().zip(list) {
        *out = value.as_f64()? as f32;
    }
    Some(out)
}
fn short(name: &str) -> String {
    name.chars().take(80).collect()
}
fn too_many() -> String {
    format!("it has more than {MAX_TRIANGLES} triangles")
}

/// Count, number format, bytes and stride.
type Data<'a> = (usize, usize, &'a [u8], Option<usize>);

struct Gltf<'a> {
    json: Value,
    binary: &'a [u8],
}

impl<'a> Gltf<'a> {
    fn item(&self, list: &str, index: usize) -> Result<&Value, String> {
        self.json
            .get(list)
            .and_then(|l| l.get(index))
            .ok_or_else(|| format!("{list} {index} is missing"))
    }
    /// A buffer view's bytes and the distance between its items, if given.
    fn view(&self, index: usize) -> Result<(&'a [u8], Option<usize>), String> {
        let view = self.item("bufferViews", index)?;
        if number(view, "buffer") != Some(0) || self.item("buffers", 0)?.get("uri").is_some() {
            return Err("its data is in another file".into());
        }
        let start = number(view, "byteOffset").unwrap_or(0);
        let bytes = number(view, "byteLength")
            .and_then(|length| start.checked_add(length))
            .and_then(|end| self.binary.get(start..end))
            .ok_or("its data is cut short")?;
        Ok((bytes, number(view, "byteStride")))
    }
    /// An accessor's count, number format and bytes from its start.
    fn data(&self, accessor: &Value) -> Result<Data<'a>, String> {
        if accessor.get("sparse").is_some() {
            return Err("sparse data is not supported".into());
        }
        let count = number(accessor, "count").unwrap_or(0);
        if count > MAX_VERTICES {
            return Err(too_many());
        }
        let kind = number(accessor, "componentType").unwrap_or(0);
        let view = number(accessor, "bufferView").ok_or("compressed data is not supported")?;
        let (bytes, stride) = self.view(view)?;
        let bytes = bytes
            .get(number(accessor, "byteOffset").unwrap_or(0)..)
            .ok_or("its data is cut short")?;
        Ok((count, kind, bytes, stride))
    }
    /// An accessor's values as `N` numbers each.
    fn accessor<const N: usize>(&self, index: usize) -> Result<Vec<[f32; N]>, String> {
        let accessor = self.item("accessors", index)?;
        let expected = if N == 2 { "VEC2" } else { "VEC3" };
        if accessor.get("type").and_then(Value::as_str) != Some(expected) {
            return Err("unexpected data layout".into());
        }
        let (count, kind, bytes, stride) = self.data(accessor)?;
        let width = match kind {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5126 => 4,
            _ => return Err("unknown number format".into()),
        };
        let normalized = accessor.get("normalized").and_then(Value::as_bool) == Some(true);
        let stride = stride.unwrap_or(N * width);
        if stride < N * width {
            return Err("unexpected data layout".into());
        }
        let mut out = Vec::new();
        for item in 0..count {
            let item = item
                .checked_mul(stride)
                .and_then(|at| bytes.get(at..at.checked_add(N * width)?))
                .ok_or("its data is cut short")?;
            let mut value = [0.0f32; N];
            for (value, b) in value.iter_mut().zip(item.chunks_exact(width)) {
                let (number, most) = match kind {
                    5120 => (b[0] as i8 as f32, 127.0),
                    5121 => (b[0] as f32, 255.0),
                    5122 => (i16::from_le_bytes([b[0], b[1]]) as f32, 32767.0),
                    5123 => (u16::from_le_bytes([b[0], b[1]]) as f32, 65535.0),
                    _ => (f32::from_le_bytes([b[0], b[1], b[2], b[3]]), 1.0),
                };
                *value = if normalized {
                    (number / most).max(-1.0)
                } else {
                    number
                };
            }
            out.push(value);
        }
        Ok(out)
    }
    fn indices(&self, index: usize) -> Result<Vec<u32>, String> {
        let accessor = self.item("accessors", index)?;
        let (count, kind, bytes, _) = self.data(accessor)?;
        let width = match kind {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => return Err("unknown number format".into()),
        };
        let bytes = bytes.get(..count * width).ok_or("its data is cut short")?;
        Ok(bytes
            .chunks_exact(width)
            .map(|b| b.iter().rev().fold(0, |n, b| n << 8 | u32::from(*b)))
            .collect())
    }
    fn texture(&self, index: usize) -> Result<Arc<Texture>, String> {
        let texture = self.item("textures", index)?;
        let source = number(texture, "source")
            .or_else(|| {
                number(
                    texture.get("extensions")?.get("EXT_texture_webp")?,
                    "source",
                )
            })
            .ok_or("a texture's format is not supported")?;
        let image = self.item("images", source)?;
        let view = number(image, "bufferView").ok_or("a texture is in another file")?;
        crate::mesh::decode_texture(self.view(view)?.0)
    }
    fn local(node: &Value) -> Matrix {
        if let Some(m) = floats::<16>(node, "matrix") {
            let mut out = IDENTITY;
            for (i, v) in m.into_iter().enumerate() {
                out[i / 4][i % 4] = v;
            }
            return out;
        }
        let t = floats::<3>(node, "translation").unwrap_or([0.0; 3]);
        let [x, y, z, w] = floats::<4>(node, "rotation").unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let s = floats::<3>(node, "scale").unwrap_or([1.0; 3]);
        let turned = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + z * w),
                2.0 * (x * z - y * w),
            ],
            [
                2.0 * (x * y - z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z + x * w),
            ],
            [
                2.0 * (x * z + y * w),
                2.0 * (y * z - x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ];
        let mut out = IDENTITY;
        for axis in 0..3 {
            for row in 0..3 {
                out[axis][row] = turned[axis][row] * s[axis];
            }
            out[3][axis] = t[axis];
        }
        out
    }
    fn add_mesh(&self, out: &mut Mesh, index: usize, place: &Matrix) -> Result<(), String> {
        let mesh = self.item("meshes", index)?;
        let primitives = mesh.get("primitives").and_then(Value::as_array);
        for primitive in primitives.into_iter().flatten() {
            let mode = number(primitive, "mode").unwrap_or(4);
            if !(4..=6).contains(&mode) {
                continue; // Points and lines have no surface.
            }
            if primitive
                .get("extensions")
                .is_some_and(|e| e.get("KHR_draco_mesh_compression").is_some())
            {
                return Err("compressed (Draco) models are not supported".into());
            }
            let attributes = primitive
                .get("attributes")
                .ok_or("a surface has no points")?;
            let at = self
                .accessor::<3>(number(attributes, "POSITION").ok_or("a surface has no points")?)?;
            let normal = number(attributes, "NORMAL")
                .map(|i| self.accessor::<3>(i))
                .transpose()?;
            let uv = number(attributes, "TEXCOORD_0")
                .map(|i| self.accessor::<2>(i))
                .transpose()?;
            let indices = match number(primitive, "indices") {
                Some(i) => self.indices(i)?,
                None => (0..at.len() as u32).collect(),
            };
            if normal.as_ref().is_some_and(|n| n.len() != at.len())
                || uv.as_ref().is_some_and(|n| n.len() != at.len())
                || indices.iter().any(|i| *i as usize >= at.len())
            {
                return Err("its surfaces refer to missing data".into());
            }
            let triangles: Vec<[u32; 3]> = match mode {
                4 => indices
                    .chunks_exact(3)
                    .map(|t| [t[0], t[1], t[2]])
                    .collect(),
                // Every second triangle of a strip is the other way around.
                5 => indices
                    .windows(3)
                    .enumerate()
                    .map(|(i, t)| {
                        if i % 2 == 0 {
                            [t[0], t[1], t[2]]
                        } else {
                            [t[1], t[0], t[2]]
                        }
                    })
                    .collect(),
                _ => indices
                    .windows(2)
                    .skip(1)
                    .map(|t| [indices[0], t[0], t[1]])
                    .collect(),
            };
            let plain = out.materials.len() - 1;
            let material = number(primitive, "material")
                .filter(|m| *m < plain)
                .unwrap_or(plain);
            let vertex = |i: usize| Vertex {
                at: transform(place, at[i]),
                normal: normal
                    .as_ref()
                    .map_or([0.0; 3], |n| transform_normal(place, n[i])),
                uv: uv.as_ref().map_or([0.0; 2], |uv| uv[i]),
            };
            let corners = if normal.is_some() {
                at.len()
            } else {
                triangles.len() * 3
            };
            if out.vertices.len() + corners > MAX_VERTICES
                || out.triangles.len() + triangles.len() > MAX_TRIANGLES
            {
                return Err(too_many());
            }
            out.has_uv |= uv.is_some();
            let base = out.vertices.len() as u32;
            if normal.is_some() {
                out.vertices.extend((0..at.len()).map(vertex));
                out.triangles
                    .extend(triangles.iter().map(|t| t.map(|i| i + base)));
            } else {
                // Without normals every face is flat, as glTF asks.
                for triangle in &triangles {
                    push_flat(out, triangle.map(|i| vertex(i as usize)));
                }
            }
            out.material
                .extend(std::iter::repeat_n(material as u16, triangles.len()));
        }
        Ok(())
    }
}

/// A triangle with corners of its own, facing straight out.
fn push_flat(out: &mut Mesh, corners: [Vertex; 3]) {
    let [a, b, c] = corners.map(|v| v.at);
    let normal = unit(cross(sub(b, a), sub(c, a)));
    let base = out.vertices.len() as u32;
    out.vertices
        .extend(corners.map(|corner| Vertex { normal, ..corner }));
    out.triangles.push([base, base + 1, base + 2]);
}

fn chunk(bytes: &[u8], at: usize) -> Option<(u32, &[u8])> {
    let word = |at: usize| {
        let b = bytes.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let length = word(at)? as usize;
    let start = at.checked_add(8)?;
    Some((word(at + 4)?, bytes.get(start..start.checked_add(length)?)?))
}

pub(crate) fn glb(bytes: &[u8]) -> Result<Mesh, String> {
    if bytes.get(..4) != Some(b"glTF") || bytes.get(4..8) != Some(&2u32.to_le_bytes()) {
        return Err("not a glTF 2 model".into());
    }
    let (kind, json) = chunk(bytes, 12).ok_or("it is cut short")?;
    if kind != u32::from_le_bytes(*b"JSON") {
        return Err("not a glTF 2 model".into());
    }
    let binary = chunk(bytes, 20 + json.len())
        .filter(|(kind, _)| *kind == u32::from_le_bytes(*b"BIN\0"))
        .map_or(&[][..], |(_, data)| data);
    let json: Value = serde_json::from_slice(json).map_err(|e| format!("it is damaged: {e}"))?;
    let gltf = Gltf { json, binary };
    let list = |key: &str| {
        gltf.json
            .get(key)
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
    };
    for name in list("extensionsRequired").iter().filter_map(Value::as_str) {
        if !UNDERSTOOD.contains(&name) {
            return Err(if name.contains("draco") || name.contains("meshopt") {
                "compressed models are not supported; export it without compression".into()
            } else {
                format!("it needs {name}, which is not supported")
            });
        }
    }
    let mut out = Mesh::default();
    let mut textures = HashMap::new();
    for (index, material) in list("materials").iter().enumerate().take(MAX_MATERIALS) {
        let mut read = Material::plain("");
        read.name = material
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| format!("Material {}", index + 1), short);
        let nothing = Value::Null;
        let pbr = material.get("pbrMetallicRoughness").unwrap_or(&nothing);
        let [r, g, b, _] = floats::<4>(pbr, "baseColorFactor").unwrap_or([1.0; 4]);
        read.color = [r, g, b].map(srgb);
        if let Some(texture) = pbr.get("baseColorTexture").and_then(|t| number(t, "index")) {
            // A picture that cannot be read leaves the plain color.
            read.texture = textures
                .entry(texture)
                .or_insert_with(|| gltf.texture(texture).ok())
                .clone();
        }
        out.materials.push(read);
    }
    // The last is for surfaces that name none.
    out.materials.push(Material::plain("Surface"));
    let children = |node: &Value, key: &str| -> Vec<usize> {
        node.get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_u64().and_then(|n| usize::try_from(n).ok()))
            .collect()
    };
    let count = list("nodes").len().min(MAX_NODES);
    let scene = number(&gltf.json, "scene").unwrap_or(0);
    let roots = match gltf.item("scenes", scene) {
        Ok(scene) => children(scene, "nodes"),
        Err(_) => (0..count).collect(),
    };
    let mut waiting: Vec<(usize, Matrix)> = roots.into_iter().map(|n| (n, IDENTITY)).collect();
    waiting.reverse();
    // A node has one parent, so each is visited once at most.
    let mut visited = vec![false; count];
    while let Some((index, parent)) = waiting.pop() {
        match visited.get_mut(index) {
            Some(seen) if !*seen => *seen = true,
            _ => continue,
        }
        let node = gltf.item("nodes", index)?;
        let place = multiply(&parent, &Gltf::local(node));
        if let Some(mesh) = number(node, "mesh") {
            gltf.add_mesh(&mut out, mesh, &place)?;
        }
        waiting.extend(children(node, "children").into_iter().map(|n| (n, place)));
    }
    let plain = out.materials.len() - 1;
    if plain > 0 && !out.material.contains(&(plain as u16)) {
        out.materials.pop();
    }
    Ok(out)
}

fn srgb(linear: f32) -> u8 {
    let v = linear.clamp(0.0, 1.0);
    let v = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round() as u8
}

/// A file named by a model, next to it or below; never anywhere else.
fn beside(folder: Option<&Path>, name: &str) -> Option<Vec<u8>> {
    let relative = std::path::PathBuf::from(name.trim().replace('\\', "/"));
    let mut parts = relative.components().peekable();
    parts.peek()?;
    if !parts.all(|c| matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    let path = folder?.join(relative);
    let file = std::fs::metadata(&path).ok().filter(|m| m.is_file())?;
    if file.len() > 128 << 20 {
        return None;
    }
    std::fs::read(path).ok()
}

/// What follows a line's first word.
fn rest(line: &str) -> &str {
    line.trim()
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim())
}

fn obj_materials(text: &str, folder: Option<&Path>, out: &mut Mesh) {
    let mut current = None;
    for line in text.lines() {
        let value = rest(line);
        match line.split_whitespace().next().unwrap_or("") {
            "newmtl" if out.materials.len() < MAX_MATERIALS => {
                out.materials.push(Material::plain(&short(value)));
                current = Some(out.materials.len() - 1);
            }
            "newmtl" => current = None,
            "Kd" => {
                let color: Vec<f32> = value
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if let ([r, g, b], Some(at)) = (&color[..], current)
                    && out.materials[at].texture.is_none()
                {
                    out.materials[at].color =
                        [*r, *g, *b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
            "map_Kd" => {
                // The name may have spaces; options before it do not matter.
                let texture = beside(folder, value)
                    .or_else(|| beside(folder, value.rsplit(char::is_whitespace).next()?))
                    .and_then(|bytes| crate::mesh::decode_texture(&bytes).ok());
                if let (Some(texture), Some(at)) = (texture, current) {
                    out.materials[at].texture = Some(texture);
                    out.materials[at].color = [255; 3];
                }
            }
            _ => {}
        }
    }
}

/// The material called `name`, made if it is new.
fn named(out: &mut Mesh, name: &str) -> u16 {
    let name = short(name);
    match out.materials.iter().position(|m| m.name == name) {
        Some(index) => index as u16,
        None if out.materials.len() < MAX_MATERIALS => {
            out.materials.push(Material::plain(&name));
            (out.materials.len() - 1) as u16
        }
        None => 0,
    }
}

type Corner = (usize, Option<usize>, Option<usize>);

/// `point/layout/normal`, counted from 1, or backward from the latest.
fn corner(word: &str, counts: [usize; 3]) -> Option<Corner> {
    let mut parts = word.split('/');
    let mut index = |count: usize| -> Option<usize> {
        let i = parts.next()?.parse::<i64>().ok()?;
        let at = if i < 0 {
            i64::try_from(count).ok()?.checked_add(i)?
        } else {
            i - 1
        };
        usize::try_from(at).ok().filter(|at| *at < count)
    };
    Some((index(counts[0])?, index(counts[1]), index(counts[2])))
}

pub(crate) fn obj(bytes: &[u8], folder: Option<&Path>) -> Result<Mesh, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Mesh::default();
    let (mut points, mut normals, mut uvs) = (Vec::new(), Vec::new(), Vec::new());
    let mut known: HashMap<Corner, u32> = HashMap::new();
    let mut material: Option<u16> = None;
    let mut corners: Vec<Corner> = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or("");
        let numbers = |n: usize| -> Result<[f32; 3], String> {
            let mut out = [0.0; 3];
            for (out, word) in out.iter_mut().zip(
                line.split_whitespace()
                    .skip(1)
                    .chain(std::iter::repeat(""))
                    .take(n),
            ) {
                *out = word.parse().map_err(|_| "a line could not be read")?;
            }
            Ok(out)
        };
        match key {
            "v" => points.push(numbers(3)?),
            "vn" => normals.push(numbers(3)?),
            "vt" => {
                let [u, v, _] = numbers(2)?;
                uvs.push([u, 1.0 - v]);
            }
            "mtllib" => {
                if let Some(bytes) = beside(folder, rest(line)) {
                    obj_materials(&String::from_utf8_lossy(&bytes), folder, &mut out);
                }
            }
            "usemtl" => material = Some(named(&mut out, rest(line))),
            "f" => {
                corners.clear();
                for word in words {
                    corners.push(
                        corner(word, [points.len(), uvs.len(), normals.len()])
                            .ok_or("a face refers to a missing point")?,
                    );
                }
                if corners.len() < 3 {
                    continue;
                }
                let current = *material.get_or_insert_with(|| named(&mut out, "Surface"));
                let make = |&(point, uv, normal): &Corner| Vertex {
                    at: points[point],
                    normal: normal.map_or([0.0; 3], |n| normals[n]),
                    uv: uv.map_or([0.0; 2], |t| uvs[t]),
                };
                out.has_uv |= corners.iter().any(|c| c.1.is_some());
                if out.triangles.len() + corners.len() - 2 > MAX_TRIANGLES {
                    return Err(too_many());
                }
                for i in 1..corners.len() - 1 {
                    let face = [corners[0], corners[i], corners[i + 1]];
                    if face.iter().all(|c| c.2.is_some()) {
                        let triangle = face.map(|corner| {
                            *known.entry(corner).or_insert_with(|| {
                                out.vertices.push(make(&corner));
                                out.vertices.len() as u32 - 1
                            })
                        });
                        out.triangles.push(triangle);
                    } else {
                        push_flat(&mut out, [0, 1, 2].map(|k| make(&face[k])));
                    }
                    out.material.push(current);
                }
            }
            _ => {}
        }
    }
    if normals.is_empty() {
        smooth(&mut out);
    }
    Ok(out)
}

/// Faces without normals get smooth ones from their neighbors, except where
/// they meet at a sharp angle.
fn smooth(mesh: &mut Mesh) {
    let mut around: HashMap<[u32; 3], Vec<u32>> = HashMap::new();
    for (i, v) in mesh.vertices.iter().enumerate() {
        around
            .entry(v.at.map(f32::to_bits))
            .or_default()
            .push(i as u32);
    }
    let flat: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.normal).collect();
    for shared in around.values() {
        // A point with a great many faces keeps its flat ones.
        if shared.len() > 64 {
            continue;
        }
        for &i in shared {
            let own = flat[i as usize];
            let sum = shared
                .iter()
                .map(|&j| flat[j as usize])
                .filter(|n| dot(*n, own) > 0.5)
                .fold([0.0; 3], add);
            mesh.vertices[i as usize].normal = unit(sum);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(json: &Value, data: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend((28 + json.len() as u32 + data.len() as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(json);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(data);
        out
    }

    /// One triangle with normals and a layout, placed by its nodes.
    fn sample(children: Value, normals: bool) -> Vec<u8> {
        let mut data = Vec::new();
        for v in [
            0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, // points
            0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, // normals
            0.0, 0.0, 1.0, 0.0, 0.0, 1.0, // layout
        ] {
            data.extend(v.to_le_bytes());
        }
        for i in [0u16, 1, 2, 0] {
            data.extend(i.to_le_bytes());
        }
        let mut attributes = serde_json::json!({"POSITION": 0, "TEXCOORD_0": 2});
        if normals {
            attributes["NORMAL"] = 1.into();
        }
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"children": children, "scale": [2.0, 2.0, 2.0]},
                {"mesh": 0, "translation": [1.0, 0.0, 0.0]}
            ],
            "meshes": [{"primitives": [{"attributes": attributes, "indices": 3, "material": 0}]}],
            "materials": [
                {"name": "Red", "pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.0, 0.0, 1.0]}}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 0, "byteOffset": 36, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 0, "byteOffset": 72, "componentType": 5126, "count": 3, "type": "VEC2"},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ],
            "bufferViews": [
                {"buffer": 0, "byteLength": 96},
                {"buffer": 0, "byteOffset": 96, "byteLength": 8}
            ],
            "buffers": [{"byteLength": data.len()}]
        });
        pack(&json, &data)
    }

    #[test]
    fn a_glb_is_read_with_its_nodes_material_and_layout() {
        for normals in [true, false] {
            let mesh = glb(&sample(serde_json::json!([1]), normals)).unwrap();
            assert_eq!(mesh.triangles, [[0, 1, 2]]);
            assert_eq!(mesh.vertices[0].at, [2.0, 0.0, 0.0]);
            assert_eq!(mesh.vertices[1].at, [4.0, 0.0, 0.0]);
            assert_eq!(mesh.vertices[2].uv, [0.0, 1.0]);
            assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
            assert_eq!(mesh.materials.len(), 1);
            assert_eq!(mesh.materials[0].color, [255, 0, 0]);
            assert!(mesh.has_uv);
        }
        // A loop in the node tree ends.
        let looped = glb(&sample(serde_json::json!([0, 1, 1]), true)).unwrap();
        assert_eq!(looped.triangles.len(), 1);
    }

    #[test]
    fn mirrored_parts_keep_their_normals_outward() {
        let mut m = IDENTITY;
        m[0][0] = -1.0;
        assert_eq!(transform_normal(&m, [0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]);
        assert_eq!(transform_normal(&m, [1.0, 0.0, 0.0]), [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn cut_or_damaged_files_give_errors_not_panics() {
        let whole = sample(serde_json::json!([1]), true);
        for length in 0..whole.len() {
            let _ = glb(&whole[..length]).and_then(Mesh::checked);
        }
        for at in 0..whole.len() {
            for value in [0, b'9', b'-', 0xFF] {
                let mut damaged = whole.clone();
                damaged[at] = value;
                let _ = glb(&damaged).and_then(Mesh::checked);
            }
        }
        let compressed = pack(
            &serde_json::json!({"extensionsRequired": ["KHR_draco_mesh_compression"]}),
            &[],
        );
        assert!(glb(&compressed).unwrap_err().contains("compress"));
    }

    #[test]
    fn an_obj_is_read_with_polygons_backward_indices_and_its_material() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("paint.mtl"), "newmtl Blue\nKd 0 0 1\n").unwrap();
        let text = "mtllib paint.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl Blue\nf 1/1 2/2 3/3 -1/-1\n";
        let mesh = obj(text.as_bytes(), Some(dir.path())).unwrap();
        assert_eq!(mesh.triangles.len(), 2);
        assert_eq!(mesh.materials[mesh.material[0] as usize].color, [0, 0, 255]);
        // OBJ's layout starts at the bottom.
        assert_eq!(mesh.vertices[0].uv, [0.0, 1.0]);
        assert!(mesh.vertices.iter().all(|v| v.normal[2] > 0.99));
        let normals = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 1 0\nf 1//1 2//1 3//1\nf 1//1 3//1 2//1\n";
        let mesh = obj(normals.as_bytes(), None).unwrap();
        assert_eq!((mesh.vertices.len(), mesh.triangles.len()), (3, 2));
        assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
        for bad in ["f 1 2 3", "v 0 0", "v 0 0 0\nf 1 2 9", "v a b c"] {
            assert!(obj(bad.as_bytes(), None).is_err(), "{bad}");
        }
        let extreme = format!("v 0 0 0\nf {} {} 0", i64::MIN, i64::MAX);
        assert!(obj(extreme.as_bytes(), None).is_err());
        assert!(obj(b"# nothing", None).unwrap().triangles.is_empty());
        // Files are only looked for beside the model.
        let inner = dir.path().join("sub");
        std::fs::create_dir(&inner).unwrap();
        for outside in ["../paint.mtl", "..\\paint.mtl", "/paint.mtl", ""] {
            assert!(beside(Some(&inner), outside).is_none(), "{outside}");
        }
        assert!(beside(Some(dir.path()), "paint.mtl").is_some());
    }
}
