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
        || width > 4096
        || height > 4096
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
    clip_blocks::decode_layout(
        reader,
        body_bytes,
        &attributes.block_sizes,
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
                    let color = 65536 + source * 4;
                    // Separate coverage plane followed by straight B/G/R/unused.
                    // Keep hidden RGB, and never multiply color by coverage here.
                    let rgba = [raw[color + 2], raw[color + 1], raw[color], raw[source]];
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(run(&data, &attrs, [4097, 4096], [0, 0], [0, 0]).is_err());
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
