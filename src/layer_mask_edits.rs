//! Ordered layer-mask commands. Mask identities survive edits but not replacement.
use crate::{
    document::{Document, LayerId},
    layer_mask::{LayerMask, Mask},
};
use std::sync::atomic::AtomicBool;
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Seed {
    RevealAll,
    HideAll,
    Selection,
}
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Command {
    Create {
        id: LayerId,
        seed: Seed,
    },
    Enabled {
        id: LayerId,
        value: bool,
    },
    Linked {
        id: LayerId,
        value: bool,
    },
    Invert(LayerId),
    Apply(LayerId),
    Remove(LayerId),
    Clear {
        id: LayerId,
        whole_layer: bool,
    },
    Fill {
        id: LayerId,
        request: crate::fill::Request,
    },
}
pub(crate) fn apply(
    doc: &mut Document,
    index: usize,
    command: Command,
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    doc.validate()?;
    crate::fill::check_cancel(cancel)?;
    let layer = doc.layers.get(index).ok_or("Mask layer no longer exists")?;
    if layer.locked {
        return Err("Unlock the layer before editing its mask".into());
    }
    let mut result = doc.clone();
    let changed = match command {
        Command::Create { id, seed } => {
            if layer.mask.is_some() {
                return Err("This layer already has a mask".into());
            }
            let coverage = match seed {
                Seed::RevealAll => Mask::new(doc.width, doc.height, 255)?,
                Seed::HideAll => Mask::new(doc.width, doc.height, 0)?,
                Seed::Selection => {
                    if doc.selection.is_none() && doc.selection_mask.is_none() {
                        return Err("Make a selection before creating a mask from it".into());
                    }
                    Mask::generate(doc.width, doc.height, 0, cancel, |x, y| {
                        Ok(doc.selection_mask.as_ref().map_or_else(
                            || {
                                if doc.selection.is_some_and(|s| s.contains(x, y)) {
                                    255
                                } else {
                                    0
                                }
                            },
                            |mask| mask.at(x, y),
                        ))
                    })?
                }
            };
            result.version = 2;
            result.layers[index].mask = Some(LayerMask {
                id,
                enabled: true,
                linked: true,
                coverage,
            });
            true
        }
        _ => {
            let current = layer.mask.as_ref().ok_or("This layer has no mask")?;
            let id = match command {
                Command::Enabled { id, .. }
                | Command::Linked { id, .. }
                | Command::Invert(id)
                | Command::Apply(id)
                | Command::Remove(id) => id,
                Command::Clear { id, .. } | Command::Fill { id, .. } => id,
                Command::Create { .. } => unreachable!(),
            };
            if current.id != id {
                return Err("Captured mask was removed or replaced".into());
            }
            match command {
                Command::Clear { whole_layer, .. } => {
                    if let Some(coverage) =
                        crate::mask_fill::clear(doc, index, current.id, whole_layer, cancel)?
                    {
                        result.layers[index].mask.as_mut().unwrap().coverage = coverage;
                        true
                    } else {
                        false
                    }
                }
                Command::Fill { request, .. } => {
                    if let Some(coverage) =
                        crate::mask_fill::fill(doc, index, current.id, request, cancel)?
                    {
                        result.layers[index].mask.as_mut().unwrap().coverage = coverage;
                        true
                    } else {
                        false
                    }
                }
                Command::Enabled { value, .. } => {
                    result.layers[index].mask.as_mut().unwrap().enabled = value;
                    value != current.enabled
                }
                Command::Linked { value, .. } => {
                    result.layers[index].mask.as_mut().unwrap().linked = value;
                    value != current.linked
                }
                Command::Invert(_) => {
                    result.layers[index].mask.as_mut().unwrap().coverage =
                        current.coverage.inverted(cancel)?;
                    true
                }
                Command::Remove(_) => {
                    result.layers[index].mask = None;
                    true
                }
                Command::Apply(_) => {
                    if layer.folder {
                        return Err("Merge the folder before applying its mask".into());
                    }
                    if current.enabled && layer.alpha_locked {
                        return Err("Unlock transparency before applying the mask".into());
                    }
                    if current.enabled {
                        let mut pixels = layer.pixels.clone();
                        let mut edits = Vec::new();
                        let mut changed = false;
                        layer.pixels.try_visit_rows(
                            doc.width,
                            doc.height,
                            [0, 0, doc.width, doc.height],
                            |left, y, row| {
                                crate::fill::check_cancel(cancel)?;
                                edits.clear();
                                for (offset, rgba) in row.chunks_exact(4).enumerate() {
                                    let before: [u8; 4] = rgba.try_into().unwrap();
                                    let after =
                                        current.coverage.apply_rgba(left + offset, y, before);
                                    if before != after {
                                        edits.push((left + offset, after));
                                    }
                                }
                                if !edits.is_empty() {
                                    changed = true;
                                    let mut writer = pixels.writer();
                                    for (x, after) in &edits {
                                        writer.rgba_mut_at(doc.width, *x, y).copy_from_slice(after);
                                    }
                                }
                                Ok::<_, String>(())
                            },
                        )?;
                        if changed {
                            pixels
                                .writer()
                                .reclaim_region(doc.width, [0, 0, doc.width, doc.height]);
                            result.layers[index].pixels = pixels;
                        }
                    }
                    result.layers[index].mask = None;
                    true
                }
                Command::Create { .. } => unreachable!(),
            }
        }
    };
    result.validate()?;
    crate::fill::check_cancel(cancel)?;
    if changed {
        *doc = result;
    }
    Ok(changed)
}
