//! One coalescing worker computes previews from the frozen gesture source.
use crate::{
    document::Document, pixels::Pixels, selection::Selection, selection_mask::Mask, transform,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub(crate) struct Candidate {
    pub(crate) changed: bool,
    pixels: Pixels,
    selection: Option<Selection>,
    mask: Option<Mask>,
    layer_mask: Option<crate::layer_mask::LayerMask>,
    offset: [i64; 2],
    net_changed: bool,
    affine: Option<Affine>,
}
impl Candidate {
    fn validated(self, source: &Document) -> Result<Self, String> {
        if source.version == 2 {
            let mut result = source.clone();
            result.layers[result.active].pixels = self.pixels.clone();
            result.layers[result.active].mask = self.layer_mask.clone();
            result.selection = self.selection;
            result.selection_mask = self.mask.clone();
            result.validate()?;
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, PartialEq)]
struct Affine {
    mapping: crate::affine::Mapping,
    sampling: crate::affine::Sampling,
    mirror: Option<crate::affine::Mirror>,
}
fn compute_request(
    source: &Document,
    expected: &Document,
    offset: [i64; 2],
    affine: Option<Affine>,
    paint_target: crate::paint_target::Target,
    cancel: &AtomicBool,
) -> Result<Candidate, String> {
    if affine.is_none() && paint_target.is_raster() {
        return compute(source, expected, offset, cancel);
    }
    let mut transformed = source.clone();
    match (paint_target, affine) {
        (_, Some(a)) if a.mirror.is_some() => {
            crate::affine::apply_pair(
                &mut transformed,
                a.mapping,
                a.sampling,
                a.mirror.unwrap(),
                paint_target,
                Some(cancel),
            )?;
        }
        (crate::paint_target::Target::Raster, Some(a)) => {
            crate::affine::apply_mapping(&mut transformed, a.mapping, a.sampling, Some(cancel))?;
        }
        (crate::paint_target::Target::Mask(id), Some(a)) => {
            crate::mask_transform::apply_mapping(
                &mut transformed,
                id,
                a.mapping,
                a.sampling,
                Some(cancel),
            )?;
        }
        (crate::paint_target::Target::Mask(id), None) => {
            crate::mask_transform::apply(
                &mut transformed,
                id,
                &crate::mask_transform::Operation::Move(offset),
                Some(cancel),
            )?;
        }
        _ => unreachable!(),
    }
    let differs = |doc: &Document| {
        Ok::<_, String>(
            transformed.layers[transformed.active].pixels != doc.layers[doc.active].pixels
                || transformed.selection != doc.selection
                || transformed.selection_mask != doc.selection_mask
                || !crate::layer_mask::equivalent_state(
                    &transformed.layers[transformed.active].mask,
                    &doc.layers[doc.active].mask,
                    Some(cancel),
                )?,
        )
    };
    let changed = differs(expected)?;
    let net_changed = differs(source)?;
    crate::fill::check_cancel(Some(cancel))?;
    Candidate {
        changed,
        pixels: transformed.layers[transformed.active].pixels.clone(),
        selection: transformed.selection,
        layer_mask: transformed.layers[transformed.active].mask.clone(),
        mask: transformed.selection_mask,
        offset,
        net_changed,
        affine,
    }
    .validated(source)
}
fn compute(
    source: &Document,
    expected: &Document,
    offset: [i64; 2],
    cancel: &AtomicBool,
) -> Result<Candidate, String> {
    crate::fill::check_cancel(Some(cancel))?;
    let layer = &source.layers[source.active];
    let region = source.selection.unwrap_or(Selection {
        left: 0,
        top: 0,
        right: source.width,
        bottom: source.height,
    });
    let (pixels, mask) = if offset == [0, 0] {
        (layer.pixels.clone(), source.selection_mask.clone())
    } else if let Some(mask) = &source.selection_mask {
        let (pixels, mask) = transform::remapped_masked_control(
            &layer.pixels,
            source.width,
            source.height,
            mask,
            transform::Remap::Offset(offset),
            Some(cancel),
        )?;
        (pixels, Some(mask))
    } else {
        (
            transform::translated_control(
                &layer.pixels,
                source.width,
                source.height,
                region,
                offset,
                Some(cancel),
            )?,
            None,
        )
    };
    let selection = if let Some(mask) = &mask {
        mask.bounds()
    } else {
        source
            .selection
            .and_then(|s| s.offset(offset[0], offset[1], source.width, source.height))
    };
    let layer_mask = if offset == [0, 0] {
        layer.mask.clone()
    } else {
        crate::layer_mask::for_raster_transform(source, Some(cancel), |mask| {
            mask.remap(source.width, source.height, 255, Some(cancel), |[x, y]| {
                let sx = (x as i64).checked_sub(offset[0])?;
                let sy = (y as i64).checked_sub(offset[1])?;
                (sx >= 0 && sy >= 0 && sx < source.width as i64 && sy < source.height as i64)
                    .then_some([sx as usize, sy as usize])
            })
        })?
    };
    let differs = |doc: &Document| {
        Ok::<_, String>(
            pixels != doc.layers[doc.active].pixels
                || selection != doc.selection
                || mask != doc.selection_mask
                || !crate::layer_mask::equivalent_state(
                    &layer_mask,
                    &doc.layers[doc.active].mask,
                    Some(cancel),
                )?,
        )
    };
    let changed = differs(expected)?;
    let net_changed = differs(source)?;
    crate::fill::check_cancel(Some(cancel))?;
    Candidate {
        pixels,
        selection,
        mask,
        layer_mask,
        offset,
        changed,
        net_changed,
        affine: None,
    }
    .validated(source)
}
pub(crate) fn apply(
    doc: &mut Document,
    offset: [i64; 2],
    cancel: Option<&AtomicBool>,
) -> Result<bool, String> {
    let default = AtomicBool::new(false);
    let candidate = compute(doc, doc, offset, cancel.unwrap_or(&default))?;
    if candidate.changed {
        doc.layers[doc.active].pixels = candidate.pixels;
        doc.layers[doc.active].mask = candidate.layer_mask;
        doc.selection = candidate.selection;
        doc.selection_mask = candidate.mask;
    }
    Ok(candidate.changed)
}
struct Job {
    offset: [i64; 2],
    affine: Option<Affine>,
    cancel: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<Result<Candidate, String>>>,
    result: Option<Result<Candidate, String>>,
}
impl Job {
    fn ready(&self) -> bool {
        self.result.is_some() || self.worker.as_ref().is_some_and(|w| w.is_finished())
    }
    fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.result = Some(
                worker
                    .join()
                    .unwrap_or_else(|_| Err("Move preview worker failed".into())),
            );
        }
    }
}
pub(crate) struct Engine {
    paint_target: crate::paint_target::Target,
    original: Document,
    expected: Document,
    latest: [i64; 2],
    latest_affine: Option<Affine>,
    applied_affine: Option<Affine>,
    pub(crate) applied: [i64; 2],
    pub(crate) net_changed: bool,
    job: Option<Job>,
}
impl Engine {
    #[cfg(test)]
    pub(crate) fn gated(
        source: &Document,
        offset: [i64; 2],
    ) -> (Self, std::sync::mpsc::Sender<()>) {
        let mut engine = Self::new(source);
        let send = engine.gate(offset);
        (engine, send)
    }
    #[cfg(test)]
    pub(crate) fn gate(&mut self, offset: [i64; 2]) -> std::sync::mpsc::Sender<()> {
        assert!(self.job.is_none());
        self.request(offset);
        let original = self.original.clone();
        let expected = self.expected.clone();
        let paint_target = self.paint_target;
        let cancel = Arc::new(AtomicBool::new(false));
        let control = cancel.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            receive.recv().map_err(|e| e.to_string())?;
            compute_request(&original, &expected, offset, None, paint_target, &control)
        });
        self.job = Some(Job {
            offset,
            affine: None,
            cancel,
            worker: Some(worker),
            result: None,
        });
        send
    }
    pub(crate) fn new(source: &Document) -> Self {
        Self::with_target(source, crate::paint_target::Target::Raster)
    }
    pub(crate) fn with_target(
        source: &Document,
        paint_target: crate::paint_target::Target,
    ) -> Self {
        Self {
            paint_target,
            original: source.clone(),
            expected: source.clone(),
            latest: [0, 0],
            latest_affine: None,
            applied_affine: None,
            applied: [0, 0],
            net_changed: false,
            job: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn gate_affine(
        &mut self,
        settings: crate::affine::Settings,
    ) -> std::sync::mpsc::Sender<()> {
        assert!(self.job.is_none());
        self.request_affine(settings).unwrap();
        let source = self.original.clone();
        let expected = self.expected.clone();
        let affine = self.latest_affine;
        let paint_target = self.paint_target;
        let cancel = Arc::new(AtomicBool::new(false));
        let control = cancel.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            receive.recv().map_err(|e| e.to_string())?;
            compute_request(&source, &expected, [0, 0], affine, paint_target, &control)
        });
        self.job = Some(Job {
            offset: [0, 0],
            affine,
            cancel,
            worker: Some(worker),
            result: None,
        });
        send
    }
    pub(crate) fn request(&mut self, offset: [i64; 2]) {
        self.latest_affine = None;
        self.latest = offset;
        if let Some(job) = &self.job
            && (job.offset != offset || job.affine.is_some())
        {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub(crate) fn request_affine(
        &mut self,
        settings: crate::affine::Settings,
    ) -> Result<(), String> {
        let affine = Affine {
            mapping: settings.mapping(&self.original)?,
            sampling: settings.sampling,
            mirror: settings.mirror,
        };
        self.latest = [0, 0];
        self.latest_affine = Some(affine);
        if let Some(job) = &self.job
            && (job.offset != self.latest || job.affine != self.latest_affine)
        {
            job.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    pub(crate) fn is_affine(&self) -> bool {
        self.latest_affine.is_some()
    }
    pub(crate) fn guard(&self, doc: &Document) -> Result<(), String> {
        if let (Some(current), Some(expected)) =
            (&doc.selection_mask, &self.expected.selection_mask)
            && !current.shares(expected)
        {
            return Err("Move source changed before preview publication".into());
        }
        if crate::document::same_snapshot(doc, &self.expected) {
            Ok(())
        } else {
            Err("Move source changed before preview publication".into())
        }
    }
    pub(crate) fn poll(&mut self, doc: &Document) -> Result<Option<Candidate>, String> {
        self.guard(doc)?;
        if let Some(job) = &self.job
            && !job.ready()
        {
            return Ok(None);
        }
        if let Some(mut job) = self.job.take() {
            job.join();
            if job.offset == self.latest
                && job.affine == self.latest_affine
                && !job.cancel.load(Ordering::Relaxed)
            {
                return job.result.take().unwrap().map(Some);
            }
        }
        if self.latest == self.applied && self.latest_affine == self.applied_affine {
            return Ok(None);
        }
        let source = self.original.clone();
        let expected = self.expected.clone();
        let offset = self.latest;
        let affine = self.latest_affine;
        let paint_target = self.paint_target;
        let cancel = Arc::new(AtomicBool::new(false));
        let control = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("dudel-move-preview".into())
            .spawn(move || {
                compute_request(&source, &expected, offset, affine, paint_target, &control)
            })
            .map_err(|error| format!("Could not start move preview: {error}"))?;
        self.job = Some(Job {
            offset,
            affine,
            cancel,
            worker: Some(worker),
            result: None,
        });
        Ok(None)
    }
    pub(crate) fn publish(
        &mut self,
        doc: &mut Document,
        candidate: Candidate,
    ) -> Result<(), String> {
        self.guard(doc)?;
        if candidate.offset != self.latest || candidate.affine != self.latest_affine {
            return Err("Move preview was superseded".into());
        }
        if candidate.changed {
            doc.layers[doc.active].pixels = candidate.pixels;
            doc.layers[doc.active].mask = candidate.layer_mask;
            doc.selection = candidate.selection;
            doc.selection_mask = candidate.mask;
            self.expected = doc.clone();
        }
        self.applied = candidate.offset;
        self.applied_affine = candidate.affine;
        self.net_changed = candidate.net_changed;
        Ok(())
    }
    /// Headless fixture fence; interactive finish transfers this engine to FIFO.
    #[cfg(test)]
    pub(crate) fn finish(&mut self, doc: &Document) -> Result<Option<Candidate>, String> {
        loop {
            if let Some(candidate) = self.poll(doc)? {
                return Ok(Some(candidate));
            }
            if let Some(job) = &mut self.job {
                job.join();
            } else {
                return Ok(None);
            }
        }
    }
    pub(crate) fn waiting(&self) -> bool {
        self.job.is_some()
    }
    pub(crate) fn waiting_worker(&self) -> bool {
        self.job.as_ref().is_some_and(|job| !job.ready())
    }
    pub(crate) fn wait_worker(&mut self) {
        if let Some(job) = &mut self.job {
            job.join();
        }
    }
    pub(crate) fn finished(&self) -> bool {
        self.job.is_none()
            && self.applied == self.latest
            && self.applied_affine == self.latest_affine
    }
    pub(crate) fn original(&self) -> &Document {
        &self.original
    }
    /// Move a vector layer's strokes the way the finished move or transform
    /// moved its pixels, and redraw them; a mirrored pair keeps its pixels
    /// and drops the strokes. Strokes that cannot be redrawn are dropped too,
    /// so the layer never shows pixels its strokes would not paint.
    pub(crate) fn follow_vector(&self, doc: &mut Document) {
        let before = &self.original;
        if !self.paint_target.is_raster() || before.layers[before.active].vector.is_none() {
            return;
        }
        let result = match self.applied_affine {
            Some(a) if a.mirror.is_some() => Err(String::new()),
            Some(a) => {
                let frame = crate::affine::Settings::bounds(before);
                let map = |x: f32, y: f32| {
                    a.mapping
                        .forward(frame, f64::from(x), f64::from(y))
                        .map(|v| v as f32)
                };
                crate::vector_layer::follow(before, doc, &map)
            }
            None => {
                let [x, y] = self.applied.map(|v| v as f32);
                crate::vector_layer::follow(before, doc, &|a, b| [a + x, b + y])
            }
        };
        if result.is_err() {
            doc.layers[before.active].vector = None;
        }
    }
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        registry.add_document(&self.original);
        registry.add_document(&self.expected);
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Document {
        let mut doc = Document::new(4, 1);
        doc.layers[0].pixels =
            vec![255, 0, 0, 255, 0, 0, 255, 255, 7, 11, 13, 0, 0, 0, 0, 0].into();
        let mask = Mask::from_coverage(4, 1, vec![128, 255, 0, 0]).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        doc
    }
    #[test]
    fn coalescing_discards_gated_old_job_and_final_preview_uses_frozen_source() {
        let mut doc = fixture();
        let original = doc.clone();
        let mut engine = Engine::new(&doc);
        let source = doc.clone();
        let expected = doc.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let control = cancel.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            receive.recv().unwrap();
            compute(&source, &expected, [1, 0], &control)
        });
        engine.job = Some(Job {
            offset: [1, 0],
            affine: None,
            cancel: cancel.clone(),
            worker: Some(worker),
            result: None,
        });
        engine.request([2, 0]);
        assert!(engine.poll(&doc).unwrap().is_none());
        assert!(cancel.load(Ordering::Relaxed) && engine.applied == [0, 0]);
        assert!(crate::document::same_snapshot(&doc, &original));
        send.send(()).unwrap();
        let candidate = engine.finish(&doc).unwrap().unwrap();
        engine.publish(&mut doc, candidate).unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(0), [255, 0, 0, 127]);
        assert_eq!(doc.layers[0].pixels.rgba(1), [0; 4]);
        assert_eq!(doc.layers[0].pixels.rgba(2), [255, 0, 0, 128]);
        assert_eq!(doc.layers[0].pixels.rgba(3), [0, 0, 255, 255]);
        assert_eq!(
            doc.selection_mask.as_ref().unwrap().bytes(),
            [0, 0, 128, 255]
        );
        assert!(engine.net_changed && engine.applied == [2, 0]);
        engine.request([1, 0]);
        let candidate = engine.finish(&doc).unwrap().unwrap();
        engine.publish(&mut doc, candidate).unwrap();
        assert_eq!(doc.layers[0].pixels.rgba(1), [255, 0, 0, 128]);
        assert_eq!(doc.layers[0].pixels.rgba(2), [0, 0, 255, 255]);
        engine.request([0, 0]);
        let candidate = engine.finish(&doc).unwrap().unwrap();
        engine.publish(&mut doc, candidate).unwrap();
        assert!(!engine.net_changed && crate::document::same_snapshot(&doc, &original));
    }
    #[test]
    fn stale_metadata_pixels_and_equal_reallocated_masks_reject_publication() {
        for change in 0..3 {
            let mut doc = fixture();
            let original = doc.clone();
            let mut engine = Engine::new(&doc);
            engine.request([1, 0]);
            let candidate = engine.finish(&doc).unwrap().unwrap();
            match change {
                0 => doc.layers[0].name = "External metadata".into(),
                1 => doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(3)
                    .copy_from_slice(&[9, 5, 3, 255]),
                _ => {
                    doc.selection_mask =
                        Some(Mask::from_coverage(4, 1, vec![128, 255, 0, 0]).unwrap())
                }
            }
            let newer = doc.clone();
            assert!(engine.publish(&mut doc, candidate).is_err());
            assert!(crate::document::same_snapshot(&doc, &newer));
            assert!(engine.applied == [0, 0] && !engine.net_changed);
            assert!(original.layers[0].pixels.rgba(3) == [0; 4]);
        }
    }
    #[test]
    fn legacy_rectangular_worker_matches_translation_and_zero_restores_hidden_rgb() {
        for tiled in [false, true] {
            let mut doc = Document::new(517, 259);
            doc.selection = Some(Selection {
                left: 254,
                top: 254,
                right: 258,
                bottom: 259,
            });
            for y in 254..259 {
                for x in 254..258 {
                    doc.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut(y * 517 + x)
                        .copy_from_slice(&[
                            17,
                            (x % 251) as u8,
                            (y % 239) as u8,
                            if y == 256 { 0 } else { 128 },
                        ]);
                }
            }
            if !tiled {
                doc.layers[0].pixels = doc.layers[0].pixels.to_vec().into();
            }
            let original = doc.clone();
            let mut engine = Engine::new(&doc);
            engine.request([3, -2]);
            let candidate = engine.finish(&doc).unwrap().unwrap();
            engine.publish(&mut doc, candidate).unwrap();
            assert_eq!(
                doc.layers[0].pixels,
                transform::translated(
                    &original.layers[0].pixels,
                    517,
                    259,
                    original.selection.unwrap(),
                    3,
                    -2
                )
            );
            engine.request([0, 0]);
            let candidate = engine.finish(&doc).unwrap().unwrap();
            engine.publish(&mut doc, candidate).unwrap();
            assert!(crate::document::same_snapshot(&doc, &original) && !engine.net_changed);
        }
    }
}
