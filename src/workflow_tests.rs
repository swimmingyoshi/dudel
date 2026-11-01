//! Application workflows through actual egui hit testing and public app actions.
use crate::{Dudel, egui};

struct Workspace {
    ctx: egui::Context,
    app: Dudel,
    output: egui::FullOutput,
    time: f64,
    size: [f32; 2],
    viewport: egui::ViewportInfo,
}

#[test]
fn layer_effects_window_adds_toggles_and_undoes_effects() {
    let mut w = Workspace::sized([1280., 800.]);
    w.app.inspector_override = Some(true);
    w.settle();
    w.app.effects_window_open = true;
    w.settle();
    let height = |w: &Workspace| {
        w.ctx
            .memory(|m| m.area_rect(egui::Id::new("Layer Effects")))
            .unwrap()
            .height()
    };
    let empty = height(&w);
    w.click(w.text("Add effect"));
    w.settle();
    w.click(w.text("Drop Shadow"));
    w.settle();
    // The window grows to show the new effect's settings.
    assert!(height(&w) > empty + 100.0, "{} -> {}", empty, height(&w));
    let active = w.app.doc.active;
    assert_eq!(w.app.doc.layers[active].effects.len(), 1);
    assert!(w.app.effects_edit.is_none(), "a click commits at once");
    assert!(w.app.history.can_undo());
    // The effect's checkbox switches it off without removing it.
    w.click(w.text("Drop Shadow"));
    w.settle();
    assert!(!w.app.doc.layers[active].effects[0].enabled);
    w.app.undo();
    finish_jobs(&mut w.app);
    assert!(w.app.doc.layers[active].effects[0].enabled);
    w.app.undo();
    finish_jobs(&mut w.app);
    assert!(w.app.doc.layers[active].effects.is_empty());
    w.settle();
}

#[test]
fn mask_view_shows_the_targeted_mask_in_grays() {
    let mut app = Dudel {
        doc: crate::document::Document::new(3, 1),
        ..Default::default()
    };
    app.doc.version = 2;
    let mut coverage = crate::layer_mask::Mask::new(3, 1, 255).unwrap();
    coverage.writer().set(1, 0, 0);
    coverage.writer().set(2, 0, 128);
    let mask = crate::layer_mask::LayerMask {
        id: crate::document::LayerId::new(),
        enabled: true,
        linked: true,
        coverage,
    };
    let id = mask.id;
    app.doc.layers[0].mask = Some(mask);
    assert!(app.mask_view_document().is_none());
    app.select_paint_target(0, Some(id));
    assert!(app.mask_view_document().is_none(), "view is off");
    app.mask_view = true;
    let view = app.mask_view_document().unwrap();
    let pixels = view.composite_transparent();
    assert_eq!(
        pixels,
        vec![255, 255, 255, 255, 0, 0, 0, 255, 128, 128, 128, 255]
    );
    // Targeting the artwork again shows the artwork.
    app.select_paint_target(0, None);
    assert!(app.mask_view_document().is_none());
}

#[test]
fn guides_reopen_after_override_with_mouse_and_same_frame_taps() {
    for size in [[1280., 800.], [640., 400.]] {
        let mut w = Workspace::sized(size);
        w.app.inspector_override = Some(true);
        w.app.tool_panel_override = Some(false);
        w.settle();
        for tap in [false, true, false] {
            let at = w.text("Guides");
            if tap {
                w.frame(vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ]);
                w.settle();
            } else {
                w.click(at);
            }
            w.click(w.text("Layer symmetry override"));
            w.key(egui::Key::Escape, Default::default());
            w.settle();
        }
    }
}

#[test]
fn selection_transform_button_clears_previous_mirror_mode() {
    for sidebar in [true, false] {
        let mut w = Workspace::sized([1280., 800.]);
        w.app.tool_panel_override = Some(sidebar);
        w.app.doc.selection = Some(crate::selection::Selection {
            left: 8,
            top: 8,
            right: 24,
            bottom: 24,
        });
        w.app.affine_settings.mirror = Some(crate::affine::Mirror {
            center: [16., 16.],
            angle_degrees: 0.,
        });
        w.app.select_tool(crate::tools::Tool::Select);
        w.settle();
        w.click(w.text("Transform selection…"));
        assert!(w.app.affine_preview.is_some());
        assert!(w.app.affine_settings.mirror.is_none());
    }
}

#[test]
fn extension_panel_opens_and_runs_its_command_through_ui() {
    let mut w = Workspace::sized([1280., 800.]);
    w.app.extensions.push(crate::extensions::Installed {
        package: serde_json::from_str(include_str!("../plugins/example-studio-kit.dudelplugin"))
            .unwrap(),
        enabled: true,
    });
    w.app.extension_manager = true;
    w.settle();
    w.click(w.text("Studio kit…"));
    assert!(w.app.extension_panels.contains("dudel-studio-kit/drawing"));
    w.click(w.text("New layer"));
    w.app.flush_render_queue();
    assert_eq!(w.app.doc.layers.len(), 2);
}

#[test]
fn mirrored_pair_is_reachable_from_selection_and_transform_toolbar() {
    for sidebar in [true, false] {
        let mut w = Workspace::sized([1280., 800.]);
        w.app.inspector_override = Some(false);
        w.app.tool_panel_override = Some(sidebar);
        w.app.doc = crate::document::Document::new(64, 32);
        w.app.doc.selection = Some(crate::selection::Selection {
            left: 8,
            top: 10,
            right: 14,
            bottom: 16,
        });
        w.app.select_tool(crate::tools::Tool::Select);
        w.settle();
        if sidebar {
            w.click(w.text("Transform mirrored pair…"));
        } else {
            w.key(egui::Key::T, egui::Modifiers::CTRL);
            w.click(w.text("Mirror"));
            w.click(w.text("Mirror pair"));
        }
        assert!(w.app.affine_preview.is_some());
        assert_eq!(w.app.affine_settings.mirror.unwrap().center, [32., 16.]);
        w.key(egui::Key::Escape, egui::Modifiers::NONE);
        w.settle();
        if w.app.affine_preview.is_some() {
            w.key(egui::Key::Escape, egui::Modifiers::NONE);
        }
        assert!(w.app.affine_preview.is_none());
        assert!(!w.app.history.can_undo());
    }
}

#[test]
fn transform_properties_scroll_above_color_in_short_windows() {
    fn check(shape: &egui::Shape, clip: egui::Rect, bottom: f32) {
        match shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("Drag inside to move") => {
                let visible = text
                    .galley
                    .rect
                    .translate(text.pos.to_vec2())
                    .intersect(clip);
                assert!(
                    !visible.is_positive() || visible.bottom() < bottom,
                    "Transform help overlaps Color: {visible:?}"
                );
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    check(shape, clip, bottom);
                }
            }
            _ => {}
        }
    }
    for size in [[1280.0, 800.0], [640.0, 400.0], [480.0, 320.0]] {
        let mut w = Workspace::sized(size);
        w.app.inspector_override = Some(false);
        w.app.tool_panel_override = Some(true);
        w.app.doc = crate::document::Document::new(8, 8);
        w.app.select_tool(crate::tools::Tool::Move);
        w.app.start_affine_preview();
        w.settle();
        for scroll in [false, true] {
            let bottom = w.text("COLOR").y;
            if scroll {
                w.frame(vec![
                    egui::Event::PointerMoved(egui::pos2(150.0, bottom - 30.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -2000.0),
                        modifiers: Default::default(),
                    },
                ]);
                w.settle();
            }
            for shape in &w.output.shapes {
                check(&shape.shape, shape.clip_rect, bottom);
            }
        }
        // The fixed toolbar still supplies Apply/Cancel even when properties scroll.
        w.text("Cancel");
        w.key(egui::Key::Escape, Default::default());
        assert!(w.app.affine_preview.is_none());
    }
}

#[test]
fn fill_edge_controls_remain_reachable_without_a_sidebar_in_small_windows() {
    for size in [[1280.0, 800.0], [640.0, 400.0], [480.0, 320.0]] {
        let mut w = Workspace::sized(size);
        w.app.select_tool(crate::tools::Tool::Fill);
        w.app.tool_panel_override = Some(false);
        w.app.inspector_override = Some(false);
        w.settle();
        assert!(w.app.fill_options.fill_edges);
        w.click(w.text("Fill options"));
        w.settle();
        w.click(w.text("Fill to edges"));
        assert!(!w.app.fill_options.fill_edges);
        w.text("Close gaps (px)");
        w.text("Expand (px)");
        w.key(egui::Key::Escape, Default::default());
        w.app.tool_panel_override = Some(true);
        w.settle();
        let color_top = w.text("COLOR").y;
        fn check(shape: &egui::Shape, clip: egui::Rect, bottom: f32) {
            match shape {
                egui::Shape::Text(text)
                    if [
                        "Reference",
                        "Tolerance",
                        "Fill to edges",
                        "Close gaps (px)",
                        "Expand (px)",
                    ]
                    .contains(&text.galley.text()) =>
                {
                    let visible = text
                        .galley
                        .rect
                        .translate(text.pos.to_vec2())
                        .intersect(clip);
                    assert!(
                        !visible.is_positive() || visible.bottom() < bottom,
                        "Fill settings overlap Color: {visible:?}"
                    );
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        check(shape, clip, bottom);
                    }
                }
                _ => {}
            }
        }
        for shape in &w.output.shapes {
            check(&shape.shape, shape.clip_rect, color_top);
        }
    }
}

#[test]
fn layer_badges_edit_local_and_global_symmetry_without_changing_artwork() {
    use crate::symmetry::{Mode, Symmetry};
    let mut w = Workspace::sized([1280.0, 950.0]);
    w.app.inspector_override = Some(true);
    w.app.tool_panel_override = Some(false);
    w.app.layers_height = Some(600.0);
    let id = w.app.doc.layers[0].id;
    w.app.symmetry.mode = Mode::LeftRight;
    w.app.layer_symmetry.insert(
        id,
        Symmetry {
            mode: Mode::Radial,
            ..Default::default()
        },
    );
    w.app.push_ruler(crate::rulers::Ruler::Line {
        a: [10.0, 20.0],
        b: [80.0, 40.0],
    });
    *w.app.ruler_owners.last_mut().unwrap() = Some(id);
    let pixels = w.app.doc.layers[0].pixels.clone();
    w.settle();
    let badge = egui::Id::new(("layer symmetry", id));
    w.click(w.ctx.read_response(badge).unwrap().rect.center());
    w.click(w.text("Top / bottom"));
    assert_eq!(w.app.layer_symmetry[&id].mode, Mode::TopBottom);
    assert_eq!(w.app.symmetry.mode, Mode::LeftRight);
    w.key(egui::Key::Escape, Default::default());
    w.settle();
    w.click(w.text("Global"));
    w.click(w.text("Four-way"));
    assert_eq!(w.app.symmetry.mode, Mode::FourWay);
    assert_eq!(w.app.layer_symmetry[&id].mode, Mode::TopBottom);
    w.key(egui::Key::Escape, Default::default());
    w.settle();
    w.click(w.ctx.read_response(badge).unwrap().rect.center());
    w.click(w.text("Layer symmetry override"));
    assert!(!w.app.layer_symmetry.contains_key(&id));
    assert_eq!(w.app.active_symmetry().mode, Mode::FourWay);
    w.click(w.text("Disable symmetry on this layer"));
    assert_eq!(w.app.active_symmetry().mode, Mode::Off);
    assert!(
        w.ctx.read_response(badge).is_some(),
        "disabled override stays identifiable"
    );
    w.key(egui::Key::Escape, Default::default());
    w.settle();
    let guide_badge = egui::Id::new(("layer guides", id));
    w.click(w.ctx.read_response(guide_badge).unwrap().rect.center());
    w.click(w.text("Layer only"));
    assert_eq!(w.app.ruler_owner(0), None);
    assert_eq!(w.app.doc.layers[0].pixels, pixels);
    assert!(
        !w.app.history.can_undo(),
        "guide edits do not create artwork undo steps"
    );
}

#[test]
fn status_message_and_controls_fit_narrow_and_wide_windows() {
    fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut w = Workspace::sized([1280.0, 640.0]);
    w.app.inspector_override = Some(false);
    w.app.tool_panel_override = Some(false);
    w.app.pressure_seen = true;
    w.app.pressure = Some(1.0);
    w.app.zoom = 8.75;
    w.app.rotation = (-179.0_f32).to_radians();
    let message = "Status with a deliberately long message that must remain accessible without pushing controls outside the window";
    for width in [1920.0, 1280.0, 1100.0, 1099.0, 800.0, 640.0, 480.0] {
        w.size[0] = width;
        w.app.status = message.into();
        w.settle();
        let at = w.text(message);
        let mut labels = Vec::new();
        for shape in &w.output.shapes {
            texts(&shape.shape, &mut labels);
        }
        let screen =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 640.0)).expand(1.0);
        for (label, rect) in labels
            .iter()
            .filter(|(_, rect)| rect.center().y >= at.y - 12.0)
        {
            assert!(
                screen.contains_rect(*rect),
                "Bottom bar {label} at {rect:?} exceeds {screen:?}"
            );
        }
        assert!(
            labels
                .iter()
                .any(|(label, rect)| label.ends_with('°') && rect.center().y >= at.y - 12.0),
            "rotation control missing"
        );
    }
    // Existing extended-distance presets remain readable until the user edits.
    w.app.brush.stabilizer_mode = crate::brush::StabilizerMode::Rope;
    w.app.brush.rope_distance = Some(350.0);
    let _ = w.ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| w.app.rope_distance_ui(ui));
    });
    assert_eq!(w.app.brush.rope_distance, Some(350.0));
}

