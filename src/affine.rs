//! App side of `affine`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::affine`, re-exported
//! here so `crate::affine::…` paths keep working.
use crate::{document::Document, selection::Selection};
pub(crate) use dudel_core::affine::*;
pub(crate) use paired::{Mirror, driver_frame};

/// Queue timings with independent constant-color rectangle geometry oracles.
pub(crate) fn benchmark(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512, 2048] {
        let bounds = Selection {
            left: size / 4,
            top: size / 4,
            right: size * 3 / 4,
            bottom: size * 3 / 4,
        };
        let mut source = Document::new(size, size);
        source.selection = Some(bounds);
        {
            let mut writer = source.layers[0].pixels.writer();
            for y in bounds.top..bounds.bottom {
                for x in bounds.left..bounds.right {
                    writer
                        .rgba_mut(y * size + x)
                        .copy_from_slice(&[220, 40, 90, 255]);
                }
            }
        }
        for fractional in [false, true] {
            let settings = if fractional {
                Settings {
                    translation: [8.5, 0.5],
                    ..Settings::default()
                }
            } else {
                Settings {
                    scale_percent: [200.0, 100.0],
                    sampling: Sampling::Nearest,
                    ..Settings::default()
                }
            };
            let edit = crate::document_edits::Edit::AffineSettingsTarget {
                target: source.layers[0].id,
                settings: Box::new(settings),
            };
            let (mut result, mut history, timing) =
                crate::render_queue::measure_edit(&source, edit)?;
            let overlap = |start: f64, end: f64, pixel: usize| {
                (end.min(pixel as f64 + 1.0) - start.max(pixel as f64)).clamp(0.0, 1.0)
            };
            for y in 0..size {
                for x in 0..size {
                    let coverage = if fractional {
                        (255.0
                            * overlap(bounds.left as f64 + 8.5, bounds.right as f64 + 8.5, x)
                            * overlap(bounds.top as f64 + 0.5, bounds.bottom as f64 + 0.5, y))
                        .round() as u8
                    } else if y >= bounds.top && y < bounds.bottom {
                        255
                    } else {
                        0
                    };
                    let expected = if coverage == 0 {
                        [0; 4]
                    } else {
                        [220, 40, 90, coverage]
                    };
                    if result.layers[0].pixels.rgba(y * size + x) != expected
                        || result.selection_mask.as_ref().unwrap().at(x, y) != coverage
                    {
                        return Err(format!(
                            "Affine geometry oracle differs at {x},{y} ({size}, fractional={fractional})"
                        ));
                    }
                }
            }
            let after = result.clone();
            if !history.undo(&mut result)
                || result.layers[0].pixels != source.layers[0].pixels
                || result.selection != source.selection
                || result.selection_mask != source.selection_mask
                || !history.redo(&mut result)
                || result.layers[0].pixels != after.layers[0].pixels
                || result.selection_mask != after.selection_mask
            {
                return Err("Affine undo/redo differs".into());
            }
            let mut previews = Vec::new();
            for pending in [false, true] {
                let (mut preview, mut history, timing) =
                    crate::affine_preview::measure(&source, settings, pending)?;
                if preview.layers[0].pixels != after.layers[0].pixels
                    || preview.selection != after.selection
                    || preview.selection_mask != after.selection_mask
                {
                    return Err(
                        "Affine preview differs from independently verified geometry".into(),
                    );
                }
                if !history.undo(&mut preview)
                    || preview.layers[0].pixels != source.layers[0].pixels
                    || preview.selection != source.selection
                    || preview.selection_mask != source.selection_mask
                    || !history.redo(&mut preview)
                    || preview.layers[0].pixels != after.layers[0].pixels
                    || preview.selection_mask != after.selection_mask
                {
                    return Err("Affine preview undo/redo differs".into());
                }
                previews.push(serde_json::json!({"phase":if pending {"pending-final-request"} else {"ready-preview"},"timing":timing,"pixels_coverage_undo_redo_verified":true}));
            }
            cases.push(serde_json::json!({ "size": size, "operation": if fractional { "bilinear-half-pixel-translation" } else { "nearest-double-width" }, "timing": timing, "previews": previews, "pixels_and_coverage_verified": size * size, "undo_redo_verified": true }));
        }
    }
    let report = serde_json::json!({ "version":2, "scope":"One repetition per sparse-tiled constant opaque rectangular selection. Static background queue plus application frozen-source preview and ready/pending-request finish, guarded publication and sliced history settlement with a 2 ms yield deadline. Static path verifies an ordered snapshot. Preview results match independently verified static geometry and undo/redo. Worker joins outside owner timing and inside wall time. Source fixture retained during timing; final release of otherwise unshared source allocations excluded. Setup and verification excluded. No GUI, handle paint/hit-test, contour, display/GPU, physical input or RSS measurement; no hard frame-time guarantee.", "cases": cases });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn bicubic_ready_and_pending_previews_match_apply_and_one_step_undo() {
        let mut source = Document::new(32, 32);
        let bytes: Vec<_> = (0..32 * 32)
            .flat_map(|i| {
                [
                    i as u8,
                    (i / 4) as u8,
                    173,
                    if i % 5 == 0 { 0 } else { 128 },
                ]
            })
            .collect();
        source.layers[0]
            .pixels
            .writer()
            .write_region(32, [0, 0, 32, 32], &bytes);
        let settings = Settings {
            sampling: Sampling::Bicubic,
            scale_percent: [125.0, 80.0],
            angle_degrees: 23.0,
            ..Default::default()
        };
        let mut expected = source.clone();
        apply(
            &mut expected,
            settings.matrix(&source).unwrap(),
            Sampling::Bicubic,
            None,
        )
        .unwrap();
        for pending in [false, true] {
            let (mut actual, mut history, _) =
                crate::affine_preview::measure(&source, settings, pending).unwrap();
            assert_eq!(actual.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(actual.selection_mask, expected.selection_mask);
            assert!(history.undo(&mut actual));
            assert_eq!(actual.layers[0].pixels, source.layers[0].pixels);
            assert!(!history.can_undo());
            assert!(history.redo(&mut actual));
            assert_eq!(actual.layers[0].pixels, expected.layers[0].pixels);
        }
        let mut canceled = source.clone();
        let cancel = AtomicBool::new(true);
        assert!(
            apply(
                &mut canceled,
                settings.matrix(&source).unwrap(),
                Sampling::Bicubic,
                Some(&cancel)
            )
            .is_err()
        );
        assert!(crate::document::same_snapshot(&canceled, &source));
    }
}
