use crate::{Dudel, egui, input, stroke, tools::Tool, view};
use egui::{Color32, CursorIcon, Pos2, Stroke, pos2, vec2};

/// An in-progress Move tool drag. Pixels are recomputed from the untouched
/// source by a coalescing worker, so the result never accumulates resampling error.
pub struct MoveDrag {
    start: Pos2,
    pub(crate) engine: crate::move_preview::Engine,
    checkpointed: bool,
    modified_before: bool,
    offset: [i64; 2],
}
/// Pointer intent without an artwork snapshot while prior commands settle.
pub(crate) struct PendingMove {
    start: Pos2,
    offset: [i64; 2],
    target: crate::document::LayerId,
    paint_target: crate::paint_target::Target,
    released: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Nav {
    Pan,
    Rotate,
    Zoom,
}

impl Dudel {
    #[cfg(test)]
    fn test_cancel_blank_sparse_stroke() {
        let mut app = Dudel {
            doc: crate::document::Document::new(517, 259),
            ..Dudel::default()
        };
        app.history.begin_stroke_checkpoint(&app.doc);
        app.stroking = true;
        app.stroke_buffer = Some(Default::default());
        app.doc.dab_buffered(
            255.5,
            255.5,
            1.0,
            &app.brush,
            [100, 80, 60],
            false,
            crate::symmetry::IDENTITY,
            app.stroke_buffer.as_mut(),
        );
        assert!(app.doc.layers[0].pixels.capacity() > 0);
        app.cancel_painted_stroke();
        assert_eq!(app.doc.layers[0].pixels.capacity(), 0);
        assert!(!app.history.has_pending_stroke());
        assert!(!app.history.can_undo());
        assert_eq!(app.history.retained_bytes(), 0);
    }
    #[cfg(test)]
    fn test_cancel_buffered_stroke() {
        let mut app = Dudel {
            doc: crate::document::Document::new(37, 65),
            ..Dudel::default()
        };
        app.history.checkpoint(&app.doc);
        app.doc.dab(8.0, 8.0, 1.0, &app.brush, [10, 20, 30], false);
        app.history.settle(&app.doc);
        let before = app.doc.layers[0].pixels.clone();
        app.history.begin_stroke_checkpoint(&app.doc);
        app.stroking = true;
        app.stroke_buffer = Some(Default::default());
        for (x, y) in [(31.5, 31.5), (35.5, 63.5)] {
            app.doc.dab_buffered(
                x,
                y,
                1.0,
                &app.brush,
                [100, 80, 60],
                false,
                crate::symmetry::IDENTITY,
                app.stroke_buffer.as_mut(),
            );
        }
        assert_ne!(app.doc.layers[0].pixels, before);
        app.cancel_painted_stroke();
        assert_eq!(app.doc.layers[0].pixels, before);
        assert!(!app.stroking);
        assert!(app.stroke_buffer.is_none());
        assert!(app.dirty);
        assert!(app.history.undo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|value| *value == 0));
    }
    pub(crate) fn view(&self, area: egui::Rect) -> view::View {
        view::View {
            origin: area.center() + self.pan,
            zoom: self.zoom,
            rotation: self.rotation,
            mirror: self.mirror,
            half: vec2(self.doc.width as f32, self.doc.height as f32) / 2.0,
        }
    }
    pub(crate) fn fit_view(&mut self) {
        self.zoom = 0.0;
        self.pan = egui::Vec2::ZERO;
    }
    pub(crate) fn actual_size(&mut self) {
        self.auto_fit = false;
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
    }
    /// Zoom about the viewport center by a factor.
    pub(crate) fn step_zoom(&mut self, factor: f32) {
        if self.zoom <= 0.0 {
            return;
        }
        let next = (self.zoom * factor).clamp(view::MIN_ZOOM, view::MAX_ZOOM);
        self.pan *= next / self.zoom;
        self.zoom = next;
        self.auto_fit = false;
    }
    fn zoom_about(&mut self, viewport_center: Pos2, anchor: Pos2, next: f32) {
        let next = next.clamp(view::MIN_ZOOM, view::MAX_ZOOM);
        self.auto_fit = false;
        let a = anchor - viewport_center;
        let pan = stroke::zoom_pan([self.pan.x, self.pan.y], [a.x, a.y], next / self.zoom);
        self.pan = vec2(pan[0], pan[1]);
        self.zoom = next;
    }
    /// Rotate the view about the viewport center.
    pub(crate) fn rotate_view(&mut self, delta: f32) {
        self.rotation = view::normalize(self.rotation + delta);
        self.pan = view::rotate(self.pan, delta);
    }
    pub(crate) fn reset_rotation(&mut self) {
        self.rotate_view(-self.rotation);
    }
    /// Flip the view horizontally. Artwork pixels are unchanged.
    pub(crate) fn toggle_flip(&mut self) {
        self.mirror = !self.mirror;
        self.pan.x = -self.pan.x;
        self.status = if self.mirror {
            "Canvas flipped for checking proportions — the artwork itself is unchanged (H)".into()
        } else {
            "Canvas unflipped".into()
        };
    }
    /// Show transparent areas as a checkerboard rather than white paper.
    /// View only: exports, picking and the artwork are unchanged.
    pub(crate) fn set_show_transparency(&mut self, show: bool) {
        // Display tiles follow on the next frame; see prepare_reference_layers.
        self.show_transparency = show;
    }
    /// Pan so a document point sits at the center of the canvas viewport.
    pub(crate) fn center_view_on(&mut self, point: Pos2) {
        if self.canvas_area.width() <= 0.0 || self.zoom <= 0.0 {
            return;
        }
        let screen = self.view(self.canvas_area).to_screen(point);
        self.pan -= screen - self.canvas_area.center();
        self.auto_fit = false;
    }
    /// Set an absolute zoom about the viewport center.
    pub(crate) fn set_zoom(&mut self, zoom: f32) {
        if self.zoom > 0.0 {
            self.step_zoom(zoom / self.zoom);
        }
    }
    /// Abandon gestures that would otherwise continue after history or
    /// document changes beneath them.
    pub(crate) fn cancel_gestures(&mut self) {
        self.cancel_adjustment();
        self.close_input_gestures();
        self.flush_render_queue();
    }
    /// Close logical input without joining pending raster work. Document
    /// mutations must then either queue behind it or use cancel_gestures.
    pub(crate) fn close_input_gestures(&mut self) {
        self.finish_painted_stroke();
        self.finish_opacity_edit();
        self.finish_effects_edit();
        self.finish_adjustment_edit();
        self.selection_drag = None;
        self.finish_move_drag();
        self.finish_affine_preview();
        self.finish_line_drag();
    }
    pub(crate) fn finish_painted_stroke(&mut self) {
        self.stroke_snap = None;
        if let Some(id) = self.queued_stroke.take() {
            let record = self.vector_record.take().map(std::sync::Arc::new);
            if let Err(error) = self.render_queue.end_recorded(id, record) {
                self.status = error;
            }
        } else {
            self.finish_stroke_history();
        }
        self.stroke_buffer = None;
        self.stroke_settings = None;
        self.stroke.reset();
        self.script_stroke = None;
        self.vector_record = None;
        self.stroking = false;
        self.stroke_previous_end = None;
    }
    fn finish_stroke_history(&mut self) {
        if !self.stroking || !self.history.has_pending_stroke() {
            return;
        }
        if let (Some(buffer), Some(settings)) = (&mut self.stroke_buffer, &self.stroke_settings) {
            let finished = buffer.finish_tail(&mut self.doc, settings);
            self.performance.dabs += finished.dabs;
            if !finished.region.is_empty() {
                self.dirty_region.merge(finished.region);
                self.modified = true;
                self.revision = self.revision.wrapping_add(1);
                self.content_revision = self.content_revision.wrapping_add(1);
            }
        }
        if let Some(buffer) = &self.stroke_buffer {
            buffer.reclaim_empty_tiles(&mut self.doc);
        }
        self.replay.finish(&self.doc, false);
        if self
            .stroke_buffer
            .as_ref()
            .is_some_and(|buffer| !buffer.has_visible_changes(&self.doc))
        {
            self.history.discard_pending_checkpoint(&self.doc);
        } else {
            self.history.settle(&self.doc);
        }
    }
    pub(crate) fn cancel_painted_stroke(&mut self) {
        self.stroke_snap = None;
        if self.queued_stroke.take().is_some() {
            self.render_queue.cancel_open();
            self.last_stroke_end = self.stroke_previous_end;
            self.stroke_settings = None;
            self.stroke.reset();
            self.vector_record = None;
            self.stroking = false;
            self.stroke_previous_end = None;
            self.selection_drag = None;
            self.move_drag = None;
            self.status = "Stroke canceled".into();
            return;
        }
        if self.stroking
            && self.stroke_buffer.is_some()
            && self.history.discard_pending_checkpoint(&self.doc)
        {
            self.stroke_buffer.take().unwrap().restore(&mut self.doc);
            self.last_stroke_end = self.stroke_previous_end;
            self.replay.finish(&self.doc, true);
            self.history.refresh_memory(&self.doc);
            self.changed();
            self.status = "Stroke canceled".into();
        }
        self.cancel_gestures();
    }
    pub(crate) fn undo(&mut self) {
        self.cancel_adjustment();
        self.close_input_gestures();
        if self.render_queue.busy() {
            self.status = match self.render_queue.undo() {
                Ok(()) => "Undo queued after preceding artwork".into(),
                Err(error) => error,
            };
            return;
        }
        if self.history.undo(&mut self.doc) {
            self.changed();
            self.status = "Undone".into();
        } else if let Some(error) = self.history.take_error() {
            self.status = error;
        }
    }
    pub(crate) fn redo(&mut self) {
        self.cancel_adjustment();
        self.close_input_gestures();
        if self.render_queue.busy() {
            self.status = match self.render_queue.redo() {
                Ok(()) => "Redo queued after preceding artwork".into(),
                Err(error) => error,
            };
            return;
        }
        if self.history.redo(&mut self.doc) {
            self.changed();
            self.status = "Redone".into();
        } else if let Some(error) = self.history.take_error() {
            self.status = error;
        }
    }
    pub(crate) fn note_recent_color(&mut self) {
        if !self.input_stroke_target().is_raster() {
            return;
        }
        let color = self.color;
        self.recent_colors.retain(|c| *c != color);
        self.recent_colors.insert(0, color);
        self.recent_colors.truncate(16);
    }
    pub(crate) fn resize_brush(&mut self, factor: f32) {
        // Proportional steps, but always at least one pixel so small sizes move.
        let next = (self.brush.size * factor).round();
        let next = if factor > 1.0 {
            next.max(self.brush.size + 1.0)
        } else {
            next.min(self.brush.size - 1.0)
        };
        self.brush.size = next.clamp(1.0, 300.0);
        self.status = format!("Brush size {:.0} px", self.brush.size);
    }

