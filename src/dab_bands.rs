//! Large dabs painted in 32-row bands across cores.
//!
//! A 300 px dab touches about 70 000 pixels and takes milliseconds on one
//! thread, mostly blending. Each band owns its rows of the layer's tiles and
//! of the stroke's coverage blocks, so bands blend without sharing anything;
//! every pixel still gets exactly the arithmetic of the row-by-row path.
use crate::{pixels::Pixels, stroke_buffer::StrokeBuffer};

/// Rows per band: one stroke-buffer block high, dividing the tile side.
const BAND: usize = 32;

/// A band's alpha and ceiling per pixel, and which blocks it paints.
type Coverage = (Vec<(f32, f32)>, Vec<bool>);

/// Whether a dab area of `width` x `height` pixels is worth splitting.
pub(crate) fn worthwhile(width: usize, height: usize) -> bool {
    height > BAND && width * height >= 1 << 13
}

/// A dab's coverage over its area, worked out once in bands across cores
/// and kept band by band, the way `apply` paints. Everything that needs the
/// dab's coverage afterwards reads it here.
pub(crate) struct Drawn {
    /// Left, top, right, bottom (exclusive).
    rect: [usize; 4],
    bands: Vec<Coverage>,
}

impl Drawn {
    /// `coverage(x, y)` is the dab's alpha and coverage ceiling there, or
    /// `None` where it paints nothing.
    pub(crate) fn new(
        rect: [usize; 4],
        coverage: impl Fn(usize, usize) -> Option<(f32, f32)> + Sync,
    ) -> Self {
        let [left, top, right, bottom] = rect;
        let bands = top / BAND..bottom.div_ceil(BAND);
        let blocks = left / BAND..right.div_ceil(BAND);
        let width = right - left;
        let bands = crate::parallel::run(bands.len(), |i| {
            let rows = band_rows(rect, bands.start + i);
            let mut values = vec![(0.0, 0.0); rows.len() * width];
            let mut touched = vec![false; blocks.len()];
            for (row, y) in rows.enumerate() {
                for x in left..right {
                    if let Some(value) = coverage(x, y).filter(|value| value.0 > 0.0) {
                        values[row * width + x - left] = value;
                        touched[x / BAND - blocks.start] = true;
                    }
                }
            }
            (values, touched)
        })
        .into_iter()
        .map(|result| result.expect("Dab coverage worker panicked"))
        .collect();
        Self { rect, bands }
    }

    /// The dab's alpha and coverage ceiling at a pixel of its area, or
    /// `None` where it paints nothing.
    pub(crate) fn at(&self, x: usize, y: usize) -> Option<(f32, f32)> {
        let [left, top, right, _] = self.rect;
        let band = y / BAND - top / BAND;
        let first = (y / BAND * BAND).max(top);
        let value = self.bands[band].0[(y - first) * (right - left) + x - left];
        (value.0 > 0.0).then_some(value)
    }
}

/// The rows of `rect` in band `band` (rows `band * BAND..` of the canvas).
fn band_rows(rect: [usize; 4], band: usize) -> std::ops::Range<usize> {
    (band * BAND).max(rect[1])..((band + 1) * BAND).min(rect[3])
}

/// Paint a dab into tiled `pixels` through `buffer`.
pub(crate) fn paint(
    pixels: &mut Pixels,
    buffer: &mut StrokeBuffer,
    layer: usize,
    drawn: &Drawn,
    color: [u8; 3],
    erase: bool,
    alpha_locked: bool,
) {
    apply(
        pixels,
        buffer,
        layer,
        drawn,
        false,
        |_, _, a, ceiling, entry, pixel| {
            let a = crate::stroke_buffer::accumulate(entry, pixel, a, ceiling);
            crate::document::paint_pixel(pixel, a, color, erase, alpha_locked);
            0.0
        },
    );
}

