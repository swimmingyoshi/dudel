use crate::{Dudel, brush::Brush, document::Document, egui};
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_capture_rejects_late_screenshot_and_preserves_isolation_while_closing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("must-not-publish.png");
        let mut app = Dudel {
            capture: Some(Capture {
                path: path.clone(),
                frames: 7,
                popup: None,
                session: None,
                pointer: None,
                requested: true,
                failed: true,
                started: std::time::Instant::now() - std::time::Duration::from_secs(30),
            }),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Screenshot {
                    viewport_id: egui::ViewportId::ROOT,
                    user_data: egui::UserData::default(),
                    image: std::sync::Arc::new(egui::ColorImage::filled(
                        [1, 1],
                        egui::Color32::RED,
                    )),
                }],
                ..Default::default()
            },
            |ctx| app.capture_frame(ctx),
        );
        assert!(!path.exists());
        assert_eq!(app.capture.as_ref().unwrap().frames, 7);
        assert!(output.viewport_output.values().any(|viewport| {
            viewport
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        }));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Popup {
    Brushes,
    Color,
    StrokeOptions,
    ViewMenu,
    FileMenu,
    Symmetry,
    Guides,
}

pub struct Capture {
    pub path: PathBuf,
    pub frames: usize,
    pub popup: Option<Popup>,
    pub(crate) session: Option<crate::native_session::Session>,
    /// A pointer held over the window, for cursor captures.
    pub pointer: Option<egui::Pos2>,
    requested: bool,
    failed: bool,
    started: std::time::Instant,
}
impl Dudel {
    pub(crate) fn open_capture_popup(&self, response: &egui::Response, popup: Popup) {
        self.open_capture_popup_id(
            &response.ctx,
            egui::Popup::default_response_id(response),
            popup,
        );
    }
    /// For popups shown under a fixed ID rather than their button's.
    pub(crate) fn open_capture_popup_id(&self, ctx: &egui::Context, id: egui::Id, popup: Popup) {
        if self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.frames <= 2 && capture.popup == Some(popup))
        {
            egui::Popup::open_id(ctx, id);
        }
    }
    pub(crate) fn capture_frame(&mut self, ctx: &egui::Context) {
        if self.capture.as_ref().is_some_and(|capture| capture.failed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some(mut session) = self.capture.as_mut().and_then(|c| c.session.take()) {
            match session.update(self, ctx) {
                Ok(false) => self.capture.as_mut().unwrap().session = Some(session),
                Ok(true) => {
                    let capture = self.capture.as_mut().unwrap();
                    capture.path = capture.path.with_extension("png");
                    capture.started = std::time::Instant::now();
                    capture.frames = 0;
                }
                Err(error) => {
                    eprintln!("Native session failed: {error}");
                    if let Err(report_error) =
                        session.write_failure_report(&self.capture.as_ref().unwrap().path, &error)
                    {
                        eprintln!("Could not preserve incomplete native evidence: {report_error}");
                    }
                    // Preserve isolated capture mode while the window closes;
                    // later callbacks must not enter the screenshot stage.
                    self.capture.as_mut().unwrap().failed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            return;
        }
        let Some(capture) = &mut self.capture else {
            return;
        };
        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        }) {
            let bytes = image
                .pixels
                .iter()
                .flat_map(|p| p.to_array())
                .collect::<Vec<_>>();
            match image::save_buffer_with_format(
                &capture.path,
                &bytes,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            ) {
                Ok(()) => println!("Captured UI to {}", capture.path.display()),
                Err(e) => eprintln!("UI capture failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        capture.frames += 1;
        if capture.frames >= 5
            && capture.started.elapsed() >= std::time::Duration::from_millis(400)
            && !capture.requested
            && self.preview.capture.is_none()
            && !self.texture.busy()
            && !self.render_queue.busy()
            && (!self.show_panels || self.thumbnail_revision == Some(self.revision))
            && self.adjustment.as_ref().is_none_or(|p| p.ready())
            && (!self.brush_studio || !self.brush_studio_preview.busy())
            && self
                .affine_preview
                .as_ref()
                .is_none_or(|p| !p.needs_repaint())
        {
            capture.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if capture.started.elapsed() > std::time::Duration::from_secs(15) {
            eprintln!("UI capture timed out");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
    pub(crate) fn for_capture(path: PathBuf) -> Self {
        let mut app = Self {
            capture: Some(Capture {
                path,
                frames: 0,
                popup: None,
                session: None,
                pointer: None,
                requested: false,
                failed: false,
                started: std::time::Instant::now(),
            }),
            doc: Document::new(1024, 768),
            ..Self::default()
        };
        app.doc.layers[0].name = "Color study".into();
        let brush = Brush {
            size: 80.0,
            pressure_size: false,
            ..Brush::default()
        };
        for index in 0..240 {
            let t = index as f32 / 240.0 * std::f32::consts::TAU;
            app.doc.dab(
                512.0 + 160.0 * t.cos(),
                384.0 + 160.0 * t.sin(),
                1.0,
                &brush,
                [80, 132, 187],
                false,
            );
        }
        app.doc.add_layer();
        app.doc.layers[1].name = "Highlights".into();
        let brush = Brush {
            size: 16.0,
            pressure_size: false,
            ..Brush::default()
        };
        for index in 0..100 {
            let t = 3.3 + index as f32 / 100.0 * 1.5;
            app.doc.dab(
                512.0 + 160.0 * t.cos(),
                384.0 + 160.0 * t.sin(),
                1.0,
                &brush,
                [208, 230, 245],
                false,
            );
        }
        app.doc.active = 1;
        let _ = app.doc.group_active();
        app.doc.layers[2].name = "Shading".into();
        app.doc.active = 2;
        app.doc.add_layer();
        app.doc.layers[3].name = "Linework".into();
        app.symmetry.mode = crate::symmetry::Mode::LeftRight;
        app.saved_guides = app.guides_record(&app.doc);
        app.status = "Sample document · UI validation".into();
        app
    }
}