#[test]
fn reference_board_drag_into_artwork_folder_and_select_individual_images() {
    let mut w = Workspace::sized([1280.0, 950.0]);
    w.app.doc = crate::document::Document::new(32, 32);
    w.app.doc.group_active().unwrap();
    w.app.doc.layers[1].name = "Reference folder".into();
    w.app.inspector_override = Some(true);
    w.app.tool_panel_override = Some(false);
    w.app.layers_height = Some(600.0);
    let image = || crate::reference_layers::Image {
        width: 4,
        height: 4,
        rgba: [90, 120, 180, 255].repeat(16),
    };
    w.app.insert_reference("Hair references".into(), image());
    let id = w.app.reference_layers.active.unwrap();
    w.app
        .insert_reference_image(id, "Boots".into(), image())
        .unwrap();
    w.app.arrange_reference_images(id);
    w.settle();
    let source = w
        .ctx
        .read_response(egui::Id::new(("reference row", id)))
        .unwrap()
        .rect;
    let from = egui::pos2(source.left() + 75.0, source.center().y);
    let target = w.text("Reference folder") + egui::vec2(0.0, 10.0);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    w.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
    w.frame(vec![egui::Event::PointerMoved(
        from + egui::vec2(10.0, 0.0),
    )]);
    w.frame(vec![egui::Event::PointerMoved(target)]);
    w.frame(vec![button(target, false)]);
    w.settle();
    assert_eq!(
        w.app.reference_layers.layers[0]
            .position
            .as_ref()
            .unwrap()
            .depth,
        1
    );
    let thumb = w
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("reference image button", id, 0usize))))
        .unwrap();
    w.click(thumb.center());
    assert_eq!(
        w.app.reference_layers.active_image, 0,
        "primary thumbnail {thumb:?}"
    );
    let thumb = w
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("reference image button", id, 1usize))))
        .unwrap();
    w.click(thumb.center());
    assert_eq!(w.app.reference_layers.active_image, 1);
    w.app.doc.layers[1].collapsed = true;
    w.settle();
    assert!(
        w.ctx
            .read_response(egui::Id::new(("reference row", id)))
            .is_none()
    );
    assert_eq!(w.app.reference_layers.active, Some(id));
}

#[test]
fn window_controls_and_file_exit_preserve_unsaved_artwork() {
    for size in [[800.0, 700.0], [960.0, 640.0], [1600.0, 900.0]] {
        let mut w = Workspace::sized(size);
        for name in ["Minimize", "Maximize", "Close"] {
            let response = w
                .ctx
                .read_response(egui::Id::new(("window-control", name)))
                .unwrap();
            assert!(w.ctx.content_rect().contains_rect(response.rect));
            assert!(response.rect.left() > w.text("Help").x);
        }
        let source = w.app.doc.clone();
        w.app.modified = true;
        w.settle();
        let close = w
            .ctx
            .read_response(egui::Id::new(("window-control", "Close")))
            .unwrap()
            .rect
            .center();
        w.click(close);
        w.text("Save your painting?");
        assert!(!w.app.close_ready);
        w.click(w.text("Cancel"));
        assert!(w.app.unsaved_action.is_none());
        assert!(w.app.modified);
        w.click(w.text("File"));
        w.text("Minimize window");
        w.text("Maximize window");
        w.click(w.text("Exit"));
        w.text("Save your painting?");
        assert!(!w.app.close_ready);
        assert!(crate::document::same_snapshot(&source, &w.app.doc));
        w.click(w.text("Discard"));
        assert!(w.app.close_ready);
    }
}

#[test]
fn window_controls_send_native_commands_without_changing_artwork() {
    let mut w = Workspace::new();
    let source = w.app.doc.clone();
    for (name, expected) in [
        ("Minimize", egui::ViewportCommand::Minimized(true)),
        ("Maximize", egui::ViewportCommand::Maximized(true)),
    ] {
        let pos = w
            .ctx
            .read_response(egui::Id::new(("window-control", name)))
            .unwrap()
            .rect
            .center();
        for pressed in [true, false] {
            w.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                },
            ]);
        }
        assert!(
            w.output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&expected)
        );
        w.settle();
    }
    let pos = egui::pos2(w.size[0] - 2.0, w.size[1] / 2.0);
    w.frame(vec![egui::Event::PointerMoved(pos)]);
    w.frame(vec![egui::Event::PointerButton {
        pos,
        pressed: true,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    }]);
    assert!(
        w.output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::BeginResize(
                egui::ResizeDirection::East
            ))
    );
    assert!(crate::document::same_snapshot(&source, &w.app.doc));
    assert!(!w.app.modified);
}

#[test]
fn window_title_moves_and_restores_and_fullscreen_disables_resize() {
    let mut w = Workspace::new();
    let pos = w
        .ctx
        .read_response(egui::Id::new("window-title-drag"))
        .unwrap()
        .rect
        .center();
    let title_rect = w
        .ctx
        .read_response(egui::Id::new("window-title-drag"))
        .unwrap()
        .rect;
    assert!(title_rect.width() > 80.0);
    assert!(title_rect.left() < w.text("Help").x + 60.0);
    let pointer = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    w.frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
    let moved = pos + egui::vec2(20.0, 0.0);
    w.frame(vec![egui::Event::PointerMoved(moved)]);
    assert!(
        w.output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::StartDrag)
    );
    w.frame(vec![pointer(moved, false)]);
    w.viewport.maximized = Some(true);
    w.settle();
    let pos = w
        .ctx
        .read_response(egui::Id::new(("window-control", "Restore")))
        .unwrap()
        .rect
        .center();
    w.frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
    w.frame(vec![pointer(pos, false)]);
    assert!(
        w.output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Maximized(false))
    );
    let pos = w
        .ctx
        .read_response(egui::Id::new("window-title-drag"))
        .unwrap()
        .rect
        .center();
    // Start a distinct click sequence after the restore-button interaction.
    w.time += 1.0;
    for _ in 0..2 {
        w.frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
        w.frame(vec![pointer(pos, false)]);
    }
    assert!(
        w.output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Maximized(false))
    );
    w.viewport.fullscreen = Some(true);
    w.viewport.maximized = Some(false);
    w.settle();
    assert!(
        !w.ctx
            .read_response(egui::Id::new(("window-control", "Maximize")))
            .unwrap()
            .enabled()
    );
    let pos = egui::pos2(w.size[0] - 2.0, w.size[1] / 2.0);
    w.frame(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]);
    assert!(
        !w.output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::BeginResize(_)))
    );
    assert!(!w.app.modified);
}

#[test]
fn compact_brush_pages_keep_common_controls_and_color_visible() {
    fn fully_visible(w: &Workspace, label: &str) -> bool {
        fn found(shape: &egui::Shape, clip: egui::Rect, label: &str) -> bool {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => clip
                    .expand(0.5)
                    .contains_rect(text.galley.rect.translate(text.pos.to_vec2())),
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| found(s, clip, label)),
                _ => false,
            }
        }
        w.output
            .shapes
            .iter()
            .any(|s| found(&s.shape, s.clip_rect, label))
    }
    for size in [[1280.0, 800.0], [1600.0, 900.0]] {
        let mut w = Workspace::sized(size);
        w.app.tool_panel_override = Some(true);
        w.app.inspector_override = Some(true);
        w.app.sidebar_widths[0] = Some(260.0);
        w.settle();
        let art = w.app.doc.clone();
        for label in [
            "Size",
            "Opacity",
            "Stabilizer",
            "Paint & pressure",
            "Swatches",
            "Studio",
        ] {
            let _ = w.text(label);
        }
        for label in ["Size", "Opacity", "Stabilizer", "Paint & pressure"] {
            assert!(fully_visible(&w, label), "{label} is clipped at {size:?}");
        }
        w.click(w.text("Paint & pressure"));
        w.settle();
        // Advanced delivery controls remain reachable in the settings scroller
        // without displacing the color section below it.
        let bottom = w.text("COLOR").y;
        w.frame(vec![
            egui::Event::PointerMoved(egui::pos2(150.0, bottom - 30.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -1000.0),
                modifiers: Default::default(),
            },
        ]);
        w.settle();
        for label in ["Flow", "Hardness", "Pressure", "Swatches"] {
            assert!(
                fully_visible(&w, label),
                "{label} is unreachable at {size:?}"
            );
        }
        w.click(w.text("Tip"));
        let _ = w.text("Aspect");
        w.click(w.text("Texture"));
        let _ = w.text("Amount");
        w.click(w.text("Advanced"));
        let _ = w.text("Pen pressure");
        w.click(w.text("Basic"));
        let _ = w.text("Flow");
        w.click(w.text("Swatches"));
        let _ = w.text("+ Save color");
        w.key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(crate::document::same_snapshot(&art, &w.app.doc));
        assert!(!w.app.modified);
    }
}

#[test]
fn keyboard_enters_actual_workspace_controls_and_opens_a_menu() {
    let mut w = Workspace::new();
    assert!(w.ctx.memory(|m| m.focused()).is_none());
    let panels = w.app.show_panels;
    w.key(egui::Key::F6, egui::Modifiers::NONE);
    let first = w
        .ctx
        .memory(|m| m.focused())
        .expect("F6 enters control focus");
    let response = w.ctx.read_response(first).unwrap();
    assert!(response.rect.contains(w.text("File")));
    w.key(egui::Key::Tab, egui::Modifiers::NONE);
    assert_ne!(w.ctx.memory(|m| m.focused()), Some(first));
    w.key(egui::Key::Tab, egui::Modifiers::SHIFT);
    assert_eq!(w.ctx.memory(|m| m.focused()), Some(first));
    w.key(egui::Key::Enter, egui::Modifiers::NONE);
    let _ = w.text("New canvas…");
    assert_eq!(w.app.show_panels, panels);
    assert!(!w.app.modified);
}

#[test]
fn about_shows_license_and_source_without_changing_artwork() {
    let mut w = Workspace::new();
    let source = w.app.doc.clone();
    let revision = w.app.revision;
    w.click(w.text("Help"));
    w.click(w.text("About Dudel"));
    assert!(w.app.about_window);
    w.text("License: GPL-3.0-only");
    w.text("Source and build instructions");
    assert!(crate::document::same_snapshot(&source, &w.app.doc));
    assert_eq!(w.app.revision, revision);
}

#[test]
fn dragging_tool_settings_closed_and_clicking_a_tool_reopens_them_at_their_width() {
    let mut w = Workspace::sized([1600.0, 900.0]);
    w.app.tool_panel_override = Some(true);
    w.settle();
    let panel = |w: &Workspace, id: &str| {
        egui::containers::panel::PanelState::load(&w.ctx, egui::Id::new(id)).map(|s| s.rect)
    };
    let before = panel(&w, "tool property").expect("tool settings are shown");
    let doc = w.app.doc.clone();
    let drag = |w: &mut Workspace, from: egui::Pos2, to: egui::Pos2| {
        let button = |pressed, pos| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        w.frame(vec![egui::Event::PointerMoved(from)]);
        w.frame(vec![button(true, from)]);
        for step in 1..=6 {
            let at = from + (to - from) * (step as f32 / 6.0);
            w.frame(vec![egui::Event::PointerMoved(at)]);
        }
        w.frame(vec![button(false, to)]);
        w.settle();
    };
    // A small drag only resizes; dragging far past the minimum closes it.
    let edge = egui::pos2(before.right(), before.center().y);
    drag(&mut w, edge, edge - egui::vec2(10.0, 0.0));
    assert_eq!(w.app.tool_panel_override, Some(true));
    let resized = w.app.sidebar_widths[0];
    drag(
        &mut w,
        egui::pos2(before.left() + resized.unwrap(), edge.y),
        egui::pos2(before.left() + 8.0, edge.y),
    );
    assert_eq!(w.app.tool_panel_override, Some(false));
    assert!(!w.app.tool_panel_visible(&w.ctx));
    assert_eq!(w.app.sidebar_widths[0], resized);
    // Clicking a tool in the rail opens the settings again at that width.
    let rail = panel(&w, "tools").unwrap();
    w.click(egui::pos2(rail.center().x, rail.top() + 22.0));
    assert_eq!(w.app.tool_panel_override, Some(true));
    let reopened = panel(&w, "tool property").unwrap();
    assert!((reopened.width() - resized.unwrap()).abs() < 1.5);
    assert_eq!(w.app.doc.composite(), doc.composite());
}

#[test]
fn laptop_color_area_resizes_past_the_old_limit_persists_and_resets() {
    let mut w = Workspace::sized([1280.0, 720.0]);
    w.app.tool_panel_override = Some(true);
    w.settle();
    let area = |w: &Workspace| {
        egui::containers::panel::PanelState::load(&w.ctx, egui::Id::new("color area"))
            .unwrap()
            .rect
    };
    let before = area(&w);
    let handle = egui::pos2(before.center().x, before.top() + 5.0);
    let button = |pressed, pos| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    w.frame(vec![egui::Event::PointerMoved(handle)]);
    w.frame(vec![button(true, handle)]);
    for step in 1..=8 {
        w.frame(vec![egui::Event::PointerMoved(
            handle - egui::vec2(0.0, 30.0 * step as f32),
        )]);
    }
    w.frame(vec![button(false, handle - egui::vec2(0.0, 240.0))]);
    w.settle();
    let grown = area(&w);
    // The old layout capped color at total - 330 (about 270 px here).
    assert!(grown.height() > 300.0, "color area {}", grown.height());
    assert!(grown.height() > before.height() + 100.0);
    let height = w.app.color_height;
    let prefs = serde_json::from_str(&w.app.encode_preferences().unwrap()).unwrap();
    let mut restored = crate::Dudel::default();
    restored.apply_preferences(prefs);
    assert_eq!(restored.color_height, height);
    // Double-clicking the handle returns to the automatic size.
    let handle = egui::pos2(grown.center().x, grown.top() + 5.0);
    w.click(handle);
    w.frame(vec![
        egui::Event::PointerMoved(handle),
        button(true, handle),
        button(false, handle),
    ]);
    w.settle();
    assert_eq!(w.app.color_height, None);
    assert!((area(&w).height() - before.height()).abs() < 1.0);
}

