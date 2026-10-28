//! Color-based selection; samples artwork without changing or flattening layers.
use crate::{document::Document, fill, selection_mask::Mask};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub source: fill::Source,
    pub tolerance: u8,
    pub contiguous: bool,
    /// Captured active-mask slot. An explicit artwork/visible reference clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<(crate::document::LayerId, crate::document::LayerId)>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            source: fill::Source::Active,
            tolerance: 12,
            contiguous: true,
            mask: None,
        }
    }
}

pub(crate) fn mask(
    doc: &Document,
    point: [usize; 2],
    settings: Settings,
    cancel: Option<&AtomicBool>,
) -> Result<Mask, String> {
    fill::check_cancel(cancel)?;
    let count = crate::selection_mask::pixels(doc.width, doc.height)?;
    let [x, y] = point;
    if x >= doc.width || y >= doc.height {
        return Err("Magic wand point is outside the canvas".into());
    }
    let mask = if let Some((layer, slot)) = settings.mask {
        let valid_reference = match settings.source {
            fill::Source::Active => doc.layers.get(doc.active).is_some_and(|l| l.id == layer),
            fill::Source::Layer(id) => id == layer,
            fill::Source::Visible => false,
        };
        if !valid_reference {
            return Err("Captured mask does not match the wand reference".into());
        }
        Some(
            &doc.layers
                .iter()
                .find(|l| l.id == layer)
                .and_then(|l| l.mask.as_ref())
                .filter(|m| m.id == slot && m.coverage.dimensions() == [doc.width, doc.height])
                .ok_or("Magic wand mask was removed or replaced")?
                .coverage,
        )
    } else {
        None
    };
    let reference = if mask.is_some() {
        None
    } else {
        Some(match settings.source {
            fill::Source::Visible => doc
                .composite_bounds_checked([0, 0, doc.width, doc.height], true, || {
                    fill::check_cancel(cancel)
                })?
                .into(),
            source => {
                let layer = match source {
                    fill::Source::Active => doc.layers.get(doc.active),
                    fill::Source::Layer(id) => doc.layers.iter().find(|layer| layer.id == id),
                    fill::Source::Visible => unreachable!(),
                }
                .ok_or("Magic wand reference layer no longer exists")?;
                if layer.folder {
                    return Err(
                        "Choose a raster reference or visible layers for the magic wand".into(),
                    );
                }
                layer.pixels.clone()
            }
        })
    };
    let pixel = |index: usize| {
        mask.map_or_else(
            || reference.as_ref().unwrap().rgba(index),
            |mask| {
                let v = mask.at(index % doc.width, index / doc.width);
                [v, v, v, 255]
            },
        )
    };
    let seed = y * doc.width + x;
    let target = pixel(seed);
    // Match fill's existing alpha and premultiplied-sRGB tolerance semantics.
    // Hidden RGB in transparent pixels cannot split an otherwise empty region.
    let matches = |index| {
        let pixel = pixel(index);
        pixel[3].abs_diff(target[3]) <= settings.tolerance
            && (0..3).all(|channel| {
                let premultiplied =
                    |p: [u8; 4]| (u16::from(p[channel]) * u16::from(p[3]) / 255) as u8;
                premultiplied(pixel).abs_diff(premultiplied(target)) <= settings.tolerance
            })
    };
    let connected = if settings.contiguous {
        Some(fill::region_mask(
            [doc.width, doc.height],
            seed,
            fill::Options::default(),
            matches,
            cancel,
        )?)
    } else {
        None
    };
    let mut coverage = vec![0; count];
    for (index, coverage) in coverage.iter_mut().enumerate() {
        if index & 1023 == 0 {
            fill::check_cancel(cancel)?;
        }
        if connected
            .as_ref()
            .map_or_else(|| matches(index), |bits| fill::contains(bits, index))
        {
            *coverage = 255;
        }
    }
    Mask::from_coverage_control(doc.width, doc.height, coverage, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document_edits::Edit, selection_mask::Combine};

    fn fixture() -> Document {
        let mut doc = Document::new(5, 3);
        let mut bytes = vec![255; 5 * 3 * 4];
        for y in 0..3 {
            bytes[(y * 5 + 2) * 4..(y * 5 + 3) * 4].copy_from_slice(&[0, 0, 0, 255]);
        }
        doc.layers[0]
            .pixels
            .writer()
            .write_region(5, [0, 0, 5, 3], &bytes);
        doc
    }
    #[test]
    fn contiguous_and_all_matching_use_color_not_existing_selection_or_locks() {
        let mut doc = fixture();
        doc.layers[0].locked = true;
        doc.selection = Some(crate::selection::Selection {
            left: 4,
            top: 2,
            right: 5,
            bottom: 3,
        });
        for contiguous in [true, false] {
            let selected = mask(
                &doc,
                [0, 0],
                Settings {
                    tolerance: 0,
                    contiguous,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
            for y in 0..3 {
                for x in 0..5 {
                    assert_eq!(
                        selected.at(x, y),
                        if x < 2 || (!contiguous && x > 2) {
                            255
                        } else {
                            0
                        }
                    );
                }
            }
        }
    }
    #[test]
    fn transparent_hidden_rgb_and_premultiplied_tolerance_match_fill() {
        let mut doc = Document::new(5, 1);
        let bytes = [
            255, 0, 0, 0, 0, 255, 255, 0, 200, 100, 50, 128, 202, 100, 50, 128, 200, 100, 50, 130,
        ];
        doc.layers[0]
            .pixels
            .writer()
            .write_region(5, [0, 0, 5, 1], &bytes);
        let transparent = mask(
            &doc,
            [0, 0],
            Settings {
                tolerance: 0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(transparent.bytes(), &[255, 255, 0, 0, 0]);
        let settings = Settings {
            tolerance: 1,
            contiguous: false,
            ..Default::default()
        };
        let selected = mask(&doc, [2, 0], settings, None).unwrap();
        assert_eq!(selected.bytes(), &[0, 0, 255, 255, 0]);
        // The same reference, tolerance and seed must select what fill paints.
        let mut filled = doc.clone();
        filled
            .fill_request(
                fill::Request {
                    point: [2, 0],
                    color: [17, 83, 201],
                    opacity: 1.0,
                    tolerance: 1,
                    source: fill::Source::Active,
                    options: Default::default(),
                },
                None,
            )
            .unwrap();
        for index in 0..5 {
            assert_eq!(
                selected.at(index, 0) > 0,
                filled.layers[0].pixels.rgba(index) != doc.layers[0].pixels.rgba(index)
            );
        }
    }
    #[test]
    fn visible_reference_matches_compositor_and_explicit_identity_survives_reorder() {
        let mut doc = fixture();
        let reference = doc.layers[0].id;
        doc.group_active().unwrap();
        doc.add_layer();
        doc.layers[2].locked = true;
        let from_layer = mask(
            &doc,
            [0, 0],
            Settings {
                source: fill::Source::Layer(reference),
                tolerance: 0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let from_visible = mask(
            &doc,
            [0, 0],
            Settings {
                source: fill::Source::Visible,
                tolerance: 0,
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(from_layer, from_visible);
        doc.move_block(2, 0, 0).unwrap();
        assert_eq!(
            from_layer,
            mask(
                &doc,
                [0, 0],
                Settings {
                    source: fill::Source::Layer(reference),
                    tolerance: 0,
                    ..Default::default()
                },
                None
            )
            .unwrap()
        );
        let folder = doc.layers.iter().find(|layer| layer.folder).unwrap().id;
        assert!(
            mask(
                &doc,
                [0, 0],
                Settings {
                    source: fill::Source::Layer(folder),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(
            mask(
                &doc,
                [0, 0],
                Settings {
                    source: fill::Source::Layer(crate::document::LayerId::new()),
                    ..Default::default()
                },
                None
            )
            .is_err()
        );
        assert!(mask(&doc, [5, 0], Settings::default(), None).is_err());
    }
    #[test]
    fn combine_is_boolean_on_complete_incoming_region_and_queue_history_is_separate() {
        for mode in [
            Combine::Replace,
            Combine::Add,
            Combine::Subtract,
            Combine::Intersect,
        ] {
            let mut doc = fixture();
            let old = Mask::rectangle(
                5,
                3,
                crate::selection::Selection {
                    left: 1,
                    top: 1,
                    right: 4,
                    bottom: 3,
                },
            )
            .unwrap();
            doc.selection = old.bounds();
            doc.selection_mask = Some(old.clone());
            let original = doc.clone();
            let incoming = mask(
                &doc,
                [0, 0],
                Settings {
                    tolerance: 0,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
            let expected = old.combine(&incoming, mode, None).unwrap();
            let mut queue = crate::render_queue::Queue::default();
            let mut history = crate::document::History::new();
            queue
                .edit(Edit::SelectionWand {
                    point: [0, 0],
                    settings: Settings {
                        tolerance: 0,
                        ..Default::default()
                    },
                    mode,
                })
                .unwrap();
            while queue.busy() {
                if queue.waiting_edit() {
                    queue.wait_edit();
                }
                queue.advance(&mut doc, &mut history, 16).unwrap();
            }
            assert_eq!(doc.selection_mask.as_ref(), Some(&expected));
            assert_eq!(doc.layers[0].pixels, original.layers[0].pixels);
            assert!(history.undo(&mut doc));
            assert_eq!(doc.selection_mask, original.selection_mask);
            assert!(history.redo(&mut doc));
            assert_eq!(doc.selection_mask, Some(expected));
        }
    }
    #[test]
    fn canceled_and_stale_workers_preserve_redo_and_artwork() {
        for phase in 0..3 {
            let mut doc = fixture();
            let mut history = crate::document::History::new();
            history.checkpoint(&doc);
            doc.layers[0].opacity = 0.5;
            history.settle(&doc);
            assert!(history.undo(&mut doc));
            let original = doc.layers[0].pixels.clone();
            let mut queue = crate::render_queue::Queue::default();
            let ticket = queue
                .edit(Edit::SelectionWand {
                    point: [0, 0],
                    settings: Settings::default(),
                    mode: Combine::Replace,
                })
                .unwrap();
            if phase > 0 {
                queue.advance(&mut doc, &mut history, 1).unwrap();
                queue.wait_edit();
            }
            if phase == 2 {
                doc.layers[0].name = "Newer artwork".into();
            } else {
                assert!(queue.cancel_background_edit());
            }
            let progress = queue.advance(&mut doc, &mut history, 4096).unwrap();
            assert_eq!(progress.edits[0].0, ticket);
            assert!(progress.edits[0].1.is_err());
            assert!(doc.selection.is_none() && doc.selection_mask.is_none());
            assert_eq!(doc.layers[0].pixels, original);
            assert!(history.can_redo() && !history.can_undo());
        }
    }

    #[test]
    fn canceled_work_preserves_source_and_rejects_before_publication() {
        let doc = fixture();
        let before = doc.clone();
        let cancel = AtomicBool::new(true);
        let edit = Edit::SelectionWand {
            point: [0, 0],
            settings: Settings::default(),
            mode: Combine::Replace,
        };
        assert!(edit.prepare_control(&doc, Some(&cancel)).is_err());
        assert!(crate::document::same_snapshot(&doc, &before));
        assert!(
            mask(
                &doc,
                [0, 0],
                Settings {
                    contiguous: false,
                    ..Default::default()
                },
                Some(&cancel)
            )
            .is_err()
        );
    }
}
