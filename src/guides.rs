//! App side of `guides`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::guides`, re-exported
//! here so `crate::guides::…` paths keep working.
use crate::{document::Document, egui, symmetry::Symmetry, view::View};
pub(crate) use dudel_core::guides::*;
use egui::{Color32, Pos2, Stroke, pos2};

const GUIDE: Color32 = Color32::from_rgb(64, 200, 190);

/// Grid lines over the visible part of the canvas only, so zooming in on a
/// large canvas never issues thousands of off-screen segments.
pub fn paint_grid(painter: &egui::Painter, view: &View, doc: [usize; 2], grid: Grid) {
    if grid.pixels && view.zoom >= PIXEL_GRID_ZOOM {
        let fade = ((view.zoom - PIXEL_GRID_ZOOM) / (PIXEL_GRID_ZOOM * 0.5)).clamp(0.0, 1.0);
        let alpha = (24.0 + 40.0 * fade).round() as u8;
        paint_lines_every(
            painter,
            view,
            doc,
            1.0,
            Stroke::new(
                1.0_f32,
                Color32::from_rgba_unmultiplied(128, 128, 128, alpha),
            ),
        );
    }
    if !grid.visible {
        return;
    }
    let (w, h) = (doc[0] as f32, doc[1] as f32);
    let major = grid.spacing.max(1) as f32;
    let divisions = grid.subdivisions.max(1) as u32;
    if major * view.zoom < 4.0 {
        return;
    }
    // Skip the minor level while it would be denser than a few screen points.
    let draw_minor = divisions > 1 && major / divisions as f32 * view.zoom >= 6.0;
    let step = if draw_minor {
        major / divisions as f32
    } else {
        major
    };
    let color = |strength: f32| {
        let alpha = (grid.opacity * strength * 255.0).round() as u8;
        Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(90, 120, 200, alpha),
        )
    };
    let (major_stroke, minor_stroke) = (color(1.0), color(0.45));
    let visible = painter.clip_rect();
    let corners = [
        visible.left_top(),
        visible.right_top(),
        visible.right_bottom(),
        visible.left_bottom(),
    ]
    .map(|p| view.to_doc(p));
    let low = |axis: fn(&Pos2) -> f32, limit: f32| {
        corners
            .iter()
            .map(axis)
            .fold(f32::INFINITY, f32::min)
            .clamp(0.0, limit)
    };
    let high = |axis: fn(&Pos2) -> f32, limit: f32| {
        corners
            .iter()
            .map(axis)
            .fold(f32::NEG_INFINITY, f32::max)
            .clamp(0.0, limit)
    };
    let x_range = (low(|p| p.x, w), high(|p| p.x, w));
    let y_range = (low(|p| p.y, h), high(|p| p.y, h));
    let lines = |range: (f32, f32), limit: f32, ends: &dyn Fn(f32) -> [Pos2; 2]| {
        let first = (range.0 / step).floor() as u32;
        let last = ((range.1 / step).ceil() as u32).min((limit / step).floor() as u32);
        for index in first..=last {
            let stroke = if !draw_minor || index.is_multiple_of(divisions) {
                major_stroke
            } else {
                minor_stroke
            };
            painter.line_segment(ends(index as f32 * step).map(|p| view.to_screen(p)), stroke);
        }
    };
    lines(x_range, w, &|x| [pos2(x, y_range.0), pos2(x, y_range.1)]);
    lines(y_range, h, &|y| [pos2(x_range.0, y), pos2(x_range.1, y)]);
}

/// Lines every `step` canvas pixels across the visible part of the canvas.
fn paint_lines_every(
    painter: &egui::Painter,
    view: &View,
    doc: [usize; 2],
    step: f32,
    stroke: Stroke,
) {
    let (w, h) = (doc[0] as f32, doc[1] as f32);
    let visible = painter.clip_rect();
    let corners = [
        visible.left_top(),
        visible.right_top(),
        visible.right_bottom(),
        visible.left_bottom(),
    ]
    .map(|p| view.to_doc(p));
    let range = |axis: fn(&Pos2) -> f32, limit: f32| {
        let (low, high) = corners
            .iter()
            .map(axis)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), v| {
                (low.min(v), high.max(v))
            });
        (low.clamp(0.0, limit), high.clamp(0.0, limit))
    };
    let (x_range, y_range) = (range(|p| p.x, w), range(|p| p.y, h));
    let lines = |range: (f32, f32), limit: f32, ends: &dyn Fn(f32) -> [Pos2; 2]| {
        let first = (range.0 / step).floor() as u32;
        let last = ((range.1 / step).ceil() as u32).min((limit / step).floor() as u32);
        for index in first..=last {
            painter.line_segment(ends(index as f32 * step).map(|p| view.to_screen(p)), stroke);
        }
    };
    lines(x_range, w, &|x| [pos2(x, y_range.0), pos2(x, y_range.1)]);
    lines(y_range, h, &|y| [pos2(x_range.0, y), pos2(x_range.1, y)]);
}

