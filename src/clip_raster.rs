//! Sparse raster leaf conversion. All state remains local until complete validation.
use crate::{
    clip_attributes::Attributes,
    clip_blocks::{self, Budget},
    pixels::Pixels,
};
use std::io::Read;

pub(crate) struct Placement {
    pub(crate) canvas: [usize; 2],
    pub(crate) layer_offset: [i64; 2],
    pub(crate) render_offset: [i64; 2],
}
#[derive(serde::Serialize)]
pub(crate) struct Statistics {
    rgba_fnv1a: String,
    nonzero_pixels: usize,
    partial_alpha_pixels: usize,
    hidden_rgb_pixels: usize,
    retained_pixel_payload_bytes: usize,
}
pub(crate) fn statistics(pixels: &Pixels) -> Statistics {
    let mut hash = 0xcbf29ce484222325u64;
    let mut nonzero = 0;
    let mut partial = 0;
    let mut hidden = 0;
    for index in 0..pixels.len() / 4 {
        let rgba = pixels.rgba(index);
        for byte in rgba {
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
        }
        nonzero += usize::from(rgba != [0; 4]);
        partial += usize::from(rgba[3] > 0 && rgba[3] < 255);
        hidden += usize::from(rgba[3] == 0 && rgba[..3] != [0; 3]);
    }
    Statistics {
        rgba_fnv1a: format!("{hash:016x}"),
        nonzero_pixels: nonzero,
        partial_alpha_pixels: partial,
        hidden_rgb_pixels: hidden,
        retained_pixel_payload_bytes: pixels.capacity(),
    }
}

pub(crate) fn decode(
    reader: &mut impl Read,
    body_bytes: u64,
    attributes: &Attributes,
    placement: Placement,
    budget: &mut Budget,
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let [width, height] = placement.canvas;
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || width * height > crate::document::MAX_PIXELS
    {
        return Err("Clip raster canvas exceeds Dudel limits".into());
    }
    if !attributes.transparent_default() {
        return Err("Unsupported clip raster default color".into());
    }
    let mut offset = [0i64; 2];
    for axis in 0..2 {
        offset[axis] = placement.layer_offset[axis]
            .checked_add(placement.render_offset[axis])
            .ok_or("Clip raster combined offset overflow")?;
        offset[axis]
            .checked_add([attributes.width, attributes.height][axis] as i64)
            .ok_or("Clip raster extent overflow")?;
    }
    let mut pixels = Pixels::sparse(width, height);
    let gray = attributes.color_channels() == 1;
    clip_blocks::decode_layout(
        reader,
        body_bytes,
        &attributes.block_sizes,
        attributes.tile_bytes(),
        budget,
        cancelled,
        &mut |index, raw| {
            let [left, top] = attributes.tile_origin(index)?;
            let mut writer = pixels.writer();
            for y in 0..256usize.min(attributes.height as usize - top) {
                if cancelled() {
                    return Err("Clip raster decoding canceled".into());
                }
                let dy = offset[1] + (top + y) as i64;
                if dy < 0 || dy >= height as i64 {
                    continue;
                }
                for x in 0..256usize.min(attributes.width as usize - left) {
                    let dx = offset[0] + (left + x) as i64;
                    if dx < 0 || dx >= width as i64 {
                        continue;
                    }
                    let source = y * 256 + x;
                    // Separate coverage plane followed by straight B/G/R/unused,
                    // or one gray byte. Keep hidden RGB, and never multiply color
                    // by coverage here.
                    let rgba = if gray {
                        let value = raw[65536 + source];
                        [value, value, value, raw[source]]
                    } else {
                        let color = 65536 + source * 4;
                        [raw[color + 2], raw[color + 1], raw[color], raw[source]]
                    };
                    if rgba != [0; 4] {
                        writer
                            .rgba_mut_at(width, dx as usize, dy as usize)
                            .copy_from_slice(&rgba);
                    }
                }
            }
            Ok(())
        },
    )?;
    Ok(pixels)
}