#[test]
fn status_bar_shows_recovery_backup_only_for_unsaved_changes() {
    fn shown(w: &Workspace) -> bool {
        fn found(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().starts_with("Backup saved"),
                egui::Shape::Vec(shapes) => shapes.iter().any(found),
                _ => false,
            }
        }
        w.output.shapes.iter().any(|s| found(&s.shape))
    }
    for size in [[960.0, 700.0], [1600.0, 900.0]] {
        let mut w = Workspace::sized(size);
        assert!(!shown(&w));
        w.app.modified = true;
        w.app.recovery_saved = Some(std::time::Instant::now());
        w.settle();
        w.text("Backup saved just now");
        // A saved project has nothing to recover.
        w.app.modified = false;
        w.settle();
        assert!(!shown(&w), "{size:?}");
        // Retiring the snapshot (save, replace) clears the cue.
        w.app.modified = true;
        w.app.clear_recovery_snapshot();
        w.settle();
        assert!(!shown(&w), "{size:?}");
        assert_eq!(w.app.recovery_saved, None);
    }
}

#[test]
fn save_progress_stays_inside_the_status_bar_without_resizing_it_or_the_canvas() {
    for size in [[960.0, 700.0], [1280.0, 720.0], [1920.0, 1080.0]] {
        let mut w = Workspace::sized(size);
        let status = |w: &Workspace| {
            egui::containers::panel::PanelState::load(&w.ctx, egui::Id::new("status"))
                .unwrap()
                .rect
        };
        let (bar, canvas) = (status(&w), w.app.canvas_area);
        w.app.queued_save = Some(crate::save_jobs::QueuedSave {
            ticket: 0,
            path: "progress.dudel".into(),
            canceled: false,
        });
        w.settle();
        w.text("Waiting for preceding artwork…");
        w.text("Cancel save");
        assert_eq!(status(&w), bar, "{size:?}: status bar moved while saving");
        assert_eq!(
            w.app.canvas_area, canvas,
            "{size:?}: canvas resized while saving"
        );
        w.app.queued_save = None;
        w.settle();
        assert_eq!(status(&w), bar, "{size:?}: status bar moved after saving");
        assert_eq!(
            w.app.canvas_area, canvas,
            "{size:?}: canvas resized after saving"
        );
    }
}

#[test]
fn clicking_a_tool_shows_settings_the_automatic_layout_hid_on_a_laptop_screen() {
    // 1920x1080 at 150% Windows scaling: automatic layout hides Brush settings.
    let mut w = Workspace::sized([1280.0, 720.0]);
    assert!(w.app.tool == crate::tools::Tool::Brush);
    assert_eq!(w.app.tool_panel_override, None);
    assert!(!w.app.tool_panel_visible(&w.ctx));
    // Keyboard-style tool switches leave the panel as it is.
    w.app.select_tool(crate::tools::Tool::Eraser);
    w.settle();
    assert_eq!(w.app.tool_panel_override, None);
    assert!(!w.app.tool_panel_visible(&w.ctx));
    let doc = w.app.doc.clone();
    let rail = egui::containers::panel::PanelState::load(&w.ctx, egui::Id::new("tools"))
        .unwrap()
        .rect;
    w.click(egui::pos2(rail.center().x, rail.top() + 22.0));
    assert!(w.app.tool == crate::tools::Tool::Brush);
    assert_eq!(w.app.tool_panel_override, Some(true));
    assert!(w.app.tool_panel_visible(&w.ctx));
    assert!(
        egui::containers::panel::PanelState::load(&w.ctx, egui::Id::new("tool property")).is_some()
    );
    // The opened settings still leave most of the window for drawing.
    assert!(
        w.app.canvas_area.width() >= 1280.0 * 0.5,
        "canvas {}",
        w.app.canvas_area.width()
    );
    assert_eq!(w.app.doc.composite(), doc.composite());
}

#[test]
fn wheel_over_an_effect_blend_box_steps_it_without_scrolling_the_window() {
    use crate::blend::BlendMode;
    use crate::layer_effects::{Effect, LayerEffect};
    // A short window, so the effects list scrolls.
    let mut w = Workspace::sized([1280.0, 600.0]);
    w.app.inspector_override = Some(true);
    let active = w.app.doc.active;
    w.app.doc.layers[active].effects = ["Drop Shadow", "Outer Glow", "Stroke"]
        .into_iter()
        .map(|name| LayerEffect::new(Effect::preset(name).unwrap()))
        .collect();
    w.app.effects_window_open = true;
    w.settle();
    // The top entry (listed first) is the stroke, blend Normal; the layer's
    // own blend box elsewhere also reads Normal, so find the one in the window.
    let window = w
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("Layer Effects")))
        .unwrap();
    let combo = w.text_in("Normal", window);
    let marker = w.text_in("Opacity", window);
    w.frame(vec![
        egui::Event::PointerMoved(combo),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -super::layers_panel::BLEND_WHEEL_STEP),
            modifiers: Default::default(),
        },
    ]);
    w.settle();
    finish_jobs(&mut w.app);
    let stroke = w.app.doc.layers[active].effects.last().unwrap();
    assert_eq!(stroke.effect.blend, BlendMode::Dissolve);
    assert_eq!(w.text_in("Opacity", window), marker, "the window scrolled");
}

#[test]
fn blend_modes_step_with_wheel_and_arrows_and_the_full_list_fits_without_scrolling() {
    use crate::blend::BlendMode;
    let mut w = Workspace::sized([1920.0, 1080.0]);
    let active = w.app.doc.active;
    let combo = w.text("Normal");
    let wheel = |w: &mut Workspace, notches: f32| {
        w.frame(vec![
            egui::Event::PointerMoved(combo),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, notches * super::layers_panel::BLEND_WHEEL_STEP),
                modifiers: Default::default(),
            },
        ]);
        w.settle();
    };
    // Wheel down moves down the list one mode per notch.
    wheel(&mut w, -1.0);
    assert_eq!(w.app.doc.layers[active].blend, BlendMode::Dissolve);
    // Arrow keys step while the box is hovered; up returns.
    w.key(egui::Key::ArrowDown, Default::default());
    assert_eq!(w.app.doc.layers[active].blend, BlendMode::Darken);
    w.key(egui::Key::ArrowUp, Default::default());
    w.key(egui::Key::ArrowUp, Default::default());
    assert_eq!(w.app.doc.layers[active].blend, BlendMode::Normal);
    // Wheel up past the first mode stays there.
    wheel(&mut w, 2.0);
    assert_eq!(w.app.doc.layers[active].blend, BlendMode::Normal);
    // The open list shows every mode on screen at once.
    w.click(combo);
    for mode in BlendMode::ALL {
        let at = w.text(mode.name());
        assert!(at.y > 0.0 && at.y < 1080.0, "{} at {at:?}", mode.name());
    }
    w.click(w.text("Luminosity"));
    assert_eq!(w.app.doc.layers[active].blend, BlendMode::Luminosity);
}

#[test]
fn automatic_layout_restores_hidden_workspace_and_survives_preferences_roundtrip() {
    for size in [
        [480.0, 320.0],
        [960.0, 700.0],
        [1280.0, 800.0],
        [2560.0, 1000.0],
    ] {
        let mut w = Workspace::sized(size);
        w.app.set_layer_opacity(0, 0.5);
        w.app.finish_opacity_edit();
        w.settle();
        assert!(w.app.history.can_undo());
        let doc = w.app.doc.clone();
        let revision = w.app.revision;
        let modified = w.app.modified;
        w.app.tool_panel_override = Some(true);
        w.app.inspector_override = Some(false);
        w.app.show_panels = false;
        w.settle();
        w.click(w.text("View"));
        w.click(w.text("Automatic panel layout"));
        assert!(w.app.show_panels);
        assert!(w.app.tool_panel_override.is_none() && w.app.inspector_override.is_none());
        assert!(crate::document::same_snapshot(&doc, &w.app.doc));
        assert_eq!(w.app.revision, revision);
        assert_eq!(w.app.modified, modified);
        assert!(w.app.history.can_undo());
        assert_eq!(w.app.inspector_visible(&w.ctx), size[0] >= 1000.0);
        assert_eq!(w.app.tool_panel_visible(&w.ctx), size[0] >= 2000.0);

        let prefs = serde_json::from_str(&w.app.encode_preferences().unwrap()).unwrap();
        let mut restored = Dudel::default();
        restored.apply_preferences(prefs);
        assert!(restored.show_panels);
        assert!(restored.tool_panel_override.is_none() && restored.inspector_override.is_none());
        assert_eq!(restored.inspector_visible(&w.ctx), size[0] >= 1000.0);
        assert_eq!(restored.tool_panel_visible(&w.ctx), size[0] >= 2000.0);
        w.app.undo();
        assert_eq!(w.app.doc.layers[0].opacity, 1.0);
    }
}

#[test]
fn brush_studio_compact_creation_revert_and_sections_fit_after_viewport_resize() {
    fn assert_actions(workspace: &Workspace) {
        fn find(shape: &egui::Shape, wanted: &str, clip: egui::Rect, screen: egui::Rect) -> bool {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                    assert!(
                        clip.contains_rect(rect) && screen.contains_rect(rect),
                        "Clipped studio action {wanted}: {rect:?}, clip {clip:?}, screen {screen:?}"
                    );
                    true
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| find(s, wanted, clip, screen)),
                _ => false,
            }
        }
        let screen = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(workspace.size[0], workspace.size[1]),
        );
        let state = workspace.app.brush.validate().err().unwrap_or_else(|| {
            if workspace.app.brush_preset_modified() {
                "Unsaved preset changes".into()
            } else {
                "Saved preset".into()
            }
        });
        for text in ["Save as new", "Update preset…", "Revert", state.as_str()] {
            assert!(
                workspace.output.shapes.iter().rev().any(|s| find(
                    &s.shape,
                    text,
                    s.clip_rect,
                    screen
                )),
                "Missing studio action: {text}"
            );
        }
    }
    for size in [[960.0, 700.0], [640.0, 400.0], [480.0, 320.0]] {
        let mut w = Workspace::sized(size);
        w.app.brush_studio = true;
        w.app.brush_studio_section = crate::brush_studio::Section::Stabilizer;
        w.app.brush.stabilization = 12.0;
        w.settle();
        assert_actions(&w);
        assert!(w.app.brush_preset_modified());
        w.click(w.text("Revert"));
        w.settle();
        assert!(!w.app.brush_preset_modified());
        w.app.brush.stabilization = 12.0;
        let before = w.app.brushes.len();
        w.settle();
        w.click(w.text("Save as new"));
        w.settle();
        assert_eq!(w.app.brushes.len(), before + 1);
        assert_eq!(w.app.brushes[w.app.selected_brush].stabilization, 12.0);
        assert!(!w.app.brush_preset_modified());
        w.size = [480.0, 320.0];
        w.settle();
        assert_actions(&w);
        w.click(w.text("Stabilizer"));
        w.settle();
        assert!(
            egui::Popup::is_any_open(&w.ctx),
            "Studio section chooser did not open at {size:?}"
        );
        let at = w.text("Stabilizer");
        w.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -320.0),
                modifiers: Default::default(),
            },
        ]);
        for _ in 0..20 {
            w.frame(vec![]);
        }
        w.click(w.text("Name and tags"));
        w.settle();
        assert_eq!(
            w.app.brush_studio_section,
            crate::brush_studio::Section::Tags
        );
        assert_actions(&w);
        w.app.brush.name.clear();
        w.settle();
        assert_actions(&w);
        let count = w.app.brushes.len();
        w.click(w.text("Save as new"));
        assert_eq!(w.app.brushes.len(), count);
        w.click(w.text("Revert"));
        w.settle();
        assert_actions(&w);
    }
}
impl Workspace {
    fn new() -> Self {
        Self::sized([960.0, 700.0])
    }
    fn sized(size: [f32; 2]) -> Self {
        let mut workspace = Self {
            ctx: egui::Context::default(),
            app: Dudel::default(),
            output: Default::default(),
            time: 0.0,
            size,
            viewport: Default::default(),
        };
        workspace.settle();
        workspace
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.frame_packets(events, &[]);
    }
    fn frame_packets(&mut self, events: Vec<egui::Event>, packets: &[crate::input::Packet]) {
        self.time += 0.05;
        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.size[0], self.size[1]),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        raw_input
            .viewports
            .insert(egui::ViewportId::ROOT, self.viewport.clone());
        eframe::App::raw_input_hook(&mut self.app, &self.ctx, &mut raw_input);
        self.output = self.ctx.run(raw_input, |ctx| {
            crate::workspace::theme(ctx);
            self.app.poll_adjustment(ctx);
            let previous_tool = self.app.tool;
            self.app.update_tool_shift(ctx);
            self.app.shortcuts(ctx);
            self.app.workspace(ctx);
            self.app
                .canvas_after_tool_change(ctx, packets, previous_tool);
            self.app.brush_studio_ui(ctx);
            self.app.effects_window(ctx);
            self.app.adjustment_window(ctx);
            self.app.unsaved_dialog(ctx);
        });
    }
    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(vec![]);
        }
    }
    /// The topmost visible `wanted` text inside `area`.
    fn text_in(&self, wanted: &str, area: egui::Rect) -> egui::Pos2 {
        fn collect(shape: &egui::Shape, wanted: &str, clip: egui::Rect, out: &mut Vec<egui::Pos2>) {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => {
                    let visible = text
                        .galley
                        .rect
                        .translate(text.pos.to_vec2())
                        .intersect(clip);
                    if visible.is_positive() {
                        out.push(visible.center());
                    }
                }
                egui::Shape::Vec(shapes) => shapes
                    .iter()
                    .for_each(|shape| collect(shape, wanted, clip, out)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for shape in &self.output.shapes {
            collect(&shape.shape, wanted, shape.clip_rect, &mut found);
        }
        found
            .into_iter()
            .filter(|at| area.contains(*at))
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap_or_else(|| panic!("{wanted} not shown in {area:?}"))
    }
    fn text(&self, wanted: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, wanted: &str, clip: egui::Rect) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => {
                    // Right-aligned galleys have a negative local x origin.
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    let visible = rect.intersect(clip);
                    visible.is_positive().then(|| visible.center())
                }
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| find(shape, wanted, clip))
                }
                _ => None,
            }
        }
        self.output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| find(&shape.shape, wanted, shape.clip_rect))
            .unwrap_or_else(|| {
                fn rendered(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Text(text) => out.push(format!(
                            "{} {:?} clip {clip:?}",
                            text.galley.text(),
                            text.galley.rect.translate(text.pos.to_vec2())
                        )),
                        egui::Shape::Vec(shapes) => {
                            for s in shapes {
                                rendered(s, clip, out);
                            }
                        }
                        _ => {}
                    }
                }
                let mut labels = Vec::new();
                for s in &self.output.shapes {
                    rendered(&s.shape, s.clip_rect, &mut labels);
                }
                panic!("Visible workspace text missing: {wanted}. Rendered: {labels:#?}")
            })
    }
    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(at), button(true)]);
        self.frame(vec![button(false)]);
        self.settle();
    }
    fn texture_rect(&self, id: egui::TextureId) -> egui::Rect {
        self.visible_texture_rect(id)
            .expect("Visible thumbnail texture")
    }
    fn visible_texture_rect(&self, id: egui::TextureId) -> Option<egui::Rect> {
        fn find(shape: &egui::Shape, id: egui::TextureId) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == id => Some(egui::Rect::from_points(
                    &mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>(),
                )),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, id)),
                _ => None,
            }
        }
        self.output.shapes.iter().find_map(|s| {
            find(&s.shape, id)
                .map(|r| r.intersect(s.clip_rect))
                .filter(|r| r.is_positive())
        })
    }
    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        // eframe reports Ctrl as the platform command key everywhere but macOS.
        let modifiers = egui::Modifiers {
            command: modifiers.command || (!cfg!(target_os = "macos") && modifiers.ctrl),
            ..modifiers
        };
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![event(true)]);
        self.frame(vec![event(false)]);
    }
    fn paint_shape(&mut self, tool: crate::tools::Tool, points: &[[f32; 2]]) {
        self.app.select_tool(tool);
        self.settle();
        let samples = points
            .iter()
            .map(|p| {
                let at = self
                    .app
                    .view(self.app.canvas_area)
                    .to_screen(egui::pos2(p[0], p[1]));
                crate::stroke::Sample {
                    x: at.x,
                    y: at.y,
                    pressure: 0.1,
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        let packets = samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i == 0 {
                    crate::input::Packet::Start(*s)
                } else if i + 1 == samples.len() {
                    crate::input::Packet::End(*s)
                } else {
                    crate::input::Packet::Move(*s)
                }
            })
            .collect::<Vec<_>>();
        let last = samples.last().unwrap();
        self.frame_packets(
            vec![egui::Event::PointerMoved(egui::pos2(last.x, last.y))],
            &packets,
        );
        self.app.flush_render_queue();
    }
}

