//! Canvas bounds edits preserve exact RGBA and selection coverage, without resampling.
use crate::{document::Document, pixels::Pixels, selection::Selection};
use std::sync::atomic::AtomicBool;

/// Session-local coordinate frame retained in undo snapshots. Guides are edited
/// independently of artwork history; frame changes map their current positions
/// rather than restoring an old guide snapshot. Saved guides already use current
/// coordinates, so loaded documents start at the identity frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    turns: u8,
    translation: [i64; 2],
    generation: u64,
}
fn generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |n| n.checked_add(1),
    )
    .expect("Canvas frame identity exhausted")
}
impl Frame {
    pub fn identity() -> std::sync::Arc<Self> {
        static IDENTITY: std::sync::OnceLock<std::sync::Arc<Frame>> = std::sync::OnceLock::new();
        IDENTITY
            .get_or_init(|| std::sync::Arc::new(Frame::default()))
            .clone()
    }
    fn translated(self, offset: [i64; 2]) -> Result<Self, String> {
        Ok(Self {
            translation: [
                self.translation[0]
                    .checked_add(offset[0])
                    .ok_or("Canvas coordinates overflow")?,
                self.translation[1]
                    .checked_add(offset[1])
                    .ok_or("Canvas coordinates overflow")?,
            ],
            generation: generation(),
            ..self
        })
    }
    pub fn rotated(self, height: usize) -> Result<Self, String> {
        Ok(Self {
            turns: (self.turns + 1) % 4,
            generation: generation(),
            translation: [
                (height as i64)
                    .checked_sub(self.translation[1])
                    .ok_or("Canvas coordinates overflow")?,
                self.translation[0],
            ],
        })
    }
    pub fn turn_from(self, before: Self) -> u8 {
        (self.turns + 4 - before.turns) % 4
    }
    pub fn angle_from(self, before: Self) -> f32 {
        (self.turns as i8 - before.turns as i8) as f32 * 90.0
    }
    pub fn point_from(self, before: Self, point: [f32; 2]) -> [f32; 2] {
        let [x, y] = [
            point[0] as f64 - before.translation[0] as f64,
            point[1] as f64 - before.translation[1] as f64,
        ];
        let p = match self.turn_from(before) {
            0 => [x, y],
            1 => [-y, x],
            2 => [-x, -y],
            _ => [y, -x],
        };
        [
            (p[0] + self.translation[0] as f64) as f32,
            (p[1] + self.translation[1] as f64) as f32,
        ]
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub width: usize,
    pub height: usize,
    /// 0 = left/top, 1 = center, 2 = right/bottom.
    pub anchor: [u8; 2],
    /// None preserves the current resolution for existing scripted edits.
    #[serde(default)]
    pub dpi: Option<f64>,
}

pub fn validate_size(width: usize, height: usize) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || width
            .checked_mul(height)
            .is_none_or(|n| n > crate::document::MAX_PIXELS)
    {
        return Err(format!("Canvas must be {}", crate::document::SIZE_LIMIT));
    }
    Ok(())
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        validate_size(self.width, self.height)?;
        if let Some(dpi) = self.dpi {
            crate::document::validate_dpi(dpi)?;
        }
        if self.anchor.iter().any(|v| *v > 2) {
            return Err("Invalid canvas anchor".into());
        }
        Ok(())
    }
    pub fn offset(&self, width: usize, height: usize) -> [i64; 2] {
        [
            ((self.width as i64 - width as i64) * i64::from(self.anchor[0])).div_euclid(2),
            ((self.height as i64 - height as i64) * i64::from(self.anchor[1])).div_euclid(2),
        ]
    }
}

