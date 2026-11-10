use crate::{Dudel, document_edits::Edit, egui, selection::Selection, selection_mask::Combine};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Shape {
    #[default]
    Rectangle,
    Lasso,
    Polygon,
    Wand,
}

pub(crate) enum Drag {
    Rectangle {
        anchor: [usize; 2],
        rectangle: Selection,
        mode: Combine,
    },
    Lasso {
        points: Vec<[f64; 2]>,
        mode: Combine,
    },
    PaintLasso {
        points: Vec<[f64; 2]>,
        settings: crate::lasso_paint::Settings,
        /// Press point and shape for rectangle and ellipse outlines.
        anchor: [f64; 2],
        options: crate::lasso_paint::Options,
    },
    /// A draft across contacts; only an explicit close publishes a selection.
    Polygon {
        points: Vec<[f64; 2]>,
        mode: Combine,
    },
    Wand {
        point: [usize; 2],
        settings: crate::wand::Settings,
        mode: Combine,
    },
    /// The Fill tool's Gradient mode: the line being dragged.
    Gradient(Box<crate::gradient_tool::Settings>),
}
impl Drag {
    pub(crate) fn rectangle(&self) -> Option<Selection> {
        match self {
            Self::Rectangle { rectangle, .. } => Some(*rectangle),
            _ => None,
        }
    }
}
/// Unsigned shoelace area; a click without a drag makes a zero-area shape.
fn polygon_area(points: &[[f64; 2]]) -> f64 {
    let mut twice = 0.0;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        twice += a[0] * b[1] - b[0] * a[1];
    }
    twice.abs() / 2.0
}
impl Dudel {
    pub(crate) fn select_target_coverage(&mut self) {
        if let Some(target) = self.input_target() {
            self.dispatch_document_edit(
                Edit::SelectionCoverage {
                    target,
                    paint_target: self.input_stroke_target(),
                },
                "Selected target coverage",
                "Selection unchanged",
            );
        }
    }
    pub(crate) fn selection_options_ui(&mut self, ui: &mut egui::Ui) {
        self.selection_transform_ui(ui);
        self.selection_shape_ui(ui);
    }
    pub(crate) fn selection_transform_ui(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.doc.selection.is_some() || self.doc.selection_mask.is_some(),
                egui::Button::new("Transform selection…"),
            )
            .on_hover_text(self.command_hint(
                crate::commands::Action::Transform,
                "Move, scale, rotate or warp the selection",
            ))
            .clicked()
        {
            self.affine_settings.mirror = None;
            self.toggle_free_transform();
        }
        if ui
            .add_enabled(
                self.doc.selection.is_some() || self.doc.selection_mask.is_some(),
                egui::Button::new("Transform mirrored pair…"),
            )
            .on_hover_text(
                "Select one feature, then transform it and its existing partner together",
            )
            .clicked()
        {
            self.affine_settings.mirror = Some(crate::affine::Mirror::from_symmetry(
                self.active_symmetry(),
                self.doc.width,
                self.doc.height,
            ));
            self.toggle_free_transform();
        }
    }
    fn selection_shape_ui(&mut self, ui: &mut egui::Ui) {
        let previous = self.selection_shape;
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.selection_shape, Shape::Rectangle, "Rectangle");
            ui.selectable_value(&mut self.selection_shape, Shape::Lasso, "Lasso");
            ui.selectable_value(&mut self.selection_shape, Shape::Polygon, "Polygon");
            ui.selectable_value(&mut self.selection_shape, Shape::Wand, "Magic wand");
        });
        if previous != self.selection_shape {
            self.selection_drag = None;
        }
        if self.selection_shape == Shape::Polygon {
            ui.label("Click vertices, then the first point to close. Enter finishes; Backspace removes a vertex; Esc cancels.");
            self.polygon_selection_controls(ui);
        }
        if self.selection_shape == Shape::Wand {
            let masked = !self.input_stroke_target().is_raster();
            ui.add(
                egui::Slider::new(&mut self.selection_wand.tolerance, 0..=255).text("Tolerance"),
            );
            ui.checkbox(&mut self.selection_wand.contiguous, "Contiguous")
                .on_hover_text("Off selects all matching colors across the canvas");
            let source = &mut self.selection_wand.source;
            let label = match source {
                crate::fill::Source::Active => {
                    if masked {
                        "Active mask"
                    } else {
                        "Active layer"
                    }
                }
                crate::fill::Source::Visible => "Visible layers",
                crate::fill::Source::Layer(id) => self
                    .doc
                    .layers
                    .iter()
                    .find(|layer| layer.id == *id)
                    .map_or("Missing reference layer", |layer| layer.name.as_str()),
            };
            egui::ComboBox::from_id_salt("wand reference")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        source,
                        crate::fill::Source::Active,
                        if masked {
                            "Active mask"
                        } else {
                            "Active layer"
                        },
                    );
                    ui.selectable_value(source, crate::fill::Source::Visible, "Visible layers");
                    ui.separator();
                    for layer in self.doc.layers.iter().rev().filter(|layer| !layer.folder) {
                        ui.selectable_value(
                            source,
                            crate::fill::Source::Layer(layer.id),
                            &layer.name,
                        );
                    }
                });
        }
        ui.add_enabled_ui(self.polygon_selection_points().is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [
                    (Combine::Replace, "Replace"),
                    (Combine::Add, "Add"),
                    (Combine::Subtract, "Subtract"),
                    (Combine::Intersect, "Intersect"),
                ] {
                    ui.selectable_value(&mut self.selection_combine, mode, label);
                }
            });
        });
        ui.add(egui::Slider::new(&mut self.selection_feather, 0..=64).text("Feather (px)"));
        if ui
            .add_enabled(
                self.doc.selection.is_some()
                    || self.doc.selection_mask.is_some()
                    || self.render_queue.busy(),
                egui::Button::new("Apply feather"),
            )
            .clicked()
        {
            self.dispatch_document_edit(
                Edit::FeatherSelection(self.selection_feather),
                "Selection feathered",
                "Selection unchanged",
            );
        }
        if self.render_queue.has_background_edit() && ui.button("Cancel background edit").clicked()
        {
            self.render_queue.cancel_background_edit();
            self.status = "Canceling background edit".into();
        }
    }
    pub(crate) fn update_selection_drag(
        &mut self,
        point: [f64; 2],
        local: [usize; 2],
        start: bool,
        end: bool,
        on_canvas: bool,
    ) {
        if !self.tool.is_lasso_paint() && self.selection_shape == Shape::Polygon {
            self.update_polygon_selection(point, start, on_canvas);
            return;
        }
        if self.polygon_selection_points().is_some() {
            self.selection_drag = None;
        }
        if start && on_canvas {
            let paint = if self.tool.is_lasso_paint() {
                let erase = self.lasso.erase;
                let paint_target = self.input_stroke_target();
                if self.canvas_edit_ticket.is_some() {
                    self.status = "Wait for the canvas bounds change to finish".into();
                    return;
                }
                if let Some(blocker) = self.stroke_target_blocker(paint_target, erase) {
                    self.status = blocker.into();
                    return;
                }
                if erase && paint_target.is_raster() && self.input_alpha_locked() {
                    self.status = "Turn off Lock transparency to use lasso eraser".into();
                    return;
                }
                let Some(target) = self.input_target() else {
                    return;
                };
                Some(Drag::PaintLasso {
                    points: vec![point],
                    anchor: point,
                    options: self.lasso,
                    settings: crate::lasso_paint::Settings {
                        target,
                        paint_target,
                        color: self.paint_color(),
                        opacity: self.brush.opacity,
                        erase,
                    },
                })
            } else {
                None
            };
            self.selection_drag = Some(if let Some(paint) = paint {
                paint
            } else {
                match self.selection_shape {
                    Shape::Rectangle => Drag::Rectangle {
                        anchor: local,
                        rectangle: Selection::from_drag(
                            local,
                            local,
                            self.doc.width,
                            self.doc.height,
                        ),
                        mode: self.selection_combine,
                    },
                    Shape::Lasso => Drag::Lasso {
                        points: vec![point],
                        mode: self.selection_combine,
                    },
                    Shape::Polygon => unreachable!("Polygon contacts handled above"),
                    Shape::Wand => {
                        let mut settings = self.selection_wand;
                        settings.mask = None;
                        if settings.source == crate::fill::Source::Active {
                            let Some(target) = self.input_target() else {
                                self.status = "Magic wand reference layer no longer exists".into();
                                return;
                            };
                            settings.source = crate::fill::Source::Layer(target);
                            if let crate::paint_target::Target::Mask(id) =
                                self.input_stroke_target()
                            {
                                settings.mask = Some((target, id));
                            }
                        }
                        Drag::Wand {
                            point: local,
                            settings,
                            mode: self.selection_combine,
                        }
                    }
                }
            });
        }
        if let Some(drag) = &mut self.selection_drag {
            match drag {
                Drag::Rectangle {
                    anchor, rectangle, ..
                } => {
                    *rectangle =
                        Selection::from_drag(*anchor, local, self.doc.width, self.doc.height)
                }
                Drag::PaintLasso {
                    points,
                    anchor,
                    options,
                    ..
                } if options.shape != crate::lasso_paint::Shape::Freehand => {
                    let point = [
                        point[0].clamp(0.0, self.doc.width as f64),
                        point[1].clamp(0.0, self.doc.height as f64),
                    ];
                    if let Some(shape) = crate::lasso_paint::shape_points(*options, *anchor, point)
                    {
                        *points = shape;
                    }
                }
                Drag::Lasso { points, .. } | Drag::PaintLasso { points, .. } => {
                    let point = [
                        point[0].clamp(0.0, self.doc.width as f64),
                        point[1].clamp(0.0, self.doc.height as f64),
                    ];
                    let last = points.last().copied().unwrap();
                    if point != last
                        && (end || (point[0] - last[0]).hypot(point[1] - last[1]) >= 0.25)
                    {
                        if points.len() >= 16_384 {
                            self.selection_drag = None;
                            self.status =
                                "Lasso path exceeds 16384 points; draw a shorter selection".into();
                            return;
                        }
                        points.push(point);
                    }
                }
                Drag::Wand { .. } | Drag::Polygon { .. } | Drag::Gradient(_) => {}
            }
        }
        if !end {
            return;
        }
        let Some(drag) = self.selection_drag.take() else {
            return;
        };
        let edit = match drag {
            Drag::Rectangle {
                rectangle,
                mode: Combine::Replace,
                ..
            } => Edit::Selection(Some(rectangle)),
            Drag::Rectangle {
                rectangle, mode, ..
            } => Edit::SelectionRectangle { rectangle, mode },
            Drag::Lasso { points, mode } if points.len() >= 3 => {
                Edit::SelectionPolygon { points, mode }
            }
            Drag::Lasso { .. } => {
                self.status = "Draw a lasso with at least three points".into();
                return;
            }
            Drag::PaintLasso {
                points, settings, ..
            } if points.len() >= 3 && polygon_area(&points) > 0.25 => {
                Edit::LassoPaint { points, settings }
            }
            Drag::PaintLasso { options, .. } => {
                self.status = if options.shape == crate::lasso_paint::Shape::Freehand {
                    "Draw a lasso with at least three points".into()
                } else {
                    format!("Drag to size the {}", options.shape.name().to_lowercase())
                };
                return;
            }
            Drag::Polygon { .. } => unreachable!("Polygon drafts handled above"),
            // Gradient drags finish in `gradient_packet`.
            Drag::Gradient(_) => return,
            Drag::Wand {
                point,
                settings,
                mode,
            } => Edit::SelectionWand {
                point,
                settings,
                mode,
            },
        };
        let painting = matches!(&edit, Edit::LassoPaint { .. });
        let fill = matches!(&edit, Edit::LassoPaint { settings, .. } if !settings.erase);
        if self.dispatch_document_edit(
            edit,
            if painting {
                "Lasso applied"
            } else {
                "Selection updated"
            },
            if painting {
                "Lasso made no changes"
            } else {
                "Selection unchanged"
            },
        ) && fill
        {
            self.note_recent_color();
        }
    }

    pub(crate) fn polygon_selection_points(&self) -> Option<&[[f64; 2]]> {
        match &self.selection_drag {
            Some(Drag::Polygon { points, .. }) => Some(points),
            _ => None,
        }
    }
    fn update_polygon_selection(&mut self, mut point: [f64; 2], start: bool, on_canvas: bool) {
        if !start || !on_canvas || !point.iter().all(|value| value.is_finite()) {
            return;
        }
        point[0] = point[0].clamp(0.0, self.doc.width as f64);
        point[1] = point[1].clamp(0.0, self.doc.height as f64);
        let Some(Drag::Polygon { points, .. }) = &mut self.selection_drag else {
            self.selection_drag = Some(Drag::Polygon {
                points: vec![point],
                mode: self.selection_combine,
            });
            self.status =
                "Click polygon vertices; click the first point or press Enter to finish".into();
            return;
        };
        // Closing tolerance is in screen points, independent of canvas zoom.
        let zoom = if self.zoom.is_finite() && self.zoom > 0.0 {
            self.zoom
        } else {
            1.0
        };
        if points.len() >= 3
            && (point[0] - points[0][0]).hypot(point[1] - points[0][1]) * f64::from(zoom) <= 6.0
        {
            self.finish_polygon_selection();
            return;
        }
        if points.last() == Some(&point) {
            return;
        }
        if points.len() >= 16_384 {
            self.status = "Polygon has 16384 vertices; finish it or remove a vertex".into();
            return;
        }
        points.push(point);
    }
    pub(crate) fn finish_polygon_selection(&mut self) {
        let Some(points) = self.polygon_selection_points() else {
            return;
        };
        if points.len() < 3 {
            self.status = "Place at least three polygon vertices".into();
            return;
        }
        let Some(Drag::Polygon { points, mode }) = self.selection_drag.take() else {
            unreachable!()
        };
        self.dispatch_document_edit(
            Edit::SelectionPolygon { points, mode },
            "Polygon selection updated",
            "Selection unchanged",
        );
    }
    pub(crate) fn remove_polygon_vertex(&mut self) {
        if let Some(Drag::Polygon { points, .. }) = &mut self.selection_drag {
            points.pop();
            if points.is_empty() {
                self.selection_drag = None;
            }
        }
    }
    pub(crate) fn polygon_selection_controls(&mut self, ui: &mut egui::Ui) {
        let Some(count) = self.polygon_selection_points().map(|points| points.len()) else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{count} vertices"));
            if ui
                .add_enabled(count >= 3, egui::Button::new("Finish"))
                .on_hover_text("Enter")
                .clicked()
            {
                self.finish_polygon_selection();
            }
            if ui
                .button("Remove vertex")
                .on_hover_text("Backspace")
                .clicked()
            {
                self.remove_polygon_vertex();
            }
            if ui.button("Cancel").on_hover_text("Esc").clicked() {
                self.selection_drag = None;
                self.status = "Polygon selection canceled".into();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn drain(app: &mut Dudel) {
        app.flush_render_queue();
    }
    #[test]
    fn native_lasso_paint_captures_settings_targets_pending_copy_and_undo_without_changing_selection()
     {
        use crate::{input::Packet, stroke::Sample, tools::Tool};
        for erase in [false, true] {
            for cancel in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Dudel {
                    doc: crate::document::Document::new(32, 32),
                    tool: Tool::Lasso,
                    lasso: crate::lasso_paint::Options {
                        erase,
                        ..Default::default()
                    },
                    selection_shape: Shape::Polygon,
                    ..Default::default()
                };
                if erase {
                    app.doc.layers[0].pixels.writer().write_region(
                        32,
                        [0, 0, 32, 32],
                        &[17, 83, 201, 255].repeat(32 * 32),
                    );
                }
                let original = app.doc.layers[0].pixels.clone();
                let selection = Selection {
                    left: 0,
                    top: 0,
                    right: 16,
                    bottom: 32,
                };
                app.doc.selection = Some(selection);
                app.color = [201, 17, 83];
                app.brush.opacity = 0.5;
                app.history.checkpoint(&app.doc);
                app.doc.layers[0].opacity = 0.5;
                app.history.settle(&app.doc);
                assert!(app.history.undo(&mut app.doc));
                let raw = || egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
                if !cancel {
                    app.render_queue.edit(Edit::SelectLayer(0)).unwrap();
                    app.duplicate_layer();
                    assert!(app.input_layer().is_none());
                }
                let samples: Vec<_> = [[4.0, 4.0], [24.0, 4.0], [24.0, 24.0], [4.0, 24.0]]
                    .map(|p| {
                        let p = app.view(app.canvas_area).to_screen(egui::pos2(p[0], p[1]));
                        Sample {
                            x: p.x,
                            y: p.y,
                            pressure: 1.0,
                            ..Default::default()
                        }
                    })
                    .into();
                let mut input = raw();
                input.events.push(egui::Event::PointerMoved(egui::pos2(
                    samples[0].x,
                    samples[0].y,
                )));
                let _ = ctx.run(input, |ctx| app.canvas(ctx, &[Packet::Start(samples[0])]));
                assert!(matches!(app.selection_drag, Some(Drag::PaintLasso { .. })));
                app.color = [0, 255, 0];
                app.brush.opacity = 1.0;
                let _ = ctx.run(raw(), |ctx| {
                    app.canvas(
                        ctx,
                        &[
                            Packet::Move(samples[1]),
                            Packet::Move(samples[2]),
                            Packet::Move(samples[3]),
                            if cancel {
                                Packet::Cancel
                            } else {
                                Packet::End(Sample {
                                    pressure: 0.0,
                                    pressure_known: true,
                                    ..samples[0]
                                })
                            },
                        ],
                    )
                });
                drain(&mut app);
                assert!(app.selection_drag.is_none());
                assert_eq!(app.doc.selection, Some(selection));
                assert!(app.doc.selection_mask.is_none());
                assert_eq!(app.doc.layers[0].pixels, original);
                if cancel {
                    assert!(app.history.can_redo() && !app.history.can_undo());
                    continue;
                }
                for y in 0..32 {
                    for x in 0..32 {
                        let expected = if (4..16).contains(&x) && (4..24).contains(&y) {
                            if erase {
                                [17, 83, 201, 128]
                            } else {
                                [201, 17, 83, 128]
                            }
                        } else {
                            original.rgba(y * 32 + x)
                        };
                        assert_eq!(app.doc.layers[1].pixels.rgba(y * 32 + x), expected);
                    }
                }
                app.undo();
                assert_eq!(app.doc.layers[1].pixels, original);
                app.undo();
                assert_eq!(app.doc.layers.len(), 1);
            }
        }
    }

    #[test]
    fn polygon_lifts_preserve_draft_and_captured_mode_then_queue_one_selection() {
        use crate::document_edits::{Property, Target};
        let mut app = Dudel {
            doc: crate::document::Document::new(100, 100),
            selection_shape: Shape::Polygon,
            selection_combine: Combine::Add,
            zoom: 1.0,
            ..Default::default()
        };
        let previous = crate::selection_mask::Mask::rectangle(
            100,
            100,
            Selection {
                left: 0,
                top: 0,
                right: 5,
                bottom: 5,
            },
        )
        .unwrap();
        app.doc.selection = previous.bounds();
        app.doc.selection_mask = Some(previous.clone());
        app.render_queue
            .edit(Edit::Property {
                target: Target::Active,
                property: Property::Name("Earlier edit".into()),
            })
            .unwrap();
        let points = [[10.0, 10.0], [70.0, 10.0], [70.0, 70.0], [10.0, 70.0]];
        for (index, point) in points.into_iter().enumerate() {
            app.update_selection_drag(
                point,
                [point[0] as usize, point[1] as usize],
                true,
                false,
                true,
            );
            app.update_selection_drag([80.0, 80.0], [80, 80], false, true, true);
            assert_eq!(app.polygon_selection_points().unwrap().len(), index + 1);
            assert_eq!(app.doc.selection_mask.as_ref(), Some(&previous));
            assert!(!app.history.can_undo());
            app.selection_combine = Combine::Subtract;
        }
        app.finish_polygon_selection();
        assert!(app.polygon_selection_points().is_none());
        drain(&mut app);
        let incoming = crate::selection_mask::Mask::polygon(100, 100, &points, None).unwrap();
        let expected = previous.combine(&incoming, Combine::Add, None).unwrap();
        assert_eq!(app.doc.selection_mask, Some(expected.clone()));
        assert_eq!(app.doc.layers[0].name, "Earlier edit");
        app.undo();
        assert_eq!(app.doc.selection_mask, Some(previous.clone()));
        assert_eq!(app.doc.layers[0].name, "Earlier edit");
        app.undo();
        assert!(!app.history.can_undo());
        app.redo();
        app.redo();
        assert_eq!(app.doc.selection_mask, Some(expected));
        assert_eq!(app.doc.layers[0].pixels.capacity(), 0);
    }

    #[test]
    fn polygon_close_radius_tracks_zoom_and_short_or_full_drafts_stay_editable() {
        for zoom in [0.5, 1.0, 2.0] {
            let mut app = Dudel {
                doc: crate::document::Document::new(100, 100),
                selection_shape: Shape::Polygon,
                zoom,
                ..Default::default()
            };
            for point in [[10.0, 10.0], [70.0, 10.0]] {
                app.update_selection_drag(point, [0, 0], true, false, true);
            }
            app.finish_polygon_selection();
            assert_eq!(app.polygon_selection_points().unwrap().len(), 2);
            assert!(!app.render_queue.busy());
            app.update_selection_drag([70.0, 70.0], [0, 0], true, true, true);
            app.update_selection_drag(
                [10.0 + 7.0 / f64::from(zoom), 10.0],
                [0, 0],
                true,
                false,
                true,
            );
            assert_eq!(app.polygon_selection_points().unwrap().len(), 4);
            app.remove_polygon_vertex();
            app.update_selection_drag(
                [10.0 + 5.0 / f64::from(zoom), 10.0],
                [0, 0],
                true,
                true,
                true,
            );
            assert!(app.polygon_selection_points().is_none());
            drain(&mut app);
            let expected = crate::selection_mask::Mask::polygon(
                100,
                100,
                &[[10.0, 10.0], [70.0, 10.0], [70.0, 70.0]],
                None,
            )
            .unwrap();
            assert_eq!(app.doc.selection_mask, Some(expected));
        }
        let mut app = Dudel {
            selection_shape: Shape::Polygon,
            zoom: 1.0,
            ..Default::default()
        };
        app.selection_drag = Some(Drag::Polygon {
            points: vec![[10.0, 10.0]; 16_384],
            mode: Combine::Replace,
        });
        app.update_selection_drag([90.0, 90.0], [0, 0], true, true, true);
        app.update_selection_drag([f64::NAN, 10.0], [0, 0], true, false, true);
        assert_eq!(app.polygon_selection_points().unwrap().len(), 16_384);
        app.remove_polygon_vertex();
        assert_eq!(app.polygon_selection_points().unwrap().len(), 16_383);
        assert!(app.doc.selection_mask.is_none() && !app.history.can_undo());
    }

    #[test]
    fn polygon_canvas_contacts_navigation_keyboard_and_cancellation_preserve_artwork() {
        use crate::{input::Packet, stroke::Sample, tools::Tool};
        for ending in 0..4 {
            let ctx = egui::Context::default();
            let mut app = Dudel {
                doc: crate::document::Document::new(32, 32),
                tool: Tool::Select,
                selection_shape: Shape::Polygon,
                ..Default::default()
            };
            app.doc.layers[0]
                .pixels
                .writer()
                .write_region(32, [0, 0, 1, 1], &[17, 83, 201, 255]);
            let pixels = app.doc.layers[0].pixels.clone();
            app.history.checkpoint(&app.doc);
            app.doc.layers[0].opacity = 0.5;
            app.history.settle(&app.doc);
            assert!(app.history.undo(&mut app.doc));
            let raw = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let key = |key, pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            };
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            for point in [[3.0, 3.0], [25.0, 3.0], [25.0, 25.0], [3.0, 25.0]] {
                let p = app
                    .view(app.canvas_area)
                    .to_screen(egui::pos2(point[0], point[1]));
                let sample = Sample {
                    x: p.x,
                    y: p.y,
                    pressure: 1.0,
                    ..Default::default()
                };
                let mut input = raw();
                input.events.push(egui::Event::PointerMoved(p));
                let _ = ctx.run(input, |ctx| {
                    app.canvas(ctx, &[Packet::Start(sample), Packet::End(sample)])
                });
            }
            assert_eq!(app.polygon_selection_points().unwrap().len(), 4);
            assert!(app.history.can_redo() && !app.history.can_undo());
            // Navigation between vertices leaves the draft in document space.
            let mut input = raw();
            input.events.push(key(egui::Key::Space, true));
            let _ = ctx.run(input, |ctx| app.canvas(ctx, &[]));
            assert_eq!(app.polygon_selection_points().unwrap().len(), 4);
            let mut input = raw();
            input.events.push(key(egui::Key::Space, false));
            input.events.push(key(egui::Key::Backspace, true));
            let _ = ctx.run(input, |ctx| {
                app.shortcuts(ctx);
                app.canvas(ctx, &[]);
            });
            assert_eq!(app.polygon_selection_points().unwrap().len(), 3);
            assert_eq!(app.doc.layers[0].pixels, pixels);
            match ending {
                0 | 1 => {
                    let mut input = raw();
                    input.events.push(key(
                        if ending == 0 {
                            egui::Key::Enter
                        } else {
                            egui::Key::Escape
                        },
                        true,
                    ));
                    let _ = ctx.run(input, |ctx| {
                        app.shortcuts(ctx);
                        app.canvas(ctx, &[]);
                    });
                }
                2 => {
                    let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[Packet::Cancel]));
                }
                _ => {
                    app.select_tool(Tool::Brush);
                    let _ = ctx.run(raw(), |ctx| {
                        app.canvas_after_tool_change(ctx, &[], Tool::Select)
                    });
                }
            }
            drain(&mut app);
            assert!(app.selection_drag.is_none());
            assert_eq!(app.doc.layers[0].pixels, pixels);
            if ending == 0 {
                let mask = app.doc.selection_mask.as_ref().unwrap();
                assert_eq!(mask.at(20, 5), 255);
                assert_eq!(mask.at(5, 20), 0);
                app.undo();
                assert!(app.doc.selection_mask.is_none() && !app.history.can_undo());
                app.redo();
                assert_eq!(app.doc.selection_mask.as_ref().unwrap().at(20, 5), 255);
            } else {
                assert!(
                    app.doc.selection_mask.is_none()
                        && app.history.can_redo()
                        && !app.history.can_undo()
                );
            }
        }
    }

    #[test]
    fn wand_controls_remain_scrollable_in_a_short_tool_panel() {
        let ctx = egui::Context::default();
        let mut app = Dudel {
            tool: crate::tools::Tool::Select,
            selection_shape: Shape::Wand,
            ..Default::default()
        };
        let mut scroll_id = egui::Id::NULL;
        for frame in 0..3 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 240.0),
                )),
                time: Some(frame as f64 * 0.1),
                ..Default::default()
            };
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(280.0, 210.0)));
            if frame == 1 {
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -1000.0),
                    modifiers: Default::default(),
                });
            }
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    scroll_id = ui.make_persistent_id(egui::Id::new("selection controls"));
                    app.tool_panel_ui(ui);
                });
            });
        }
        assert!(
            egui::scroll_area::State::load(&ctx, scroll_id)
                .unwrap()
                .offset
                .y
                > 0.0
        );
    }

    #[test]
    fn canvas_wand_captures_pending_reference_and_settings_on_contact() {
        use crate::{input::Packet, stroke::Sample, tools::Tool};
        for cancel in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Dudel {
                doc: crate::document::Document::new(32, 32),
                tool: Tool::Select,
                selection_shape: Shape::Wand,
                ..Default::default()
            };
            let mut bytes = vec![255; 32 * 32 * 4];
            for y in 0..32 {
                bytes[(y * 32 + 16) * 4..(y * 32 + 17) * 4].copy_from_slice(&[0, 0, 0, 255]);
            }
            app.doc.layers[0]
                .pixels
                .writer()
                .write_region(32, [0, 0, 32, 32], &bytes);
            app.history.checkpoint(&app.doc);
            app.doc.layers[0].opacity = 0.5;
            app.history.settle(&app.doc);
            assert!(app.history.undo(&mut app.doc));
            let raw = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            if !cancel {
                // The selected raster does not exist yet in the rendered stack.
                app.render_queue.edit(Edit::SelectLayer(0)).unwrap();
                app.duplicate_layer();
                assert!(app.input_layer().is_none());
            }
            let p = app.view(app.canvas_area).to_screen(egui::pos2(3.5, 4.5));
            let sample = Sample {
                x: p.x,
                y: p.y,
                pressure: 1.0,
                ..Default::default()
            };
            let mut input = raw();
            input.events.push(egui::Event::PointerMoved(p));
            let _ = ctx.run(input, |ctx| app.canvas(ctx, &[Packet::Start(sample)]));
            assert!(app.selection_drag.is_some() && app.doc.selection_mask.is_none());
            app.selection_wand.contiguous = false;
            app.selection_wand.tolerance = 255;
            app.selection_combine = Combine::Subtract;
            let _ = ctx.run(raw(), |ctx| {
                app.canvas(
                    ctx,
                    &[if cancel {
                        Packet::Cancel
                    } else {
                        Packet::End(sample)
                    }],
                )
            });
            drain(&mut app);
            if cancel {
                assert!(app.doc.selection_mask.is_none());
                assert!(app.history.can_redo() && !app.history.can_undo());
            } else {
                let mask = app.doc.selection_mask.as_ref().unwrap();
                for y in 0..32 {
                    for x in 0..32 {
                        assert_eq!(mask.at(x, y), if x < 16 { 255 } else { 0 });
                    }
                }
                assert_eq!(app.doc.layers[0].pixels, app.doc.layers[1].pixels);
                app.undo();
                assert!(app.doc.selection_mask.is_none() && app.doc.layers.len() == 2);
                app.undo();
                assert_eq!(app.doc.layers.len(), 1);
            }
        }
    }

    #[test]
    fn canvas_lasso_packets_commit_on_lift_and_cancel_preserves_redo() {
        use crate::{input::Packet, stroke::Sample, tools::Tool};
        for cancel in [false, true] {
            let ctx = egui::Context::default();
            let mut app = Dudel {
                doc: crate::document::Document::new(512, 512),
                tool: Tool::Select,
                selection_shape: Shape::Lasso,
                ..Default::default()
            };
            app.history.checkpoint(&app.doc);
            app.doc.layers[0].opacity = 0.5;
            app.history.settle(&app.doc);
            assert!(app.history.undo(&mut app.doc));
            let raw = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
            let samples: Vec<Sample> = [
                [100.0, 100.0],
                [300.0, 100.0],
                [300.0, 300.0],
                [100.0, 300.0],
            ]
            .into_iter()
            .map(|p| {
                let position = app.view(app.canvas_area).to_screen(egui::pos2(p[0], p[1]));
                Sample {
                    x: position.x,
                    y: position.y,
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
                app.canvas(
                    ctx,
                    &[
                        Packet::Start(samples[0]),
                        Packet::Move(samples[1]),
                        Packet::Move(samples[2]),
                        Packet::Move(samples[3]),
                    ],
                )
            });
            assert!(app.selection_drag.is_some() && app.doc.selection_mask.is_none());
            let _ = ctx.run(raw(), |ctx| {
                app.canvas(
                    ctx,
                    &[if cancel {
                        Packet::Cancel
                    } else {
                        Packet::End(samples[0])
                    }],
                )
            });
            drain(&mut app);
            assert!(app.selection_drag.is_none());
            if cancel {
                assert!(
                    app.doc.selection_mask.is_none()
                        && app.history.can_redo()
                        && !app.history.can_undo()
                );
            } else {
                let mask = app.doc.selection_mask.as_ref().unwrap();
                assert_eq!(mask.at(150, 150), 255);
                assert_eq!(mask.at(350, 150), 0);
                assert!(app.history.undo(&mut app.doc));
                assert!(app.doc.selection_mask.is_none());
            }
            assert!(app.doc.layers[0].pixels.capacity() == 0);
        }
    }
    #[test]
    fn lasso_gesture_captures_mode_queues_then_restores_selection_on_undo() {
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 8),
            ..Default::default()
        };
        app.selection_shape = Shape::Lasso;
        app.selection_combine = Combine::Replace;
        app.update_selection_drag([1.0, 1.0], [1, 1], true, false, true);
        app.selection_combine = Combine::Subtract; // The in-flight gesture keeps its captured mode.
        for (i, point) in [[5.0, 1.0], [5.0, 5.0], [1.0, 5.0]].into_iter().enumerate() {
            app.update_selection_drag(
                point,
                [point[0] as usize, point[1] as usize],
                false,
                i == 2,
                true,
            );
        }
        assert!(app.selection_drag.is_none() && app.render_queue.busy());
        assert!(app.doc.selection_mask.is_none());
        drain(&mut app);
        let mask = app.doc.selection_mask.as_ref().unwrap().clone();
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    mask.at(x, y),
                    if (1..5).contains(&x) && (1..5).contains(&y) {
                        255
                    } else {
                        0
                    }
                );
            }
        }
        assert!(app.history.undo(&mut app.doc));
        assert!(app.doc.selection_mask.is_none() && app.doc.selection.is_none());
        assert!(app.history.redo(&mut app.doc));
        assert_eq!(app.doc.selection_mask, Some(mask));
    }
    #[test]
    fn rectangle_replace_preserves_legacy_state_and_short_lasso_preserves_redo() {
        let mut app = Dudel {
            doc: crate::document::Document::new(8, 8),
            ..Default::default()
        };
        app.update_selection_drag([1.0, 1.0], [1, 1], true, false, true);
        app.update_selection_drag([3.0, 4.0], [3, 4], false, true, true);
        assert!(!app.render_queue.busy() && app.doc.selection_mask.is_none());
        assert_eq!(
            app.doc.selection,
            Some(Selection {
                left: 1,
                top: 1,
                right: 4,
                bottom: 5
            })
        );
        assert!(app.history.undo(&mut app.doc));
        app.selection_shape = Shape::Lasso;
        app.update_selection_drag([1.0, 1.0], [1, 1], true, false, true);
        app.update_selection_drag([2.0, 2.0], [2, 2], false, true, true);
        assert!(!app.render_queue.busy() && !app.history.can_undo() && app.history.can_redo());
    }
    #[test]
    fn rectangle_and_lasso_previews_start_from_outside_the_canvas_edge() {
        use crate::{input::Packet, stroke::Sample, tools::Tool};
        for shape in [Shape::Rectangle, Shape::Lasso] {
            let ctx = egui::Context::default();
            let mut app = Dudel {
                doc: crate::document::Document::new(64, 48),
                tool: Tool::Select,
                selection_shape: shape,
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
            let at = |app: &Dudel, x: f32, y: f32| {
                let p = app.view(app.canvas_area).to_screen(egui::pos2(x, y));
                Sample {
                    x: p.x,
                    y: p.y,
                    pressure: 1.0,
                    ..Default::default()
                }
            };
            // Press on the workspace just above-left of the artwork, a common
            // way to start a selection that reaches the canvas edge.
            let outside = at(&app, -3.0, -3.0);
            let inside = at(&app, 30.0, 20.0);
            let corner = at(&app, 40.0, 4.0);
            assert!(app.canvas_area.contains(egui::pos2(outside.x, outside.y)));
            for packets in [
                vec![Packet::Start(outside)],
                vec![Packet::Move(inside), Packet::Move(corner)],
            ] {
                let mut input = raw();
                input
                    .events
                    .push(egui::Event::PointerMoved(egui::pos2(inside.x, inside.y)));
                let _ = ctx.run(input, |ctx| app.canvas(ctx, &packets));
            }
            let name = if shape == Shape::Rectangle {
                "rectangle"
            } else {
                "lasso"
            };
            assert!(
                app.selection_drag.is_some(),
                "{name}: no preview while dragging"
            );
            let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[Packet::End(corner)]));
            app.flush_render_queue();
            let selected = app.doc.selection.expect("selection applied");
            // The rectangle clamps to the corner; the lasso's thin sliver of
            // the first pixel column may round away, so allow one pixel.
            let slack = if shape == Shape::Rectangle { 0 } else { 1 };
            assert!(selected.left <= slack && selected.top <= slack, "{name}");
        }
    }
}