pub fn paint_lines(
    painter: &egui::Painter,
    view: &View,
    doc: [usize; 2],
    lines: &[GuideLine],
    active: Option<usize>,
) {
    let reach = (doc[0] + doc[1]) as f32 * 4.0;
    for (index, line) in lines.iter().enumerate() {
        let points = line_ends(*line, doc, reach).map(|p| view.to_screen(p));
        let width: f32 = if active == Some(index) { 2.0 } else { 1.0 };
        painter.line_segment(
            points,
            Stroke::new(width + 2.0, Color32::from_black_alpha(60)),
        );
        painter.line_segment(points, Stroke::new(width, GUIDE));
    }
}

/// Top and left rulers in canvas pixels. Ticks follow the view while it is
/// unrotated; a rotated view keeps the rulers for dragging out guides.
pub fn paint_rulers(painter: &egui::Painter, area: egui::Rect, view: &View, rotated: bool) {
    let fill = Color32::from_rgb(30, 33, 40);
    let tick = Color32::from_gray(120);
    let text = Color32::from_gray(150);
    let top = egui::Rect::from_min_max(area.min, pos2(area.right(), area.top() + RULER));
    let left = egui::Rect::from_min_max(area.min, pos2(area.left() + RULER, area.bottom()));
    painter.rect_filled(top, 0.0, fill);
    painter.rect_filled(left, 0.0, fill);
    painter.line_segment(
        [top.left_bottom(), top.right_bottom()],
        Stroke::new(1.0_f32, tick),
    );
    painter.line_segment(
        [left.right_top(), left.right_bottom()],
        Stroke::new(1.0_f32, tick),
    );
    if rotated {
        return;
    }
    let step = tick_step(view.zoom);
    let font = egui::FontId::proportional(10.0);
    // Visible canvas range along each ruler, whatever the mirroring.
    let x0 = view.to_doc(pos2(area.left() + RULER, area.top())).x;
    let x1 = view.to_doc(pos2(area.right(), area.top())).x;
    let y0 = view.to_doc(pos2(area.left(), area.top() + RULER)).y;
    let y1 = view.to_doc(pos2(area.left(), area.bottom())).y;
    let ticks = |from: f32, to: f32| {
        let (low, high) = (from.min(to), from.max(to));
        let first = (low / step).floor() as i64;
        let last = (high / step).ceil() as i64;
        (first..=last).map(move |k| k as f32 * step)
    };
    for x in ticks(x0, x1) {
        let sx = view.to_screen(pos2(x, 0.0)).x;
        if sx < area.left() + RULER || sx > area.right() {
            continue;
        }
        painter.line_segment(
            [pos2(sx, top.bottom() - 6.0), pos2(sx, top.bottom())],
            Stroke::new(1.0_f32, tick),
        );
        painter.text(
            pos2(sx + 3.0, top.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{x:.0}"),
            font.clone(),
            text,
        );
    }
    for y in ticks(y0, y1) {
        let sy = view.to_screen(pos2(0.0, y)).y;
        if sy < area.top() + RULER || sy > area.bottom() {
            continue;
        }
        painter.line_segment(
            [pos2(left.right() - 6.0, sy), pos2(left.right(), sy)],
            Stroke::new(1.0_f32, tick),
        );
        painter.text(
            pos2(left.left() + 2.0, sy + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{y:.0}"),
            font.clone(),
            text,
        );
    }
    painter.rect_filled(
        egui::Rect::from_min_size(area.min, egui::vec2(RULER, RULER)),
        0.0,
        fill,
    );
}

impl crate::Dudel {
    /// The saved form of the live guides, with per-layer symmetry keyed by the
    /// layer's stack index in `doc` (layer identities are not persisted).
    pub(crate) fn guides_record(&self, doc: &Document) -> Guides {
        let mut layer_symmetry: Vec<(usize, Symmetry)> = doc
            .layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| {
                self.layer_symmetry
                    .get(&layer.id)
                    .map(|symmetry| (index, *symmetry))
            })
            .collect();
        layer_symmetry.sort_by_key(|(index, _)| *index);
        // Rulers saved with their owner's stack index; a ruler whose layer
        // was deleted goes with it.
        let (special_rulers, ruler_layers): (Vec<_>, Vec<_>) = self
            .rulers
            .iter()
            .enumerate()
            .filter_map(|(index, ruler)| match self.ruler_owner(index) {
                None => Some((*ruler, None)),
                Some(id) => doc
                    .layers
                    .iter()
                    .position(|layer| layer.id == id)
                    .map(|layer| (*ruler, Some(layer))),
            })
            .unzip();
        Guides {
            symmetry: self.symmetry,
            layer_symmetry,
            grid: self.grid,
            lines: self.guide_lines.clone(),
            locked: self.guides_locked,
            rulers: self.show_rulers,
            special_rulers,
            ruler_layers,
            snapping: self.snapping,
            snap_grid: self.snap_grid,
        }
    }
    pub(crate) fn ruler_owner(&self, index: usize) -> Option<crate::document::LayerId> {
        self.ruler_owners.get(index).copied().flatten()
    }
    /// Whether ruler `index` shows and snaps now: it belongs to every layer,
    /// or to the layer being drawn on.
    pub(crate) fn ruler_active(&self, index: usize) -> bool {
        match self.ruler_owner(index) {
            None => true,
            Some(id) => self
                .input_layer()
                .and_then(|layer| self.doc.layers.get(layer))
                .is_some_and(|layer| layer.id == id),
        }
    }
    /// Indices and copies of the rulers active on the current layer.
    pub(crate) fn active_rulers(&self) -> (Vec<usize>, Vec<crate::rulers::Ruler>) {
        (0..self.rulers.len())
            .filter(|index| self.ruler_active(*index))
            .map(|index| (index, self.rulers[index]))
            .unzip()
    }
    pub(crate) fn push_ruler(&mut self, ruler: crate::rulers::Ruler) {
        self.ruler_owners.resize(self.rulers.len(), None);
        self.rulers.push(ruler);
        self.ruler_owners.push(None);
    }
    pub(crate) fn remove_ruler(&mut self, index: usize) {
        if index < self.rulers.len() {
            self.rulers.remove(index);
        }
        if index < self.ruler_owners.len() {
            self.ruler_owners.remove(index);
        }
        self.ruler_drag = None;
    }
    /// Whether rulers differ from `saved`, without allocating.
    fn rulers_changed(&self, saved: &Guides) -> bool {
        let mut kept = 0;
        for (index, ruler) in self.rulers.iter().enumerate() {
            let owner = match self.ruler_owner(index) {
                None => None,
                Some(id) => match self.doc.layers.iter().position(|layer| layer.id == id) {
                    Some(layer) => Some(layer),
                    None => continue,
                },
            };
            if saved.special_rulers.get(kept) != Some(ruler)
                || saved.ruler_layers.get(kept).copied().flatten() != owner
            {
                return true;
            }
            kept += 1;
        }
        kept != saved.special_rulers.len()
    }
    /// Take guides from a newly installed document; None resets them.
    pub(crate) fn apply_guides(&mut self, guides: Option<Box<Guides>>) {
        let mut guides = guides.map(|guides| *guides).unwrap_or_default();
        guides.sanitize(&self.doc);
        self.symmetry = guides.symmetry;
        self.layer_symmetry = guides
            .layer_symmetry
            .iter()
            .map(|(index, symmetry)| (self.doc.layers[*index].id, *symmetry))
            .collect();
        self.grid = guides.grid;
        self.guide_lines = guides.lines.clone();
        self.guides_locked = guides.locked;
        self.show_rulers = guides.rulers;
        self.rulers = guides.special_rulers.clone();
        self.ruler_owners = guides
            .ruler_layers
            .iter()
            .map(|owner| owner.map(|index| self.doc.layers[index].id))
            .collect();
        self.snapping = guides.snapping;
        self.snap_grid = guides.snap_grid;
        self.ruler_drag = None;
        self.ruler_create = None;
        self.guide_drag = None;
        self.symmetry_drag = false;
        self.observed_guides = guides.clone();
        self.saved_guides = guides;
    }
    /// Advance the persistence revision once per settled guide change without
    /// adding guide state to artwork history or invalidating display pixels.
    pub(crate) fn track_guide_changes(&mut self) {
        if self.guide_drag.is_none()
            && !self.symmetry_drag
            && self.guides_differ(&self.observed_guides)
        {
            self.observed_guides = self.guides_record(&self.doc);
            self.metadata_changed();
        }
    }
    /// Whether guides or reference layers, the app-owned state saved beside
    /// the artwork, differ from the last save or load, without allocating.
    pub(crate) fn guides_changed(&self) -> bool {
        self.guides_differ(&self.saved_guides) || self.references_changed()
    }
    fn guides_differ(&self, saved: &Guides) -> bool {
        let live_layer_symmetry = self
            .doc
            .layers
            .iter()
            .filter(|layer| self.layer_symmetry.contains_key(&layer.id))
            .count();
        if saved.symmetry != self.symmetry
            || saved.grid != self.grid
            || saved.lines != self.guide_lines
            || saved.locked != self.guides_locked
            || saved.rulers != self.show_rulers
            || self.rulers_changed(saved)
            || saved.snapping != self.snapping
            || saved.snap_grid != self.snap_grid
            || saved.layer_symmetry.len() != live_layer_symmetry
        {
            return true;
        }
        saved.layer_symmetry.iter().any(|(index, symmetry)| {
            self.doc
                .layers
                .get(*index)
                .and_then(|layer| self.layer_symmetry.get(&layer.id))
                != Some(symmetry)
        })
    }
    pub(crate) fn sync_canvas_frame(&mut self) {
        let before = self.observed_canvas_frame;
        let after = *self.doc.canvas_frame;
        if before == after {
            return;
        }
        map_symmetry(&mut self.symmetry, before, after);
        for symmetry in self.layer_symmetry.values_mut() {
            map_symmetry(symmetry, before, after);
        }
        for line in &mut self.guide_lines {
            map_line(line, before, after);
        }
        for ruler in &mut self.rulers {
            ruler.map_canvas(before, after);
        }
        self.map_references(before, after);
        self.observed_canvas_frame = after;
        self.last_stroke_end = None;
        self.auto_fit = true;
        self.dirty = true;
        self.dirty_region = Default::default();
    }
    /// Guides move with the Pointer tool, or with any tool once unlocked.
    pub(crate) fn guides_editable(&self) -> bool {
        !self.guides_locked || self.tool == crate::tools::Tool::Pointer
    }
    /// Pull a guide line out of a ruler, or move an unlocked one. Returns true
    /// when the packet belongs to a guide drag, which never paints.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn guide_packet(
        &mut self,
        area: egui::Rect,
        view: &View,
        pos: Pos2,
        at: Pos2,
        start: bool,
        end: bool,
        navigation_allowed: bool,
    ) -> bool {
        if self.ruler_packet(view, pos, at, start, end, navigation_allowed) {
            return true;
        }
        if start && navigation_allowed && self.guide_drag.is_none() {
            let doc = [self.doc.width, self.doc.height];
            if let Some(vertical) = self.show_rulers.then(|| ruler_at(area, pos)).flatten() {
                if self.guide_lines.len() < MAX_LINES {
                    self.guide_lines.push(GuideLine {
                        vertical,
                        position: 0.0,
                    });
                    self.guide_drag = Some(GuideDrag {
                        index: self.guide_lines.len() - 1,
                        original: None,
                    });
                } else {
                    self.status = format!("At most {MAX_LINES} guide lines");
                    return true;
                }
            } else if self.guides_editable()
                && area.contains(pos)
                && let Some(index) = line_at(&self.guide_lines, doc, view, pos)
            {
                self.guide_drag = Some(GuideDrag {
                    index,
                    original: Some(self.guide_lines[index].position),
                });
            }
        }
        let Some(drag) = self.guide_drag else {
            return false;
        };
        let Some(line) = self.guide_lines.get_mut(drag.index) else {
            self.guide_drag = None;
            return false;
        };
        let limit = if line.vertical {
            self.doc.width
        } else {
            self.doc.height
        } as f32;
        let value = if line.vertical { at.x } else { at.y };
        line.position = value.round().clamp(0.0, limit);
        if end {
            // Dropping a guide off the canvas or back on a ruler removes it.
            let removed = !(0.0..=limit).contains(&value)
                || !area.contains(pos)
                || (self.show_rulers && ruler_at(area, pos).is_some());
            if removed {
                self.guide_lines.remove(drag.index);
                self.status = "Guide line removed".into();
            } else {
                self.status = "Guide line placed".into();
            }
            self.guide_drag = None;
        }
        true
    }
    /// Create a ruler with the Pointer tool, or drag a ruler handle. Returns
    /// true while the packet belongs to a ruler gesture, which never paints.
    fn ruler_packet(
        &mut self,
        view: &View,
        pos: Pos2,
        at: Pos2,
        start: bool,
        end: bool,
        navigation_allowed: bool,
    ) -> bool {
        use crate::rulers::{MAX_RULERS, Ruler, RulerDrag, handle_at};
        let point = [at.x, at.y];
        if start && navigation_allowed && self.ruler_drag.is_none() {
            if let Some(kind) = self
                .ruler_create
                .filter(|_| self.tool == crate::tools::Tool::Pointer)
            {
                if self.rulers.len() >= MAX_RULERS {
                    self.status = format!("At most {MAX_RULERS} rulers");
                    return true;
                }
                let ruler = Ruler::new(kind, point);
                self.push_ruler(ruler);
                self.ruler_drag = Some(RulerDrag {
                    index: self.rulers.len() - 1,
                    handle: ruler.handles().len() - 1,
                    from: Some(point),
                    original: ruler,
                });
            } else if self.guides_editable()
                && let Some((index, handle)) = {
                    let (indices, active) = self.active_rulers();
                    handle_at(&active, view, pos).map(|(at, handle)| (indices[at], handle))
                }
            {
                self.ruler_drag = Some(RulerDrag {
                    index,
                    handle,
                    from: None,
                    original: self.rulers[index],
                });
            }
        }
        let Some(drag) = self.ruler_drag else {
            return false;
        };
        let Some(ruler) = self.rulers.get_mut(drag.index) else {
            self.ruler_drag = None;
            return false;
        };
        match drag.from {
            Some(from) => ruler.drag_out(from, point),
            None => ruler.set_handle(drag.handle, point),
        }
        if end {
            if drag.from.is_some() {
                self.ruler_create = None;
                if ruler.usable() {
                    self.status = format!("{} ruler added", ruler.kind().name());
                } else {
                    self.remove_ruler(drag.index);
                    self.status = "Drag farther to size the ruler".into();
                }
            } else {
                self.status = "Ruler moved".into();
            }
            self.ruler_drag = None;
        }
        true
    }
    /// Undo an unfinished ruler drag (pen canceled or focus lost).
    pub(crate) fn cancel_ruler_drag(&mut self) {
        if let Some(drag) = self.ruler_drag.take()
            && drag.index < self.rulers.len()
        {
            if drag.from.is_some() {
                self.remove_ruler(drag.index);
            } else {
                self.rulers[drag.index] = drag.original;
            }
        }
    }
    /// Resize cursor over a ruler or an unlocked guide line.
    pub(crate) fn guide_cursor(
        &self,
        area: egui::Rect,
        view: &View,
        pos: Pos2,
    ) -> Option<egui::CursorIcon> {
        if self.ruler_drag.is_some() {
            return Some(egui::CursorIcon::Grabbing);
        }
        if self.tool == crate::tools::Tool::Pointer && self.ruler_create.is_some() {
            return Some(egui::CursorIcon::Crosshair);
        }
        if self.guides_editable()
            && crate::rulers::handle_at(&self.active_rulers().1, view, pos).is_some()
        {
            return Some(egui::CursorIcon::Grab);
        }
        let vertical = if let Some(drag) = self.guide_drag {
            self.guide_lines.get(drag.index)?.vertical
        } else if let Some(vertical) = self.show_rulers.then(|| ruler_at(area, pos)).flatten() {
            vertical
        } else if self.guides_editable() {
            let doc = [self.doc.width, self.doc.height];
            self.guide_lines[line_at(&self.guide_lines, doc, view, pos)?].vertical
        } else {
            return None;
        };
        Some(if vertical {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        })
    }
    /// Grid, ruler and guide-line controls for the View menu.
    pub(crate) fn guides_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.grid.visible, "Show grid");
        ui.add_enabled_ui(self.grid.visible, |ui| {
            ui.horizontal(|ui| {
                ui.label("Spacing");
                ui.add(
                    egui::DragValue::new(&mut self.grid.spacing)
                        .range(4..=4096)
                        .suffix(" px"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Divisions");
                ui.add(egui::DragValue::new(&mut self.grid.subdivisions).range(1..=16));
            });
            ui.horizontal(|ui| {
                ui.label("Opacity");
                ui.add(egui::Slider::new(&mut self.grid.opacity, 0.05..=1.0));
            });
        });
        ui.checkbox(&mut self.grid.pixels, "Pixel grid when zoomed in")
            .on_hover_text("Outlines every pixel at 800% zoom and closer, for pixel art");
        ui.checkbox(&mut self.symmetry.wrap, "Wrap around edges (seamless tiles)")
            .on_hover_text(
                "Strokes that cross an edge of the canvas continue on the opposite side,                  and copies of the canvas show around it, for textures and tiles that repeat",
            );
        ui.separator();
        ui.checkbox(&mut self.show_rulers, "Show rulers")
            .on_hover_text("Drag from a ruler onto the canvas to add a guide line");
        let center = self
            .view(self.canvas_area)
            .to_doc(self.canvas_area.center());
        let add = |vertical: bool, position: f32, limit: usize| GuideLine {
            vertical,
            position: position.round().clamp(0.0, limit as f32),
        };
        ui.horizontal(|ui| {
            let full = self.guide_lines.len() >= MAX_LINES;
            if ui
                .add_enabled(!full, egui::Button::new("Add horizontal"))
                .clicked()
            {
                self.guide_lines.push(add(false, center.y, self.doc.height));
            }
            if ui
                .add_enabled(!full, egui::Button::new("Add vertical"))
                .clicked()
            {
                self.guide_lines.push(add(true, center.x, self.doc.width));
            }
        });
        if ui
            .add_enabled(
                !self.guide_lines.is_empty(),
                egui::Button::new("Clear guide lines"),
            )
            .clicked()
        {
            self.guide_lines.clear();
            self.guide_drag = None;
        }
        ui.separator();
        ui.checkbox(&mut self.snapping, "Snap strokes to rulers and guides")
            .on_hover_text(
                "Brush and eraser strokes follow special rulers, and guide lines they start near",
            );
        ui.add_enabled_ui(self.snapping, |ui| {
            ui.checkbox(&mut self.snap_grid, "Snap to grid lines")
                .on_hover_text("Strokes starting near a visible grid line follow it");
        });
        ui.checkbox(&mut self.guides_locked, "Lock guides")
            .on_hover_text("Locked guide lines and the symmetry center cannot be dragged");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, egui, symmetry::Symmetry, view::View};
    use egui::pos2;
    #[test]
    fn settled_guide_revision_ignores_orphaned_layer_settings_and_repeated_polling() {
        let mut app = crate::Dudel::default();
        let orphan = {
            app.doc.add_layer();
            app.doc.layers.pop().unwrap().id
        };
        app.doc.active = 0;
        app.layer_symmetry.insert(orphan, Symmetry::default());
        app.grid.visible = true;
        let revision = app.revision;
        app.track_guide_changes();
        assert_ne!(app.revision, revision);
        let settled = app.revision;
        for _ in 0..20 {
            app.track_guide_changes();
        }
        assert_eq!(app.revision, settled);
        assert!(app.modified && !app.history.can_undo());
        let mut fresh = crate::Dudel::default();
        fresh.grid.visible = true;
        assert!(!fresh.request_replace(crate::document_actions::Action::New(17, 19, 300.0)));
        assert!(
            fresh.unsaved_action.is_some(),
            "guide-only edits prompt before a canvas replacement even without a frame update"
        );
    }
    #[test]
    fn sanitize_repairs_untrusted_guides() {
        let mut doc = Document::new(100, 50);
        doc.insert_layer_above().unwrap();
        let mut guides = Guides {
            symmetry: Symmetry {
                angle: f32::NAN,
                segments: 99,
                center: Some([500.0, f32::INFINITY]),
                ..Symmetry::default()
            },
            layer_symmetry: vec![
                (1, Symmetry::default()),
                (7, Symmetry::default()),
                (1, Symmetry::default()),
            ],
            grid: Grid {
                spacing: 0,
                opacity: f32::NAN,
                ..Grid::default()
            },
            special_rulers: vec![
                crate::rulers::Ruler::Focus {
                    center: [f32::NAN, 1.0],
                },
                crate::rulers::Ruler::Line {
                    a: [0.0, 0.0],
                    b: [1.0e10, 0.0],
                },
                crate::rulers::Ruler::Focus {
                    center: [-200.0, 20.0],
                },
            ],
            lines: vec![
                GuideLine {
                    vertical: true,
                    position: 40.0,
                },
                GuideLine {
                    vertical: false,
                    position: 80.0,
                },
                GuideLine {
                    vertical: true,
                    position: f32::NAN,
                },
            ],
            ..Guides::default()
        };
        guides.sanitize(&doc);
        assert_eq!(guides.symmetry.angle, 0.0);
        assert_eq!(guides.symmetry.segments, 16);
        assert_eq!(guides.symmetry.center, None);
        assert_eq!(guides.layer_symmetry.len(), 1);
        assert_eq!(guides.grid.spacing, 4);
        assert_eq!(guides.grid.opacity, Grid::default().opacity);
        assert_eq!(
            guides.special_rulers,
            vec![crate::rulers::Ruler::Focus {
                center: [-200.0, 20.0]
            }]
        );
        assert_eq!(
            guides.lines,
            vec![
                GuideLine {
                    vertical: true,
                    position: 40.0
                },
                GuideLine {
                    vertical: false,
                    position: 80.0
                }
            ]
        );
    }
    #[test]
    fn ticks_stay_readable_and_guides_pick_by_screen_distance() {
        assert_eq!(tick_step(1.0), 100.0);
        assert_eq!(tick_step(0.1), 1000.0);
        assert_eq!(tick_step(8.0), 10.0);
        let view = View {
            origin: pos2(200.0, 100.0),
            zoom: 2.0,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(50.0, 25.0),
        };
        let lines = [
            GuideLine {
                vertical: true,
                position: 10.0,
            },
            GuideLine {
                vertical: false,
                position: 20.0,
            },
        ];
        let x = view.to_screen(pos2(10.0, 0.0)).x;
        assert_eq!(
            line_at(&lines, [100, 50], &view, pos2(x + 4.0, 300.0)),
            Some(0)
        );
        assert_eq!(
            line_at(&lines, [100, 50], &view, pos2(x + 9.0, 300.0)),
            None
        );
        let y = view.to_screen(pos2(0.0, 20.0)).y;
        assert_eq!(
            line_at(&lines, [100, 50], &view, pos2(-500.0, y - 3.0)),
            Some(1)
        );
    }
    #[test]
    fn folder_symmetry_survives_project_reload_and_reorder() {
        use crate::symmetry::Mode;
        let mut app = crate::Dudel {
            doc: Document::new(32, 32),
            ..Default::default()
        };
        app.doc.group_active().unwrap();
        app.doc.group_active().unwrap();
        let own = [Mode::LeftRight, Mode::FourWay, Mode::Radial].map(|mode| Symmetry {
            mode,
            angle: 35.0,
            ..Default::default()
        });
        app.layer_symmetry
            .extend(app.doc.layers.iter().map(|layer| layer.id).zip(own));
        app.duplicate_layer();
        app.reorder_canvas_layer(5, 0, 0);
        let expected = app.guides_record(&app.doc);
        assert_eq!(expected.layer_symmetry.len(), 6);
        let mut saved = app.doc.clone();
        saved.guides = Some(Box::new(expected.clone()));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested-guides.dudel");
        crate::storage::save_project(&path, &saved).unwrap();
        let loaded = crate::storage::load_project(&path).unwrap();
        let guides = loaded.guides.clone();
        app.doc = loaded;
        app.apply_guides(guides);
        assert_eq!(app.guides_record(&app.doc), expected);
        assert!(!app.guides_changed());
        for (index, symmetry) in expected.layer_symmetry {
            assert_eq!(app.symmetry_for(Some(index)), symmetry);
        }
    }

    #[test]
    fn guides_round_trip_through_a_saved_project() {
        use crate::{document_actions::Action, symmetry::Mode};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("guides.dudel");
        let mut app = crate::Dudel {
            doc: Document::new(200, 100),
            ..Default::default()
        };
        app.doc.insert_layer_above().unwrap();
        assert!(!app.guides_changed());
        app.symmetry = Symmetry {
            mode: Mode::FourWay,
            angle: 30.0,
            center: Some([50.0, 40.0]),
            ..Symmetry::default()
        };
        let top = app.doc.active;
        app.layer_symmetry.insert(
            app.doc.layers[top].id,
            Symmetry {
                mode: Mode::Radial,
                segments: 8,
                ..Symmetry::default()
            },
        );
        app.grid = Grid {
            visible: true,
            spacing: 32,
            ..Grid::default()
        };
        app.guide_lines = vec![GuideLine {
            vertical: true,
            position: 60.0,
        }];
        app.guides_locked = false;
        app.show_rulers = true;
        app.rulers = vec![
            crate::rulers::Ruler::Ellipse {
                center: [80.0, 50.0],
                radii: [40.0, 15.0],
                angle: 20.0,
            },
            // Focus points may sit beyond the canvas.
            crate::rulers::Ruler::Focus {
                center: [-150.0, 30.0],
            },
        ];
        app.snapping = false;
        app.snap_grid = true;
        assert!(app.guides_changed());
        let expected = app.guides_record(&app.doc);
        assert_eq!(expected.layer_symmetry.len(), 1);
        assert_eq!(expected.layer_symmetry[0].0, top);
        let mut saved = app.doc.clone();
        saved.guides = Some(Box::new(expected.clone()));
        crate::storage::save_project(&path, &saved).unwrap();

        let mut other = crate::Dudel::default();
        other.request_replace(Action::Replace {
            document: Box::new(crate::storage::load_project(&path).unwrap()),
            path: Some(path.clone()),
            modified: false,
            status: "Project opened",
        });
        assert!(
            other.doc.guides.is_none(),
            "the live document never holds guides"
        );
        assert_eq!(other.guides_record(&other.doc), expected);
        assert!(!other.guides_changed());
        assert_eq!(other.symmetry_for(Some(top)).mode, Mode::Radial);
        assert_eq!(other.symmetry_for(Some(1 - top)).mode, Mode::FourWay);

        // Files written before guides existed open with default guides.
        crate::storage::save_project(&path, &app.doc).unwrap();
        other.request_replace(Action::Replace {
            document: Box::new(crate::storage::load_project(&path).unwrap()),
            path: Some(path),
            modified: false,
            status: "Project opened",
        });
        assert_eq!(other.guides_record(&other.doc), Guides::default());
    }
    #[test]
    fn guide_lines_follow_a_quarter_turn() {
        let mut app = crate::Dudel {
            doc: Document::new(200, 100),
            ..Default::default()
        };
        app.guide_lines = vec![
            GuideLine {
                vertical: true,
                position: 30.0,
            },
            GuideLine {
                vertical: false,
                position: 20.0,
            },
        ];
        app.doc.canvas_frame = app.doc.canvas_frame.rotated(100).unwrap().into();
        app.sync_canvas_frame();
        assert_eq!(
            app.guide_lines,
            vec![
                GuideLine {
                    vertical: false,
                    position: 30.0
                },
                GuideLine {
                    vertical: true,
                    position: 80.0
                },
            ]
        );
    }
    #[test]
    fn locked_guides_move_only_with_the_pointer_tool() {
        use crate::tools::Tool;
        let mut app = crate::Dudel {
            doc: Document::new(200, 100),
            ..Default::default()
        };
        app.guide_lines = vec![GuideLine {
            vertical: true,
            position: 50.0,
        }];
        assert!(app.guides_locked);
        let area = egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 200.0));
        let view = View {
            origin: area.center(),
            zoom: 1.0,
            rotation: 0.0,
            mirror: false,
            half: egui::vec2(100.0, 50.0),
        };
        let screen = |x: f32| view.to_screen(pos2(x, 40.0));
        app.tool = Tool::Brush;
        assert!(!app.guide_packet(
            area,
            &view,
            screen(50.0),
            pos2(50.0, 40.0),
            true,
            false,
            true
        ));
        assert!(app.guide_drag.is_none());
        app.tool = Tool::Pointer;
        assert!(app.guide_packet(
            area,
            &view,
            screen(50.0),
            pos2(50.0, 40.0),
            true,
            false,
            true
        ));
        assert!(app.guide_packet(
            area,
            &view,
            screen(80.0),
            pos2(80.0, 40.0),
            false,
            true,
            true
        ));
        assert_eq!(
            app.guide_lines,
            vec![GuideLine {
                vertical: true,
                position: 80.0
            }]
        );
        assert!(app.guide_drag.is_none());
    }
    #[test]
    fn layer_rulers_apply_only_on_their_layer_and_leave_with_it() {
        use crate::rulers::Ruler;
        let mut app = crate::Dudel {
            doc: Document::new(200, 100),
            ..Default::default()
        };
        app.doc.insert_layer_above().unwrap();
        let top = app.doc.active;
        let bottom = 1 - top;
        app.push_ruler(Ruler::Focus {
            center: [10.0, 10.0],
        });
        app.push_ruler(Ruler::Parallel {
            a: [0.0, 0.0],
            b: [10.0, 0.0],
        });
        app.ruler_owners[0] = Some(app.doc.layers[top].id);
        assert_eq!(app.active_rulers().0, vec![0, 1]);
        app.doc.active = bottom;
        assert_eq!(app.active_rulers().0, vec![1]);
        // Saved by stack index and restored to the same layer.
        let record = app.guides_record(&app.doc);
        assert_eq!(record.ruler_layers, vec![Some(top), None]);
        let mut other = crate::Dudel {
            doc: app.doc.clone(),
            ..Default::default()
        };
        other.apply_guides(Some(Box::new(record)));
        assert_eq!(other.ruler_owner(0), Some(other.doc.layers[top].id));
        assert!(!other.guides_changed());
        // A ruler whose layer is deleted is no longer saved.
        let id = app.doc.layers[top].id;
        app.doc.layers.retain(|layer| layer.id != id);
        app.doc.active = 0;
        let record = app.guides_record(&app.doc);
        assert_eq!(record.special_rulers.len(), 1);
        assert_eq!(record.ruler_layers, vec![None]);
        app.remove_ruler(0);
        assert_eq!((app.rulers.len(), app.ruler_owners.len()), (1, 1));
    }
}
