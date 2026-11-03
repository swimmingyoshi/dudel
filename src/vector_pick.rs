//! Single lines on a vector layer, handled with the Pointer tool (O): a
//! press on a line picks it (Shift adds or removes it), a lasso drawn from
//! empty canvas picks the lines inside it, dragging moves the picked lines,
//! and the tool panel resizes, smooths, tapers, recolors or deletes them.
//! The pick follows the strokes themselves, so undo or other edits that
//! replace a line simply drop it from the pick.
use crate::{
    Dudel,
    document::LayerId,
    egui,
    vector_layer::{Stroke, VectorLayer},
    view::View,
};
use egui::{Color32, Pos2, Shape};
use std::sync::Arc;

/// How far from a line, in screen points, a press still picks it.
const REACH: f32 = 6.0;
/// Largest line size the panel sets, in canvas pixels.
pub(crate) const MAX_SIZE: f32 = 300.0;

/// The picked lines of one vector layer.
#[derive(Default)]
pub(crate) struct Pick {
    layer: Option<LayerId>,
    lines: Vec<Arc<Stroke>>,
    drag: Option<Drag>,
    lasso: Option<Lasso>,
}

/// A lasso being drawn around lines, in canvas pixels.
struct Lasso {
    points: Vec<[f32; 2]>,
    /// Shift: the lines join the pick instead of replacing it.
    adds: bool,
}

/// Lasso points kept at most.
const MAX_LASSO: usize = 20_000;

