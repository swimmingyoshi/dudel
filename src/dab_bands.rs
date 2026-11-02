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

/// Paint `rect` (left, top, right, bottom; exclusive) of a dab into tiled
/// `pixels` through `buffer`. `coverage(x, y)` is the dab's alpha and
/// coverage ceiling there, or `None` where it paints nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint(
    pixels: &mut Pixels,
    buffer: &mut StrokeBuffer,
    layer: usize,
    rect: [usize; 4],
    coverage: impl Fn(usize, usize) -> Option<(f32, f32)> + Sync,
    color: [u8; 3],
    erase: bool,
    alpha_locked: bool,
) {
    let [left, top, right, bottom] = rect;
    let side = crate::pixels::SIDE;
    let bands = top / BAND..bottom.div_ceil(BAND);
    let blocks = left / BAND..right.div_ceil(BAND);
    let tiles = left / side..right.div_ceil(side);
    let width = right - left;
    let rows = |band: usize| (band * BAND).max(top)..((band + 1) * BAND).min(bottom);
    // Coverage first, so blocks and tiles are only made where paint lands.
    let drawn: Vec<Coverage> = crate::parallel::run(bands.len(), |i| {
        let rows = rows(bands.start + i);
        let mut values = vec![(0.0, 0.0); rows.len() * width];
        let mut touched = vec![false; blocks.len()];
        for (row, y) in rows.enumerate() {
            for x in left..right {
                if let Some(value) = coverage(x, y) {
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
    for (i, (_, touched)) in drawn.iter().enumerate() {
        for (column, _) in touched.iter().enumerate().filter(|(_, t)| **t) {
            buffer.touch_block(layer, blocks.start + column, bands.start + i);
        }
    }
    let per_tile = side / BAND;
    let tile_rows = pixels.band_tiles(BAND, bands.clone(), tiles.clone(), |band, tile| {
        drawn[band - bands.start]
            .1
            .iter()
            .enumerate()
            .any(|(column, t)| *t && (blocks.start + column) / per_tile == tile)
    });
    let block_rows = buffer.band_blocks(layer, bands.clone(), blocks.clone());
    let work: Vec<_> = tile_rows
        .into_iter()
        .zip(block_rows)
        .zip(drawn)
        .map(std::sync::Mutex::new)
        .collect();
    let results = crate::parallel::run(work.len(), |i| {
        let mut band = work[i].lock().unwrap();
        let ((tile_row, block_row), (values, _)) = &mut *band;
        for (row, y) in rows(bands.start + i).enumerate() {
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
                let a = crate::stroke_buffer::accumulate(entry, pixel, a, ceiling);
                crate::document::paint_pixel(pixel, a, color, erase, alpha_locked);
            }
        }
    });
    for result in results {
        result.expect("Dab paint worker panicked");
    }
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
}