    pub(crate) fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        if ctx.wants_keyboard_input()
            || self.new_canvas
            || self.unsaved_action.is_some()
            || egui::Popup::is_any_open(ctx)
            || self.renaming.is_some()
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
            || self.canvas_resize.is_some()
            || self.canvas_edit_ticket.is_some()
        {
            return;
        }
        let opening_new = ctx.input_mut(|input| {
            let opening = input.events.iter().any(crate::new_canvas::opening_shortcut);
            if opening {
                input
                    .events
                    .retain(|event| !crate::new_canvas::opening_shortcut(event));
            }
            opening
        });
        if opening_new {
            self.open_new_canvas();
            return;
        }
        let paste_shortcut = self.clipboard_paste_shortcut(ctx);
        let command_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let key = |modifiers: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        // Text selected in a label (Import details, notes) copies as text;
        // leave Copy to egui instead of copying the canvas.
        let text_selected = ctx
            .with_plugin(|labels: &mut egui::text_selection::LabelSelectionState| {
                labels.has_selection()
            })
            .unwrap_or(false);
        let (copy, cut, paste) = ctx.input_mut(|input| {
            let mut commands = (false, false, false);
            input.events.retain(|event| match event {
                egui::Event::Copy if !text_selected => {
                    commands.0 = true;
                    false
                }
                egui::Event::Cut => {
                    commands.1 = true;
                    false
                }
                egui::Event::Paste(_) => {
                    commands.2 = true;
                    false
                }
                _ => true,
            });
            commands
        });
        if cut || key(Modifiers::COMMAND, Key::X) {
            self.copy_canvas(true);
        } else if copy || (!text_selected && key(Modifiers::COMMAND, Key::C)) {
            self.copy_canvas(false);
        }
        if paste || paste_shortcut || key(Modifiers::COMMAND, Key::V) {
            self.paste_canvas();
        }
        // Chords with Shift are consumed first: unspecified modifiers match loosely.
        if key(command_shift, Key::Z) {
            self.redo();
        } else if key(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
        if key(Modifiers::COMMAND, Key::Y) {
            self.redo();
        }
        if key(command_shift, Key::N) {
            self.new_layer();
        }
        if key(command_shift, Key::S) {
            self.save(true);
        } else if key(Modifiers::COMMAND, Key::S) {
            self.save(false);
        }
        if key(Modifiers::COMMAND, Key::O) {
            self.open();
        }
        if key(command_shift, Key::I) {
            self.invert_selection();
        }
        if key(Modifiers::COMMAND, Key::A) {
            self.select_all();
        }
        if key(Modifiers::COMMAND, Key::D) {
            self.set_selection(None);
        }
        if key(command_shift, Key::G) {
            self.ungroup_folder();
        } else if key(Modifiers::COMMAND, Key::G) {
            self.group_into_folder();
        }
        if key(Modifiers::COMMAND, Key::J) {
            self.duplicate_layer();
        }
        if key(Modifiers::COMMAND, Key::E) {
            self.merge_down();
        }
        if key(Modifiers::COMMAND, Key::Equals) || key(Modifiers::COMMAND, Key::Plus) {
            self.step_zoom(1.25);
        }
        if key(Modifiers::COMMAND, Key::Minus) {
            self.step_zoom(0.8);
        }
        if key(Modifiers::COMMAND, Key::Num0) {
            self.fit_view();
        }
        if key(Modifiers::COMMAND, Key::Num1) {
            self.actual_size();
        }
        if key(command_shift, Key::T) {
            self.set_show_transparency(!self.show_transparency);
        }
        if key(Modifiers::COMMAND, Key::T) {
            self.toggle_free_transform();
        }
        if key(Modifiers::COMMAND, Key::W) {
            self.close_tab(self.tabs.active);
        }
        if self.affine_preview.is_some() {
            if key(Modifiers::NONE, Key::Enter) {
                self.finish_affine_preview();
            }
            for (modifiers, step) in [(Modifiers::NONE, 1.0), (Modifiers::SHIFT, 10.0)] {
                for (arrow, dx, dy) in [
                    (Key::ArrowLeft, -step, 0.0),
                    (Key::ArrowRight, step, 0.0),
                    (Key::ArrowUp, 0.0, -step),
                    (Key::ArrowDown, 0.0, step),
                ] {
                    if key(modifiers, arrow) {
                        self.nudge_transform(dx, dy);
                    }
                }
            }
        }
        if self.polygon_selection_points().is_some() {
            if key(Modifiers::NONE, Key::Enter) {
                self.finish_polygon_selection();
            }
            if key(Modifiers::NONE, Key::Backspace) || key(Modifiers::NONE, Key::Delete) {
                self.remove_polygon_vertex();
                return;
            }
        }
        if key(Modifiers::NONE, Key::Escape) {
            if self.affine_preview.is_some() {
                self.cancel_affine_preview();
            } else if self.move_drag.is_some() || self.pending_move.is_some() {
                self.cancel_move_drag();
            } else if self.stroking {
                self.cancel_painted_stroke();
            } else if self.selection_drag.is_some() {
                self.selection_drag = None;
            } else if self.tool == Tool::Move && self.deferred_move_ticket.is_some() {
                self.cancel_move_drag();
            } else if self.render_queue.cancel_background_edit() {
                self.status = "Canceling background edit".into();
            } else if self.tool == Tool::Pointer && !self.picked_lines().is_empty() {
                self.deselect_lines();
            } else {
                self.set_selection(None);
            }
        }
        if key(Modifiers::NONE, Key::Delete) || key(Modifiers::NONE, Key::Backspace) {
            if self.tool == Tool::Pointer && !self.picked_lines().is_empty() {
                self.delete_picked_lines();
            } else {
                self.clear_selection_pixels();
            }
        }
        if ctx.input(|i| i.modifiers.is_none()) {
            if key(Modifiers::NONE, Key::OpenBracket) {
                self.resize_brush(0.85);
            }
            if key(Modifiers::NONE, Key::CloseBracket) {
                self.resize_brush(1.0 / 0.85);
            }
            if key(Modifiers::NONE, Key::X) {
                self.swap_paint_colors();
            }
            if key(Modifiers::NONE, Key::H) {
                self.toggle_flip();
            }
            if key(Modifiers::NONE, Key::Num4) {
                self.rotate_view(-15f32.to_radians());
            }
            if key(Modifiers::NONE, Key::Num6) {
                self.rotate_view(15f32.to_radians());
            }
            if key(Modifiers::NONE, Key::Num5) {
                self.reset_rotation();
            }
        }
    }

    /// Without new pen packets (a resting pen or mouse), keep easing the
    /// stabilized line toward the pen on the frame clock, with the same
    /// smoothing it uses while the pen moves.
    fn stabilizer_catch_up(&mut self, ctx: &egui::Context, allowed: bool) {
        let now = std::time::Instant::now();
        let Some(settings) = self.stroke_settings.as_ref() else {
            return;
        };
        if !allowed
            || !self.stroking
            || !matches!(self.tool, Tool::Brush | Tool::Eraser)
            || settings.brush.stabilization <= 0.0
            || !match &self.script_stroke {
                Some(script) => script.lagging(0.25 / self.zoom.max(1e-3), &settings.brush),
                None => self
                    .stroke
                    .lagging(0.25 / self.zoom.max(1e-3), &settings.brush),
            }
        {
            self.smoothing_clock = None;
            return;
        }
        let elapsed = self
            .smoothing_clock
            .replace(now)
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        let fraction = stroke::smoothing_fraction(elapsed, &settings.brush);
        if fraction > 0.0 {
            let brush = settings.brush.clone();
            self.replay.catch_up(fraction);
            let dabs = match &mut self.script_stroke {
                Some(script) => script.catch_up(fraction, &brush),
                None => {
                    crate::vector_layer::record(
                        &mut self.vector_record,
                        crate::vector_layer::Command::CatchUp(fraction),
                    );
                    self.stroke.catch_up(fraction, &brush)
                }
            };
            self.deliver_canvas_dabs(dabs);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(8));
    }
    /// The Pointer tool pans when a drag starts away from every guide, ruler
    /// and the symmetry center; a press on one of those moves it instead.
    fn pointer_pans(
        &self,
        ctx: &egui::Context,
        area: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        if self.tool != Tool::Pointer
            || self.affine_preview.is_some()
            || self.guide_drag.is_some()
            || self.symmetry_drag
            || !response.dragged()
        {
            return false;
        }
        let Some(origin) = ctx.input(|i| i.pointer.press_origin()) else {
            return false;
        };
        let view = self.view(area);
        let symmetry = self.active_symmetry();
        let [cx, cy] = symmetry.center(self.doc.width, self.doc.height);
        let on_symmetry =
            symmetry.active() && (view.to_screen(pos2(cx, cy)) - origin).length() <= 12.0;
        !on_symmetry
            && self.guide_cursor(area, &view, origin).is_none()
            && !self.reference_grabs(view, origin)
            && !self.line_grabs(view, origin)
    }
    pub(crate) fn canvas(&mut self, ctx: &egui::Context, packets: &[input::Packet]) {
        self.tab_bar(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE.fill(crate::color_wheel::color32(self.theme.canvas_background)),
            )
            .show(ctx, |ui| self.canvas_ui(ctx, ui, packets));
        self.finish_navigator_paint(ctx);
    }

