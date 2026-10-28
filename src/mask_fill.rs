//! Private, cancelable coverage edits. Never expand the destination mask to RGBA.
use crate::{
    document::{Document, LayerId},
    fill::{self, Request, Source},
    layer_mask::Mask,
    paint_target::Target,
};
use std::sync::atomic::AtomicBool;

fn source(doc: &Document, index: usize, id: LayerId) -> Result<&Mask, String> {
    if let Some(error) = Target::Mask(id).blocker(doc, index, false) {
        return Err(error.into());
    }
    Ok(&doc.layers[index].mask.as_ref().unwrap().coverage)
}
fn selected(doc: &Document, x: usize, y: usize) -> u8 {
    if doc.selection.is_some_and(|s| !s.contains(x, y)) {
        0
    } else {
        doc.selection_mask.as_ref().map_or(255, |m| m.at(x, y))
    }
}
pub(crate) fn whole_selection(doc: &Document, cancel: Option<&AtomicBool>) -> Result<bool, String> {
    if doc.selection.is_some_and(|s| {
        s.left != 0 || s.top != 0 || s.right != doc.width || s.bottom != doc.height
    }) {
        return Ok(false);
    }
    if let Some(mask) = &doc.selection_mask {
        for bytes in mask.bytes().chunks(4096) {
            fill::check_cancel(cancel)?;
            if bytes.iter().any(|b| *b != 255) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn constant(before: &Mask, value: u8, cancel: Option<&AtomicBool>) -> Result<Option<Mask>, String> {
    let [w, h] = before.dimensions();
    let after = Mask::new(w, h, value)?;
    if before.equivalent(&after, cancel)? {
        Ok(None)
    } else {
        Ok(Some(after))
    }
}
pub(crate) fn clear(
    doc: &Document,
    index: usize,
    id: LayerId,
    whole: bool,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Mask>, String> {
    fill::check_cancel(cancel)?;
    let before = source(doc, index, id)?;
    if whole || whole_selection(doc, cancel)? {
        return constant(before, 255, cancel);
    }
    if doc
        .selection_mask
        .as_ref()
        .is_some_and(|m| m.bounds().is_none())
    {
        return Ok(None);
    }
    let bounds = doc.selection.unwrap_or(crate::selection::Selection {
        left: 0,
        top: 0,
        right: doc.width,
        bottom: doc.height,
    });
    let mut after = before.clone();
    let mut changed = false;
    {
        let mut writer = after.writer();
        for y in bounds.top..bounds.bottom {
            fill::check_cancel(cancel)?;
            for x in bounds.left..bounds.right {
                let amount = selected(doc, x, y);
                if amount > 0 {
                    changed |= writer.paint(x, y, 255, f32::from(amount) / 255.0);
                }
            }
        }
        writer.reclaim();
    }
    fill::check_cancel(cancel)?;
    Ok(changed.then_some(after))
}

pub(crate) fn fill(
    doc: &Document,
    index: usize,
    id: LayerId,
    request: Request,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Mask>, String> {
    request.validate()?;
    fill::check_cancel(cancel)?;
    let before = source(doc, index, id)?;
    let [x, y] = request.point;
    if x >= doc.width || y >= doc.height || selected(doc, x, y) == 0 {
        return Ok(None);
    }
    let reference = match request.source {
        Source::Active => None,
        Source::Visible if request.opacity > 0.0 => Some(
            doc.composite_bounds_checked([0, 0, doc.width, doc.height], true, || {
                fill::check_cancel(cancel)
            })?
            .into(),
        ),
        Source::Visible => None,
        Source::Layer(id) => {
            let layer = doc
                .layers
                .iter()
                .find(|l| l.id == id)
                .ok_or("Fill reference layer no longer exists")?;
            if layer.folder {
                return Err("Choose a raster fill reference".into());
            }
            Some(layer.pixels.clone())
        }
    };
    if request.opacity == 0.0 {
        return Ok(None);
    }
    let read = |at: usize| {
        reference.as_ref().map_or_else(
            || {
                let value = before.at(at % doc.width, at / doc.width);
                [value, value, value, 255]
            },
            |pixels| pixels.rgba(at),
        )
    };
    let seed = y * doc.width + x;
    let seed_color = read(seed);
    let region = fill::region_mask(
        [doc.width, doc.height],
        seed,
        request.options,
        |at| {
            selected(doc, at % doc.width, at / doc.width) > 0
                && fill::equivalent(&read(at), &seed_color, request.tolerance)
        },
        cancel,
    )?;
    let value = Target::Mask(id).params(request.color, false, false).0[0];
    if request.opacity == 1.0
        && region
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum::<usize>()
            == doc.width * doc.height
        && whole_selection(doc, cancel)?
    {
        return constant(before, value, cancel);
    }
    let mut after = before.clone();
    let mut changed = false;
    {
        let mut writer = after.writer();
        for (base, word) in region.iter().enumerate() {
            if base & 15 == 0 {
                fill::check_cancel(cancel)?;
            }
            let mut bits = *word;
            while bits != 0 {
                let at = base * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let (x, y) = (at % doc.width, at / doc.width);
                let amount = request.opacity * f32::from(selected(doc, x, y)) / 255.0;
                if amount > 0.0 {
                    changed |= writer.paint(x, y, value, amount);
                }
            }
        }
        writer.reclaim();
    }
    fill::check_cancel(cancel)?;
    Ok(changed.then_some(after))
}

/// Paint antialiased shape coverage into a mask with the same selection math as
/// opaque, alpha-locked grayscale artwork. A mask eraser supplies white here.
pub(crate) fn paint_shape(
    doc: &Document,
    index: usize,
    id: LayerId,
    region: &crate::selection_mask::Mask,
    value: u8,
    opacity: f32,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Mask>, String> {
    fill::check_cancel(cancel)?;
    let before = source(doc, index, id)?;
    if region.dimensions() != [doc.width, doc.height] {
        return Err("Mask shape dimensions differ from the canvas".into());
    }
    let Some(bounds) = region.bounds() else {
        return Ok(None);
    };
    if opacity == 1.0 && whole_selection(doc, cancel)? {
        let mut full = true;
        for bytes in region.bytes().chunks(4096) {
            fill::check_cancel(cancel)?;
            if bytes.iter().any(|b| *b != 255) {
                full = false;
                break;
            }
        }
        if full {
            return constant(before, value, cancel);
        }
    }
    let mut after = before.clone();
    let mut changed = false;
    {
        let mut writer = after.writer();
        for y in bounds.top..bounds.bottom {
            fill::check_cancel(cancel)?;
            for x in bounds.left..bounds.right {
                let coverage =
                    f32::from(region.at(x, y)) / 255.0 * (f32::from(selected(doc, x, y)) / 255.0);
                if coverage > 0.0 {
                    changed |= writer.paint(x, y, value, opacity * coverage);
                }
            }
        }
        writer.reclaim_region([bounds.left, bounds.top, bounds.right, bounds.bottom]);
    }
    fill::check_cancel(cancel)?;
    Ok(changed.then_some(after))
}
