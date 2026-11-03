//! Single lines on a vector layer, handled with the Pointer tool (O): a
//! press on a line picks it (Shift adds or removes it), dragging moves the
//! picked lines, and the tool panel resizes, recolors or deletes them.
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
    /// Whether a Pointer press at `pos` lands on a line of the active
    /// vector layer, so the Pointer does not also pan.
    pub(crate) fn line_grabs(&self, view: View, pos: Pos2) -> bool {
        self.vector_pick.drag.is_some()
            || self
                .vector_target()
                .and_then(|index| self.line_at(index, view.to_doc(pos), REACH / self.zoom))
                .is_some()
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
    /// Lift the dragged lines: one undo step for the whole move.
    pub(crate) fn finish_line_drag(&mut self) {
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
    pub(crate) fn recolor_picked_lines(&mut self) {
        let color = self.color;
        self.change_lines(&|s| Some(s.recolored(color)), "Recolored the picked lines");
    }
    pub(crate) fn deselect_lines(&mut self) {
        self.finish_line_drag();
        self.vector_pick.lines.clear();
    }
    /// Outline the picked lines along their middles.
    pub(crate) fn paint_vector_pick(&self, painter: &egui::Painter, view: View) {
        if self.tool != crate::tools::Tool::Pointer {
            return;
        }
        let size = [self.doc.width, self.doc.height];
        let accent = Color32::from_rgb(240, 160, 60);
        for line in self.picked_lines() {
            let path = line.path(size);
            // Long lines are outlined through at most 2000 of their points.
            let step = path.len().div_ceil(2000).max(1);
            let mut points: Vec<Pos2> = path
                .iter()
                .step_by(step)
                .map(|&[x, y]| view.to_screen(egui::pos2(x, y)))
                .collect();
            if let Some(&[x, y]) = path.last()
                && (path.len() - 1) % step != 0
            {
                points.push(view.to_screen(egui::pos2(x, y)));
            }
            if points.len() == 1 {
                painter.circle_filled(points[0], 3.0, accent);
                continue;
            }
            painter.add(Shape::line(
                points.clone(),
                egui::Stroke::new(3.5_f32, Color32::from_black_alpha(110)),
            ));
            painter.add(Shape::line(points, egui::Stroke::new(1.5_f32, accent)));
        }
    }
    /// The Pointer panel's controls for picked lines.
    pub(crate) fn picked_lines_ui(&mut self, ui: &mut egui::Ui) {
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
        let id = ui.id().with("picked line size");
        let shown = line_size(&lines[0]);
        let mut size = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(shown);
        let response = ui
            .horizontal(|ui| {
                ui.label("Size");
                ui.add(
                    egui::DragValue::new(&mut size)
                        .range(0.1..=MAX_SIZE)
                        .speed(0.2)
                        .max_decimals(1)
                        .suffix(" px"),
                )
            })
            .inner
            .on_hover_text("Every picked line takes this size");
        // The size applies when a drag or typed value is done, as one step.
        if response.dragged() || response.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, size));
        } else {
            ui.data_mut(|d| d.remove::<f32>(id));
            if (response.changed() || response.drag_stopped() || response.lost_focus())
                && (size - shown).abs() > 1e-3
            {
                self.resize_picked_lines(size);
            }
        }
        ui.horizontal_wrapped(|ui| {
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
