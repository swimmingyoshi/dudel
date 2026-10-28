//! A paired edit samples both existing regions from the same frozen source.
//! The partner's map is R * T * R, where R reflects across the chosen axis.
use super::*;
use crate::paint_target::Target;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mirror {
    pub center: [f64; 2],
    /// Clockwise tilt from a vertical mirror axis, in document coordinates.
    pub angle_degrees: f64,
}
impl Mirror {
    pub(crate) fn validate(self) -> Result<(), String> {
        if self
            .center
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1.0e6)
            || !self.angle_degrees.is_finite()
            || self.angle_degrees.abs() > 360.0
        {
            return Err("Mirror axis must be finite and within the canvas coordinate range".into());
        }
        Ok(())
    }
    pub(crate) fn matrix(self) -> Matrix {
        let (s, c) = (2.0 * self.angle_degrees.to_radians()).sin_cos();
        let snap = |v: f64| if v.abs() < 1e-12 { 0.0 } else { v };
        let (a, b, c, d) = (snap(-c), snap(-s), snap(-s), snap(c));
        let [x, y] = self.center;
        Matrix([a, b, c, d, x - a * x - c * y, y - b * x - d * y])
    }
    pub(crate) fn point(self, [x, y]: [f64; 2]) -> [f64; 2] {
        self.matrix().point(x, y)
    }
    pub(crate) fn from_symmetry(s: crate::symmetry::Symmetry, width: usize, height: usize) -> Self {
        Self {
            center: s.center(width, height).map(f64::from),
            angle_degrees: (f64::from(s.angle)
                + if s.mode == crate::symmetry::Mode::TopBottom {
                    90.0
                } else {
                    0.0
                }
                + 180.0)
                .rem_euclid(360.0)
                - 180.0,
        }
    }
}