    fn canvas_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, packets: &[input::Packet]) {
        self.prepare_transform_overlay(ctx);
        self.prepare_tip_cursor(ctx);
        let (area, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
        let document_size = vec2(self.doc.width as f32, self.doc.height as f32);
        if self.zoom == 0.0 {
            self.auto_fit = true;
        }
        if self.auto_fit {
            self.zoom = view::fit_zoom(area.size(), document_size, self.rotation);
        }
        let typing = ctx.wants_keyboard_input();
        if self.unsaved_action.is_some()
            || self.dialogs.open()
            || self.canvas_edit_ticket.is_some()
            || self.adjustment_ticket.is_some()
        {
            self.pump_render_queue(ctx);
            self.canvas_area = area;
            let view = self.view(area);
            self.upload_canvas(ctx);
            self.paint_canvas(ctx, ui, area, &view, None, false);
            return;
        }
        let (space, shift, command, rotate_key, middle, alt) = ctx.input(|i| {
            (
                i.key_down(egui::Key::Space) && !typing,
                i.modifiers.shift,
                i.modifiers.command,
                i.key_down(egui::Key::R) && !typing,
                i.pointer.button_down(egui::PointerButton::Middle),
                i.modifiers.alt,
            )
        });
        let nav = if middle {
            Some(Nav::Pan)
        } else if space && command {
            Some(Nav::Zoom)
        } else if (space && shift) || rotate_key {
            Some(Nav::Rotate)
        } else if space || self.pointer_pans(ctx, area, &response) {
            Some(Nav::Pan)
        } else {
            None
        };
        if response.hovered() {
            let scroll = ctx.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0
                && let Some(cursor) = ctx.input(|i| i.pointer.hover_pos())
            {
                self.zoom_about(area.center(), cursor, self.zoom * (scroll * 0.0015).exp());
            }
        }
        if let Some(nav) = nav
            && response.dragged()
        {
            let delta = ctx.input(|i| i.pointer.delta());
            match nav {
                Nav::Pan => {
                    self.auto_fit = false;
                    self.pan += delta;
                }
                Nav::Rotate => {
                    if let Some(pointer) = ctx.input(|i| i.pointer.interact_pos()) {
                        let after = pointer - area.center();
                        let before = after - delta;
                        if after.length() > 8.0 && before.length() > 8.0 {
                            let turn = after.y.atan2(after.x) - before.y.atan2(before.x);
                            self.rotate_view(view::normalize(turn));
                        }
                    }
                }
                Nav::Zoom => {
                    let anchor = ctx
                        .input(|i| i.pointer.press_origin())
                        .unwrap_or(area.center());
                    let factor = ((delta.x - delta.y) * 0.008).exp();
                    self.zoom_about(area.center(), anchor, self.zoom * factor);
                }
            }
        }
        if response.drag_stopped()
            && nav == Some(Nav::Rotate)
            && self.rotation.abs() < 2f32.to_radians()
        {
            self.reset_rotation();
        }

        self.canvas_area = area;
        let view = self.view(area);
        let modal = self.new_canvas
            || self.input_settings
            || self.recovery_window
            || self.adjustment.is_some()
            || self.adjustment_ticket.is_some()
            || self.canvas_resize.is_some();
        let navigation_allowed = nav.is_none() && !modal;
        let blocker = self.canvas_paint_blocker();
        let allowed = navigation_allowed
            && blocker.is_none()
            && !(self.tool == Tool::Eraser
                && self.input_stroke_target().is_raster()
                && self.input_alpha_locked());
        let symmetry = self.active_symmetry();
        let symmetry_center = symmetry.center(self.doc.width, self.doc.height);
        // Locked guides cannot be picked up by a stroke that starts nearby.
        let symmetry_handle = (symmetry.active()
            && (self.tool == Tool::Pointer
                || (!self.guides_locked && matches!(self.tool, Tool::Brush | Tool::Eraser))))
        .then(|| view.to_screen(pos2(symmetry_center[0], symmetry_center[1])));
        if !navigation_allowed && (modal || self.polygon_selection_points().is_none()) {
            self.selection_drag = None;
        }
        if !allowed {
            self.finish_painted_stroke();
        }
        // A press on a sidebar's resize edge resizes the panel, never paints.
        let sidebar_edge = crate::workspace::sidebar_edge_active(ctx);
        let mut moved = false;
        for packet in packets {
            let (sample, start, end) = match packet {
                input::Packet::Start(s) => (*s, true, false),
                input::Packet::Move(s) => (*s, false, false),
                input::Packet::End(s) => (*s, false, true),
                input::Packet::Cancel => {
                    self.cancel_ruler_drag();
                    if let Some(drag) = self.guide_drag.take()
                        && drag.index < self.guide_lines.len()
                    {
                        match drag.original {
                            Some(position) => self.guide_lines[drag.index].position = position,
                            None => {
                                self.guide_lines.remove(drag.index);
                            }
                        }
                    }
                    self.cancel_affine_preview();
                    self.cancel_reference_drag();
                    self.finish_line_drag();
                    if self.move_drag.is_some()
                        || self.pending_move.is_some()
                        || (self.tool == Tool::Move
                            && !self.stroking
                            && self.selection_drag.is_none())
                    {
                        self.cancel_move_drag();
                    }
                    self.cancel_painted_stroke();
                    continue;
                }
            };
            let pos = pos2(sample.x, sample.y);
            let at = view.to_doc(pos);
            let inside = at.x >= 0.0
                && at.y >= 0.0
                && at.x < self.doc.width as f32
                && at.y < self.doc.height as f32;
            let on_ruler = self.show_rulers && crate::guides::ruler_at(area, pos).is_some();
            let on_workspace = area.contains(pos)
                && ctx.layer_id_at(pos) == Some(response.layer_id)
                && !(start && (sidebar_edge || on_ruler));
            let on_canvas = inside && on_workspace;
            if self.guide_packet(area, &view, pos, at, start, end, navigation_allowed) {
                continue;
            }
            if start && on_canvas && navigation_allowed {
                self.finish_opacity_edit();
            }
            let local_x = (at.x.floor().max(0.0) as usize).min(self.doc.width - 1);
            let local_y = (at.y.floor().max(0.0) as usize).min(self.doc.height - 1);
            // Free transform owns canvas input under any tool until applied or
            // canceled; Alt scales from the center there instead of picking.
            if self.affine_preview.is_some() && navigation_allowed {
                self.affine_handle_packet(
                    view,
                    pos,
                    start,
                    end,
                    area.contains(pos) && ctx.layer_id_at(pos) == Some(response.layer_id),
                    crate::affine_preview::Modifiers {
                        shift,
                        alt,
                        ctrl: command,
                    },
                );
                continue;
            }
            // A selected reference layer owns canvas input until deselected.
            if navigation_allowed
                && self.reference_packet(
                    view,
                    pos,
                    start,
                    end,
                    area.contains(pos) && ctx.layer_id_at(pos) == Some(response.layer_id),
                    shift,
                )
            {
                continue;
            }
            let picking = self.tool == Tool::Picker || alt;
            // Dragging the symmetry center handle repositions the guide.
            if start
                && navigation_allowed
                && symmetry_handle.is_some_and(|handle| (handle - pos).length() <= 12.0)
            {
                self.symmetry_drag = true;
            }
            if self.symmetry_drag {
                let clamped = [
                    at.x.clamp(0.0, self.doc.width as f32),
                    at.y.clamp(0.0, self.doc.height as f32),
                ];
                self.active_symmetry_mut().center = Some(clamped);
                if end {
                    self.symmetry_drag = false;
                    self.status = "Symmetry center moved".into();
                }
                continue;
            }
            if navigation_allowed
                && self.vector_pick_packet(
                    view,
                    pos,
                    start,
                    end,
                    area.contains(pos) && ctx.layer_id_at(pos) == Some(response.layer_id),
                    shift,
                )
            {
                continue;
            }
            if start && on_canvas && navigation_allowed && picking {
                self.cancel_gestures();
                if let crate::paint_target::Target::Mask(id) = self.input_stroke_target() {
                    if let Some(mask) = self
                        .doc
                        .layers
                        .iter()
                        .find(|l| Some(l.id) == self.input_target())
                        .and_then(|l| l.mask.as_ref())
                        .filter(|m| m.id == id)
                    {
                        self.mask_colors[0] = mask.coverage.at(local_x, local_y);
                        self.status = format!(
                            "Sampled mask coverage: {:.0}% reveal",
                            f32::from(self.mask_colors[0]) * 100.0 / 255.0
                        );
                    } else {
                        self.status = "Mask sampling target is not ready".into();
                    }
                } else if let Some(color) = self.doc.pick_color(local_x, local_y) {
                    self.color = color;
                    self.status = "Color sampled from visible artwork".into();
                }
                self.finish_painted_stroke();
                continue;
            }
            if (self.tool == Tool::Select || self.tool.is_lasso_paint())
                && navigation_allowed
                && !picking
            {
                // Shapes may start on the workspace beside the artwork and
                // clamp to its edge; only the wand needs a pixel to sample.
                let wand = self.tool == Tool::Select
                    && self.selection_shape == crate::selection_ui::Shape::Wand;
                let can_start = if wand {
                    on_canvas
                } else {
                    area.contains(pos)
                        && ctx.layer_id_at(pos) == Some(response.layer_id)
                        && !(start && (sidebar_edge || on_ruler))
                };
                self.update_selection_drag(
                    [f64::from(at.x), f64::from(at.y)],
                    [local_x, local_y],
                    start,
                    end,
                    can_start,
                );
                continue;
            }
            if self.tool == Tool::Move && navigation_allowed && !picking {
                if start && on_canvas {
                    self.start_move_drag(at);
                }
                if let Some(drag) = &mut self.pending_move
                    && !drag.released
                {
                    drag.offset = [
                        (at.x - drag.start.x).round() as i64,
                        (at.y - drag.start.y).round() as i64,
                    ];
                    moved = true;
                }
                if let Some(drag) = &mut self.move_drag {
                    drag.offset = [
                        (at.x - drag.start.x).round() as i64,
                        (at.y - drag.start.y).round() as i64,
                    ];
                    moved = true;
                }
                if end {
                    self.apply_move();
                    self.finish_move_drag();
                    moved = false;
                }
                continue;
            }
            if start && on_canvas && allowed && self.tool == Tool::Fill {
                self.fill_canvas(local_x, local_y);
                self.finish_painted_stroke();
                continue;
            }
            if start {
                if on_workspace {
                    if let Some(blocker) = self.canvas_paint_blocker() {
                        self.status = blocker.into();
                    } else if self.tool == Tool::Eraser
                        && self.input_stroke_target().is_raster()
                        && self.input_alpha_locked()
                    {
                        self.status =
                            "Alpha lock protects transparency — turn it off to erase".into();
                    } else {
                        self.status = if self.input_stroke_target().is_raster() {
                            "Ready"
                        } else {
                            "Editing mask: black conceals, white reveals"
                        }
                        .into();
                    }
                }
                self.finish_painted_stroke();
                let vector_erase = allowed
                    && on_workspace
                    && (self.tool == Tool::Eraser || sample.eraser)
                    && self.vector_target().is_some();
                if vector_erase {
                    // The eraser removes whole strokes from a vector layer.
                    let brush = if sample.eraser {
                        self.eraser_for_stroke().into_owned()
                    } else {
                        self.brush.clone()
                    };
                    self.begin_vector_erase(brush);
                } else if allowed
                    && on_workspace
                    && matches!(self.tool, Tool::Brush | Tool::Eraser)
                    && let Some(blocker) = self.vector_stroke_blocker()
                {
                    self.status = blocker.into();
                } else if allowed && on_workspace && matches!(self.tool, Tool::Brush | Tool::Eraser)
                {
                    let layer = self.input_layer().unwrap_or(self.doc.active);
                    // The pen's eraser end uses the eraser tip when it has its own.
                    let brush = if sample.eraser {
                        self.eraser_for_stroke().into_owned()
                    } else {
                        self.brush.clone()
                    };
                    let settings = crate::stroke_buffer::Settings {
                        target: self.input_stroke_target(),
                        brush,
                        color: self.paint_color(),
                        erase: self.tool == Tool::Eraser || sample.eraser,
                        layer,
                        symmetry: self.active_symmetry(),
                    };
                    if !self.begin_canvas_stroke(settings) {
                        continue;
                    }
                    self.begin_script_stroke();
                    self.begin_vector_record();
                    if self.tool == Tool::Brush {
                        self.note_recent_color();
                    }
                    // Shift+click continues from the previous stroke's end as a
                    // line (scripted tools draw only what their script asks).
                    if shift
                        && self.script_stroke.is_none()
                        && let Some(last) = self.last_stroke_end
                    {
                        self.replay.seed(last, self.zoom);
                        let brush = self
                            .stroke_settings
                            .as_ref()
                            .map_or(&self.brush, |s| &s.brush);
                        let _ = self.stroke.sample_at_zoom(last, brush, self.zoom);
                        crate::vector_layer::record(
                            &mut self.vector_record,
                            crate::vector_layer::Command::Seed {
                                sample: last,
                                zoom: self.zoom,
                            },
                        );
                        self.stroke_snap = None;
                    } else {
                        let (_, active) = self.active_rulers();
                        let sources = crate::rulers::Sources {
                            rulers: &active,
                            guides: &self.guide_lines,
                            grid: (self.snap_grid && self.grid.visible)
                                .then_some(self.grid.spacing as f32),
                            zoom: self.zoom,
                        };
                        self.stroke_snap = self
                            .snapping
                            .then(|| crate::rulers::StrokeSnap::begin(&sources, [at.x, at.y]))
                            .flatten();
                    }
                }
            }
            if picking {
                self.finish_painted_stroke();
            }
            if self.vector_erase.is_some() {
                let local = stroke::Sample {
                    x: at.x,
                    y: at.y,
                    ..sample
                };
                self.vector_erase_sample(local, end);
                if end {
                    self.finish_vector_erase();
                }
                continue;
            }
            if allowed && self.stroking && matches!(self.tool, Tool::Brush | Tool::Eraser) {
                let Some(settings) = &self.stroke_settings else {
                    continue;
                };
                if self.queued_stroke.is_none() && settings.layer != self.doc.active {
                    self.cancel_gestures();
                    continue;
                }
                let brush = &settings.brush;
                // The stroke owns this contact until lift/cancel. Keep off-page
                // samples for stabilization and edge crossings; raster writes clip.
                let local = stroke::Sample {
                    x: at.x,
                    y: at.y,
                    ..sample
                };
                // Packets advance the smoothing themselves; frame-clock
                // catch-up only covers time after the latest packet.
                self.smoothing_clock = Some(std::time::Instant::now());
                // A snapped stroke draws its points projected onto the
                // chosen ruler; replay records those document points.
                let points = match &mut self.stroke_snap {
                    Some(snap) => snap.feed(local, self.zoom, end),
                    None => {
                        self.replay.point(sample, view, end);
                        vec![local]
                    }
                };
                let snapped = self.stroke_snap.is_some();
                let count = points.len();
                let mut dabs = Vec::new();
                let mut failed = None;
                for (index, point) in points.into_iter().enumerate() {
                    let lift = end && index + 1 == count;
                    if snapped {
                        self.replay.local(point, self.zoom, lift);
                    }
                    let drawn = match &mut self.script_stroke {
                        Some(script) if lift => script.finish(point, brush, self.zoom),
                        Some(script) => script.sample(point, brush, self.zoom),
                        None if lift => Ok(self.stroke.finish_at_zoom(point, brush, self.zoom)),
                        None => Ok(self.stroke.sample_at_zoom(point, brush, self.zoom)),
                    };
                    if self.script_stroke.is_none() {
                        crate::vector_layer::record(
                            &mut self.vector_record,
                            crate::vector_layer::Command::Sample {
                                sample: point,
                                zoom: self.zoom,
                                lift,
                            },
                        );
                    }
                    match drawn {
                        Ok(drawn) => dabs.extend(drawn),
                        Err(error) => {
                            failed = Some(error);
                            break;
                        }
                    }
                    self.last_stroke_end = Some(point);
                }
                self.deliver_canvas_dabs(dabs);
                if let Some(error) = failed {
                    // What was drawn stays, as one undo step; the tool stops.
                    let label = self.scripted_tool.take().map(|a| a.label);
                    self.status = format!("{}: {error}", label.unwrap_or_default());
                    self.finish_painted_stroke();
                    continue;
                }
            }
            if end {
                self.finish_painted_stroke();
            }
        }
        self.stabilizer_catch_up(ctx, allowed);
        self.apply_line_drag();
        if moved || self.move_drag.is_some() || self.pending_move.is_some() {
            self.apply_move();
        }
        if self
            .move_drag
            .as_ref()
            .is_some_and(|drag| drag.engine.waiting())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        if self.pending_move.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        self.poll_affine_preview();
        if self
            .affine_preview
            .as_ref()
            .is_some_and(|p| p.needs_repaint())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
        self.pump_render_queue(ctx);
        self.upload_canvas(ctx);
        if let Some(error) = self
            .selection_contour
            .update(self.doc.selection_mask.as_ref(), ctx)
        {
            self.status = error;
        }
        self.paint_canvas(ctx, ui, area, &view, nav, navigation_allowed);
    }

    /// Admit the latest offset and publish only a ready, current preview.
    fn start_move_drag(&mut self, start: Pos2) {
        if self.canvas_edit_ticket.is_some() {
            return;
        }
        let paint_target = self.input_stroke_target();
        self.close_input_gestures();
        if self.pending_move.is_some() {
            return;
        } // A released move is retrying a full queue.
        self.deferred_move_ticket = None;
        if let Some(blocker) = self.stroke_target_blocker(paint_target, false) {
            self.status = blocker.into();
        } else if paint_target.is_raster() && self.input_alpha_locked() {
            self.status = "Turn off Lock transparency to move pixels".into();
        } else if let Some(target) = self.input_target() {
            self.pending_move = Some(PendingMove {
                start,
                offset: [0, 0],
                target,
                paint_target,
                released: false,
            });
        }
    }
    fn apply_move(&mut self) {
        if self
            .pending_move
            .as_ref()
            .is_some_and(|pending| pending.released)
        {
            self.finish_move_drag();
            return;
        }
        if self.pending_move.is_some() {
            if self.render_queue.busy() {
                return;
            }
            let pending = self.pending_move.take().unwrap();
            if self.doc.layers[self.doc.active].id != pending.target {
                self.status = "Move target is no longer active".into();
                return;
            }
            if let Some(blocker) = pending
                .paint_target
                .blocker(&self.doc, self.doc.active, false)
            {
                self.status = blocker.into();
                return;
            }
            if pending.paint_target.is_raster() && self.doc.layers[self.doc.active].alpha_locked {
                self.status = "Turn off Lock transparency to move pixels".into();
                return;
            }
            self.begin_geometry_preview();
            self.move_drag = Some(MoveDrag {
                start: pending.start,
                engine: crate::move_preview::Engine::with_target(&self.doc, pending.paint_target),
                checkpointed: false,
                modified_before: self.modified,
                offset: pending.offset,
            });
        }
        let Some(drag) = &mut self.move_drag else {
            return;
        };
        drag.engine.request(drag.offset);
        match drag.engine.poll(&self.doc) {
            Ok(Some(candidate)) => self.publish_move_candidate(candidate),
            Ok(None) => {}
            Err(error) => self.abort_move_preview(error),
        }
    }
    fn publish_move_candidate(&mut self, candidate: crate::move_preview::Candidate) {
        let Some(drag) = &mut self.move_drag else {
            return;
        };
        if let Err(error) = drag.engine.guard(&self.doc) {
            self.abort_move_preview(error);
            return;
        }
        let changed = candidate.changed;
        if changed && !drag.checkpointed {
            self.history.begin_gesture_checkpoint(&self.doc);
            drag.checkpointed = true;
        }
        if let Err(error) = drag.engine.publish(&mut self.doc, candidate) {
            self.abort_move_preview(error);
            return;
        }
        if changed {
            self.geometry_preview_changed();
        }
    }
    fn abort_move_preview(&mut self, error: String) {
        if let Some(drag) = self.move_drag.take() {
            if drag.checkpointed {
                self.history.settle(&self.doc);
            }
            self.finish_geometry_preview(
                drag.engine.net_changed || drag.engine.guard(&self.doc).is_err(),
            );
        }
        self.status = error;
    }
    fn finish_move_drag(&mut self) {
        if let Some(mut pending) = self.pending_move.take() {
            if pending.offset == [0, 0] {
                self.status = "Move made no changes".into();
            } else {
                let edit = match pending.paint_target {
                    crate::paint_target::Target::Raster => {
                        crate::document_edits::Edit::MoveTarget {
                            target: pending.target,
                            offset: pending.offset,
                        }
                    }
                    crate::paint_target::Target::Mask(mask) => {
                        crate::document_edits::Edit::MaskTransform {
                            target: pending.target,
                            mask,
                            operation: crate::mask_transform::Operation::Move(pending.offset),
                        }
                    }
                };
                match self.render_queue.edit(edit) {
                    Ok(ticket) => {
                        self.deferred_move_ticket = Some(ticket);
                        self.edit_feedback.insert(
                            ticket,
                            crate::render_controller::EditFeedback {
                                changed: "Move applied",
                                unchanged: "Move made no changes",
                            },
                        );
                        self.status = "Move queued after preceding artwork".into();
                    }
                    Err(error) => {
                        pending.released = true;
                        self.pending_move = Some(pending);
                        self.status = error;
                    }
                }
            }
            return;
        }
        let Some(mut drag) = self.move_drag.take() else {
            return;
        };
        drag.engine.request(drag.offset);
        if !drag.checkpointed && !drag.engine.net_changed && drag.engine.finished() {
            self.finish_geometry_preview(false);
            self.status = "Move made no changes".into();
            return;
        }
        match self
            .render_queue
            .finish_move(drag.engine, drag.checkpointed, drag.modified_before)
        {
            Ok(()) => self.status = "Finishing Move in background".into(),
            Err((engine, error)) => {
                self.move_drag = Some(MoveDrag {
                    start: drag.start,
                    engine: *engine,
                    checkpointed: drag.checkpointed,
                    modified_before: drag.modified_before,
                    offset: drag.offset,
                });
                self.status = error;
            }
        }
    }
    fn cancel_move_drag(&mut self) {
        if self.pending_move.take().is_some() {
            self.status = "Move canceled".into();
            return;
        }
        if self.move_drag.is_none()
            && let Some(ticket) = self.deferred_move_ticket.take()
        {
            self.status = if self.render_queue.cancel_edit(ticket) {
                "Canceling Move"
            } else {
                "Move published; use Undo"
            }
            .into();
            return;
        }
        let Some(drag) = self.move_drag.take() else {
            return;
        };
        if let Err(error) = drag.engine.guard(&self.doc) {
            if drag.checkpointed {
                self.history.settle(&self.doc);
            }
            self.finish_geometry_preview(true);
            self.status = error;
            return;
        }
        if drag.checkpointed {
            let source = drag.engine.original();
            self.doc.layers[self.doc.active].pixels = source.layers[source.active].pixels.clone();
            self.doc.selection = source.selection;
            self.doc.layers[self.doc.active].mask = source.layers[source.active].mask.clone();
            self.doc.selection_mask = source.selection_mask.clone();
            self.history.discard_pending_checkpoint(&self.doc);
            self.geometry_preview_changed();
        }
        if !self.finish_geometry_preview(false) {
            self.modified = drag.modified_before;
        }
        self.status = "Move canceled".into();
    }

    fn texture_options(&self) -> egui::TextureOptions {
        egui::TextureOptions {
            // Crisp pixels when zoomed in; trilinear filtering when zoomed out.
            magnification: if self.zoom >= 2.0 {
                egui::TextureFilter::Nearest
            } else {
                egui::TextureFilter::Linear
            },
            minification: egui::TextureFilter::Linear,
            wrap_mode: egui::TextureWrapMode::ClampToEdge,
            mipmap_mode: Some(egui::TextureFilter::Linear),
        }
    }

    /// While the mask view is on and brushes target a mask: the mask in
    /// grays (white reveals, black conceals), as a white layer carrying the
    /// mask over a black one, so mask edits redraw only the tiles they touch.
    pub(crate) fn mask_view_document(&mut self) -> Option<crate::document::Document> {
        if !self.mask_view {
            return None;
        }
        let crate::paint_target::Target::Mask(id) = self.input_stroke_target() else {
            return None;
        };
        let mask = self
            .doc
            .layers
            .iter()
            .find_map(|l| l.mask.as_ref().filter(|m| m.id == id))?
            .clone();
        let size = [self.doc.width, self.doc.height];
        let (black, white) = match &self.mask_view_layers {
            Some((cached, black, white)) if *cached == size => (black.clone(), white.clone()),
            _ => {
                let black = crate::pixels::Pixels::solid(size[0], size[1], [0, 0, 0, 255]);
                let white = crate::pixels::Pixels::solid(size[0], size[1], [255; 4]);
                self.mask_view_layers = Some((size, black.clone(), white.clone()));
                (black, white)
            }
        };
        let mut view = self.doc.clone();
        let mut below = crate::document::Layer::raster("Mask view".into(), size[0], size[1]);
        below.pixels = black;
        let mut above = crate::document::Layer::raster("Mask".into(), size[0], size[1]);
        above.pixels = white;
        above.mask = Some(crate::layer_mask::LayerMask {
            enabled: true,
            ..mask
        });
        // Fixed identities keep the display caches from seeing new layers.
        below.id = self.mask_view_ids.0;
        above.id = self.mask_view_ids.1;
        view.layers = vec![below, above];
        view.active = 1;
        view.version = 2;
        Some(view)
    }
    fn upload_canvas(&mut self, ctx: &egui::Context) {
        self.prepare_reference_layers(ctx);
        let options = self.texture_options();
        self.texture
            .set_viewport(self.canvas_area, self.view(self.canvas_area));
        self.preview.note_damage(self.dirty, &self.dirty_region);
        let scene = self.reference_layers.display_document(&self.doc);
        let mask_view = self.mask_view_document();
        let base = mask_view.as_ref().or(scene.as_ref()).unwrap_or(&self.doc);
        // Layer effects are drawn as extra layers. A layer whose drawing is
        // out of date shows its previous one while a background worker
        // redraws around what changed, so effects follow a stroke and
        // finishing one on a huge layer never pauses.
        let mut drawings = Vec::new();
        let expanded = if base.needs_expansion() {
            let repaint = ctx.clone();
            let wake: crate::layer_effects::Wake =
                std::sync::Arc::new(move || repaint.request_repaint());
            let (doc, used, waiting) = crate::layer_effects::expand_for_display(base, &wake);
            drawings = used;
            self.effects_waiting = waiting;
            Some(doc)
        } else {
            self.effects_waiting = false;
            None
        };
        let damage = self.dirty_region.take();
        // Redrawn effects can change anywhere they reach; a full update
        // still recomposites only tiles whose layers changed.
        // Switching between the mask view and the artwork replaces everything.
        let switched = mask_view.is_some() != self.mask_view_shown;
        self.mask_view_shown = mask_view.is_some();
        // Effect drawings this canvas has not shown yet (drawn here or by an
        // export, preview or color pick) can differ anywhere they reach.
        let redrawn = drawings != self.effects_shown;
        if redrawn {
            self.effects_shown = drawings;
        }
        let full = self.dirty || redrawn || switched;
        self.texture.update_damage(
            ctx,
            expanded.as_ref().unwrap_or(base),
            full,
            damage,
            options,
            &mut self.performance,
        );
        self.dirty = false;
        if let Some(error) = self.texture.take_error() {
            self.status = error;
        }
    }

    fn paint_canvas(
        &self,
        ctx: &egui::Context,
        ui: &egui::Ui,
        area: egui::Rect,
        view: &view::View,
        nav: Option<Nav>,
        navigation_allowed: bool,
    ) {
        let painter = ui.painter_at(area);
        let corners = view.corners();
        // Soft drop shadow lifts the paper off the workspace.
        for (spread, alpha) in [(10.0, 18), (5.0, 30), (2.0, 50)] {
            let center = view.origin;
            let grown: Vec<Pos2> = corners
                .iter()
                .map(|c| *c + (*c - center).normalized() * spread + vec2(0.0, spread * 0.4))
                .collect();
            painter.add(egui::Shape::convex_polygon(
                grown,
                Color32::from_black_alpha(alpha),
                Stroke::NONE,
            ));
        }
        painter.extend(self.layered_canvas_shapes(area, *view, false));
        let dimensions = [self.doc.width, self.doc.height];
        crate::guides::paint_grid(&painter, view, dimensions, self.grid);
        self.paint_affine_handles(&painter, *view);
        self.paint_reference_handles(&painter, *view);
        self.paint_vector_pick(&painter, *view);
        if let Some(
            crate::selection_ui::Drag::Lasso { points, .. }
            | crate::selection_ui::Drag::PaintLasso { points, .. },
        ) = &self.selection_drag
        {
            let mut outline: Vec<Pos2> = points
                .iter()
                .map(|p| view.to_screen(pos2(p[0] as f32, p[1] as f32)))
                .collect();
            if let Some(first) = outline.first().copied() {
                outline.push(first);
            }
            painter.add(egui::Shape::line(
                outline.clone(),
                Stroke::new(3.0_f32, Color32::BLACK),
            ));
            painter.add(egui::Shape::line(
                outline,
                Stroke::new(1.0_f32, Color32::WHITE),
            ));
        }
        if let Some(points) = self.polygon_selection_points() {
            let mut outline: Vec<Pos2> = points
                .iter()
                .map(|p| view.to_screen(pos2(p[0] as f32, p[1] as f32)))
                .collect();
            if let Some(first) = outline.first().copied() {
                if let Some(hover) = ctx
                    .input(|input| input.pointer.hover_pos())
                    .filter(|p| area.contains(*p))
                {
                    outline.push(hover);
                }
                outline.push(first);
                painter.add(egui::Shape::line(
                    outline.clone(),
                    Stroke::new(3.0_f32, Color32::BLACK),
                ));
                painter.add(egui::Shape::line(
                    outline,
                    Stroke::new(1.0_f32, Color32::WHITE),
                ));
                painter.circle_stroke(
                    first,
                    6.0,
                    Stroke::new(1.5_f32, Color32::from_rgb(64, 200, 190)),
                );
            }
        }
        // Rope stabilizer: the string from the drawn point to the pen.
        if self.stroking
            && let Some(settings) = &self.stroke_settings
            && let Some((at, pen, _)) = self.stroke.rope(&settings.brush)
        {
            let (at, pen) = (
                view.to_screen(pos2(at[0], at[1])),
                view.to_screen(pos2(pen[0], pen[1])),
            );
            let accent = ctx.style().visuals.selection.bg_fill;
            painter.line_segment(
                [at, pen],
                Stroke::new(3.0_f32, Color32::from_black_alpha(90)),
            );
            painter.line_segment([at, pen], Stroke::new(1.5_f32, accent));
            painter.circle_filled(at, 3.5, accent);
            painter.circle_stroke(pen, 4.0, Stroke::new(1.5_f32, accent));
        }
        let contour = self
            .doc
            .selection_mask
            .as_ref()
            .and_then(|mask| self.selection_contour.contour(mask));
        if let Some(contour) = contour {
            contour.paint(&painter, *view);
        }
        if let Some(selection) = self
            .selection_drag
            .as_ref()
            .and_then(|drag| drag.rectangle())
            .or(if contour.is_none() {
                self.doc.selection
            } else {
                None
            })
        {
            let points: Vec<Pos2> = [
                pos2(selection.left as f32, selection.top as f32),
                pos2(selection.right as f32, selection.top as f32),
                pos2(selection.right as f32, selection.bottom as f32),
                pos2(selection.left as f32, selection.bottom as f32),
                pos2(selection.left as f32, selection.top as f32),
            ]
            .map(|p| view.to_screen(p))
            .to_vec();
            painter.add(egui::Shape::line(
                points.clone(),
                Stroke::new(1.0_f32, Color32::WHITE),
            ));
            painter.extend(egui::Shape::dashed_line(
                &points,
                Stroke::new(1.0_f32, Color32::BLACK),
                5.0,
                5.0,
            ));
        }
        if self.active_symmetry().active() {
            self.paint_symmetry_guide(&painter, view);
        }
        crate::guides::paint_lines(
            &painter,
            view,
            dimensions,
            &self.guide_lines,
            self.guide_drag.map(|drag| drag.index),
        );
        crate::rulers::paint(
            &painter,
            view,
            &self.active_rulers().1,
            self.snapping,
            self.stroke_snap.as_ref().and_then(|snap| snap.chosen()),
        );
        if self.show_rulers {
            crate::guides::paint_rulers(&painter, area, view, self.rotation.abs() > 1e-4);
        }
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let over_canvas =
            hover.filter(|p| area.contains(*p) && ctx.layer_id_at(*p) == Some(ui.layer_id()));
        let Some(pos) = over_canvas else {
            return;
        };
        let dragging = ctx.input(|i| i.pointer.any_down());
        match nav {
            Some(Nav::Pan) => ctx.set_cursor_icon(if dragging {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            }),
            Some(Nav::Rotate) => ctx.set_cursor_icon(CursorIcon::Alias),
            Some(Nav::Zoom) => ctx.set_cursor_icon(CursorIcon::ZoomIn),
            None => {}
        }
        if nav.is_some() || !navigation_allowed {
            return;
        }
        if let Some(cursor) = self.transform_cursor(*view, pos) {
            ctx.set_cursor_icon(cursor);
            return;
        }
        if let Some(cursor) = self.guide_cursor(area, view, pos) {
            ctx.set_cursor_icon(cursor);
            return;
        }
        let picking = self.tool == Tool::Picker || ctx.input(|i| i.modifiers.alt);
        if picking {
            ctx.set_cursor_icon(CursorIcon::Crosshair);
            return;
        }
        match self.tool {
            Tool::Brush | Tool::Eraser => {
                ctx.set_cursor_icon(CursorIcon::None);
                let (placement, brush) = self.tip_placement(pos, *view);
                self.paint_tip_cursor(&painter, *view, &brush, &placement);
                // A small crosshair keeps the hotspot visible for tiny brushes.
                for (a, b) in [
                    (vec2(-4.0, 0.0), vec2(4.0, 0.0)),
                    (vec2(0.0, -4.0), vec2(0.0, 4.0)),
                ] {
                    painter.line_segment(
                        [pos + a, pos + b],
                        Stroke::new(3.0_f32, Color32::from_black_alpha(110)),
                    );
                    painter.line_segment([pos + a, pos + b], Stroke::new(1.0_f32, Color32::WHITE));
                }
            }
            Tool::Move => ctx.set_cursor_icon(CursorIcon::Move),
            Tool::Pointer => ctx.set_cursor_icon(CursorIcon::Default),
            Tool::Fill | Tool::Picker | Tool::Select | Tool::Lasso => {
                ctx.set_cursor_icon(CursorIcon::Crosshair)
            }
        }
    }
}

