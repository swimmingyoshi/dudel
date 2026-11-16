//! App side of `wet`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::wet`, re-exported
//! here so `crate::wet::…` paths keep working.
use crate::{Dudel, egui};
pub(crate) use dudel_core::wet::*;

/// The app's drying state for the open project.
#[derive(Default)]
pub(crate) struct Drying {
    /// Steps owed, in fractions of a step.
    owed: f32,
    /// The document revision after our last step: anything else changing
    /// the document in between ends our undo step.
    revision: Option<u64>,
    /// Undo or redo happened: what was wet is no longer there.
    pub(crate) reset: bool,
    /// The step being worked out on a background worker, and the document
    /// revision it was started from.
    job: Option<(u64, crate::background::Handle<Plan>)>,
}

impl Dudel {
    /// Let wet paint spread and dry while nothing else is working on the
    /// document. Drying joins the undo step of the stroke that left the
    /// paint wet, so one undo takes back both; it is a step of its own only
    /// when something else was done after that stroke.
    pub(crate) fn dry_tick(&mut self, ctx: &egui::Context) {
        if self.doc.wet.is_empty() {
            self.drying.revision = None;
            self.drying.reset = false;
            return;
        }
        let busy = self.render_queue.busy()
            || self.stroking
            || self.queued_stroke.is_some()
            || self.move_drag.is_some()
            || self.pending_move.is_some()
            || self.affine_preview.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
            || self.canvas_edit_ticket.is_some()
            || self.filter_job.is_some()
            || self.pending_layers.is_some()
            || self.text_moving
            || self.dialogs.open();
        if busy {
            // Water waits; it does not dry behind the artist's back.
            self.drying.owed = 0.0;
            self.drying.job = None;
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if std::mem::take(&mut self.drying.reset) {
            self.doc.wet.clear();
            self.drying.revision = None;
            self.drying.job = None;
            return;
        }
        // A step is worked out on a background worker from a snapshot and
        // written here only if the document is still as it was, so a large
        // wash never holds up a frame for longer than the write.
        let mut damage = None;
        let mut committed = false;
        match self.drying.job.take() {
            Some((revision, job)) if job.is_finished() => {
                if revision == self.revision
                    && let Ok(plan) = job.join()
                {
                    if self.drying.revision != Some(self.revision) {
                        // The document changed since drying last wrote: the
                        // drying from here joins the last stroke if that is
                        // what made this layer wet, else it starts a step.
                        let wet =
                            self.doc.wet.layer().and_then(|id| {
                                self.doc.layers.iter().position(|layer| layer.id == id)
                            });
                        if !wet.is_some_and(|layer| self.history.reopen_last(&self.doc, layer)) {
                            self.history.begin_gesture_checkpoint(&self.doc);
                        }
                    }
                    committed = true;
                    match commit(&mut self.doc, plan) {
                        Some([_, _, 0, 0]) | None => {}
                        Some(rect) => damage = Some(rect),
                    }
                } else {
                    // Planned for a document that has changed since.
                    self.drying.owed += 1.0;
                }
            }
            job => self.drying.job = job,
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.drying.owed = (self.drying.owed + dt * RATE).min(4.0);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        if let Some(rect) = damage {
            self.dirty = true;
            self.dirty_region.include(rect);
            self.modified = true;
            self.revision = self.revision.wrapping_add(1);
            self.content_revision = self.content_revision.wrapping_add(1);
        }
        if committed {
            self.drying.revision = Some(self.revision);
        }
        if self.doc.wet.is_empty() {
            self.history.settle(&self.doc);
            self.drying.revision = None;
            return;
        }
        if self.drying.job.is_none()
            && self.drying.owed >= 1.0
            && let Some(inputs) = inputs(&mut self.doc)
        {
            self.drying.owed -= 1.0;
            let ctx = ctx.clone();
            let job = crate::background::spawn(move || {
                let plan = plan(inputs);
                ctx.request_repaint();
                plan
            });
            self.drying.job = Some((self.revision, job));
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::{Dudel, document::Document, egui};

    use crate::brush::{Blend, Brush};

    fn wash() -> Brush {
        Brush {
            size: 24.0,
            hardness: 0.9,
            flow: 0.5,
            spacing: 0.1,
            pressure_size: false,
            blend: Blend::Wet,
            wetness: 1.0,
            ..Default::default()
        }
    }

    fn stroke(doc: &mut Document, brush: &Brush, color: [u8; 3], y: f32) {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color,
            secondary: color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for i in 0..=20 {
            let sample = crate::stroke::Sample {
                x: 30.0 + i as f32 * 5.0,
                y,
                pressure: 1.0,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == 20 {
                path.finish(sample, brush)
            } else {
                path.sample(sample, brush)
            };
            for dab in dabs {
                buffer.paint(doc, dab, &settings);
            }
        }
        buffer.finish_tail(doc, &settings);
    }

    /// One frame of drying, waiting for the step it started as the next
    /// frame would find it.
    fn tick(app: &mut Dudel, ctx: &egui::Context) {
        app.dry_tick(ctx);
        while app
            .drying
            .job
            .as_ref()
            .is_some_and(|(_, job)| !job.is_finished())
        {
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_wet_stroke_and_its_drying_undo_together() {
        // The owner's report of 2026-11-13: wet strokes needed two undos,
        // one for the drying and one for the stroke.
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(160, 80),
            ..Default::default()
        };
        let blank = app.doc.layers[0].pixels.clone();
        app.history.begin_stroke_checkpoint(&app.doc);
        stroke(&mut app.doc, &wash(), [200, 30, 30], 34.0);
        app.history.settle(&app.doc);
        app.revision = app.revision.wrapping_add(1);
        let painted = app.doc.layers[0].pixels.clone();
        let mut ticks = 0;
        while !app.doc.wet.is_empty() {
            tick(&mut app, &ctx);
            ticks += 1;
            assert!(ticks < 5000, "drying ends");
        }
        let dried = app.doc.layers[0].pixels.clone();
        assert!(dried != painted, "paint spread while drying");
        // One undo takes back the stroke with its drying.
        app.undo();
        assert!(app.doc.layers[0].pixels == blank);
        assert!(!app.history.can_undo());
        // Redo brings back the stroke as it dried.
        app.redo();
        assert!(app.doc.layers[0].pixels == dried);
        tick(&mut app, &ctx);
        assert!(app.doc.wet.is_empty());
        // Undo halfway through drying also takes back the whole stroke.
        let mut app = Dudel {
            doc: Document::new(160, 80),
            ..Default::default()
        };
        app.history.begin_stroke_checkpoint(&app.doc);
        stroke(&mut app.doc, &wash(), [200, 30, 30], 34.0);
        app.history.settle(&app.doc);
        app.revision = app.revision.wrapping_add(1);
        for _ in 0..10 {
            tick(&mut app, &ctx);
        }
        assert!(!app.doc.wet.is_empty());
        app.undo();
        assert!(app.doc.layers[0].pixels == blank);
        assert!(!app.history.can_undo());
        tick(&mut app, &ctx);
        assert!(app.doc.wet.is_empty());
    }

    /// Paint that is wet without a stroke in the history (or after some
    /// other action) dries as a step of its own.
    #[test]
    fn drying_in_the_app_is_one_undo_step_and_undo_stops_it() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(160, 80),
            ..Default::default()
        };
        stroke(&mut app.doc, &wash(), [200, 30, 30], 34.0);
        stroke(&mut app.doc, &wash(), [30, 30, 200], 54.0);
        let painted = app.doc.layers[0].pixels.clone();
        let mut ticks = 0;
        while !app.doc.wet.is_empty() {
            tick(&mut app, &ctx);
            ticks += 1;
            assert!(ticks < 5000, "drying ends");
        }
        assert!(
            app.doc.layers[0].pixels != painted,
            "paint spread while drying"
        );
        assert!(app.modified);
        // One undo takes back the drying, and only the drying.
        app.undo();
        assert!(app.doc.layers[0].pixels == painted);
        assert!(!app.history.can_undo());
        // Redo brings the dried result back; nothing is wet any more.
        app.redo();
        assert!(app.doc.layers[0].pixels != painted);
        tick(&mut app, &ctx);
        assert!(app.doc.wet.is_empty());
        // Undo in the middle of drying stops it.
        let mut app = Dudel {
            doc: Document::new(160, 80),
            ..Default::default()
        };
        stroke(&mut app.doc, &wash(), [200, 30, 30], 34.0);
        for _ in 0..10 {
            tick(&mut app, &ctx);
        }
        assert!(!app.doc.wet.is_empty());
        app.undo();
        tick(&mut app, &ctx);
        assert!(app.doc.wet.is_empty());
    }
}

/// `DUDEL_WET_SHEET=out.png cargo test --release wet_sheet -- --ignored
/// --nocapture` paints overlapping washes on cold-pressed paper, saves them
/// wet (top) and dried (bottom), and times drying a large wet area.
#[cfg(test)]
#[test]
#[ignore = "writes a picture and prints timings for review"]
fn wet_sheet() {
    use crate::document::Document;
    let Ok(path) = std::env::var("DUDEL_WET_SHEET") else {
        return;
    };
    let wash = crate::workspace::default_brushes()
        .into_iter()
        .find(|b| b.name == "Watercolor wash")
        .unwrap();
    let (w, h) = (640usize, 260usize);
    let mut doc = Document::new(w, h);
    doc.paper = Some(crate::paper::Paper::presets()[1].1);
    let run = |doc: &mut Document, color: [u8; 3], points: &[[f32; 2]], pressure: f32| {
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: wash.clone(),
            color,
            secondary: color,
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut path = crate::stroke::Stroke::default();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        let steps = 80;
        for i in 0..=steps {
            let t = i as f32 / steps as f32 * (points.len() - 1) as f32;
            let (a, b) = (
                points[t.floor() as usize],
                points[(t.ceil() as usize).min(points.len() - 1)],
            );
            let f = t - t.floor();
            let sample = crate::stroke::Sample {
                x: a[0] + (b[0] - a[0]) * f,
                y: a[1] + (b[1] - a[1]) * f,
                pressure,
                pressure_known: true,
                ..Default::default()
            };
            let dabs = if i == steps {
                path.finish(sample, &wash)
            } else {
                path.sample(sample, &wash)
            };
            for dab in dabs {
                buffer.paint(doc, dab, &settings);
            }
        }
    };
    for pass in 0..3 {
        let y = 70.0 + pass as f32 * 8.0;
        run(
            &mut doc,
            [36, 84, 170],
            &[[60.0, y], [330.0, y + 40.0]],
            0.9,
        );
    }
    for pass in 0..3 {
        let y = 150.0 + pass as f32 * 8.0;
        run(
            &mut doc,
            [200, 60, 50],
            &[[250.0, y], [580.0, y - 60.0]],
            0.9,
        );
    }
    run(
        &mut doc,
        [230, 180, 60],
        &[[120.0, 200.0], [300.0, 120.0], [480.0, 200.0]],
        0.7,
    );
    let mut sheet = doc.composite();
    let wet_tiles = doc.wet.tiles.len();
    let start = std::time::Instant::now();
    let mut steps = 0;
    while dry_step(&mut doc).is_some() {
        steps += 1;
    }
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{wet_tiles} wet tiles ({} px), {steps} steps, {:.2} ms per step",
        wet_tiles * TILE * TILE,
        elapsed / f64::from(steps.max(1))
    );
    sheet.extend(doc.composite());
    image::save_buffer(
        path,
        &sheet,
        w as u32,
        (h * 2) as u32,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    // A large wash: 2000 x 1000 px wet at once.
    let mut big = Document::new(2000, 1000);
    let row = vec![1.0f32; 2000];
    let id = big.layers[0].id;
    for y in 0..1000 {
        big.wet.add_row(id, y, 0, &row);
    }
    let start = std::time::Instant::now();
    for _ in 0..10 {
        dry_step(&mut big);
    }
    println!(
        "2,000,000 wet px: {:.2} ms per step",
        start.elapsed().as_secs_f64() * 100.0
    );
}