fn reflected_bounds(r: [f64; 4], mirror: Mirror) -> [f64; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
        .map(|p| mirror.point(p))
        .iter()
        .fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        )
}
fn clip(b: [f64; 4], doc: &Document) -> Selection {
    Selection {
        left: b[0].floor().clamp(0.0, doc.width as f64) as usize,
        top: b[1].floor().clamp(0.0, doc.height as f64) as usize,
        right: b[2].ceil().clamp(0.0, doc.width as f64) as usize,
        bottom: b[3].ceil().clamp(0.0, doc.height as f64) as usize,
    }
}
fn rect_array(b: Selection) -> [f64; 4] {
    [b.left as f64, b.top as f64, b.right as f64, b.bottom as f64]
}
fn coverage_at(mask: Coverage<'_>, doc: &Document, [x, y]: [f64; 2]) -> u8 {
    let (x, y) = (x - 0.5, y - 0.5);
    let (l, t) = (x.floor(), y.floor());
    let (fx, fy) = (x - l, y - t);
    let mut value = 0.0;
    for (dx, dy, weight) in [
        (0., 0., (1. - fx) * (1. - fy)),
        (1., 0., fx * (1. - fy)),
        (0., 1., (1. - fx) * fy),
        (1., 1., fx * fy),
    ] {
        let (px, py) = (l + dx, t + dy);
        if px >= 0.0 && py >= 0.0 && px < doc.width as f64 && py < doc.height as f64 {
            value += weight * f64::from(mask.at(px as usize, py as usize));
        }
    }
    value.round().clamp(0.0, 255.0) as u8
}
fn multiply(a: [f64; 9], b: [f64; 9]) -> [f64; 9] {
    std::array::from_fn(|i| (0..3).map(|k| a[i / 3 * 3 + k] * b[k * 3 + i % 3]).sum())
}
fn homogeneous(Matrix([a, b, c, d, x, y]): Matrix) -> [f64; 9] {
    [a, c, x, b, d, y, 0.0, 0.0, 1.0]
}
fn reflected_inverse(
    mapping: Mapping,
    frame: Selection,
    rect: Selection,
    mirror: Mirror,
    cancel: Option<&AtomicBool>,
) -> Result<Inverse, String> {
    let m = homogeneous(mirror.matrix());
    match mapping {
        Mapping::Affine(a) => Ok(Inverse::Projective(Homography(multiply(
            multiply(m, homogeneous(a.inverse())),
            m,
        )))),
        Mapping::Projective(h) => {
            let inverse = h
                .inverse()
                .ok_or("Perspective transform is too close to singular")?;
            Ok(Inverse::Projective(Homography(multiply(
                multiply(m, inverse.0),
                m,
            ))))
        }
        Mapping::Mesh { .. } => {
            let mut coords =
                vec![[f32::NAN; 2]; (rect.right - rect.left) * (rect.bottom - rect.top)];
            let source = |i: usize, j: usize| {
                [
                    frame.left as f64
                        + (frame.right - frame.left) as f64 * i as f64 / MESH_STEPS as f64,
                    frame.top as f64
                        + (frame.bottom - frame.top) as f64 * j as f64 / MESH_STEPS as f64,
                ]
            };
            for j in 0..MESH_STEPS {
                crate::fill::check_cancel(cancel)?;
                for i in 0..MESH_STEPS {
                    for triangle in [
                        [(i, j), (i + 1, j), (i + 1, j + 1)],
                        [(i, j), (i + 1, j + 1), (i, j + 1)],
                    ] {
                        let s = triangle.map(|(i, j)| source(i, j));
                        let d = s.map(|[x, y]| mirror.point(mapping.forward(frame, x, y)));
                        rasterize(d, s.map(|p| mirror.point(p)), rect, &mut coords, cancel)?;
                    }
                }
            }
            Ok(Inverse::Lookup { rect, coords })
        }
    }
}
/// Symmetric coverage-weighted merge when the two incoming regions overlap.
/// Neither side wins because of traversal order; transparent hidden RGB survives.
fn merged(a: ([u8; 4], u8), b: ([u8; 4], u8)) -> ([u8; 4], u8) {
    if a.1 == 0 {
        return b;
    }
    if b.1 == 0 {
        return a;
    }
    let (ca, cb) = (u32::from(a.1), u32::from(b.1));
    let total = ca + cb;
    let (aa, ab) = (ca * u32::from(a.0[3]), cb * u32::from(b.0[3]));
    let alpha = aa + ab;
    let mut pixel = [0; 4];
    pixel[3] = ((alpha + total / 2) / total) as u8;
    for (c, channel) in pixel[..3].iter_mut().enumerate() {
        *channel = (u32::from(a.0[c]) * aa + u32::from(b.0[c]) * ab + alpha / 2)
            .checked_div(alpha)
            .unwrap_or_else(|| {
                (u32::from(a.0[c]) * ca + u32::from(b.0[c]) * cb + total / 2) / total
            }) as u8;
    }
    (pixel, total.min(255) as u8)
}