const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Tools",
        &[
            ("B", "Brush"),
            ("E", "Eraser"),
            ("G", "Fill"),
            ("I", "Color picker (hold Alt with any tool)"),
            ("M", "Rectangle selection"),
            ("V", "Move selection or layer"),
            (
                "O",
                "Pointer: move guides and symmetry center, pan the view",
            ),
            (
                "Hold a tool key > 0.5 s",
                "Use temporarily; release to return",
            ),
            ("Shift + click", "Straight line from the last stroke"),
            (
                "Drag symmetry center",
                "Move the symmetry guide (Pointer, or unlocked guides)",
            ),
            (
                "Ctrl + T",
                "Free transform the selection or layer; again to apply",
            ),
            ("Enter  /  Esc", "Apply / cancel free transform"),
            ("Arrows  (Shift)", "Nudge free transform 1px (10px)"),
            (
                "Ctrl + drag (mirror pair)",
                "Snap the moving half onto the mirror axis",
            ),
            ("[  /  ]", "Smaller / larger brush"),
            ("X", "Swap main and secondary color"),
        ],
    ),
    (
        "View",
        &[
            ("Space + drag", "Pan (or middle mouse / pen button)"),
            ("Scroll", "Zoom at the cursor"),
            ("Ctrl + Space + drag", "Zoom"),
            ("R + drag  or  Shift + Space + drag", "Rotate view"),
            ("4  /  6", "Rotate view 15° left / right"),
            ("5", "Reset rotation"),
            ("H", "Flip canvas horizontally (view only)"),
            ("Ctrl + 0  /  Ctrl + 1", "Fit / actual pixels"),
            ("Ctrl + +  /  Ctrl + -", "Zoom in / out"),
            ("Tab", "Hide or show panels"),
            ("F6", "Focus controls; then Tab / Shift+Tab to navigate"),
        ],
    ),
    (
        "Edit",
        &[
            ("Ctrl + Z", "Undo"),
            ("Ctrl + Shift + Z  /  Ctrl + Y", "Redo"),
            ("Ctrl + C  /  Ctrl + X", "Copy / cut layer pixels"),
            ("Ctrl + V", "Paste image as a new layer"),
            ("Ctrl + A  /  Ctrl + D", "Select all / deselect"),
            ("Ctrl + Shift + I", "Invert selection"),
            ("Delete", "Clear selection or layer"),
            ("Esc", "Cancel selection"),
        ],
    ),
    (
        "Layers and files",
        &[
            ("Ctrl + Shift + N", "New layer"),
            ("Ctrl + J", "Duplicate layer"),
            ("Ctrl + E", "Merge down (or merge a selected folder)"),
            (
                "Ctrl + G  /  Ctrl + Shift + G",
                "Put layer in a new folder / ungroup",
            ),
            ("Ctrl + N  /  Ctrl + O", "New canvas / open"),
            ("Ctrl + S  /  Ctrl + Shift + S", "Save / save as"),
        ],
    ),
];