#[test]
fn short_inspector_keeps_layers_masks_and_navigator_reachable_without_overlap() {
    fn text_rect(shape: &egui::Shape, wanted: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, wanted)),
            _ => None,
        }
    }
    fn visible(w: &Workspace, wanted: &str) -> egui::Rect {
        w.output
            .shapes
            .iter()
            .find_map(|shape| {
                text_rect(&shape.shape, wanted).filter(|rect| shape.clip_rect.contains_rect(*rect))
            })
            .unwrap_or_else(|| panic!("{wanted} clipped at {:?}", w.size))
    }
    for size in [[480.0, 320.0], [640.0, 400.0], [960.0, 500.0]] {
        let mut w = Workspace::sized(size);
        w.app.doc = crate::document::Document::new(32, 32);
        w.app.inspector_override = Some(true);
        w.app.tool_panel_override = Some(false);
        w.app.layers_height = Some(600.0);
        w.app.navigator_height = Some(600.0);
        w.settle();
        let navigator = visible(&w, "Navigator");
        let layers = visible(&w, "LAYERS");
        assert!(navigator.bottom() < layers.top());
        for label in ["Global", "Guides"] {
            visible(&w, label);
        }
        let layer = w.app.layer_thumbnails[&w.app.doc.layers[0].id].id();
        let thumbnail = w.texture_rect(layer);
        assert!(thumbnail.height() >= 39.0, "{size:?}: {thumbnail:?}");
        if !w
            .output
            .shapes
            .iter()
            .any(|s| text_rect(&s.shape, "Artwork").is_some())
        {
            w.click(w.text("Layer"));
            w.settle();
        }
        for label in ["Artwork", "Mask", "Mask actions", "Normal"] {
            visible(&w, label);
        }
        w.click(w.text("Mask actions"));
        w.click(w.text("Reveal all"));
        w.app.flush_render_queue();
        w.settle();
        w.click(w.text("Mask"));
        assert!(!w.app.input_stroke_target().is_raster());
        w.key(egui::Key::Escape, Default::default());
        w.click(w.text("Navigator"));
        w.settle();
        w.click(w.text("Flip canvas"));
        assert!(w.app.mirror);
        assert_eq!(w.app.layers_height, Some(600.0));
        assert_eq!(w.app.navigator_height, Some(600.0));
        w.key(egui::Key::Escape, Default::default());
        w.size = [1280.0, 800.0];
        w.settle();
        visible(&w, "LAYERS");
        visible(&w, "Mask actions");
    }
}

#[test]
fn layer_list_reveals_selected_mask_on_resize_and_preserves_manual_scroll() {
    let directory = tempfile::tempdir().unwrap();
    let mut ui = Workspace::sized([1280.0, 700.0]);
    ui.app = Dudel::for_capture(directory.path().join("not-requested.png"));
    ui.app.doc.active = 0;
    ui.app.inspector_override = Some(true);
    assert!(
        ui.app
            .create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll)
    );
    ui.app.flush_render_queue();
    ui.settle();
    let id = ui.app.mask_thumbnails[&ui.app.doc.layers[0].id].id();
    ui.size[1] = 500.0;
    for _ in 0..20 {
        ui.frame(vec![]);
    }
    let rect = ui.texture_rect(id);
    assert!(rect.height() >= 39.0, "Selected mask was clipped: {rect:?}");
    ui.frame(vec![
        egui::Event::PointerMoved(rect.center()),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 400.0),
            modifiers: Default::default(),
        },
    ]);
    for _ in 0..20 {
        ui.frame(vec![]);
    }
    assert!(
        ui.visible_texture_rect(id)
            .is_none_or(|r| r.height() < 39.0),
        "Manual scroll was overridden"
    );
    ui.size[1] = 620.0;
    for _ in 0..20 {
        ui.frame(vec![]);
    }
    assert!(ui.texture_rect(id).height() >= 39.0);
}

#[test]
fn long_layer_lists_paint_only_visible_rows_and_still_reveal_the_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut ui = Workspace::sized([1280.0, 700.0]);
    ui.app = Dudel::for_capture(directory.path().join("not-requested.png"));
    ui.app.inspector_override = Some(true);
    while ui.app.doc.layers.len() < 200 {
        ui.app.doc.insert_layer_above().unwrap();
    }
    ui.app.changed();
    for _ in 0..300 {
        if ui.app.thumbnail_revision == Some(ui.app.revision) {
            break;
        }
        ui.frame(vec![]);
    }
    assert_eq!(ui.app.layer_thumbnails.len(), 200);
    let thumbnail =
        |ui: &Workspace, index: usize| ui.app.layer_thumbnails[&ui.app.doc.layers[index].id].id();
    for (selected, hidden) in [(199, 0), (0, 199)] {
        ui.app.doc.active = selected;
        for _ in 0..20 {
            ui.frame(vec![]);
        }
        let id = thumbnail(&ui, selected);
        assert!(
            ui.texture_rect(id).height() >= 39.0,
            "layer {selected} was not revealed"
        );
        // Rows far outside the list are not laid out at all, not even as
        // shapes clipped away.
        let painted = |id: egui::TextureId| {
            fn find(shape: &egui::Shape, id: egui::TextureId) -> bool {
                match shape {
                    egui::Shape::Mesh(mesh) => mesh.texture_id == id,
                    egui::Shape::Vec(shapes) => shapes.iter().any(|s| find(s, id)),
                    _ => false,
                }
            }
            ui.output.shapes.iter().any(|s| find(&s.shape, id))
        };
        assert!(painted(thumbnail(&ui, selected)));
        assert!(!painted(thumbnail(&ui, hidden)));
        assert!(!painted(thumbnail(&ui, 100)));
    }
}

#[test]
fn navigator_reserves_controls_and_compact_view_options_remain_clickable() {
    let mut ui = Workspace::sized([1280.0, 700.0]);
    ui.app.inspector_override = Some(true);
    ui.app.tool_panel_override = Some(false);
    ui.app.navigator_height = Some(600.0);
    ui.app.layers_height = Some(260.0);
    ui.settle();
    fn fully_visible(shape: &egui::Shape, wanted: &str, clip: egui::Rect) -> bool {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                clip.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| fully_visible(s, wanted, clip)),
            _ => false,
        }
    }
    assert!(
        ui.output
            .shapes
            .iter()
            .any(|s| fully_visible(&s.shape, "Flip canvas", s.clip_rect))
    );
    assert_eq!(ui.app.navigator_height, Some(600.0)); // Fit without losing the preferred height.
    ui.app.layers_height = Some(440.0);
    for _ in 0..8 {
        ui.frame(vec![]);
    }
    let options = ui.text("View options");
    assert!(
        ui.output
            .shapes
            .iter()
            .any(|s| fully_visible(&s.shape, "View options", s.clip_rect))
    );
    ui.click(options);
    ui.click(ui.text("Flip canvas"));
    assert!(ui.app.mirror);
    ui.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    ui.app.layers_height = Some(260.0);
    ui.settle();
    assert!(
        ui.output
            .shapes
            .iter()
            .any(|s| fully_visible(&s.shape, "Flip canvas", s.clip_rect))
    );
    assert_eq!(ui.app.navigator_height, Some(600.0));
}

