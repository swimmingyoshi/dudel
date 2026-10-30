//! Canvas bounds edits preserve exact RGBA and selection coverage, without resampling.
use crate::{document::Document, pixels::Pixels, selection::Selection};
use std::sync::atomic::AtomicBool;

/// Session-local coordinate frame retained in undo snapshots. Guides are edited
/// independently of artwork history; frame changes map their current positions
/// rather than restoring an old guide snapshot. Saved guides already use current
/// coordinates, so loaded documents start at the identity frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Frame {
    turns: u8,
    translation: [i64; 2],
    generation: u64,
}
fn generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |n| n.checked_add(1),
    )
    .expect("Canvas frame identity exhausted")
}
impl Frame {
    pub(crate) fn identity() -> std::sync::Arc<Self> {
        static IDENTITY: std::sync::OnceLock<std::sync::Arc<Frame>> = std::sync::OnceLock::new();
        IDENTITY
            .get_or_init(|| std::sync::Arc::new(Frame::default()))
            .clone()
    }
    fn translated(self, offset: [i64; 2]) -> Result<Self, String> {
        Ok(Self {
            translation: [
                self.translation[0]
                    .checked_add(offset[0])
                    .ok_or("Canvas coordinates overflow")?,
                self.translation[1]
                    .checked_add(offset[1])
                    .ok_or("Canvas coordinates overflow")?,
            ],
            generation: generation(),
            ..self
        })
    }
    pub(crate) fn rotated(self, height: usize) -> Result<Self, String> {
        Ok(Self {
            turns: (self.turns + 1) % 4,
            generation: generation(),
            translation: [
                (height as i64)
                    .checked_sub(self.translation[1])
                    .ok_or("Canvas coordinates overflow")?,
                self.translation[0],
            ],
        })
    }
    pub(crate) fn turn_from(self, before: Self) -> u8 {
        (self.turns + 4 - before.turns) % 4
    }
    pub(crate) fn angle_from(self, before: Self) -> f32 {
        (self.turns as i8 - before.turns as i8) as f32 * 90.0
    }
    pub(crate) fn point_from(self, before: Self, point: [f32; 2]) -> [f32; 2] {
        let [x, y] = [
            point[0] as f64 - before.translation[0] as f64,
            point[1] as f64 - before.translation[1] as f64,
        ];
        let p = match self.turn_from(before) {
            0 => [x, y],
            1 => [-y, x],
            2 => [-x, -y],
            _ => [y, -x],
        };
        [
            (p[0] + self.translation[0] as f64) as f32,
            (p[1] + self.translation[1] as f64) as f32,
        ]
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub width: usize,
    pub height: usize,
    /// 0 = left/top, 1 = center, 2 = right/bottom.
    pub anchor: [u8; 2],
    /// None preserves the current resolution for existing scripted edits.
    #[serde(default)]
    pub dpi: Option<f64>,
}

impl crate::Dudel {
    pub(crate) fn open_canvas_size(&mut self) {
        if self.canvas_edit_ticket.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
        {
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.canvas_resize = Some(Settings {
            width: self.doc.width,
            height: self.doc.height,
            dpi: Some(self.doc.dpi),
            anchor: [1, 1],
        });
    }
    pub(crate) fn dispatch_canvas_size(&mut self, edit: crate::document_edits::Edit) {
        if self.canvas_edit_ticket.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
        {
            self.status = "A canvas bounds change is already running".into();
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        let (changed, unchanged) = match &edit {
            crate::document_edits::Edit::CanvasSize(_) => {
                ("Canvas resized", "Canvas size unchanged")
            }
            crate::document_edits::Edit::CropCanvas => (
                "Canvas cropped to selection bounds",
                "Canvas bounds unchanged",
            ),
            crate::document_edits::Edit::Rotate(_) => ("Image rotated", "Image unchanged"),
            _ => {
                self.status = "Invalid canvas operation".into();
                return;
            }
        };
        match self.render_queue.edit(edit) {
            Ok(ticket) => {
                self.canvas_edit_ticket = Some(ticket);
                self.edit_feedback.insert(
                    ticket,
                    crate::render_controller::EditFeedback { changed, unchanged },
                );
                self.status = "Canvas change queued after preceding artwork".into();
            }
            Err(error) => self.status = error,
        }
    }
    pub(crate) fn canvas_size_ui(&mut self, ctx: &crate::egui::Context) {
        use crate::egui;
        if let Some(ticket) = self.canvas_edit_ticket {
            egui::Modal::new(egui::Id::new("canvas bounds progress")).show(ctx, |ui| {
                ui.set_width(300.0);
                ui.heading("Updating canvas");
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finishing the canvas change…");
                });
                if ui.button("Cancel operation").clicked() {
                    self.render_queue.cancel_edit(ticket);
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
            return;
        }
        let Some(mut settings) = self.canvas_resize else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("canvas size")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading("Canvas size");
            egui::ScrollArea::vertical().id_salt("canvas size controls")
                .max_height((ctx.content_rect().height() - 96.0).max(80.0))
                .show(ui, |ui| {
            ui.label(format!("Current: {} × {} px", self.doc.width, self.doc.height));
            ui.horizontal(|ui| {
                ui.label("Width");
                ui.add(egui::DragValue::new(&mut settings.width).range(1..=crate::document::MAX_SIDE).suffix(" px"));
                ui.label("Height");
                ui.add(egui::DragValue::new(&mut settings.height).range(1..=crate::document::MAX_SIDE).suffix(" px"));
            });
            let dpi = settings.dpi.get_or_insert(self.doc.dpi);
            ui.horizontal(|ui| {
                ui.label("Resolution");
                ui.add(egui::DragValue::new(dpi).range(1.0..=12000.0).speed(1.0).suffix(" DPI"));
            });
            ui.small(format!("Print size: {:.2} \u{00d7} {:.2} cm", settings.width as f64 / *dpi * 2.54, settings.height as f64 / *dpi * 2.54));
            ui.small("Changing DPI changes print size; pixels stay the same.");
            ui.label("Anchor existing artwork");
            ui.horizontal(|ui| {
                egui::Grid::new("canvas anchor").spacing(egui::vec2(4.0,4.0)).show(ui, |ui| {
                    let names = [["Top left", "Top center", "Top right"], ["Middle left", "Center", "Middle right"], ["Bottom left", "Bottom center", "Bottom right"]];
                    for (y,row) in names.iter().enumerate() { for (x,name) in row.iter().enumerate() {
                        let anchor = [x as u8,y as u8];
                        if anchor_button(ui,anchor,settings.anchor == anchor,name).clicked() { settings.anchor = anchor; }
                    } ui.end_row(); }
                });
                let (area, _) = ui.allocate_exact_size(egui::vec2(144.0,120.0), egui::Sense::hover());
                let offset = settings.offset(self.doc.width, self.doc.height);
                let old = egui::Rect::from_min_size(egui::pos2(offset[0] as f32,offset[1] as f32), egui::vec2(self.doc.width as f32,self.doc.height as f32));
                let new = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(settings.width as f32,settings.height as f32));
                let both = old.union(new);
                let scale = (area.width()/both.width()).min(area.height()/both.height()) * 0.85;
                let rect = |r: egui::Rect| egui::Rect::from_min_max(area.center() + (r.min-both.center()) * scale, area.center() + (r.max-both.center()) * scale);
                let painter = ui.painter_at(area);
                painter.rect_filled(rect(new), 0.0, egui::Color32::from_rgb(43,73,104));
                painter.rect_stroke(rect(new), 0.0, egui::Stroke::new(2.0_f32,egui::Color32::from_rgb(140,190,240)),egui::StrokeKind::Inside);
                painter.rect_stroke(rect(old), 0.0, egui::Stroke::new(1.0_f32,egui::Color32::LIGHT_GRAY),egui::StrokeKind::Inside);
            });
            ui.small("Blue: new canvas · outline: current canvas");
            ui.label("Changes canvas bounds on every layer without scaling the artwork. Added space is transparent. Pixels outside the new bounds are removed; Undo restores them.");
            });
            let valid = settings.validate();
            if let Err(error) = &valid { ui.colored_label(ui.visuals().error_fg_color, error); }
            ui.horizontal(|ui| {
                apply = ui.add_enabled(valid.is_ok() && ([settings.width,settings.height] != [self.doc.width,self.doc.height] || settings.dpi.is_some_and(|dpi| dpi != self.doc.dpi)), egui::Button::new("Apply")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if apply {
            self.canvas_resize = None;
            self.dispatch_canvas_size(crate::document_edits::Edit::CanvasSize(settings));
        } else if cancel || response.should_close() {
            self.canvas_resize = None;
        } else {
            self.canvas_resize = Some(settings);
        }
    }
}
fn anchor_button(
    ui: &mut crate::egui::Ui,
    anchor: [u8; 2],
    selected: bool,
    name: &str,
) -> crate::egui::Response {
    use crate::egui;
    let response = ui
        .add(
            egui::Button::new("")
                .min_size(egui::vec2(32.0, 32.0))
                .selected(selected),
        )
        .on_hover_text(name);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, name)
    });
    let center = response.rect.center();
    let color = if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    if anchor == [1, 1] {
        ui.painter().circle_filled(center, 3.0, color);
    } else {
        let direction = egui::vec2(anchor[0] as f32 - 1.0, anchor[1] as f32 - 1.0).normalized();
        let tip = center + direction * 7.0;
        let back = direction * 5.0;
        let across = egui::vec2(-direction.y, direction.x) * 4.0;
        let stroke = egui::Stroke::new(1.5_f32, color);
        ui.painter()
            .line_segment([center - direction * 5.0, tip], stroke);
        ui.painter()
            .line_segment([tip - back + across, tip], stroke);
        ui.painter()
            .line_segment([tip - back - across, tip], stroke);
    }
    response
}
pub(crate) fn validate_size(width: usize, height: usize) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > crate::document::MAX_SIDE
        || height > crate::document::MAX_SIDE
        || width
            .checked_mul(height)
            .is_none_or(|n| n > crate::document::MAX_PIXELS)
    {
        return Err(format!("Canvas must be {}", crate::document::SIZE_LIMIT));
    }
    Ok(())
}
impl Settings {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_size(self.width, self.height)?;
        if let Some(dpi) = self.dpi {
            crate::document::validate_dpi(dpi)?;
        }
        if self.anchor.iter().any(|v| *v > 2) {
            return Err("Invalid canvas anchor".into());
        }
        Ok(())
    }
    pub(crate) fn offset(&self, width: usize, height: usize) -> [i64; 2] {
        [
            ((self.width as i64 - width as i64) * i64::from(self.anchor[0])).div_euclid(2),
            ((self.height as i64 - height as i64) * i64::from(self.anchor[1])).div_euclid(2),
        ]
    }
}

pub(crate) fn resize(
    source: &Document,
    settings: Settings,
    cancel: Option<&AtomicBool>,
) -> Result<(Document, bool), String> {
    settings.validate()?;
    source.validate()?;
    let (mut doc, mut changed) = reframe(
        source,
        [settings.width, settings.height],
        settings.offset(source.width, source.height),
        cancel,
    )?;
    if let Some(dpi) = settings.dpi {
        changed |= doc.dpi != dpi;
        doc.dpi = dpi;
    }
    Ok((doc, changed))
}
pub(crate) fn crop(
    source: &Document,
    cancel: Option<&AtomicBool>,
) -> Result<(Document, bool), String> {
    source.validate()?;
    let s = source
        .selection
        .ok_or("Select an area before cropping the canvas")?;
    reframe(
        source,
        [s.right - s.left, s.bottom - s.top],
        [-(s.left as i64), -(s.top as i64)],
        cancel,
    )
}
fn reframe(
    source: &Document,
    [width, height]: [usize; 2],
    offset: [i64; 2],
    cancel: Option<&AtomicBool>,
) -> Result<(Document, bool), String> {
    validate_size(width, height)?;
    crate::fill::check_cancel(cancel)?;
    if [width, height] == [source.width, source.height] && offset == [0, 0] {
        return Ok((source.clone(), false));
    }
    let mut doc = source.clone();
    doc.canvas_frame = source.canvas_frame.translated(offset)?.into();
    let translate = |s: Selection| {
        let left = (s.left as i64 + offset[0]).clamp(0, width as i64) as usize;
        let right = (s.right as i64 + offset[0]).clamp(0, width as i64) as usize;
        let top = (s.top as i64 + offset[1]).clamp(0, height as i64) as usize;
        let bottom = (s.bottom as i64 + offset[1]).clamp(0, height as i64) as usize;
        (left < right && top < bottom).then_some(Selection {
            left,
            top,
            right,
            bottom,
        })
    };
    // Source overlap bounds are computed before visiting any stored rows.
    let left = (-offset[0]).clamp(0, source.width as i64) as usize;
    let top = (-offset[1]).clamp(0, source.height as i64) as usize;
    let right = (width as i64 - offset[0]).clamp(left as i64, source.width as i64) as usize;
    let bottom = (height as i64 - offset[1]).clamp(top as i64, source.height as i64) as usize;
    for (out, layer) in doc.layers.iter_mut().zip(&source.layers) {
        if let Some(mask) = &mut out.mask {
            mask.coverage = mask.coverage.remap(width, height, 255, cancel, |[x, y]| {
                let sx = (x as i64).checked_sub(offset[0])?;
                let sy = (y as i64).checked_sub(offset[1])?;
                (sx >= 0 && sy >= 0 && sx < source.width as i64 && sy < source.height as i64)
                    .then_some([sx as usize, sy as usize])
            })?;
        }
        if layer.folder {
            continue;
        }
        let mut pixels = Pixels::sparse(width, height);
        {
            let mut writer = pixels.writer();
            layer.pixels.try_visit_rows(
                source.width,
                source.height,
                [left, top, right, bottom],
                |x, y, bytes| {
                    crate::fill::check_cancel(cancel)?;
                    let tx = (x as i64 + offset[0]) as usize;
                    let ty = (y as i64 + offset[1]) as usize;
                    // Skip all-zero pixels, but retain alpha-zero hidden RGB exactly.
                    for (i, rgba) in bytes.chunks_exact(4).enumerate() {
                        if rgba != [0; 4] {
                            writer.rgba_mut_at(width, tx + i, ty).copy_from_slice(rgba);
                        }
                    }
                    Ok::<_, String>(())
                },
            )?;
        }
        out.pixels = pixels;
    }
    if let Some(mask) = &source.selection_mask {
        let mut coverage = vec![0; width * height];
        for y in top..bottom {
            crate::fill::check_cancel(cancel)?;
            let ty = (y as i64 + offset[1]) as usize;
            let tx = (left as i64 + offset[0]) as usize;
            coverage[ty * width + tx..ty * width + tx + right - left]
                .copy_from_slice(&mask.bytes()[y * source.width + left..y * source.width + right]);
        }
        let mask =
            crate::selection_mask::Mask::from_coverage_control(width, height, coverage, cancel)?;
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
    } else {
        doc.selection = source.selection.and_then(translate);
        if source.selection.is_some() && doc.selection.is_none() {
            doc.selection_mask = Some(crate::selection_mask::Mask::from_coverage_control(
                width,
                height,
                vec![0; width * height],
                cancel,
            )?);
        }
    }
    doc.width = width;
    doc.height = height;
    if let Some(guides) = &mut doc.guides {
        guides.map_canvas(*source.canvas_frame, *doc.canvas_frame);
    }
    crate::fill::check_cancel(cancel)?;
    doc.validate()?;
    Ok((doc, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dudel, document_edits::Edit, selection_mask::Mask};

    #[test]
    fn dpi_edit_preserves_pixels_and_survives_history_and_save() {
        let mut app = crate::Dudel {
            doc: source(7, 9),
            ..Default::default()
        };
        let original = serde_json::to_value(&app.doc).unwrap();
        app.dispatch_canvas_size(crate::document_edits::Edit::CanvasSize(Settings {
            width: 7,
            height: 9,
            anchor: [1, 1],
            dpi: Some(600.5),
        }));
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(app.doc.dpi, 600.5);
        assert!(app.modified);
        let mut expected = original.clone();
        expected["dpi"] = serde_json::json!(600.5);
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected);
        app.undo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), original);
        app.redo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dpi.dudel");
        crate::storage::save_project(&path, &app.doc).unwrap();
        let loaded = crate::storage::load_project(&path).unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), expected);
        let mut legacy = expected.clone();
        legacy.as_object_mut().unwrap().remove("dpi");
        let legacy: Document = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.dpi, 72.0);
        for dpi in [0.0, -1.0, f64::NAN, f64::INFINITY, 12001.0] {
            assert!(
                resize(
                    &app.doc,
                    Settings {
                        width: 7,
                        height: 9,
                        anchor: [1, 1],
                        dpi: Some(dpi)
                    },
                    None
                )
                .is_err()
            );
        }
    }
    fn source(width: usize, height: usize) -> Document {
        let mut doc = Document::new(width, height);
        doc.add_layer();
        for (layer_index, layer) in doc.layers.iter_mut().enumerate() {
            let mut writer = layer.pixels.writer();
            for y in 0..height {
                for x in 0..width {
                    writer.rgba_mut_at(width, x, y).copy_from_slice(&[
                        x as u8,
                        (y * 29) as u8,
                        (layer_index * 80 + 11) as u8,
                        [0, 64, 255][(x + y) % 3],
                    ]);
                }
            }
        }
        doc.layers[0].locked = true;
        doc.layers[1].alpha_locked = true;
        doc.layers[1].opacity = 0.7;
        doc.layers[1].blend = crate::blend::BlendMode::Multiply;
        doc.add_folder().unwrap();
        let mut coverage = vec![0; width * height];
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                coverage[y * width + x] = (1 + (x + y) % 255) as u8;
            }
        }
        let mask = Mask::from_coverage(width, height, coverage).unwrap();
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
        doc.validate().unwrap();
        doc
    }
    fn reference(source: &Document, result: &Document, offset: [i64; 2]) {
        assert_eq!(source.layers.len(), result.layers.len());
        assert_eq!(source.active, result.active);
        for (a, b) in source.layers.iter().zip(&result.layers) {
            assert_eq!(
                (
                    a.id,
                    a.depth,
                    a.folder,
                    a.collapsed,
                    a.visible,
                    a.locked,
                    a.alpha_locked,
                    a.clipped
                ),
                (
                    b.id,
                    b.depth,
                    b.folder,
                    b.collapsed,
                    b.visible,
                    b.locked,
                    b.alpha_locked,
                    b.clipped
                )
            );
            assert_eq!(
                (&a.name, a.opacity, a.blend, a.csp_byte_opacity),
                (&b.name, b.opacity, b.blend, b.csp_byte_opacity)
            );
            if a.folder {
                assert!(b.pixels.is_empty());
                continue;
            }
            for y in 0..result.height {
                for x in 0..result.width {
                    let (sx, sy) = (x as i64 - offset[0], y as i64 - offset[1]);
                    let expected = if sx >= 0
                        && sy >= 0
                        && sx < source.width as i64
                        && sy < source.height as i64
                    {
                        a.pixels.rgba(sy as usize * source.width + sx as usize)
                    } else {
                        [0; 4]
                    };
                    assert_eq!(b.pixels.rgba(y * result.width + x), expected);
                }
            }
        }
        if let Some(mask) = &source.selection_mask {
            let mapped = result.selection_mask.as_ref().unwrap();
            for y in 0..result.height {
                for x in 0..result.width {
                    let (sx, sy) = (x as i64 - offset[0], y as i64 - offset[1]);
                    let expected = if sx >= 0
                        && sy >= 0
                        && sx < source.width as i64
                        && sy < source.height as i64
                    {
                        mask.at(sx as usize, sy as usize)
                    } else {
                        0
                    };
                    assert_eq!(mapped.at(x, y), expected);
                }
            }
            assert_eq!(result.selection, mapped.bounds());
        }
        result.validate().unwrap();
    }
    #[test]
    fn nine_anchors_preserve_exact_layer_bytes_mask_and_metadata_across_tile_edges() {
        for flat in [false, true] {
            let mut doc = source(513, 5);
            if flat {
                for layer in doc.layers.iter_mut().filter(|l| !l.folder) {
                    layer.pixels = layer.pixels.to_vec().into();
                }
            }
            let before = serde_json::to_value(&doc).unwrap();
            for anchor_y in 0..3 {
                for anchor_x in 0..3 {
                    for [width, height] in [[517, 9], [507, 3], [519, 2], [1, 1]] {
                        let settings = Settings {
                            width,
                            height,
                            anchor: [anchor_x, anchor_y],
                            dpi: None,
                        };
                        let (result, changed) = resize(&doc, settings, None).unwrap();
                        assert!(changed);
                        reference(&doc, &result, settings.offset(doc.width, doc.height));
                    }
                }
            }
            assert_eq!(serde_json::to_value(&doc).unwrap(), before);
        }
    }
    #[test]
    fn crop_preserves_coverage_hidden_rgb_and_empty_selection_does_not_select_everything() {
        let doc = source(9, 7);
        let s = doc.selection.unwrap();
        let (cropped, changed) = crop(&doc, None).unwrap();
        assert!(changed);
        assert_eq!(
            [cropped.width, cropped.height],
            [s.right - s.left, s.bottom - s.top]
        );
        reference(&doc, &cropped, [-(s.left as i64), -(s.top as i64)]);
        let mut rect = Document::new(5, 5);
        rect.selection = Some(Selection {
            left: 3,
            top: 3,
            right: 5,
            bottom: 5,
        });
        let (empty, _) = resize(
            &rect,
            Settings {
                width: 1,
                height: 1,
                anchor: [0, 0],
                dpi: None,
            },
            None,
        )
        .unwrap();
        assert!(empty.selection.is_none());
        assert_eq!(empty.selection_mask.unwrap().bytes(), &[0]);
    }
    #[test]
    fn no_op_invalid_limits_and_cancellation_preserve_source_and_sparse_empty_layers() {
        let doc = Document::new(2048, 2048);
        let same = Settings {
            width: 2048,
            height: 2048,
            anchor: [1, 1],
            dpi: None,
        };
        let (unchanged, changed) = resize(&doc, same, None).unwrap();
        assert!(!changed && crate::document::same_snapshot(&doc, &unchanged));
        let (small, _) = resize(
            &doc,
            Settings {
                width: 257,
                height: 259,
                anchor: [2, 2],
                dpi: None,
            },
            None,
        )
        .unwrap();
        assert!(small.layers[0].pixels.nonzero_tiles(257, 259).is_empty());
        for [width, height] in [[0, 1], [16385, 1], [10001, 10000], [usize::MAX, 1]] {
            assert!(
                resize(
                    &doc,
                    Settings {
                        width,
                        height,
                        anchor: [1, 1],
                        dpi: None,
                    },
                    None
                )
                .is_err()
            );
        }
        assert!(
            resize(
                &doc,
                Settings {
                    anchor: [3, 0],
                    dpi: None,
                    ..same
                },
                None
            )
            .is_err()
        );
        let cancel = AtomicBool::new(true);
        assert!(
            resize(
                &doc,
                Settings {
                    width: 1,
                    height: 1,
                    ..same
                },
                Some(&cancel)
            )
            .is_err()
        );
        assert!(crop(&doc, None).is_err());
        let mut invalid = Document::new(5, 5);
        invalid.selection = Some(Selection {
            left: 3,
            top: 0,
            right: 0,
            bottom: 1,
        });
        assert!(crop(&invalid, None).is_err());
        assert!(
            resize(
                &invalid,
                Settings {
                    width: 3,
                    height: 3,
                    anchor: [1, 1],
                    dpi: None,
                },
                None
            )
            .is_err()
        );
        assert!(doc.layers[0].pixels.nonzero_tiles(2048, 2048).is_empty());
    }
    #[test]
    fn resize_crop_rotation_and_undo_map_current_guides_and_roundtrip_off_canvas_positions() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            doc: source(7, 5),
            ..Default::default()
        };
        app.guide_lines = vec![
            crate::guides::GuideLine {
                vertical: true,
                position: 4.0,
            },
            crate::guides::GuideLine {
                vertical: false,
                position: 0.0,
            },
        ];
        app.symmetry.center = Some([2.0, 2.0]);
        app.push_ruler(crate::rulers::Ruler::Focus { center: [3.0, 4.0] });
        let original = serde_json::to_value(&app.doc).unwrap();
        app.dispatch_canvas_size(Edit::CanvasSize(Settings {
            width: 11,
            height: 9,
            anchor: [1, 1],
            dpi: None,
        }));
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!(app.symmetry.center, Some([4.0, 4.0]));
        assert_eq!(app.guide_lines[0].position, 6.0);
        app.guide_lines[0].position += 1.0; // Independently edited guides must not be undone.
        app.undo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), original);
        assert_eq!(app.guide_lines[0].position, 5.0);
        app.redo();
        assert_eq!(app.guide_lines[0].position, 7.0);
        let resized = serde_json::to_value(&app.doc).unwrap();
        app.rotate_canvas(1);
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!([app.doc.width, app.doc.height], [9, 11]);
        assert!(!app.guide_lines[0].vertical);
        assert_eq!(app.guide_lines[0].position, 7.0);
        app.undo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), resized);
        assert!(app.guide_lines[0].vertical);
        assert_eq!(app.symmetry.center, Some([4.0, 4.0]));
        app.dispatch_canvas_size(Edit::CropCanvas);
        crate::workflow_tests::finish_jobs(&mut app);
        assert_eq!([app.doc.width, app.doc.height], [5, 3]);
        assert_eq!(app.guide_lines[1].position, -1.0);
        let expected = serde_json::to_value(&app.doc).unwrap();
        let guides = app.guides_record(&app.doc);
        app.undo();
        app.redo();
        assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected);
        assert_eq!(app.guides_record(&app.doc), guides);
        let project = directory.path().join("cropped.dudel");
        app.path = Some(project.clone());
        app.save(false);
        crate::workflow_tests::finish_jobs(&mut app);
        let mut reopened = Dudel::default();
        reopened.load_document(project, false);
        crate::workflow_tests::finish_jobs(&mut reopened);
        assert_eq!(serde_json::to_value(&reopened.doc).unwrap(), expected);
        assert_eq!(reopened.guides_record(&reopened.doc), guides);
        assert_eq!(*reopened.doc.canvas_frame, Frame::default());
    }
    #[test]
    fn ordered_canvas_change_cancellation_has_no_partial_artwork_or_history() {
        let mut app = Dudel {
            doc: source(7, 5),
            ..Default::default()
        };
        let before = app.doc.clone();
        app.dispatch_canvas_size(Edit::CanvasSize(Settings {
            width: 9,
            height: 9,
            anchor: [2, 2],
            dpi: None,
        }));
        let ticket = app.canvas_edit_ticket.unwrap();
        assert!(app.render_queue.cancel_edit(ticket));
        crate::workflow_tests::finish_jobs(&mut app);
        assert!(app.canvas_edit_ticket.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &before));
        assert!(!app.history.can_undo());
        assert!(app.status.contains("canceled"));
    }
    #[test]
    fn queued_stroke_resize_save_and_export_share_the_new_bounds_and_reject_old_input() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = Dudel {
            doc: Document::new(33, 35),
            ..Default::default()
        };
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 9.0,
                taper_out: 20.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [30, 40, 50],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let sample = crate::stroke::Sample {
            x: 7.5,
            y: 9.5,
            pressure: 1.0,
            ..Default::default()
        };
        let mut painted = app.doc.clone();
        let mut reference = crate::stroke_buffer::StrokeBuffer::default();
        reference.paint(&mut painted, sample, &settings);
        reference.finish_tail(&mut painted, &settings);
        assert!(app.begin_canvas_stroke(settings.clone()));
        app.deliver_canvas_dabs(vec![sample]);
        let bounds = Settings {
            width: 41,
            height: 39,
            anchor: [1, 1],
            dpi: None,
        };
        app.dispatch_canvas_size(Edit::CanvasSize(bounds));
        assert!(app.canvas_edit_ticket.is_some() && !app.stroking);
        assert!(!app.begin_canvas_stroke(settings));
        let project = directory.path().join("resized.dudel");
        let png = directory.path().join("resized.png");
        app.path = Some(project.clone());
        app.save(false);
        app.export_to(png.clone(), true);
        crate::workflow_tests::finish_jobs(&mut app);
        let (expected, _) = resize(&painted, bounds, None).unwrap();
        let reopened = crate::storage::load_project(&project).unwrap();
        assert_eq!([reopened.width, reopened.height], [41, 39]);
        assert_eq!(reopened.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(
            image::open(png).unwrap().into_rgba8().as_raw(),
            &expected.composite_transparent()
        );
        assert!(!app.modified);
        app.undo();
        assert_eq!([app.doc.width, app.doc.height], [33, 35]);
        assert_eq!(app.doc.layers[0].pixels, painted.layers[0].pixels);
        app.undo();
        assert!(app.doc.layers[0].pixels.nonzero_tiles(33, 35).is_empty());
    }
    #[test]
    fn geometry_snapshot_metadata_is_shared_and_not_reported_as_raster_payload() {
        let before = Document::new(8, 8);
        let (after, _) = resize(
            &before,
            Settings {
                width: 12,
                height: 12,
                anchor: [0, 0],
                dpi: None,
            },
            None,
        )
        .unwrap();
        let snapshot = crate::snapshot_memory::Snapshot::capture(&before);
        let one = crate::snapshot_memory::beyond_live(&after, [&snapshot]);
        let two = crate::snapshot_memory::beyond_live(&after, [&snapshot, &snapshot]);
        assert_eq!(two.pixel_bytes_beyond_live, one.pixel_bytes_beyond_live);
        assert_eq!(
            two.metadata_bytes,
            2 * one.metadata_bytes - std::mem::size_of::<Frame>()
        );
    }
}