impl Dudel {
    pub(crate) fn shortcuts_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("Keyboard shortcuts")
            .open(&mut self.shortcuts_window)
            .default_width(460.0)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Tablet express keys can be mapped to these keys in your pen driver.",
                    )
                    .weak(),
                );
                egui::ScrollArea::vertical()
                    .max_height(480.0)
                    .show(ui, |ui| {
                        for (group, entries) in SHORTCUTS {
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(*group).strong());
                            egui::Grid::new(group)
                                .num_columns(2)
                                .spacing([16.0, 4.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for (keys, action) in *entries {
                                        ui.label(egui::RichText::new(*keys).monospace());
                                        ui.label(*action);
                                        ui.end_row();
                                    }
                                });
                        }
                    });
            });
    }
}

impl Dudel {
    fn paint_symmetry_guide(&self, painter: &egui::Painter, view: &view::View) {
        use crate::symmetry::Mode;
        let (w, h) = (self.doc.width as f32, self.doc.height as f32);
        let symmetry = self.active_symmetry();
        let [cx, cy] = symmetry.center(self.doc.width, self.doc.height);
        let guide = Color32::from_rgb(64, 200, 190);
        let reach = w.hypot(h);
        let mut lines: Vec<[Pos2; 2]> = vec![];
        for axis in symmetry.axes() {
            let (sin, cos) = axis.sin_cos();
            let tip = pos2(cx + cos * reach, cy + sin * reach);
            lines.push([pos2(cx, cy), tip]);
            // Mirror axes run through the center in both directions.
            if symmetry.mode != Mode::Radial {
                lines.push([pos2(cx, cy), pos2(cx - cos * reach, cy - sin * reach)]);
            }
        }
        let canvas = egui::Rect::from_min_size(Pos2::ZERO, vec2(w, h));
        for [a, b] in lines {
            // Clip spokes to the canvas so the guide never floats outside it.
            let b = clip_to(canvas, a, b);
            let points = [view.to_screen(a), view.to_screen(b)];
            painter.line_segment(points, Stroke::new(3.0_f32, Color32::from_black_alpha(70)));
            painter.extend(egui::Shape::dashed_line(
                &points,
                Stroke::new(1.5_f32, guide),
                8.0,
                5.0,
            ));
        }
        let center = view.to_screen(pos2(cx, cy));
        painter.circle_filled(center, 7.0, Color32::from_black_alpha(90));
        if self.guides_locked {
            // A plain dot: the handle cannot be dragged while guides are locked.
            painter.circle_filled(center, 3.0, guide);
        } else {
            painter.circle_stroke(center, 6.0, Stroke::new(2.0_f32, guide));
            painter.circle_filled(center, 2.0, guide);
        }
    }
}

/// Shorten segment a→b so b stays within `rect` (a is inside).
fn clip_to(rect: egui::Rect, a: Pos2, b: Pos2) -> Pos2 {
    let d = b - a;
    let mut t: f32 = 1.0;
    for (start, delta, low, high) in [
        (a.x, d.x, rect.min.x, rect.max.x),
        (a.y, d.y, rect.min.y, rect.max.y),
    ] {
        if delta > 0.0 {
            t = t.min((high - start) / delta);
        } else if delta < 0.0 {
            t = t.min((low - start) / delta);
        }
    }
    a + d * t.max(0.0)
}

fn drain_move_probe(app: &mut Dudel) -> Result<(f64, f64, usize), String> {
    let ctx = egui::Context::default();
    let wall = std::time::Instant::now();
    let mut maximum = 0.0_f64;
    let mut calls = 0;
    while app.render_queue.busy() {
        if app.render_queue.waiting_edit() {
            app.render_queue.wait_edit();
        }
        let started = std::time::Instant::now();
        app.pump_render_queue(&ctx);
        maximum = maximum.max(started.elapsed().as_secs_f64() * 1000.0);
        calls += 1;
        if wall.elapsed() > std::time::Duration::from_secs(30) {
            return Err("Move finish probe did not drain".into());
        }
    }
    Ok((maximum, wall.elapsed().as_secs_f64() * 1000.0, calls))
}
/// Actual application Move acquisition/publication/settlement; worker waits are
/// outside owner timers. No display, GPU, input driver or real frame scheduling.
pub(crate) fn measure_move_target(
    source: &crate::document::Document,
    offset: [i64; 2],
    target: crate::paint_target::Target,
    pending: bool,
) -> Result<
    (
        crate::document::Document,
        crate::document::History,
        serde_json::Value,
    ),
    String,
> {
    use std::time::Instant;
    let mut app = Dudel {
        doc: source.clone(),
        ..Default::default()
    };
    if let crate::paint_target::Target::Mask(id) = target {
        app.mask_focus = Some((source.layers[source.active].id, id));
    }
    let wall = Instant::now();
    let start = Instant::now();
    app.start_move_drag(Pos2::ZERO);
    let intent = app
        .pending_move
        .as_mut()
        .ok_or_else(|| app.status.clone())?;
    intent.offset = if pending { [0, 0] } else { offset };
    app.apply_move();
    let admission = start.elapsed().as_secs_f64() * 1000.0;
    let mut max_owner = admission;
    let mut calls = 1;
    loop {
        let drag = app.move_drag.as_mut().ok_or_else(|| app.status.clone())?;
        if drag.engine.finished() {
            break;
        }
        drag.engine.wait_worker();
        let start = Instant::now();
        app.apply_move();
        max_owner = max_owner.max(start.elapsed().as_secs_f64() * 1000.0);
        calls += 1;
        if wall.elapsed() > std::time::Duration::from_secs(30) {
            return Err("Mask Move preview timed out".into());
        }
    }
    let preview_wall = wall.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    if pending {
        app.move_drag.as_mut().unwrap().offset = offset;
        app.apply_move();
    }
    let final_request = start.elapsed().as_secs_f64() * 1000.0;
    let worker_pending = app
        .move_drag
        .as_ref()
        .is_some_and(|d| d.engine.waiting_worker());
    let start = Instant::now();
    app.finish_move_drag();
    let finish_admission = start.elapsed().as_secs_f64() * 1000.0;
    if app.move_drag.is_some() {
        return Err(app.status);
    }
    let (finish_max, finish_wall, finish_calls) = drain_move_probe(&mut app)?;
    if !app.modified || app.history.has_pending_stroke() {
        return Err(format!("Mask Move failed: {}", app.status));
    }
    Ok((
        app.doc,
        app.history,
        serde_json::json!({"preview_admission_ms":admission,"preview_max_owner_ms":max_owner,"preview_calls":calls,"preview_wall_ms":preview_wall,"final_request_ms":final_request,"worker_pending_at_finish":worker_pending,"finish_admission_ms":finish_admission,"finish_max_owner_ms":finish_max,"finish_wall_ms":finish_wall,"finish_owner_calls":finish_calls}),
    ))
}
pub(crate) fn benchmark_moves(path: &std::path::Path) -> Result<(), String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        for storage in ["sparse", "dense-flat", "dense-tiled"] {
            let mut source = crate::document::Document::new(size, size);
            {
                let mut writer = source.layers[0].pixels.writer();
                for y in 0..size {
                    for x in 0..size {
                        if storage != "sparse" || (x % 257 == 0 && y % 251 == 0) {
                            writer.rgba_mut(y * size + x).copy_from_slice(&[
                                (x % 251) as u8,
                                (y % 239) as u8,
                                ((x + y) % 233) as u8,
                                255,
                            ]);
                        }
                    }
                }
            }
            if storage == "dense-flat" {
                source.layers[0].pixels = source.layers[0].pixels.to_vec().into();
            }
            let rectangle = crate::selection::Selection {
                left: 0,
                top: 0,
                right: size / 2,
                bottom: size,
            };
            let mask = crate::selection_mask::Mask::rectangle(size, size, rectangle)?;
            source.selection = mask.bounds();
            source.selection_mask = Some(mask);
            let mut app = Dudel {
                doc: source.clone(),
                ..Default::default()
            };
            app.move_drag = Some(MoveDrag {
                start: Pos2::ZERO,
                engine: crate::move_preview::Engine::new(&source),
                checkpointed: false,
                modified_before: false,
                offset: [(size / 4) as i64, 0],
            });
            let wall = std::time::Instant::now();
            let started = std::time::Instant::now();
            app.apply_move();
            let admission_ms = started.elapsed().as_secs_f64() * 1000.0;
            if !crate::document::same_snapshot(&app.doc, &source) {
                return Err("Move admission published before the worker finished".into());
            }
            let latest = [(size / 8) as i64, 0];
            app.move_drag.as_mut().unwrap().offset = latest;
            let mut calls = 0;
            let mut max_owner_ms = 0.0_f64;
            while app
                .move_drag
                .as_ref()
                .is_some_and(|drag| drag.engine.applied != latest)
            {
                let started = std::time::Instant::now();
                app.apply_move();
                max_owner_ms = max_owner_ms.max(started.elapsed().as_secs_f64() * 1000.0);
                calls += 1;
                if wall.elapsed() > std::time::Duration::from_secs(30) {
                    return Err("Move preview probe did not finish".into());
                }
                if app.move_drag.is_none() {
                    return Err(app.status.clone());
                }
                if app.move_drag.as_ref().unwrap().engine.waiting() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;
            let started = std::time::Instant::now();
            app.finish_move_drag();
            let ready_finish_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (ready_owner, ready_wall, ready_calls) = drain_move_probe(&mut app)?;
            let expected = crate::transform::translated(
                &source.layers[0].pixels,
                size,
                size,
                rectangle,
                latest[0],
                0,
            );
            let moved = rectangle.offset(latest[0], 0, size, size).unwrap();
            let expected_mask = crate::selection_mask::Mask::rectangle(size, size, moved)?;
            if app.doc.layers[0].pixels != expected
                || app.doc.selection_mask.as_ref() != Some(&expected_mask)
            {
                return Err("Moved pixels/coverage differ from legacy binary rectangle".into());
            }
            if !app.history.undo(&mut app.doc)
                || app.doc.layers[0].pixels != source.layers[0].pixels
                || app.doc.selection_mask != source.selection_mask
            {
                return Err("Move undo differs".into());
            }
            if !app.history.redo(&mut app.doc)
                || app.doc.layers[0].pixels != expected
                || app.doc.selection_mask.as_ref() != Some(&expected_mask)
            {
                return Err("Move redo differs".into());
            }
            let mut fence = Dudel {
                doc: source.clone(),
                ..Default::default()
            };
            fence.move_drag = Some(MoveDrag {
                start: Pos2::ZERO,
                engine: crate::move_preview::Engine::new(&source),
                checkpointed: false,
                modified_before: false,
                offset: latest,
            });
            fence.apply_move();
            let started = std::time::Instant::now();
            fence.finish_move_drag();
            let pending_finish_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (pending_owner, pending_wall, pending_calls) = drain_move_probe(&mut fence)?;
            if fence.doc.layers[0].pixels != expected
                || fence.doc.selection_mask.as_ref() != Some(&expected_mask)
            {
                return Err("Pending move finish published a different position".into());
            }
            let mut deferred = Dudel {
                doc: source.clone(),
                ..Default::default()
            };
            deferred
                .render_queue
                .edit(crate::document_edits::Edit::MoveTarget {
                    target: source.layers[0].id,
                    offset: [7, 0],
                })?;
            let started = std::time::Instant::now();
            deferred.start_move_drag(Pos2::ZERO);
            deferred
                .pending_move
                .as_mut()
                .ok_or_else(|| deferred.status.clone())?
                .offset = [11, 0];
            deferred.apply_move();
            let contact_ms = started.elapsed().as_secs_f64() * 1000.0;
            if deferred.move_drag.is_some()
                || deferred.pending_move.is_none()
                || !crate::document::same_snapshot(&deferred.doc, &source)
            {
                return Err("Deferred contact acquired/mutated an unsettled source".into());
            }
            let started = std::time::Instant::now();
            deferred.finish_move_drag();
            let lift_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (deferred_owner, deferred_wall, deferred_calls) = drain_move_probe(&mut deferred)?;
            let intermediate =
                crate::transform::translated(&source.layers[0].pixels, size, size, rectangle, 7, 0);
            let intermediate_region = rectangle.offset(7, 0, size, size).unwrap();
            let expected_deferred =
                crate::transform::translated(&intermediate, size, size, intermediate_region, 11, 0);
            let final_mask = crate::selection_mask::Mask::rectangle(
                size,
                size,
                rectangle.offset(18, 0, size, size).unwrap(),
            )?;
            if deferred.doc.layers[0].pixels != expected_deferred
                || deferred.doc.selection_mask.as_ref() != Some(&final_mask)
                || !deferred.history.undo(&mut deferred.doc)
                || deferred.doc.layers[0].pixels != intermediate
                || !deferred.history.undo(&mut deferred.doc)
                || deferred.doc.layers[0].pixels != source.layers[0].pixels
                || deferred.doc.selection_mask != source.selection_mask
                || !deferred.history.redo(&mut deferred.doc)
                || !deferred.history.redo(&mut deferred.doc)
                || deferred.doc.layers[0].pixels != expected_deferred
                || deferred.doc.selection_mask.as_ref() != Some(&final_mask)
            {
                return Err("Deferred Move geometry/order/undo/redo differs".into());
            }
            cases.push(serde_json::json!({"size":size,"storage":storage,"admission_ms":admission_ms,"max_preview_owner_ms":max_owner_ms,"owner_calls":calls,"coalesced_preview_wall_ms":wall_ms,"ready_finish_admission_ms":ready_finish_ms,"ready_finish_owner_max_ms":ready_owner,"ready_finish_wall_ms":ready_wall,"ready_finish_owner_calls":ready_calls,"pending_finish_admission_ms":pending_finish_ms,"pending_finish_owner_max_ms":pending_owner,"pending_finish_wall_ms":pending_wall,"pending_finish_owner_calls":pending_calls,"deferred_contact_ms":contact_ms,"deferred_lift_ms":lift_ms,"deferred_drain_owner_max_ms":deferred_owner,"deferred_drain_wall_ms":deferred_wall,"deferred_drain_owner_calls":deferred_calls,"deferred_order_undo_redo_verified":true,"legacy_rgba_coverage_undo_redo_verified":true,"pending_finish_position_verified":true}));
        }
    }
    crate::storage::atomic_write(path,&serde_json::to_vec_pretty(&serde_json::json!({"version":3,"scope":"One repetition per size/storage. Actual application Move admission/coalesced preview publication and finish handoff measured. Deferred contact/lift follow an unstarted queued integer Move; timed drain includes both ordered Moves, with exact legacy geometry and two separate undo/redo actions verified. Gated already-running predecessor/cancel/save cases are tested separately. Finish owner calls use the real controller pump with a 2 ms yield deadline, including ready publication, initial checkpoint/revision invalidation, sliced history settlement and completion feedback. Headless joins are outside owner timing, inside wall time. Preview 1 ms sleeps likewise excluded from owner timing. Binary half-canvas masks, sparse/opaque patterned dense sources. Exact RGBA/coverage/undo/redo and pending finish final position verified outside timing. Excludes setup/verification, GUI frames, contours, display/GPU and physical input. Memory/RSS are unmeasured. The original fixture remains retained during timing, so final release of otherwise unshared original source buffers is not measured. Fractional/hidden-RGB/cancel/ordering tests are separate. Document-replacement fences can still join through the shared flush helper. Move contact defers source acquisition and lift queues its move behind earlier artwork.","cases":cases})).map_err(|e|e.to_string())?)
}

