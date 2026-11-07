use crate::{Dudel, document::Layer, egui, plugins};
use std::sync::atomic::Ordering;

pub struct FilterJob {
    pub worker: crate::background::Handle<Result<Layer, String>>,
    pub control: plugins::FilterControl,
    pub revision: u64,
    pub layer_index: usize,
    pub total: usize,
}
impl Dudel {
    pub(crate) fn cancel_filter(&mut self) {
        if let Some(job) = &self.filter_job {
            job.control.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn commit_filter(
        &mut self,
        revision: u64,
        layer_index: usize,
        canceled: bool,
        result: Result<Layer, String>,
    ) {
        match result {
            Ok(layer)
                if !canceled
                    && !self.stroking
                    && !self.render_queue.busy()
                    && self.opacity_edit.is_none()
                    && self.text_edit.is_none()
                    && revision == self.revision
                    && layer_index < self.doc.layers.len() =>
            {
                if layer.pixels != self.doc.layers[layer_index].pixels {
                    self.history.checkpoint(&self.doc);
                    self.doc.layers[layer_index] = layer;
                    self.changed();
                    self.status = "Filter applied".into();
                } else {
                    self.status = "Filter made no changes".into();
                }
            }
            Ok(_) => {
                self.status = if canceled {
                    "Filter canceled".into()
                } else {
                    "Filter result discarded because artwork changed; run it again".into()
                }
            }
            Err(e) => self.status = format!("Filter: {e}"),
        }
    }
    pub(crate) fn poll_filter(&mut self, ctx: &egui::Context) {
        if self
            .filter_job
            .as_ref()
            .is_some_and(|job| job.worker.is_finished())
        {
            let job = self.filter_job.take().unwrap();
            let canceled = job.control.cancel.load(Ordering::Relaxed);
            match job.worker.join() {
                Ok(result) => self.commit_filter(job.revision, job.layer_index, canceled, result),
                Err(_) => self.status = "Filter worker failed; artwork was preserved".into(),
            }
        }

        if self.filter_job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    /// Filter progress, drawn inside the status row like save progress.
    /// Returns false when no filter is running.
    pub(crate) fn filter_progress_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(job) = &self.filter_job else {
            return false;
        };
        ui.label("Applying filter");
        ui.add(
            egui::ProgressBar::new(
                job.control.progress.load(Ordering::Relaxed) as f32 / job.total.max(1) as f32,
            )
            .desired_width(160.0)
            .desired_height(14.0)
            .show_percentage(),
        );
        if ui.add(egui::Button::new("Cancel").small()).clicked() {
            job.control.cancel.store(true, Ordering::Relaxed);
        }
        true
    }
}
#[cfg(test)]
mod gesture_order_tests {
    #[test]
    fn matching_revision_filter_cannot_commit_during_closed_pending_rendering() {
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(32, 32),
            ..Default::default()
        };
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: Default::default(),
            color: [17, 83, 201],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        assert!(app.begin_canvas_stroke(settings));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 16.5,
            y: 16.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        app.finish_painted_stroke();
        assert!(!app.stroking && app.render_queue.busy());
        let before = app.doc.layers[0].pixels.clone();
        let mut result = app.doc.layers[0].clone();
        result
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 255]);
        app.commit_filter(app.revision, 0, false, Ok(result));
        assert_eq!(app.doc.layers[0].pixels, before);
        app.flush_render_queue();
        assert_ne!(app.doc.layers[0].pixels, before);
    }
    #[test]
    fn a_filter_cannot_commit_during_an_opacity_transaction() {
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(1, 1),
            ..Default::default()
        };
        app.set_layer_opacity(0, 0.4);
        let before = app.doc.layers[0].pixels.clone();
        let mut result = app.doc.layers[0].clone();
        result
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 255]);
        app.commit_filter(app.revision, 0, false, Ok(result));
        assert_eq!(app.doc.layers[0].pixels, before);
        assert!(app.history.has_pending_stroke());
        app.finish_opacity_edit();
        assert!(!app.history.has_pending_stroke());
    }
    #[test]
    fn matching_revision_filter_does_not_commit_beneath_an_open_stroke() {
        let (mut app, _) = crate::edit_order_tests::pending_taper();
        let preview = app.doc.layers[0].pixels.clone();
        let mut result = app.doc.layers[0].clone();
        result
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[1, 2, 3, 255]);
        app.commit_filter(app.revision, 0, false, Ok(result));
        assert_eq!(app.doc.layers[0].pixels, preview);
        assert!(app.stroking && app.history.has_pending_stroke());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sparse_filter_commit_and_history_never_materialize_a_whole_layer() {
        let mut app = Dudel {
            doc: Document::new(2048, 2048),
            ..Default::default()
        };
        for index in [0, 2048 * 2048 - 1] {
            app.doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(index)
                .copy_from_slice(&[10, 30, 200, 128]);
        }
        let before = app.doc.layers[0].pixels.clone();
        let mut layer = app.doc.layers[0].clone();
        crate::plugins::apply_filter_controlled(
            &mut layer,
            "fn filter(r,g,b,a) {[255-r,255-g,255-b,a]}",
            None,
            2048,
            &plugins::FilterControl::default(),
        )
        .unwrap();
        app.commit_filter(app.revision, 0, false, Ok(layer));
        let expected_capacity = 2 * crate::pixels::SIDE * crate::pixels::SIDE * 4;
        assert_eq!(before.capacity(), expected_capacity);
        assert_eq!(app.doc.layers[0].pixels.capacity(), expected_capacity);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, before);
        assert_eq!(app.doc.layers[0].pixels.capacity(), expected_capacity);
        assert_eq!(before.capacity(), expected_capacity);
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(
            app.doc.layers[0].pixels.rgba(2048 * 2048 - 1),
            [245, 225, 55, 128]
        );
        assert_eq!(app.doc.layers[0].pixels.capacity(), expected_capacity);
    }
    use super::*;
    use crate::document::Document;
    #[test]
    fn actual_wasm_result_commits_and_undo_restores_original() {
        let mut app = Dudel {
            doc: Document::new(1, 1),
            ..Dudel::default()
        };
        app.doc.layers[0].pixels = vec![10, 30, 200, 128].into();
        let before = app.doc.layers[0].pixels.clone();
        let mut layer = app.doc.layers[0].clone();
        let bytes = wat::parse_str(include_str!("../plugins/example-invert.wat")).unwrap();
        crate::wasm_filters::apply(
            &mut layer,
            &bytes,
            None,
            1,
            &plugins::FilterControl::default(),
        )
        .unwrap();
        app.commit_filter(app.revision, 0, false, Ok(layer));
        assert_eq!(app.doc.layers[0].pixels, vec![245, 225, 55, 128]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, before);
        assert!(!app.history.undo(&mut app.doc));
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, vec![245, 225, 55, 128]);
    }
    #[test]
    fn canceled_and_stale_results_do_not_replace_artwork() {
        let mut app = Dudel {
            doc: Document::new(1, 1),
            ..Dudel::default()
        };
        let mut result = app.doc.layers[0].clone();
        result.pixels = vec![255, 0, 0, 255].into();
        app.commit_filter(app.revision, 0, true, Ok(result.clone()));
        assert_eq!(app.doc.layers[0].pixels, vec![0; 4]);
        let old_revision = app.revision;
        app.doc.layers[0].pixels = vec![0, 255, 0, 255].into();
        app.changed();
        app.commit_filter(old_revision, 0, false, Ok(result));
        assert_eq!(app.doc.layers[0].pixels, vec![0, 255, 0, 255]);
    }
    #[test]
    fn successful_result_commits_once_and_is_undoable() {
        let mut app = Dudel {
            doc: Document::new(1, 1),
            ..Dudel::default()
        };
        let mut result = app.doc.layers[0].clone();
        result.pixels = vec![255, 0, 0, 255].into();
        app.commit_filter(app.revision, 0, false, Ok(result));
        assert_eq!(app.doc.layers[0].pixels, vec![255, 0, 0, 255]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, vec![0; 4]);
        assert!(!app.history.undo(&mut app.doc));
    }
}