#[test]
fn mask_focus_thumbnails_actions_and_canvas_packets_keep_artwork_separate() {
    let directory = tempfile::tempdir().unwrap();
    let mut ui = Workspace::sized([1280.0, 800.0]);
    ui.app = Dudel::for_capture(directory.path().join("not-requested.png"));
    ui.app.doc = crate::document::Document::new(32, 32);
    ui.app.symmetry = Default::default();
    ui.app.doc.layers[0].pixels = [17, 83, 201, 255].repeat(32 * 32).into();
    ui.app.doc.layers[0].alpha_locked = true;
    let pixels = ui.app.doc.layers[0].pixels.clone();
    assert!(
        ui.app
            .create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll)
    );
    ui.app.flush_render_queue();
    ui.app.show_panels = true;
    ui.app.inspector_override = Some(true);
    ui.app.tool_panel_override = Some(false);
    ui.app.color = [0; 3];
    ui.app.brush = crate::brush::Brush {
        size: 8.0,
        hardness: 1.0,
        pressure_size: false,
        ..Default::default()
    };
    ui.settle();
    let mask = ui.app.doc.layers[0].mask.as_ref().unwrap().id;
    let mask_image = ui.app.mask_thumbnails[&ui.app.doc.layers[0].id].id();
    let raster_image = ui.app.layer_thumbnails[&ui.app.doc.layers[0].id].id();
    ui.click(ui.texture_rect(raster_image).center());
    assert!(ui.app.input_stroke_target().is_raster() && ui.app.renaming.is_none());
    // A mask replaced between pointer press and release must not inherit the click.
    let at = ui.texture_rect(mask_image).center();
    ui.frame(vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        },
    ]);
    assert!(
        ui.app
            .edit_canvas_mask(crate::layer_mask_edits::Command::Remove(mask))
    );
    assert!(
        ui.app
            .create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll)
    );
    ui.app.flush_render_queue();
    ui.app.focus_current_mask(false);
    ui.frame(vec![egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    }]);
    ui.settle();
    assert!(ui.app.input_stroke_target().is_raster());
    let mask = ui.app.doc.layers[0].mask.as_ref().unwrap().id;
    ui.click(ui.texture_rect(mask_image).center());
    assert_eq!(
        ui.app.input_stroke_target(),
        crate::paint_target::Target::Mask(mask)
    );
    assert!(ui.app.renaming.is_none());
    let screen = ui
        .app
        .view(ui.app.canvas_area)
        .to_screen(egui::pos2(16.5, 16.5));
    let sample = crate::stroke::Sample {
        x: screen.x,
        y: screen.y,
        pressure: 1.0,
        ..Default::default()
    };
    ui.frame_packets(
        vec![egui::Event::PointerMoved(screen)],
        &[
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ],
    );
    ui.app.flush_render_queue();
    assert!(ui.app.doc.layers[0].pixels.shares(&pixels));
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        0
    );
    assert!(!ui.ctx.wants_keyboard_input() && !egui::Popup::is_any_open(&ui.ctx));
    ui.key(egui::Key::Z, egui::Modifiers::CTRL);
    ui.app.flush_render_queue();
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        255
    );
    ui.settle();
    ui.click(ui.text("Mask actions"));
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    ui.app.select_tool(crate::tools::Tool::Fill);
    ui.app.brush.opacity = 0.5;
    ui.settle();
    let screen = ui
        .app
        .view(ui.app.canvas_area)
        .to_screen(egui::pos2(16.5, 16.5));
    let sample = crate::stroke::Sample {
        x: screen.x,
        y: screen.y,
        ..sample
    };
    ui.frame_packets(
        vec![egui::Event::PointerMoved(screen)],
        &[
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ],
    );
    ui.app.flush_render_queue();
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        128
    );
    ui.key(egui::Key::Delete, egui::Modifiers::NONE);
    ui.app.flush_render_queue();
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        255
    );
    assert!(ui.app.doc.layers[0].pixels.shares(&pixels));
    ui.app.lasso.erase = false;
    ui.paint_shape(
        crate::tools::Tool::Lasso,
        &[[4.0, 4.0], [28.0, 4.0], [28.0, 28.0], [4.0, 28.0]],
    );
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        128
    );
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(0, 0),
        255
    );
    ui.app.brush.opacity = 1.0;
    ui.app.lasso.erase = true;
    ui.paint_shape(
        crate::tools::Tool::Lasso,
        &[[4.0, 4.0], [28.0, 4.0], [28.0, 28.0], [4.0, 28.0]],
    );
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        255
    );
    assert!(ui.app.doc.layers[0].pixels.shares(&pixels));
    ui.app.select_tool(crate::tools::Tool::Brush);
    ui.settle();
    ui.click(ui.text("Edit"));
    ui.text("Copy mask");
    ui.text("Cut mask");
    ui.text("Paste into mask");
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    ui.click(ui.text("Mask actions"));
    ui.click(ui.text("Enable mask"));
    ui.app.flush_render_queue();
    assert!(!ui.app.doc.layers[0].mask.as_ref().unwrap().enabled);
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    ui.click(ui.text("Artwork"));
    assert!(ui.app.input_stroke_target().is_raster());
    assert!(ui.app.doc.layers[0].pixels.shares(&pixels));
    assert!(ui.app.doc.layers[0].alpha_locked);
    // The toolbar target chooser remains reachable with the inspector hidden.
    ui.size = [768.0, 420.0];
    ui.app.inspector_override = Some(false);
    ui.settle();
    // The target menu is labeled with the paint group, such as Pen.
    let label = ui.app.tool_label().0;
    ui.click(ui.text(label));
    ui.click(ui.text("Edit mask"));
    assert_eq!(
        ui.app.input_stroke_target(),
        crate::paint_target::Target::Mask(mask)
    );
    ui.click(ui.text("Mask"));
    ui.click(ui.text("Edit artwork"));
    assert!(ui.app.input_stroke_target().is_raster());
}

#[test]
fn mask_coverage_popup_paint_picker_and_shortcuts_preserve_artwork_colors() {
    let directory = tempfile::tempdir().unwrap();
    let mut ui = Workspace::sized([1280.0, 800.0]);
    ui.app = Dudel::for_capture(directory.path().join("unused.png"));
    ui.app.doc = crate::document::Document::new(32, 32);
    ui.app.symmetry = Default::default();
    ui.app.color = [201, 17, 83];
    ui.app.secondary = [17, 201, 83];
    let colors = (ui.app.color, ui.app.secondary);
    ui.app.doc.layers[0].pixels = [17, 83, 201, 255].repeat(32 * 32).into();
    let pixels = ui.app.doc.layers[0].pixels.clone();
    assert!(
        ui.app
            .create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll)
    );
    ui.app.flush_render_queue();
    ui.app.mask_colors = [64, 0];
    ui.app.brush = crate::brush::Brush {
        size: 8.0,
        hardness: 1.0,
        pressure_size: false,
        ..Default::default()
    };
    ui.settle();
    ui.click(ui.text("Coverage"));
    ui.text("Mask coverage");
    ui.click(ui.text("Reveal"));
    assert_eq!(ui.app.mask_colors[0], 255);
    assert_eq!((ui.app.color, ui.app.secondary), colors);
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    ui.key(egui::Key::X, egui::Modifiers::NONE);
    assert_eq!(ui.app.mask_colors, [0, 255]);
    ui.app.mask_colors[0] = 64;
    let screen = ui
        .app
        .view(ui.app.canvas_area)
        .to_screen(egui::pos2(16.5, 16.5));
    let sample = crate::stroke::Sample {
        x: screen.x,
        y: screen.y,
        pressure: 1.0,
        ..Default::default()
    };
    ui.frame_packets(
        vec![egui::Event::PointerMoved(screen)],
        &[
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ],
    );
    ui.app.flush_render_queue();
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        64
    );
    ui.app.select_tool(crate::tools::Tool::Picker);
    let slot = ui.app.doc.layers[0].mask.as_ref().unwrap().id;
    ui.app
        .edit_canvas_mask(crate::layer_mask_edits::Command::Enabled {
            id: slot,
            value: false,
        });
    finish_jobs(&mut ui.app);
    ui.app.mask_colors[0] = 255;
    ui.settle();
    let screen = ui
        .app
        .view(ui.app.canvas_area)
        .to_screen(egui::pos2(16.5, 16.5));
    let sample = crate::stroke::Sample {
        x: screen.x,
        y: screen.y,
        ..sample
    };
    ui.frame_packets(
        vec![egui::Event::PointerMoved(screen)],
        &[
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ],
    );
    assert_eq!(ui.app.mask_colors[0], 64);
    assert!(ui.app.status.contains("mask coverage"));
    assert_eq!((ui.app.color, ui.app.secondary), colors);
    assert!(ui.app.doc.layers[0].pixels.shares(&pixels));
    ui.app.select_tool(crate::tools::Tool::Select);
    ui.app.selection_shape = crate::selection_ui::Shape::Wand;
    ui.app.tool_panel_override = Some(true);
    ui.settle();
    ui.text("Active mask");
    let screen = ui
        .app
        .view(ui.app.canvas_area)
        .to_screen(egui::pos2(16.5, 16.5));
    let sample = crate::stroke::Sample {
        x: screen.x,
        y: screen.y,
        ..sample
    };
    ui.frame_packets(
        vec![egui::Event::PointerMoved(screen)],
        &[
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ],
    );
    finish_jobs(&mut ui.app);
    assert_eq!(ui.app.doc.selection_mask.as_ref().unwrap().at(16, 16), 255);
    ui.app.focus_current_mask(false);
    assert_eq!(ui.app.paint_color(), colors.0);
}

#[test]
fn rope_mode_keeps_the_inline_brush_bar_fixed_and_moves_distance_to_its_menu() {
    let mut ui = Workspace::sized([1600.0, 700.0]);
    ui.app.select_tool(crate::tools::Tool::Brush);
    ui.app.inspector_override = Some(false);
    ui.app.tool_panel_override = Some(false);
    ui.settle();
    let shows = |ui: &Workspace, wanted: &str| {
        fn find(shape: &egui::Shape, wanted: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text() == wanted,
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| find(s, wanted)),
                _ => false,
            }
        }
        ui.output.shapes.iter().any(|s| find(&s.shape, wanted))
    };
    let smooth = (ui.text("Snap"), ui.text("Smooth"));
    ui.app.toggle_rope_stabilizer();
    ui.settle();
    assert_eq!(
        ui.text("Snap"),
        smooth.0,
        "switching to Rope must not move the bar"
    );
    assert_eq!(
        ui.text("Rope"),
        smooth.1,
        "the follow-mode button keeps its place"
    );
    assert!(
        !shows(&ui, "Distance"),
        "Rope distance stays out of the inline bar"
    );
    ui.click(ui.text("Rope"));
    ui.text("Distance");
    ui.text("Less smoothing on fast strokes");
}

#[test]
fn narrow_mask_toolbar_keeps_stroke_options_fully_visible_and_clickable() {
    for width in [840.0, 768.0, 640.0, 480.0] {
        let mut ui = Workspace::sized([width, 420.0]);
        ui.app.inspector_override = Some(false);
        ui.app.tool_panel_override = Some(false);
        assert!(
            ui.app
                .create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll)
        );
        finish_jobs(&mut ui.app);
        ui.settle();
        fn visible(shape: &egui::Shape, clip: egui::Rect) -> bool {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == "Stroke options" => {
                    let rect = egui::Rect::from_min_size(text.pos, text.galley.size())
                        .expand2(egui::vec2(10.0, 7.0));
                    assert!(
                        clip.contains_rect(rect),
                        "Stroke options bounds {rect:?} exceed {clip:?}"
                    );
                    true
                }
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| visible(s, clip)),
                _ => false,
            }
        }
        assert!(
            ui.output
                .shapes
                .iter()
                .any(|s| visible(&s.shape, s.clip_rect))
        );
        ui.click(ui.text("Stroke options"));
        ui.text("Stabilizer");
        if width < 700.0 {
            ui.text("Size");
            ui.text("Opacity");
        }
    }
}

#[test]
fn compact_header_keeps_menus_and_document_state_separate() {
    let mut w = Workspace::sized([1920.0, 800.0]);
    let name = format!("{}.dudel", "Long illustration name ".repeat(8));
    w.app.path = Some(std::path::PathBuf::from(&name));
    for width in [1920.0, 960.0, 840.0, 768.0, 640.0, 480.0] {
        w.size = [width, 320.0];
        for modified in [false, true] {
            w.app.modified = modified;
            w.settle();
            let y = w.text("DUDEL").y;
            let menu_y = w.text("File").y;
            fn header(shape: &egui::Shape, y: f32, out: &mut Vec<(String, egui::Rect)>) {
                match shape {
                    egui::Shape::Text(text) => {
                        let r = text.galley.rect.translate(text.pos.to_vec2());
                        if (r.center().y - y).abs() < 2.0 {
                            out.push((text.galley.text().to_owned(), r));
                        }
                    }
                    egui::Shape::Vec(shapes) => {
                        for s in shapes {
                            header(s, y, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut labels = Vec::new();
            for s in &w.output.shapes {
                header(&s.shape, y, &mut labels);
                if (menu_y - y).abs() > 2.0 {
                    header(&s.shape, menu_y, &mut labels);
                }
            }
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 320.0));
            for (i, (label, rect)) in labels.iter().enumerate() {
                assert!(
                    screen.contains_rect(*rect),
                    "Clipped header {label}: {rect:?} at {width}"
                );
                for (other, bounds) in labels.iter().skip(i + 1) {
                    assert!(
                        !rect.intersects(*bounds),
                        "Overlapping header {label}/{other} at {width}"
                    );
                }
            }
            w.text(if modified { "Unsaved changes" } else { "Saved" });
        }
        w.click(w.text("File"));
        w.click(w.text("New canvas\u{2026}"));
        assert!(w.app.new_canvas);
        w.click(w.text("Cancel"));
        assert!(!w.app.new_canvas);
        w.settle();
    }
}

#[test]
fn new_canvas_keyboard_presets_and_orientation_do_not_submit_the_form() {
    let mut w = Workspace::new();
    let original = w.app.doc.clone();
    w.app.open_new_canvas();
    w.settle();
    // Width has initial focus; the preceding controls are Save as preset,
    // Swap and 4K.
    for _ in 0..3 {
        w.key(egui::Key::Tab, egui::Modifiers::SHIFT);
    }
    w.key(egui::Key::Enter, Default::default());
    assert!(w.app.new_canvas);
    assert_eq!(w.app.new_canvas_form.dimensions(), Ok([3840, 2160]));
    w.key(egui::Key::Tab, Default::default());
    w.key(egui::Key::Enter, Default::default());
    assert!(w.app.new_canvas);
    assert_eq!(w.app.new_canvas_form.dimensions(), Ok([2160, 3840]));
    assert!(crate::document::same_snapshot(&w.app.doc, &original));
    w.key(egui::Key::Escape, Default::default());
    assert!(!w.app.new_canvas);
    assert!(crate::document::same_snapshot(&w.app.doc, &original));
}

#[test]
fn new_canvas_keyboard_submission_uses_current_field_text_and_blocks_artwork_shortcuts() {
    let mut w = Workspace::new();
    w.app.set_selection(Some(crate::selection::Selection {
        left: 1,
        top: 1,
        right: 3,
        bottom: 3,
    }));
    w.app.modified = false; // A saved document can retain its selection.
    let old = w.app.doc.clone();
    let tool = w.app.tool;
    w.key(egui::Key::N, egui::Modifiers::CTRL);
    assert!(w.app.new_canvas);
    w.key(egui::Key::A, egui::Modifiers::CTRL);
    w.frame(vec![egui::Event::Text("32".into())]);
    w.key(egui::Key::Tab, Default::default());
    w.key(egui::Key::A, egui::Modifiers::CTRL);
    w.frame(vec![egui::Event::Text("16".into())]);
    w.key(egui::Key::Tab, Default::default());
    w.key(egui::Key::A, egui::Modifiers::CTRL);
    w.frame(vec![egui::Event::Text("600".into())]);
    assert_eq!(w.app.new_canvas_form.dimensions().unwrap(), [32, 16]);
    assert!(crate::document::same_snapshot(&w.app.doc, &old));
    assert!(w.app.tool == tool);
    w.key(egui::Key::Enter, Default::default());
    assert!(!w.app.new_canvas);
    assert_eq!([w.app.doc.width, w.app.doc.height], [32, 16]);
    assert_eq!(w.app.doc.dpi, 600.0);
    assert!(w.app.path.is_none() && !w.app.modified && !w.app.history.can_undo());
    let created = w.app.doc.clone();
    w.app.open_new_canvas();
    w.settle();
    // Width, height, DPI, the timelapse option and Create come before Cancel.
    for _ in 0..5 {
        w.key(egui::Key::Tab, Default::default());
    }
    w.key(egui::Key::Enter, Default::default());
    assert!(!w.app.new_canvas && crate::document::same_snapshot(&w.app.doc, &created));
}

#[test]
fn new_canvas_cancel_and_invalid_dimensions_preserve_the_document_at_small_sizes() {
    for size in [[960.0, 700.0], [640.0, 400.0], [480.0, 320.0]] {
        let mut w = Workspace::sized(size);
        w.app.doc = crate::document::Document::new(8, 8);
        w.app.doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[23, 84, 163, 255]);
        w.app.modified = true;
        w.app.set_selection(Some(crate::selection::Selection {
            left: 1,
            top: 1,
            right: 3,
            bottom: 3,
        }));
        let original = serde_json::to_value(&w.app.doc).unwrap();
        w.app.open_new_canvas();
        w.settle();
        for label in ["Create canvas", "Cancel"] {
            let p = w.text(label);
            assert!(p.x > 0.0 && p.x < size[0] && p.y > 0.0 && p.y < size[1]);
        }
        for (width, height) in [
            ("", "8"),
            ("0", "8"),
            ("16385", "8"),
            ("10001", "10000"),
            ("9999999999999999", "8"),
            ("12.5", "8"),
        ] {
            w.app.new_canvas_form.width = width.into();
            w.app.new_canvas_form.height = height.into();
            w.settle();
            w.click(w.text("Create canvas"));
            w.key(egui::Key::Enter, Default::default());
            assert!(w.app.new_canvas && w.app.unsaved_action.is_none());
            assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), original);
        }
        w.key(egui::Key::Escape, Default::default());
        assert!(!w.app.new_canvas);
        assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), original);
        w.app.open_new_canvas();
        w.settle();
        w.click(w.text("Cancel"));
        assert!(!w.app.new_canvas && w.app.modified);
        assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), original);
    }
}

