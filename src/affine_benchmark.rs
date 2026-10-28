//! Dense transform timings and logical working allocations, with geometry oracles.
use crate::{
    affine::{self, Matrix, Sampling},
    document::Document,
    selection::Selection,
    selection_mask::Mask,
};

pub(crate) fn run(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512, 2048] {
        for storage in ["flat", "tiled"] {
            for selection in ["none", "rectangle", "mask"] {
                for bilinear in [false, true] {
                    let region = if selection == "none" {
                        Selection {
                            left: 0,
                            top: 0,
                            right: size,
                            bottom: size,
                        }
                    } else {
                        Selection {
                            left: size / 4,
                            top: size / 4,
                            right: 3 * size / 4,
                            bottom: 3 * size / 4,
                        }
                    };
                    let mut source = Document::new(size, size);
                    let color = |x: usize, y: usize| {
                        if bilinear {
                            [220, 40, 90, 255]
                        } else {
                            [
                                (x % 251) as u8,
                                (y % 239) as u8,
                                ((x + y) % 233) as u8,
                                if (x + y).is_multiple_of(17) {
                                    0
                                } else {
                                    ((3 * x + 5 * y) % 256) as u8
                                },
                            ]
                        }
                    };
                    let mut bytes = Vec::with_capacity(size * size * 4);
                    for y in 0..size {
                        for x in 0..size {
                            bytes.extend(color(x, y));
                        }
                    }
                    if storage == "flat" {
                        source.layers[0].pixels = bytes.into();
                    } else {
                        source.layers[0].pixels.writer().write_region(
                            size,
                            [0, 0, size, size],
                            &bytes,
                        );
                    }
                    if selection != "none" {
                        source.selection = Some(region);
                    }
                    if selection == "mask" {
                        source.selection_mask = Some(Mask::rectangle(size, size, region)?);
                    }
                    let matrix = if bilinear {
                        Matrix([1.0, 0.0, 0.0, 1.0, 0.5, 0.5])
                    } else {
                        Matrix([-1.0, 0.0, 0.0, -1.0, size as f64, size as f64])
                    };
                    let sampling = if bilinear {
                        Sampling::Bilinear
                    } else {
                        Sampling::Nearest
                    };
                    let edit = crate::document_edits::Edit::AffineTarget {
                        target: source.layers[0].id,
                        matrix,
                        sampling,
                    };
                    let (mut result, mut history, timing) =
                        crate::render_queue::measure_edit(&source, edit)?;
                    let contains = |x, y| {
                        x >= region.left && x < region.right && y >= region.top && y < region.bottom
                    };
                    let overlap = |start: f64, end: f64, p: usize| {
                        (end.min(p as f64 + 1.0) - start.max(p as f64)).clamp(0.0, 1.0)
                    };
                    for y in 0..size {
                        for x in 0..size {
                            let coverage = if bilinear {
                                (255.0
                                    * overlap(
                                        region.left as f64 + 0.5,
                                        region.right as f64 + 0.5,
                                        x,
                                    )
                                    * overlap(
                                        region.top as f64 + 0.5,
                                        region.bottom as f64 + 0.5,
                                        y,
                                    ))
                                .round() as u8
                            } else if contains(x, y) {
                                255
                            } else {
                                0
                            };
                            let expected = if bilinear {
                                let mut p = color(x, y);
                                if contains(x, y) {
                                    p[3] = coverage;
                                    if coverage == 0 {
                                        p = [0; 4];
                                    }
                                }
                                p
                            } else if contains(x, y) {
                                color(size - 1 - x, size - 1 - y)
                            } else {
                                color(x, y)
                            };
                            if result.layers[0].pixels.rgba(y * size + x) != expected {
                                return Err(format!(
                                    "Dense affine pixel oracle differs at {x},{y} ({size},{storage},{selection},bilinear={bilinear})"
                                ));
                            }
                            if selection != "none"
                                && result
                                    .selection_mask
                                    .as_ref()
                                    .is_none_or(|m| m.at(x, y) != coverage)
                            {
                                return Err("Dense affine coverage oracle differs".into());
                            }
                        }
                    }
                    if selection == "none"
                        && (result.selection.is_some() || result.selection_mask.is_some())
                    {
                        return Err("Unselected transform created a selection".into());
                    }
                    let after = result.clone();
                    if !history.undo(&mut result)
                        || result.layers[0].pixels != source.layers[0].pixels
                        || result.selection != source.selection
                        || result.selection_mask != source.selection_mask
                        || !history.redo(&mut result)
                        || result.layers[0].pixels != after.layers[0].pixels
                        || result.selection != after.selection
                        || result.selection_mask != after.selection_mask
                    {
                        return Err("Dense affine undo/redo differs".into());
                    }
                    let mut profiled = source.clone();
                    let working = affine::profile(&mut profiled, matrix, sampling)?;
                    if profiled.layers[0].pixels != after.layers[0].pixels
                        || profiled.selection != after.selection
                        || profiled.selection_mask != after.selection_mask
                    {
                        return Err("Profiled affine differs from timed/oracle result".into());
                    }
                    cases.push(serde_json::json!({"size":size,"storage":storage,"selection":selection,"operation":if bilinear {"bilinear-half-pixel-translation"} else {"nearest-half-turn"},"timing":timing,"working":working,"pixels_and_coverage_verified":size*size,"undo_redo_verified":true}));
                }
            }
        }
    }
    let report = serde_json::json!({"version":1,"scope":"One repetition per 512/2048 dense flat/tiled source, unrestricted/rectangular/binary-mask selection and two operations. Half-turn oracle reverses patterned RGBA including partial alpha/hidden RGB; bilinear oracle uses independently projected constant-color rectangle overlap. Real ordered background edit/snapshot and sliced history, 2 ms yield deadline. Worker joins outside owner timing and inside wall. Working allocations are a separate untimed raster pass: actual source/destination coverage vector capacities plus output pixel/storage allocations deduplicated beyond the retained source, observed before tile reclamation. Output grows monotonically until reclamation; mask wrapping adds no coverage payload. Excludes source/live/history/other snapshots, Arc/allocator headers, registry/profile bookkeeping, thread stacks and retired outputs. Not RSS, a global budget or GUI/display/input latency. Source fixtures remain retained; final release of otherwise unshared source buffers, setup and verification excluded. Caps unchanged.","cases":cases});
    crate::storage::atomic_write(
        path,
        &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
}