pub fn resize(
    source: &Document,
    settings: Settings,
    cancel: Option<&AtomicBool>,
) -> Result<(Document, bool), String> {
    settings.validate()?;
    source.validate()?;
    let (mut doc, mut changed) = reframe(
        source,
        [settings.width, settings.height],
        settings.offset(source.width, source.height),
        cancel,
    )?;
    if let Some(dpi) = settings.dpi {
        changed |= doc.dpi != dpi;
        doc.dpi = dpi;
    }
    Ok((doc, changed))
}
pub fn crop(source: &Document, cancel: Option<&AtomicBool>) -> Result<(Document, bool), String> {
    source.validate()?;
    let s = source
        .selection
        .ok_or("Select an area before cropping the canvas")?;
    reframe(
        source,
        [s.right - s.left, s.bottom - s.top],
        [-(s.left as i64), -(s.top as i64)],
        cancel,
    )
}
fn reframe(
    source: &Document,
    [width, height]: [usize; 2],
    offset: [i64; 2],
    cancel: Option<&AtomicBool>,
) -> Result<(Document, bool), String> {
    validate_size(width, height)?;
    crate::fill::check_cancel(cancel)?;
    if [width, height] == [source.width, source.height] && offset == [0, 0] {
        return Ok((source.clone(), false));
    }
    let mut doc = source.clone();
    doc.canvas_frame = source.canvas_frame.translated(offset)?.into();
    let translate = |s: Selection| {
        let left = (s.left as i64 + offset[0]).clamp(0, width as i64) as usize;
        let right = (s.right as i64 + offset[0]).clamp(0, width as i64) as usize;
        let top = (s.top as i64 + offset[1]).clamp(0, height as i64) as usize;
        let bottom = (s.bottom as i64 + offset[1]).clamp(0, height as i64) as usize;
        (left < right && top < bottom).then_some(Selection {
            left,
            top,
            right,
            bottom,
        })
    };
    // Source overlap bounds are computed before visiting any stored rows.
    let left = (-offset[0]).clamp(0, source.width as i64) as usize;
    let top = (-offset[1]).clamp(0, source.height as i64) as usize;
    let right = (width as i64 - offset[0]).clamp(left as i64, source.width as i64) as usize;
    let bottom = (height as i64 - offset[1]).clamp(top as i64, source.height as i64) as usize;
    for (out, layer) in doc.layers.iter_mut().zip(&source.layers) {
        let from = |[x, y]: [usize; 2]| {
            let sx = (x as i64).checked_sub(offset[0])?;
            let sy = (y as i64).checked_sub(offset[1])?;
            (sx >= 0 && sy >= 0 && sx < source.width as i64 && sy < source.height as i64)
                .then_some([sx as usize, sy as usize])
        };
        if let Some(mask) = &mut out.mask {
            mask.coverage = mask.coverage.remap(width, height, 255, cancel, from)?;
        }
        crate::material::follow(out, |relief| relief.remap(width, height, 0, cancel, from))?;
        if layer.folder {
            continue;
        }
        let mut pixels = Pixels::sparse(width, height);
        {
            let mut writer = pixels.writer();
            layer.pixels.try_visit_rows(
                source.width,
                source.height,
                [left, top, right, bottom],
                |x, y, bytes| {
                    crate::fill::check_cancel(cancel)?;
                    let tx = (x as i64 + offset[0]) as usize;
                    let ty = (y as i64 + offset[1]) as usize;
                    // Skip all-zero pixels, but retain alpha-zero hidden RGB exactly.
                    for (i, rgba) in bytes.chunks_exact(4).enumerate() {
                        if rgba != [0; 4] {
                            writer.rgba_mut_at(width, tx + i, ty).copy_from_slice(rgba);
                        }
                    }
                    Ok::<_, String>(())
                },
            )?;
        }
        out.pixels = pixels;
    }
    if let Some(mask) = &source.selection_mask {
        let mut coverage = vec![0; width * height];
        for y in top..bottom {
            crate::fill::check_cancel(cancel)?;
            let ty = (y as i64 + offset[1]) as usize;
            let tx = (left as i64 + offset[0]) as usize;
            coverage[ty * width + tx..ty * width + tx + right - left]
                .copy_from_slice(&mask.bytes()[y * source.width + left..y * source.width + right]);
        }
        let mask =
            crate::selection_mask::Mask::from_coverage_control(width, height, coverage, cancel)?;
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
    } else {
        doc.selection = source.selection.and_then(translate);
        if source.selection.is_some() && doc.selection.is_none() {
            doc.selection_mask = Some(crate::selection_mask::Mask::from_coverage_control(
                width,
                height,
                vec![0; width * height],
                cancel,
            )?);
        }
    }
    doc.width = width;
    doc.height = height;
    if let Some(guides) = &mut doc.guides {
        guides.map_canvas(*source.canvas_frame, *doc.canvas_frame);
    }
    crate::fill::check_cancel(cancel)?;
    doc.validate()?;
    Ok((doc, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection_mask::Mask;

    fn source(width: usize, height: usize) -> Document {
        let mut doc = Document::new(width, height);
        doc.add_layer();
        for (layer_index, layer) in doc.layers.iter_mut().enumerate() {
            let mut writer = layer.pixels.writer();
            for y in 0..height {
                for x in 0..width {
                    writer.rgba_mut_at(width, x, y).copy_from_slice(&[
                        x as u8,
                        (y * 29) as u8,
                        (layer_index * 80 + 11) as u8,
                        [0, 64, 255][(x + y) % 3],
                    ]);
                }
            }
        }
        doc.layers[0].locked = true;
        doc.layers[1].alpha_locked = true;
        doc.layers[1].opacity = 0.7;
        doc.layers[1].blend = crate::blend::BlendMode::Multiply;
        doc.add_folder().unwrap();
        let mut coverage = vec![0; width * height];
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                coverage[y * width + x] = (1 + (x + y) % 255) as u8;
            }
        }
        let mask = Mask::from_coverage(width, height, coverage).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        doc.validate().unwrap();
        doc
    }

    fn reference(source: &Document, result: &Document, offset: [i64; 2]) {
        assert_eq!(source.layers.len(), result.layers.len());
        assert_eq!(source.active, result.active);
        for (a, b) in source.layers.iter().zip(&result.layers) {
            assert_eq!(
                (
                    a.id,
                    a.depth,
                    a.folder,
                    a.collapsed,
                    a.visible,
                    a.locked,
                    a.alpha_locked,
                    a.clipped
                ),
                (
                    b.id,
                    b.depth,
                    b.folder,
                    b.collapsed,
                    b.visible,
                    b.locked,
                    b.alpha_locked,
                    b.clipped
                )
            );
            assert_eq!(
                (&a.name, a.opacity, a.blend, a.csp_byte_opacity),
                (&b.name, b.opacity, b.blend, b.csp_byte_opacity)
            );
            if a.folder {
                assert!(b.pixels.is_empty());
                continue;
            }
            for y in 0..result.height {
                for x in 0..result.width {
                    let (sx, sy) = (x as i64 - offset[0], y as i64 - offset[1]);
                    let expected = if sx >= 0
                        && sy >= 0
                        && sx < source.width as i64
                        && sy < source.height as i64
                    {
                        a.pixels.rgba(sy as usize * source.width + sx as usize)
                    } else {
                        [0; 4]
                    };
                    assert_eq!(b.pixels.rgba(y * result.width + x), expected);
                }
            }
        }
        if let Some(mask) = &source.selection_mask {
            let mapped = result.selection_mask.as_ref().unwrap();
            for y in 0..result.height {
                for x in 0..result.width {
                    let (sx, sy) = (x as i64 - offset[0], y as i64 - offset[1]);
                    let expected = if sx >= 0
                        && sy >= 0
                        && sx < source.width as i64
                        && sy < source.height as i64
                    {
                        mask.at(sx as usize, sy as usize)
                    } else {
                        0
                    };
                    assert_eq!(mapped.at(x, y), expected);
                }
            }
            assert_eq!(result.selection, mapped.bounds());
        }
        result.validate().unwrap();
    }

    #[test]
    fn nine_anchors_preserve_exact_layer_bytes_mask_and_metadata_across_tile_edges() {
        for flat in [false, true] {
            let mut doc = source(513, 5);
            if flat {
                for layer in doc.layers.iter_mut().filter(|l| !l.folder) {
                    layer.pixels = layer.pixels.to_vec().into();
                }
            }
            let before = serde_json::to_value(&doc).unwrap();
            for anchor_y in 0..3 {
                for anchor_x in 0..3 {
                    for [width, height] in [[517, 9], [507, 3], [519, 2], [1, 1]] {
                        let settings = Settings {
                            width,
                            height,
                            anchor: [anchor_x, anchor_y],
                            dpi: None,
                        };
                        let (result, changed) = resize(&doc, settings, None).unwrap();
                        assert!(changed);
                        reference(&doc, &result, settings.offset(doc.width, doc.height));
                    }
                }
            }
            assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        }
    }

    #[test]
    fn crop_preserves_coverage_hidden_rgb_and_empty_selection_does_not_select_everything() {
        let doc = source(9, 7);
        let s = doc.selection.unwrap();
        let (cropped, changed) = crop(&doc, None).unwrap();
        assert!(changed);
        assert_eq!(
            [cropped.width, cropped.height],
            [s.right - s.left, s.bottom - s.top]
        );
        reference(&doc, &cropped, [-(s.left as i64), -(s.top as i64)]);
        let mut rect = Document::new(5, 5);
        rect.selection = Some(Selection {
            left: 3,
            top: 3,
            right: 5,
            bottom: 5,
        });
        let (empty, _) = resize(
            &rect,
            Settings {
                width: 1,
                height: 1,
                anchor: [0, 0],
                dpi: None,
            },
            None,
        )
        .unwrap();
        assert!(empty.selection.is_none());
        assert_eq!(empty.selection_mask.unwrap().bytes(), &[0]);
    }

    #[test]
    fn no_op_invalid_limits_and_cancellation_preserve_source_and_sparse_empty_layers() {
        let doc = Document::new(2048, 2048);
        let same = Settings {
            width: 2048,
            height: 2048,
            anchor: [1, 1],
            dpi: None,
        };
        let (unchanged, changed) = resize(&doc, same, None).unwrap();
        assert!(!changed && crate::document::same_snapshot(&doc, &unchanged));
        let (small, _) = resize(
            &doc,
            Settings {
                width: 257,
                height: 259,
                anchor: [2, 2],
                dpi: None,
            },
            None,
        )
        .unwrap();
        assert!(small.layers[0].pixels.nonzero_tiles(257, 259).is_empty());
        for [width, height] in [[0, 1], [16385, 1], [10001, 10000], [usize::MAX, 1]] {
            assert!(
                resize(
                    &doc,
                    Settings {
                        width,
                        height,
                        anchor: [1, 1],
                        dpi: None,
                    },
                    None
                )
                .is_err()
            );
        }
        assert!(
            resize(
                &doc,
                Settings {
                    anchor: [3, 0],
                    dpi: None,
                    ..same
                },
                None
            )
            .is_err()
        );
        let cancel = AtomicBool::new(true);
        assert!(
            resize(
                &doc,
                Settings {
                    width: 1,
                    height: 1,
                    ..same
                },
                Some(&cancel)
            )
            .is_err()
        );
        assert!(crop(&doc, None).is_err());
        let mut invalid = Document::new(5, 5);
        invalid.selection = Some(Selection {
            left: 3,
            top: 0,
            right: 0,
            bottom: 1,
        });
        assert!(crop(&invalid, None).is_err());
        assert!(
            resize(
                &invalid,
                Settings {
                    width: 3,
                    height: 3,
                    anchor: [1, 1],
                    dpi: None,
                },
                None
            )
            .is_err()
        );
        assert!(doc.layers[0].pixels.nonzero_tiles(2048, 2048).is_empty());
    }

    #[test]
    fn geometry_snapshot_metadata_is_shared_and_not_reported_as_raster_payload() {
        let before = Document::new(8, 8);
        let (after, _) = resize(
            &before,
            Settings {
                width: 12,
                height: 12,
                anchor: [0, 0],
                dpi: None,
            },
            None,
        )
        .unwrap();
        let snapshot = crate::snapshot_memory::Snapshot::capture(&before);
        let one = crate::snapshot_memory::beyond_live(&after, [&snapshot]);
        let two = crate::snapshot_memory::beyond_live(&after, [&snapshot, &snapshot]);
        assert_eq!(two.pixel_bytes_beyond_live, one.pixel_bytes_beyond_live);
        assert_eq!(
            two.metadata_bytes,
            2 * one.metadata_bytes - std::mem::size_of::<Frame>()
        );
    }
}
