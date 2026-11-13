//! Restartable live dab, including old-prefix retirement and arc-reset sealing.
use super::{MAX_TAIL_DABS, Settings, StrokeBuffer, TailReplay, TailStep};
use crate::{
    document::{DabRaster, Document},
    stroke::Sample,
    symmetry::Map,
};
use std::sync::Arc;

struct Copies {
    sample: Sample,
    copies: Vec<([f32; 2], Map)>,
    /// Smudge: how far each copy moved since the previous dab.
    moves: Vec<[f32; 2]>,
    at: usize,
    raster: Option<(Box<DabRaster>, i32)>,
    peak_payload: usize,
}

impl Copies {
    fn new(sample: Sample, doc: &Document, settings: &Settings) -> Self {
        let copies = settings
            .symmetry
            .copies(sample.x, sample.y, doc.width, doc.height);
        let peak_payload = copies.capacity() * std::mem::size_of::<([f32; 2], Map)>();
        Self {
            sample,
            copies,
            moves: Vec::new(),
            at: 0,
            raster: None,
            peak_payload,
        }
    }
    /// One prepared raster row at most; expensive tables are retained between rows.
    fn step(
        &mut self,
        buffer: &mut StrokeBuffer,
        doc: &mut Document,
        settings: &Settings,
        prefix: bool,
        result: &mut TailStep,
    ) -> bool {
        if self.at == self.copies.len() {
            return true;
        }
        if self.raster.is_none() {
            let ([x, y], map) = self.copies[self.at];
            let bounds =
                crate::render_region::dab_bounds(x, y, &settings.brush, doc.width, doc.height);
            if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
                self.at += 1;
                return false;
            }
            result.painted.dabs += 1;
            if !prefix {
                result.painted.region.include(bounds);
            }
            let timed = crate::stage_timer::scope(crate::stage_timer::Stage::Raster);
            let mut raster = Box::new(DabRaster::new(
                Sample {
                    x,
                    y,
                    ..self.sample
                },
                &settings.brush,
                map,
                doc.width,
                doc.height,
            ));
            drop(timed);
            raster.smear = self.moves.get(self.at).copied().unwrap_or_default();
            raster.secondary = Some(settings.secondary);
            let row = raster.top;
            self.raster = Some((raster, row));
            self.peak_payload = self.peak_payload.max(self.payload_capacity_bytes());
        }
        let (raster, row) = self.raster.as_mut().unwrap();
        // One row at a time keeps slices short; a large live raster dab is
        // painted whole, split across cores.
        let last = if !prefix
            && settings.target.is_raster()
            && raster.left <= raster.right
            && *row <= raster.bottom
            && crate::dab_bands::worthwhile(
                (raster.right - raster.left + 1) as usize,
                (raster.bottom - *row + 1) as usize,
            ) {
            raster.bottom
        } else {
            *row
        };
        buffer.prefix_pass = prefix;
        doc.paint_dab_rows(
            raster,
            &settings.brush,
            settings.color,
            settings.erase,
            Some(buffer),
            *row,
            last,
        );
        buffer.prefix_pass = false;
        if !prefix && raster.left <= raster.right && *row <= raster.bottom {
            result.changed_region.include([
                raster.left as usize,
                *row as usize,
                raster.right as usize + 1,
                last as usize + 1,
            ]);
        }
        *row = last + 1;
        if *row > raster.bottom {
            self.raster = None;
            self.at += 1;
        }
        false
    }
    fn payload_capacity_bytes(&self) -> usize {
        self.copies.capacity() * std::mem::size_of::<([f32; 2], Map)>()
            + self
                .raster
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<DabRaster>())
    }
}
enum Phase {
    Seal(Box<TailReplay>),
    Retire,
    Prefix(Copies),
    Live(Copies),
    Store,
    Done,
}
pub struct DabPaint {
    settings: Arc<Settings>,
    sample: Sample,
    size: [usize; 2],
    layer_id: crate::document::LayerId,
    selection: Option<crate::selection::Selection>,
    selection_mask: Option<crate::selection_mask::Mask>,
    flags: [bool; 4],
    phase: Phase,
    peak_payload: usize,
}
impl DabPaint {
    pub(super) fn new(
        buffer: &mut StrokeBuffer,
        doc: &Document,
        sample: Sample,
        settings: Arc<Settings>,
    ) -> Self {
        let phase = if settings.brush.tail_length() > 0.0
            && buffer
                .tail
                .back()
                .is_some_and(|old| sample.distance < old.distance)
        {
            Phase::Seal(Box::new(buffer.begin_tail(doc, &settings).unwrap()))
        } else {
            Phase::Retire
        };
        let layer = &doc.layers[settings.layer];
        Self {
            settings,
            sample,
            size: [doc.width, doc.height],
            layer_id: doc.layers[doc.active].id,
            selection: doc.selection,
            selection_mask: doc.selection_mask.clone(),
            flags: [
                layer.folder,
                layer.locked,
                layer.visible,
                layer.alpha_locked,
            ],
            phase,
            peak_payload: 0,
        }
    }
    pub fn step(
        &mut self,
        buffer: &mut StrokeBuffer,
        doc: &mut Document,
        max_work: usize,
    ) -> Result<TailStep, String> {
        if doc.layers.get(doc.active).is_none_or(|layer| {
            layer.id != self.layer_id || !self.settings.target.valid(layer, [doc.width, doc.height])
        }) || self.size != [doc.width, doc.height]
            || doc.active != self.settings.layer
            || doc.selection != self.selection
            || doc.selection_mask != self.selection_mask
            || doc.layers.get(doc.active).is_none_or(|layer| {
                [
                    layer.folder,
                    layer.locked,
                    layer.visible,
                    layer.alpha_locked,
                ] != self.flags
            })
        {
            return Err("Document changed during dab rendering".into());
        }
        let mut result = TailStep::default();
        while result.work_units < max_work && !matches!(self.phase, Phase::Done) {
            result.work_units += 1;
            let phase = std::mem::replace(&mut self.phase, Phase::Done);
            self.phase = match phase {
                Phase::Seal(mut cursor) => {
                    let step = cursor.step(buffer, doc, 1)?;
                    self.peak_payload = self.peak_payload.max(
                        std::mem::size_of::<TailReplay>() + cursor.peak_payload_capacity_bytes(),
                    );
                    result.painted.merge(step.painted);
                    result.changed_region.merge(step.changed_region);
                    if step.complete {
                        Phase::Retire
                    } else {
                        Phase::Seal(cursor)
                    }
                }
                Phase::Retire => {
                    if self.settings.brush.tail_length() > 0.0
                        && (buffer.tail.front().is_some_and(|old| {
                            self.sample.distance - old.distance
                                > self.settings.brush.tail_length() as f64
                        }) || buffer.tail.len() >= MAX_TAIL_DABS)
                    {
                        Phase::Prefix(Copies::new(
                            buffer.tail.pop_front().unwrap(),
                            doc,
                            &self.settings,
                        ))
                    } else {
                        let mut copies = Copies::new(self.sample, doc, &self.settings);
                        if !self.settings.brush.blend.is_paint() {
                            copies.moves = buffer.blend_moves(
                                self.sample,
                                &self.settings,
                                [doc.width, doc.height],
                            );
                        }
                        Phase::Live(copies)
                    }
                }
                Phase::Prefix(mut copies) => {
                    let complete = copies.step(buffer, doc, &self.settings, true, &mut result);
                    self.peak_payload = self.peak_payload.max(copies.peak_payload);
                    if complete {
                        Phase::Retire
                    } else {
                        Phase::Prefix(copies)
                    }
                }
                Phase::Live(mut copies) => {
                    let complete = copies.step(buffer, doc, &self.settings, false, &mut result);
                    self.peak_payload = self.peak_payload.max(copies.peak_payload);
                    if complete {
                        Phase::Store
                    } else {
                        Phase::Live(copies)
                    }
                }
                Phase::Store => {
                    if self.settings.brush.tail_length() > 0.0 {
                        buffer.tail.push_back(self.sample);
                        buffer.tail_peak = buffer.tail_peak.max(buffer.tail.len());
                    }
                    Phase::Done
                }
                Phase::Done => Phase::Done,
            };
            self.peak_payload = self.peak_payload.max(self.payload_capacity_bytes());
        }
        result.complete = matches!(self.phase, Phase::Done);
        Ok(result)
    }
    /// Cursor-only capacity. Settings are held by a shared Arc and counted once
    /// by its owner; any tail-seal cursor has its existing separate payload estimate.
    pub fn payload_capacity_bytes(&self) -> usize {
        match &self.phase {
            Phase::Seal(cursor) => {
                std::mem::size_of::<TailReplay>() + cursor.payload_capacity_bytes()
            }
            Phase::Prefix(copies) | Phase::Live(copies) => copies.payload_capacity_bytes(),
            _ => 0,
        }
    }
    pub fn peak_payload_capacity_bytes(&self) -> usize {
        self.peak_payload.max(self.payload_capacity_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings(erase: bool, alpha_lock: bool, image: bool) -> (Document, Arc<Settings>) {
        let mut doc = Document::new(75, 73);
        for i in 0..75 * 73 {
            doc.layers[0].pixels.writer().rgba_mut(i).copy_from_slice(&[
                13,
                67,
                121,
                if i % 7 == 0 { 0 } else { 170 },
            ]);
        }
        doc.layers[0].alpha_locked = alpha_lock;
        doc.selection = Some(crate::selection::Selection {
            left: 3,
            top: 4,
            right: 70,
            bottom: 69,
        });
        let settings = Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 13.0,
                aspect: 0.7,
                angle: 37.0,
                flow: 0.27,
                opacity: 0.6,
                hardness: 0.4,
                tip: if image {
                    crate::brush::Tip::Image
                } else {
                    crate::brush::Tip::Square
                },
                image: image.then_some(crate::brush::TipImage {
                    width: 2,
                    height: 2,
                    alpha: (vec![0, 100, 190, 255]).into(),
                }),
                grain: Some(crate::brush::TipImage {
                    width: 2,
                    height: 2,
                    alpha: (vec![40, 190, 255, 100]).into(),
                }),
                grain_amount: 0.35,
                adjust_flow_by_spacing: true,
                taper_out: 12.0,
                softness_profile: crate::brush::SoftnessProfile::Eased,
                ..Default::default()
            },
            color: [210, 90, 35],
            secondary: [210, 90, 35],
            erase,
            layer: 0,
            symmetry: crate::symmetry::Symmetry {
                mode: crate::symmetry::Mode::FourWay,
                ..Default::default()
            },
        };
        (doc, Arc::new(settings))
    }
    fn sample(i: usize) -> Sample {
        Sample {
            x: 37.0 + (i as f32 * 0.5).sin() * 30.0,
            y: 36.0 + (i as f32 * 0.7).cos() * 25.0,
            pressure: 0.6 + (i as f32).cos() * 0.35,
            time: Some(i as f64 * 0.01),
            pressure_known: true,
            random: Some((i % 7) as f32 / 7.0),
            taper: Some(0.05 + 0.95 * (i % 11) as f32 / 10.0),
            spacing: Some(0.5),
            distance: (i % 20) as f64 * 1.5,
            ..Default::default()
        }
    }
    #[test]
    fn live_prefix_and_arc_reset_slices_match_each_synchronous_preview_and_undo() {
        for (erase, alpha, image) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            for budget in [1, 23, 128] {
                let (mut sliced, settings) = settings(erase, alpha, image);
                let base = sliced.layers[0].pixels.clone();
                let mut expected = sliced.clone();
                let mut original = StrokeBuffer::default();
                let mut buffer = StrokeBuffer::default();
                let mut history = crate::document::History::new();
                history.begin_stroke_checkpoint(&sliced);
                for i in 0..40 {
                    let sample = sample(i);
                    let reference = original.paint(&mut expected, sample, &settings);
                    let before = sliced.layers[0].pixels.clone();
                    let mut cursor = buffer.begin_dab(&sliced, sample, settings.clone()).unwrap();
                    let zero = cursor.step(&mut buffer, &mut sliced, 0).unwrap();
                    assert_eq!(zero.work_units, 0);
                    assert!(!zero.complete);
                    assert_eq!(sliced.layers[0].pixels, before);
                    let mut painted = super::super::Painted::default();
                    let mut display = (budget == 23 && image)
                        .then(|| sliced.layers[0].pixels.as_slice().to_vec());
                    loop {
                        let step = cursor.step(&mut buffer, &mut sliced, budget).unwrap();
                        assert!(step.work_units <= budget);
                        assert!(!buffer.prefix_pass && !buffer.rebuilding);
                        if let Some(display) = &mut display {
                            for &[left, top, right, bottom] in step.changed_region.rects() {
                                for y in top..bottom {
                                    for x in left..right {
                                        let index = (y * sliced.width + x) * 4;
                                        display[index..index + 4].copy_from_slice(
                                            &sliced.layers[0].pixels.rgba(y * sliced.width + x),
                                        );
                                    }
                                }
                            }
                            assert_eq!(display, sliced.layers[0].pixels.as_slice());
                        }
                        painted.merge(step.painted);
                        if step.complete {
                            break;
                        }
                    }
                    assert_eq!(painted.dabs, reference.dabs);
                    // Slices may split the same damage into different rectangles.
                    assert_eq!(painted.region.bounds(), reference.region.bounds());
                    assert_eq!(
                        sliced.layers[0].pixels, expected.layers[0].pixels,
                        "dab {i}, budget {budget}"
                    );
                    assert!(cursor.step(&mut buffer, &mut sliced, 1).unwrap().complete);
                    assert!(buffer.tail.len() <= MAX_TAIL_DABS);
                }
                buffer.finish_tail(&mut sliced, &settings);
                original.finish_tail(&mut expected, &settings);
                assert_eq!(sliced.layers[0].pixels, expected.layers[0].pixels);
                history.settle(&sliced);
                assert!(history.undo(&mut sliced));
                assert_eq!(sliced.layers[0].pixels, base);
                assert!(history.redo(&mut sliced));
                assert_eq!(sliced.layers[0].pixels, expected.layers[0].pixels);
            }
        }
    }
    #[test]
    fn forced_tail_retirement_at_capacity_matches_stationary_pixel_reference() {
        let mut doc = Document::new(9, 9);
        let mut expected = doc.clone();
        let settings = Arc::new(Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 1.0,
                pressure_size: false,
                taper_out: 300.0,
                flow: 0.02,
                ..Default::default()
            },
            color: [1, 2, 3],
            secondary: [1, 2, 3],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        });
        let mut buffer = StrokeBuffer::default();
        let mut reference = StrokeBuffer::default();
        for _ in 0..MAX_TAIL_DABS + 7 {
            let sample = Sample {
                x: 4.5,
                y: 4.5,
                pressure: 1.0,
                ..Default::default()
            };
            reference.paint(&mut expected, sample, &settings);
            let mut cursor = buffer.begin_dab(&doc, sample, settings.clone()).unwrap();
            loop {
                if cursor.step(&mut buffer, &mut doc, 3).unwrap().complete {
                    break;
                }
            }
            assert!(buffer.tail.len() <= MAX_TAIL_DABS);
            assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
        }
        assert_eq!(buffer.tail_peak, MAX_TAIL_DABS);
        reference.finish_tail(&mut expected, &settings);
        buffer.finish_tail(&mut doc, &settings);
        assert_eq!(doc.layers[0].pixels, expected.layers[0].pixels);
    }
    #[test]
    fn partial_live_prefix_and_seal_cancel_restore_original_pixels_and_redo() {
        for target in 0..3 {
            let (mut doc, settings) = settings(false, false, false);
            let mut history = crate::document::History::new();
            history.checkpoint(&doc);
            doc.dab(40.0, 30.0, 1.0, &settings.brush, [1, 2, 3], false);
            history.settle(&doc);
            assert!(history.undo(&mut doc));
            let before = doc.layers[0].pixels.clone();
            history.begin_stroke_checkpoint(&doc);
            let mut buffer = StrokeBuffer::default();
            if target > 0 {
                for i in 0..20 {
                    buffer.paint(&mut doc, sample(i), &settings);
                }
            }
            let next = match target {
                0 => sample(0),
                1 => Sample {
                    distance: 100.0,
                    ..sample(19)
                },
                _ => sample(20),
            };
            let mut cursor = buffer.begin_dab(&doc, next, settings.clone()).unwrap();
            let mut steps = 0;
            loop {
                let in_phase = matches!(
                    (&cursor.phase, target),
                    (Phase::Live(_), 0) | (Phase::Prefix(_), 1) | (Phase::Seal(_), 2)
                );
                let step = cursor.step(&mut buffer, &mut doc, 1).unwrap();
                assert!(!step.complete);
                steps += 1;
                assert!(steps < 20_000);
                if in_phase && (target == 1 || !step.changed_region.is_empty()) {
                    break;
                }
            }
            let preview = doc.layers[0].pixels.clone();
            doc.layers[0].locked = true;
            assert!(cursor.step(&mut buffer, &mut doc, 64).is_err());
            assert_eq!(doc.layers[0].pixels, preview);
            doc.layers[0].locked = false;
            assert!(history.discard_pending_checkpoint(&doc));
            drop(cursor);
            buffer.restore(&mut doc);
            history.refresh_memory(&doc);
            assert_eq!(doc.layers[0].pixels, before);
            assert!(!history.can_undo());
            assert!(history.can_redo());
            assert!(history.redo(&mut doc));
        }
    }
    #[test]
    fn large_dabs_painted_whole_match_synchronous_painting() {
        let mut sliced = Document::new(620, 590);
        let mut expected = sliced.clone();
        let settings = Arc::new(Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 280.0,
                hardness: 0.4,
                flow: 0.3,
                taper_out: 120.0,
                ..Default::default()
            },
            color: [30, 140, 220],
            secondary: [30, 140, 220],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        });
        let (mut buffer, mut reference) = (StrokeBuffer::default(), StrokeBuffer::default());
        let mut display = sliced.layers[0].pixels.as_slice().to_vec();
        for i in 0..12 {
            let sample = Sample {
                x: 150.0 + i as f32 * 30.0,
                y: 300.0 + (i as f32).sin() * 40.0,
                pressure: 0.7 + (i % 3) as f32 * 0.1,
                pressure_known: true,
                distance: i as f64 * 30.0,
                ..Default::default()
            };
            reference.paint(&mut expected, sample, &settings);
            let mut cursor = buffer.begin_dab(&sliced, sample, settings.clone()).unwrap();
            loop {
                let step = cursor.step(&mut buffer, &mut sliced, 1).unwrap();
                for &[left, top, right, bottom] in step.changed_region.rects() {
                    for y in top..bottom {
                        for x in left..right {
                            let index = y * sliced.width + x;
                            display[index * 4..index * 4 + 4]
                                .copy_from_slice(&sliced.layers[0].pixels.rgba(index));
                        }
                    }
                }
                if step.complete {
                    break;
                }
            }
            assert_eq!(
                sliced.layers[0].pixels, expected.layers[0].pixels,
                "dab {i}"
            );
            assert_eq!(display, sliced.layers[0].pixels.as_slice(), "dab {i}");
        }
        buffer.finish_tail(&mut sliced, &settings);
        reference.finish_tail(&mut expected, &settings);
        assert_eq!(sliced.layers[0].pixels, expected.layers[0].pixels);
    }
}
