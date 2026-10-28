//! Bounded comparison against a full-size CSP PNG preview. No image exports.
use crate::document::Document;
use image::ImageDecoder;
#[derive(serde::Serialize)]
pub(crate) struct RgbaComparison {
    exact_pixels: usize,
    total_pixels: usize,
    max_channel_difference: u8,
    mean_absolute_channel_difference: f64,
    hidden_rgb_normalized: bool,
}
/// Numerical research comparison only. Hidden RGB normalization is explicit;
/// it must never mutate either source or become an import eligibility check.
pub(crate) fn compare_pixels(
    actual: &crate::pixels::Pixels,
    expected: &crate::pixels::Pixels,
    normalize_hidden: bool,
) -> Result<RgbaComparison, String> {
    if actual.len() != expected.len()
        || actual.is_empty()
        || !actual.len().is_multiple_of(4)
        || actual.len() > 4_194_304 * 4
    {
        return Err("Raster comparison geometry exceeds bounds or differs".into());
    }
    let total = actual.len() / 4;
    let mut exact = 0;
    let mut maximum = 0;
    let mut sum = 0u64;
    for index in 0..total {
        let mut a = actual.rgba(index);
        let mut b = expected.rgba(index);
        if normalize_hidden {
            if a[3] == 0 {
                a[..3].fill(0);
            }
            if b[3] == 0 {
                b[..3].fill(0);
            }
        }
        exact += usize::from(a == b);
        for channel in 0..4 {
            let delta = a[channel].abs_diff(b[channel]);
            maximum = maximum.max(delta);
            sum += delta as u64;
        }
    }
    Ok(RgbaComparison {
        exact_pixels: exact,
        total_pixels: total,
        max_channel_difference: maximum,
        mean_absolute_channel_difference: sum as f64 / actual.len() as f64,
        hidden_rgb_normalized: normalize_hidden,
    })
}
/// Experimental destructive conversion of an unpublished study document only.
/// Never call this on editor/import state: source alpha and opacity must remain
/// independently editable there. Folder opacity semantics are not modeled here.
pub(crate) fn bake_leaf_opacity_hypothesis(doc: &mut Document) {
    for layer in &mut doc.layers {
        if layer.folder || layer.opacity == 1.0 {
            continue;
        }
        for index in 0..doc.width * doc.height {
            let alpha = layer.pixels.rgba(index)[3];
            let effective = (alpha as f32 * layer.opacity).floor() as u8;
            if effective != alpha {
                layer.pixels.writer().rgba_mut(index)[3] = effective;
            }
        }
        layer.opacity = 1.0;
    }
}
#[derive(serde::Serialize)]
pub(crate) struct Comparison {
    preview: [u32; 2],
    compared_full_size: bool,
    exact_pixels: Option<usize>,
    total_pixels: usize,
    max_channel_difference: Option<u8>,
    mean_absolute_channel_difference: Option<f64>,
}
pub(crate) fn compare(
    encoded: &[u8],
    declared: [u32; 2],
    doc: &Document,
) -> Result<Comparison, String> {
    if encoded.len() > 16 * 1024 * 1024
        || declared.contains(&0)
        || declared[0] > 4096
        || declared[1] > 4096
        || declared[0] as u64 * declared[1] as u64 > 4_194_304
    {
        return Err("CSP preview exceeds comparison bounds".into());
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(encoded), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|error| error.to_string())?;
    if decoder.dimensions() != (declared[0], declared[1]) {
        return Err("CSP preview dimensions disagree with PNG header".into());
    }
    let mut result = Comparison {
        preview: declared,
        compared_full_size: false,
        exact_pixels: None,
        total_pixels: doc.width * doc.height,
        max_channel_difference: None,
        mean_absolute_channel_difference: None,
    };
    if declared != [doc.width as u32, doc.height as u32] {
        return Ok(result);
    }
    let stored = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| error.to_string())?
        .into_rgba8()
        .into_raw();
    let actual = doc.composite_transparent();
    let mut exact = 0;
    let mut maximum = 0;
    let mut sum = 0u64;
    for (a, b) in actual.chunks_exact(4).zip(stored.chunks_exact(4)) {
        exact += usize::from(a == b);
        for (a, b) in a.iter().zip(b) {
            let delta = a.abs_diff(*b);
            maximum = maximum.max(delta);
            sum += delta as u64;
        }
    }
    result.compared_full_size = true;
    result.exact_pixels = Some(exact);
    result.max_channel_difference = Some(maximum);
    result.mean_absolute_channel_difference = Some(sum as f64 / actual.len() as f64);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_and_visible_cache_comparisons_are_explicit_and_preserve_sources() {
        use crate::pixels::Pixels;
        let a = Pixels::from(vec![10, 20, 30, 0, 50, 60, 70, 128]);
        let b = Pixels::from(vec![90, 80, 70, 0, 50, 60, 70, 128]);
        assert_eq!(compare_pixels(&a, &b, false).unwrap().exact_pixels, 1);
        assert_eq!(compare_pixels(&a, &b, true).unwrap().exact_pixels, 2);
        assert_eq!(a.rgba(0), [10, 20, 30, 0]);
        assert_eq!(b.rgba(0), [90, 80, 70, 0]);
        assert!(compare_pixels(&a, &Pixels::from(vec![0; 4]), false).is_err());
        assert!(
            compare_pixels(&Pixels::from(Vec::new()), &Pixels::from(Vec::new()), true).is_err()
        );
    }
    fn png(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
        use image::ImageEncoder;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(pixels, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }
    #[test]
    fn exact_and_channel_error_metrics_cover_all_rgba_bytes() {
        let mut doc = Document::new(2, 1);
        doc.layers[0].pixels = crate::pixels::Pixels::from(vec![255, 0, 0, 255, 0, 255, 0, 128]);
        let actual = doc.composite_transparent();
        let data = png(2, 1, &actual);
        let exact = compare(&data, [2, 1], &doc).unwrap();
        assert_eq!(exact.exact_pixels, Some(2));
        assert_eq!(exact.max_channel_difference, Some(0));
        let mut changed = actual;
        changed[5] -= 1;
        let result = compare(&png(2, 1, &changed), [2, 1], &doc).unwrap();
        assert_eq!(result.exact_pixels, Some(1));
        assert_eq!(result.max_channel_difference, Some(1));
        assert_eq!(result.mean_absolute_channel_difference, Some(0.125));
    }
    #[test]
    fn reduced_previews_do_not_claim_full_size_comparison_and_bad_headers_fail() {
        let doc = Document::new(2, 2);
        let data = png(1, 1, &[255; 4]);
        let result = compare(&data, [1, 1], &doc).unwrap();
        assert!(!result.compared_full_size);
        assert!(result.exact_pixels.is_none());
        assert!(compare(&data, [2, 2], &doc).is_err());
        assert!(compare(&data, [4096, 4096], &doc).is_err());
        assert!(compare(b"broken", [1, 1], &doc).is_err());
    }
    #[test]
    fn opacity_hypothesis_is_local_preserves_hidden_rgb_and_uses_floor() {
        let mut original = Document::new(2, 1);
        original.layers[0].opacity = 156. / 256.;
        original.layers[0].pixels =
            crate::pixels::Pixels::from(vec![10, 20, 30, 1, 40, 50, 60, 128]);
        let mut study = original.clone();
        bake_leaf_opacity_hypothesis(&mut study);
        assert_eq!(study.layers[0].pixels.rgba(0), [10, 20, 30, 0]);
        assert_eq!(study.layers[0].pixels.rgba(1), [40, 50, 60, 78]);
        assert_eq!(study.layers[0].opacity, 1.0);
        assert_eq!(original.layers[0].pixels.rgba(0), [10, 20, 30, 1]);
        assert_eq!(original.layers[0].opacity, 156. / 256.);
    }
}
