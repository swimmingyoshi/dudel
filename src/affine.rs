//! Inverse raster sampling of a frozen selected fraction; canvas dimensions stay fixed.
use crate::{document::Document, pixels::Pixels, selection::Selection, selection_mask::Mask};
use std::sync::atomic::AtomicBool;
#[path = "paired_transform.rs"]
mod paired;
pub(crate) use paired::{Mirror, apply_pair, driver_frame};

/// x'=a*x+c*y+tx, y'=b*x+d*y+ty, in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Matrix(pub [f64; 6]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Sampling {
    Nearest,
    Bilinear,
    Bicubic,
}
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub scale_percent: [f64; 2],
    pub angle_degrees: f64,
    pub translation: [f64; 2],
    pub sampling: Sampling,
    /// Mirror the selection across its own vertical / horizontal axis before
    /// rotating. Older settings without it are unflipped.
    #[serde(default)]
    pub flip: [bool; 2],
    /// Free corners or a mesh. When set, it replaces scale, rotation,
    /// translation and flips; older settings have none.
    #[serde(default)]
    pub warp: Option<Warp>,
    /// Link the selected region to its reflected partner, preserving both artworks.
    #[serde(default)]
    pub mirror: Option<Mirror>,
    /// The driving half's bounds when a mirror-pair selection crosses its
    /// axis, so the box fits that half. Computed per transform session.
    #[serde(skip)]
    pub pair_frame: Option<Selection>,
}

/// Distort, perspective and mesh warp shapes, in document coordinates.
/// A mesh holds up to 64 points; settings are copied a few times per drag
/// frame and boxed in queued edits, so the size is not a concern.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Warp {
    /// Where the selection's top-left, top-right, bottom-right and
    /// bottom-left corners land; mapped with a true perspective homography.
    Corners([[f64; 2]; 4]),
    /// A grid of control points over the selection, interpolated smoothly.
    Mesh(Mesh),
}
/// Largest mesh grid per side.
pub const MESH_MAX: usize = 8;
/// Default mesh grid per side.
pub const MESH_DEFAULT: u8 = 4;

/// Mesh control points, `columns` x `rows` row by row, in a fixed-capacity
/// array so transform settings stay cheap to copy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mesh {
    columns: u8,
    rows: u8,
    points: [[f64; 2]; MESH_MAX * MESH_MAX],
}
impl Mesh {
    /// A grid with every point from `point(column, row)`; sides clamp to 2..=MESH_MAX.
    pub fn from_fn(columns: u8, rows: u8, point: impl Fn(usize, usize) -> [f64; 2]) -> Self {
        let (columns, rows) = (
            columns.clamp(2, MESH_MAX as u8),
            rows.clamp(2, MESH_MAX as u8),
        );
        let mut points = [[0.0; 2]; MESH_MAX * MESH_MAX];
        for row in 0..rows as usize {
            for column in 0..columns as usize {
                points[row * columns as usize + column] = point(column, row);
            }
        }
        Self {
            columns,
            rows,
            points,
        }
    }
    pub fn columns(&self) -> usize {
        self.columns as usize
    }
    pub fn rows(&self) -> usize {
        self.rows as usize
    }
    pub fn points(&self) -> &[[f64; 2]] {
        &self.points[..self.columns() * self.rows()]
    }
    pub fn points_mut(&mut self) -> &mut [[f64; 2]] {
        let n = self.columns() * self.rows();
        &mut self.points[..n]
    }
    pub fn at(&self, column: usize, row: usize) -> [f64; 2] {
        self.points[row * self.columns() + column]
    }
    /// The grid's outer corners: top-left, top-right, bottom-right, bottom-left.
    pub fn corners(&self) -> [[f64; 2]; 4] {
        let (c, r) = (self.columns() - 1, self.rows() - 1);
        [self.at(0, 0), self.at(c, 0), self.at(c, r), self.at(0, r)]
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct MeshRecord {
    columns: u8,
    rows: u8,
    points: Vec<[f64; 2]>,
}
impl serde::Serialize for Mesh {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        MeshRecord {
            columns: self.columns,
            rows: self.rows,
            points: self.points().to_vec(),
        }
        .serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for Mesh {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let record = MeshRecord::deserialize(deserializer)?;
        let sides = 2..=MESH_MAX as u8;
        if !sides.contains(&record.columns)
            || !sides.contains(&record.rows)
            || record.points.len() != record.columns as usize * record.rows as usize
        {
            return Err(serde::de::Error::custom("mesh grid size out of range"));
        }
        Ok(Mesh::from_fn(record.columns, record.rows, |column, row| {
            record.points[row * record.columns as usize + column]
        }))
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            scale_percent: [100.0; 2],
            angle_degrees: 0.0,
            translation: [0.0; 2],
            sampling: Sampling::Bilinear,
            flip: [false; 2],
            warp: None,
            mirror: None,
            pair_frame: None,
        }
    }
}
impl Settings {
    pub(crate) fn validate(self) -> Result<(), String> {
        if let Some(mirror) = self.mirror {
            mirror.validate()?;
        }
        if self
            .scale_percent
            .iter()
            .any(|v| !v.is_finite() || *v < 0.1 || *v > 10000.0)
            || !self.angle_degrees.is_finite()
            || self.angle_degrees.abs() > 360.0
            || self
                .translation
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 1000000.0)
        {
            return Err("Transform settings exceed their supported ranges".into());
        }
        let points: &[[f64; 2]] = match &self.warp {
            None => &[],
            Some(Warp::Corners(points)) => points,
            Some(Warp::Mesh(mesh)) => mesh.points(),
        };
        if points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 1.0e7)
        {
            return Err("Transform points exceed their supported range".into());
        }
        Ok(())
    }
    /// The selection rectangle the transform acts on.
    pub(crate) fn bounds(doc: &Document) -> Selection {
        doc.selection.unwrap_or(Selection {
            left: 0,
            top: 0,
            right: doc.width,
            bottom: doc.height,
        })
    }
    /// The box the handles, pivot and warps are laid over.
    pub(crate) fn frame(self, doc: &Document) -> Selection {
        self.pair_frame.unwrap_or_else(|| Self::bounds(doc))
    }
    /// How the selection maps: affine from scale/rotate/move, or the warp.
    pub(crate) fn mapping(self, doc: &Document) -> Result<Mapping, String> {
        self.validate()?;
        let bounds = self.frame(doc);
        match self.warp {
            None => Ok(Mapping::Affine(self.matrix(doc)?)),
            Some(Warp::Corners(corners)) => Ok(Mapping::Projective(Homography::rect_to_quad(
                bounds, corners,
            )?)),
            Some(Warp::Mesh(mesh)) => Ok(Mapping::Mesh { bounds, mesh }),
        }
    }
    /// Where the current transform puts the selection's corners (TL, TR,
    /// BR, BL), to start distorting from what is shown.
    pub(crate) fn corners(self, doc: &Document) -> Result<[[f64; 2]; 4], String> {
        let b = self.frame(doc);
        let mapping = self.mapping(doc)?;
        Ok([
            [b.left as f64, b.top as f64],
            [b.right as f64, b.top as f64],
            [b.right as f64, b.bottom as f64],
            [b.left as f64, b.bottom as f64],
        ]
        .map(|[x, y]| mapping.forward(b, x, y)))
    }
    /// The current transform sampled as a mesh grid, to start warping.
    /// Resampling an existing mesh at a new size keeps its shape closely.
    pub(crate) fn mesh(self, doc: &Document, columns: u8, rows: u8) -> Result<Mesh, String> {
        let b = self.frame(doc);
        let mapping = self.mapping(doc)?;
        let (w, h) = ((b.right - b.left) as f64, (b.bottom - b.top) as f64);
        let (columns, rows) = (
            columns.clamp(2, MESH_MAX as u8),
            rows.clamp(2, MESH_MAX as u8),
        );
        Ok(Mesh::from_fn(columns, rows, |column, row| {
            let u = column as f64 / (columns - 1) as f64;
            let v = row as f64 / (rows - 1) as f64;
            mapping.forward(b, b.left as f64 + u * w, b.top as f64 + v * h)
        }))
    }
    pub(crate) fn matrix(self, doc: &Document) -> Result<Matrix, String> {
        self.validate()?;
        let s = self.frame(doc);
        let [px, py] = [
            (s.left as f64 + s.right as f64) / 2.0,
            (s.top as f64 + s.bottom as f64) / 2.0,
        ];
        let angle = self.angle_degrees.rem_euclid(360.0);
        let (sin, cos) = if angle == 0.0 {
            (0.0, 1.0)
        } else if angle == 90.0 {
            (1.0, 0.0)
        } else if angle == 180.0 {
            (0.0, -1.0)
        } else if angle == 270.0 {
            (-1.0, 0.0)
        } else {
            angle.to_radians().sin_cos()
        };
        let [sx, sy] = [0, 1].map(|axis| {
            let scale = self.scale_percent[axis] / 100.0;
            if self.flip[axis] { -scale } else { scale }
        });
        let [a, b, c, d] = [cos * sx, sin * sx, -sin * sy, cos * sy];
        let matrix = Matrix([
            a,
            b,
            c,
            d,
            px - a * px - c * py + self.translation[0],
            py - b * px - d * py + self.translation[1],
        ]);
        matrix.validate()?;
        Ok(matrix)
    }
}
impl Matrix {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    pub(crate) fn validate(self) -> Result<(), String> {
        if self.0.iter().any(|v| !v.is_finite())
            || self.0[..4].iter().any(|v| v.abs() > 1.0e6)
            || self.0[4..].iter().any(|v| v.abs() > 1.0e9)
        {
            return Err("Affine coefficients must be finite and bounded".into());
        }
        let [a, b, c, d, _, _] = self.0;
        if (a * d - b * c).abs() < 1.0e-12 {
            return Err("Affine transform is singular or too close to singular".into());
        }
        Ok(())
    }
    pub(crate) fn point(self, x: f64, y: f64) -> [f64; 2] {
        let [a, b, c, d, tx, ty] = self.0;
        [a * x + c * y + tx, b * x + d * y + ty]
    }
    fn inverse(self) -> Self {
        let [a, b, c, d, tx, ty] = self.0;
        let det = a * d - b * c;
        Self([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * ty - d * tx) / det,
            (b * tx - a * ty) / det,
        ])
    }
}
/// A 3x3 perspective map, row-major: x' = (a x + b y + c) / (g x + h y + i).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Homography(pub [f64; 9]);
impl Homography {
    pub(crate) fn point(self, x: f64, y: f64) -> Option<[f64; 2]> {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        let w = g * x + h * y + i;
        (w.abs() > 1e-12).then(|| [(a * x + b * y + c) / w, (d * x + e * y + f) / w])
    }
    fn inverse(self) -> Option<Self> {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        (det.abs() > 1e-18).then(|| {
            Self([
                (e * i - f * h) / det,
                (c * h - b * i) / det,
                (b * f - c * e) / det,
                (f * g - d * i) / det,
                (a * i - c * g) / det,
                (c * d - a * f) / det,
                (d * h - e * g) / det,
                (b * g - a * h) / det,
                (a * e - b * d) / det,
            ])
        })
    }
    /// Maps rectangle `b` onto corners TL, TR, BR, BL, which must form a
    /// convex shape so the map stays one-to-one.
    pub(crate) fn rect_to_quad(b: Selection, q: [[f64; 2]; 4]) -> Result<Self, String> {
        let cross = |o: [f64; 2], p: [f64; 2], r: [f64; 2]| {
            (p[0] - o[0]) * (r[1] - o[1]) - (p[1] - o[1]) * (r[0] - o[0])
        };
        let turns: Vec<f64> = (0..4)
            .map(|k| cross(q[k], q[(k + 1) % 4], q[(k + 2) % 4]))
            .collect();
        if !(turns.iter().all(|t| *t > 1e-6) || turns.iter().all(|t| *t < -1e-6)) {
            return Err("Corners must form a convex shape".into());
        }
        let ([x0, y0], [x1, y1], [x2, y2], [x3, y3]) = (q[0], q[1], q[2], q[3]);
        // Unit square to quad (Heckbert), then the rectangle onto the square.
        let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
        let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
        let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
            (0.0, 0.0)
        } else {
            let den = dx1 * dy2 - dx2 * dy1;
            if den.abs() < 1e-12 {
                return Err("Corners are too close to a line".into());
            }
            ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
        };
        let square = [
            x1 - x0 + g * x1,
            x3 - x0 + h * x3,
            x0,
            y1 - y0 + g * y1,
            y3 - y0 + h * y3,
            y0,
            g,
            h,
            1.0,
        ];
        let (l, t) = (b.left as f64, b.top as f64);
        let (w, hgt) = (
            (b.right - b.left).max(1) as f64,
            (b.bottom - b.top).max(1) as f64,
        );
        // square * [[1/w, 0, -l/w], [0, 1/h, -t/h], [0, 0, 1]]
        let m = |row: usize| {
            let [p, q, r] = [square[row * 3], square[row * 3 + 1], square[row * 3 + 2]];
            [p / w, q / hgt, r - p * l / w - q * t / hgt]
        };
        let (r0, r1, r2) = (m(0), m(1), m(2));
        Ok(Self([
            r0[0], r0[1], r0[2], r1[0], r1[1], r1[2], r2[0], r2[1], r2[2],
        ]))
    }
}

