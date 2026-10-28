//! Small restartable units for gathering, coverage reset/replay and writeback.
//! The caller keeps document edits ordered until this operation completes.
use super::{Painted, Settings, StrokeBuffer};
use crate::{
    document::{DabRaster, Document},
    stroke::Sample,
    symmetry::Map,
};
use std::collections::{HashSet, VecDeque};

type Key = (usize, usize, usize);
type Copies = Vec<([f32; 2], Map)>;

struct Grid {
    x: usize,
    y: usize,
    left: usize,
    right: usize,
    bottom: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_phase_can_cancel_and_metadata_changes_reject_without_artwork_mutation() {
        for target in 0..4 {
            let mut doc = Document::new(37, 39);
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[9, 8, 7, 0]);
            let before = doc.layers[0].pixels.clone();
            let settings = Settings {
                target: crate::paint_target::Target::Raster,
                brush: crate::brush::Brush {
                    size: 18.0,
                    taper_out: 20.0,
                    flow: 0.3,
                    ..Default::default()
                },
                color: [1, 2, 3],
                erase: false,
                layer: 0,
                symmetry: Default::default(),
            };
            let mut buffer = StrokeBuffer::default();
            for i in 0..15 {
                buffer.paint(
                    &mut doc,
                    Sample {
                        x: 11.5 + i as f32,
                        y: 20.5,
                        pressure: 1.0,
                        distance: i as f64,
                        ..Default::default()
                    },
                    &settings,
                );
            }
            let preview = doc.layers[0].pixels.clone();
            let mut cursor = buffer.begin_tail(&doc, &settings).unwrap();
            let mut steps = 0;
            while !matches!(
                (&cursor.phase, target),
                (Phase::Gather { .. }, 0)
                    | (Phase::Reset { .. }, 1)
                    | (Phase::Replay { .. }, 2)
                    | (Phase::Write { .. }, 3)
            ) {
                assert!(!cursor.step(&mut buffer, &mut doc, 1).unwrap().complete);
                steps += 1;
                assert!(steps < 10_000);
                if target < 3 {
                    assert_eq!(doc.layers[0].pixels, preview);
                }
            }
            cursor.step(&mut buffer, &mut doc, 3).unwrap();
            if target == 3 {
                let mut display = doc.layers[0].pixels.as_slice().to_vec();
                let mut saw_write = false;
                for _ in 0..128 {
                    let step = cursor.step(&mut buffer, &mut doc, 1).unwrap();
                    if !step.changed_region.is_empty() {
                        for &[left, top, right, bottom] in step.changed_region.rects() {
                            for y in top..bottom {
                                for x in left..right {
                                    let index = (y * doc.width + x) * 4;
                                    display[index..index + 4].copy_from_slice(
                                        &doc.layers[0].pixels.rgba(y * doc.width + x),
                                    );
                                }
                            }
                        }
                        assert_eq!(display, doc.layers[0].pixels.as_slice());
                        assert_ne!(doc.layers[0].pixels, preview);
                        saw_write = true;
                        break;
                    }
                    assert!(!step.complete);
                }
                assert!(
                    saw_write,
                    "Cancellation test must reach an actual partial pixel write"
                );
            }
            let current = doc.layers[0].pixels.clone();
            doc.layers[0].alpha_locked = true;
            assert!(cursor.step(&mut buffer, &mut doc, 10).is_err());
            assert_eq!(doc.layers[0].pixels, current);
            assert!(!buffer.prefix_pass && !buffer.rebuilding);
            doc.layers[0].alpha_locked = false;
            drop(cursor);
            buffer.restore(&mut doc);
            assert_eq!(doc.layers[0].pixels, before);
        }
    }
}
impl Grid {
    fn next(&mut self, layer: usize) -> Option<Key> {
        if self.y > self.bottom {
            return None;
        }
        let key = (layer, self.x, self.y);
        if self.x == self.right {
            self.x = self.left;
            self.y += 1;
        } else {
            self.x += 1;
        }
        Some(key)
    }
}
enum Phase {
    Gather {
        dab: usize,
        copy: usize,
        copies: Copies,
        grid: Option<Grid>,
    },
    Reset {
        at: usize,
        row: usize,
    },
    Replay {
        dab: usize,
        copy: usize,
        copies: Copies,
        raster: Option<(Box<DabRaster>, i32)>,
    },
    Write {
        at: usize,
        row: usize,
    },
    Done,
}