#[test]
fn new_canvas_unsaved_cancel_returns_to_draft_and_discard_replaces_only_after_confirmation() {
    let mut w = Workspace::new();
    // Single-project mode: a new canvas replaces the open one.
    w.app.tabs.max = 1;
    w.app.doc = crate::document::Document::new(8, 8);
    w.paint_shape(crate::tools::Tool::Brush, &[[1.0, 1.0], [5.0, 5.0]]);
    assert!(w.app.modified && w.app.history.can_undo());
    let original = serde_json::to_value(&w.app.doc).unwrap();
    let brush = w.app.brush.clone();
    w.app.open_new_canvas();
    w.app.new_canvas_form.width = "12".into();
    w.app.new_canvas_form.height = "7".into();
    w.settle();
    w.click(w.text("Create canvas"));
    w.text("Save your painting?");
    assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), original);
    w.click(w.text("Cancel"));
    w.settle();
    assert!(w.app.new_canvas && w.app.unsaved_action.is_none() && w.app.history.can_undo());
    assert_eq!(w.app.new_canvas_form.dimensions().unwrap(), [12, 7]);
    w.click(w.text("Create canvas"));
    w.click(w.text("Discard"));
    w.settle();
    assert!(!w.app.new_canvas && !w.app.modified && !w.app.history.can_undo());
    assert_eq!([w.app.doc.width, w.app.doc.height], [12, 7]);
    assert!(w.app.path.is_none());
    assert!(w.app.brush == brush);
}

#[test]
fn new_canvas_saves_the_old_painting_before_creating_the_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Original.dudel");
    let mut w = Workspace::new();
    // Single-project mode: a new canvas replaces the open one.
    w.app.tabs.max = 1;
    w.app.doc = crate::document::Document::new(8, 8);
    w.paint_shape(crate::tools::Tool::Brush, &[[1.0, 1.0], [5.0, 5.0]]);
    let original = w.app.doc.composite_transparent();
    w.app.path = Some(path.clone());
    w.app.open_new_canvas();
    w.app.new_canvas_form.width = "6".into();
    w.app.new_canvas_form.height = "9".into();
    w.settle();
    w.click(w.text("Create canvas"));
    w.click(w.text("Save"));
    finish_jobs(&mut w.app);
    w.settle();
    assert_eq!([w.app.doc.width, w.app.doc.height], [6, 9]);
    assert!(!w.app.new_canvas && !w.app.modified);
    let saved = crate::storage::load_project(&path).unwrap();
    assert_eq!(saved.composite_transparent(), original);
}

#[test]
fn new_canvas_failed_save_returns_to_form_without_losing_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Blocked.dudel");
    std::fs::create_dir(&path).unwrap();
    let mut w = Workspace::new();
    // Single-project mode: a new canvas replaces the open one.
    w.app.tabs.max = 1;
    w.app.doc = crate::document::Document::new(8, 8);
    w.paint_shape(crate::tools::Tool::Brush, &[[1.0, 1.0], [5.0, 5.0]]);
    let original = serde_json::to_value(&w.app.doc).unwrap();
    w.app.path = Some(path.clone());
    w.app.open_new_canvas();
    w.app.new_canvas_form.width = "12".into();
    w.app.new_canvas_form.height = "7".into();
    w.settle();
    w.click(w.text("Create canvas"));
    w.click(w.text("Save"));
    finish_jobs(&mut w.app);
    w.settle();
    assert!(w.app.new_canvas && w.app.modified && w.app.pending_action.is_none());
    assert!(
        w.app
            .new_canvas_form
            .feedback
            .as_ref()
            .unwrap()
            .contains("could not be saved")
    );
    assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), original);
    assert!(w.app.history.can_undo());
    assert_eq!(w.app.path.as_ref(), Some(&path));
    assert!(path.is_dir());
    w.click(w.text("Cancel"));
    assert!(!w.app.new_canvas);
}

#[test]
fn new_canvas_waits_for_an_existing_save_and_recovers_the_form_if_it_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Blocked.dudel");
    std::fs::create_dir(&path).unwrap();
    let mut w = Workspace::new();
    // Single-project mode: a new canvas replaces the open one.
    w.app.tabs.max = 1;
    w.app.doc = crate::document::Document::new(8, 8);
    w.paint_shape(crate::tools::Tool::Brush, &[[1.0, 1.0], [5.0, 5.0]]);
    let original = serde_json::to_value(&w.app.doc).unwrap();
    w.app.path = Some(path);
    w.app.save(false);
    assert!(w.app.save_job.is_some());
    w.app.open_new_canvas();
    w.app.new_canvas_form.width = "12".into();
    w.app.new_canvas_form.height = "7".into();
    w.settle();
    w.click(w.text("Create canvas"));
    assert!(
        w.app.new_canvas
            && matches!(
                w.app.pending_action,
                Some(crate::document_actions::Action::New(12, 7, 300.0))
            )
    );
    finish_jobs(&mut w.app);
    w.settle();
    assert!(
        w.app.new_canvas
            && w.app.pending_action.is_none()
            && w.app.new_canvas_form.feedback.is_some()
    );
    assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), original);
    assert!(w.app.modified && w.app.history.can_undo());
    w.click(w.text("Cancel"));
    assert!(!w.app.new_canvas);
}

#[test]
fn new_canvas_canceled_by_newer_edits_explains_why_and_preserves_the_latest_drawing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Original.dudel");
    let mut w = Workspace::new();
    // Single-project mode: a new canvas replaces the open one.
    w.app.tabs.max = 1;
    w.app.doc = crate::document::Document::new(8, 8);
    w.paint_shape(crate::tools::Tool::Brush, &[[1.0, 1.0], [5.0, 5.0]]);
    w.app.path = Some(path);
    w.app.save(false);
    w.app.open_new_canvas();
    w.app.new_canvas_form.width = "12".into();
    w.app.new_canvas_form.height = "7".into();
    w.settle();
    w.click(w.text("Create canvas"));
    assert!(w.app.pending_action.is_some());
    // An edit worker can finish after the save snapshot was captured.
    w.app.doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[210, 54, 97, 255]);
    w.app.changed();
    let latest = serde_json::to_value(&w.app.doc).unwrap();
    finish_jobs(&mut w.app);
    w.settle();
    assert!(w.app.new_canvas && w.app.modified && w.app.pending_action.is_none());
    assert!(
        w.app
            .new_canvas_form
            .feedback
            .as_ref()
            .unwrap()
            .contains("changed during saving")
    );
    assert_eq!(serde_json::to_value(&w.app.doc).unwrap(), latest);
    w.click(w.text("Cancel"));
}

#[test]
fn stroke_options_and_symmetry_remain_usable_in_minimum_scaled_viewport() {
    let mut w = Workspace::sized([480.0, 320.0]);
    w.app.inspector_override = Some(false);
    w.app.tool_panel_override = Some(false);
    w.settle();
    w.click(w.text("Stroke options"));
    for label in ["Size", "Opacity", "Stabilizer", "Snap", "Symmetry"] {
        let at = w.text(label);
        assert!(
            at.x >= 0.0 && at.x < 480.0 && at.y >= 0.0 && at.y < 320.0,
            "Offscreen stroke option {label}: {at:?}"
        );
    }
    w.click(w.text("Symmetry"));
    let anchor = w.text("Snap");
    w.frame(vec![
        egui::Event::PointerMoved(anchor),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -180.0),
            modifiers: Default::default(),
        },
    ]);
    for _ in 0..20 {
        w.frame(vec![]);
    }
    w.click(w.text("Radial"));
    assert_eq!(w.app.active_symmetry().mode, crate::symmetry::Mode::Radial);
    w.settle();
    let anchor = w.text("Symmetry: Radial");
    w.frame(vec![
        egui::Event::PointerMoved(anchor),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -80.0),
            modifiers: Default::default(),
        },
    ]);
    for _ in 0..20 {
        w.frame(vec![]);
    }
    // Copies now sits at the popup's bottom edge; scroll over a mode button.
    w.text("Copies");
    let anchor = w.text("Radial");
    w.frame(vec![
        egui::Event::PointerMoved(anchor),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -260.0),
            modifiers: Default::default(),
        },
    ]);
    for _ in 0..20 {
        w.frame(vec![]);
    }
    let locked = w.app.guides_locked;
    w.click(w.text("Lock guides"));
    assert_eq!(w.app.guides_locked, !locked);
}

#[test]
fn long_brush_toolbar_fits_and_controls_work_across_resizes_and_targets() {
    fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut w = Workspace::sized([1920.0, 800.0]);
    assert!(
        w.app
            .create_canvas_mask(crate::layer_mask_edits::Seed::RevealAll)
    );
    finish_jobs(&mut w.app);
    w.app.inspector_override = Some(false);
    w.app.tool_panel_override = Some(false);
    w.app.symmetry.mode = crate::symmetry::Mode::TopBottom;
    for width in [
        1920.0, 1600.0, 1440.0, 1280.0, 1100.0, 1050.0, 1049.0, 960.0, 840.0, 768.0, 700.0, 699.0,
        640.0, 480.0,
    ] {
        w.size = [width, 420.0];
        for tool in [crate::tools::Tool::Brush, crate::tools::Tool::Eraser] {
            w.app.select_tool(tool);
            for mask in [false, true] {
                w.app.focus_current_mask(mask);
                let name = if mask {
                    "\u{7b46}".repeat(64)
                } else {
                    "Long brush name ".repeat(12)
                };
                w.app.brush.name = name.clone();
                w.settle();
                let row = w.text(if mask { "Coverage" } else { "Color" });
                let mut labels = Vec::new();
                for shape in &w.output.shapes {
                    texts(&shape.shape, &mut labels);
                }
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 420.0));
                for (label, rect) in &labels {
                    if (rect.center().y - row.y).abs() < 2.0 {
                        assert!(
                            screen.contains_rect(*rect),
                            "Toolbar {label} at {rect:?} exceeds {screen:?}, tool {}, mask {mask}",
                            tool.name()
                        );
                    }
                }
                let picker = labels
                    .iter()
                    .find(|(label, _)| label == &name)
                    .expect("Truncated brush button retains full accessible text")
                    .1;
                assert!(
                    picker.width() <= 192.0,
                    "Unbounded brush picker: {picker:?}"
                );
                if [480.0, 768.0, 1280.0, 1920.0].contains(&width) {
                    if let Some((_, options)) =
                        labels.iter().find(|(label, _)| label == "Stroke options")
                    {
                        w.click(options.center());
                    }
                    let before = w.app.snapping;
                    w.click(w.text("Snap"));
                    assert_eq!(w.app.snapping, !before);
                    w.key(egui::Key::Escape, Default::default());
                    w.settle();
                    w.click(w.text(&name));
                    w.text("Brush settings\u{2026}");
                    let preset = w.app.brushes[0].name.clone();
                    w.click(w.text(&preset));
                    assert_eq!(w.app.selected_brush, 0);
                    assert_eq!(w.app.brush.name, preset);
                    assert!(!egui::Popup::is_any_open(&w.ctx));
                    w.click(w.text(if mask { "Coverage" } else { "Color" }));
                    assert!(egui::Popup::is_any_open(&w.ctx));
                    w.key(egui::Key::Escape, Default::default());
                    w.settle();
                }
            }
        }
    }
}