/// A layer mask bitmap (one coverage byte per pixel) placed like a raster;
/// outside the bitmap the mask hides. Unwritten blocks hide, or show when
/// the mask was created showing everything.
pub(crate) fn decode_mask(
    reader: &mut impl Read,
    body_bytes: u64,
    attributes: &Attributes,
    placement: Placement,
    budget: &mut Budget,
    cancelled: &impl Fn() -> bool,
) -> Result<crate::layer_mask::Mask, String> {
    let [width, height] = placement.canvas;
    let opaque = attributes.opaque_default();
    if !attributes.is_mask() || !(attributes.transparent_default() || opaque) {
        return Err("Unsupported clip mask layout".into());
    }
    let combined = |axis: usize| {
        placement.layer_offset[axis]
            .checked_add(placement.render_offset[axis])
            .ok_or("Clip mask combined offset overflow")
    };
    let offset = [combined(0)?, combined(1)?];
    // The canvas part the bitmap covers.
    let span = |axis: usize, size: u32, limit: usize| {
        let start = offset[axis].clamp(0, limit as i64) as usize;
        let end = (offset[axis] + i64::from(size)).clamp(0, limit as i64) as usize;
        start..end
    };
    let (columns, rows) = (
        span(0, attributes.width, width),
        span(1, attributes.height, height),
    );
    let whole = columns == (0..width) && rows == (0..height);
    let mut mask =
        crate::layer_mask::Mask::new(width, height, if opaque && whole { 255 } else { 0 })?;
    if opaque && !whole {
        let mut writer = mask.writer();
        for y in rows {
            if cancelled() {
                return Err("Clip mask decoding canceled".into());
            }
            for x in columns.clone() {
                writer.set(x, y, 255);
            }
        }
    }
    clip_blocks::decode_layout(
        reader,
        body_bytes,
        &attributes.block_sizes,
        attributes.tile_bytes(),
        budget,
        cancelled,
        &mut |index, raw| {
            let [left, top] = attributes.tile_origin(index)?;
            let mut writer = mask.writer();
            for y in 0..256usize.min(attributes.height as usize - top) {
                let dy = offset[1] + (top + y) as i64;
                if dy < 0 || dy >= height as i64 {
                    continue;
                }
                for x in 0..256usize.min(attributes.width as usize - left) {
                    let dx = offset[0] + (left + x) as i64;
                    let value = raw[y * 256 + x];
                    if (value != 0 || opaque) && dx >= 0 && dx < width as i64 {
                        writer.set(dx as usize, dy as usize, value);
                    }
                }
            }
            Ok(())
        },
    )?;
    Ok(mask)
}