pub(crate) fn apply_pair(
    doc: &mut Document,
    mapping: Mapping,
    sampling: Sampling,
    mirror: Mirror,
    target: Target,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    mirror.validate()?;
    crate::fill::check_cancel(cancel)?;
    if let Some(error) = target.blocker(doc, doc.active, false) {
        return Err(error.into());
    }
    if target.is_raster() && doc.layers[doc.active].alpha_locked {
        return Err("Turn off Lock transparency to transform pixels".into());
    }
    if doc.selection.is_none() && doc.selection_mask.is_none() {
        return Err("Select one side before enabling Mirror pair".into());
    }
    if let Mapping::Affine(matrix) = mapping {
        matrix.validate()?;
    }
    let frame = Settings::bounds(doc);
    let core = doc
        .selection_mask
        .as_ref()
        .map_or(Coverage::Rectangle(frame), Coverage::Mask);
    let Some(bounds) = core.bounds() else {
        return Ok(false);
    };
    if mapping.is_identity() {
        return Ok(false);
    }
    let reflection = mirror.matrix();
    let partner_bounds = clip(reflected_bounds(rect_array(bounds), mirror), doc);
    let mut partner = crate::selection_mask::Builder::new(doc.width, doc.height)?;
    for y in partner_bounds.top..partner_bounds.bottom {
        crate::fill::check_cancel(cancel)?;
        for x in partner_bounds.left..partner_bounds.right {
            let value = coverage_at(core, doc, reflection.point(x as f64 + 0.5, y as f64 + 0.5));
            if value > 0 {
                if core.at(x, y) > 0 {
                    return Err("Select only one side of the mirror axis; its partner is included automatically".into());
                }
                partner.set(x, y, value);
            }
        }
    }
    let partner = partner.finish(cancel)?;
    let mut partner_doc = doc.clone();
    partner_doc.selection = partner.bounds();
    partner_doc.selection_mask = Some(partner.clone());
    let padding = match sampling {
        Sampling::Nearest => 0.,
        Sampling::Bilinear => 0.5,
        Sampling::Bicubic => 1.5,
    };
    let destination = mapping.target(frame, padding);
    let primary_rect = clip(destination, doc);
    let partner_rect = clip(reflected_bounds(destination, mirror), doc);
    let inverse = Inverse::new_control(&mapping, frame, primary_rect, cancel)?;
    let other_inverse = reflected_inverse(mapping, frame, partner_rect, mirror, cancel)?;
    let rect = [bounds, partner_bounds, primary_rect, partner_rect]
        .into_iter()
        .fold(bounds, |a, b| Selection {
            left: a.left.min(b.left),
            top: a.top.min(b.top),
            right: a.right.max(b.right),
            bottom: a.bottom.max(b.bottom),
        });
    let layer = &doc.layers[doc.active];
    let mut pixels = layer.pixels.clone();
    let mut layer_mask = layer.mask.clone();
    let mut moved = crate::selection_mask::Builder::new(doc.width, doc.height)?;
    let mut changed = false;
    {
        let mut pixel_writer = target.is_raster().then(|| pixels.writer());
        let mut mask_writer = if target.is_raster() {
            None
        } else {
            Some(layer_mask.as_mut().unwrap().coverage.writer())
        };
        let read = |source: &Document, coverage, point| {
            if target.is_raster() {
                sample(
                    &layer.pixels,
                    coverage,
                    doc.width,
                    doc.height,
                    point,
                    sampling,
                )
            } else {
                let (value, amount) = crate::mask_transform::sample_selected(
                    source,
                    &layer.mask.as_ref().unwrap().coverage,
                    point,
                    sampling,
                );
                ([value, value, value, 255], amount)
            }
        };
        for y in rect.top..rect.bottom {
            crate::fill::check_cancel(cancel)?;
            for x in rect.left..rect.right {
                let first = if primary_rect.contains(x, y) {
                    read(doc, core, inverse.source(x, y))
                } else {
                    ([0; 4], 0)
                };
                let second = if partner_rect.contains(x, y) {
                    read(
                        &partner_doc,
                        Coverage::Mask(&partner),
                        other_inverse.source(x, y),
                    )
                } else {
                    ([0; 4], 0)
                };
                // Keep the driving selection for the next linked edit. The partner
                // outline is shown separately while the transform is active.
                if first.1 > 0 {
                    moved.set(x, y, first.1);
                }
                let departing = core.at(x, y).max(partner.at(x, y));
                let (pixel, incoming) = merged(first, second);
                if let Some(writer) = &mut pixel_writer {
                    let before = layer.pixels.rgba(y * doc.width + x);
                    let after = crate::transform::transferred(before, pixel, departing, incoming);
                    if after != before {
                        writer.rgba_mut(y * doc.width + x).copy_from_slice(&after);
                        changed = true;
                    }
                } else if let Some(writer) = &mut mask_writer {
                    let before = layer.mask.as_ref().unwrap().coverage.at(x, y);
                    let after =
                        crate::mask_transform::transferred(before, pixel[0], departing, incoming);
                    if before != after {
                        writer.set(x, y, after);
                        changed = true;
                    }
                }
            }
        }
    }
    let moved = moved.finish(cancel)?;
    let selection_changed =
        doc.selection != moved.bounds() || doc.selection_mask.as_ref() != Some(&moved);
    let mut result = doc.clone();
    if changed {
        if target.is_raster() {
            pixels
                .writer()
                .reclaim_region(doc.width, [rect.left, rect.top, rect.right, rect.bottom]);
            result.layers[doc.active].pixels = pixels;
        } else {
            layer_mask
                .as_mut()
                .unwrap()
                .coverage
                .writer()
                .reclaim_region([rect.left, rect.top, rect.right, rect.bottom]);
            result.layers[doc.active].mask = layer_mask;
        }
    }
    if selection_changed {
        result.selection = moved.bounds();
        result.selection_mask = Some(moved);
    }
    result.validate()?;
    crate::fill::check_cancel(cancel)?;
    *doc = result;
    Ok(changed || selection_changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Document {
        let mut doc = Document::new(64, 32);
        doc.layers[0].pixels = (0..64 * 32)
            .flat_map(|i| {
                [
                    (i * 13 % 251) as u8,
                    (i * 7 % 253) as u8,
                    83,
                    if i % 7 == 0 { 0 } else { 201 },
                ]
            })
            .collect::<Vec<_>>()
            .into();
        doc.selection = Some(Selection {
            left: 8,
            top: 10,
            right: 14,
            bottom: 16,
        });
        let mut builder = crate::selection_mask::Builder::new(64, 32).unwrap();
        for y in 10..16 {
            for x in 8..14 {
                builder.set(x, y, if x == 8 { 128 } else { 255 });
            }
        }
        doc.selection_mask = Some(builder.finish(None).unwrap());
        doc
    }
    fn mirror() -> Mirror {
        Mirror {
            center: [32., 16.],
            angle_degrees: 0.,
        }
    }
    fn partner_settings(s: Settings) -> Settings {
        let warp = match s.warp {
            None => None,
            Some(Warp::Corners(q)) => Some(Warp::Corners(
                [q[1], q[0], q[3], q[2]].map(|p| mirror().point(p)),
            )),
            Some(Warp::Mesh(mesh)) => Some(Warp::Mesh(Mesh::from_fn(
                mesh.columns() as u8,
                mesh.rows() as u8,
                |x, y| mirror().point(mesh.at(mesh.columns() - 1 - x, y)),
            ))),
        };
        Settings {
            mirror: None,
            angle_degrees: -s.angle_degrees,
            translation: [-s.translation[0], s.translation[1]],
            warp,
            ..s
        }
    }
    #[test]
    fn paired_affine_perspective_and_mesh_match_independent_edits_of_each_artwork() {
        let original = fixture();
        let mut variants = vec![
            Settings {
                translation: [2., 1.],
                ..Settings::default()
            },
            Settings {
                scale_percent: [150., 50.],
                angle_degrees: 90.,
                flip: [true, false],
                translation: [1., -1.],
                ..Settings::default()
            },
            Settings {
                warp: Some(Warp::Corners([
                    [7.17, 9.13],
                    [15.31, 10.27],
                    [14.41, 17.19],
                    [8.23, 16.37],
                ])),
                ..Settings::default()
            },
        ];
        let mut mesh = Settings::default().mesh(&original, 3, 3).unwrap();
        for point in mesh.points_mut() {
            point[0] += 1.;
            point[1] -= 1.;
        }
        variants.push(Settings {
            warp: Some(Warp::Mesh(mesh)),
            ..Settings::default()
        });
        for sampling in [Sampling::Nearest, Sampling::Bilinear, Sampling::Bicubic] {
            for (case, settings) in variants.iter().enumerate() {
                let settings = Settings {
                    sampling,
                    ..*settings
                };
                let mut expected = original.clone();
                apply_settings(&mut expected, settings, Target::Raster, None).unwrap();
                let selected = (expected.selection, expected.selection_mask.clone());
                let mut builder = crate::selection_mask::Builder::new(64, 32).unwrap();
                for y in 0..32 {
                    for x in 0..64 {
                        builder.set(
                            x,
                            y,
                            original.selection_mask.as_ref().unwrap().at(63 - x, y),
                        );
                    }
                }
                let mask = builder.finish(None).unwrap();
                expected.selection = mask.bounds();
                expected.selection_mask = Some(mask);
                apply_settings(
                    &mut expected,
                    partner_settings(settings),
                    Target::Raster,
                    None,
                )
                .unwrap();
                expected.selection = selected.0;
                expected.selection_mask = selected.1;
                let mut actual = original.clone();
                apply_settings(
                    &mut actual,
                    Settings {
                        mirror: Some(mirror()),
                        ..settings
                    },
                    Target::Raster,
                    None,
                )
                .unwrap();
                assert_eq!(
                    actual.selection_mask, expected.selection_mask,
                    "case {case}, {sampling:?}"
                );
                for at in 0..64 * 32 {
                    let a = actual.layers[0].pixels.rgba(at);
                    let b = expected.layers[0].pixels.rgba(at);
                    assert_eq!(a, b, "pixel {at}, case {case}, {sampling:?}");
                }
            }
        }
    }
    #[test]
    fn curved_mesh_inverse_reflects_the_original_triangles() {
        let doc = fixture();
        let frame = Settings::bounds(&doc);
        let mut mesh = Settings::default().mesh(&doc, 4, 4).unwrap();
        mesh.points_mut()[5][0] += 1.7;
        mesh.points_mut()[6][1] -= 1.3;
        let mapping = Mapping::Mesh {
            bounds: frame,
            mesh,
        };
        let rect = Selection {
            left: 0,
            top: 0,
            right: 64,
            bottom: 32,
        };
        let first = Inverse::new_control(&mapping, frame, rect, None).unwrap();
        let second = reflected_inverse(mapping, frame, rect, mirror(), None).unwrap();
        for y in 0..32 {
            for x in 0..64 {
                let p = first.source(x, y);
                let q = second.source(63 - x, y);
                if p[0] < -1.0e8 {
                    assert!(q[0] < -1.0e8);
                } else {
                    let expected = mirror().point(p);
                    assert!(
                        (expected[0] - q[0]).abs() < 0.00001
                            && (expected[1] - q[1]).abs() < 0.00001,
                        "({x},{y}): {expected:?} != {q:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn horizontal_tilted_axes_masks_clipping_and_rejection_are_atomic() {
        for angle in [0., 90., -45.] {
            let mut doc = Document::new(32, 32);
            doc.selection = Some(Selection {
                left: 4,
                top: 12,
                right: 7,
                bottom: 15,
            });
            let axis = Mirror {
                center: [16., 16.],
                angle_degrees: angle,
            };
            let reflect = axis.matrix();
            for y in 12..15 {
                for x in 4..7 {
                    doc.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut(y * 32 + x)
                        .copy_from_slice(&[200, 30, 10, 255]);
                    let [px, py] = reflect.point(x as f64 + 0.5, y as f64 + 0.5);
                    doc.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut(py.floor() as usize * 32 + px.floor() as usize)
                        .copy_from_slice(&[10, 40, 200, 255]);
                }
            }
            let before = doc.clone();
            let settings = Settings {
                mirror: Some(axis),
                translation: [-2., 1.],
                sampling: Sampling::Nearest,
                ..Settings::default()
            };
            apply_settings(&mut doc, settings, Target::Raster, None).unwrap();
            let mut expected = vec![[0; 4]; 32 * 32];
            for y in 12..15 {
                for x in 4..7 {
                    expected[(y + 1) * 32 + x - 2] = [200, 30, 10, 255];
                    let [px, py] = reflect.point(x as f64 - 2. + 0.5, y as f64 + 1. + 0.5);
                    if px >= 0. && py >= 0. && px < 32. && py < 32. {
                        expected[py.floor() as usize * 32 + px.floor() as usize] =
                            [10, 40, 200, 255];
                    }
                }
            }
            for (at, pixel) in expected.iter().enumerate() {
                assert_eq!(
                    doc.layers[0].pixels.rgba(at),
                    *pixel,
                    "angle {angle}, pixel {at}"
                );
            }
            assert_ne!(doc.layers[0].pixels, before.layers[0].pixels);
        }
        let original = fixture();
        for case in 0..4 {
            let mut doc = original.clone();
            let mut settings = Settings {
                mirror: Some(mirror()),
                translation: [1., 0.],
                ..Settings::default()
            };
            if case == 0 {
                doc.selection = None;
                doc.selection_mask = None;
            }
            if case == 1 {
                settings.mirror.as_mut().unwrap().center[0] = 11.;
            }
            if case == 2 {
                settings.mirror.as_mut().unwrap().angle_degrees = f64::NAN;
            }
            let before = doc.clone();
            let cancel = AtomicBool::new(case == 3);
            assert!(apply_settings(&mut doc, settings, Target::Raster, Some(&cancel)).is_err());
            assert!(crate::document::same_snapshot(&doc, &before));
        }
    }
    #[test]
    fn paired_mask_edit_preserves_artwork_and_each_masks_own_values() {
        let mut doc = fixture();
        doc.version = 2;
        let id = crate::document::LayerId::new();
        let mut mask = crate::layer_mask::Mask::new(64, 32, 255).unwrap();
        for y in 10..16 {
            for x in 8..14 {
                mask.writer().set(x, y, 30);
                mask.writer().set(63 - x, y, 90);
            }
        }
        doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id,
            enabled: true,
            linked: true,
            coverage: mask,
        });
        let before = doc.clone();
        let settings = Settings {
            mirror: Some(mirror()),
            translation: [-2., 0.],
            sampling: Sampling::Nearest,
            ..Settings::default()
        };
        apply_settings(&mut doc, settings, Target::Mask(id), None).unwrap();
        assert_eq!(doc.layers[0].pixels, before.layers[0].pixels);
        let m = &doc.layers[0].mask.as_ref().unwrap().coverage;
        assert_eq!(m.at(8, 12), 30);
        assert_eq!(m.at(55, 12), 90);
        assert_eq!(m.at(13, 12), 255);
        assert_eq!(m.at(50, 12), 255);
        assert_eq!(m.at(32, 12), 255);
        assert_eq!(doc.layers[0].mask.as_ref().unwrap().id, id);
    }
    #[test]
    fn overlapping_destinations_merge_without_order_bias_and_legacy_settings_are_off() {
        let a = ([200, 20, 10, 128], 180);
        let b = ([30, 150, 220, 64], 140);
        assert_eq!(merged(a, b), merged(b, a));
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("mirror");
        assert!(
            serde_json::from_value::<Settings>(value)
                .unwrap()
                .mirror
                .is_none()
        );
    }

    #[test]
    fn crossing_the_axis_samples_both_originals_and_off_canvas_motion_only_clears_the_pair() {
        let mut original = Document::new(32, 16);
        original.selection = Some(Selection {
            left: 4,
            top: 6,
            right: 7,
            bottom: 9,
        });
        for y in 6..9 {
            for x in 4..7 {
                original.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 32 + x)
                    .copy_from_slice(&[200, 30, 10, 255]);
                original.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 32 + 31 - x)
                    .copy_from_slice(&[10, 40, 200, 255]);
            }
        }
        original.layers[0]
            .pixels
            .writer()
            .rgba_mut(8 * 32 + 16)
            .copy_from_slice(&[60, 70, 80, 255]);
        for offset in [21., 1.0e6] {
            let mut actual = original.clone();
            apply_settings(
                &mut actual,
                Settings {
                    mirror: Some(Mirror {
                        center: [16., 8.],
                        angle_degrees: 0.,
                    }),
                    translation: [offset, 0.],
                    sampling: Sampling::Nearest,
                    ..Settings::default()
                },
                Target::Raster,
                None,
            )
            .unwrap();
            for y in 0..16 {
                for x in 0..32 {
                    let expected = if (x, y) == (16, 8) {
                        [60, 70, 80, 255]
                    } else if offset == 21. && (6..9).contains(&y) && (4..7).contains(&x) {
                        [10, 40, 200, 255]
                    } else if offset == 21. && (6..9).contains(&y) && (25..28).contains(&x) {
                        [200, 30, 10, 255]
                    } else {
                        [0; 4]
                    };
                    assert_eq!(
                        actual.layers[0].pixels.rgba(y * 32 + x),
                        expected,
                        "offset {offset}, {x},{y}"
                    );
                }
            }
            if offset == 1.0e6 {
                assert!(actual.selection_mask.as_ref().unwrap().bounds().is_none());
            }
        }
    }
}
