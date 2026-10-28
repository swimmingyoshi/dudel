//! Grayscale mask interchange; OS transactions and barriers live in clipboard.
use crate::{
    clipboard::Raster,
    document::{Document, LayerId},
    layer_mask::Mask,
    paint_target::Target,
};
fn resolve(doc: &Document, layer: LayerId, mask: LayerId, write: bool) -> Result<usize, String> {
    doc.validate()?;
    let index = doc
        .layers
        .iter()
        .position(|l| l.id == layer)
        .ok_or("Clipboard mask layer no longer exists")?;
    if !Target::Mask(mask).valid(&doc.layers[index], [doc.width, doc.height]) {
        return Err("Clipboard mask was removed or replaced".into());
    }
    if write && let Some(error) = Target::Mask(mask).blocker(doc, index, false) {
        return Err(error.into());
    }
    Ok(index)
}
fn selected(doc: &Document, x: usize, y: usize) -> u8 {
    if doc.selection.is_some_and(|s| !s.contains(x, y)) {
        0
    } else {
        doc.selection_mask.as_ref().map_or(255, |m| m.at(x, y))
    }
}
pub(crate) fn copy(
    doc: &Document,
    layer: LayerId,
    mask: LayerId,
    cut: bool,
) -> Result<(Raster, Option<Document>), String> {
    let index = resolve(doc, layer, mask, cut)?;
    if doc.selection_mask.is_some() && doc.selection.is_none() {
        return Err("Selection is empty".into());
    }
    let area = doc.selection.unwrap_or(crate::selection::Selection {
        left: 0,
        top: 0,
        right: doc.width,
        bottom: doc.height,
    });
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    let mut rgba = vec![0; width * height * 4];
    let before = &doc.layers[index].mask.as_ref().unwrap().coverage;
    for y in 0..height {
        for x in 0..width {
            let coverage = selected(doc, area.left + x, area.top + y);
            if coverage > 0 {
                let value = before.at(area.left + x, area.top + y);
                rgba[(y * width + x) * 4..(y * width + x) * 4 + 4]
                    .copy_from_slice(&[value, value, value, coverage]);
            }
        }
    }
    let raster = Raster {
        width,
        height,
        rgba,
    };
    raster.validate()?;
    let updated = if cut {
        if let Some(coverage) = crate::mask_fill::clear(doc, index, mask, false, None)? {
            let mut updated = doc.clone();
            updated.layers[index].mask.as_mut().unwrap().coverage = coverage;
            updated.validate()?;
            Some(updated)
        } else {
            None
        }
    } else {
        None
    };
    Ok((raster, updated))
}
pub(crate) fn paste(
    doc: &Document,
    raster: &Raster,
    layer: LayerId,
    mask: LayerId,
) -> Result<Option<Document>, String> {
    let index = resolve(doc, layer, mask, true)?;
    raster.validate()?;
    if raster.width > doc.width || raster.height > doc.height {
        return Err("Clipboard image does not fit this canvas; use a larger canvas".into());
    }
    if doc.selection_mask.is_some() && doc.selection.is_none() {
        return Ok(None);
    }
    let area = doc.selection.unwrap_or(crate::selection::Selection {
        left: 0,
        top: 0,
        right: doc.width,
        bottom: doc.height,
    });
    let left =
        ((area.left + area.right).saturating_sub(raster.width) / 2).min(doc.width - raster.width);
    let top = ((area.top + area.bottom).saturating_sub(raster.height) / 2)
        .min(doc.height - raster.height);
    let before = &doc.layers[index].mask.as_ref().unwrap().coverage;
    let gray = |p: &[u8]| {
        Target::Mask(mask)
            .params([p[0], p[1], p[2]], false, false)
            .0[0]
    };
    let sample = &raster.rgba[..4];
    let value = gray(sample);
    let constant = left == 0
        && top == 0
        && raster.width == doc.width
        && raster.height == doc.height
        && (sample[3] == 255 || before.payload_bytes() == 0)
        && crate::mask_fill::whole_selection(doc, None)?
        && raster
            .rgba
            .chunks_exact(4)
            .all(|p| p[3] == sample[3] && gray(p) == value);
    let coverage = if constant {
        let alpha = f32::from(sample[3]) / 255.0;
        let value =
            (f32::from(before.at(0, 0)) * (1.0 - alpha) + f32::from(value) * alpha).round() as u8;
        let after = Mask::new(doc.width, doc.height, value)?;
        if before.equivalent(&after, None)? {
            return Ok(None);
        }
        after
    } else {
        let mut after = before.clone();
        let mut changed = false;
        {
            let mut writer = after.writer();
            for y in 0..raster.height {
                for x in 0..raster.width {
                    let pixel =
                        &raster.rgba[(y * raster.width + x) * 4..(y * raster.width + x) * 4 + 4];
                    let alpha = f32::from(pixel[3]) / 255.0
                        * (f32::from(selected(doc, left + x, top + y)) / 255.0);
                    if alpha > 0.0 {
                        changed |= writer.paint(left + x, top + y, gray(pixel), alpha);
                    }
                }
            }
            writer.reclaim_region([left, top, left + raster.width, top + raster.height]);
        }
        if !changed {
            return Ok(None);
        }
        after
    };
    let mut updated = doc.clone();
    updated.layers[index].mask.as_mut().unwrap().coverage = coverage;
    updated.validate()?;
    Ok(Some(updated))
}