/// Catmull-Rom through grid points, extending the ends linearly so the
/// border control points are reached exactly.
fn mesh_eval(mesh: &Mesh, u: f64, v: f64) -> [f64; 2] {
    let (columns, rows) = (mesh.columns() as isize, mesh.rows() as isize);
    let segment = |t: f64, count: isize| {
        let g = t.clamp(0.0, 1.0) * (count - 1) as f64;
        // The far edge stays in the last cell at t = 1, landing exactly on
        // the border control point.
        let i = (g.floor() as isize).min(count - 2);
        (i, g - i as f64)
    };
    let curve = |get: &dyn Fn(isize) -> [f64; 2], i: isize, t: f64| {
        let [w0, w1, w2, w3] = cubic_weights(t);
        let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        [0, 1].map(|k| w0 * p0[k] + w1 * p1[k] + w2 * p2[k] + w3 * p3[k])
    };
    let extend = |a: [f64; 2], b: [f64; 2]| [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]];
    let at = |row: isize, column: isize| -> [f64; 2] {
        let pick = |r: isize, c: isize| mesh.at(c as usize, r as usize);
        let in_row = |r: isize| -> [f64; 2] {
            if column < 0 {
                extend(pick(r, 0), pick(r, 1))
            } else if column >= columns {
                extend(pick(r, columns - 1), pick(r, columns - 2))
            } else {
                pick(r, column)
            }
        };
        if row < 0 {
            extend(in_row(0), in_row(1))
        } else if row >= rows {
            extend(in_row(rows - 1), in_row(rows - 2))
        } else {
            in_row(row)
        }
    };
    let (column, tu) = segment(u, columns);
    let (row, tv) = segment(v, rows);
    let along_row = |r: isize| curve(&|c| at(r, c), column, tu);
    curve(&along_row, row, tv)
}

/// How each selected pixel moves: affine, perspective or a smooth mesh.
/// Only the active transform holds one, so the mesh variant's size is fine.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mapping {
    Affine(Matrix),
    Projective(Homography),
    Mesh { bounds: Selection, mesh: Mesh },
}
/// Mesh warps are rasterised as this many triangle pairs per side.
const MESH_STEPS: usize = 48;
impl Mapping {
    /// Where source point (x, y) of selection `b` lands.
    pub(crate) fn forward(&self, b: Selection, x: f64, y: f64) -> [f64; 2] {
        match *self {
            Mapping::Affine(matrix) => matrix.point(x, y),
            Mapping::Projective(h) => h.point(x, y).unwrap_or([x, y]),
            Mapping::Mesh { mesh, .. } => {
                let (w, h) = (
                    (b.right - b.left).max(1) as f64,
                    (b.bottom - b.top).max(1) as f64,
                );
                mesh_eval(&mesh, (x - b.left as f64) / w, (y - b.top as f64) / h)
            }
        }
    }
    pub(crate) fn is_identity(&self) -> bool {
        matches!(self, Mapping::Affine(m) if *m == Matrix::IDENTITY)
    }
    /// Destination bounding box of source rectangle `b` (with sampling
    /// padding), before clipping to the canvas.
    pub(crate) fn target(&self, b: Selection, padding: f64) -> [f64; 4] {
        let (l, t, r, bt) = (
            b.left as f64 - padding,
            b.top as f64 - padding,
            b.right as f64 + padding,
            b.bottom as f64 + padding,
        );
        let mut points = vec![];
        match self {
            Mapping::Affine(_) | Mapping::Projective(_) => {
                for (x, y) in [(l, t), (r, t), (r, bt), (l, bt)] {
                    points.push(self.forward(b, x, y));
                }
            }
            Mapping::Mesh { .. } => {
                for k in 0..=MESH_STEPS {
                    let f = k as f64 / MESH_STEPS as f64;
                    for (x, y) in [
                        (l + (r - l) * f, t),
                        (l + (r - l) * f, bt),
                        (l, t + (bt - t) * f),
                        (r, t + (bt - t) * f),
                    ] {
                        points.push(self.forward(b, x, y));
                    }
                }
                // Interior control points can bulge past the border.
                if let Mapping::Mesh { mesh, .. } = self {
                    points.extend(mesh.points().iter().copied());
                }
            }
        }
        points.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |a, p| {
                [
                    a[0].min(p[0]),
                    a[1].min(p[1]),
                    a[2].max(p[0]),
                    a[3].max(p[1]),
                ]
            },
        )
    }
}