/// An image material layer's original picture (`source`, `size`) placed on
/// the canvas by where its top-left, top-right and bottom-left corners land.
/// Bilinear in premultiplied color (clamped to the picture, so its edge does
/// not fade), averaged over the footprint when the picture shrinks; pixels
/// the picture's boundary crosses are covered in proportion.
pub(crate) fn place(
    source: &Pixels,
    size: [usize; 2],
    corners: [[f64; 2]; 3],
    canvas: [usize; 2],
    cancelled: &impl Fn() -> bool,
) -> Result<Pixels, String> {
    let ([sw, sh], [cw, ch]) = (size, canvas);
    let flat = source.region(sw, [0, 0, sw, sh]);
    let [origin, right, down] = corners;
    // Canvas step per source pixel along the picture's rows and columns.
    let ax = [
        (right[0] - origin[0]) / sw as f64,
        (right[1] - origin[1]) / sw as f64,
    ];
    let ay = [
        (down[0] - origin[0]) / sh as f64,
        (down[1] - origin[1]) / sh as f64,
    ];
    let det = ax[0] * ay[1] - ax[1] * ay[0];
    if !det.is_finite() || det.abs() < 1e-9 {
        return Err("Degenerate clip image placement".into());
    }
    let far = [
        right[0] + down[0] - origin[0],
        right[1] + down[1] - origin[1],
    ];
    let span = |axis: usize, limit: usize| {
        let values = [origin[axis], right[axis], down[axis], far[axis]];
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (
            low.floor().clamp(0.0, limit as f64) as usize,
            high.ceil().clamp(0.0, limit as f64) as usize,
        )
    };
    let ((left, end_x), (top, end_y)) = (span(0, cw), span(1, ch));
    let mut pixels = Pixels::sparse(cw, ch);
    if left >= end_x || top >= end_y {
        return Ok(pixels);
    }
    let scale = ax[0].hypot(ax[1]).min(ay[0].hypot(ay[1]));
    let taps = if scale < 1.0 {
        ((1.0 / scale).ceil() as usize).min(8)
    } else {
        1
    };
    let texel = |x: i64, y: i64| -> [f64; 4] {
        let (x, y) = (x.clamp(0, sw as i64 - 1), y.clamp(0, sh as i64 - 1));
        let i = (y as usize * sw + x as usize) * 4;
        let a = f64::from(flat[i + 3]) / 255.0;
        [
            f64::from(flat[i]) * a,
            f64::from(flat[i + 1]) * a,
            f64::from(flat[i + 2]) * a,
            a,
        ]
    };
    // Canvas point to source position, in source pixels.
    let source_at = |px: f64, py: f64| {
        let (dx, dy) = (px - origin[0], py - origin[1]);
        (
            (dx * ay[1] - dy * ay[0]) / det,
            (ax[0] * dy - ax[1] * dx) / det,
        )
    };
    const EDGE: usize = 4;
    let mut row = vec![0u8; (end_x - left) * 4];
    for y in top..end_y {
        if cancelled() {
            return Err("Clip image placement canceled".into());
        }
        for (column, x) in (left..end_x).enumerate() {
            let mut inside = 0;
            for ey in 0..EDGE {
                for ex in 0..EDGE {
                    let (u, v) = source_at(
                        x as f64 + (ex as f64 + 0.5) / EDGE as f64,
                        y as f64 + (ey as f64 + 0.5) / EDGE as f64,
                    );
                    inside +=
                        usize::from((0.0..sw as f64).contains(&u) && (0.0..sh as f64).contains(&v));
                }
            }
            let out = &mut row[column * 4..column * 4 + 4];
            if inside == 0 {
                out.copy_from_slice(&[0; 4]);
                continue;
            }
            let coverage = inside as f64 / (EDGE * EDGE) as f64;
            let mut sum = [0.0; 4];
            for ty in 0..taps {
                for tx in 0..taps {
                    let (u, v) = source_at(
                        x as f64 + (tx as f64 + 0.5) / taps as f64,
                        y as f64 + (ty as f64 + 0.5) / taps as f64,
                    );
                    // Pixel centers sit at +0.5.
                    let (u, v) = (u - 0.5, v - 0.5);
                    let (x0, y0) = (u.floor(), v.floor());
                    let (fx, fy) = (u - x0, v - y0);
                    let (x0, y0) = (x0 as i64, y0 as i64);
                    for (sx, sy, w) in [
                        (x0, y0, (1.0 - fx) * (1.0 - fy)),
                        (x0 + 1, y0, fx * (1.0 - fy)),
                        (x0, y0 + 1, (1.0 - fx) * fy),
                        (x0 + 1, y0 + 1, fx * fy),
                    ] {
                        let t = texel(sx, sy);
                        for c in 0..4 {
                            sum[c] += t[c] * w;
                        }
                    }
                }
            }
            let n = (taps * taps) as f64;
            let alpha = sum[3] / n * coverage;
            if alpha * 255.0 < 0.5 {
                out.copy_from_slice(&[0; 4]);
                continue;
            }
            for c in 0..3 {
                out[c] = (sum[c] / sum[3]).round().clamp(0.0, 255.0) as u8;
            }
            out[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        pixels
            .writer()
            .write_region(cw, [left, y, end_x, y + 1], &row);
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placed_pictures_scale_and_move_by_their_corners() {
        // A 2 x 2 picture, left column red, right column blue, doubled and
        // moved to (4, 2) on a 10 x 8 canvas.
        let mut source = Pixels::sparse(2, 2);
        for y in 0..2 {
            source
                .writer()
                .write_region(2, [0, y, 2, y + 1], &[255, 0, 0, 255, 0, 0, 255, 255]);
        }
        let placed = place(
            &source,
            [2, 2],
            [[4.0, 2.0], [8.0, 2.0], [4.0, 6.0]],
            [10, 8],
            &|| false,
        )
        .unwrap();
        let at = |x: usize, y: usize| placed.rgba(y * 10 + x);
        assert_eq!(at(4, 3), [255, 0, 0, 255]);
        assert_eq!(at(7, 4), [0, 0, 255, 255]);
        assert_eq!(at(3, 3)[3], 0);
        assert_eq!(at(8, 3)[3], 0);
        // Between the columns the colors blend; color never darkens toward
        // the transparent outside (premultiplied sampling).
        assert!(at(5, 3)[0] > 0 && at(6, 3)[2] > 0);
        assert_eq!(at(4, 2)[0], 255);
        // Shrinking averages: half size, one pixel per column pair.
        let small = place(
            &source,
            [2, 2],
            [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            [4, 4],
            &|| false,
        )
        .unwrap();
        let [r, _, b, a] = small.rgba(0);
        assert!(r > 100 && b > 100 && a == 255, "{r} {b} {a}");
        assert!(place(&source, [2, 2], [[0.0; 2]; 3], [4, 4], &|| false).is_err());
    }
    use std::io::{Cursor, Write};
    fn body(raws: &[Option<Vec<u8>>]) -> Vec<u8> {
        let streams: Vec<_> = raws
            .iter()
            .map(|raw| {
                raw.as_ref().map(|raw| {
                    let mut z =
                        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                    z.write_all(raw).unwrap();
                    z.finish().unwrap()
                })
            })
            .collect();
        clip_blocks::tests::fixture(&streams)
    }
    fn raw() -> Vec<u8> {
        vec![0; 327680]
    }
    fn pixel(raw: &mut [u8], x: usize, y: usize, rgba: [u8; 4], unused: u8) {
        let i = y * 256 + x;
        raw[i] = rgba[3];
        raw[65536 + i * 4..65536 + i * 4 + 4].copy_from_slice(&[rgba[2], rgba[1], rgba[0], unused]);
    }
    fn attributes(w: u32, h: u32, body: &[u8]) -> Attributes {
        let mut at = 8;
        let mut sizes = Vec::new();
        for _ in 0..w.div_ceil(256) * h.div_ceil(256) {
            let n = u32::from_be_bytes(body[at..at + 4].try_into().unwrap());
            sizes.push(n);
            at += n as usize;
        }
        crate::clip_attributes::parse(&crate::clip_attributes::fixture(
            w,
            h,
            &[20, 0, 0, 0, 4],
            &sizes,
        ))
        .unwrap()
    }
    fn run(
        body: &[u8],
        attrs: &Attributes,
        canvas: [usize; 2],
        layer: [i64; 2],
        render: [i64; 2],
    ) -> Result<Pixels, String> {
        decode(
            &mut Cursor::new(body),
            body.len() as u64,
            attrs,
            Placement {
                canvas,
                layer_offset: layer,
                render_offset: render,
            },
            &mut Budget::default(),
            &|| false,
        )
    }
    #[test]
    fn masks_decode_one_coverage_byte_placed_by_offsets() {
        let mut tile = vec![0u8; 65536];
        tile[0] = 255;
        tile[3 * 256 + 2] = 128;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&tile).unwrap();
        let body = clip_blocks::tests::fixture_sized(&[Some(z.finish().unwrap())], 65536);
        let size = u32::from_be_bytes(body[8..12].try_into().unwrap());
        let attrs = crate::clip_attributes::parse(&crate::clip_attributes::fixture_packed(
            8,
            8,
            &[20, 0, 0, 0, 4],
            &[size],
            crate::clip_attributes::MASK_PACKING,
        ))
        .unwrap();
        assert!(attrs.is_mask() && attrs.tile_bytes() == 65536);
        let mask = decode_mask(
            &mut Cursor::new(&body),
            body.len() as u64,
            &attrs,
            Placement {
                canvas: [10, 10],
                layer_offset: [1, 1],
                render_offset: [0, 0],
            },
            &mut Budget::default(),
            &|| false,
        )
        .unwrap();
        assert_eq!((mask.at(1, 1), mask.at(3, 4), mask.at(0, 0)), (255, 128, 0));
        // A mask created showing everything: unwritten blocks show, written
        // ones keep their zeros, and outside the bitmap still hides.
        let opaque = crate::clip_attributes::parse(&crate::clip_attributes::fixture_packed(
            8,
            8,
            &[20, 1, u32::MAX, 0, 4],
            &[size],
            crate::clip_attributes::MASK_PACKING,
        ))
        .unwrap();
        let place = |offset: i64| Placement {
            canvas: [10, 10],
            layer_offset: [offset, offset],
            render_offset: [0, 0],
        };
        let painted = decode_mask(
            &mut Cursor::new(&body),
            body.len() as u64,
            &opaque,
            place(1),
            &mut Budget::default(),
            &|| false,
        )
        .unwrap();
        assert_eq!(
            (
                painted.at(1, 1),
                painted.at(3, 4),
                painted.at(2, 1),
                painted.at(0, 0)
            ),
            (255, 128, 0, 0)
        );
        let empty = clip_blocks::tests::fixture_sized(&[None], 65536);
        let unwritten = crate::clip_attributes::parse(&crate::clip_attributes::fixture_packed(
            10,
            10,
            &[20, 1, u32::MAX, 0, 4],
            &[u32::from_be_bytes(empty[8..12].try_into().unwrap())],
            crate::clip_attributes::MASK_PACKING,
        ))
        .unwrap();
        let shown = decode_mask(
            &mut Cursor::new(&empty),
            empty.len() as u64,
            &unwritten,
            place(0),
            &mut Budget::default(),
            &|| false,
        )
        .unwrap();
        assert!((0..10).all(|y| (0..10).all(|x| shown.at(x, y) == 255)));
        // Color layouts are not masks.
        assert!(
            decode_mask(
                &mut Cursor::new(&body),
                body.len() as u64,
                &attributes(8, 8, &body),
                Placement {
                    canvas: [10, 10],
                    layer_offset: [0, 0],
                    render_offset: [0, 0],
                },
                &mut Budget::default(),
                &|| false,
            )
            .is_err()
        );
    }
    #[test]
    fn known_channels_hidden_rgb_alpha_and_partial_edges_are_exact() {
        let mut first = raw();
        for (x, value) in [
            [255, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 0],
            [12, 34, 56, 1],
        ]
        .into_iter()
        .enumerate()
        {
            pixel(&mut first, x, 0, value, 255 - x as u8);
        }
        pixel(&mut first, 255, 255, [99, 88, 77, 66], 0);
        let mut edge = raw();
        pixel(&mut edge, 0, 0, [7, 8, 9, 10], 42);
        pixel(&mut edge, 1, 0, [90, 91, 92, 93], 99);
        let data = body(&[Some(first), Some(edge)]);
        let attrs = attributes(257, 1, &data);
        let result = run(&data, &attrs, [260, 2], [0, 0], [0, 0]).unwrap();
        assert_eq!(result.rgba(0), [255, 0, 0, 255]);
        assert_eq!(result.rgba(1), [0, 255, 0, 128]);
        assert_eq!(result.rgba(2), [0, 0, 255, 0]);
        assert_eq!(result.rgba(3), [12, 34, 56, 1]);
        assert_eq!(result.rgba(256), [7, 8, 9, 10]);
        assert_eq!(result.rgba(257), [0; 4]);
        assert_eq!(result.rgba(260), [0; 4]);
    }
    #[test]
    fn gray_layers_expand_one_color_byte_to_rgb_and_keep_alpha() {
        let mut tile = vec![0; 256 * 256 * 2];
        for (x, [gray, alpha]) in [[0u8, 255], [200, 128], [77, 0], [255, 1]]
            .into_iter()
            .enumerate()
        {
            tile[x] = alpha;
            tile[65536 + x] = gray;
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&tile).unwrap();
        let data = clip_blocks::tests::fixture_sized(&[Some(z.finish().unwrap())], tile.len());
        let size = u32::from_be_bytes(data[8..12].try_into().unwrap());
        let attrs = crate::clip_attributes::parse(&crate::clip_attributes::fixture_packed(
            4,
            1,
            &[20, 0, 0, 0, 4],
            &[size],
            crate::clip_attributes::GRAY_PACKING,
        ))
        .unwrap();
        let result = run(&data, &attrs, [4, 1], [0, 0], [0, 0]).unwrap();
        assert_eq!(result.rgba(0), [0, 0, 0, 255]);
        assert_eq!(result.rgba(1), [200, 200, 200, 128]);
        // Hidden gray under zero alpha is kept, like hidden RGB.
        assert_eq!(result.rgba(2), [77, 77, 77, 0]);
        assert_eq!(result.rgba(3), [255, 255, 255, 1]);
        // RGBA tiles are rejected for a Gray layer, and the reverse.
        let rgba = body(&[Some(raw())]);
        assert!(run(&rgba, &attrs, [4, 1], [0, 0], [0, 0]).is_err());
        assert!(run(&data, &attributes(4, 1, &data), [4, 1], [0, 0], [0, 0]).is_err());
    }
    #[test]
    fn signed_combined_offsets_clip_without_wrapping_and_empty_tiles_stay_sparse() {
        let mut tile = raw();
        pixel(&mut tile, 0, 0, [1, 2, 3, 255], 0);
        pixel(&mut tile, 2, 1, [4, 5, 6, 128], 0);
        let data = body(&[Some(tile)]);
        let attrs = attributes(3, 2, &data);
        let same = run(&data, &attrs, [3, 2], [-7, -9], [7, 9]).unwrap();
        assert_eq!(same.rgba(5), [4, 5, 6, 128]);
        let moved = run(&data, &attrs, [3, 2], [-1, -1], [0, 0]).unwrap();
        assert_eq!(moved.rgba(1), [4, 5, 6, 128]);
        assert_eq!(moved.rgba(0), [0; 4]);
        let offscreen = run(&data, &attrs, [3, 2], [1000, 1000], [0, 0]).unwrap();
        assert_eq!(offscreen.capacity(), 0);
        let empty = body(&[None]);
        let empty_attrs = attributes(3, 2, &empty);
        assert_eq!(
            run(&empty, &empty_attrs, [3, 2], [0, 0], [0, 0])
                .unwrap()
                .capacity(),
            0
        );
        assert!(run(&data, &attrs, [3, 2], [i64::MAX, 0], [1, 0]).is_err());
        assert!(run(&data, &attrs, [3, 2], [i64::MAX, 0], [0, 0]).is_err());
        assert!(run(&data, &attrs, [16385, 1], [0, 0], [0, 0]).is_err());
        assert!(run(&data, &attrs, [10001, 10000], [0, 0], [0, 0]).is_err());
    }
    #[test]
    fn grid_stride_uses_bitmap_geometry_when_canvas_is_a_single_pixel() {
        let mut corner = raw();
        pixel(&mut corner, 0, 0, [21, 43, 65, 87], 99);
        pixel(&mut corner, 1, 0, [255, 0, 0, 255], 1);
        let data = body(&[None, None, None, Some(corner)]);
        let attrs = attributes(257, 257, &data);
        let result = run(&data, &attrs, [1, 1], [-256, -256], [0, 0]).unwrap();
        assert_eq!(result.rgba(0), [21, 43, 65, 87]);
        assert_eq!(statistics(&result).nonzero_pixels, 1);
        assert_eq!(statistics(&result).partial_alpha_pixels, 1);
    }
    #[test]
    fn late_checksum_failure_bad_defaults_and_cancellation_return_no_pixels() {
        let mut visible = raw();
        pixel(&mut visible, 0, 0, [1, 2, 3, 128], 0);
        let data = body(&[Some(visible)]);
        let attrs = attributes(1, 1, &data);
        let mut broken = data.clone();
        *broken.last_mut().unwrap() ^= 1;
        assert!(run(&broken, &attrs, [1, 1], [0, 0], [0, 0]).is_err());
        let wrong = crate::clip_attributes::parse(&crate::clip_attributes::fixture(
            1,
            1,
            &[20, 1, u32::MAX, 4, 4, 0, 0, 0, 0],
            &attrs.block_sizes,
        ))
        .unwrap();
        assert!(run(&data, &wrong, [1, 1], [0, 0], [0, 0]).is_err());
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        assert!(
            decode(
                &mut Cursor::new(&data),
                data.len() as u64,
                &attrs,
                Placement {
                    canvas: [1, 1],
                    layer_offset: [0, 0],
                    render_offset: [0, 0]
                },
                &mut Budget::default(),
                &cancel
            )
            .err()
            .unwrap()
            .contains("canceled")
        );
    }
}
