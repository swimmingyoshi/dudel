//! Coverage-only geometry. Vacated and out-of-canvas mask areas reveal artwork.
use crate::{
    affine::{self, Mapping, Sampling},
    document::{Document, LayerId},
    layer_mask::Mask,
    selection::Selection,
    transform::{Remap, Transform},
};
use std::sync::atomic::AtomicBool;
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    Move([i64; 2]),
    Flip(Transform),
    Affine(Box<affine::Settings>),
}
impl Operation {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Self::Affine(s) = self {
            s.validate()
        } else {
            Ok(())
        }
    }
}
fn full(doc: &Document) -> Selection {
    Selection {
        left: 0,
        top: 0,
        right: doc.width,
        bottom: doc.height,
    }
}
fn selected(doc: &Document, x: usize, y: usize) -> u8 {
    if doc.selection.is_some_and(|s| !s.contains(x, y)) {
        0
    } else {
        doc.selection_mask.as_ref().map_or(255, |m| m.at(x, y))
    }
}
/// Selection overlap replaces shared coverage; only departing coverage that is
/// not replaced by incoming content is revealed white.
pub(crate) fn transferred(before: u8, source: u8, departing: u8, incoming: u8) -> u8 {
    let (d, i) = (u32::from(departing), u32::from(incoming));
    ((u32::from(before) * (255 - d.max(i))
        + u32::from(source) * i
        + 255 * d.saturating_sub(i)
        + 127)
        / 255) as u8
}
fn source(doc: &Document, id: LayerId) -> Result<&Mask, String> {
    doc.validate()?;
    if let Some(error) = crate::paint_target::Target::Mask(id).blocker(doc, doc.active, false) {
        return Err(error.into());
    }
    Ok(&doc.layers[doc.active].mask.as_ref().unwrap().coverage)
}
enum SelectionOutput {
    Unchanged,
    Bitmap(crate::selection_mask::Mask),
    Rectangle(Option<Selection>),
}
/// Checked in a wider integer type so even extreme pointer/queued offsets clip
/// without overflow. No destination means an explicit empty selection.
fn translated_bounds(bounds: Selection, offset: [i64; 2], doc: &Document) -> Option<Selection> {
    let x = |v: usize| (v as i128 + i128::from(offset[0])).clamp(0, doc.width as i128) as usize;
    let y = |v: usize| (v as i128 + i128::from(offset[1])).clamp(0, doc.height as i128) as usize;
    let result = Selection {
        left: x(bounds.left),
        top: y(bounds.top),
        right: x(bounds.right),
        bottom: y(bounds.bottom),
    };
    (result.left < result.right && result.top < result.bottom).then_some(result)
}
type Regions = [Option<Selection>; 5];
fn one_region(rect: Selection) -> Regions {
    [Some(rect), None, None, None, None]
}
fn move_regions(from: Selection, to: Option<Selection>) -> Regions {
    let Some(to) = to else {
        return one_region(from);
    };
    let overlap = Selection {
        left: from.left.max(to.left),
        top: from.top.max(to.top),
        right: from.right.min(to.right),
        bottom: from.bottom.min(to.bottom),
    };
    if overlap.left >= overlap.right || overlap.top >= overlap.bottom {
        return [Some(from), Some(to), None, None, None];
    }
    let valid = |r: Selection| (r.left < r.right && r.top < r.bottom).then_some(r);
    // Destination strips exclude the source rectangle. No gaps or overlap tests
    // enter the per-pixel loop, including when the rectangles are far apart.
    [
        Some(from),
        valid(Selection {
            left: to.left,
            top: to.top,
            right: to.right,
            bottom: overlap.top,
        }),
        valid(Selection {
            left: to.left,
            top: overlap.bottom,
            right: to.right,
            bottom: to.bottom,
        }),
        valid(Selection {
            left: to.left,
            top: overlap.top,
            right: overlap.left,
            bottom: overlap.bottom,
        }),
        valid(Selection {
            left: overlap.right,
            top: overlap.top,
            right: to.right,
            bottom: overlap.bottom,
        }),
    ]
}
fn finish(
    doc: &mut Document,
    id: LayerId,
    coverage: Mask,
    moved: SelectionOutput,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let before = source(doc, id)?;
    let different = !before.equivalent(&coverage, cancel)?;
    let mut result = doc.clone();
    if different {
        result.layers[result.active].mask.as_mut().unwrap().coverage = coverage;
    }
    let moved = match moved {
        SelectionOutput::Unchanged => None,
        SelectionOutput::Bitmap(mask) => Some(mask),
        SelectionOutput::Rectangle(Some(bounds)) => {
            result.selection = Some(bounds);
            result.selection_mask = None;
            None
        }
        SelectionOutput::Rectangle(None) => {
            Some(crate::selection_mask::Builder::new(doc.width, doc.height)?.finish(cancel)?)
        }
    };
    if let Some(mut selection) = moved {
        let bounds = selection.bounds();
        let mut rectangle = doc.selection_mask.is_none() && bounds.is_some();
        if let Some(b) = bounds.filter(|_| rectangle) {
            // Bounds enclose every non-zero byte; only the enclosed rectangle
            // needs checking, not the known-zero surrounding canvas.
            for y in b.top..b.bottom {
                crate::fill::check_cancel(cancel)?;
                if selection.bytes()[y * doc.width + b.left..y * doc.width + b.right]
                    .iter()
                    .any(|v| *v != 255)
                {
                    rectangle = false;
                    break;
                }
            }
        }
        result.selection = bounds;
        result.selection_mask = if rectangle {
            None
        } else {
            if doc.selection_mask.as_ref() == Some(&selection) {
                selection = doc.selection_mask.as_ref().unwrap().clone();
            }
            Some(selection)
        };
    }
    let changed = different
        || result.selection != doc.selection
        || result.selection_mask != doc.selection_mask;
    if changed {
        result.validate()?;
        crate::fill::check_cancel(cancel)?;
        *doc = result;
    }
    Ok(changed)
}
fn build(
    doc: &Document,
    before: &Mask,
    regions: Regions,
    explicit: bool,
    track_selection: bool,
    cancel: Option<&AtomicBool>,
    sample: impl Fn(usize, usize) -> (u8, u8) + Sync,
) -> Result<(Mask, Option<crate::selection_mask::Mask>), String> {
    if !explicit && regions == one_region(full(doc)) {
        return Ok((before.generate_shared(cancel, |x, y| sample(x, y).0)?, None));
    }
    let mut after = before.clone();
    let mut moved = track_selection
        .then(|| crate::selection_mask::Builder::new(doc.width, doc.height))
        .transpose()?;
    let mut uniform = None;
    let mut same = true;
    let visited = regions
        .iter()
        .flatten()
        .map(|r| (r.right - r.left) * (r.bottom - r.top))
        .sum::<usize>();
    {
        let mut writer = after.writer();
        for rect in regions.into_iter().flatten() {
            for y in rect.top..rect.bottom {
                crate::fill::check_cancel(cancel)?;
                for x in rect.left..rect.right {
                    let (value, incoming) = sample(x, y);
                    if let Some(builder) = &mut moved {
                        builder.set(x, y, incoming);
                    }
                    let value = if explicit {
                        transferred(before.at(x, y), value, selected(doc, x, y), incoming)
                    } else {
                        value
                    };
                    if let Some(first) = uniform {
                        same &= first == value;
                    } else {
                        uniform = Some(value);
                    }
                    writer.set(x, y, value);
                }
            }
            writer.reclaim_region([rect.left, rect.top, rect.right, rect.bottom]);
        }
    }
    if visited == doc.width * doc.height
        && same
        && let Some(value) = uniform
    {
        after = Mask::new(doc.width, doc.height, value)?;
    }
    crate::fill::check_cancel(cancel)?;
    Ok((
        after,
        moved.map(|builder| builder.finish(cancel)).transpose()?,
    ))
}
pub(crate) fn apply(
    doc: &mut Document,
    id: LayerId,
    operation: &Operation,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    operation.validate()?;
    crate::fill::check_cancel(cancel)?;
    if let Operation::Affine(settings) = operation {
        return affine::apply_settings(
            doc,
            **settings,
            crate::paint_target::Target::Mask(id),
            cancel,
        );
    }
    let before = source(doc, id)?;
    let explicit = doc.selection.is_some() || doc.selection_mask.is_some();
    if doc
        .selection_mask
        .as_ref()
        .is_some_and(|m| m.bounds().is_none())
    {
        return Ok(false);
    }
    let bounds = doc.selection.unwrap_or(full(doc));
    let map = match operation {
        Operation::Move(offset) => {
            if *offset == [0, 0] {
                return Ok(false);
            }
            Remap::Offset(*offset)
        }
        Operation::Flip(flip) => Remap::Flip(*flip, bounds),
        Operation::Affine(_) => unreachable!(),
    };
    if !explicit && let Some(regions) = before.white_sparse_regions() {
        let mut after = Mask::new(doc.width, doc.height, 255)?;
        {
            let mut writer = after.writer();
            for [l, t, r, b] in regions {
                for y in t..b {
                    crate::fill::check_cancel(cancel)?;
                    for x in l..r {
                        if let Some([tx, ty]) = map.point(x, y, doc.width, doc.height, false) {
                            writer.set(tx, ty, before.at(x, y));
                        }
                    }
                }
            }
        }
        return finish(doc, id, after, SelectionOutput::Unchanged, cancel);
    }
    if let Operation::Move(offset) = operation
        && (!explicit || (doc.selection_mask.is_none() && doc.selection == Some(full(doc))))
    {
        let after = before.offset(*offset, cancel)?;
        let selection = if explicit {
            SelectionOutput::Rectangle(translated_bounds(full(doc), *offset, doc))
        } else {
            SelectionOutput::Unchanged
        };
        return finish(doc, id, after, selection, cancel);
    }
    let regions = match operation {
        Operation::Flip(_) => one_region(bounds),
        Operation::Move(offset) if explicit => {
            move_regions(bounds, translated_bounds(bounds, *offset, doc))
        }
        _ => one_region(full(doc)),
    };
    let rectangle = doc.selection.filter(|_| doc.selection_mask.is_none());
    let (after, moved) = build(
        doc,
        before,
        regions,
        explicit,
        explicit && rectangle.is_none(),
        cancel,
        |x, y| {
            map.point(x, y, doc.width, doc.height, true)
                .map_or((255, 0), |[sx, sy]| {
                    (
                        before.at(sx, sy),
                        if explicit { selected(doc, sx, sy) } else { 255 },
                    )
                })
        },
    )?;
    let moved = if let Some(bounds) = rectangle {
        SelectionOutput::Rectangle(match operation {
            Operation::Move(offset) => translated_bounds(bounds, *offset, doc),
            _ => Some(bounds),
        })
    } else {
        moved.map_or(SelectionOutput::Unchanged, SelectionOutput::Bitmap)
    };
    finish(doc, id, after, moved, cancel)
}
pub(crate) fn apply_mapping(
    doc: &mut Document,
    id: LayerId,
    mapping: Mapping,
    sampling: Sampling,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    crate::fill::check_cancel(cancel)?;
    let before = source(doc, id)?;
    if let Mapping::Affine(matrix) = mapping {
        matrix.validate()?;
    }
    if mapping.is_identity()
        || doc
            .selection_mask
            .as_ref()
            .is_some_and(|m| m.bounds().is_none())
    {
        return Ok(false);
    }
    if let Mapping::Affine(affine::Matrix([a, b, c, d, dx, dy])) = mapping
        && [a, b, c, d] == [1.0, 0.0, 0.0, 1.0]
        && [dx, dy]
            .iter()
            .all(|v| v.fract() == 0.0 && *v >= i64::MIN as f64 && *v < -(i64::MIN as f64))
    {
        // At pixel centres, all three sampling kernels reduce to the exact
        // source byte for an integer translation, including selection coverage.
        return apply(doc, id, &Operation::Move([dx as i64, dy as i64]), cancel);
    }
    let explicit = doc.selection.is_some() || doc.selection_mask.is_some();
    let frame = doc.selection.unwrap_or(full(doc));
    let padding = match sampling {
        Sampling::Nearest => 0.0,
        Sampling::Bilinear => 0.5,
        Sampling::Bicubic => 1.5,
    };
    let target = mapping.target(frame, padding);
    let rect = if !explicit {
        full(doc)
    } else {
        Selection {
            left: frame
                .left
                .min(target[0].floor().clamp(0.0, doc.width as f64) as usize),
            top: frame
                .top
                .min(target[1].floor().clamp(0.0, doc.height as f64) as usize),
            right: frame
                .right
                .max(target[2].ceil().clamp(0.0, doc.width as f64) as usize),
            bottom: frame
                .bottom
                .max(target[3].ceil().clamp(0.0, doc.height as f64) as usize),
        }
    };
    let inverse = affine::Inverse::new_control(&mapping, frame, rect, cancel)?;
    if !explicit
        && matches!(mapping, Mapping::Affine(_))
        && let Some(regions) = before.white_sparse_regions()
    {
        let regions: Vec<Selection> = regions
            .into_iter()
            .filter_map(|[left, top, right, bottom]| {
                let b = mapping.target(
                    Selection {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    padding,
                );
                let r = Selection {
                    left: b[0].floor().clamp(0.0, doc.width as f64) as usize,
                    top: b[1].floor().clamp(0.0, doc.height as f64) as usize,
                    right: b[2].ceil().clamp(0.0, doc.width as f64) as usize,
                    bottom: b[3].ceil().clamp(0.0, doc.height as f64) as usize,
                };
                (r.left < r.right && r.top < r.bottom).then_some(r)
            })
            .collect();
        // Large zooms and overlapping tile bounds fall back to the full scan,
        // which also detects non-white constant results without a dense payload.
        let work: usize = regions
            .iter()
            .map(|r| (r.right - r.left) * (r.bottom - r.top))
            .sum();
        if work <= doc.width * doc.height / 4 {
            let mut after = Mask::new(doc.width, doc.height, 255)?;
            {
                let mut writer = after.writer();
                for r in &regions {
                    for y in r.top..r.bottom {
                        crate::fill::check_cancel(cancel)?;
                        for x in r.left..r.right {
                            writer.set(
                                x,
                                y,
                                affine::sample_layer_mask(before, inverse.source(x, y), sampling),
                            );
                        }
                    }
                }
                for r in &regions {
                    writer.reclaim_region([r.left, r.top, r.right, r.bottom]);
                }
            }
            return finish(doc, id, after, SelectionOutput::Unchanged, cancel);
        }
    }
    let (after, moved) = build(
        doc,
        before,
        one_region(rect),
        explicit,
        explicit,
        cancel,
        |x, y| {
            let point = inverse.source(x, y);
            if explicit {
                sample_selected(doc, before, point, sampling)
            } else {
                (affine::sample_layer_mask(before, point, sampling), 255)
            }
        },
    )?;
    finish(
        doc,
        id,
        after,
        moved.map_or(SelectionOutput::Unchanged, SelectionOutput::Bitmap),
        cancel,
    )
}
pub(crate) fn sample_selected(
    doc: &Document,
    mask: &Mask,
    [x, y]: [f64; 2],
    sampling: Sampling,
) -> (u8, u8) {
    let tap = |x: f64, y: f64| {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= doc.width as f64
            || y >= doc.height as f64
        {
            (255, 0)
        } else {
            (
                mask.at(x as usize, y as usize),
                selected(doc, x as usize, y as usize),
            )
        }
    };
    if matches!(sampling, Sampling::Nearest) {
        return tap(x.floor(), y.floor());
    }
    if !x.is_finite() || !y.is_finite() {
        return (255, 0);
    }
    let (x, y) = (x - 0.5, y - 0.5);
    let (l, t) = (x.floor(), y.floor());
    let (fx, fy) = (x - l, y - t);
    let cubic = |a: f64| {
        [
            -0.5 * a + a * a - 0.5 * a * a * a,
            1.0 - 2.5 * a * a + 1.5 * a * a * a,
            0.5 * a + 2.0 * a * a - 1.5 * a * a * a,
            -0.5 * a * a + 0.5 * a * a * a,
        ]
    };
    let (wx, wy, offset, n) = if matches!(sampling, Sampling::Bilinear) {
        ([1.0 - fx, fx, 0.0, 0.0], [1.0 - fy, fy, 0.0, 0.0], 0.0, 2)
    } else {
        (cubic(fx), cubic(fy), -1.0, 4)
    };
    let (mut value, mut coverage) = (0.0, 0.0);
    for (dy, wy) in wy.iter().enumerate().take(n) {
        for (dx, wx) in wx.iter().enumerate().take(n) {
            let (v, c) = tap(l + dx as f64 + offset, t + dy as f64 + offset);
            let weight = wx * wy * f64::from(c);
            value += weight * f64::from(v);
            coverage += weight;
        }
    }
    if coverage.clamp(0.0, 255.0).round() == 0.0 {
        (255, 0)
    } else {
        (
            (value / coverage).round().clamp(0.0, 255.0) as u8,
            coverage.round().clamp(0.0, 255.0) as u8,
        )
    }
}

#[cfg(test)]
#[path = "mask_transform_tests.rs"]
mod tests;