/// Source position for each destination pixel of a region.
pub(crate) enum Inverse {
    Affine(Matrix),
    Projective(Homography),
    /// Per-pixel source coordinates over `rect`; NaN where nothing lands.
    Lookup {
        rect: Selection,
        coords: Vec<[f32; 2]>,
    },
}
impl Inverse {
    pub(crate) fn new(mapping: &Mapping, b: Selection, rect: Selection) -> Result<Self, String> {
        Self::new_control(mapping, b, rect, None)
    }
    pub(crate) fn new_control(
        mapping: &Mapping,
        b: Selection,
        rect: Selection,
        cancel: Option<&AtomicBool>,
    ) -> Result<Self, String> {
        crate::fill::check_cancel(cancel)?;
        Ok(match *mapping {
            Mapping::Affine(matrix) => Inverse::Affine(matrix.inverse()),
            Mapping::Projective(h) => Inverse::Projective(
                h.inverse()
                    .ok_or("Perspective transform is too close to singular")?,
            ),
            Mapping::Mesh { .. } => {
                let (rw, rh) = (rect.right - rect.left, rect.bottom - rect.top);
                let mut coords = vec![[f32::NAN; 2]; rw * rh];
                let (w, h) = (
                    (b.right - b.left).max(1) as f64,
                    (b.bottom - b.top).max(1) as f64,
                );
                let n = MESH_STEPS;
                let source = |i: usize, j: usize| {
                    [
                        b.left as f64 + w * i as f64 / n as f64,
                        b.top as f64 + h * j as f64 / n as f64,
                    ]
                };
                let lattice: Vec<[f64; 2]> = (0..=n)
                    .flat_map(|j| (0..=n).map(move |i| (i, j)))
                    .map(|(i, j)| {
                        let [x, y] = source(i, j);
                        mapping.forward(b, x, y)
                    })
                    .collect();
                let at = |i: usize, j: usize| lattice[j * (n + 1) + i];
                for j in 0..n {
                    crate::fill::check_cancel(cancel)?;
                    for i in 0..n {
                        for triangle in [
                            [(i, j), (i + 1, j), (i + 1, j + 1)],
                            [(i, j), (i + 1, j + 1), (i, j + 1)],
                        ] {
                            let d = triangle.map(|(i, j)| at(i, j));
                            let s = triangle.map(|(i, j)| source(i, j));
                            rasterize(d, s, rect, &mut coords, cancel)?;
                        }
                    }
                }
                Inverse::Lookup { rect, coords }
            }
        })
    }
    #[inline]
    pub(crate) fn source(&self, x: usize, y: usize) -> [f64; 2] {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let outside = [-1.0e9, -1.0e9];
        match self {
            Inverse::Affine(inverse) => inverse.point(px, py),
            Inverse::Projective(inverse) => inverse.point(px, py).unwrap_or(outside),
            Inverse::Lookup { rect, coords } => {
                let c = coords[(y - rect.top) * (rect.right - rect.left) + (x - rect.left)];
                if c[0].is_nan() {
                    outside
                } else {
                    [c[0] as f64, c[1] as f64]
                }
            }
        }
    }
}