#[test]
fn normal_mask_workflow_creates_paints_selects_duplicates_transforms_saves_reopens_recovers_and_exports()
 {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("Masked illustration.dudel");
    let png = directory.path().join("Masked illustration.png");
    let recovery = directory.path().join("masked-recovery.gz");
    let mut ui = Workspace::sized([1280.0, 800.0]);
    assert!(ui.app.capture.is_none());
    ui.app.doc = crate::document::Document::new(32, 32);
    ui.app.doc.layers[0].pixels = [17, 83, 201, 200].repeat(32 * 32).into();
    ui.app.inspector_override = Some(true);
    ui.app.tool_panel_override = Some(false);
    ui.app.brush = crate::brush::Brush {
        size: 8.0,
        hardness: 1.0,
        pressure_size: false,
        ..Default::default()
    };
    ui.settle();
    ui.click(ui.text("Mask actions"));
    ui.click(ui.text("Reveal all"));
    finish_jobs(&mut ui.app);
    let first_mask = ui.app.doc.layers[0].mask.as_ref().unwrap().id;
    assert_eq!(
        ui.app.input_stroke_target(),
        crate::paint_target::Target::Mask(first_mask)
    );
    ui.app.mask_colors[0] = 64;
    let screen = ui
        .app
        .view(ui.app.canvas_area)
        .to_screen(egui::pos2(16.5, 16.5));
    let sample = crate::stroke::Sample {
        x: screen.x,
        y: screen.y,
        pressure: 1.0,
        ..Default::default()
    };
    ui.frame_packets(
        vec![egui::Event::PointerMoved(screen)],
        &[
            crate::input::Packet::Start(sample),
            crate::input::Packet::End(sample),
        ],
    );
    finish_jobs(&mut ui.app);
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        64
    );
    let painted = ui.app.doc.clone();
    ui.key(egui::Key::Z, egui::Modifiers::CTRL);
    finish_jobs(&mut ui.app);
    assert_eq!(
        ui.app.doc.layers[0]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(16, 16),
        255
    );
    ui.key(egui::Key::Z, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    finish_jobs(&mut ui.app);
    assert!(crate::document::same_snapshot(&ui.app.doc, &painted));
    ui.settle();
    ui.click(ui.text("Mask actions"));
    ui.click(ui.text("Enable mask"));
    finish_jobs(&mut ui.app);
    assert!(!ui.app.doc.layers[0].mask.as_ref().unwrap().enabled);
    assert_eq!(
        ui.app.doc.composite_transparent(),
        [17, 83, 201, 200].repeat(32 * 32)
    );
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    ui.click(ui.text("Select"));
    ui.click(ui.text("From mask coverage"));
    finish_jobs(&mut ui.app);
    assert_eq!(ui.app.doc.selection_mask.as_ref().unwrap().at(16, 16), 64);
    ui.key(egui::Key::D, egui::Modifiers::CTRL);
    finish_jobs(&mut ui.app);
    ui.settle();
    ui.click(ui.text("Mask actions"));
    ui.click(ui.text("Enable mask"));
    finish_jobs(&mut ui.app);
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    ui.key(egui::Key::J, egui::Modifiers::CTRL);
    finish_jobs(&mut ui.app);
    assert_eq!(ui.app.doc.layers.len(), 2);
    let active = ui.app.doc.active;
    let copied = ui.app.doc.layers[active].mask.as_ref().unwrap().id;
    assert_ne!(copied, first_mask);
    ui.app.focus_current_mask(true);
    let before_transform = ui.app.doc.clone();
    ui.key(egui::Key::T, egui::Modifiers::CTRL);
    ui.app.affine_settings = crate::affine::Settings {
        translation: [2.0, 1.0],
        sampling: crate::affine::Sampling::Nearest,
        ..Default::default()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while crate::document::same_snapshot(&ui.app.doc, &before_transform) {
        ui.app.poll_affine_preview();
        assert!(std::time::Instant::now() < deadline, "{}", ui.app.status);
        std::thread::yield_now();
    }
    ui.settle();
    ui.click(ui.text("Apply"));
    finish_jobs(&mut ui.app);
    assert_eq!(
        ui.app.doc.layers[active]
            .mask
            .as_ref()
            .unwrap()
            .coverage
            .at(18, 17),
        64
    );
    assert_eq!(
        ui.app.doc.layers[active].pixels,
        before_transform.layers[active].pixels
    );
    let expected = serde_json::to_value(&ui.app.doc).unwrap();
    let composite = ui.app.doc.composite_transparent();
    ui.app.path = Some(project.clone());
    ui.app.save(false);
    ui.app.export_to(png.clone(), true);
    finish_jobs(&mut ui.app);
    assert!(!ui.app.modified);
    assert_eq!(image::open(&png).unwrap().into_rgba8().as_raw(), &composite);
    crate::storage::save_recovery(
        &recovery,
        &crate::storage::Recovery {
            document: ui.app.doc.clone(),
            original_path: Some(project.clone()),
        },
    )
    .unwrap();
    assert!(
        ui.app
            .request_replace(crate::document_actions::Action::New(3, 2, 300.0))
    );
    ui.app.load_document(project.clone(), false);
    finish_jobs(&mut ui.app);
    assert_eq!(serde_json::to_value(&ui.app.doc).unwrap(), expected);
    assert_eq!(ui.app.doc.composite_transparent(), composite);
    assert!(!ui.app.modified && !ui.app.history.can_undo());
    assert!(ui.app.input_stroke_target().is_raster());
    assert!(
        ui.app
            .request_replace(crate::document_actions::Action::New(3, 2, 300.0))
    );
    ui.app.load_recovery(recovery);
    finish_jobs(&mut ui.app);
    assert_eq!(serde_json::to_value(&ui.app.doc).unwrap(), expected);
    assert!(ui.app.modified && ui.app.path.is_none());
    assert_eq!(ui.app.doc.composite_transparent(), composite);
}

#[test]
fn adjustment_menu_preview_apply_cancel_and_short_workspace() {
    let mut ui = Workspace::sized([768.0, 420.0]);
    ui.app.doc = crate::document::Document::new(8, 8);
    ui.app.doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[120, 40, 20, 255]);
    let original = ui.app.doc.clone();
    ui.settle();
    ui.click(ui.text("Filter"));
    ui.click(ui.text("Brightness / contrast…"));
    ui.app.adjustment.as_mut().unwrap().settings =
        crate::adjustments::Settings::BrightnessContrast {
            brightness: 30.0,
            contrast: 0.0,
        };
    finish_jobs(&mut ui.app);
    ui.settle();
    ui.text("Brightness");
    ui.text("Contrast");
    ui.key(egui::Key::Z, egui::Modifiers::CTRL);
    assert!(ui.app.adjustment.is_some() && !ui.app.history.can_undo());
    ui.click(ui.text("Cancel"));
    assert!(ui.app.adjustment.is_none());
    assert!(crate::document::same_snapshot(&ui.app.doc, &original));
    ui.click(ui.text("Filter"));
    ui.click(ui.text("Hue / saturation…"));
    ui.app.adjustment.as_mut().unwrap().settings = crate::adjustments::Settings::HueSaturation {
        hue: 120.0,
        saturation: 0.0,
        lightness: 0.0,
    };
    finish_jobs(&mut ui.app);
    ui.settle();
    let expected = ui.app.doc.composite_transparent();
    ui.click(ui.text("Apply"));
    finish_jobs(&mut ui.app);
    assert!(ui.app.adjustment.is_none() && ui.app.history.can_undo());
    assert_eq!(ui.app.doc.composite_transparent(), expected);
    ui.app.undo();
    finish_jobs(&mut ui.app);
    assert_eq!(
        ui.app.doc.composite_transparent(),
        original.composite_transparent()
    );
    // A tall Levels panel must scroll above its pinned footer on a short screen.
    ui.size = [480.0, 320.0];
    ui.app.start_adjustment(crate::adjustments::Kind::Levels);
    finish_jobs(&mut ui.app);
    ui.settle();
    let cancel = ui.text("Cancel");
    assert!(cancel.y < 320.0);
    ui.click(cancel);
    assert!(ui.app.adjustment.is_none());
}

#[test]
fn brush_settings_remain_visible_in_short_scaled_workspace() {
    let mut ui = Workspace::sized([768.0, 420.0]);
    ui.click(ui.text("Round ink"));
    let at = ui.text("Brush settings…");
    assert!(at.y > 0.0 && at.y < 414.0);
    ui.click(at);
    assert!(ui.app.tool_panel_visible(&ui.ctx));
    assert!(!egui::Popup::is_any_open(&ui.ctx));
}

#[test]
fn canvas_size_dialog_supports_cancel_and_applies_dimensions_from_keyboard() {
    let mut ui = Workspace::sized([960.0, 700.0]);
    let before = serde_json::to_value(&ui.app.doc).unwrap();
    ui.click(ui.text("Canvas"));
    ui.click(ui.text("Canvas size…"));
    assert!(ui.app.canvas_resize.is_some());
    ui.click(ui.text("Cancel"));
    assert!(ui.app.canvas_resize.is_none());
    assert_eq!(serde_json::to_value(&ui.app.doc).unwrap(), before);
    assert!(!ui.app.history.can_undo());

    ui.click(ui.text("Canvas"));
    ui.click(ui.text("Canvas size…"));
    ui.click(ui.text("1024 px"));
    ui.key(egui::Key::A, egui::Modifiers::CTRL);
    ui.frame(vec![egui::Event::Text("800".into())]);
    ui.key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(ui.app.canvas_resize.unwrap().width, 800);
    ui.click(ui.text("Apply"));
    finish_jobs(&mut ui.app);
    assert_eq!([ui.app.doc.width, ui.app.doc.height], [800, 768]);
    assert!(ui.app.canvas_resize.is_none() && ui.app.canvas_edit_ticket.is_none());
    assert!(ui.app.history.can_undo());
    ui.app.undo();
    assert_eq!(serde_json::to_value(&ui.app.doc).unwrap(), before);
}

#[test]
fn selection_cropped_out_by_resize_can_be_deselected_from_the_menu() {
    let mut ui = Workspace::new();
    ui.app
        .request_replace(crate::document_actions::Action::New(5, 5, 300.0));
    ui.app.set_selection(Some(crate::selection::Selection {
        left: 3,
        top: 3,
        right: 5,
        bottom: 5,
    }));
    ui.app
        .dispatch_canvas_size(crate::document_edits::Edit::CanvasSize(
            crate::canvas_size::Settings {
                dpi: None,
                width: 1,
                height: 1,
                anchor: [0, 0],
            },
        ));
    finish_jobs(&mut ui.app);
    assert!(ui.app.doc.selection.is_none() && ui.app.doc.selection_mask.is_some());
    assert!(ui.app.status.contains("selection is empty"));
    ui.settle();
    ui.click(ui.text("Select"));
    ui.click(ui.text("Deselect"));
    finish_jobs(&mut ui.app);
    assert!(ui.app.doc.selection.is_none() && ui.app.doc.selection_mask.is_none());
}

#[test]
fn canvas_size_dialog_keeps_cancel_visible_on_a_short_scaled_viewport() {
    let mut ui = Workspace::sized([480.0, 320.0]);
    ui.click(ui.text("Canvas"));
    ui.click(ui.text("Canvas size…"));
    let at = ui.text("Cancel");
    assert!(at.x > 0.0 && at.x < 480.0 && at.y > 0.0 && at.y < 320.0);
    ui.click(at);
    assert!(ui.app.canvas_resize.is_none());
    assert!(!ui.app.modified && !ui.app.history.can_undo());
}

#[test]
fn compact_workspace_brush_and_hex_color_work_without_sidebars_or_artwork_shortcuts() {
    let mut ui = Workspace::new();
    assert!(!ui.app.tool_panel_visible(&ui.ctx));
    assert!(!ui.app.inspector_visible(&ui.ctx));
    ui.click(ui.text("Round ink"));
    ui.click(ui.text("G-pen"));
    assert_eq!(ui.app.brush.name, "G-pen");
    assert_eq!(ui.app.selected_brush, 1);
    assert!(!egui::Popup::is_any_open(&ui.ctx));
    ui.click(ui.text("G-pen"));
    ui.click(ui.text("Search names or tags"));
    assert!(ui.ctx.wants_keyboard_input());
    ui.key(egui::Key::A, egui::Modifiers::CTRL);
    ui.frame(vec![egui::Event::Text("pencil".into())]);
    ui.settle();
    assert!(egui::Popup::is_any_open(&ui.ctx));
    ui.click(ui.text("Pencil"));
    assert_eq!(ui.app.brush.name, "Pencil");
    assert!(!egui::Popup::is_any_open(&ui.ctx));
    ui.app.brush.size = 29.0;
    ui.click(ui.text("Pencil"));
    ui.click(ui.text("Pencil"));
    assert_eq!(ui.app.brush.size, ui.app.brushes[2].size);
    assert!(!egui::Popup::is_any_open(&ui.ctx));
    ui.click(ui.text("Color"));
    let _ = ui.text("Swatches");
    ui.click(ui.text("262C3B"));
    assert!(egui::Popup::is_any_open(&ui.ctx));
    assert!(ui.ctx.wants_keyboard_input());
    ui.key(egui::Key::A, egui::Modifiers::CTRL);
    ui.frame(vec![egui::Event::Text("F34".into())]);
    ui.key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(ui.app.color, [255, 51, 68]);
    assert!(
        ui.app.doc.selection.is_none(),
        "Ctrl+A in a color field must not select artwork"
    );
    assert!(!ui.app.modified && !ui.app.history.can_undo());
    // The popup owns Escape and tool keys, rather than editing the canvas.
    ui.app.select_all();
    let selection = ui.app.doc.selection;
    ui.key(egui::Key::E, egui::Modifiers::NONE);
    assert!(ui.app.tool == crate::tools::Tool::Brush);
    ui.key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(!egui::Popup::is_any_open(&ui.ctx));
    assert_eq!(ui.app.doc.selection, selection);
}