/// Run `change(x, y, alpha, ceiling, stroke state, pixel)` for every pixel
/// the dab covers, in bands across cores. With `collect`, what each call
/// returns comes back band by band from the top, one value per pixel of the
/// dab's area, row by row (zero where the dab paints nothing); otherwise
/// the bands come back empty.
pub(crate) fn apply(
    pixels: &mut Pixels,
    buffer: &mut StrokeBuffer,
    layer: usize,
    drawn: &Drawn,
    collect: bool,
    change: impl Fn(usize, usize, f32, f32, &mut crate::stroke_buffer::PixelState, &mut [u8]) -> f32
    + Sync,
) -> Vec<Vec<f32>> {
    let rect = drawn.rect;
    let [left, top, right, bottom] = rect;
    let side = crate::pixels::SIDE;
    let bands = top / BAND..bottom.div_ceil(BAND);
    let blocks = left / BAND..right.div_ceil(BAND);
    let tiles = left / side..right.div_ceil(side);
    let width = right - left;
    // Blocks and tiles are only made where paint lands.
    for (i, (_, touched)) in drawn.bands.iter().enumerate() {
        for (column, _) in touched.iter().enumerate().filter(|(_, t)| **t) {
            buffer.touch_block(layer, blocks.start + column, bands.start + i);
        }
    }
    let per_tile = side / BAND;
    let tile_rows = pixels.band_tiles(BAND, bands.clone(), tiles.clone(), |band, tile| {
        drawn.bands[band - bands.start]
            .1
            .iter()
            .enumerate()
            .any(|(column, t)| *t && (blocks.start + column) / per_tile == tile)
    });
    let block_rows = buffer.band_blocks(layer, bands.clone(), blocks.clone());
    let work: Vec<_> = tile_rows
        .into_iter()
        .zip(block_rows)
        .map(std::sync::Mutex::new)
        .collect();
    crate::parallel::run(work.len(), |i| {
        let mut band = work[i].lock().unwrap();
        let (tile_row, block_row) = &mut *band;
        let values = &drawn.bands[i].0;
        let mut returned = vec![0.0; if collect { values.len() } else { 0 }];
        for (row, y) in band_rows(rect, bands.start + i).enumerate() {
            for x in left..right {
                let (a, ceiling) = values[row * width + x - left];
                if a <= 0.0 {
                    continue;
                }
                let entry = &mut block_row[x / BAND - blocks.start]
                    .as_mut()
                    .expect("coverage block made for painted pixel")[(y % BAND) * BAND + x % BAND];
                let offset = ((y % BAND) * side + x % side) * 4;
                let pixel = &mut tile_row[x / side - tiles.start]
                    .as_mut()
                    .expect("tile made for painted pixel")[offset..offset + 4];
                let value = change(x, y, a, ceiling, entry, pixel);
                if collect {
                    returned[row * width + x - left] = value;
                }
            }
        }
        returned
    })
    .into_iter()
    .map(|result| result.expect("Dab paint worker panicked"))
    .collect()
}

#[cfg(test)]
mod tests {
    use crate::{
        document::{DabRaster, Document},
        stroke::Sample,
        stroke_buffer::StrokeBuffer,
    };

    #[test]
    fn bands_match_painting_row_by_row() {
        let (width, height) = (620, 590);
        let mut base = Document::new(width, height);
        // Artwork in one tile only, so dabs also make new tiles.
        {
            let mut writer = base.layers[0].pixels.writer();
            for y in 0..200 {
                for x in 0..240 {
                    let rgba = [(x * 7) as u8, (y * 3) as u8, 90, ((x + y) % 256) as u8];
                    writer.rgba_mut(y * width + x).copy_from_slice(&rgba);
                }
            }
        }
        assert!(base.layers[0].pixels.is_tiled(), "bands paint tiled layers");
        let samples = [(250.0, 260.0), (300.0, 240.0), (180.0, 330.0)];
        for case in 0..5 {
            let mut doc = base.clone();
            let brush = crate::brush::Brush {
                size: 300.0,
                hardness: if case == 1 { 0.0 } else { 0.6 },
                flow: 0.4,
                opacity: 0.8,
                pressure_size: false,
                ..Default::default()
            };
            let erase = case == 2;
            doc.layers[0].alpha_locked = case == 3;
            if case == 4 {
                let coverage = (0..width * height)
                    .map(|i| (i % width % 256) as u8)
                    .collect();
                doc.selection_mask = Some(
                    crate::selection_mask::Mask::from_coverage(width, height, coverage).unwrap(),
                );
            }
            let mut rows = doc.clone();
            let (mut whole_buffer, mut rows_buffer) =
                (StrokeBuffer::default(), StrokeBuffer::default());
            for (x, y) in samples {
                let sample = Sample {
                    x,
                    y,
                    pressure: 1.0,
                    ..Default::default()
                };
                doc.dab_sample_buffered(
                    sample,
                    &brush,
                    [200, 40, 10],
                    erase,
                    crate::symmetry::IDENTITY,
                    Some(&mut whole_buffer),
                );
                let raster =
                    DabRaster::new(sample, &brush, crate::symmetry::IDENTITY, width, height);
                for row in raster.top..=raster.bottom {
                    rows.paint_dab_rows(
                        &raster,
                        &brush,
                        [200, 40, 10],
                        erase,
                        Some(&mut rows_buffer),
                        row,
                        row,
                    );
                }
                assert_eq!(doc.layers[0].pixels, rows.layers[0].pixels, "case {case}");
            }
            assert_ne!(doc.layers[0].pixels, base.layers[0].pixels, "case {case}");
            whole_buffer.restore(&mut doc);
            assert_eq!(doc.layers[0].pixels, base.layers[0].pixels, "case {case}");
        }
    }