/// Fill the source coordinates of destination triangle `d` (source `s`)
/// for pixel centres inside it.
fn rasterize(
    d: [[f64; 2]; 3],
    s: [[f64; 2]; 3],
    rect: Selection,
    coords: &mut [[f32; 2]],
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    let area =
        (d[1][0] - d[0][0]) * (d[2][1] - d[0][1]) - (d[2][0] - d[0][0]) * (d[1][1] - d[0][1]);
    if area.abs() < 1e-12 {
        return Ok(());
    }
    let min_x = d
        .iter()
        .map(|p| p[0])
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(rect.left as f64) as usize;
    let max_x = d
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min(rect.right as f64) as usize;
    let min_y = d
        .iter()
        .map(|p| p[1])
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(rect.top as f64) as usize;
    let max_y = d
        .iter()
        .map(|p| p[1])
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min(rect.bottom as f64) as usize;
    let width = rect.right - rect.left;
    for y in min_y..max_y {
        crate::fill::check_cancel(cancel)?;
        for x in min_x..max_x {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let w1 = ((px - d[0][0]) * (d[2][1] - d[0][1]) - (d[2][0] - d[0][0]) * (py - d[0][1]))
                / area;
            let w2 = ((d[1][0] - d[0][0]) * (py - d[0][1]) - (px - d[0][0]) * (d[1][1] - d[0][1]))
                / area;
            let w0 = 1.0 - w1 - w2;
            let edge = -1e-9;
            if w0 >= edge && w1 >= edge && w2 >= edge {
                let sx = w0 * s[0][0] + w1 * s[1][0] + w2 * s[2][0];
                let sy = w0 * s[0][1] + w1 * s[1][1] + w2 * s[2][1];
                coords[(y - rect.top) * width + (x - rect.left)] = [sx as f32, sy as f32];
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Coverage<'a> {
    Mask(&'a Mask),
    Rectangle(Selection),
}
impl Coverage<'_> {
    fn at(self, x: usize, y: usize) -> u8 {
        match self {
            Self::Mask(mask) => mask.at(x, y),
            Self::Rectangle(s) => {
                if x >= s.left && x < s.right && y >= s.top && y < s.bottom {
                    255
                } else {
                    0
                }
            }
        }
    }
    fn bounds(self) -> Option<Selection> {
        match self {
            Self::Mask(mask) => mask.bounds(),
            Self::Rectangle(s) => (s.left < s.right && s.top < s.bottom).then_some(s),
        }
    }
}
impl<'a> From<&'a Mask> for Coverage<'a> {
    fn from(mask: &'a Mask) -> Self {
        Self::Mask(mask)
    }
}
fn tap(
    pixels: &Pixels,
    mask: Coverage<'_>,
    width: usize,
    height: usize,
    x: f64,
    y: f64,
) -> ([u8; 4], u8) {
    if x < 0.0 || y < 0.0 || x >= width as f64 || y >= height as f64 {
        return ([0; 4], 0);
    }
    let (x, y) = (x as usize, y as usize);
    (pixels.rgba(y * width + x), mask.at(x, y))
}
fn sample(
    pixels: &Pixels,
    mask: Coverage<'_>,
    width: usize,
    height: usize,
    [x, y]: [f64; 2],
    sampling: Sampling,
) -> ([u8; 4], u8) {
    if matches!(sampling, Sampling::Nearest) {
        return tap(pixels, mask, width, height, x.floor(), y.floor());
    }
    if matches!(sampling, Sampling::Bicubic) {
        return sample_bicubic(pixels, mask, width, height, [x, y]);
    }
    let (x, y) = (x - 0.5, y - 0.5);
    let (left, top) = (x.floor(), y.floor());
    let (fx, fy) = (x - left, y - top);
    let mut coverage = 0.0;
    let mut alpha = 0.0;
    let mut colors = [0.0; 3];
    let mut hidden = [0.0; 3];
    for (dx, dy, weight) in [
        (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
        (1.0, 0.0, fx * (1.0 - fy)),
        (0.0, 1.0, (1.0 - fx) * fy),
        (1.0, 1.0, fx * fy),
    ] {
        let (pixel, c) = tap(pixels, mask, width, height, left + dx, top + dy);
        let selected = weight * f64::from(c);
        let a = selected * f64::from(pixel[3]);
        coverage += selected;
        alpha += a;
        for channel in 0..3 {
            colors[channel] += a * f64::from(pixel[channel]);
            hidden[channel] += selected * f64::from(pixel[channel]);
        }
    }
    if coverage <= 0.0 {
        return ([0; 4], 0);
    }
    let mut pixel = [0; 4];
    pixel[3] = (alpha / coverage).round().clamp(0.0, 255.0) as u8;
    for channel in 0..3 {
        pixel[channel] = if alpha > 0.0 {
            (colors[channel] / alpha).round()
        } else {
            (hidden[channel] / coverage).round()
        }
        .clamp(0.0, 255.0) as u8;
    }
    (pixel, coverage.round().clamp(0.0, 255.0) as u8)
}

/// Interpolating Catmull-Rom weights at offsets -1, 0, 1, 2.
fn cubic_weights(t: f64) -> [f64; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        -0.5 * t + t2 - 0.5 * t3,
        1.0 - 2.5 * t2 + 1.5 * t3,
        0.5 * t + 2.0 * t2 - 1.5 * t3,
        -0.5 * t2 + 0.5 * t3,
    ]
}
/// Layer masks are straight grayscale, sampled independently from raster alpha.
/// New canvas locations reveal content; masks retain their original RGB-free data.
#[inline]
pub(crate) fn sample_layer_mask(
    mask: &crate::layer_mask::Mask,
    [x, y]: [f64; 2],
    sampling: Sampling,
) -> u8 {
    if !x.is_finite() || !y.is_finite() {
        return 255;
    }
    let [w, h] = mask.dimensions();
    let tap = |x: f64, y: f64| {
        if x < 0.0 || y < 0.0 || x >= w as f64 || y >= h as f64 {
            255.0
        } else {
            f64::from(mask.at(x as usize, y as usize))
        }
    };
    if matches!(sampling, Sampling::Nearest) {
        return tap(x.floor(), y.floor()) as u8;
    }
    let (x, y) = (x - 0.5, y - 0.5);
    let (l, t) = (x.floor(), y.floor());
    let (fx, fy) = (x - l, y - t);
    let value = if matches!(sampling, Sampling::Bilinear) {
        // Convert the footprint's integer origin once. The fully interior path
        // reads all taps from one tile when possible; canvas/tile edges retain
        // exactly the same white-outside semantics and arithmetic order.
        if l < -1.0 || t < -1.0 || l >= w as f64 || t >= h as f64 {
            return 255;
        }
        let (ix, iy) = (l as isize, t as isize);
        let values = if ix >= 0 && iy >= 0 && ix + 1 < w as isize && iy + 1 < h as isize {
            mask.bilinear_taps(ix as usize, iy as usize)
        } else {
            let edge = |dx, dy| {
                let (sx, sy) = (ix + dx, iy + dy);
                if sx < 0 || sy < 0 || sx >= w as isize || sy >= h as isize {
                    255
                } else {
                    mask.at(sx as usize, sy as usize)
                }
            };
            [edge(0, 0), edge(1, 0), edge(0, 1), edge(1, 1)]
        };
        f64::from(values[0]) * (1.0 - fx) * (1.0 - fy)
            + f64::from(values[1]) * fx * (1.0 - fy)
            + f64::from(values[2]) * (1.0 - fx) * fy
            + f64::from(values[3]) * fx * fy
    } else {
        let wx = cubic_weights(fx);
        let wy = cubic_weights(fy);
        let mut value = 0.0;
        for (dy, wy) in wy.into_iter().enumerate() {
            for (dx, wx) in wx.into_iter().enumerate() {
                value += wx * wy * tap(l + dx as f64 - 1.0, t + dy as f64 - 1.0);
            }
        }
        value
    };
    value.round().clamp(0.0, 255.0) as u8
}

fn sample_bicubic(
    pixels: &Pixels,
    mask: Coverage<'_>,
    width: usize,
    height: usize,
    [x, y]: [f64; 2],
) -> ([u8; 4], u8) {
    let (x, y) = (x - 0.5, y - 0.5);
    let (left, top) = (x.floor(), y.floor());
    let wx = cubic_weights(x - left);
    let wy = cubic_weights(y - top);
    let mut coverage = 0.0;
    let mut alpha = 0.0;
    let mut colors = [0.0; 3];
    let mut hidden = [0.0; 3];
    for (dy, wy) in wy.into_iter().enumerate() {
        for (dx, wx) in wx.into_iter().enumerate() {
            let (pixel, selected) = tap(
                pixels,
                mask,
                width,
                height,
                left + dx as f64 - 1.0,
                top + dy as f64 - 1.0,
            );
            let selected = wx * wy * f64::from(selected);
            let a = selected * f64::from(pixel[3]);
            coverage += selected;
            alpha += a;
            for channel in 0..3 {
                colors[channel] += a * f64::from(pixel[channel]);
                hidden[channel] += selected * f64::from(pixel[channel]);
            }
        }
    }
    // Normalize with the original weighted denominators before clamping.
    // Otherwise coverage overshoot would change a constant color's opacity.
    let bounded_coverage = coverage.clamp(0.0, 255.0);
    if bounded_coverage.round() == 0.0 {
        return ([0; 4], 0);
    }
    let mut pixel = [0; 4];
    pixel[3] = (alpha / coverage).round().clamp(0.0, 255.0) as u8;
    for channel in 0..3 {
        pixel[channel] = if alpha > 0.0 {
            (colors[channel] / alpha).round()
        } else {
            (hidden[channel] / coverage).round()
        }
        .clamp(0.0, 255.0) as u8;
    }
    (pixel, bounded_coverage.round() as u8)
}
pub(crate) fn apply(
    doc: &mut Document,
    matrix: Matrix,
    sampling: Sampling,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    apply_inner(doc, Mapping::Affine(matrix), sampling, cancel, None)
}
/// Affine, perspective or mesh transform of the selection on the active layer.
pub(crate) fn apply_mapping(
    doc: &mut Document,
    mapping: Mapping,
    sampling: Sampling,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    apply_inner(doc, mapping, sampling, cancel, None)
}
pub(crate) fn apply_settings(
    doc: &mut Document,
    settings: Settings,
    target: crate::paint_target::Target,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let mapping = settings.mapping(doc)?;
    if let Some(mirror) = settings.mirror {
        apply_pair(doc, mapping, settings.sampling, mirror, target, cancel)
    } else {
        match target {
            crate::paint_target::Target::Raster => {
                apply_mapping(doc, mapping, settings.sampling, cancel)
            }
            crate::paint_target::Target::Mask(id) => {
                crate::mask_transform::apply_mapping(doc, id, mapping, settings.sampling, cancel)
            }
        }
    }
}
#[derive(Default, serde::Serialize)]
pub(crate) struct WorkingSet {
    pub source_coverage_bytes: usize,
    pub destination_coverage_bytes: usize,
    pub output_bytes_beyond_source: usize,
    pub peak_working_bytes: usize,
}
pub(crate) fn profile(
    doc: &mut Document,
    matrix: Matrix,
    sampling: Sampling,
) -> Result<WorkingSet, String> {
    let mut memory = WorkingSet::default();
    apply_inner(
        doc,
        Mapping::Affine(matrix),
        sampling,
        None,
        Some(&mut memory),
    )?;
    Ok(memory)
}
fn apply_inner(
    doc: &mut Document,
    mapping: Mapping,
    sampling: Sampling,
    cancel: Option<&AtomicBool>,
    memory: Option<&mut WorkingSet>,
) -> Result<bool, String> {
    if doc.version == 2 {
        let mut result = doc.clone();
        let changed = apply_pixels(&mut result, mapping, sampling, cancel, memory)?;
        result.validate()?;
        crate::fill::check_cancel(cancel)?;
        *doc = result;
        return Ok(changed);
    }
    apply_pixels(doc, mapping, sampling, cancel, memory)
}
fn apply_pixels(
    doc: &mut Document,
    mapping: Mapping,
    sampling: Sampling,
    cancel: Option<&AtomicBool>,
    memory: Option<&mut WorkingSet>,
) -> Result<bool, String> {
    if let Mapping::Affine(matrix) = mapping {
        matrix.validate()?;
    }
    crate::fill::check_cancel(cancel)?;
    let layer = &doc.layers[doc.active];
    if layer.folder || layer.locked || layer.alpha_locked {
        return Err(
            "Select an unlocked raster layer and unlock transparency before transforming".into(),
        );
    }
    if mapping.is_identity() {
        return Ok(false);
    }
    let explicit = doc.selection.is_some() || doc.selection_mask.is_some();
    let count = crate::selection_mask::pixels(doc.width, doc.height)?;
    let mask = if let Some(mask) = &doc.selection_mask {
        Coverage::Mask(mask)
    } else {
        let rectangle = doc.selection.unwrap_or(Selection {
            left: 0,
            top: 0,
            right: doc.width,
            bottom: doc.height,
        });
        if !rectangle.valid(doc.width, doc.height) {
            return Err("Invalid selection rectangle".into());
        }
        Coverage::Rectangle(rectangle)
    };
    let Some(bounds) = mask.bounds() else {
        return Ok(false);
    };
    let padding = match sampling {
        Sampling::Nearest => 0.0,
        Sampling::Bilinear => 0.5,
        Sampling::Bicubic => 1.5,
    };
    // Warps are defined over the selection rectangle the transform acts on.
    let frame = Settings::bounds(doc);
    let target = mapping.target(
        if matches!(mapping, Mapping::Affine(_)) {
            bounds
        } else {
            frame
        },
        padding,
    );
    let target = Selection {
        left: target[0].floor().clamp(0.0, doc.width as f64) as usize,
        top: target[1].floor().clamp(0.0, doc.height as f64) as usize,
        right: target[2].ceil().clamp(0.0, doc.width as f64) as usize,
        bottom: target[3].ceil().clamp(0.0, doc.height as f64) as usize,
    };
    let rect = Selection {
        left: bounds.left.min(target.left),
        top: bounds.top.min(target.top),
        right: bounds.right.max(target.right),
        bottom: bounds.bottom.max(target.bottom),
    };
    let inverse = Inverse::new(&mapping, frame, rect)?;
    let mut coverage = explicit.then(|| vec![0; count]);
    let mut output = layer.pixels.clone();
    let mut changed = false;
    {
        let mut writer = output.writer();
        for y in rect.top..rect.bottom {
            crate::fill::check_cancel(cancel)?;
            for x in rect.left..rect.right {
                let (pixel, incoming) = sample(
                    &layer.pixels,
                    mask,
                    doc.width,
                    doc.height,
                    inverse.source(x, y),
                    sampling,
                );
                if let Some(coverage) = &mut coverage {
                    coverage[y * doc.width + x] = incoming;
                }
                let before = layer.pixels.rgba(y * doc.width + x);
                let after = crate::transform::transferred(before, pixel, mask.at(x, y), incoming);
                if before != after {
                    writer.rgba_mut(y * doc.width + x).copy_from_slice(&after);
                    changed = true;
                }
            }
        }
    }
    let layer_mask = crate::layer_mask::for_raster_transform(doc, cancel, |mask| {
        crate::layer_mask::Mask::generate(doc.width, doc.height, 255, cancel, |x, y| {
            Ok(sample_layer_mask(mask, inverse.source(x, y), sampling))
        })
    })?;
    let mask_changed = !crate::layer_mask::same_snapshot(&layer_mask, &layer.mask);
    if let Some(memory) = memory {
        // Source coverage is borrowed or described by four rectangle coordinates.
        memory.source_coverage_bytes = 0;
        memory.destination_coverage_bytes = coverage.as_ref().map_or(0, Vec::capacity);
        let mut registry = crate::snapshot_memory::Registry::new(doc);
        output.visit_allocations(|id, bytes| registry.add_pixels(id, bytes));
        if let Some(mask) = &layer_mask {
            mask.coverage
                .visit_allocations(|id, bytes| registry.add_pixels(id, bytes));
        }
        memory.output_bytes_beyond_source = registry.additional_bytes();
        memory.peak_working_bytes = memory.source_coverage_bytes
            + memory.destination_coverage_bytes
            + memory.output_bytes_beyond_source;
    }
    if changed {
        output
            .writer()
            .reclaim_region(doc.width, [rect.left, rect.top, rect.right, rect.bottom]);
    }
    let mapped = coverage
        .map(|coverage| Mask::from_coverage_control(doc.width, doc.height, coverage, cancel))
        .transpose()?;
    crate::fill::check_cancel(cancel)?;
    let selection_changed = mapped.as_ref().is_some_and(|mapped| {
        doc.selection != mapped.bounds() || doc.selection_mask.as_ref() != Some(mapped)
    });
    if changed {
        doc.layers[doc.active].pixels = output;
    }
    if mask_changed {
        doc.layers[doc.active].mask = layer_mask;
    }
    if selection_changed {
        let mapped = mapped.unwrap();
        doc.selection = mapped.bounds();
        doc.selection_mask = Some(mapped);
    }
    Ok(changed || mask_changed || selection_changed)
}

/// Queue timings with independent constant-color rectangle geometry oracles.
pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512, 2048] {
        let bounds = Selection {
            left: size / 4,
            top: size / 4,
            right: size * 3 / 4,
            bottom: size * 3 / 4,
        };
        let mut source = Document::new(size, size);
        source.selection = Some(bounds);
        {
            let mut writer = source.layers[0].pixels.writer();
            for y in bounds.top..bounds.bottom {
                for x in bounds.left..bounds.right {
                    writer
                        .rgba_mut(y * size + x)
                        .copy_from_slice(&[220, 40, 90, 255]);
                }
            }
        }
        for fractional in [false, true] {
            let settings = if fractional {
                Settings {
                    translation: [8.5, 0.5],
                    ..Settings::default()
                }
            } else {
                Settings {
                    scale_percent: [200.0, 100.0],
                    sampling: Sampling::Nearest,
                    ..Settings::default()
                }
            };
            let edit = crate::document_edits::Edit::AffineSettingsTarget {
                target: source.layers[0].id,
                settings: Box::new(settings),
            };
            let (mut result, mut history, timing) =
                crate::render_queue::measure_edit(&source, edit)?;
            let overlap = |start: f64, end: f64, pixel: usize| {
                (end.min(pixel as f64 + 1.0) - start.max(pixel as f64)).clamp(0.0, 1.0)
            };
            for y in 0..size {
                for x in 0..size {
                    let coverage = if fractional {
                        (255.0
                            * overlap(bounds.left as f64 + 8.5, bounds.right as f64 + 8.5, x)
                            * overlap(bounds.top as f64 + 0.5, bounds.bottom as f64 + 0.5, y))
                        .round() as u8
                    } else if y >= bounds.top && y < bounds.bottom {
                        255
                    } else {
                        0
                    };
                    let expected = if coverage == 0 {
                        [0; 4]
                    } else {
                        [220, 40, 90, coverage]
                    };
                    if result.layers[0].pixels.rgba(y * size + x) != expected
                        || result.selection_mask.as_ref().unwrap().at(x, y) != coverage
                    {
                        return Err(format!(
                            "Affine geometry oracle differs at {x},{y} ({size}, fractional={fractional})"
                        ));
                    }
                }
            }
            let after = result.clone();
            if !history.undo(&mut result)
                || result.layers[0].pixels != source.layers[0].pixels
                || result.selection != source.selection
                || result.selection_mask != source.selection_mask
                || !history.redo(&mut result)
                || result.layers[0].pixels != after.layers[0].pixels
                || result.selection_mask != after.selection_mask
            {
                return Err("Affine undo/redo differs".into());
            }
            let mut previews = Vec::new();
            for pending in [false, true] {
                let (mut preview, mut history, timing) =
                    crate::affine_preview::measure(&source, settings, pending)?;
                if preview.layers[0].pixels != after.layers[0].pixels
                    || preview.selection != after.selection
                    || preview.selection_mask != after.selection_mask
                {
                    return Err(
                        "Affine preview differs from independently verified geometry".into(),
                    );
                }
                if !history.undo(&mut preview)
                    || preview.layers[0].pixels != source.layers[0].pixels
                    || preview.selection != source.selection
                    || preview.selection_mask != source.selection_mask
                    || !history.redo(&mut preview)
                    || preview.layers[0].pixels != after.layers[0].pixels
                    || preview.selection_mask != after.selection_mask
                {
                    return Err("Affine preview undo/redo differs".into());
                }
                previews.push(serde_json::json!({"phase":if pending {"pending-final-request"} else {"ready-preview"},"timing":timing,"pixels_coverage_undo_redo_verified":true}));
            }
            cases.push(serde_json::json!({ "size": size, "operation": if fractional { "bilinear-half-pixel-translation" } else { "nearest-double-width" }, "timing": timing, "previews": previews, "pixels_and_coverage_verified": size * size, "undo_redo_verified": true }));
        }
    }
    let report = serde_json::json!({ "version":2, "scope":"One repetition per sparse-tiled constant opaque rectangular selection. Static background queue plus application frozen-source preview and ready/pending-request finish, guarded publication and sliced history settlement with a 2 ms yield deadline. Static path verifies an ordered snapshot. Preview results match independently verified static geometry and undo/redo. Worker joins outside owner timing and inside wall time. Source fixture retained during timing; final release of otherwise unshared source allocations excluded. Setup and verification excluded. No GUI, handle paint/hit-test, contour, display/GPU, physical input or RSS measurement; no hard frame-time guarantee.", "cases": cases });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn square_doc() -> Document {
        let mut doc = Document::new(64, 64);
        doc.selection = Some(Selection {
            left: 16,
            top: 16,
            right: 48,
            bottom: 48,
        });
        doc.layers[0].pixels.writer().write_region(
            64,
            [16, 16, 48, 48],
            &[220, 30, 40, 255].repeat(32 * 32),
        );
        doc
    }
    #[test]
    fn homography_hits_corners_and_inverts() {
        let b = Selection {
            left: 10,
            top: 20,
            right: 110,
            bottom: 70,
        };
        let quad = [[0.0, 0.0], [120.0, 30.0], [100.0, 90.0], [5.0, 60.0]];
        let h = Homography::rect_to_quad(b, quad).unwrap();
        let rect = [[10.0, 20.0], [110.0, 20.0], [110.0, 70.0], [10.0, 70.0]];
        for (from, to) in rect.iter().zip(quad) {
            let p = h.point(from[0], from[1]).unwrap();
            assert!(
                (p[0] - to[0]).abs() < 1e-9 && (p[1] - to[1]).abs() < 1e-9,
                "{p:?} {to:?}"
            );
        }
        let inverse = h.inverse().unwrap();
        let back = inverse
            .point(60.0, 50.0)
            .and_then(|p| h.point(p[0], p[1]))
            .unwrap();
        assert!((back[0] - 60.0).abs() < 1e-9 && (back[1] - 50.0).abs() < 1e-9);
        // A bow-tie is not a transform.
        let twisted = [[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]];
        assert!(Homography::rect_to_quad(b, twisted).is_err());
    }
    #[test]
    fn untouched_mesh_is_the_identity_and_hits_control_points() {
        let doc = square_doc();
        let settings = Settings::default();
        let mut grid = settings.mesh(&doc, 4, 4).unwrap();
        let identity = Mapping::Mesh {
            bounds: Settings::bounds(&doc),
            mesh: grid,
        };
        let b = Settings::bounds(&doc);
        for (x, y) in [(16.0, 16.0), (23.5, 40.25), (48.0, 48.0)] {
            let p = identity.forward(b, x, y);
            assert!((p[0] - x).abs() < 1e-9 && (p[1] - y).abs() < 1e-9);
        }
        grid.points_mut()[5] = [30.0, 18.0];
        let bent = Mapping::Mesh {
            bounds: b,
            mesh: grid,
        };
        let p = bent.forward(b, 16.0 + 32.0 / 3.0, 16.0 + 32.0 / 3.0);
        assert!((p[0] - 30.0).abs() < 1e-9 && (p[1] - 18.0).abs() < 1e-9);
        // Identity warps leave the pixels as they are.
        for warp in [
            Warp::Corners(settings.corners(&doc).unwrap()),
            Warp::Mesh(settings.mesh(&doc, 4, 4).unwrap()),
            Warp::Mesh(settings.mesh(&doc, 2, 7).unwrap()),
        ] {
            let mut warped = doc.clone();
            let mapping = Settings {
                warp: Some(warp),
                ..settings
            }
            .mapping(&doc)
            .unwrap();
            apply_mapping(&mut warped, mapping, Sampling::Nearest, None).unwrap();
            assert_eq!(warped.layers[0].pixels, doc.layers[0].pixels);
        }
    }
    #[test]
    fn distorting_corners_moves_pixels_into_the_quad() {
        let doc = square_doc();
        // Pull the right edge together into a perspective trapezoid.
        let corners = [[16.0, 16.0], [56.0, 28.0], [56.0, 36.0], [16.0, 48.0]];
        let settings = Settings {
            warp: Some(Warp::Corners(corners)),
            ..Settings::default()
        };
        let mut warped = doc.clone();
        let mapping = settings.mapping(&doc).unwrap();
        assert!(apply_mapping(&mut warped, mapping, Sampling::Bilinear, None).unwrap());
        let alpha = |x: usize, y: usize| warped.layers[0].pixels.rgba(y * 64 + x)[3];
        assert_eq!(alpha(20, 32), 255, "inside near the left edge");
        assert_eq!(alpha(52, 32), 255, "inside near the narrow right edge");
        assert_eq!(alpha(52, 18), 0, "outside the trapezoid");
        assert_eq!(alpha(40, 20), 0, "above the sloping top edge");
        // The selection follows the pixels.
        let selection = warped.selection.unwrap();
        // Strong perspective magnifies the wide edge ~5x, so bilinear's soft
        // half-pixel border spreads a few pixels past it.
        assert!(
            selection.right >= 55 && selection.left >= 12,
            "{selection:?}"
        );
        // Converting a scale/rotate transform to corners keeps its shape.
        let scaled = Settings {
            scale_percent: [150.0, 50.0],
            angle_degrees: 30.0,
            ..Settings::default()
        };
        let matrix = scaled.matrix(&doc).unwrap();
        let corners = scaled.corners(&doc).unwrap();
        let expected = matrix.point(48.0, 16.0);
        assert!(
            (corners[1][0] - expected[0]).abs() < 1e-9
                && (corners[1][1] - expected[1]).abs() < 1e-9
        );
        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.warp, settings.warp);
        let old: Settings = serde_json::from_str(r#"{"scale_percent":[100.0,100.0],"angle_degrees":0.0,"translation":[0.0,0.0],"sampling":"Bilinear"}"#).unwrap();
        assert_eq!(old.warp, None);
    }
    #[test]
    fn bicubic_is_interpolating_preserves_linear_ramps_and_has_cubic_weights() {
        let mut source = Document::new(6, 6);
        let bytes: Vec<_> = (0..6)
            .flat_map(|y| {
                (0..6).flat_map(move |x| {
                    [
                        40 + x * 20 + y * 10,
                        40 + x * 20 + y * 10,
                        40 + x * 20 + y * 10,
                        255,
                    ]
                })
            })
            .collect();
        source.layers[0]
            .pixels
            .writer()
            .write_region(6, [0, 0, 6, 6], &bytes);
        let full = Coverage::Rectangle(Selection {
            left: 0,
            top: 0,
            right: 6,
            bottom: 6,
        });
        assert_eq!(
            sample(
                &source.layers[0].pixels,
                full,
                6,
                6,
                [2.75, 2.25],
                Sampling::Bicubic
            ),
            ([103, 103, 103, 255], 255)
        );
        for y in 0..6 {
            for x in 0..6 {
                assert_eq!(
                    sample(
                        &source.layers[0].pixels,
                        full,
                        6,
                        6,
                        [x as f64 + 0.5, y as f64 + 0.5],
                        Sampling::Bicubic
                    ),
                    (source.layers[0].pixels.rgba(y * 6 + x), 255)
                );
            }
        }
        let pixels: Pixels =
            vec![0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255].into();
        let full = Coverage::Rectangle(Selection {
            left: 0,
            top: 0,
            right: 4,
            bottom: 1,
        });
        assert_eq!(
            sample(&pixels, full, 4, 1, [2.0, 0.5], Sampling::Bicubic),
            ([143, 143, 143, 255], 255)
        );
        assert_eq!(
            sample(&pixels, full, 4, 1, [2.0, 0.5], Sampling::Bilinear),
            ([128, 128, 128, 255], 255)
        );
    }

    #[test]
    fn bicubic_normalizes_before_clamping_and_excludes_transparent_or_unselected_colors() {
        let mask = Mask::from_coverage(4, 1, vec![0, 255, 255, 0]).unwrap();
        for alpha in [128, 255] {
            let pixels: Pixels = [80, 120, 200, alpha].repeat(4).into();
            assert_eq!(
                sample(&pixels, (&mask).into(), 4, 1, [2.0, 0.5], Sampling::Bicubic),
                ([80, 120, 200, alpha], 255)
            );
        }
        let pixels: Pixels = [[255, 0, 0, 255].repeat(3), [0, 0, 255, 0].repeat(3)]
            .concat()
            .into();
        let full = Coverage::Rectangle(Selection {
            left: 0,
            top: 0,
            right: 6,
            bottom: 1,
        });
        assert_eq!(
            sample(&pixels, full, 6, 1, [3.0, 0.5], Sampling::Bicubic),
            ([255, 0, 0, 128], 255)
        );
        let pixels: Pixels = vec![
            0, 255, 0, 255, 80, 120, 200, 128, 80, 120, 200, 128, 255, 0, 0, 255,
        ]
        .into();
        assert_eq!(
            sample(&pixels, (&mask).into(), 4, 1, [2.0, 0.5], Sampling::Bicubic),
            ([80, 120, 200, 128], 255)
        );
    }

    #[test]
    fn bicubic_padding_keeps_outer_support_and_integer_translation_matches_nearest() {
        let mut source = Document::new(9, 9);
        source.layers[0]
            .pixels
            .writer()
            .write_region(9, [4, 4, 5, 5], &[17, 83, 201, 255]);
        source.selection = Some(Selection {
            left: 4,
            top: 4,
            right: 5,
            bottom: 5,
        });
        let mut result = source.clone();
        apply(
            &mut result,
            Matrix([1.0, 0.0, 0.0, 1.0, 0.5, 0.5]),
            Sampling::Bicubic,
            None,
        )
        .unwrap();
        assert_eq!(result.selection_mask.as_ref().unwrap().at(3, 3), 1);
        assert_eq!(result.layers[0].pixels.rgba(3 * 9 + 3), [17, 83, 201, 1]);
        source.layers[0]
            .pixels
            .writer()
            .write_region(9, [5, 4, 6, 5], &[91, 37, 203, 0]);
        source.selection = Some(Selection {
            left: 4,
            top: 4,
            right: 6,
            bottom: 5,
        });
        let matrix = Matrix([1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        let mut nearest = source.clone();
        let mut cubic = source.clone();
        apply(&mut nearest, matrix, Sampling::Nearest, None).unwrap();
        apply(&mut cubic, matrix, Sampling::Bicubic, None).unwrap();
        assert_eq!(nearest.layers[0].pixels, cubic.layers[0].pixels);
        assert_eq!(nearest.selection_mask, cubic.selection_mask);
        assert_eq!(Settings::default().sampling, Sampling::Bilinear);
    }

    #[test]
    fn bicubic_ready_and_pending_previews_match_apply_and_one_step_undo() {
        let mut source = Document::new(32, 32);
        let bytes: Vec<_> = (0..32 * 32)
            .flat_map(|i| {
                [
                    i as u8,
                    (i / 4) as u8,
                    173,
                    if i % 5 == 0 { 0 } else { 128 },
                ]
            })
            .collect();
        source.layers[0]
            .pixels
            .writer()
            .write_region(32, [0, 0, 32, 32], &bytes);
        let settings = Settings {
            sampling: Sampling::Bicubic,
            scale_percent: [125.0, 80.0],
            angle_degrees: 23.0,
            ..Default::default()
        };
        let mut expected = source.clone();
        apply(
            &mut expected,
            settings.matrix(&source).unwrap(),
            Sampling::Bicubic,
            None,
        )
        .unwrap();
        for pending in [false, true] {
            let (mut actual, mut history, _) =
                crate::affine_preview::measure(&source, settings, pending).unwrap();
            assert_eq!(actual.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(actual.selection_mask, expected.selection_mask);
            assert!(history.undo(&mut actual));
            assert_eq!(actual.layers[0].pixels, source.layers[0].pixels);
            assert!(!history.can_undo());
            assert!(history.redo(&mut actual));
            assert_eq!(actual.layers[0].pixels, expected.layers[0].pixels);
        }
        let mut canceled = source.clone();
        let cancel = AtomicBool::new(true);
        assert!(
            apply(
                &mut canceled,
                settings.matrix(&source).unwrap(),
                Sampling::Bicubic,
                Some(&cancel)
            )
            .is_err()
        );
        assert!(crate::document::same_snapshot(&canceled, &source));
    }

    #[test]
    fn implicit_coverage_matches_materialized_masks_for_dense_alpha_and_hidden_rgb() {
        let mut source = Document::new(33, 19);
        for y in 0..19 {
            for x in 0..33 {
                source.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 33 + x)
                    .copy_from_slice(&[
                        (x * 7) as u8,
                        (y * 11) as u8,
                        31,
                        if (x + y).is_multiple_of(7) {
                            0
                        } else {
                            (x * 3 + y * 5) as u8
                        },
                    ]);
            }
        }
        for selected in [false, true] {
            let region = if selected {
                Selection {
                    left: 3,
                    top: 2,
                    right: 30,
                    bottom: 17,
                }
            } else {
                Selection {
                    left: 0,
                    top: 0,
                    right: 33,
                    bottom: 19,
                }
            };
            let mut implicit = source.clone();
            implicit.selection = selected.then_some(region);
            let mut materialized = implicit.clone();
            materialized.selection = Some(region);
            materialized.selection_mask = Some(Mask::rectangle(33, 19, region).unwrap());
            for matrix in [
                Matrix([-1.0, 0.0, 0.0, -1.0, 33.0, 19.0]),
                Matrix([0.8, 0.3, -0.4, 1.1, 2.0, -1.0]),
                Matrix([1.0, 0.0, 0.0, 1.0, -2.5, 1.5]),
            ] {
                for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Bicubic] {
                    let mut result = implicit.clone();
                    let mut reference = materialized.clone();
                    apply(&mut result, matrix, sampling, None).unwrap();
                    apply(&mut reference, matrix, sampling, None).unwrap();
                    assert_eq!(result.layers[0].pixels, reference.layers[0].pixels);
                    if selected {
                        assert_eq!(result.selection_mask, reference.selection_mask);
                        assert_eq!(result.selection, reference.selection);
                    } else {
                        assert!(result.selection.is_none() && result.selection_mask.is_none());
                    }
                    assert!(result.validate().is_ok());
                }
            }
        }
    }
    #[test]
    fn working_profile_counts_only_required_coverage_and_deduplicates_source_pixels() {
        let mut source = Document::new(17, 13);
        source.layers[0].pixels = [200, 30, 70, 255].repeat(17 * 13).into();
        let matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.5, 0.5]);
        for selected in [false, true] {
            let mut result = source.clone();
            result.selection = selected.then_some(Selection {
                left: 2,
                top: 2,
                right: 15,
                bottom: 11,
            });
            let working = profile(&mut result, matrix, Sampling::Bilinear).unwrap();
            assert_eq!(working.source_coverage_bytes, 0);
            assert_eq!(
                working.destination_coverage_bytes,
                if selected { 17 * 13 } else { 0 }
            );
            assert!(working.output_bytes_beyond_source >= 17 * 13 * 4);
            assert_eq!(
                working.peak_working_bytes,
                working.destination_coverage_bytes + working.output_bytes_beyond_source
            );
        }
        let working = profile(&mut source, Matrix::IDENTITY, Sampling::Bilinear).unwrap();
        assert_eq!(working.peak_working_bytes, 0);
    }
    #[test]
    fn implicit_coverage_keeps_invalid_rectangle_and_document_caps_atomic() {
        let matrix = Matrix([1.0, 0.0, 0.0, 1.0, 1.0, 0.0]);
        let mut invalid = Document::new(2, 1);
        invalid.selection = Some(Selection {
            left: 0,
            top: 0,
            right: 3,
            bottom: 1,
        });
        let before = invalid.clone();
        assert!(apply(&mut invalid, matrix, Sampling::Nearest, None).is_err());
        assert!(crate::document::same_snapshot(&invalid, &before));
        let mut oversized = Document::new(10001, 10000);
        let before = oversized.clone();
        assert!(apply(&mut oversized, matrix, Sampling::Nearest, None).is_err());
        assert!(crate::document::same_snapshot(&oversized, &before));
    }
    #[test]
    fn bilinear_sampling_uses_selected_premultiplied_rgba_and_ignores_hole_colors() {
        let pixels: Pixels = vec![255, 0, 0, 255, 0, 0, 255, 0].into();
        let mask = Mask::from_coverage(2, 1, vec![255, 255]).unwrap();
        assert_eq!(
            sample(
                &pixels,
                (&mask).into(),
                2,
                1,
                [1.0, 0.5],
                Sampling::Bilinear
            ),
            ([255, 0, 0, 128], 255)
        );
        let pixels: Pixels = vec![255, 0, 0, 255, 0, 255, 0, 255].into();
        let mask = Mask::from_coverage(2, 1, vec![255, 0]).unwrap();
        assert_eq!(
            sample(
                &pixels,
                (&mask).into(),
                2,
                1,
                [1.0, 0.5],
                Sampling::Bilinear
            ),
            ([255, 0, 0, 255], 128)
        );
        assert_eq!(
            sample(
                &pixels,
                (&mask).into(),
                2,
                1,
                [-1.0, 0.5],
                Sampling::Bilinear
            ),
            ([0; 4], 0)
        );
    }
    #[test]
    fn identity_shares_every_payload_and_invalid_or_canceled_operations_are_atomic() {
        let mut doc = Document::new(2, 1);
        doc.layers[0].pixels = vec![9, 7, 3, 0, 17, 83, 201, 128].into();
        let mask = Mask::from_coverage(2, 1, vec![128, 255]).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        let original = doc.clone();
        assert!(!apply(&mut doc, Matrix::IDENTITY, Sampling::Bilinear, None).unwrap());
        assert!(crate::document::same_snapshot(&doc, &original));
        for matrix in [
            Matrix([0.0; 6]),
            Matrix([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
            Matrix([1.0, 0.0, 0.0, 1.0, 1.0e10, 0.0]),
        ] {
            assert!(apply(&mut doc, matrix, Sampling::Nearest, None).is_err());
            assert!(crate::document::same_snapshot(&doc, &original));
        }
        assert!(
            apply(
                &mut doc,
                Matrix([1.0, 0.0, 0.0, 1.0, 1.0, 0.0]),
                Sampling::Nearest,
                Some(&AtomicBool::new(true))
            )
            .is_err()
        );
        assert!(crate::document::same_snapshot(&doc, &original));
        doc.layers[0].alpha_locked = true;
        assert!(apply(&mut doc, Matrix::IDENTITY, Sampling::Nearest, None).is_err());
    }
    #[test]
    fn nearest_integer_translation_matches_masked_remap_across_tile_edges() {
        let mut doc = Document::new(517, 259);
        let mut coverage = vec![0; 517 * 259];
        for y in 254..259 {
            for x in 254..258 {
                coverage[y * 517 + x] = if x == 256 { 0 } else { 128 };
                doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 517 + x)
                    .copy_from_slice(&[
                        (x % 251) as u8,
                        17,
                        (y % 239) as u8,
                        if y == 256 { 0 } else { 255 },
                    ]);
            }
        }
        let mask = Mask::from_coverage(517, 259, coverage).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask.clone());
        for [dx, dy] in [[1, 0], [-2, -1], [1000000, 0]] {
            let mut changed = doc.clone();
            let (expected, mapped) = crate::transform::remapped_masked(
                &doc.layers[0].pixels,
                517,
                259,
                &mask,
                crate::transform::Remap::Offset([dx, dy]),
            )
            .unwrap();
            apply(
                &mut changed,
                Matrix([1.0, 0.0, 0.0, 1.0, dx as f64, dy as f64]),
                Sampling::Nearest,
                None,
            )
            .unwrap();
            assert_eq!(changed.layers[0].pixels, expected);
            assert_eq!(changed.selection_mask, Some(mapped));
            assert!(changed.validate().is_ok());
        }
    }
    #[test]
    fn clockwise_quarter_turn_preserves_exact_rgba_and_four_turns_restore_source() {
        let mut doc = Document::new(2, 2);
        doc.layers[0].pixels =
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 7, 3, 0].into();
        let mask = Mask::from_coverage(2, 2, vec![255; 4]).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        let source = doc.clone();
        let matrix = Matrix([0.0, 1.0, -1.0, 0.0, 2.0, 0.0]);
        apply(&mut doc, matrix, Sampling::Nearest, None).unwrap();
        assert_eq!(
            doc.layers[0].pixels.to_vec(),
            vec![0, 0, 255, 255, 255, 0, 0, 255, 9, 7, 3, 0, 0, 255, 0, 255]
        );
        for _ in 0..3 {
            apply(&mut doc, matrix, Sampling::Nearest, None).unwrap();
        }
        assert_eq!(doc.layers[0].pixels, source.layers[0].pixels);
        assert_eq!(doc.selection_mask, source.selection_mask);
    }
    #[test]
    fn queued_affine_precedes_snapshot_and_undo_restores_selection_and_hidden_rgb() {
        let mut doc = Document::new(3, 1);
        doc.layers[0].pixels = vec![255, 0, 0, 255, 9, 7, 3, 0, 0, 0, 255, 255].into();
        doc.selection = Some(Selection {
            left: 0,
            top: 0,
            right: 2,
            bottom: 1,
        });
        let original = doc.clone();
        let edit = crate::document_edits::Edit::AffineTarget {
            target: doc.layers[0].id,
            matrix: Matrix([1.0, 0.0, 0.0, 1.0, 1.0, 0.0]),
            sampling: Sampling::Nearest,
        };
        let (mut result, mut history, measurement) =
            crate::render_queue::measure_edit(&doc, edit).unwrap();
        assert_eq!(measurement["completion_and_snapshot_verified"], true);
        assert_eq!(result.layers[0].pixels.rgba(0), [0; 4]);
        assert_eq!(result.layers[0].pixels.rgba(1), [255, 0, 0, 255]);
        assert_eq!(result.layers[0].pixels.rgba(2), [0, 0, 255, 255]);
        let after = result.clone();
        assert!(history.undo(&mut result));
        assert_eq!(result.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(result.selection, original.selection);
        assert!(result.selection_mask.is_none());
        assert!(history.redo(&mut result));
        assert_eq!(result.layers[0].pixels, after.layers[0].pixels);
        assert_eq!(result.selection_mask, after.selection_mask);
    }
    #[test]
    fn settings_pivot_tracks_execution_selection_and_rejects_unbounded_inputs() {
        let mut doc = Document::new(8, 4);
        assert_eq!(Settings::default().matrix(&doc).unwrap(), Matrix::IDENTITY);
        let settings = Settings {
            scale_percent: [200.0, 50.0],
            translation: [1.0, -1.0],
            ..Settings::default()
        };
        let canvas_matrix = settings.matrix(&doc).unwrap();
        doc.selection = Some(Selection {
            left: 0,
            top: 0,
            right: 2,
            bottom: 2,
        });
        let selected_matrix = settings.matrix(&doc).unwrap();
        assert_eq!(selected_matrix.point(1.0, 1.0), [2.0, 0.0]);
        assert_ne!(canvas_matrix, selected_matrix);
        for invalid in [
            Settings {
                scale_percent: [0.0, 100.0],
                ..settings
            },
            Settings {
                angle_degrees: f64::NAN,
                ..settings
            },
            Settings {
                translation: [f64::INFINITY, 0.0],
                ..settings
            },
        ] {
            assert!(invalid.matrix(&doc).is_err());
        }
        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: Settings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.matrix(&doc).unwrap(), selected_matrix);
    }
    #[test]
    fn queued_settings_use_preceding_selection_pivot_and_keep_separate_undo() {
        let mut doc = Document::new(8, 2);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[255, 0, 0, 255]);
        let original = doc.clone();
        let target = doc.layers[0].id;
        let mut queue = crate::render_queue::Queue::default();
        let mut history = crate::document::History::new();
        queue
            .edit(crate::document_edits::Edit::Selection(Some(Selection {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            })))
            .unwrap();
        queue
            .edit(crate::document_edits::Edit::AffineSettingsTarget {
                target,
                settings: Box::new(Settings {
                    scale_percent: [200.0, 100.0],
                    sampling: Sampling::Nearest,
                    ..Settings::default()
                }),
            })
            .unwrap();
        let snapshot = queue.snapshot().unwrap();
        let mut saved = None;
        for _ in 0..100 {
            if queue.waiting_edit() {
                queue.wait_edit();
            }
            let progress = queue.advance(&mut doc, &mut history, 4096).unwrap();
            if let Some((ticket, source)) = progress.snapshot {
                assert_eq!(ticket, snapshot);
                saved = Some(source);
            }
            if !queue.busy() {
                break;
            }
        }
        assert!(!queue.busy());
        let saved = saved.expect("ordered snapshot completed");
        assert_eq!(saved.layers[0].pixels.rgba(0), [255, 0, 0, 255]);
        assert_eq!(saved.selection.unwrap().right, 3);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(doc.selection.unwrap().right, 2);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.selection, None);
        assert!(!history.can_undo());
    }
}