#[cfg(test)]
mod cancellation_tests {
    #[test]
    fn painting_contacts_cross_page_edges_and_start_in_the_surround_without_resetting() {
        use crate::{
            brush::{Brush, StabilizerMode},
            input::Packet,
            stroke::Sample,
        };
        for rope in [false, true] {
            for vertical in [false, true] {
                for rotated in [false, true] {
                    let ctx = egui::Context::default();
                    let mut app = Dudel {
                        doc: crate::document::Document::new(32, 32),
                        brush: Brush {
                            size: 2.0,
                            hardness: 1.0,
                            pressure_size: false,
                            stabilization: 0.0,
                            stabilizer_mode: if rope {
                                StabilizerMode::Rope
                            } else {
                                StabilizerMode::Smooth
                            },
                            rope_distance: Some(8.0),
                            ..Brush::default()
                        },
                        zoom: 4.0,
                        auto_fit: false,
                        rotation: if rotated { 0.45 } else { 0.0 },
                        mirror: rotated,
                        ..Default::default()
                    };
                    let raw = || egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        ..Default::default()
                    };
                    let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
                    for (index, v) in [-8.0, 8.0, 40.0, 24.0, -8.0].into_iter().enumerate() {
                        let point = if vertical {
                            egui::pos2(16.0, v)
                        } else {
                            egui::pos2(v, 16.0)
                        };
                        let screen = app.view(app.canvas_area).to_screen(point);
                        let sample = Sample {
                            x: screen.x,
                            y: screen.y,
                            pressure: 1.0,
                            time: Some(index as f64 * 0.1),
                            ..Default::default()
                        };
                        let packet = if index == 0 {
                            Packet::Start(sample)
                        } else if index == 4 {
                            Packet::End(sample)
                        } else {
                            Packet::Move(sample)
                        };
                        let mut input = raw();
                        input.events.push(egui::Event::PointerMoved(screen));
                        let _ = ctx.run(input, |ctx| app.canvas(ctx, &[packet]));
                        if index < 4 {
                            assert!(app.stroking);
                        }
                        if rope && index == 2 {
                            assert!(
                                app.stroke.rope(&app.brush).is_some(),
                                "leaving page must not reset rope"
                            );
                        }
                    }
                    app.flush_render_queue();
                    assert!(!app.stroking);
                    for v in 0..32 {
                        let index = if vertical { v * 32 + 16 } else { 16 * 32 + v };
                        assert!(
                            app.doc.layers[0].pixels.rgba(index)[3] > 0,
                            "missing edge/path pixel {v}, rope={rope}, vertical={vertical}, rotated={rotated}"
                        );
                    }
                    let painted = app.doc.layers[0].pixels.clone();
                    app.undo();
                    assert!(!app.history.can_undo(), "one contact is one undo");
                    assert!((0..32 * 32).all(|i| app.doc.layers[0].pixels.rgba(i)[3] == 0));
                    app.redo();
                    assert_eq!(app.doc.layers[0].pixels, painted);
                }
            }
        }
    }

    #[test]
    fn move_preview_cancel_and_zero_return_preserve_guide_and_following_edit_revisions() {
        for masked in [false, true] {
            for outcome in 0..3 {
                for guide_edit in [false, true] {
                    let mut app = Dudel {
                        doc: crate::document::Document::new(8, 6),
                        ..Default::default()
                    };
                    app.doc.layers[0]
                        .pixels
                        .writer()
                        .rgba_mut(10)
                        .copy_from_slice(&[17, 83, 201, 255]);
                    if masked {
                        app.doc.version = 2;
                        let id = crate::document::LayerId::new();
                        app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
                            id,
                            enabled: true,
                            linked: false,
                            coverage: crate::layer_mask::Mask::new(8, 6, 0).unwrap(),
                        });
                        app.mask_focus = Some((app.doc.layers[0].id, id));
                    }
                    app.saved_content_revision = Some(0);
                    let source = app.doc.clone();
                    app.start_move_drag(egui::Pos2::ZERO);
                    app.pending_move.as_mut().unwrap().offset = [1, 0];
                    let started = std::time::Instant::now();
                    while crate::document::same_snapshot(&app.doc, &source) {
                        app.apply_move();
                        assert!(started.elapsed() < std::time::Duration::from_secs(5));
                        std::thread::yield_now();
                    }
                    assert!(app.geometry_preview.is_some() && app.modified);
                    assert_eq!(app.content_revision, 0);
                    if guide_edit {
                        app.grid.visible = true;
                        app.track_guide_changes();
                    }
                    match outcome {
                        0 => app.cancel_move_drag(),
                        1 => {
                            app.finish_move_drag();
                            assert!(app.render_queue.cancel_background_edit());
                        }
                        _ => {
                            app.move_drag.as_mut().unwrap().offset = [0, 0];
                            app.finish_move_drag();
                        }
                    }
                    // A later ordered edit must not be absorbed into rollback.
                    if outcome == 1 {
                        app.render_queue
                            .edit(crate::document_edits::Edit::Property {
                                target: crate::document_edits::Target::Active,
                                property: crate::document_edits::Property::Name(
                                    "Later edit".into(),
                                ),
                            })
                            .unwrap();
                    }
                    app.flush_render_queue();
                    assert!(app.geometry_preview.is_none());
                    assert_eq!(
                        app.content_revision,
                        u64::from(guide_edit) + u64::from(outcome == 1)
                    );
                    assert_eq!(app.modified, guide_edit || outcome == 1);
                    assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
                    assert!(
                        crate::layer_mask::equivalent_state(
                            &app.doc.layers[0].mask,
                            &source.layers[0].mask,
                            None
                        )
                        .unwrap()
                    );
                    assert_eq!(app.history.can_undo(), outcome == 1);
                    assert_eq!(app.grid.visible, guide_edit);
                }
            }
        }
    }
    #[test]
    fn selected_mask_move_gesture_cancels_or_commits_without_moving_pixels() {
        for cancel in [false, true] {
            let mut app = Dudel {
                doc: crate::document::Document::new(4, 1),
                ..Default::default()
            };
            app.doc.version = 2;
            app.doc.layers[0].pixels = [17, 83, 201, 127].repeat(4).into();
            app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
                id: crate::document::LayerId::new(),
                enabled: true,
                linked: false,
                coverage: crate::layer_mask::Mask::from_coverage(4, 1, &[0, 64, 128, 255], None)
                    .unwrap(),
            });
            app.doc.layers[0].alpha_locked = true;
            let mask = app.doc.layers[0].mask.as_ref().unwrap().id;
            app.mask_focus = Some((app.doc.layers[0].id, mask));
            let original = app.doc.clone();
            app.start_move_drag(egui::Pos2::ZERO);
            assert!(app.pending_move.is_some(), "{}", app.status);
            app.pending_move.as_mut().unwrap().offset = [1, 0];
            if cancel {
                let start = std::time::Instant::now();
                while crate::document::same_snapshot(&app.doc, &original) {
                    app.apply_move();
                    assert!(start.elapsed() < std::time::Duration::from_secs(5));
                    std::thread::yield_now();
                }
                app.cancel_move_drag();
                assert!(crate::document::same_snapshot(&app.doc, &original));
                assert!(!app.history.can_undo());
            } else {
                // Release before source acquisition exercises the deferred target.
                app.finish_move_drag();
                crate::workflow_tests::finish_jobs(&mut app);
                assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
                assert_eq!(
                    app.doc.layers[0].mask.as_ref().unwrap().coverage.at(1, 0),
                    original.layers[0].mask.as_ref().unwrap().coverage.at(0, 0)
                );
                assert!(app.history.undo(&mut app.doc));
                assert!(crate::document::same_snapshot(&app.doc, &original));
                assert!(!app.history.can_undo());
            }
        }
    }
    use crate::{Dudel, egui};
    #[test]
    fn cancel_after_lift_restores_published_preview_without_waiting_for_final_worker() {
        let mut app = moving_mask_app();
        let original = app.doc.clone();
        app.move_drag.as_mut().unwrap().offset = [1, 0];
        app.apply_move();
        let candidate = app
            .move_drag
            .as_mut()
            .unwrap()
            .engine
            .finish(&app.doc)
            .unwrap()
            .unwrap();
        app.publish_move_candidate(candidate);
        assert!(app.modified && app.history.has_pending_stroke());
        let drag = app.move_drag.as_mut().unwrap();
        drag.offset = [2, 0];
        let release = drag.engine.gate([2, 0]);
        app.finish_move_drag();
        assert!(app.render_queue.cancel_background_edit());
        app.flush_render_queue();
        assert!(!app.modified && !app.render_queue.busy() && !app.history.has_pending_stroke());
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, original.selection_mask);
        assert!(app.history.can_redo() && !app.history.can_undo());
        release.send(()).unwrap();
    }
    #[test]
    fn stroke_queued_after_gated_move_uses_final_mask_and_undo_stays_separate() {
        let mut app = moving_mask_app();
        let original = app.doc.clone();
        let (engine, release) = crate::move_preview::Engine::gated(&app.doc, [2, 0]);
        let drag = app.move_drag.as_mut().unwrap();
        drag.engine = engine;
        drag.offset = [2, 0];
        app.finish_move_drag();
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 1.0,
                pressure_size: false,
                hardness: 1.0,
                ..Default::default()
            },
            color: [0, 255, 0],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let dabs = vec![
            crate::stroke::Sample {
                x: 0.5,
                y: 0.5,
                pressure: 1.0,
                ..Default::default()
            },
            crate::stroke::Sample {
                x: 2.5,
                y: 0.5,
                pressure: 1.0,
                ..Default::default()
            },
        ];
        assert!(app.begin_canvas_stroke(settings.clone()));
        app.deliver_canvas_dabs(dabs.clone());
        app.finish_painted_stroke();
        assert!(crate::document::same_snapshot(&app.doc, &original));
        release.send(()).unwrap();
        app.flush_render_queue();
        let mut expected = original.clone();
        expected.layers[0].pixels =
            vec![255, 0, 0, 127, 0, 0, 255, 255, 255, 0, 0, 128, 0, 0, 0, 0].into();
        let mask = crate::selection_mask::Mask::from_coverage(4, 1, vec![0, 0, 128, 0]).unwrap();
        expected.selection = mask.bounds();
        expected.selection_mask = Some(mask);
        let moved = expected.clone();
        let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
        for dab in dabs {
            buffer.paint(&mut expected, dab, &settings);
        }
        assert_ne!(
            expected.layers[0].pixels.rgba(2),
            moved.layers[0].pixels.rgba(2)
        );
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, expected.selection_mask);
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [255, 0, 0, 127]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, moved.layers[0].pixels);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, original.selection_mask);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn save_admits_snapshot_without_joining_gated_move_and_writes_final_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("finished-move.dudel");
        let mut app = moving_mask_app();
        app.path = Some(path.clone());
        let (engine, release) = crate::move_preview::Engine::gated(&app.doc, [2, 0]);
        let drag = app.move_drag.as_mut().unwrap();
        drag.engine = engine;
        drag.offset = [2, 0];
        app.finish_move_drag();
        app.save(false);
        assert!(
            app.queued_save.is_some() && app.save_job.is_none() && app.render_queue.waiting_edit()
        );
        release.send(()).unwrap();
        app.flush_render_queue();
        let job = app.save_job.take().unwrap();
        job.worker.join().unwrap().unwrap();
        let saved = crate::storage::load_project(&path).unwrap();
        assert_eq!(saved.layers[0].pixels, app.doc.layers[0].pixels);
        assert_eq!(saved.selection_mask, app.doc.selection_mask);
        assert_eq!(saved.layers[0].pixels.rgba(2), [255, 0, 0, 128]);
        assert_eq!(saved.selection_mask.as_ref().unwrap().at(2, 0), 128);
    }
    #[test]
    fn gated_finish_returns_then_selection_and_snapshot_wait_for_final_position() {
        let mut app = moving_mask_app();
        let original = app.doc.clone();
        let (engine, release) = crate::move_preview::Engine::gated(&app.doc, [2, 0]);
        let drag = app.move_drag.as_mut().unwrap();
        drag.engine = engine;
        drag.offset = [2, 0];
        app.finish_move_drag();
        assert!(app.move_drag.is_none() && app.render_queue.waiting_edit());
        assert!(crate::document::same_snapshot(&app.doc, &original));
        app.set_selection(None);
        let ticket = app.render_queue.snapshot().unwrap();
        let progress = app
            .render_queue
            .advance(&mut app.doc, &mut app.history, 1)
            .unwrap();
        assert_eq!(progress.work_units, 0);
        assert!(progress.snapshot.is_none());
        release.send(()).unwrap();
        let mut saved = None;
        while app.render_queue.busy() {
            if app.render_queue.waiting_edit() {
                app.render_queue.wait_edit();
            }
            let progress = app
                .render_queue
                .advance(&mut app.doc, &mut app.history, 1)
                .unwrap();
            if let Some(snapshot) = progress.snapshot {
                saved = Some(snapshot);
            }
        }
        let (id, saved) = saved.unwrap();
        assert_eq!(id, ticket);
        assert!(saved.selection_mask.is_none());
        assert_eq!(saved.layers[0].pixels.rgba(2), [255, 0, 0, 128]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(2, 0), 128);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, original.selection_mask);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn canceled_queued_move_finishes_before_later_edit_and_restores_unsaved_state() {
        let mut app = moving_mask_app();
        let original = app.doc.clone();
        let (engine, release) = crate::move_preview::Engine::gated(&app.doc, [1, 0]);
        let drag = app.move_drag.as_mut().unwrap();
        drag.engine = engine;
        drag.offset = [1, 0];
        app.finish_move_drag();
        assert!(app.render_queue.cancel_background_edit());
        app.set_selection(None);
        release.send(()).unwrap();
        app.flush_render_queue();
        assert!(
            app.modified,
            "The following deselect action must keep the document unsaved"
        );
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert!(app.doc.selection_mask.is_none());
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.selection_mask, original.selection_mask);
        assert!(!app.history.can_undo());
    }
    fn moving_mask_app() -> crate::Dudel {
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(4, 1),
            ..Default::default()
        };
        app.doc.layers[0].pixels =
            vec![255, 0, 0, 255, 0, 0, 255, 255, 9, 11, 13, 0, 0, 0, 0, 0].into();
        let mask = crate::selection_mask::Mask::from_coverage(4, 1, vec![128, 0, 0, 0]).unwrap();
        app.doc.selection = mask.bounds();
        app.doc.selection_mask = Some(mask);
        app.history.checkpoint(&app.doc);
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(3)
            .copy_from_slice(&[7, 5, 3, 255]);
        app.history.settle(&app.doc);
        assert!(app.history.undo(&mut app.doc));
        app.move_drag = Some(super::MoveDrag {
            start: crate::egui::pos2(0.0, 0.0),
            engine: crate::move_preview::Engine::new(&app.doc),
            checkpointed: false,
            modified_before: app.modified,
            offset: [0, 0],
        });
        app
    }
    #[test]
    fn masked_move_repeated_return_to_zero_commits_one_action_and_restores_source_on_undo() {
        let mut app = moving_mask_app();
        let original = app.doc.clone();
        for offset in [[1, 0], [0, 0], [2, 0], [0, 0], [1, 0]] {
            app.move_drag.as_mut().unwrap().offset = offset;
            app.apply_move();
        }
        app.finish_move_drag();
        app.flush_render_queue();
        assert!(!app.history.has_pending_stroke());
        assert_eq!(app.doc.layers[0].pixels.rgba(0), [255, 0, 0, 127]);
        assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(1, 0), 128);
        let after = app.doc.clone();
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, original.selection_mask);
        assert!(!app.history.can_undo());
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, after.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, after.selection_mask);
    }
    #[test]
    fn canceled_or_zero_return_masked_move_preserves_redo_and_original_hidden_rgb() {
        for cancel in [false, true] {
            let mut app = moving_mask_app();
            let original = app.doc.clone();
            app.move_drag.as_mut().unwrap().offset = [1, 0];
            app.apply_move();
            let candidate = app
                .move_drag
                .as_mut()
                .unwrap()
                .engine
                .finish(&app.doc)
                .unwrap()
                .unwrap();
            app.publish_move_candidate(candidate);
            assert!(app.history.has_pending_stroke() && app.modified);
            if cancel {
                app.cancel_move_drag();
            } else {
                app.move_drag.as_mut().unwrap().offset = [0, 0];
                app.apply_move();
                app.finish_move_drag();
                app.flush_render_queue();
            }
            assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
            assert_eq!(app.doc.selection_mask, original.selection_mask);
            assert!(!app.history.has_pending_stroke());
            assert!(!app.history.can_undo());
            assert!(!app.modified);
            assert!(app.history.redo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels.rgba(3), [7, 5, 3, 255]);
        }
    }
    #[test]
    fn canvas_paints_queued_duplicate_with_its_own_symmetry() {
        use crate::{document::Document, input::Packet, stroke::Sample, symmetry::Mode};
        let ctx = egui::Context::default();
        let mut app = Dudel {
            doc: Document::new(32, 32),
            tool: crate::tools::Tool::Brush,
            new_canvas: false,
            ..Default::default()
        };
        app.brush.size = 1.0;
        app.brush.pressure_size = false;
        app.brush.hardness = 1.0;
        app.layer_symmetry.insert(
            app.doc.layers[0].id,
            crate::symmetry::Symmetry {
                mode: Mode::LeftRight,
                ..Default::default()
            },
        );
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        app.render_queue.redo().unwrap();
        app.duplicate_layer();
        assert_eq!(app.doc.layers.len(), 1);
        assert!(app.input_layer().is_none());
        let p = app.view(app.canvas_area).to_screen(egui::pos2(8.5, 10.5));
        let sample = Sample {
            x: p.x,
            y: p.y,
            pressure: 1.0,
            ..Default::default()
        };
        let mut input = raw();
        input.events.push(egui::Event::PointerMoved(p));
        let _ = ctx.run(input, |ctx| {
            app.canvas(ctx, &[Packet::Start(sample), Packet::End(sample)]);
        });
        app.flush_render_queue();
        assert_eq!(app.doc.layers.len(), 2);
        let pixels = &app.doc.layers[1].pixels;
        assert!(pixels.rgba(10 * 32 + 8)[3] > 0);
        assert_eq!(pixels.rgba(10 * 32 + 8), pixels.rgba(10 * 32 + 23));
        assert!(app.doc.layers[0].pixels.iter().all(|byte| *byte == 0));
        app.undo();
        assert!(app.doc.layers[1].pixels.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn canvas_move_packets_publish_final_offset_and_cancel_a_published_preview() {
        use crate::{egui, input::Packet, stroke::Sample, tools::Tool};
        for cancel in [false, true] {
            let ctx = egui::Context::default();
            let mut app = moving_mask_app();
            app.tool = Tool::Move;
            let original = app.doc.clone();
            let raw = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            let samples: Vec<Sample> = [0.5_f32, 1.5, 2.5]
                .into_iter()
                .map(|x| {
                    let p = app.view(app.canvas_area).to_screen(egui::pos2(x, 0.5));
                    Sample {
                        x: p.x,
                        y: p.y,
                        pressure: 1.0,
                        ..Default::default()
                    }
                })
                .collect();
            let mut input = raw();
            input.events.push(egui::Event::PointerMoved(egui::pos2(
                samples[0].x,
                samples[0].y,
            )));
            let _ = ctx.run(input, |ctx| {
                app.canvas(ctx, &[Packet::Start(samples[0]), Packet::Move(samples[1])])
            });
            let candidate = app
                .move_drag
                .as_mut()
                .unwrap()
                .engine
                .finish(&app.doc)
                .unwrap()
                .unwrap();
            app.publish_move_candidate(candidate);
            assert!(app.history.has_pending_stroke() && app.modified);
            let _ = ctx.run(raw(), |ctx| {
                app.canvas(
                    ctx,
                    &[if cancel {
                        Packet::Cancel
                    } else {
                        Packet::End(samples[2])
                    }],
                )
            });
            app.flush_render_queue();
            assert!(app.move_drag.is_none() && !app.history.has_pending_stroke());
            if cancel {
                assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
                assert_eq!(app.doc.selection_mask, original.selection_mask);
                assert!(!app.modified && app.history.can_redo() && !app.history.can_undo());
            } else {
                assert_eq!(app.doc.layers[0].pixels.rgba(2), [255, 0, 0, 128]);
                assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(2, 0), 128);
                assert!(app.history.undo(&mut app.doc));
                assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
                assert_eq!(app.doc.selection_mask, original.selection_mask);
                assert!(!app.history.can_undo());
            }
        }
    }
    fn deferred_move_app() -> (
        Dudel,
        std::sync::mpsc::Sender<()>,
        crate::document::Document,
    ) {
        let mut app = Dudel {
            doc: crate::document::Document::new(5, 1),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[200, 30, 70, 255]);
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(4)
            .copy_from_slice(&[7, 11, 13, 0]);
        let mask = crate::selection_mask::Mask::from_coverage(5, 1, vec![255, 0, 0, 0, 0]).unwrap();
        app.doc.selection = mask.bounds();
        app.doc.selection_mask = Some(mask);
        let source = app.doc.clone();
        let (engine, send) = crate::move_preview::Engine::gated(&app.doc, [1, 0]);
        app.render_queue
            .finish_move(engine, false, false)
            .unwrap_or_else(|_| panic!("exclusive finish"));
        (app, send, source)
    }
    #[test]
    fn move_contact_defers_source_until_preceding_worker_and_cancellation_keeps_earlier_artwork() {
        for acquired in [false, true] {
            let (mut app, send, source) = deferred_move_app();
            app.start_move_drag(egui::pos2(0.5, 0.5));
            app.pending_move.as_mut().unwrap().offset = [2, 0];
            app.apply_move();
            assert!(app.pending_move.is_some() && app.move_drag.is_none());
            assert!(crate::document::same_snapshot(&app.doc, &source));
            if acquired {
                send.send(()).unwrap();
                app.flush_render_queue();
                app.apply_move();
                let drag = app.move_drag.as_ref().unwrap();
                assert!(drag.modified_before);
                assert_eq!(
                    drag.engine.original().layers[0].pixels.rgba(1),
                    [200, 30, 70, 255]
                );
                assert!(app.pending_move.is_none());
                app.cancel_move_drag();
            } else {
                app.cancel_move_drag();
                send.send(()).unwrap();
                app.flush_render_queue();
            }
            assert!(app.modified && app.pending_move.is_none() && app.move_drag.is_none());
            assert_eq!(app.doc.layers[0].pixels.rgba(1), [200, 30, 70, 255]);
            assert!(app.history.undo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
            assert_eq!(app.doc.selection_mask, source.selection_mask);
            assert!(!app.history.can_undo());
        }
    }
    #[test]
    fn released_deferred_move_runs_after_prior_work_and_before_save_with_separate_undo() {
        let (mut app, send, source) = deferred_move_app();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deferred-move.dudel");
        app.path = Some(path.clone());
        app.start_move_drag(egui::pos2(0.5, 0.5));
        app.pending_move.as_mut().unwrap().offset = [2, 0];
        app.apply_move();
        app.finish_move_drag();
        assert!(app.pending_move.is_none() && app.move_drag.is_none());
        app.save(false);
        assert!(app.queued_save.is_some() && app.save_job.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &source));
        send.send(()).unwrap();
        app.flush_render_queue();
        app.save_job.take().unwrap().worker.join().unwrap().unwrap();
        let saved = crate::storage::load_project(&path).unwrap();
        assert_eq!(saved.layers[0].pixels.rgba(3), [200, 30, 70, 255]);
        assert_eq!(saved.layers[0].pixels.rgba(4), [7, 11, 13, 0]);
        assert_eq!(saved.selection_mask.as_ref().unwrap().at(3, 0), 255);
        assert_eq!(saved.layers[0].pixels, app.doc.layers[0].pixels);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels.rgba(1), [200, 30, 70, 255]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, source.selection_mask);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn deferred_move_revalidates_lock_at_acquisition_or_execution() {
        for released in [false, true] {
            let (mut app, send, _) = deferred_move_app();
            app.render_queue
                .edit(crate::document_edits::Edit::Property {
                    target: crate::document_edits::Target::Active,
                    property: crate::document_edits::Property::AlphaLocked(true),
                })
                .unwrap();
            app.start_move_drag(egui::pos2(0.5, 0.5));
            app.pending_move.as_mut().unwrap().offset = [2, 0];
            if released {
                app.finish_move_drag();
            }
            send.send(()).unwrap();
            app.flush_render_queue();
            if !released {
                app.apply_move();
            }
            assert!(app.pending_move.is_none() && app.move_drag.is_none());
            assert!(app.doc.layers[0].alpha_locked);
            assert_eq!(app.doc.layers[0].pixels.rgba(1), [200, 30, 70, 255]);
            assert_eq!(app.doc.layers[0].pixels.rgba(3), [0; 4]);
            assert!(app.status.contains("transparency"));
            assert!(app.history.undo(&mut app.doc)); // lock
            assert!(app.history.undo(&mut app.doc)); // prior move
            assert!(!app.history.can_undo());
        }
    }
    #[test]
    fn native_move_packets_can_lift_while_preceding_worker_is_gated() {
        use crate::{input::Packet, stroke::Sample, tools::Tool};
        let (mut app, send, source) = deferred_move_app();
        app.tool = Tool::Move;
        app.zoom = 24.0;
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        let view = app.view(app.canvas_area);
        let sample = |x| {
            let p = view.to_screen(egui::pos2(x, 0.5));
            Sample {
                x: p.x,
                y: p.y,
                pressure: 1.0,
                ..Default::default()
            }
        };
        let first = sample(0.5);
        let last = sample(2.5);
        let mut input = raw();
        input
            .events
            .push(egui::Event::PointerMoved(egui::pos2(first.x, first.y)));
        let _ = ctx.run(input, |ctx| {
            app.canvas(
                ctx,
                &[Packet::Start(first), Packet::Move(last), Packet::End(last)],
            )
        });
        assert!(
            app.pending_move.is_none()
                && app.move_drag.is_none()
                && app.render_queue.waiting_edit()
        );
        assert!(crate::document::same_snapshot(&app.doc, &source));
        send.send(()).unwrap();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels.rgba(3), [200, 30, 70, 255]);
        assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(3, 0), 255);
    }
    #[test]
    fn deferred_move_rejects_captured_target_after_preceding_layer_creation() {
        for released in [false, true] {
            let (mut app, send, _) = deferred_move_app();
            app.render_queue
                .edit(crate::document_edits::Edit::NewLayer)
                .unwrap();
            app.start_move_drag(egui::pos2(0.5, 0.5));
            app.pending_move.as_mut().unwrap().offset = [2, 0];
            if released {
                app.finish_move_drag();
            }
            send.send(()).unwrap();
            app.flush_render_queue();
            if !released {
                app.apply_move();
            }
            assert!(app.pending_move.is_none() && app.move_drag.is_none());
            assert_eq!(app.doc.active, 1);
            assert_eq!(app.doc.layers[0].pixels.rgba(1), [200, 30, 70, 255]);
            assert!(app.doc.layers[1].pixels.iter().all(|b| *b == 0));
            assert!(app.status.contains("active"));
            assert!(app.history.undo(&mut app.doc));
            assert!(app.history.undo(&mut app.doc));
            assert!(!app.history.can_undo());
        }
    }
    #[test]
    fn canceling_released_deferred_move_cancels_its_ticket_and_preserves_prior_owner() {
        let (mut app, send, source) = deferred_move_app();
        app.start_move_drag(egui::pos2(0.5, 0.5));
        app.pending_move.as_mut().unwrap().offset = [2, 0];
        app.finish_move_drag();
        let ticket = app.deferred_move_ticket.unwrap();
        app.cancel_move_drag();
        assert!(app.deferred_move_ticket.is_none());
        assert!(app.render_queue.waiting_edit()); // Earlier gated finish survives.
        assert!(!app.render_queue.cancel_edit(ticket)); // Already canceled descriptor.
        send.send(()).unwrap();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels.rgba(1), [200, 30, 70, 255]);
        assert_eq!(app.doc.layers[0].pixels.rgba(3), [0; 4]);
        assert!(app.modified && app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn full_queue_retries_released_move_as_an_edit_without_reopening_preview() {
        let mut app = Dudel {
            doc: crate::document::Document::new(3, 1),
            ..Default::default()
        };
        app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[200, 30, 70, 255]);
        let source = app.doc.clone();
        let mut queued = 0;
        while app
            .render_queue
            .edit(crate::document_edits::Edit::SelectLayer(0))
            .is_ok()
        {
            queued += 1;
            assert!(queued <= 4096);
        }
        app.start_move_drag(egui::pos2(0.5, 0.5));
        app.pending_move.as_mut().unwrap().offset = [1, 0];
        app.finish_move_drag();
        assert!(app.pending_move.as_ref().unwrap().released);
        app.start_move_drag(egui::pos2(2.5, 0.5));
        assert_eq!(app.pending_move.as_ref().unwrap().offset, [1, 0]);
        assert!(app.move_drag.is_none() && app.deferred_move_ticket.is_none());
        app.flush_render_queue();
        app.apply_move();
        assert!(
            app.pending_move.is_none()
                && app.move_drag.is_none()
                && app.deferred_move_ticket.is_some()
        );
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels.rgba(1), [200, 30, 70, 255]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn canceling_a_later_brush_contact_does_not_cancel_an_earlier_queued_move() {
        use crate::{input::Packet, tools::Tool};
        let (mut app, send, _) = deferred_move_app();
        app.start_move_drag(egui::pos2(0.5, 0.5));
        app.pending_move.as_mut().unwrap().offset = [2, 0];
        app.finish_move_drag();
        let ticket = app.deferred_move_ticket;
        app.tool = Tool::Brush;
        assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: crate::brush::Brush {
                size: 1.0,
                pressure_size: false,
                ..Default::default()
            },
            color: [10, 20, 30],
            erase: false,
            layer: 0,
            symmetry: Default::default()
        }));
        app.deliver_canvas_dabs(vec![crate::stroke::Sample {
            x: 3.5,
            y: 0.5,
            pressure: 1.0,
            ..Default::default()
        }]);
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| app.canvas(ctx, &[Packet::Cancel]));
        assert_eq!(app.deferred_move_ticket, ticket);
        assert!(!app.stroking && app.queued_stroke.is_none());
        send.send(()).unwrap();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels.rgba(3), [200, 30, 70, 255]);
        assert!(app.history.undo(&mut app.doc));
        assert!(app.history.undo(&mut app.doc));
        assert!(!app.history.can_undo());
    }
    #[test]
    fn following_selection_edit_waits_for_latest_move_and_has_separate_undo() {
        let mut app = moving_mask_app();
        let original = app.doc.clone();
        app.move_drag.as_mut().unwrap().offset = [1, 0];
        app.apply_move();
        let candidate = app
            .move_drag
            .as_mut()
            .unwrap()
            .engine
            .finish(&app.doc)
            .unwrap()
            .unwrap();
        app.publish_move_candidate(candidate);
        app.move_drag.as_mut().unwrap().offset = [2, 0];
        app.apply_move();
        app.set_selection(None);
        app.flush_render_queue();
        assert!(app.move_drag.is_none() && app.doc.selection_mask.is_none());
        assert_eq!(app.doc.layers[0].pixels.rgba(2), [255, 0, 0, 128]);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(2, 0), 128);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, original.selection_mask);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn tail_finish_updates_revision_dirty_region_and_commits_history_once() {
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(33, 35),
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
        app.history.begin_stroke_checkpoint(&app.doc);
        app.stroking = true;
        app.stroke_buffer = Some(Default::default());
        app.stroke_settings = Some(settings.clone());
        app.stroke_buffer.as_mut().unwrap().paint(
            &mut app.doc,
            crate::stroke::Sample {
                x: 16.5,
                y: 17.5,
                pressure: 1.0,
                ..Default::default()
            },
            &settings,
        );
        let preview = app.doc.layers[0].pixels.clone();
        app.modified = false;
        app.dirty_region = Default::default();
        let revision = app.revision;
        app.finish_painted_stroke();
        assert!(app.modified);
        assert!(!app.dirty_region.is_empty());
        assert_ne!(app.revision, revision);
        let final_pixels = app.doc.layers[0].pixels.clone();
        assert_ne!(final_pixels, preview);
        let revision = app.revision;
        app.finish_painted_stroke();
        assert_eq!(app.revision, revision);
        assert!(app.history.undo(&mut app.doc));
        assert!(!app.history.can_undo());
        assert!(app.doc.layers[0].pixels.iter().all(|&p| p == 0));
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, final_pixels);
    }
    #[test]
    fn finished_eraser_reclaims_zero_tiles_and_preserves_hidden_rgb() {
        for color in [[0, 0, 0], [10, 20, 30]] {
            let mut app = crate::Dudel {
                doc: crate::document::Document::new(517, 259),
                ..crate::Dudel::default()
            };
            let index = 255 * 517 + 255;
            app.doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(index)
                .copy_from_slice(&[color[0], color[1], color[2], 255]);
            app.history.begin_stroke_checkpoint(&app.doc);
            app.stroking = true;
            app.stroke_buffer = Some(Default::default());
            let brush = crate::brush::Brush {
                size: 12.0,
                hardness: 1.0,
                pressure_size: false,
                ..crate::brush::Brush::default()
            };
            app.doc.dab_buffered(
                255.5,
                255.5,
                1.0,
                &brush,
                [0; 3],
                true,
                crate::symmetry::IDENTITY,
                app.stroke_buffer.as_mut(),
            );
            app.finish_painted_stroke();
            let expected_capacity = if color == [0; 3] {
                0
            } else {
                crate::pixels::SIDE * crate::pixels::SIDE * 4
            };
            assert_eq!(app.doc.layers[0].pixels.capacity(), expected_capacity);
            assert!(app.history.undo(&mut app.doc));
            assert_eq!(
                app.doc.layers[0].pixels.rgba(index),
                [color[0], color[1], color[2], 255]
            );
            assert!(app.history.redo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels.capacity(), expected_capacity);
            assert_eq!(
                app.doc.layers[0].pixels.rgba(index),
                [color[0], color[1], color[2], 0]
            );
        }
    }
    #[test]
    fn cancel_blank_stroke_releases_sparse_tiles_and_refreshes_history() {
        super::Dudel::test_cancel_blank_sparse_stroke();
    }
    #[test]
    fn repeated_gesture_cleanup_commits_partial_stroke_once() {
        let mut app = crate::Dudel {
            doc: crate::document::Document::new(16, 16),
            ..crate::Dudel::default()
        };
        app.history.begin_stroke_checkpoint(&app.doc);
        app.stroking = true;
        app.stroke_buffer = Some(Default::default());
        app.doc.dab_buffered(
            8.0,
            8.0,
            1.0,
            &app.brush,
            [20, 40, 60],
            false,
            crate::symmetry::IDENTITY,
            app.stroke_buffer.as_mut(),
        );
        let painted = app.doc.layers[0].pixels.clone();
        app.cancel_gestures();
        app.cancel_gestures();
        assert!(!app.history.has_pending_stroke());
        assert!(!app.stroking);
        assert!(app.stroke_buffer.is_none());
        assert!(app.history.undo(&mut app.doc));
        assert!(app.doc.layers[0].pixels.iter().all(|value| *value == 0));
        assert!(!app.history.can_undo());
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, painted);
    }
    #[test]
    fn invisible_stroke_keeps_redo_without_an_empty_undo_step() {
        use crate::{Dudel, brush::Brush, document::Document, symmetry::IDENTITY};
        for flow in [0.0, 0.00001] {
            let mut app = Dudel {
                doc: Document::new(16, 16),
                ..Dudel::default()
            };
            app.history.checkpoint(&app.doc);
            app.doc.dab(8.0, 8.0, 1.0, &app.brush, [20, 40, 60], false);
            let painted = app.doc.layers[0].pixels.clone();
            assert!(app.history.undo(&mut app.doc));
            app.history.begin_stroke_checkpoint(&app.doc);
            app.stroking = true;
            app.stroke_buffer = Some(Default::default());
            let brush = Brush {
                flow,
                ..Brush::default()
            };
            app.doc.dab_buffered(
                8.0,
                8.0,
                1.0,
                &brush,
                [0, 0, 0],
                false,
                IDENTITY,
                app.stroke_buffer.as_mut(),
            );
            app.finish_painted_stroke();
            app.cancel_gestures();
            assert!(!app.history.can_undo());
            assert!(app.history.can_redo());
            assert_eq!(app.doc.layers[0].pixels.capacity(), 0);
            assert!(app.history.redo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels, painted);
        }
    }
    #[test]
    fn explicit_cancel_restores_pixels_and_preserves_previous_undo() {
        super::Dudel::test_cancel_buffered_stroke();
    }
}