    #[test]
    fn blend_and_material_dabs_in_bands_match_painting_row_by_row() {
        use crate::brush::Blend;
        use crate::material::{Kind, Material, Settings};
        let (width, height) = (620, 590);
        let mut base = Document::new(width, height);
        {
            let mut writer = base.layers[0].pixels.writer();
            for y in 0..height {
                for x in 0..400 {
                    let rgba = [(x * 7) as u8, (y * 3) as u8, 90, ((x + y) % 256) as u8];
                    writer.rgba_mut(y * width + x).copy_from_slice(&rgba);
                }
            }
        }
        base.paper = Some(crate::paper::Paper::presets()[1].1);
        let cases = [
            (Blend::Smudge, None, false),
            (Blend::Blur, None, false),
            (Blend::Wet, None, false),
            (Blend::Wet, Some(Kind::Watercolor), false),
            (Blend::Wet, Some(Kind::Watercolor), true),
            (Blend::Smudge, Some(Kind::Paint), false),
            (Blend::Paint, Some(Kind::Paint), false),
        ];
        for (case, (blend, kind, locked)) in cases.into_iter().enumerate() {
            let mut doc = base.clone();
            doc.layers[0].material = kind.map(|kind| Material::new(Settings::new(kind)));
            doc.layers[0].alpha_locked = locked;
            let brush = crate::brush::Brush {
                size: 260.0,
                hardness: 0.5,
                opacity: 0.8,
                wetness: 0.7,
                pigment: 0.6,
                thickness: 0.8,
                pressure_size: false,
                blend,
                ..Default::default()
            };
            let mut rows = doc.clone();
            let (mut whole_buffer, mut rows_buffer) =
                (StrokeBuffer::default(), StrokeBuffer::default());
            for (x, y) in [(250.0, 260.0), (300.0, 240.0), (330.0, 300.0)] {
                let sample = Sample {
                    x,
                    y,
                    pressure: 1.0,
                    ..Default::default()
                };
                let mut raster =
                    DabRaster::new(sample, &brush, crate::symmetry::IDENTITY, width, height);
                raster.smear = [9.0, -4.0];
                raster.heading = Some(30.0);
                doc.paint_dab_rows(
                    &raster,
                    &brush,
                    [200, 40, 10],
                    false,
                    Some(&mut whole_buffer),
                    raster.top,
                    raster.bottom,
                );
                for row in raster.top..=raster.bottom {
                    rows.paint_dab_rows(
                        &raster,
                        &brush,
                        [200, 40, 10],
                        false,
                        Some(&mut rows_buffer),
                        row,
                        row,
                    );
                }
                assert_eq!(doc.layers[0].pixels, rows.layers[0].pixels, "case {case}");
            }
            assert_ne!(doc.layers[0].pixels, base.layers[0].pixels, "case {case}");
            // The water left behind and the paint's height match too.
            let water = |doc: &Document| {
                let mut all = Vec::new();
                let mut row = vec![0.0f32; width];
                for y in 0..height {
                    doc.wet.read_row(doc.layers[0].id, y, 0, &mut row);
                    all.extend_from_slice(&row);
                }
                all
            };
            assert_eq!(water(&doc), water(&rows), "case {case}");
            assert_eq!(
                water(&doc).iter().any(|w| *w > 0.0),
                blend == Blend::Wet,
                "case {case}"
            );
            let relief = |doc: &Document| {
                doc.layers[0]
                    .material
                    .as_ref()
                    .and_then(|material| material.relief.clone())
            };
            match (relief(&doc), relief(&rows)) {
                (Some(a), Some(b)) => assert!(a.equivalent(&b, None).unwrap(), "case {case}"),
                (a, b) => assert_eq!(a.is_some(), b.is_some(), "case {case}"),
            }
            whole_buffer.restore(&mut doc);
            assert_eq!(doc.layers[0].pixels, base.layers[0].pixels, "case {case}");
        }
    }
}
