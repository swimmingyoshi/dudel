//! Bounded derived preview for the experimental indexed schema. No image exports.
use crate::{document::Document, indexed_tile};
pub(crate) const LIMIT: usize = 256;
pub(crate) const MAX_BYTES: usize = 32 + LIMIT * LIMIT * 4;
pub(crate) struct Thumbnail {
    pub(crate) size: [u16; 2],
    pub(crate) rgba: Vec<u8>,
}
pub(crate) fn size([width, height]: [usize; 2]) -> Result<[u16; 2], String> {
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || width * height > crate::document::MAX_PIXELS
    {
        return Err("Unsupported indexed thumbnail source geometry".into());
    }
    let longest = width.max(height);
    Ok(if longest <= LIMIT {
        [width as u16, height as u16]
    } else {
        [
            (width * LIMIT / longest).max(1) as u16,
            (height * LIMIT / longest).max(1) as u16,
        ]
    })
}
/// Nearest source pixel center; native white-canvas composition, no overlays or
/// selection outline. Work/output are bounded by 256 squared samples, not canvas.
pub(crate) fn encode(doc: &Document, cancel: &impl Fn() -> bool) -> Result<Vec<u8>, String> {
    if cancel() {
        return Err("Indexed thumbnail canceled".into());
    }
    doc.validate()?;
    if doc
        .layers
        .iter()
        .any(|layer| !layer.folder && !layer.pixels.has_geometry(doc.width, doc.height))
    {
        return Err("Indexed thumbnail source pixel geometry differs".into());
    }
    let dimensions = size([doc.width, doc.height])?;
    let expanded = if doc.needs_expansion() {
        Some(crate::layer_effects::expand(doc, false, cancel)?)
    } else {
        None
    };
    let doc = expanded.as_ref().unwrap_or(doc);
    let [width, height] = dimensions.map(usize::from);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        if cancel() {
            return Err("Indexed thumbnail canceled".into());
        }
        let sy = ((2 * y + 1) * doc.height / (2 * height)).min(doc.height - 1);
        for x in 0..width {
            let sx = ((2 * x + 1) * doc.width / (2 * width)).min(doc.width - 1);
            rgba.extend(doc.composite_rgba_pixel(sy * doc.width + sx));
        }
    }
    if cancel() {
        return Err("Indexed thumbnail canceled".into());
    }
    indexed_tile::encode(dimensions, &rgba)?
        .ok_or("Opaque indexed thumbnail unexpectedly empty".into())
}
pub(crate) fn decode(encoded: &[u8], canvas: [usize; 2]) -> Result<Thumbnail, String> {
    let size = size(canvas)?;
    if encoded.len() > MAX_BYTES {
        return Err("Indexed thumbnail record exceeds budget".into());
    }
    let rgba = indexed_tile::decode(encoded, size)?;
    if rgba.chunks_exact(4).any(|pixel| pixel[3] != 255) {
        return Err("Indexed thumbnail is not an opaque native canvas preview".into());
    }
    Ok(Thumbnail { size, rgba })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_preview_includes_native_layer_effects() {
        use crate::layer_effects::{Effect, LayerEffect, Paint};
        let mut doc = Document::new(1, 1);
        doc.layers[0].pixels = vec![255, 0, 0, 255].into();
        let mut overlay = Effect::preset("Color Overlay").unwrap();
        overlay.paint = Paint::Color([0, 0, 255]);
        doc.layers[0].effects.push(LayerEffect::new(overlay));
        let encoded = encode(&doc, &|| false).unwrap();
        assert_eq!(decode(&encoded, [1, 1]).unwrap().rgba, vec![0, 0, 255, 255]);
    }
    #[test]
    fn preview_matches_native_canvas_pixels_with_folders_clipping_and_byte_opacity() {
        let mut doc = crate::indexed_document::fixture();
        doc.layers[1].visible = true;
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(256)
            .copy_from_slice(&[20, 60, 100, 255]);
        let before = doc.clone();
        let encoded = encode(&doc, &|| false).unwrap();
        let thumbnail = decode(&encoded, [doc.width, doc.height]).unwrap();
        let reference = doc.composite();
        let [w, h] = thumbnail.size.map(usize::from);
        for y in 0..h {
            for x in 0..w {
                let sx = (2 * x + 1) * doc.width / (2 * w);
                let sy = (2 * y + 1) * doc.height / (2 * h);
                assert_eq!(
                    &thumbnail.rgba[(y * w + x) * 4..(y * w + x + 1) * 4],
                    &reference[(sy * doc.width + sx) * 4..(sy * doc.width + sx + 1) * 4]
                );
            }
        }
        assert!(crate::indexed_document::equivalent(&before, &doc).unwrap());
    }
    #[test]
    fn dimensions_never_upscale_and_thin_large_canvases_stay_bounded() {
        for (canvas, expected) in [
            ([1, 1], [1, 1]),
            ([7, 13], [7, 13]),
            ([4096, 1], [256, 1]),
            ([1, 4096], [1, 256]),
            ([2048, 2048], [256, 256]),
            ([257, 259], [254, 256]),
        ] {
            assert_eq!(size(canvas).unwrap(), expected);
        }
        for canvas in [[0, 1], [1, 0], [16385, 1], [10001, 10000]] {
            assert!(size(canvas).is_err());
        }
        let doc = Document::new(7, 13);
        let encoded = encode(&doc, &|| false).unwrap();
        let thumbnail = decode(&encoded, [7, 13]).unwrap();
        assert!(thumbnail.rgba.iter().all(|b| *b == 255));
    }
    #[test]
    fn canceled_corrupt_or_transparent_previews_never_return_partial_pixels() {
        let doc = Document::new(32, 32);
        assert!(encode(&doc, &|| true).is_err());
        let checks = std::cell::Cell::new(0);
        assert!(
            encode(&doc, &|| {
                let n = checks.get() + 1;
                checks.set(n);
                n >= 3
            })
            .is_err()
        );
        let encoded = encode(&doc, &|| false).unwrap();
        for end in 0..encoded.len() {
            assert!(decode(&encoded[..end], [32, 32]).is_err());
        }
        assert!(decode(&encoded, [31, 32]).is_err());
        let transparent = indexed_tile::encode([1, 1], &[1, 2, 3, 0])
            .unwrap()
            .unwrap();
        assert!(decode(&transparent, [1, 1]).is_err());
    }
}