/// Whether (`x`, `y`) lies inside `polygon` (even-odd rule).
fn inside(polygon: &[[f32; 2]], x: f32, y: f32) -> bool {
    let mut odd = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let ([xi, yi], [xj, yj]) = (polygon[i], polygon[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// Picked lines being dragged: they move live, in one undo step.
struct Drag {
    grab: [f32; 2],
    offset: [f32; 2],
    /// The layer's strokes when the drag began, and which of them move.
    before: Vec<Arc<Stroke>>,
    moving: Vec<usize>,
    /// Where the moving lines were last drawn.
    drawn: Option<[usize; 4]>,
    applied: [f32; 2],
    checkpointed: bool,
}

fn union(a: Option<[usize; 4]>, b: Option<[usize; 4]>) -> Option<[usize; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, b) => a.or(b),
    }
}

/// A line's size as the panel shows it: its widest diameter.
pub(crate) fn line_size(stroke: &Stroke) -> f32 {
    match &stroke.csp {
        Some(line) => (line.width * 2.0) as f32,
        None => stroke.brush.size,
    }
}

/// `stroke` drawn `size` wide.
fn resized(stroke: &Stroke, size: f32) -> Stroke {
    let mut out = stroke.clone();
    match &stroke.csp {
        Some(line) => {
            let mut line = (**line).clone();
            line.width = f64::from(size.clamp(0.1, MAX_SIZE)) / 2.0;
            out.csp = Some(Arc::new(line));
        }
        None => {
            let mut brush = (*stroke.brush).clone();
            brush.size = size.clamp(1.0, MAX_SIZE);
            out.brush = Arc::new(brush);
        }
    }
    out.fresh()
}

impl Dudel {
    /// The index of the layer whose lines are picked, while it is the
    /// active vector layer.
    fn picked_layer(&self) -> Option<usize> {
        let index = self.vector_target()?;
        (self.vector_pick.layer == Some(self.doc.layers[index].id)).then_some(index)
    }
    /// The picked lines still on the active vector layer.
    pub(crate) fn picked_lines(&self) -> Vec<Arc<Stroke>> {
        let Some(index) = self.picked_layer() else {
            return Vec::new();
        };
        let strokes = &self.doc.layers[index].vector.as_ref().unwrap().strokes;
        self.vector_pick
            .lines
            .iter()
            .filter(|l| strokes.iter().any(|s| Arc::ptr_eq(s, l)))
            .cloned()
            .collect()
    }
    /// The topmost line of layer `index` within `reach` canvas pixels.
    fn line_at(&self, index: usize, at: Pos2, reach: f32) -> Option<Arc<Stroke>> {
        let size = [self.doc.width, self.doc.height];
        let vector = self.doc.layers[index].vector.as_ref()?;
        vector
            .strokes
            .iter()
            .rev()
            .find(|s| s.touches(at.x, at.y, reach, size))
            .cloned()
    }
    /// Whether a Pointer press at `pos` picks lines of the active vector
    /// layer (on a line, or anywhere with the lasso on), so the Pointer
    /// does not also pan.
    pub(crate) fn line_grabs(&self, view: View, pos: Pos2) -> bool {
        self.vector_pick.drag.is_some()
            || self.vector_pick.lasso.is_some()
            || self.vector_target().is_some_and(|index| {
                self.pointer_lasso
                    || self
                        .line_at(index, view.to_doc(pos), REACH / self.zoom)
                        .is_some()
            })
    }
    fn line_edit_blocker(&self, index: usize) -> Option<&'static str> {
        if self.doc.layers[index].locked {
            Some("Unlock the layer to edit its lines")
        } else if !self.doc.shown(index) {
            Some("Show the layer and its folder to edit its lines")
        } else {
            None
        }
    }
    /// With the Pointer tool, presses on the active vector layer's lines
    /// pick them and drags move them. Returns whether the packet was used.
    pub(crate) fn vector_pick_packet(
        &mut self,
        view: View,
        pos: Pos2,
        start: bool,
        end: bool,
        on_area: bool,
        shift: bool,
    ) -> bool {
        if self.tool != crate::tools::Tool::Pointer {
            self.finish_line_drag();
            return false;
        }
        let at = view.to_doc(pos);
        if start {
            self.finish_line_drag();
            let Some(index) = self.vector_target().filter(|_| on_area) else {
                return false;
            };
            let id = self.doc.layers[index].id;
            if self.vector_pick.layer != Some(id) {
                self.vector_pick = Pick {
                    layer: Some(id),
                    ..Default::default()
                };
            }
            self.vector_pick.lines = self.picked_lines();
            let Some(hit) = self.line_at(index, at, REACH / self.zoom) else {
                if self.pointer_lasso {
                    self.vector_pick.lasso = Some(Lasso {
                        points: vec![[at.x, at.y]],
                        adds: shift,
                    });
                    return true;
                }
                // Empty canvas: let go of the lines and let the Pointer pan.
                if !shift {
                    self.vector_pick.lines.clear();
                }
                return false;
            };
            let lines = &mut self.vector_pick.lines;
            match lines.iter().position(|l| Arc::ptr_eq(l, &hit)) {
                Some(i) if shift => {
                    lines.remove(i);
                    return true;
                }
                Some(_) => {}
                None if shift => lines.push(hit),
                None => *lines = vec![hit],
            }
            if let Some(blocker) = self.line_edit_blocker(index) {
                self.status = blocker.into();
                return true;
            }
            let strokes = self.doc.layers[index]
                .vector
                .as_ref()
                .unwrap()
                .strokes
                .clone();
            let moving: Vec<usize> = strokes
                .iter()
                .enumerate()
                .filter(|(_, s)| self.vector_pick.lines.iter().any(|l| Arc::ptr_eq(l, s)))
                .map(|(i, _)| i)
                .collect();
            let size = [self.doc.width, self.doc.height];
            let drawn = moving
                .iter()
                .map(|&i| strokes[i].bounds(size))
                .fold(None, union);
            self.vector_pick.drag = Some(Drag {
                grab: [at.x, at.y],
                offset: [0.0; 2],
                before: strokes,
                moving,
                drawn,
                applied: [0.0; 2],
                checkpointed: false,
            });
            return true;
        }
        if let Some(lasso) = &mut self.vector_pick.lasso {
            let last = *lasso.points.last().unwrap();
            let step = 2.0 / self.zoom.max(1e-3);
            if (at.x - last[0]).hypot(at.y - last[1]) >= step && lasso.points.len() < MAX_LASSO {
                lasso.points.push([at.x, at.y]);
            }
            if end {
                self.finish_lasso();
            }
            return true;
        }
        let Some(drag) = &mut self.vector_pick.drag else {
            return false;
        };
        drag.offset = [at.x - drag.grab[0], at.y - drag.grab[1]];
        if end {
            self.finish_line_drag();
        }
        true
    }
    /// Move the dragged lines to where the pointer is (once a frame).
    pub(crate) fn apply_line_drag(&mut self) {
        let Some(drag) = &self.vector_pick.drag else {
            return;
        };
        if drag.offset == drag.applied {
            return;
        }
        let Some(index) = self.picked_layer() else {
            self.vector_pick.drag = None;
            return;
        };
        if !drag.checkpointed {
            // Strokes still rendering must land before lines move.
            self.finish_layer_rename(true);
            self.flush_render_queue();
            self.history.begin_gesture_checkpoint(&self.doc);
        }
        let drag = self.vector_pick.drag.as_mut().unwrap();
        drag.checkpointed = true;
        drag.applied = drag.offset;
        let size = [self.doc.width, self.doc.height];
        let [dx, dy] = drag.offset;
        let mut strokes = drag.before.clone();
        let mut lines = Vec::new();
        let mut drawn = None;
        for &i in &drag.moving {
            let moved = Arc::new(drag.before[i].mapped(&|x, y| [x + dx, y + dy], size));
            drawn = union(drawn, moved.bounds(size));
            strokes[i] = moved.clone();
            lines.push(moved);
        }
        let rect = union(drag.drawn, drawn);
        drag.drawn = drawn;
        self.vector_pick.lines = lines;
        let layer = &mut self.doc.layers[index];
        let vector = VectorLayer { strokes };
        if let Some(rect) = rect {
            if let Err(error) = vector.redraw(&mut layer.pixels, rect, size, &|| false) {
                self.status = error;
            }
            self.dirty_region.include(rect);
        }
        layer.vector = Some(vector);
        self.changed();
    }
    /// Close the lasso: the lines entirely inside it are picked. A lasso
    /// too small to hold anything is a click on empty canvas.
    fn finish_lasso(&mut self) {
        let Some(lasso) = self.vector_pick.lasso.take() else {
            return;
        };
        if !lasso.adds {
            self.vector_pick.lines.clear();
        }
        let Some(index) = self.picked_layer() else {
            return;
        };
        if lasso.points.len() < 3 {
            return;
        }
        let size = [self.doc.width, self.doc.height];
        let polygon = &lasso.points;
        let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
        for &[x, y] in polygon {
            (low, high) = (
                [low[0].min(x), low[1].min(y)],
                [high[0].max(x), high[1].max(y)],
            );
        }
        let mut lines = self.picked_lines();
        let mut added = 0;
        for stroke in &self.doc.layers[index].vector.as_ref().unwrap().strokes {
            let path = stroke.path(size);
            let within = !path.is_empty()
                && path
                    .iter()
                    .all(|&[x, y]| x >= low[0] && x <= high[0] && y >= low[1] && y <= high[1])
                && path
                    .iter()
                    .step_by(path.len().div_ceil(500).max(1))
                    .chain(path.last())
                    .all(|&[x, y]| inside(polygon, x, y));
            if within && !lines.iter().any(|l| Arc::ptr_eq(l, stroke)) {
                lines.push(stroke.clone());
                added += 1;
            }
        }
        self.vector_pick.lines = lines;
        if added > 0 {
            let count = self.vector_pick.lines.len();
            self.status = format!("{count} line{} picked", if count == 1 { "" } else { "s" });
        }
    }
    /// Lift the dragged lines: one undo step for the whole move.
    pub(crate) fn finish_line_drag(&mut self) {
        self.vector_pick.lasso = None;
        self.apply_line_drag();
        if let Some(drag) = self.vector_pick.drag.take()
            && drag.checkpointed
        {
            self.history.settle(&self.doc);
            let count = drag.moving.len();
            self.status = format!("Moved {count} line{}", if count == 1 { "" } else { "s" });
        }
    }
    /// Replace each picked line by `change` of it (`None` removes it), in
    /// one undo step.
    fn change_lines(&mut self, change: &dyn Fn(&Stroke) -> Option<Stroke>, done: &str) {
        let Some(index) = self.picked_layer() else {
            return;
        };
        if let Some(blocker) = self.line_edit_blocker(index) {
            self.status = blocker.into();
            return;
        }
        self.finish_layer_rename(true);
        self.close_input_gestures();
        self.flush_render_queue();
        let picked = self.picked_lines();
        if picked.is_empty() {
            return;
        }
        let size = [self.doc.width, self.doc.height];
        let mut rect = None;
        let mut kept = Vec::new();
        let strokes = self.doc.layers[index]
            .vector
            .as_ref()
            .unwrap()
            .strokes
            .iter()
            .filter_map(|s| {
                if !picked.iter().any(|l| Arc::ptr_eq(l, s)) {
                    return Some(s.clone());
                }
                rect = union(rect, s.bounds(size));
                let changed = change(s).map(Arc::new)?;
                rect = union(rect, changed.bounds(size));
                kept.push(changed.clone());
                Some(changed)
            })
            .collect();
        self.history.begin_gesture_checkpoint(&self.doc);
        let layer = &mut self.doc.layers[index];
        let vector = VectorLayer { strokes };
        if let Some(rect) = rect {
            if let Err(error) = vector.redraw(&mut layer.pixels, rect, size, &|| false) {
                self.status = error;
                self.history.discard_pending_checkpoint(&self.doc);
                return;
            }
            self.dirty_region.include(rect);
        }
        layer.vector = Some(vector);
        self.changed();
        self.history.settle(&self.doc);
        self.vector_pick.lines = kept;
        self.status = done.into();
    }
    pub(crate) fn delete_picked_lines(&mut self) {
        let count = self.picked_lines().len();
        self.change_lines(
            &|_| None,
            &format!("Deleted {count} line{}", if count == 1 { "" } else { "s" }),
        );
    }
    pub(crate) fn resize_picked_lines(&mut self, size: f32) {
        self.change_lines(&|s| Some(resized(s, size)), "Resized the picked lines");
    }
    pub(crate) fn smooth_picked_lines(&mut self) {
        self.change_lines(
            &|s| Some(s.smoothed(4.0 + line_size(s))),
            "Smoothed the picked lines",
        );
    }
    pub(crate) fn taper_picked_lines(&mut self, start: Option<f32>, end: Option<f32>) {
        self.change_lines(
            &|s| Some(s.with_taper(start, end)),
            "Tapered the picked lines",
        );
    }
    pub(crate) fn recolor_picked_lines(&mut self) {
        let color = self.color;
        self.change_lines(&|s| Some(s.recolored(color)), "Recolored the picked lines");
    }
    pub(crate) fn deselect_lines(&mut self) {
        self.finish_line_drag();
        self.vector_pick.lines.clear();
    }
    /// Outline the picked lines along their middles, the line under the
    /// pointer, and the lasso being drawn.
    pub(crate) fn paint_vector_pick(&self, painter: &egui::Painter, view: View) {
        if self.tool != crate::tools::Tool::Pointer {
            return;
        }
        let size = [self.doc.width, self.doc.height];
        let accent = Color32::from_rgb(240, 160, 60);
        let outline = |line: &Stroke, width: f32, color: Color32| {
            let path = line.path(size);
            // Long lines are outlined through at most 2000 of their points.
            let step = path.len().div_ceil(2000).max(1);
            let mut points: Vec<Pos2> = path
                .iter()
                .step_by(step)
                .map(|&[x, y]| view.to_screen(egui::pos2(x, y)))
                .collect();
            if let Some(&[x, y]) = path.last()
                && !(path.len() - 1).is_multiple_of(step)
            {
                points.push(view.to_screen(egui::pos2(x, y)));
            }
            if points.len() == 1 {
                painter.circle_filled(points[0], width + 1.5, color);
                return;
            }
            painter.add(Shape::line(
                points.clone(),
                egui::Stroke::new(width + 2.0, Color32::from_black_alpha(110)),
            ));
            painter.add(Shape::line(points, egui::Stroke::new(width, color)));
        };
        let picked = self.picked_lines();
        if self.vector_pick.drag.is_none()
            && self.vector_pick.lasso.is_none()
            && let Some(index) = self.vector_target()
            && let Some(hover) = painter.ctx().input(|i| i.pointer.hover_pos())
            && painter.clip_rect().contains(hover)
            && let Some(line) = self.line_at(index, view.to_doc(hover), REACH / self.zoom)
            && !picked.iter().any(|l| Arc::ptr_eq(l, &line))
        {
            outline(&line, 1.0, accent.gamma_multiply(0.6));
        }
        for line in &picked {
            outline(line, 1.5, accent);
        }
        if let Some(lasso) = &self.vector_pick.lasso {
            let mut points: Vec<Pos2> = lasso
                .points
                .iter()
                .map(|&[x, y]| view.to_screen(egui::pos2(x, y)))
                .collect();
            points.push(points[0]);
            painter.add(Shape::line(
                points.clone(),
                egui::Stroke::new(3.0_f32, Color32::from_black_alpha(110)),
            ));
            painter.add(Shape::line(points, egui::Stroke::new(1.0_f32, accent)));
        }
    }
    /// The Pointer panel's controls for picked lines.
    pub(crate) fn picked_lines_ui(&mut self, ui: &mut egui::Ui) {
        if self.vector_target().is_some() {
            ui.checkbox(&mut self.pointer_lasso, "Lasso lines from empty canvas")
                .on_hover_text(
                    "Dragging empty canvas draws a lasso that picks the lines inside it (Shift adds). Off: it pans. Space or the middle button always pan.",
                );
        }
        let lines = self.picked_lines();
        if lines.is_empty() {
            if self.vector_target().is_some() {
                crate::tool_panel::hint(
                    ui,
                    "Click a line of this vector layer to pick it; Shift+click picks more.",
                );
            }
            return;
        }
        let count = lines.len();
        ui.label(
            egui::RichText::new(format!(
                "{count} line{} picked",
                if count == 1 { "" } else { "s" }
            ))
            .strong(),
        );
        crate::tool_panel::hint(
            ui,
            "Drag a picked line to move them. Shift+click picks more or lets one go. Delete removes them.",
        );
        let first = &lines[0];
        if let Some(size) = committed_value(
            ui,
            "Size",
            line_size(first),
            0.1..=MAX_SIZE,
            "Every picked line takes this size",
        ) {
            self.resize_picked_lines(size);
        }
        let [start, end] = first.taper();
        let taper = crate::vector_layer::MAX_TAPER;
        if let Some(start) = committed_value(
            ui,
            "Taper start",
            start,
            0.0..=taper,
            "How far along each picked line its start narrows",
        ) {
            self.taper_picked_lines(Some(start), None);
        }
        if let Some(end) = committed_value(
            ui,
            "Taper end",
            end,
            0.0..=taper,
            "How far along each picked line its end narrows",
        ) {
            self.taper_picked_lines(None, Some(end));
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Smooth")
                .on_hover_text("Even out wobbles in the picked lines; click again to smooth more")
                .clicked()
            {
                self.smooth_picked_lines();
            }
            if ui
                .button("Recolor")
                .on_hover_text("Give the picked lines the current color")
                .clicked()
            {
                self.recolor_picked_lines();
            }
            if ui.button("Delete").on_hover_text("Delete").clicked() {
                self.delete_picked_lines();
            }
            if ui.button("Deselect").on_hover_text("Esc").clicked() {
                self.deselect_lines();
            }
        });
        ui.separator();
    }
}

/// A labeled pixel field showing `shown`; the new value once a drag or a
/// typed edit is done, so each change is one undo step.
fn committed_value(
    ui: &mut egui::Ui,
    label: &str,
    shown: f32,
    range: std::ops::RangeInclusive<f32>,
    hover: &str,
) -> Option<f32> {
    let id = ui.id().with(("picked lines", label));
    let mut value = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(shown);
    let response = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.add(
                egui::DragValue::new(&mut value)
                    .range(range)
                    .speed(0.2)
                    .max_decimals(1)
                    .suffix(" px"),
            )
        })
        .inner
        .on_hover_text(hover);
    if response.dragged() || response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, value));
        return None;
    }
    ui.data_mut(|d| d.remove::<f32>(id));
    ((response.changed() || response.drag_stopped() || response.lost_focus())
        && (value - shown).abs() > 1e-3)
        .then_some(value)
}