pub(crate) fn finish_jobs(app: &mut Dudel) {
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.adjustment_ticket.is_some()
        || app.retired_adjustment.is_some()
        || app.adjustment.as_ref().is_some_and(|p| !p.ready())
        || app.render_queue.busy()
        || app.save_job.is_some()
        || app.queued_save.is_some()
        || app.export_job.is_some()
        || app.queued_export.is_some()
        || app.load_job.is_some()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "Workflow did not settle: {}",
            app.status
        );
        let _ = ctx.run(Default::default(), |ctx| {
            app.pump_render_queue(ctx);
            app.poll_save(ctx);
            app.poll_export(ctx);
            app.poll_load(ctx);
            app.poll_adjustment(ctx);
        });
        std::thread::yield_now();
    }
}

fn draw(app: &mut Dudel, point: [f32; 2]) {
    assert!(app.begin_canvas_stroke(crate::stroke_buffer::Settings {
        target: crate::paint_target::Target::Raster,
        brush: app.brush.clone(),
        color: app.color,
        erase: false,
        layer: app.doc.active,
        symmetry: Default::default(),
    }));
    app.deliver_canvas_dabs(vec![crate::stroke::Sample {
        x: point[0],
        y: point[1],
        pressure: 1.0,
        ..Default::default()
    }]);
    app.finish_painted_stroke();
}

#[test]
fn layered_illustration_selection_move_undo_save_reopen_and_export_preserve_artwork() {
    use crate::document_edits::{Edit, Property, Target};
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("Illustration.dudel");
    let png = directory.path().join("Illustration.png");
    let mut app = Dudel::default();
    assert!(app.request_replace(crate::document_actions::Action::New(96, 80, 300.0)));
    app.choose_brush(2);
    app.brush.size = 6.0;
    app.color = [75, 80, 110];
    draw(&mut app, [22.0, 18.0]);
    // Creating and painting another layer must work before the first settles.
    app.new_layer();
    app.choose_brush(0);
    app.brush.size = 8.0;
    app.color = [20, 25, 35];
    draw(&mut app, [40.0, 35.0]);
    finish_jobs(&mut app);
    assert_eq!(app.doc.layers.len(), 2);
    assert_eq!(app.doc.active, 1);
    assert_ne!(app.doc.layers[0].pixels, app.doc.layers[1].pixels);
    app.renaming = Some(1);
    app.rename_draft = "Ink and color".into();
    app.finish_layer_rename(true);

    assert!(app.dispatch_document_edit(
        Edit::SelectionPolygon {
            points: vec![[30.0, 26.0], [68.0, 30.0], [59.0, 61.0], [33.0, 55.0]],
            mode: crate::selection_mask::Combine::Replace,
        },
        "Selected",
        "Unchanged"
    ));
    finish_jobs(&mut app);
    assert!(app.doc.selection_mask.is_some());
    let target = app.doc.layers[app.doc.active].id;
    assert!(app.dispatch_document_edit(
        Edit::FillTarget {
            target,
            source: crate::fill::Source::Active,
            point: [50, 43],
            color: [100, 170, 205],
            opacity: 1.0,
            tolerance: 0,
            options: Default::default(),
        },
        "Colored",
        "Unchanged"
    ));
    finish_jobs(&mut app);
    let before_move = app.doc.clone();
    assert!(app.dispatch_document_edit(
        Edit::MoveTarget {
            target,
            offset: [4, 3]
        },
        "Moved",
        "Unchanged"
    ));
    finish_jobs(&mut app);
    let after_move = app.doc.clone();
    assert_ne!(before_move.layers[1].pixels, after_move.layers[1].pixels);
    app.undo();
    finish_jobs(&mut app);
    assert_eq!(app.doc.layers[1].pixels, before_move.layers[1].pixels);
    assert_eq!(app.doc.selection_mask, before_move.selection_mask);
    app.redo();
    finish_jobs(&mut app);
    assert_eq!(app.doc.layers[1].pixels, after_move.layers[1].pixels);
    assert!(app.dispatch_document_edit(
        Edit::Property {
            target: Target::Identity(target),
            property: Property::Opacity(0.7),
        },
        "Opacity changed",
        "Unchanged"
    ));
    let before_group = app.doc.composite_transparent();
    app.group_into_folder();
    finish_jobs(&mut app);
    assert!(app.doc.layers.iter().any(|layer| layer.folder));
    assert_eq!(app.doc.composite_transparent(), before_group);

    app.grid.visible = true;
    app.guide_lines.push(crate::guides::GuideLine {
        vertical: true,
        position: 24.0,
    });
    app.track_guide_changes();
    let expected_guides = app.guides_record(&app.doc);
    let expected_document = serde_json::to_value(&app.doc).unwrap();
    let expected_png = app.doc.composite_transparent();
    app.path = Some(project.clone()); // Native save dialog's chosen destination.
    app.save(false);
    app.export_to(png.clone(), true);
    finish_jobs(&mut app);
    assert!(!app.modified);
    assert_eq!(app.path.as_ref(), Some(&project));
    assert_eq!(
        image::open(&png).unwrap().into_rgba8().as_raw(),
        &expected_png
    );

    assert!(app.request_replace(crate::document_actions::Action::New(3, 2, 300.0)));
    app.load_document(project.clone(), false);
    finish_jobs(&mut app);
    assert!(app.unsaved_action.is_none());
    assert_eq!(serde_json::to_value(&app.doc).unwrap(), expected_document);
    assert_eq!(app.guides_record(&app.doc), expected_guides);
    assert_eq!(app.path.as_ref(), Some(&project));
    assert!(!app.modified);
    assert!(
        !app.history.can_undo(),
        "A reopened project starts a new editing history"
    );
    app.export_to(directory.path().join("Reopened.png"), false);
    finish_jobs(&mut app);
    assert_eq!(
        image::open(directory.path().join("Reopened.png"))
            .unwrap()
            .into_rgba8()
            .as_raw(),
        &app.doc.composite()
    );
    assert!(!app.modified && app.path.as_ref() == Some(&project));
}

#[test]
fn toolbar_symmetry_popup_stays_open_while_its_label_grows() {
    // These widths fit the bar with "Symmetry" but not "Symmetry: Four-way";
    // a label naming the mode collapsed the bar and closed the popup.
    for width in [950.0, 1000.0, 1030.0, 1430.0, 1470.0] {
        let mut w = Workspace::sized([width, 800.0]);
        w.app.select_tool(crate::tools::Tool::Brush);
        w.settle();
        w.click(w.text("Symmetry"));
        w.click(w.text("Four-way"));
        assert!(egui::Popup::is_any_open(&w.ctx), "closed at {width}");
        w.click(w.text("Layer symmetry override"));
        w.click(w.text("Radial"));
        assert!(egui::Popup::is_any_open(&w.ctx), "closed at {width}");
        assert_eq!(w.app.active_symmetry().mode, crate::symmetry::Mode::Radial);
    }
}

#[test]
fn invert_selection_reverses_coverage_from_menu_shortcut_and_plugin() {
    let mut w = Workspace::sized([1280., 800.]);
    w.app.doc = crate::document::Document::new(4, 4);
    w.app.invert_selection();
    assert_eq!(w.app.status, "Nothing is selected to invert");
    let rect = crate::selection::Selection {
        left: 1,
        top: 1,
        right: 3,
        bottom: 3,
    };
    w.app.set_selection(Some(rect));
    w.app.flush_render_queue();
    w.settle();
    w.key(egui::Key::I, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    w.app.flush_render_queue();
    assert!(
        w.app.tool != crate::tools::Tool::Picker,
        "Ctrl+Shift+I is not I"
    );
    let mask = w.app.doc.selection_mask.clone().unwrap();
    for (x, y) in [(0, 0), (3, 3), (1, 1), (2, 2)] {
        let inside = (1..3).contains(&x) && (1..3).contains(&y);
        assert_eq!(mask.at(x, y), if inside { 0 } else { 255 }, "{x},{y}");
    }
    assert_eq!(
        w.app.doc.selection,
        Some(crate::selection::Selection {
            left: 0,
            top: 0,
            right: 4,
            bottom: 4
        })
    );
    // Soft edges stay soft: coverage 64 becomes 191.
    let mut coverage = vec![0; 16];
    coverage[5] = 64;
    w.app.dispatch_document_edit(
        crate::document_edits::Edit::SelectionMask(Some(
            crate::selection_mask::Mask::from_coverage(4, 4, coverage).unwrap(),
        )),
        "",
        "",
    );
    w.app.flush_render_queue();
    w.click(w.text("Select"));
    w.click(w.text("Invert selection"));
    w.app.flush_render_queue();
    assert_eq!(w.app.doc.selection_mask.as_ref().unwrap().at(1, 1), 191);
    assert_eq!(w.app.doc.selection_mask.as_ref().unwrap().at(0, 0), 255);
    w.app.undo();
    w.app.flush_render_queue();
    assert_eq!(w.app.doc.selection_mask.as_ref().unwrap().at(1, 1), 64);
    // Inverting everything leaves nothing selected.
    w.app.select_all();
    w.app.flush_render_queue();
    w.app.extensions.push(crate::extensions::Installed {
        package: serde_json::from_str(include_str!("../plugins/example-studio-kit.dudelplugin"))
            .unwrap(),
        enabled: true,
    });
    w.app
        .run_extension_command("dudel-studio-kit", "invert-selection");
    w.app.flush_render_queue();
    assert!(w.app.doc.selection.is_none() && w.app.doc.selection_mask.is_none());
}

#[test]
fn installed_filters_run_from_the_filter_menu() {
    let mut w = Workspace::sized([1280., 800.]);
    w.app.doc = crate::document::Document::new(1, 1);
    w.app.doc.layers[0].pixels = vec![10, 30, 200, 255].into();
    for (source, enabled) in [
        (
            include_str!("../plugins/example-wasm-invert.dudelplugin"),
            true,
        ),
        (
            include_str!("../plugins/example-studio-kit.dudelplugin"),
            true,
        ),
        (include_str!("../plugins/example-invert.dudelplugin"), false),
    ] {
        w.app.extensions.push(crate::extensions::Installed {
            package: serde_json::from_str(source).unwrap(),
            enabled,
        });
    }
    w.settle();
    w.click(w.text("Filter"));
    w.text("Invert colors");
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.text("Example invert")))
            .is_err(),
        "disabled packages are not listed"
    );
    w.click(w.text("WASM invert"));
    assert!(!egui::Popup::is_any_open(&w.ctx), "the menu closes");
    for _ in 0..200 {
        if w.app.filter_job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        w.app.poll_filter(&w.ctx);
    }
    assert!(w.app.filter_job.is_none(), "{}", w.app.status);
    w.app.flush_render_queue();
    assert_eq!(&w.app.doc.layers[0].pixels[..], &[245, 225, 55, 255]);
}

#[test]
fn new_canvas_saves_presets_and_remembers_recent_sizes() {
    let mut w = Workspace::new();
    w.app.open_new_canvas();
    w.settle();
    w.click(w.text("Full HD"));
    w.click(w.text("Save as preset…"));
    w.frame(vec![egui::Event::Text("Stream overlay".into())]);
    w.key(egui::Key::Enter, Default::default());
    assert!(w.app.new_canvas, "naming a preset does not create a canvas");
    let presets = &w.app.new_canvas_form.prefs.presets;
    assert_eq!(presets.len(), 1);
    assert_eq!(
        (
            presets[0].name.as_str(),
            presets[0].width,
            presets[0].height
        ),
        ("Stream overlay", 1920, 1080)
    );
    w.click(w.text("Sketch"));
    w.click(w.text("Stream overlay"));
    assert_eq!(w.app.new_canvas_form.dimensions(), Ok([1920, 1080]));
    w.click(w.text("Create canvas"));
    w.settle();
    assert!(!w.app.new_canvas);
    assert_eq!((w.app.doc.width, w.app.doc.height), (1920, 1080));
    assert_eq!(w.app.new_canvas_form.prefs.recent[0].width, 1920);
    // Presets and recent sizes survive a restart; invalid entries do not.
    let source = w.app.encode_preferences().unwrap();
    let mut restored = crate::Dudel::default();
    restored.restore_preferences_source(&source);
    assert_eq!(restored.new_canvas_form.prefs, w.app.new_canvas_form.prefs);
    let mut prefs = w.app.new_canvas_form.prefs.clone();
    prefs.presets[0].width = 0;
    prefs.recent[0].dpi = f64::NAN;
    let prefs = prefs.sanitized();
    assert!(prefs.presets.is_empty() && prefs.recent.is_empty());
}

#[test]
fn settings_search_finds_settings_across_pages_by_keyword() {
    let mut w = Workspace::sized([1280., 800.]);
    w.app.settings_open = true;
    w.settle();
    w.text("Show workspace panels");
    w.click(w.text("Search settings"));
    w.frame(vec![egui::Event::Text("wacom".into())]);
    w.settle();
    w.text("Use native pen input");
    let hidden = |w: &Workspace, text: &str| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.text(text))).is_err()
    };
    assert!(hidden(&w, "Show workspace panels"));
    assert!(hidden(&w, "Reset this page…"));
    w.click(w.text("✕"));
    w.text("Show workspace panels");
    w.click(w.text("Reset this page…"));
    assert!(w.app.dialogs.open(), "resetting asks first");
}
