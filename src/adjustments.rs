//! Native raster adjustments and an isolated, cancelable canvas preview.
use crate::{
    Dudel,
    document::{Document, LayerId},
    egui,
    plugins::FilterControl,
    snapshot_memory::SourceWorker,
};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    BrightnessContrast,
    HueSaturation,
    Levels,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Document {
        let mut doc = Document::new(65, 3);
        for (i, rgba) in [
            (0, [80, 120, 160, 255]),
            (63, [20, 40, 60, 128]),
            (64, [17, 29, 41, 0]),
            (129, [255, 0, 0, 255]),
        ] {
            doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(i)
                .copy_from_slice(&rgba);
        }
        doc
    }
    fn bright(value: f32) -> Settings {
        Settings::BrightnessContrast {
            brightness: value,
            contrast: 0.0,
        }
    }
    fn run(doc: &Document, settings: Settings) -> Document {
        prepare(
            doc,
            doc.layers[doc.active].id,
            settings,
            &FilterControl::default(),
        )
        .unwrap()
    }
    fn app() -> Dudel {
        Dudel {
            doc: fixture(),
            ..Default::default()
        }
    }
    fn wait(app: &mut Dudel) {
        crate::workflow_tests::finish_jobs(app);
    }
    #[test]
    fn adjustment_popup_stays_fixed_while_preview_prepares_updates_and_finishes() {
        for size in [egui::vec2(1280.0, 800.0), egui::vec2(480.0, 320.0)] {
            for kind in [Kind::BrightnessContrast, Kind::HueSaturation, Kind::Levels] {
                let mut app = app();
                app.start_adjustment(kind);
                wait(&mut app);
                let ctx = egui::Context::default();
                let frame = |app: &mut Dudel| {
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| {
                            crate::workspace::theme(ctx);
                            app.adjustment_ui(ctx);
                        },
                    );
                    let area = ctx
                        .memory(|m| m.area_rect(egui::Id::new("raster adjustment")))
                        .unwrap();
                    let apply = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == "Apply" => Some(text.pos),
                        _ => None,
                    });
                    (area, apply)
                };
                for _ in 0..4 {
                    frame(&mut app);
                }
                let ready = frame(&mut app);
                assert!(ready.1.is_some(), "Apply must be visible");
                for _ in 0..3 {
                    let result = app.adjustment.as_mut().unwrap().result.take();
                    for _ in 0..3 {
                        assert_eq!(frame(&mut app), ready, "{kind:?} updating at {size:?}");
                    }
                    app.adjustment.as_mut().unwrap().result = result;
                    assert_eq!(frame(&mut app), ready, "{kind:?} ready at {size:?}");
                }
                let source = app.adjustment.as_mut().unwrap().source.take();
                for _ in 0..3 {
                    assert_eq!(frame(&mut app), ready, "{kind:?} preparing at {size:?}");
                }
                app.adjustment.as_mut().unwrap().source = source;
                app.cancel_adjustment();
            }
        }
    }
    #[test]
    fn adjustment_math_and_validation() {
        assert_eq!(bright(100.0).channel(0), 255);
        assert_eq!(bright(-100.0).channel(255), 0);
        for n in 0..=255 {
            assert_eq!(bright(0.0).channel(n), n);
        }
        let flat = Settings::BrightnessContrast {
            brightness: 0.0,
            contrast: -100.0,
        };
        assert_eq!(flat.channel(0), 128);
        assert_eq!(flat.channel(255), 128);
        let threshold = Settings::BrightnessContrast {
            brightness: 0.0,
            contrast: 100.0,
        };
        assert_eq!(threshold.channel(127), 0);
        assert_eq!(threshold.channel(128), 255);
        let hue = Settings::HueSaturation {
            hue: 120.0,
            saturation: 0.0,
            lightness: 0.0,
        };
        assert_eq!(hue.hsl([255, 0, 0, 73]), [0, 255, 0, 73]);
        let gray = Settings::HueSaturation {
            hue: 0.0,
            saturation: -100.0,
            lightness: 0.0,
        };
        assert_eq!(gray.hsl([255, 0, 0, 73]), [128, 128, 128, 73]);
        let levels = Settings::Levels {
            black: 20,
            white: 220,
            gamma: 2.0,
            output_black: 10,
            output_white: 240,
        };
        assert_eq!(levels.channel(0), 10);
        assert_eq!(levels.channel(255), 240);
        assert_eq!(levels.channel(70), 125);
        assert!(bright(f32::NAN).validate().is_err());
        assert!(
            Settings::Levels {
                black: 20,
                white: 20,
                gamma: 1.0,
                output_black: 0,
                output_white: 255
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings::HueSaturation {
                hue: 181.0,
                saturation: 0.0,
                lightness: 0.0
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn selection_alpha_hidden_pixels_and_sparse_tiles_are_preserved() {
        let mut doc = fixture();
        let original = doc.clone();
        let mut coverage = vec![0; 65 * 3];
        coverage[0] = 255;
        coverage[63] = 128;
        coverage[64] = 255;
        doc.selection_mask =
            Some(crate::selection_mask::Mask::from_coverage(65, 3, coverage).unwrap());
        doc.selection = doc.selection_mask.as_ref().unwrap().bounds();
        let result = run(&doc, bright(100.0));
        assert_eq!(result.layers[0].pixels.rgba(0), [255, 255, 255, 255]);
        assert_eq!(
            result.layers[0].pixels.rgba(63),
            crate::selection_mask::mix_rgba([20, 40, 60, 128], [255, 255, 255, 128], 128)
        );
        assert_eq!(result.layers[0].pixels.rgba(64), [17, 29, 41, 0]);
        assert_eq!(result.layers[0].pixels.rgba(129), [255, 0, 0, 255]);
        assert!(doc.layers[0].pixels.shares(&original.layers[0].pixels));
        for kind in [Kind::BrightnessContrast, Kind::HueSaturation, Kind::Levels] {
            assert!(crate::document::same_snapshot(
                &doc,
                &run(&doc, kind.defaults())
            ));
        }
        doc.layers[0].locked = true;
        assert!(
            prepare(
                &doc,
                doc.layers[0].id,
                bright(20.0),
                &FilterControl::default()
            )
            .is_err()
        );
        doc.layers[0].locked = false;
        let control = FilterControl::default();
        control.cancel.store(true, Ordering::Relaxed);
        assert!(prepare(&doc, doc.layers[0].id, bright(20.0), &control).is_err());
        assert!(doc.layers[0].pixels.shares(&original.layers[0].pixels));
    }
    #[test]
    fn preview_cancel_and_toggle_do_not_change_persistence_or_history() {
        let mut app = app();
        let source = app.doc.clone();
        let revision = app.revision;
        app.start_adjustment(Kind::BrightnessContrast);
        app.adjustment.as_mut().unwrap().settings = bright(50.0);
        wait(&mut app);
        assert!(!crate::document::same_snapshot(&app.doc, &source));
        assert_eq!(app.revision, revision);
        assert!(!app.modified);
        assert!(!app.history.can_undo());
        app.adjustment.as_mut().unwrap().enabled = false;
        app.poll_adjustment(&egui::Context::default());
        assert!(crate::document::same_snapshot(&app.doc, &source));
        app.adjustment.as_mut().unwrap().enabled = true;
        app.poll_adjustment(&egui::Context::default());
        let dir = tempfile::tempdir().unwrap();
        app.path = Some(dir.path().join("preview.dudel"));
        app.save(false);
        app.export_to(dir.path().join("preview.png"), true);
        assert!(app.save_job.is_none() && app.export_job.is_none());
        assert!(!app.path.as_ref().unwrap().exists());
        app.cancel_adjustment();
        wait(&mut app);
        assert!(crate::document::same_snapshot(&app.doc, &source));
        assert_eq!(app.revision, revision);
        assert!(!app.modified);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn apply_one_undo_redo_save_reopen_and_export() {
        let mut app = app();
        let source = app.doc.composite_transparent();
        app.start_adjustment(Kind::HueSaturation);
        app.adjustment.as_mut().unwrap().settings = Settings::HueSaturation {
            hue: 90.0,
            saturation: 20.0,
            lightness: 10.0,
        };
        wait(&mut app);
        let adjusted = app.doc.composite_transparent();
        assert_ne!(source, adjusted);
        app.apply_adjustment();
        wait(&mut app);
        assert!(app.adjustment.is_none() && app.adjustment_ticket.is_none());
        assert!(app.modified);
        app.undo();
        wait(&mut app);
        assert_eq!(app.doc.composite_transparent(), source);
        assert!(!app.history.can_undo());
        app.redo();
        wait(&mut app);
        assert_eq!(app.doc.composite_transparent(), adjusted);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("adjusted.dudel");
        let png = dir.path().join("adjusted.png");
        app.path = Some(project.clone());
        app.save(false);
        app.export_to(png.clone(), true);
        wait(&mut app);
        assert_eq!(
            crate::storage::load_project(&project)
                .unwrap()
                .composite_transparent(),
            adjusted
        );
        assert_eq!(image::open(png).unwrap().into_rgba8().as_raw(), &adjusted);
        assert!(!app.modified);
    }
    #[test]
    fn latest_settings_win_and_stale_documents_are_never_overwritten() {
        let mut app = app();
        app.start_adjustment(Kind::BrightnessContrast);
        app.adjustment.as_mut().unwrap().settings = bright(70.0);
        app.poll_adjustment(&egui::Context::default());
        app.adjustment.as_mut().unwrap().settings = bright(-30.0);
        wait(&mut app);
        assert_eq!(
            app.doc.layers[0].pixels.rgba(0),
            run(&fixture(), bright(-30.0)).layers[0].pixels.rgba(0)
        );
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 255]);
        let newer = app.doc.clone();
        app.poll_adjustment(&egui::Context::default());
        assert!(app.adjustment.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &newer));
    }
    #[test]
    fn neutral_apply_preserves_redo_and_replacement_discards_preview() {
        let mut app = app();
        app.start_adjustment(Kind::BrightnessContrast);
        app.adjustment.as_mut().unwrap().settings = bright(20.0);
        wait(&mut app);
        app.apply_adjustment();
        wait(&mut app);
        app.undo();
        wait(&mut app);
        assert!(app.history.can_redo());
        let revision = app.revision;
        app.start_adjustment(Kind::Levels);
        wait(&mut app);
        app.apply_adjustment();
        wait(&mut app);
        assert!(app.history.can_redo());
        assert_eq!(app.revision, revision);
        let original = app.doc.clone();
        app.start_adjustment(Kind::BrightnessContrast);
        app.adjustment.as_mut().unwrap().settings = bright(80.0);
        wait(&mut app);
        app.request_replace(crate::document_actions::Action::New(4, 4, 300.0));
        assert!(app.adjustment.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &original));
        assert!(app.unsaved_action.is_some());
    }
    #[test]
    fn queued_artwork_precedes_preview_and_preview_never_enters_recovery() {
        let mut app = app();
        let original = app.doc.clone();
        app.render_queue
            .edit(crate::document_edits::Edit::Property {
                target: crate::document_edits::Target::Active,
                property: crate::document_edits::Property::Name("Pending artwork".into()),
            })
            .unwrap();
        app.start_adjustment(Kind::BrightnessContrast);
        app.adjustment.as_mut().unwrap().settings = bright(40.0);
        app.poll_adjustment(&egui::Context::default());
        assert!(app.adjustment.as_ref().unwrap().source.is_none());
        wait(&mut app);
        assert_eq!(app.doc.layers[0].name, "Pending artwork");
        let directory = tempfile::tempdir().unwrap();
        app.recovery_path = Some(directory.path().join("preview.gz"));
        app.modified = true;
        app.recovery_revision = app.revision.wrapping_sub(1);
        app.recovery_timer = std::time::Instant::now() - std::time::Duration::from_secs(16);
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.poll_recovery(ctx));
        assert!(app.recovery_job.is_none());
        app.cancel_adjustment();
        assert!(app.doc.layers[0].pixels.shares(&original.layers[0].pixels));
        assert_eq!(app.doc.layers[0].name, "Pending artwork");
    }
    #[test]
    fn prepared_publication_rejects_stale_source_without_changing_history() {
        let source = fixture();
        let result = run(&source, bright(30.0));
        let mut newer = source.clone();
        newer.layers[0].name = "New name".into();
        let mut queue = crate::render_queue::Queue::default();
        let mut history = crate::document::History::new();
        assert!(
            queue
                .finish_prepared_edit(&mut newer, &mut history, &source, result)
                .is_err()
        );
        assert_eq!(newer.layers[0].name, "New name");
        assert!(!queue.busy() && !history.can_undo());
    }
}
impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::BrightnessContrast => "Brightness / contrast",
            Self::HueSaturation => "Hue / saturation",
            Self::Levels => "Levels",
        }
    }
    pub(crate) fn defaults(self) -> Settings {
        match self {
            Self::BrightnessContrast => Settings::BrightnessContrast {
                brightness: 0.0,
                contrast: 0.0,
            },
            Self::HueSaturation => Settings::HueSaturation {
                hue: 0.0,
                saturation: 0.0,
                lightness: 0.0,
            },
            Self::Levels => Settings::Levels {
                black: 0,
                white: 255,
                gamma: 1.0,
                output_black: 0,
                output_white: 255,
            },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Settings {
    BrightnessContrast {
        brightness: f32,
        contrast: f32,
    },
    HueSaturation {
        hue: f32,
        saturation: f32,
        lightness: f32,
    },
    Levels {
        black: u8,
        white: u8,
        gamma: f32,
        output_black: u8,
        output_white: u8,
    },
}
impl Settings {
    fn kind(self) -> Kind {
        match self {
            Self::BrightnessContrast { .. } => Kind::BrightnessContrast,
            Self::HueSaturation { .. } => Kind::HueSaturation,
            Self::Levels { .. } => Kind::Levels,
        }
    }
    pub(crate) fn validate(self) -> Result<(), String> {
        let percent = |v: f32| v.is_finite() && (-100.0..=100.0).contains(&v);
        let valid = match self {
            Self::BrightnessContrast {
                brightness,
                contrast,
            } => percent(brightness) && percent(contrast),
            Self::HueSaturation {
                hue,
                saturation,
                lightness,
            } => {
                hue.is_finite()
                    && (-180.0..=180.0).contains(&hue)
                    && percent(saturation)
                    && percent(lightness)
            }
            Self::Levels {
                black,
                white,
                gamma,
                output_black,
                output_white,
            } => {
                black < white
                    && gamma.is_finite()
                    && (0.1..=10.0).contains(&gamma)
                    && output_black <= output_white
            }
        };
        if valid {
            Ok(())
        } else {
            Err("Adjustment settings are outside their supported range".into())
        }
    }
    fn channel(self, channel: u8) -> u8 {
        let value = match self {
            Self::BrightnessContrast {
                brightness,
                contrast,
            } => {
                let x = channel as f32 / 255.0 + brightness / 100.0;
                if contrast == 100.0 {
                    if x >= 0.5 { 1.0 } else { 0.0 }
                } else {
                    (x - 0.5) * (1.0 + contrast / 100.0) / (1.0 - contrast / 100.0) + 0.5
                }
            }
            Self::Levels {
                black,
                white,
                gamma,
                output_black,
                output_white,
            } => {
                let x = ((channel as f32 - black as f32) / (white as f32 - black as f32))
                    .clamp(0.0, 1.0)
                    .powf(1.0 / gamma);
                (output_black as f32 + x * (output_white as f32 - output_black as f32)) / 255.0
            }
            _ => channel as f32 / 255.0,
        };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    fn hsl(self, mut pixel: [u8; 4]) -> [u8; 4] {
        let Self::HueSaturation {
            hue,
            saturation,
            lightness,
        } = self
        else {
            return pixel;
        };
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(|v| v as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let mut l = (max + min) * 0.5;
        let mut s = if delta == 0.0 {
            0.0
        } else {
            delta / (1.0 - (2.0 * l - 1.0).abs())
        };
        let h = if delta == 0.0 {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / delta + 2.0) / 6.0
        } else {
            ((r - g) / delta + 4.0) / 6.0
        };
        let h = (h + hue / 360.0).rem_euclid(1.0) * 6.0;
        s = if saturation >= 0.0 {
            s + (1.0 - s) * saturation / 100.0
        } else {
            s * (1.0 + saturation / 100.0)
        };
        l = if lightness >= 0.0 {
            l + (1.0 - l) * lightness / 100.0
        } else {
            l * (1.0 + lightness / 100.0)
        };
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let m = l - c * 0.5;
        let rgb = match h as u8 {
            0 => [c, x, 0.0],
            1 => [x, c, 0.0],
            2 => [0.0, c, x],
            3 => [0.0, x, c],
            4 => [x, 0.0, c],
            _ => [c, 0.0, x],
        };
        for i in 0..3 {
            pixel[i] = ((rgb[i] + m).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        pixel
    }
}
pub(crate) fn prepare(
    source: &Document,
    target: LayerId,
    settings: Settings,
    control: &FilterControl,
) -> Result<Document, String> {
    settings.validate()?;
    source.validate()?;
    if source.layers[source.active].id != target {
        return Err("Adjustment layer is no longer selected".into());
    }
    if let Some(error) = source.paint_blocker() {
        return Err(error.into());
    }
    if control.cancel.load(Ordering::Relaxed) {
        return Err("Adjustment canceled".into());
    }
    if settings == settings.kind().defaults() {
        return Ok(source.clone());
    }
    let mut result = source.clone();
    let layer = &mut result.layers[source.active];
    if matches!(settings, Settings::HueSaturation { .. }) {
        crate::plugins::filter_pixels_masked(
            layer,
            source.selection,
            source.selection_mask.as_ref(),
            source.width,
            control,
            |p| Ok(settings.hsl(p)),
        )?;
    } else {
        let table: [u8; 256] = std::array::from_fn(|n| settings.channel(n as u8));
        crate::plugins::filter_pixels_masked(
            layer,
            source.selection,
            source.selection_mask.as_ref(),
            source.width,
            control,
            |mut p| {
                for c in &mut p[..3] {
                    *c = table[*c as usize];
                }
                Ok(p)
            },
        )?;
    }
    Ok(result)
}

pub(crate) struct Job {
    pub worker: SourceWorker<Result<Document, String>>,
    control: FilterControl,
    settings: Settings,
}
pub(crate) struct Preview {
    target: LayerId,
    pub settings: Settings,
    source: Option<Document>,
    revision: u64,
    result: Option<(Settings, Document)>,
    job: Option<Job>,
    enabled: bool,
    showing: bool,
    error: Option<String>,
}
impl Preview {
    fn expected(&self) -> Option<&Document> {
        if self.showing {
            self.result.as_ref().map(|r| &r.1)
        } else {
            self.source.as_ref()
        }
    }
    fn owns_display(&self, doc: &Document) -> bool {
        self.expected()
            .is_none_or(|expected| crate::document::same_snapshot(expected, doc))
    }
    pub(crate) fn ready(&self) -> bool {
        self.error.is_none()
            && self.job.is_none()
            && self.result.as_ref().is_some_and(|r| r.0 == self.settings)
    }
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        if let Some(source) = &self.source {
            registry.add_document(source);
        }
        if let Some((_, result)) = &self.result {
            registry.add_document(result);
        }
        if let Some(job) = &self.job {
            job.worker.register_source(registry);
        }
    }
}
impl Dudel {
    pub(crate) fn start_adjustment(&mut self, kind: Kind) {
        if !self.require_artwork_target() {
            return;
        }
        if self.adjustment.is_some() || self.adjustment_ticket.is_some() {
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.cancel_filter();
        if let Some(error) = self.input_paint_blocker() {
            self.status = error.into();
            return;
        }
        let Some(target) = self.input_target() else {
            return;
        };
        self.adjustment = Some(Box::new(Preview {
            target,
            settings: kind.defaults(),
            source: None,
            revision: 0,
            result: None,
            job: None,
            enabled: true,
            showing: false,
            error: None,
        }));
    }
    pub(crate) fn cancel_adjustment(&mut self) {
        let Some(mut preview) = self.adjustment.take() else {
            return;
        };
        if preview.owns_display(&self.doc)
            && let Some(source) = preview.source.take()
        {
            self.doc = source;
            self.dirty = true;
            self.dirty_region = Default::default();
            self.thumbnail_revision = None;
        }
        if let Some(job) = preview.job.take() {
            job.control.cancel.store(true, Ordering::Relaxed);
            self.retired_adjustment = Some(job.worker);
        }
        self.status = "Adjustment canceled".into();
    }
    pub(crate) fn poll_adjustment(&mut self, ctx: &egui::Context) {
        if self
            .retired_adjustment
            .as_ref()
            .is_some_and(|j| j.is_finished())
        {
            let _ = self.retired_adjustment.take().unwrap().join();
        }
        if self.retired_adjustment.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        let Some(mut preview) = self.adjustment.take() else {
            return;
        };
        if preview.source.is_none() {
            if self.render_queue.busy()
                || self.copy_job.is_some()
                || self.paste_job.is_some()
                || self.clipboard_settlement.is_some()
                || self.filter_job.is_some()
            {
                self.adjustment = Some(preview);
                ctx.request_repaint_after(std::time::Duration::from_millis(8));
                return;
            }
            if self.doc.layers[self.doc.active].id != preview.target
                || self.doc.paint_blocker().is_some()
            {
                self.status = "Select a visible, unlocked raster layer for adjustments".into();
                return;
            }
            self.track_guide_changes();
            preview.revision = self.revision;
            preview.source = Some(self.doc.clone());
        }
        if preview.revision != self.revision || !preview.owns_display(&self.doc) {
            self.adjustment = Some(preview);
            self.cancel_adjustment();
            self.status = "Adjustment canceled because artwork changed".into();
            return;
        }
        if preview
            .job
            .as_ref()
            .is_some_and(|j| j.settings != preview.settings)
        {
            preview
                .job
                .as_ref()
                .unwrap()
                .control
                .cancel
                .store(true, Ordering::Relaxed);
        }
        if preview.job.as_ref().is_some_and(|j| j.worker.is_finished()) {
            let job = preview.job.take().unwrap();
            let result = job
                .worker
                .join()
                .unwrap_or_else(|_| Err("Adjustment preview worker failed".into()));
            if job.settings == preview.settings && !job.control.cancel.load(Ordering::Relaxed) {
                match result {
                    Ok(result) => {
                        self.doc = if preview.enabled {
                            result.clone()
                        } else {
                            preview.source.as_ref().unwrap().clone()
                        };
                        preview.showing = preview.enabled;
                        preview.result = Some((job.settings, result));
                        preview.error = None;
                        self.dirty = true;
                        self.dirty_region = Default::default();
                        self.thumbnail_revision = None;
                    }
                    Err(error) => preview.error = Some(error),
                }
            }
        }
        if preview.enabled != preview.showing
            && let Some((_, result)) = &preview.result
        {
            self.doc = if preview.enabled {
                result.clone()
            } else {
                preview.source.as_ref().unwrap().clone()
            };
            preview.showing = preview.enabled;
            self.dirty = true;
            self.dirty_region = Default::default();
            self.thumbnail_revision = None;
        }
        if preview.job.is_none()
            && self.retired_adjustment.is_none()
            && preview.error.is_none()
            && !preview
                .result
                .as_ref()
                .is_some_and(|r| r.0 == preview.settings)
        {
            match preview.settings.validate() {
                Err(error) => preview.error = Some(error),
                Ok(()) => {
                    let source = preview.source.as_ref().unwrap().clone();
                    let settings = preview.settings;
                    let target = preview.target;
                    let control = FilterControl::default();
                    let worker_control = control.clone();
                    preview.job = Some(Job {
                        worker: SourceWorker::spawn(source, move |source| {
                            prepare(&source, target, settings, &worker_control)
                        }),
                        control,
                        settings,
                    });
                }
            }
        }
        if !preview.ready() && preview.error.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        self.adjustment = Some(preview);
    }
    pub(crate) fn apply_adjustment(&mut self) {
        let Some(preview) = self.adjustment.as_ref() else {
            return;
        };
        if !preview.ready() || self.render_queue.busy() {
            return;
        }
        if preview.revision != self.revision || !preview.owns_display(&self.doc) {
            self.cancel_adjustment();
            self.status = "Adjustment source changed; artwork was preserved".into();
            return;
        }
        let mut preview = self.adjustment.take().unwrap();
        let source = preview.source.take().unwrap();
        let (_, result) = preview.result.take().unwrap();
        self.doc = source.clone();
        match self.render_queue.finish_prepared_edit(
            &mut self.doc,
            &mut self.history,
            &source,
            result,
        ) {
            Ok(Some(ticket)) => {
                self.adjustment_ticket = Some(ticket);
                self.edit_feedback.insert(
                    ticket,
                    crate::render_controller::EditFeedback {
                        changed: "Adjustment applied",
                        unchanged: "Adjustment made no changes",
                    },
                );
                self.changed();
            }
            Ok(None) => self.status = "Adjustment made no changes".into(),
            Err(error) => self.status = error,
        }
        self.dirty = true;
        self.dirty_region = Default::default();
        self.thumbnail_revision = None;
    }
    pub(crate) fn adjustment_ui(&mut self, ctx: &egui::Context) {
        if self.adjustment_ticket.is_some() {
            egui::Modal::new(egui::Id::new("adjustment settlement")).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finishing adjustment…");
                });
            });
            return;
        }
        let Some(preview) = self.adjustment.as_mut() else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let id = egui::Id::new("raster adjustment");
        let response = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).fade_in(false))
            .show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(preview.settings.kind().name());
            // Keep the controls and status slot present throughout worker updates.
            // A centered modal moves under the pointer if its contents change size.
                egui::ScrollArea::vertical()
                    .id_salt("adjustment controls")
                    .max_height((ctx.content_rect().height() - 140.0).max(80.0))
                    .show(ui, |ui| {
                        let before = preview.settings;
                        ui.add_enabled_ui(preview.source.is_some(), |ui| match &mut preview.settings {
                            Settings::BrightnessContrast { brightness, contrast } => {
                                ui.add(egui::Slider::new(brightness, -100.0..=100.0).text("Brightness").integer());
                                ui.add(egui::Slider::new(contrast, -100.0..=100.0).text("Contrast").integer());
                            }
                            Settings::HueSaturation { hue, saturation, lightness } => {
                                ui.add(egui::Slider::new(hue, -180.0..=180.0).text("Hue").suffix("\u{b0}").integer());
                                ui.add(egui::Slider::new(saturation, -100.0..=100.0).text("Saturation").integer());
                                ui.add(egui::Slider::new(lightness, -100.0..=100.0).text("Lightness").integer());
                            }
                            Settings::Levels { black, white, gamma, output_black, output_white } => {
                                ui.label("Input levels");
                                ui.add(egui::Slider::new(black, 0..=white.saturating_sub(1)).text("Black point"));
                                ui.add(egui::Slider::new(gamma, 0.1..=10.0).logarithmic(true).text("Midtones").max_decimals(2));
                                ui.add(egui::Slider::new(white, black.saturating_add(1)..=255).text("White point"));
                                ui.label("Output levels");
                                ui.add(egui::Slider::new(output_black, 0..=*output_white).text("Black"));
                                ui.add(egui::Slider::new(output_white, *output_black..=255).text("White"));
                            }
                        });
                        if before != preview.settings { preview.error = None; }
                        ui.checkbox(&mut preview.enabled, "Preview");
                        ui.horizontal(|ui| {
                            let busy = !preview.ready() && preview.error.is_none();
                            let (_, slot) = ui.allocate_space(egui::vec2(18.0, 18.0));
                            if busy { ui.put(slot, egui::Spinner::new().size(18.0)); }
                            let status = if let Some(error) = &preview.error {
                                egui::RichText::new(error).color(ui.visuals().error_fg_color)
                            } else {
                                egui::RichText::new(if preview.source.is_none() {
                                    "Preparing preview\u{2026}"
                                } else if !preview.ready() {
                                    "Updating preview\u{2026}"
                                } else if !preview.enabled {
                                    "Preview off"
                                } else { "Preview ready" })
                            };
                            let label = ui.add(egui::Label::new(status).truncate());
                            if let Some(error) = &preview.error { label.on_hover_text(error); }
                        });
                        ui.small("Adjusts the selected layer inside the selection. Transparency is preserved. Apply creates one undo step.");
                    });
            ui.horizontal(|ui| {
                apply = ui.add_enabled(preview.ready(), egui::Button::new("Apply")).clicked();
                if ui.button("Reset").clicked() { preview.settings = preview.settings.kind().defaults(); preview.error = None; }
                cancel = ui.button("Cancel").clicked();
            });
        });
        if apply {
            self.apply_adjustment();
        } else if cancel || response.should_close() {
            self.cancel_adjustment();
        }
    }
}