pub struct TailReplay {
    settings: Settings,
    dabs: VecDeque<Sample>,
    end: f64,
    size: [usize; 2],
    layer_id: crate::document::LayerId,
    selection: Option<crate::selection::Selection>,
    selection_mask: Option<crate::selection_mask::Mask>,
    alpha_locked: bool,
    layer_flags: [bool; 3],
    keys: HashSet<Key>,
    ordered: Vec<Key>,
    phase: Phase,
    peak_payload: usize,
}
#[derive(Default)]
pub struct TailStep {
    pub painted: Painted,
    /// Display damage from writeback only; coverage work never changes pixels.
    pub changed_region: crate::render_region::Damage,
    pub work_units: usize,
    pub complete: bool,
}
impl TailReplay {
    pub(super) fn new(dabs: VecDeque<Sample>, doc: &Document, settings: &Settings) -> Self {
        Self {
            end: dabs.back().unwrap().distance,
            dabs,
            settings: settings.clone(),
            size: [doc.width, doc.height],
            layer_id: doc.layers[doc.active].id,
            selection: doc.selection,
            selection_mask: doc.selection_mask.clone(),
            alpha_locked: doc.layers[settings.layer].alpha_locked,
            layer_flags: [
                doc.layers[settings.layer].folder,
                doc.layers[settings.layer].locked,
                doc.layers[settings.layer].visible,
            ],
            keys: HashSet::new(),
            ordered: vec![],
            phase: Phase::Gather {
                dab: 0,
                copy: 0,
                copies: vec![],
                grid: None,
            },
            peak_payload: 0,
        }
    }
    /// Zero work is a no-op. Each positive unit does at most one key insertion,
    /// one 32-pixel block row, or one prepared dab raster row. No frame clock or
    /// new sample timing is introduced. The caller chooses its own time deadline.
    pub fn step(
        &mut self,
        buffer: &mut StrokeBuffer,
        doc: &mut Document,
        max_work: usize,
    ) -> Result<TailStep, String> {
        if doc.layers.get(doc.active).is_none_or(|layer| {
            layer.id != self.layer_id || !self.settings.target.valid(layer, [doc.width, doc.height])
        }) || self.size != [doc.width, doc.height]
            || self.settings.layer != doc.active
            || doc.layers.get(doc.active).is_none_or(|layer| {
                [layer.folder, layer.locked, layer.visible] != self.layer_flags
                    || layer.alpha_locked != self.alpha_locked
            })
            || doc.selection != self.selection
            || doc.selection_mask != self.selection_mask
        {
            return Err("Document changed during tail finalization".into());
        }
        let mut result = TailStep::default();
        while result.work_units < max_work && !matches!(self.phase, Phase::Done) {
            result.work_units += 1;
            let phase = std::mem::replace(&mut self.phase, Phase::Done);
            self.phase = match phase {
                Phase::Gather {
                    mut dab,
                    mut copy,
                    mut copies,
                    mut grid,
                } => {
                    if let Some(key) = grid
                        .as_mut()
                        .and_then(|grid| grid.next(self.settings.layer))
                    {
                        if self.keys.insert(key) {
                            self.ordered.push(key);
                        }
                    } else if grid.take().is_some() {
                        copy += 1;
                    } else if dab == self.dabs.len() {
                        self.phase = Phase::Reset { at: 0, row: 0 };
                        continue;
                    } else if copies.is_empty() {
                        let sample = self.dabs[dab];
                        copies = self
                            .settings
                            .symmetry
                            .copies(sample.x, sample.y, doc.width, doc.height);
                    } else if copy == copies.len() {
                        dab += 1;
                        copy = 0;
                        copies.clear();
                    } else {
                        let [x, y] = copies[copy].0;
                        let [left, top, right, bottom] = crate::render_region::dab_bounds(
                            x,
                            y,
                            &self.settings.brush,
                            doc.width,
                            doc.height,
                        );
                        if left < right && top < bottom {
                            grid = Some(Grid {
                                x: left / 32,
                                left: left / 32,
                                y: top / 32,
                                right: (right - 1) / 32,
                                bottom: (bottom - 1) / 32,
                            });
                        } else {
                            copy += 1;
                        }
                    }
                    Phase::Gather {
                        dab,
                        copy,
                        copies,
                        grid,
                    }
                }
                Phase::Reset { mut at, mut row } => {
                    if at == self.ordered.len() {
                        self.phase = Phase::Replay {
                            dab: 0,
                            copy: 0,
                            copies: vec![],
                            raster: None,
                        };
                        continue;
                    }
                    if let Some(&block) = buffer.indices.get(&self.ordered[at]) {
                        let prefix = buffer.prefix[block].as_ref();
                        for offset in row * 32..(row + 1) * 32 {
                            let total = &mut buffer.blocks[block][offset].1;
                            if *total >= 0.0 {
                                *total = prefix.map_or(0.0, |coverage| coverage[offset]);
                            }
                        }
                        row += 1;
                    } else {
                        row = 32;
                    }
                    if row == 32 {
                        at += 1;
                        row = 0;
                    }
                    Phase::Reset { at, row }
                }
                Phase::Replay {
                    mut dab,
                    mut copy,
                    mut copies,
                    mut raster,
                } => {
                    if dab == self.dabs.len() {
                        self.phase = Phase::Write { at: 0, row: 0 };
                        continue;
                    }
                    if copies.is_empty() {
                        let sample = self.dabs[dab];
                        copies = self
                            .settings
                            .symmetry
                            .copies(sample.x, sample.y, doc.width, doc.height);
                    } else if copy == copies.len() {
                        dab += 1;
                        copy = 0;
                        copies.clear();
                    } else {
                        if raster.is_none() {
                            let ([x, y], map) = copies[copy];
                            let bounds = crate::render_region::dab_bounds(
                                x,
                                y,
                                &self.settings.brush,
                                doc.width,
                                doc.height,
                            );
                            if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
                                copy += 1;
                                self.phase = Phase::Replay {
                                    dab,
                                    copy,
                                    copies,
                                    raster,
                                };
                                continue;
                            }
                            result.painted.dabs += 1;
                            result.painted.region.include(bounds);
                            let mut sample = self.dabs[dab];
                            let factor = 0.05
                                + 0.95
                                    * ((self.end - sample.distance)
                                        / self.settings.brush.taper_out as f64)
                                        .clamp(0.0, 1.0)
                                        as f32;
                            sample.taper = Some(sample.taper.unwrap_or(1.0) * factor);
                            sample.x = x;
                            sample.y = y;
                            let plan = Box::new(DabRaster::new(
                                sample,
                                &self.settings.brush,
                                map,
                                doc.width,
                                doc.height,
                            ));
                            let row = plan.top;
                            // The phase is temporarily moved out of self here;
                            // include allocations that may disappear in this unit.
                            self.peak_payload = self.peak_payload.max(
                                self.payload_capacity_bytes()
                                    + copies.capacity() * std::mem::size_of::<([f32; 2], Map)>()
                                    + std::mem::size_of::<DabRaster>(),
                            );
                            raster = Some((plan, row));
                        }
                        let (plan, row) = raster.as_mut().unwrap();
                        buffer.prefix_pass = true;
                        buffer.rebuilding = true;
                        doc.paint_dab_rows(
                            plan,
                            &self.settings.brush,
                            self.settings.color,
                            self.settings.erase,
                            Some(buffer),
                            *row,
                            *row,
                        );
                        buffer.prefix_pass = false;
                        buffer.rebuilding = false;
                        *row += 1;
                        if *row > plan.bottom {
                            raster = None;
                            copy += 1;
                        }
                    }
                    Phase::Replay {
                        dab,
                        copy,
                        copies,
                        raster,
                    }
                }
                Phase::Write { mut at, mut row } => {
                    if at == self.ordered.len() {
                        self.phase = Phase::Done;
                        continue;
                    }
                    let (layer, bx, by) = self.ordered[at];
                    if let Some(&block) = buffer.indices.get(&(layer, bx, by)) {
                        let y = by * 32 + row;
                        let target = self.settings.target;
                        let mut writer = target.writer(&mut doc.layers[layer], doc.width);
                        let (color, erase, locked) = target.params(
                            self.settings.color,
                            self.settings.erase,
                            self.alpha_locked,
                        );
                        let mut changed = false;
                        for offset in row * 32..(row + 1) * 32 {
                            let x = bx * 32 + offset % 32;
                            let (original, total) = buffer.blocks[block][offset];
                            if total < 0.0 || x >= doc.width || y >= doc.height {
                                continue;
                            }
                            writer.edit(x, y, |pixel| {
                                let before: [u8; 4] = pixel.try_into().unwrap();
                                pixel.copy_from_slice(&original);
                                crate::document::paint_pixel(
                                    pixel,
                                    total * self.settings.brush.opacity,
                                    color,
                                    erase,
                                    locked,
                                );
                                changed |= pixel != before;
                            });
                        }
                        if changed {
                            result.changed_region.include([
                                bx * 32,
                                by * 32,
                                (bx * 32 + 32).min(doc.width),
                                (by * 32 + 32).min(doc.height),
                            ]);
                        }
                        result.painted.region.include([
                            bx * 32,
                            by * 32,
                            (bx * 32 + 32).min(doc.width),
                            (by * 32 + 32).min(doc.height),
                        ]);
                        row += 1;
                    } else {
                        row = 32;
                    }
                    if row == 32 {
                        at += 1;
                        row = 0;
                    }
                    Phase::Write { at, row }
                }
                Phase::Done => Phase::Done,
            };
            self.peak_payload = self.peak_payload.max(self.payload_capacity_bytes());
        }
        result.complete = matches!(self.phase, Phase::Done);
        Ok(result)
    }
    /// Cursor bookkeeping and cloned tip/grain payload. Source coverage/original
    /// blocks are reported by StrokeBuffer. Excludes allocator and map control bytes.
    pub fn payload_capacity_bytes(&self) -> usize {
        self.dabs.capacity() * std::mem::size_of::<Sample>()
            + self
                .settings
                .brush
                .image
                .as_ref()
                .map_or(0, |image| image.alpha.capacity())
            + self
                .settings
                .brush
                .grain
                .as_ref()
                .map_or(0, |image| image.alpha.capacity())
            + self.keys.capacity() * std::mem::size_of::<Key>()
            + self.ordered.capacity() * std::mem::size_of::<Key>()
            + match &self.phase {
                Phase::Gather { copies, .. } | Phase::Replay { copies, .. } => {
                    copies.capacity() * std::mem::size_of::<([f32; 2], Map)>()
                }
                _ => 0,
            }
            + if matches!(
                &self.phase,
                Phase::Replay {
                    raster: Some(_),
                    ..
                }
            ) {
                std::mem::size_of::<DabRaster>()
            } else {
                0
            }
    }
    pub fn peak_payload_capacity_bytes(&self) -> usize {
        self.peak_payload.max(self.payload_capacity_bytes())
    }
}
