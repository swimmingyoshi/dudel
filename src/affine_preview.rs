//! Frozen-source affine preview ownership and asynchronous finish.
use crate::{Dudel, document_edits::Edit, move_preview::Engine};

/// Application preview/finish timings without GUI or display work.
pub(crate) fn measure(
    source: &crate::document::Document,
    settings: crate::affine::Settings,
    pending: bool,
) -> Result<
    (
        crate::document::Document,
        crate::document::History,
        serde_json::Value,
    ),
    String,
> {
    measure_target(
        source,
        settings,
        pending,
        crate::paint_target::Target::Raster,
    )
}

pub(crate) fn measure_target(
    source: &crate::document::Document,
    settings: crate::affine::Settings,
    pending: bool,
    target: crate::paint_target::Target,
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
        affine_settings: if pending {
            crate::affine::Settings::default()
        } else {
            settings
        },
        ..Dudel::default()
    };
    let ctx = crate::egui::Context::default();
    if let crate::paint_target::Target::Mask(id) = target {
        app.mask_focus = Some((source.layers[source.active].id, id));
    }
    let wall = Instant::now();
    let start = Instant::now();
    app.start_affine_preview();
    app.poll_affine_preview();
    let preview_admission_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut preview_max_ms = preview_admission_ms;
    let mut preview_calls = 1;
    loop {
        let session = app
            .affine_preview
            .as_mut()
            .and_then(|p| p.session.as_mut())
            .ok_or_else(|| app.status.clone())?;
        if session.engine.finished() {
            break;
        }
        session.engine.wait_worker();
        let start = Instant::now();
        app.poll_affine_preview();
        preview_max_ms = preview_max_ms.max(start.elapsed().as_secs_f64() * 1000.0);
        preview_calls += 1;
        if preview_calls > 10000 {
            return Err("Affine preview probe did not finish".into());
        }
    }
    let preview_wall_ms = wall.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    if pending {
        app.affine_settings = settings;
        app.poll_affine_preview();
    }
    let final_request_ms = start.elapsed().as_secs_f64() * 1000.0;
    let worker_pending = app
        .affine_preview
        .as_ref()
        .and_then(|p| p.session.as_ref())
        .is_some_and(|s| s.engine.waiting_worker());
    let wall = Instant::now();
    let start = Instant::now();
    app.finish_affine_preview();
    let finish_admission_ms = start.elapsed().as_secs_f64() * 1000.0;
    if app.affine_preview.is_some() {
        return Err(app.status);
    }
    let mut owner_max_ms = 0.0_f64;
    let mut owner_calls = 0;
    while app.render_queue.busy() {
        if app.render_queue.waiting_edit() {
            app.render_queue.wait_edit();
        }
        let start = Instant::now();
        app.pump_render_queue(&ctx);
        owner_max_ms = owner_max_ms.max(start.elapsed().as_secs_f64() * 1000.0);
        owner_calls += 1;
        if owner_calls > 100000 {
            return Err("Affine finish probe did not finish".into());
        }
    }
    if app.history.has_pending_stroke() || !app.modified {
        return Err(format!("Affine probe failed: {}", app.status));
    }
    Ok((
        app.doc,
        app.history,
        serde_json::json!({"preview_admission_ms":preview_admission_ms,"preview_max_owner_ms":preview_max_ms,"preview_calls":preview_calls,"preview_wall_ms":preview_wall_ms,"final_request_ms":final_request_ms,"worker_pending_at_finish":worker_pending,"finish_admission_ms":finish_admission_ms,"finish_max_owner_ms":owner_max_ms,"finish_owner_calls":owner_calls,"finish_wall_ms":wall.elapsed().as_secs_f64()*1000.0}),
    ))
}

pub(crate) struct Preview {
    target: crate::document::LayerId,
    paint_target: crate::paint_target::Target,
    session: Option<Session>,
    drag: Option<HandleDrag>,
    /// The selection's pixels as a texture, drawn warped on the GPU while a
    /// handle is dragged so large transforms follow the pen instantly.
    overlay: Option<crate::egui::TextureHandle>,
    /// A drag started: the layer shows the selection lifted out and the
    /// overlay stands in until the full-quality result is published.
    overlay_active: bool,
}

/// Largest overlay texture side; the overlay is only a drag preview.
const OVERLAY_SIDE: usize = 1024;
/// Settings that move the selection entirely off the canvas, leaving the
/// layer with the selection lifted out. Nearly every sample falls outside,
/// so the engine computes this quickly.
fn lifted() -> crate::affine::Settings {
    crate::affine::Settings {
        translation: [1.0e6, 1.0e6],
        sampling: crate::affine::Sampling::Nearest,
        ..crate::affine::Settings::default()
    }
}

/// A separately painted mesh only reproduces a top-level Normal layer at full
/// opacity, with nothing above it. Other cases need the real stack compositor.
fn can_overlay(doc: &crate::document::Document) -> bool {
    let layer = &doc.layers[doc.active];
    !layer.folder
        && layer.depth == 0
        && layer.visible
        && layer.opacity == 1.0
        && !layer.csp_byte_opacity
        && !layer.clipped
        && layer.blend == crate::blend::BlendMode::Normal
        && doc.layers.iter().all(|l| l.mask.is_none())
        && doc.layers[doc.active + 1..].iter().all(|l| !l.visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        affine::{Sampling, Settings},
        document::Document,
        selection::Selection,
        selection_mask::Mask,
    };
    fn free_drag(handle: Handle, from: [f64; 2], to: [f64; 2], modifiers: Modifiers) -> Settings {
        let b = Selection {
            left: 100,
            top: 100,
            right: 300,
            bottom: 200,
        };
        dragged_settings(
            HandleDrag {
                handle,
                settings: Settings::default(),
                start: from,
            },
            to,
            b,
            modifiers,
            None,
        )
    }
    fn mapped(settings: Settings, point: [f64; 2]) -> [f64; 2] {
        let mut doc = Document::new(1000, 1000);
        doc.selection = Some(Selection {
            left: 100,
            top: 100,
            right: 300,
            bottom: 200,
        });
        settings.matrix(&doc).unwrap().point(point[0], point[1])
    }
    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }
    #[test]
    fn dragging_shows_the_gpu_overlay_and_release_renders_full_quality() {
        use crate::egui::{pos2, vec2};
        let mut app = app();
        app.start_affine_preview();
        ready(&mut app);
        let source = app.doc.clone();
        let ctx = crate::egui::Context::default();
        let view = crate::view::View {
            origin: pos2(300.0, 250.0),
            zoom: 20.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(4.0, 2.0),
        };
        let center = view.to_screen(pos2(3.0, 2.0));
        app.affine_handle_packet(view, center, true, false, true, Modifiers::default());
        app.poll_affine_preview();
        let _ = ctx.run(Default::default(), |ctx| app.prepare_transform_overlay(ctx));
        ready(&mut app);
        assert!(app.transform_overlay_active());
        // The layer shows the selection lifted out under the overlay.
        // Pixel (3, 1) of the 8-wide fixture lies inside the selection.
        let selected = 8 + 3;
        assert!(source.layers[0].pixels.rgba(selected)[3] > 0);
        assert_eq!(app.doc.layers[0].pixels.rgba(selected)[3], 0);
        let moved = view.to_screen(pos2(4.0, 2.0));
        app.affine_handle_packet(view, moved, false, false, true, Modifiers::default());
        app.poll_affine_preview();
        assert!(app.transform_overlay_active());
        // The overlay is a textured mesh placed exactly by the mapping.
        let overlay = app
            .affine_preview
            .as_ref()
            .unwrap()
            .overlay
            .as_ref()
            .unwrap()
            .id();
        let output = ctx.run(Default::default(), |ctx| {
            let painter = ctx.layer_painter(crate::egui::LayerId::background());
            app.paint_affine_handles(&painter, view);
        });
        let mesh = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                crate::egui::Shape::Mesh(mesh) if mesh.texture_id == overlay => Some(mesh.clone()),
                _ => None,
            })
            .expect("overlay mesh painted");
        let b = bounds(&source);
        let mapping = app.affine_settings.mapping(&source).unwrap();
        let [x, y] = mapping.forward(b, b.left as f64, b.top as f64);
        let first = mesh.vertices[0];
        assert_eq!(first.uv, pos2(0.0, 0.0));
        assert!((first.pos - view.to_screen(pos2(x as f32, y as f32))).length() < 1e-3);
        assert_eq!(mesh.vertices.last().unwrap().uv, pos2(1.0, 1.0));
        app.affine_handle_packet(view, moved, false, true, true, Modifiers::default());
        ready(&mut app);
        assert!(
            !app.transform_overlay_active(),
            "full quality replaces the overlay"
        );
        let mut expected = source.clone();
        let mapping = app.affine_settings.mapping(&source).unwrap();
        crate::affine::apply_mapping(&mut expected, mapping, app.affine_settings.sampling, None)
            .unwrap();
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        app.cancel_affine_preview();
        assert!(crate::document::same_snapshot(&app.doc, &source));
    }
    #[test]
    fn warp_drags_move_corners_partners_and_mesh_points() {
        use crate::affine::Warp;
        let corners = [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
        let base = Settings {
            warp: Some(Warp::Corners(corners)),
            ..Settings::default()
        };
        let drag = |handle, to: [f64; 2], mode, shift| {
            let start = match handle {
                Handle::Point(i) => corners[i],
                _ => [50.0, 25.0],
            };
            let drag = HandleDrag {
                handle,
                settings: base,
                start,
            };
            warp_points(warp_dragged(
                drag,
                to,
                mode,
                Modifiers { shift, alt: false },
            ))
        };
        // Distort: only the dragged corner moves.
        let points = drag(Handle::Point(2), [120.0, 70.0], Mode::Distort, false);
        assert_eq!(
            points,
            vec![[0.0, 0.0], [100.0, 0.0], [120.0, 70.0], [0.0, 50.0]]
        );
        // Perspective: moving the top-right corner down pulls bottom-right up.
        let points = drag(Handle::Point(1), [102.0, 10.0], Mode::Perspective, false);
        assert_eq!(
            points,
            vec![[0.0, 0.0], [100.0, 10.0], [100.0, 40.0], [0.0, 50.0]]
        );
        // ...and sideways moves the top-left the opposite way.
        let points = drag(Handle::Point(1), [110.0, 1.0], Mode::Perspective, false);
        assert_eq!(
            points,
            vec![[-10.0, 0.0], [110.0, 0.0], [100.0, 50.0], [0.0, 50.0]]
        );
        // Moving and rotating apply to every point.
        let points = drag(Handle::Translate, [60.0, 26.0], Mode::Distort, true);
        assert_eq!(points[0], [10.0, 0.0]);
        // Shift snaps a quarter turn about the centre (50, 25).
        let turned = warp_points(warp_dragged(
            HandleDrag {
                handle: Handle::Rotate,
                settings: base,
                start: [150.0, 25.0],
            },
            [52.0, 125.0],
            Mode::Distort,
            Modifiers {
                shift: true,
                alt: false,
            },
        ));
        assert!(
            (turned[0][0] - 75.0).abs() < 1e-9 && (turned[0][1] + 25.0).abs() < 1e-9,
            "{turned:?}"
        );
        // Mesh: one control point.
        let grid = crate::affine::Mesh::from_fn(4, 4, |column, row| {
            [column as f64 * 10.0, row as f64 * 10.0]
        });
        let mesh = Settings {
            warp: Some(Warp::Mesh(grid)),
            ..Settings::default()
        };
        let moved = warp_points(warp_dragged(
            HandleDrag {
                handle: Handle::Point(5),
                settings: mesh,
                start: [10.0, 10.0],
            },
            [14.0, 7.0],
            Mode::Mesh,
            Modifiers::default(),
        ));
        assert_eq!(moved[5], [14.0, 7.0]);
        assert_eq!(moved[6], [20.0, 10.0]);
    }
    #[test]
    fn distort_on_canvas_previews_applies_and_undoes() {
        use crate::egui::{pos2, vec2};
        let mut app = app();
        app.start_affine_preview();
        ready(&mut app);
        let source = app.doc.clone();
        app.set_transform_mode(Mode::Distort);
        assert_eq!(app.transform_mode, Mode::Distort);
        // Scale/rotate is refused while a warp is active.
        app.set_transform_mode(Mode::ScaleRotate);
        assert_eq!(app.transform_mode, Mode::Distort);
        let view = crate::view::View {
            origin: pos2(300.0, 250.0),
            zoom: 20.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(4.0, 2.0),
        };
        let Some(crate::affine::Warp::Corners(corners)) = app.affine_settings.warp else {
            panic!("distort starts from the current corners");
        };
        let grab = view.to_screen(pos2(corners[2][0] as f32, corners[2][1] as f32));
        app.affine_handle_packet(view, grab, true, false, true, Modifiers::default());
        let target = view.to_screen(pos2(5.0, 3.6));
        app.affine_handle_packet(view, target, false, true, true, Modifiers::default());
        let Some(crate::affine::Warp::Corners(moved)) = app.affine_settings.warp else {
            panic!("still distorting");
        };
        assert!((moved[2][0] - 5.0).abs() < 1e-4 && (moved[2][1] - 3.6).abs() < 1e-4);
        assert_eq!(moved[0], corners[0]);
        ready(&mut app);
        assert!(
            !crate::document::same_snapshot(&app.doc, &source),
            "preview shows the warp"
        );
        // Mesh keeps the distorted outline.
        app.set_transform_mode(Mode::Mesh);
        let Some(crate::affine::Warp::Mesh(grid)) = app.affine_settings.warp else {
            panic!("mesh from the distorted shape");
        };
        let corner = grid.at(3, 3);
        assert!((corner[0] - 5.0).abs() < 1e-6 && (corner[1] - 3.6).abs() < 1e-6);
        // A finer grid resamples the same outline.
        app.set_mesh_grid(6, 3);
        let Some(crate::affine::Warp::Mesh(fine)) = app.affine_settings.warp else {
            panic!("still a mesh");
        };
        assert_eq!((fine.columns(), fine.rows()), (6, 3));
        let corner = fine.at(5, 2);
        assert!((corner[0] - 5.0).abs() < 1e-6 && (corner[1] - 3.6).abs() < 1e-6);
        let json = serde_json::to_string(&app.affine_settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        let Some(crate::affine::Warp::Mesh(read)) = back.warp else {
            panic!("mesh survives serialization");
        };
        assert_eq!((read.columns(), read.rows()), (6, 3));
        assert!(
            read.points()
                .iter()
                .zip(fine.points())
                .all(|(a, b)| { (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 })
        );
        app.finish_affine_preview();
        app.flush_render_queue();
        let applied = app.doc.clone();
        assert!(!crate::document::same_snapshot(&applied, &source));
        app.undo();
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
    }
    #[test]
    fn free_transform_corners_and_sides_scale_from_the_opposite_edge() {
        let plain = Modifiers::default();
        // Bottom-right corner: the top-left corner stays put.
        let s = free_drag(Handle::Scale([1, 1]), [300.0, 200.0], [400.0, 250.0], plain);
        assert!(close(
            [s.scale_percent[0], s.scale_percent[1]],
            [150.0, 150.0]
        ));
        assert!(close(mapped(s, [100.0, 100.0]), [100.0, 100.0]));
        assert!(close(mapped(s, [300.0, 200.0]), [400.0, 250.0]));
        // Right side: only width changes; the left edge and vertical center stay.
        let s = free_drag(Handle::Scale([1, 0]), [300.0, 150.0], [350.0, 150.0], plain);
        assert!(close(
            [s.scale_percent[0], s.scale_percent[1]],
            [125.0, 100.0]
        ));
        assert!(close(mapped(s, [100.0, 150.0]), [100.0, 150.0]));
        // Shift keeps proportions using the larger change.
        let shift = Modifiers {
            shift: true,
            alt: false,
        };
        let s = free_drag(Handle::Scale([1, 1]), [300.0, 200.0], [400.0, 210.0], shift);
        assert!(close(
            [s.scale_percent[0], s.scale_percent[1]],
            [150.0, 150.0]
        ));
        assert!(close(mapped(s, [100.0, 100.0]), [100.0, 100.0]));
        // Alt scales about the center.
        let alt = Modifiers {
            shift: false,
            alt: true,
        };
        let s = free_drag(Handle::Scale([1, 1]), [300.0, 200.0], [350.0, 225.0], alt);
        assert!(close(
            [s.scale_percent[0], s.scale_percent[1]],
            [150.0, 150.0]
        ));
        assert!(close(mapped(s, [200.0, 150.0]), [200.0, 150.0]));
        // Dragging a side past the opposite edge flips instead of collapsing.
        let s = free_drag(Handle::Scale([1, 0]), [300.0, 150.0], [50.0, 150.0], plain);
        assert!(s.flip[0] && !s.flip[1]);
        assert!(close(
            [s.scale_percent[0], s.scale_percent[1]],
            [25.0, 100.0]
        ));
        assert!(close(mapped(s, [100.0, 150.0]), [100.0, 150.0]));
        assert!(close(mapped(s, [300.0, 150.0]), [50.0, 150.0]));
    }
    #[test]
    fn free_transform_rotation_snaps_and_moves_snap_to_guides() {
        let shift = Modifiers {
            shift: true,
            alt: false,
        };
        // Center (200, 150); a quarter turn plus a little snaps to 90 degrees.
        let s = free_drag(Handle::Rotate, [300.0, 150.0], [205.0, 260.0], shift);
        assert!((s.angle_degrees - 90.0).abs() < 1e-9, "{}", s.angle_degrees);
        let snap = Snap {
            xs: vec![500.0],
            ys: vec![],
            grid: None,
            tolerance: 8.0,
        };
        let b = Selection {
            left: 100,
            top: 100,
            right: 300,
            bottom: 200,
        };
        let drag = HandleDrag {
            handle: Handle::Translate,
            settings: Settings::default(),
            start: [200.0, 150.0],
        };
        // The right edge lands 5px short of the guide and snaps onto it.
        let s = dragged_settings(drag, [395.0, 151.0], b, Modifiers::default(), Some(&snap));
        assert_eq!(s.translation, [200.0, 1.0]);
        // Shift moves along the dominant axis only.
        let s = dragged_settings(drag, [260.0, 170.0], b, shift, None);
        assert_eq!(s.translation, [60.0, 0.0]);
        // A flipped box picks the handle under the pointer and winds either way.
        let flipped = Settings {
            flip: [true, false],
            ..Settings::default()
        };
        let view = crate::view::View {
            origin: crate::egui::pos2(500.0, 500.0),
            zoom: 1.0,
            rotation: 0.0,
            mirror: false,
            half: crate::egui::vec2(500.0, 500.0),
        };
        let mut doc = Document::new(1000, 1000);
        doc.selection = Some(b);
        let h = handles(&doc, flipped, view).unwrap();
        assert_eq!(
            pick(&h, view.to_screen(crate::egui::pos2(200.0, 150.0))),
            Handle::Translate
        );
        assert_eq!(
            pick(&h, view.to_screen(crate::egui::pos2(600.0, 600.0))),
            Handle::Rotate
        );
        assert_eq!(pick(&h, h.sides[1]), Handle::Scale([1, 0]));
    }
    fn app() -> Dudel {
        let mut app = Dudel {
            doc: Document::new(8, 4),
            ..Dudel::default()
        };
        let mut coverage = vec![0; 32];
        for y in 1..3 {
            for x in 2..4 {
                coverage[y * 8 + x] = if x == 2 { 128 } else { 255 };
                app.doc.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(y * 8 + x)
                    .copy_from_slice(&[200, 30, 70, if y == 2 { 0 } else { 255 }]);
            }
        }
        let mask = Mask::from_coverage(8, 4, coverage).unwrap();
        app.doc.selection = mask.bounds();
        app.doc.selection_mask = Some(mask);
        app.affine_settings.sampling = Sampling::Nearest;
        app
    }
    fn ready(app: &mut Dudel) {
        for _ in 0..100 {
            app.poll_affine_preview();
            let session = app
                .affine_preview
                .as_mut()
                .expect("preview retained")
                .session
                .as_mut();
            if let Some(session) = session {
                if session.engine.finished() {
                    return;
                }
                session.engine.wait_worker();
            }
        }
        panic!("preview did not finish");
    }
    #[test]
    fn mirror_pair_preview_cancel_pending_apply_and_single_undo_preserve_both_artworks() {
        for pending in [false, true] {
            for cancel in [false, true] {
                let mut app = app();
                for y in 1..3 {
                    for x in 4..6 {
                        app.doc.layers[0]
                            .pixels
                            .writer()
                            .rgba_mut(y * 8 + x)
                            .copy_from_slice(&[30, 170, 220, 128]);
                    }
                }
                app.affine_settings.mirror = Some(crate::affine::Mirror {
                    center: [4., 2.],
                    angle_degrees: 0.,
                });
                let original = app.doc.clone();
                app.start_affine_preview();
                ready(&mut app);
                app.affine_settings.translation = [-1., 0.];
                app.poll_affine_preview();
                if !pending {
                    ready(&mut app);
                }
                if cancel {
                    app.cancel_affine_preview();
                    assert!(crate::document::same_snapshot(&app.doc, &original));
                    assert!(!app.history.can_undo());
                    assert!(!app.modified);
                } else {
                    let mut expected = original.clone();
                    crate::affine::apply_settings(
                        &mut expected,
                        app.affine_settings,
                        crate::paint_target::Target::Raster,
                        None,
                    )
                    .unwrap();
                    app.finish_affine_preview();
                    app.flush_render_queue();
                    assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
                    assert_eq!(app.doc.selection_mask, expected.selection_mask);
                    app.undo();
                    assert_eq!(app.doc.layers[0].pixels, original.layers[0].pixels);
                    assert_eq!(app.doc.selection_mask, original.selection_mask);
                    assert!(!app.history.can_undo());
                    app.redo();
                    assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
                    assert_eq!(app.doc.selection_mask, expected.selection_mask);
                }
            }
        }
    }
    #[test]
    fn layer_mask_drag_uses_full_quality_worker_and_cancel_restores_shared_coverage() {
        use crate::egui::{pos2, vec2};
        let mut app = app();
        app.doc.selection = None;
        app.doc.selection_mask = None;
        app.doc.version = 2;
        let mut coverage = crate::layer_mask::Mask::new(8, 4, 255).unwrap();
        coverage.writer().set(2, 1, 0);
        app.doc.layers[0].mask = Some(crate::layer_mask::LayerMask {
            id: crate::document::LayerId::new(),
            enabled: true,
            linked: true,
            coverage,
        });
        let source = app.doc.clone();
        app.start_affine_preview();
        ready(&mut app);
        let view = crate::view::View {
            origin: pos2(300.0, 250.0),
            zoom: 20.0,
            rotation: 0.0,
            mirror: false,
            half: vec2(4.0, 2.0),
        };
        let center = view.to_screen(pos2(4.0, 2.0));
        app.affine_handle_packet(view, center, true, false, true, Modifiers::default());
        app.affine_settings.translation = [2.0, 0.0];
        app.poll_affine_preview();
        let ctx = crate::egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.prepare_transform_overlay(ctx));
        ready(&mut app);
        assert!(!app.transform_overlay_active());
        assert_eq!(
            app.doc.layers[0].mask.as_ref().unwrap().coverage.at(4, 1),
            0
        );
        assert_eq!(
            app.doc.layers[0].pixels.rgba(12),
            source.layers[0].pixels.rgba(10)
        );
        app.cancel_affine_preview();
        assert!(crate::document::same_snapshot(&app.doc, &source));
        assert!(!app.modified && !app.history.can_undo());
    }
    #[test]
    fn contextual_drag_matches_the_composite_and_keeps_cancel_and_undo_exact() {
        use crate::egui::{pos2, vec2};
        for case in [
            "blend",
            "clip",
            "opacity",
            "upper",
            "folder",
            "byte opacity",
            "reference",
        ] {
            for cancel in [false, true] {
                let mut app = app();
                let mut base = crate::document::Layer::raster("Backdrop".into(), 8, 4);
                base.pixels
                    .writer()
                    .write_region(8, [0, 0, 8, 4], &[80, 150, 220, 255].repeat(32));
                app.doc.layers.insert(0, base);
                app.doc.active = 1;
                match case {
                    "blend" => app.doc.layers[1].blend = crate::blend::BlendMode::Multiply,
                    "clip" => app.doc.layers[1].clipped = true,
                    "opacity" => app.doc.layers[1].opacity = 0.4,
                    "upper" => {
                        let mut upper = crate::document::Layer::raster("Foreground".into(), 8, 4);
                        upper.pixels.writer().write_region(
                            8,
                            [4, 1, 6, 3],
                            &[20, 200, 60, 200].repeat(4),
                        );
                        app.doc.layers.push(upper);
                    }
                    "folder" => {
                        app.doc.layers[1].depth = 1;
                        let mut folder = crate::document::Layer::new_folder("Group".into());
                        folder.opacity = 0.5;
                        app.doc.layers.push(folder);
                    }
                    "byte opacity" => app.doc.layers[1].csp_byte_opacity = true,
                    "reference" => {
                        app.insert_reference(
                            "Study".into(),
                            crate::reference_layers::Image {
                                width: 8,
                                height: 4,
                                rgba: [20, 200, 60, 255].repeat(32),
                            },
                        );
                        app.reference_layers.active = None;
                    }
                    _ => unreachable!(),
                }
                let source = app.doc.clone();
                let modified_before = app.modified;
                app.start_affine_preview();
                ready(&mut app);
                let view = crate::view::View {
                    origin: pos2(300.0, 250.0),
                    zoom: 20.0,
                    rotation: 0.0,
                    mirror: false,
                    half: vec2(4.0, 2.0),
                };
                app.affine_handle_packet(
                    view,
                    view.to_screen(pos2(3.0, 2.0)),
                    true,
                    false,
                    true,
                    Modifiers::default(),
                );
                app.affine_settings.translation = [2.0, 0.0];
                app.poll_affine_preview();
                let ctx = crate::egui::Context::default();
                let _ = ctx.run(Default::default(), |ctx| app.prepare_transform_overlay(ctx));
                ready(&mut app);
                assert!(!app.transform_overlay_active(), "{case}");
                assert!(
                    app.affine_preview.as_ref().unwrap().overlay.is_none(),
                    "{case}: no unused overlay upload"
                );
                let mut expected = source.clone();
                crate::affine::apply(
                    &mut expected,
                    app.affine_settings.matrix(&source).unwrap(),
                    app.affine_settings.sampling,
                    None,
                )
                .unwrap();
                assert_eq!(
                    app.doc.composite(),
                    expected.composite(),
                    "{case}: drag composite"
                );
                assert_eq!(
                    app.doc.layers[1].pixels, expected.layers[1].pixels,
                    "{case}: drag pixels"
                );
                if cancel {
                    app.cancel_affine_preview();
                    assert!(
                        !app.history.can_undo() && app.modified == modified_before,
                        "{case}"
                    );
                } else {
                    app.finish_affine_preview();
                    app.flush_render_queue();
                    assert_eq!(
                        app.doc.composite(),
                        expected.composite(),
                        "{case}: applied composite"
                    );
                    assert!(app.history.undo(&mut app.doc), "{case}");
                    assert!(!app.history.can_undo(), "{case}: one undo action");
                }
                // Compact undo restores byte content, not Arc identity.
                assert_eq!(
                    serde_json::to_value(&app.doc).unwrap(),
                    serde_json::to_value(&source).unwrap(),
                    "{case}: restored"
                );
                assert!(
                    app.doc
                        .layers
                        .iter()
                        .zip(&source.layers)
                        .all(|(a, b)| a.id == b.id)
                );
            }
        }
    }
    #[test]
    fn affine_coalesces_gated_requests_and_always_samples_frozen_original() {
        let mut app = app();
        let source = app.doc.clone();
        app.start_affine_preview();
        ready(&mut app);
        let old = Settings {
            translation: [1.0, 0.0],
            ..app.affine_settings
        };
        let send = app
            .affine_preview
            .as_mut()
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .engine
            .gate_affine(old);
        app.affine_settings = Settings {
            scale_percent: [150.0, 75.0],
            angle_degrees: 31.0,
            translation: [2.0, 0.0],
            sampling: Sampling::Bilinear,
            flip: [false; 2],
            warp: None,
            mirror: None,
        };
        app.poll_affine_preview(); // Cancels gated old generation without joining.
        assert!(crate::document::same_snapshot(&source, &app.doc));
        send.send(()).unwrap();
        ready(&mut app);
        let mut expected = source.clone();
        crate::affine::apply(
            &mut expected,
            app.affine_settings.matrix(&source).unwrap(),
            Sampling::Bilinear,
            None,
        )
        .unwrap();
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, expected.selection_mask);
        assert!(app.history.has_pending_stroke());
        app.affine_settings = old;
        ready(&mut app);
        let mut expected = source.clone();
        crate::affine::apply(
            &mut expected,
            old.matrix(&source).unwrap(),
            old.sampling,
            None,
        )
        .unwrap();
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, expected.selection_mask);
    }
    #[test]
    fn cancel_or_identity_finish_restores_source_redo_and_unsaved_state() {
        for cancel in [false, true] {
            let mut app = app();
            app.history.checkpoint(&app.doc);
            app.doc.layers[0]
                .pixels
                .writer()
                .rgba_mut(7)
                .copy_from_slice(&[9, 7, 5, 255]);
            app.history.settle(&app.doc);
            assert!(app.history.undo(&mut app.doc));
            let source = app.doc.clone();
            app.affine_settings.translation = [2.0, 0.0];
            app.start_affine_preview();
            ready(&mut app);
            assert!(app.modified);
            if cancel {
                app.cancel_affine_preview();
            } else {
                app.affine_settings = Settings::default();
                ready(&mut app);
                app.finish_affine_preview();
                app.flush_render_queue();
            }
            assert!(crate::document::same_snapshot(&app.doc, &source));
            assert!(!app.modified && !app.history.has_pending_stroke() && !app.history.can_undo());
            assert!(app.history.redo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels.rgba(7), [9, 7, 5, 255]);
        }
    }
    #[test]
    fn affine_finish_returns_with_gated_worker_and_orders_later_edit_and_undo() {
        let mut app = app();
        let source = app.doc.clone();
        app.affine_settings.translation = [1.0, 0.0];
        app.start_affine_preview();
        ready(&mut app);
        app.affine_settings.translation = [2.0, 0.0];
        let send = app
            .affine_preview
            .as_mut()
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .engine
            .gate_affine(app.affine_settings);
        app.finish_affine_preview();
        assert!(app.affine_preview.is_none() && app.render_queue.waiting_edit());
        app.dispatch_document_edit(Edit::Selection(None), "Deselected", "Unchanged");
        send.send(()).unwrap();
        app.flush_render_queue();
        let mut expected = source.clone();
        crate::affine::apply(
            &mut expected,
            app.affine_settings.matrix(&source).unwrap(),
            app.affine_settings.sampling,
            None,
        )
        .unwrap();
        assert_eq!(app.doc.layers[0].pixels, expected.layers[0].pixels);
        assert!(app.doc.selection.is_none() && app.doc.selection_mask.is_none());
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.selection_mask, expected.selection_mask);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
        assert_eq!(app.doc.selection_mask, source.selection_mask);
        assert!(!app.history.can_undo());
    }
    #[test]
    fn pending_acquisition_waits_for_selection_and_can_finish_or_cancel_without_source() {
        for cancel in [false, true] {
            let mut app = app();
            let selection = Selection {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            };
            app.dispatch_document_edit(Edit::Selection(Some(selection)), "Selected", "Unchanged");
            // Force the selection itself behind a queued metadata command.
            app.render_queue
                .edit(Edit::Selection(Some(selection)))
                .unwrap();
            app.affine_settings.scale_percent = [200.0, 100.0];
            app.start_affine_preview();
            app.poll_affine_preview();
            assert!(app.affine_preview.as_ref().unwrap().session.is_none());
            if cancel {
                app.cancel_affine_preview();
            } else {
                app.finish_affine_preview();
            }
            app.flush_render_queue();
            assert!(app.affine_preview.is_none());
            assert_eq!(app.doc.selection.unwrap().right, if cancel { 2 } else { 3 });
        }
    }
    #[test]
    fn stale_preview_rejects_equal_reallocated_mask_and_preserves_newer_state() {
        let mut app = app();
        app.affine_settings.translation = [1.0, 0.0];
        app.start_affine_preview();
        ready(&mut app);
        let mask = app.doc.selection_mask.as_ref().unwrap();
        let bytes = (0..4)
            .flat_map(|y| (0..8).map(move |x| mask.at(x, y)))
            .collect();
        app.doc.selection_mask = Some(Mask::from_coverage(8, 4, bytes).unwrap());
        let newer = app.doc.clone();
        app.cancel_affine_preview();
        assert!(crate::document::same_snapshot(&app.doc, &newer));
        assert!(app.status.contains("source changed"));
        assert!(!app.history.has_pending_stroke());
    }
    #[test]
    fn canvas_handles_scale_translate_rotate_in_mirrored_rotated_view_and_keep_preview_open_on_lift()
     {
        use crate::egui::{pos2, vec2};
        let mut app = app();
        app.start_affine_preview();
        ready(&mut app);
        let view = crate::view::View {
            origin: pos2(300.0, 250.0),
            zoom: 8.0,
            rotation: 0.8,
            mirror: true,
            half: vec2(4.0, 2.0),
        };
        let source = app.doc.clone();
        let h = handles(&source, app.affine_settings, view).unwrap();
        let plain = Modifiers::default();
        app.affine_handle_packet(view, h.corners[2], true, false, true, plain);
        let destination = view.to_screen(pos2(5.0, 3.5));
        app.affine_handle_packet(view, destination, false, true, true, plain);
        // The opposite corner (2, 1) anchors the 2x2 selection.
        assert!((app.affine_settings.scale_percent[0] - 150.0).abs() < 0.001);
        assert!((app.affine_settings.scale_percent[1] - 125.0).abs() < 0.001);
        assert!(app.affine_preview.is_some());
        ready(&mut app);
        let h = handles(&source, app.affine_settings, view).unwrap();
        app.affine_handle_packet(view, h.center, true, false, true, plain);
        app.affine_handle_packet(
            view,
            view.to_screen(pos2(4.0, 1.1)),
            false,
            true,
            true,
            plain,
        );
        assert!((app.affine_settings.translation[0] - 1.0).abs() < 0.001);
        // Unsnapped the box would move to -0.9 with its top edge 0.15px above
        // the canvas; the edge snaps onto it.
        assert!((app.affine_settings.translation[1] + 0.75).abs() < 0.001);
        let h = handles(&source, app.affine_settings, view).unwrap();
        let center = view.to_doc(h.center);
        let start = view.to_doc(h.rotation) - center;
        let endpoint = center + vec2(-start.y, start.x);
        app.affine_handle_packet(view, h.rotation, true, false, true, plain);
        app.affine_handle_packet(view, view.to_screen(endpoint), false, true, true, plain);
        assert!((app.affine_settings.angle_degrees - 90.0).abs() < 0.001);
        app.cancel_affine_preview();
        assert!(crate::document::same_snapshot(&app.doc, &source));
    }
    #[test]
    fn native_canvas_packets_drag_a_transform_handle_and_cancel_restores_source() {
        use crate::{egui, input::Packet, stroke::Sample, tools::Tool};
        let mut app = app();
        app.tool = Tool::Move;
        app.zoom = 24.0;
        let source = app.doc.clone();
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[]));
        app.start_affine_preview();
        ready(&mut app);
        let view = app.view(app.canvas_area);
        let sample = |x| {
            let p = view.to_screen(egui::pos2(x, 2.0));
            Sample {
                x: p.x,
                y: p.y,
                pressure: 1.0,
                ..Default::default()
            }
        };
        let first = sample(3.0);
        let last = sample(4.0);
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
        assert!((app.affine_settings.translation[0] - 1.0).abs() < 0.001);
        assert!(app.move_drag.is_none() && app.affine_preview.is_some());
        ready(&mut app);
        assert!(app.modified);
        let _ = ctx.run(raw(), |ctx| app.canvas(ctx, &[Packet::Cancel]));
        assert!(app.affine_preview.is_none());
        assert!(crate::document::same_snapshot(&app.doc, &source));
        assert!(!app.modified && !app.history.can_undo());
    }
    #[test]
    fn canceling_gated_affine_finish_restores_shown_preview_before_following_edit() {
        let mut app = app();
        let source = app.doc.clone();
        app.affine_settings.translation = [1.0, 0.0];
        app.start_affine_preview();
        ready(&mut app);
        app.affine_settings.translation = [2.0, 0.0];
        let send = app
            .affine_preview
            .as_mut()
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .engine
            .gate_affine(app.affine_settings);
        app.finish_affine_preview();
        assert!(app.render_queue.cancel_background_edit());
        app.dispatch_document_edit(Edit::Selection(None), "Deselected", "Unchanged");
        app.flush_render_queue();
        assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
        assert!(app.doc.selection.is_none() && app.modified);
        assert!(app.history.undo(&mut app.doc));
        assert_eq!(app.doc.selection_mask, source.selection_mask);
        assert!(!app.history.can_undo() && !app.history.has_pending_stroke());
        send.send(()).unwrap();
    }
    #[test]
    fn save_waits_for_final_affine_generation_and_round_trips_pixels_and_coverage() {
        for shown in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("finished-affine.dudel");
            let mut app = app();
            let source = app.doc.clone();
            app.path = Some(path.clone());
            if shown {
                app.affine_settings.translation = [1.0, 0.0];
            }
            app.start_affine_preview();
            ready(&mut app);
            app.affine_settings = Settings {
                angle_degrees: 90.0,
                translation: [2.0, 0.0],
                sampling: Sampling::Nearest,
                ..Settings::default()
            };
            let send = app
                .affine_preview
                .as_mut()
                .unwrap()
                .session
                .as_mut()
                .unwrap()
                .engine
                .gate_affine(app.affine_settings);
            app.save(false);
            assert!(
                app.affine_preview.is_none()
                    && app.queued_save.is_some()
                    && app.save_job.is_none()
                    && app.render_queue.waiting_edit()
            );
            send.send(()).unwrap();
            app.flush_render_queue();
            app.save_job.take().unwrap().worker.join().unwrap().unwrap();
            let saved = crate::storage::load_project(&path).unwrap();
            let mut expected = source.clone();
            crate::affine::apply(
                &mut expected,
                app.affine_settings.matrix(&source).unwrap(),
                Sampling::Nearest,
                None,
            )
            .unwrap();
            assert_eq!(saved.layers[0].pixels, expected.layers[0].pixels);
            assert_eq!(saved.selection_mask, expected.selection_mask);
            assert_eq!(saved.selection, expected.selection);
            assert!(app.history.undo(&mut app.doc));
            assert_eq!(app.doc.layers[0].pixels, source.layers[0].pixels);
            assert_eq!(app.doc.selection_mask, source.selection_mask);
            assert!(!app.history.can_undo());
        }
    }
}
/// Shift and Alt while dragging a free-transform handle.
#[derive(Clone, Copy, Default)]
pub(crate) struct Modifiers {
    /// Keep proportions, snap rotation to 15 degrees, or move along one axis.
    pub shift: bool,
    /// Scale about the center instead of the opposite edge.
    pub alt: bool,
}
/// Corner and side handles carry local signs: -1 left/top, 1 right/bottom,
/// 0 along an axis they do not change.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Scale([i8; 2]),
    Translate,
    Rotate,
    /// A distort corner or mesh control point.
    Point(usize),
}

/// What a free transform's handles do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    ScaleRotate,
    /// Each corner moves freely, with true perspective in between.
    Distort,
    /// A corner drag moves its partner the opposite way along that edge.
    Perspective,
    /// A 4 x 4 grid of points bends the selection smoothly.
    Mesh,
}
impl Mode {
    pub(crate) const ALL: [Mode; 4] = [
        Mode::ScaleRotate,
        Mode::Distort,
        Mode::Perspective,
        Mode::Mesh,
    ];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::ScaleRotate => "Scale / rotate",
            Mode::Distort => "Distort",
            Mode::Perspective => "Perspective",
            Mode::Mesh => "Mesh",
        }
    }
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Mode::ScaleRotate => "Corners scale, sides stretch, inside moves, outside rotates",
            Mode::Distort => "Drag any corner freely; inside moves, outside rotates",
            Mode::Perspective => {
                "Drag a corner along an edge; its partner moves the opposite way for a symmetric perspective"
            }
            Mode::Mesh => "Drag the grid points to bend the selection; drag inside to move it",
        }
    }
}

fn warp_points(settings: crate::affine::Settings) -> Vec<[f64; 2]> {
    match settings.warp {
        Some(crate::affine::Warp::Corners(points)) => points.to_vec(),
        Some(crate::affine::Warp::Mesh(mesh)) => mesh.points().to_vec(),
        None => vec![],
    }
}
fn set_warp_points(settings: &mut crate::affine::Settings, points: &[[f64; 2]]) {
    match &mut settings.warp {
        Some(crate::affine::Warp::Corners(corners)) => corners.copy_from_slice(&points[..4]),
        Some(crate::affine::Warp::Mesh(mesh)) => mesh.points_mut().copy_from_slice(points),
        None => {}
    }
}
/// The warped selection's border in screen space, for hit testing and the
/// outline.
fn warp_outline(
    source: &crate::document::Document,
    settings: crate::affine::Settings,
    view: crate::view::View,
) -> Option<Vec<crate::egui::Pos2>> {
    let b = bounds(source);
    let mapping = settings.mapping(source).ok()?;
    let (l, t, r, bt) = (b.left as f64, b.top as f64, b.right as f64, b.bottom as f64);
    let steps = 24;
    let mut border = Vec::with_capacity(steps * 4);
    let edges = [
        ((l, t), (r, t)),
        ((r, t), (r, bt)),
        ((r, bt), (l, bt)),
        ((l, bt), (l, t)),
    ];
    for ((x0, y0), (x1, y1)) in edges {
        for k in 0..steps {
            let f = k as f64 / steps as f64;
            let [x, y] = mapping.forward(b, x0 + (x1 - x0) * f, y0 + (y1 - y0) * f);
            border.push(view.to_screen(crate::egui::pos2(x as f32, y as f32)));
        }
    }
    Some(border)
}
fn inside_polygon(polygon: &[crate::egui::Pos2], point: crate::egui::Pos2) -> bool {
    let mut inside = false;
    let mut j = polygon.len().wrapping_sub(1);
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}
/// Handle a press grabs in a warp mode: a point, else moving from inside,
/// else (distort and perspective) rotating from outside.
fn warp_pick(
    source: &crate::document::Document,
    settings: crate::affine::Settings,
    view: crate::view::View,
    pos: crate::egui::Pos2,
) -> Option<Handle> {
    let near = warp_points(settings)
        .iter()
        .enumerate()
        .map(|(index, p)| {
            let screen = view.to_screen(crate::egui::pos2(p[0] as f32, p[1] as f32));
            (index, (screen - pos).length())
        })
        .filter(|(_, distance)| *distance <= 12.0)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((index, _)) = near {
        return Some(Handle::Point(index));
    }
    let outline = warp_outline(source, settings, view)?;
    if inside_polygon(&outline, pos) {
        Some(Handle::Translate)
    } else if matches!(settings.warp, Some(crate::affine::Warp::Corners(_))) {
        Some(Handle::Rotate)
    } else {
        None
    }
}
/// Settings after dragging a warp handle from `drag.start` to `at`.
fn warp_dragged(
    drag: HandleDrag,
    at: [f64; 2],
    mode: Mode,
    modifiers: Modifiers,
) -> crate::affine::Settings {
    let mut settings = drag.settings;
    let mut points = warp_points(settings);
    let mut delta = [at[0] - drag.start[0], at[1] - drag.start[1]];
    match drag.handle {
        Handle::Translate => {
            if modifiers.shift {
                let axis = usize::from(delta[1].abs() > delta[0].abs());
                delta[1 - axis] = 0.0;
            }
            for p in &mut points {
                *p = [p[0] + delta[0], p[1] + delta[1]];
            }
        }
        Handle::Rotate => {
            let n = points.len() as f64;
            let center = points
                .iter()
                .fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
            let angle = |p: [f64; 2]| (p[1] - center[1]).atan2(p[0] - center[0]);
            let mut turn = angle(at) - angle(drag.start);
            if modifiers.shift {
                let step = 15f64.to_radians();
                turn = (turn / step).round() * step;
            }
            let (sin, cos) = turn.sin_cos();
            for p in &mut points {
                let (x, y) = (p[0] - center[0], p[1] - center[1]);
                *p = [center[0] + cos * x - sin * y, center[1] + sin * x + cos * y];
            }
        }
        Handle::Point(index) if index < points.len() => {
            if mode == Mode::Perspective && points.len() == 4 {
                // Move along one axis; the corner sharing that edge moves the
                // opposite way, keeping the trapezoid symmetric.
                let (axis, partner) = if delta[0].abs() >= delta[1].abs() {
                    (0, [1, 0, 3, 2][index])
                } else {
                    (1, [3, 2, 1, 0][index])
                };
                points[index][axis] += delta[axis];
                points[partner][axis] -= delta[axis];
            } else {
                points[index] = [points[index][0] + delta[0], points[index][1] + delta[1]];
            }
        }
        _ => {}
    }
    set_warp_points(&mut settings, &points);
    settings
}
const CORNERS: [[i8; 2]; 4] = [[-1, -1], [1, -1], [1, 1], [-1, 1]];
const SIDES: [[i8; 2]; 4] = [[0, -1], [1, 0], [0, 1], [-1, 0]];
#[derive(Clone, Copy)]
struct HandleDrag {
    handle: Handle,
    settings: crate::affine::Settings,
    start: [f64; 2],
}
struct Handles {
    corners: [crate::egui::Pos2; 4],
    sides: [crate::egui::Pos2; 4],
    center: crate::egui::Pos2,
    rotation: crate::egui::Pos2,
    top: crate::egui::Pos2,
}
fn bounds(source: &crate::document::Document) -> crate::selection::Selection {
    source.selection.unwrap_or(crate::selection::Selection {
        left: 0,
        top: 0,
        right: source.width,
        bottom: source.height,
    })
}
/// The transformed box in document space: center, rotated axes, half size
/// of the untransformed selection and signed scale (negative when flipped).
#[derive(Clone, Copy)]
struct Frame {
    pivot: [f64; 2],
    center: [f64; 2],
    u: [f64; 2],
    v: [f64; 2],
    half: [f64; 2],
    scale: [f64; 2],
}
impl Frame {
    fn new(settings: crate::affine::Settings, b: crate::selection::Selection) -> Self {
        let pivot = [
            (b.left as f64 + b.right as f64) / 2.0,
            (b.top as f64 + b.bottom as f64) / 2.0,
        ];
        let (sin, cos) = settings.angle_degrees.to_radians().sin_cos();
        Self {
            pivot,
            center: [
                pivot[0] + settings.translation[0],
                pivot[1] + settings.translation[1],
            ],
            u: [cos, sin],
            v: [-sin, cos],
            half: [
                (b.right - b.left).max(1) as f64 / 2.0,
                (b.bottom - b.top).max(1) as f64 / 2.0,
            ],
            scale: [0, 1].map(|axis| {
                let scale = settings.scale_percent[axis] / 100.0;
                if settings.flip[axis] { -scale } else { scale }
            }),
        }
    }
    /// Document position of local signs `g` on the transformed box.
    fn point(&self, g: [f64; 2]) -> [f64; 2] {
        let x = g[0] * self.half[0] * self.scale[0];
        let y = g[1] * self.half[1] * self.scale[1];
        [
            self.center[0] + self.u[0] * x + self.v[0] * y,
            self.center[1] + self.u[1] * x + self.v[1] * y,
        ]
    }
}
fn handles(
    source: &crate::document::Document,
    settings: crate::affine::Settings,
    view: crate::view::View,
) -> Option<Handles> {
    use crate::egui::pos2;
    settings.validate().ok()?;
    let frame = Frame::new(settings, bounds(source));
    let project = |g: [i8; 2]| {
        let [x, y] = frame.point([g[0] as f64, g[1] as f64]);
        view.to_screen(pos2(x as f32, y as f32))
    };
    let center = project([0, 0]);
    // The knob sits outside the edge that started as the top.
    let top = project([0, -1]);
    let outward = if (top - center).length() > 1.0 {
        (top - center).normalized()
    } else {
        crate::egui::vec2(0.0, -1.0)
    };
    Some(Handles {
        corners: CORNERS.map(project),
        sides: SIDES.map(project),
        center,
        top,
        rotation: top + outward * 28.0,
    })
}
/// Whether a screen point lies inside the transformed box, whichever way the
/// corners wind after a flip.
fn inside(corners: &[crate::egui::Pos2; 4], point: crate::egui::Pos2) -> bool {
    let sides = (0..4).map(|i| {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
    });
    let signs: Vec<f32> = sides.collect();
    signs.iter().all(|s| *s >= 0.0) || signs.iter().all(|s| *s <= 0.0)
}
/// The handle a press at `pos` grabs: a nearby handle, else moving from
/// inside the box, else rotating from outside it. On a box small enough for
/// handles to crowd its middle, a press nearer the center than any handle
/// still moves it.
fn pick(h: &Handles, pos: crate::egui::Pos2) -> Handle {
    let within = inside(&h.corners, pos);
    let center = (h.center - pos).length();
    CORNERS
        .iter()
        .zip(h.corners)
        .chain(SIDES.iter().zip(h.sides))
        .map(|(g, p)| (Handle::Scale(*g), (p - pos).length()))
        .chain([(Handle::Rotate, (h.rotation - pos).length())])
        .filter(|(_, distance)| *distance <= 12.0 && !(within && center < *distance))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
        .unwrap_or(if within {
            Handle::Translate
        } else {
            Handle::Rotate
        })
}
/// Snap targets for moving: guide lines, grid lines and the canvas edges and
/// center, within `tolerance` document pixels.
pub(crate) struct Snap {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub grid: Option<f64>,
    pub tolerance: f64,
}
impl Snap {
    /// Smallest correction that brings one of `values` onto a target.
    fn offset(&self, values: [f64; 3], targets: &[f64]) -> Option<f64> {
        let mut best: Option<f64> = None;
        for value in values {
            let mut consider = |target: f64| {
                let delta = target - value;
                if delta.abs() <= self.tolerance && best.is_none_or(|b| delta.abs() < b.abs()) {
                    best = Some(delta);
                }
            };
            targets.iter().copied().for_each(&mut consider);
            if let Some(step) = self.grid {
                consider((value / step).round() * step);
            }
        }
        best
    }
}
fn dragged_settings(
    drag: HandleDrag,
    at: [f64; 2],
    b: crate::selection::Selection,
    modifiers: Modifiers,
    snap: Option<&Snap>,
) -> crate::affine::Settings {
    let mut settings = drag.settings;
    let frame = Frame::new(drag.settings, b);
    let dot = |a: [f64; 2], b: [f64; 2]| a[0] * b[0] + a[1] * b[1];
    match drag.handle {
        Handle::Translate => {
            let mut delta = [at[0] - drag.start[0], at[1] - drag.start[1]];
            if modifiers.shift {
                // Constrain to the dominant axis.
                let axis = usize::from(delta[1].abs() > delta[0].abs());
                delta[1 - axis] = 0.0;
            }
            if let Some(snap) = snap {
                let corners = CORNERS.map(|g| frame.point([g[0] as f64, g[1] as f64]));
                for (axis, targets) in [(0, &snap.xs), (1, &snap.ys)] {
                    if modifiers.shift && delta[axis] == 0.0 {
                        continue;
                    }
                    let low = corners
                        .iter()
                        .map(|c| c[axis])
                        .fold(f64::INFINITY, f64::min);
                    let high = corners
                        .iter()
                        .map(|c| c[axis])
                        .fold(f64::NEG_INFINITY, f64::max);
                    let edges = [low, (low + high) / 2.0, high].map(|v| v + delta[axis]);
                    if let Some(correction) = snap.offset(edges, targets) {
                        delta[axis] += correction;
                    }
                }
            }
            for (translation, (start, delta)) in settings
                .translation
                .iter_mut()
                .zip(drag.settings.translation.iter().zip(delta))
            {
                *translation = (start + delta).clamp(-1000000.0, 1000000.0);
            }
        }
        Handle::Rotate => {
            let angle = |p: [f64; 2]| (p[1] - frame.center[1]).atan2(p[0] - frame.center[0]);
            let mut degrees =
                drag.settings.angle_degrees + (angle(at) - angle(drag.start)).to_degrees();
            if modifiers.shift {
                degrees = (degrees / 15.0).round() * 15.0;
            }
            settings.angle_degrees = (degrees + 180.0).rem_euclid(360.0) - 180.0;
        }
        // Warp points only exist in warp modes.
        Handle::Point(_) => {}
        Handle::Scale(g) => {
            let g = [g[0] as f64, g[1] as f64];
            // Keep the grab point under the pointer rather than jumping the
            // handle onto it.
            let handle = frame.point(g);
            let pointer = [
                at[0] + handle[0] - drag.start[0],
                at[1] + handle[1] - drag.start[1],
            ];
            let anchor = if modifiers.alt {
                frame.center
            } else {
                frame.point([-g[0], -g[1]])
            };
            let reach = if modifiers.alt { 1.0 } else { 2.0 };
            let d = [pointer[0] - anchor[0], pointer[1] - anchor[1]];
            let local = [dot(d, frame.u), dot(d, frame.v)];
            let mut scale = frame.scale;
            for axis in 0..2 {
                if g[axis] != 0.0 {
                    scale[axis] = local[axis] / (reach * g[axis] * frame.half[axis]);
                }
            }
            if modifiers.shift {
                // Uniform: the larger relative change drives both axes.
                let ratio = (0..2)
                    .filter(|axis| g[*axis] != 0.0)
                    .map(|axis| (scale[axis] / frame.scale[axis]).abs())
                    .fold(0.0, f64::max);
                for (value, start) in scale.iter_mut().zip(frame.scale) {
                    let sign = if *value < 0.0 { -1.0 } else { 1.0 };
                    *value = sign * start.abs() * ratio;
                }
            }
            for value in &mut scale {
                let sign = if *value < 0.0 { -1.0 } else { 1.0 };
                *value = sign * value.abs().clamp(0.001, 100.0);
            }
            let center = if modifiers.alt {
                frame.center
            } else {
                // The anchor stays put; the box grows away from it.
                let x = g[0] * frame.half[0] * scale[0];
                let y = g[1] * frame.half[1] * scale[1];
                let mid = [
                    anchor[0] + frame.u[0] * x + frame.v[0] * y,
                    anchor[1] + frame.u[1] * x + frame.v[1] * y,
                ];
                // Axes the handle does not move keep their center.
                let back = [frame.center[0] - anchor[0], frame.center[1] - anchor[1]];
                let kept = [
                    if g[0] == 0.0 { dot(back, frame.u) } else { 0.0 },
                    if g[1] == 0.0 { dot(back, frame.v) } else { 0.0 },
                ];
                [
                    mid[0] + frame.u[0] * kept[0] + frame.v[0] * kept[1],
                    mid[1] + frame.u[1] * kept[0] + frame.v[1] * kept[1],
                ]
            };
            settings.scale_percent = scale.map(|v| v.abs() * 100.0);
            settings.flip = scale.map(|v| v < 0.0);
            settings.translation =
                [0, 1].map(|axis| (center[axis] - frame.pivot[axis]).clamp(-1000000.0, 1000000.0));
        }
    }
    settings
}
struct Session {
    engine: Engine,
    checkpointed: bool,
    modified_before: bool,
}
impl Preview {
    pub(crate) fn needs_repaint(&self) -> bool {
        self.session
            .as_ref()
            .is_none_or(|s| s.engine.waiting() || !s.engine.finished())
    }
    pub(crate) fn register_allocations(&self, registry: &mut crate::snapshot_memory::Registry) {
        if let Some(session) = &self.session {
            session.engine.register_allocations(registry);
        }
    }
}
impl Dudel {
    pub(crate) fn mirror_pair_ui(&mut self, ui: &mut crate::egui::Ui) {
        let source = self
            .affine_preview
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .map_or(&self.doc, |s| s.engine.original());
        let selected = source.selection.is_some();
        let mut enabled = self.affine_settings.mirror.is_some();
        if ui.add_enabled(selected || enabled, crate::egui::Checkbox::new(&mut enabled, "Mirror pair"))
            .on_hover_text("Select one eye or feature. Transform its existing partner equally across the mirror axis.")
            .on_disabled_hover_text("Select one side first")
            .changed()
        {
            self.affine_settings.mirror = enabled.then(|| crate::affine::Mirror::from_symmetry(self.active_symmetry(), self.doc.width, self.doc.height));
            if let Some(preview) = &mut self.affine_preview {
                preview.overlay_active = false;
            }
        }
        if self.affine_settings.mirror.is_some() {
            ui.label("Select one side; its partner follows across the dashed axis.");
            ui.collapsing("Mirror axis", |ui| {
                let mirror = self.affine_settings.mirror.as_mut().unwrap();
                for (axis, label) in [(0, "Center X"), (1, "Center Y")] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(
                            crate::egui::DragValue::new(&mut mirror.center[axis])
                                .range(-1.0e6..=1.0e6)
                                .suffix(" px"),
                        );
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Tilt");
                    ui.add(
                        crate::egui::DragValue::new(&mut mirror.angle_degrees)
                            .range(-180.0..=180.0)
                            .suffix("°"),
                    );
                });
                if ui.button("Use symmetry guide").clicked() {
                    self.affine_settings.mirror = Some(crate::affine::Mirror::from_symmetry(
                        self.active_symmetry(),
                        self.doc.width,
                        self.doc.height,
                    ));
                }
            });
        }
    }
    fn paint_mirror_partner(
        &self,
        painter: &crate::egui::Painter,
        view: crate::view::View,
        source: &crate::document::Document,
    ) {
        use crate::egui::{Color32, Shape, Stroke, pos2};
        let Some(mirror) = self.affine_settings.mirror else {
            return;
        };
        let Ok(mapping) = self.affine_settings.mapping(source) else {
            return;
        };
        let screen = |[x, y]: [f64; 2]| view.to_screen(pos2(x as f32, y as f32));
        let b = crate::affine::Settings::bounds(source);
        let corners = [
            [b.left as f64, b.top as f64],
            [b.right as f64, b.top as f64],
            [b.right as f64, b.bottom as f64],
            [b.left as f64, b.bottom as f64],
        ];
        let mut border = Vec::new();
        for edge in 0..4 {
            for step in 0..=24 {
                let t = step as f64 / 24.0;
                let a = corners[edge];
                let z = corners[(edge + 1) % 4];
                let [x, y] = [a[0] + (z[0] - a[0]) * t, a[1] + (z[1] - a[1]) * t];
                border.push(screen(mirror.point(mapping.forward(b, x, y))));
            }
        }
        let stroke = Stroke::new(1.5_f32, Color32::from_rgb(255, 190, 85));
        painter.add(Shape::line(border, stroke));
        let (sin, cos) = mirror.angle_degrees.to_radians().sin_cos();
        let length = (source.width as f64).hypot(source.height as f64) * 2.0;
        let ends = [-1.0, 1.0].map(|sign| {
            screen([
                mirror.center[0] - sin * length * sign,
                mirror.center[1] + cos * length * sign,
            ])
        });
        painter.extend(Shape::dashed_line(&ends, stroke, 6.0, 5.0));
    }
    pub(crate) fn start_affine_preview(&mut self) {
        if self.canvas_edit_ticket.is_some() {
            return;
        }
        let paint_target = self.input_stroke_target();
        if let Some(blocker) = self.stroke_target_blocker(paint_target, false) {
            self.status = blocker.into();
            return;
        }
        if paint_target.is_raster() && self.input_alpha_locked() {
            self.status = "Turn off Lock transparency to transform pixels".into();
            return;
        }
        if self.adjustment.is_some() || self.adjustment_ticket.is_some() {
            return;
        }
        if self.affine_preview.is_some() {
            return;
        }
        if let Err(error) = self.affine_settings.validate() {
            self.status = error;
            return;
        }
        self.close_input_gestures();
        if let Some(target) = self.input_target() {
            self.affine_preview = Some(Preview {
                target,
                paint_target,
                session: None,
                drag: None,
                overlay: None,
                overlay_active: false,
            });
            self.status = "Preparing transform preview".into();
        }
    }
    pub(crate) fn poll_affine_preview(&mut self) {
        let Some(mut preview) = self.affine_preview.take() else {
            return;
        };
        if preview.session.is_none() {
            if self.render_queue.busy() {
                self.affine_preview = Some(preview);
                return;
            }
            let layer = &self.doc.layers[self.doc.active];
            if layer.id != preview.target {
                self.status = "Transform target is no longer active".into();
                return;
            }
            if let Some(blocker) = preview
                .paint_target
                .blocker(&self.doc, self.doc.active, false)
            {
                self.status = blocker.into();
                return;
            }
            if preview.paint_target.is_raster() && layer.alpha_locked {
                self.status = "Turn off Lock transparency to transform pixels".into();
                return;
            }
            self.begin_geometry_preview();
            preview.session = Some(Session {
                engine: Engine::with_target(&self.doc, preview.paint_target),
                checkpointed: false,
                modified_before: self.modified,
            });
            self.status = "Transform preview; Apply or Cancel".into();
        }
        let session = preview.session.as_mut().unwrap();
        // While dragging, the layer shows the selection lifted out and the
        // GPU overlay draws the moving selection; the full-quality result is
        // rendered once the drag ends.
        // Blending, clipping, masks, opacity and occlusion must go through the
        // real layer stack. Reference boards can also sit above this layer.
        let overlay = preview.drag.is_some()
            && self.affine_settings.mirror.is_none()
            && preview.paint_target.is_raster()
            && can_overlay(session.engine.original())
            && self.reference_layers.layers.iter().all(|b| !b.visible);
        if preview.drag.is_some() {
            preview.overlay_active = overlay;
        }
        let request = if overlay {
            lifted()
        } else {
            self.affine_settings
        };
        let result = session
            .engine
            .request_affine(request)
            .and_then(|()| session.engine.poll(&self.doc));

        match result {
            Ok(Some(candidate)) => {
                let changed = candidate.changed;
                if changed && !session.checkpointed {
                    self.history.begin_gesture_checkpoint(&self.doc);
                    session.checkpointed = true;
                }
                if let Err(error) = session.engine.publish(&mut self.doc, candidate) {
                    if session.checkpointed {
                        self.history
                            .retain_gesture_snapshot(session.engine.original(), &self.doc);
                    }
                    self.finish_geometry_preview(
                        session.engine.net_changed || session.engine.guard(&self.doc).is_err(),
                    );
                    self.status = error;
                    return;
                }
                if changed {
                    self.geometry_preview_changed();
                }
            }
            Ok(None) => {}
            Err(error) => {
                if session.checkpointed {
                    self.history
                        .retain_gesture_snapshot(session.engine.original(), &self.doc);
                }
                self.finish_geometry_preview(
                    session.engine.net_changed || session.engine.guard(&self.doc).is_err(),
                );
                self.status = error;
                return;
            }
        }
        // Hand off from the overlay once the final result is on the layer.
        if preview.drag.is_none()
            && preview
                .session
                .as_ref()
                .is_some_and(|session| session.engine.finished())
        {
            preview.overlay_active = false;
        }
        self.affine_preview = Some(preview);
    }
    pub(crate) fn finish_affine_preview(&mut self) {
        let Some(mut preview) = self.affine_preview.take() else {
            return;
        };
        if let Some(mut session) = preview.session.take() {
            if let Err(error) = session.engine.request_affine(self.affine_settings) {
                preview.session = Some(session);
                self.affine_preview = Some(preview);
                self.status = error;
                return;
            }
            match self.render_queue.finish_move(
                session.engine,
                session.checkpointed,
                session.modified_before,
            ) {
                Ok(()) => self.status = "Finishing transform in background".into(),
                Err((engine, error)) => {
                    preview.session = Some(Session {
                        engine: *engine,
                        checkpointed: session.checkpointed,
                        modified_before: session.modified_before,
                    });
                    self.affine_preview = Some(preview);
                    self.status = error;
                }
            }
        } else {
            // Pending acquisition keeps settings/identity but resolves its pivot
            // after preceding commands, without forcing a source fence here.
            let edit = match preview.paint_target {
                crate::paint_target::Target::Raster => Edit::AffineSettingsTarget {
                    target: preview.target,
                    settings: Box::new(self.affine_settings),
                },
                crate::paint_target::Target::Mask(mask) => Edit::MaskTransform {
                    target: preview.target,
                    mask,
                    operation: crate::mask_transform::Operation::Affine(Box::new(
                        self.affine_settings,
                    )),
                },
            };
            self.dispatch_document_edit(edit, "Transform applied", "Transform unchanged");
        }
    }
    pub(crate) fn cancel_affine_preview(&mut self) {
        let Some(preview) = self.affine_preview.take() else {
            return;
        };
        if let Some(session) = preview.session {
            if let Err(error) = session.engine.guard(&self.doc) {
                if session.checkpointed {
                    self.history
                        .retain_gesture_snapshot(session.engine.original(), &self.doc);
                }
                self.finish_geometry_preview(true);
                self.status = error;
                return;
            }
            if session.checkpointed {
                let source = session.engine.original();
                self.doc.layers[self.doc.active].pixels =
                    source.layers[source.active].pixels.clone();
                self.doc.selection = source.selection;
                self.doc.layers[self.doc.active].mask = source.layers[source.active].mask.clone();
                self.doc.selection_mask = source.selection_mask.clone();
                self.history.discard_pending_checkpoint(&self.doc);
                self.geometry_preview_changed();
            }
            if !self.finish_geometry_preview(false) {
                self.modified = session.modified_before;
            }
        }
        self.status = "Transform canceled".into();
    }
    pub(crate) fn affine_handle_packet(
        &mut self,
        view: crate::view::View,
        pos: crate::egui::Pos2,
        start: bool,
        end: bool,
        can_start: bool,
        modifiers: Modifiers,
    ) {
        let snap = self.transform_snap(view);
        let Some(preview) = self.affine_preview.as_mut() else {
            return;
        };
        let Some(session) = &preview.session else {
            return;
        };
        let source = session.engine.original();
        if source.layers.iter().any(|l| l.mask.is_some()) {
            return;
        }
        let b = bounds(source);
        let at = view.to_doc(pos);
        let warping = self.affine_settings.warp.is_some();
        if start && can_start {
            let handle = if warping {
                warp_pick(source, self.affine_settings, view, pos)
            } else {
                handles(source, self.affine_settings, view).map(|h| pick(&h, pos))
            };
            preview.drag = handle.map(|handle| HandleDrag {
                handle,
                settings: self.affine_settings,
                start: [at.x as f64, at.y as f64],
            });
        }
        if let Some(drag) = preview.drag {
            let next = if warping {
                warp_dragged(
                    drag,
                    [at.x as f64, at.y as f64],
                    self.transform_mode,
                    modifiers,
                )
            } else {
                dragged_settings(drag, [at.x as f64, at.y as f64], b, modifiers, Some(&snap))
            };
            // A drag that would fold the corners keeps the last valid shape.
            if next.mapping(source).is_ok() {
                self.affine_settings = next;
            }
        }
        if end {
            preview.drag = None;
        }
    }
    /// Snap targets for the current guides, grid and canvas, with a tolerance
    /// of a few screen points at the current zoom.
    fn transform_snap(&self, view: crate::view::View) -> Snap {
        let (w, h) = (self.doc.width as f64, self.doc.height as f64);
        let mut xs = vec![0.0, w / 2.0, w];
        let mut ys = vec![0.0, h / 2.0, h];
        for line in &self.guide_lines {
            if line.vertical {
                xs.push(line.position as f64);
            } else {
                ys.push(line.position as f64);
            }
        }
        Snap {
            xs,
            ys,
            grid: self.grid.visible.then_some(self.grid.spacing as f64),
            tolerance: 8.0 / view.zoom.max(1e-3) as f64,
        }
    }
    /// Cursor showing what a drag from `pos` would do to the transform box.
    pub(crate) fn transform_cursor(
        &self,
        view: crate::view::View,
        pos: crate::egui::Pos2,
    ) -> Option<crate::egui::CursorIcon> {
        use crate::egui::CursorIcon;
        let preview = self.affine_preview.as_ref()?;
        let session = preview.session.as_ref()?;
        if self.affine_settings.warp.is_some() {
            let handle = preview.drag.map(|drag| drag.handle).or_else(|| {
                warp_pick(session.engine.original(), self.affine_settings, view, pos)
            })?;
            return Some(match handle {
                Handle::Point(_) => CursorIcon::Crosshair,
                Handle::Rotate => CursorIcon::Alias,
                _ => CursorIcon::Move,
            });
        }
        let h = handles(session.engine.original(), self.affine_settings, view)?;
        let handle = preview
            .drag
            .map_or_else(|| pick(&h, pos), |drag| drag.handle);
        Some(match handle {
            Handle::Translate | Handle::Point(_) => CursorIcon::Move,
            Handle::Rotate => CursorIcon::Alias,
            Handle::Scale(g) => {
                // Resize along the handle's on-screen direction from the center.
                let target = if g[0] != 0 && g[1] != 0 {
                    h.corners[CORNERS.iter().position(|c| *c == g)?]
                } else {
                    h.sides[SIDES.iter().position(|s| *s == g)?]
                };
                let d = target - h.center;
                let angle = d.y.atan2(d.x).to_degrees().rem_euclid(180.0);
                match ((angle + 22.5) / 45.0) as u32 % 4 {
                    0 => CursorIcon::ResizeHorizontal,
                    1 => CursorIcon::ResizeNwSe,
                    2 => CursorIcon::ResizeVertical,
                    _ => CursorIcon::ResizeNeSw,
                }
            }
        })
    }
    /// Arrow keys move the free-transform box by a pixel, or ten with Shift.
    pub(crate) fn nudge_transform(&mut self, dx: f64, dy: f64) {
        if self.affine_preview.is_none() {
            return;
        }
        if self.affine_settings.warp.is_some() {
            let points: Vec<[f64; 2]> = warp_points(self.affine_settings)
                .into_iter()
                .map(|p| [p[0] + dx, p[1] + dy])
                .collect();
            set_warp_points(&mut self.affine_settings, &points);
            return;
        }
        let t = &mut self.affine_settings.translation;
        t[0] = (t[0] + dx).clamp(-1000000.0, 1000000.0);
        t[1] = (t[1] + dy).clamp(-1000000.0, 1000000.0);
    }
    /// Switch transform mode, carrying the current shape over. Leaving a warp
    /// for scale/rotate is not possible without Reset, so it is refused.
    pub(crate) fn set_transform_mode(&mut self, mode: Mode) {
        use crate::affine::Warp;
        let source = self
            .affine_preview
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .map_or(&self.doc, |s| s.engine.original());
        let settings = self.affine_settings;
        let warp = match (mode, settings.warp) {
            (Mode::ScaleRotate, None) => None,
            (Mode::ScaleRotate, Some(_)) => {
                self.status = "Reset the transform to return to scale and rotate".into();
                return;
            }
            (Mode::Distort | Mode::Perspective, Some(Warp::Corners(corners))) => {
                Some(Warp::Corners(corners))
            }
            (Mode::Distort | Mode::Perspective, Some(Warp::Mesh(mesh))) => {
                Some(Warp::Corners(mesh.corners()))
            }
            (Mode::Distort | Mode::Perspective, None) => match settings.corners(source) {
                Ok(corners) => Some(Warp::Corners(corners)),
                Err(error) => {
                    self.status = error;
                    return;
                }
            },
            (Mode::Mesh, Some(Warp::Mesh(mesh))) => Some(Warp::Mesh(mesh)),
            (Mode::Mesh, _) => match settings.mesh(source, self.mesh_grid[0], self.mesh_grid[1]) {
                Ok(mesh) => Some(Warp::Mesh(mesh)),
                Err(error) => {
                    self.status = error;
                    return;
                }
            },
        };
        let next = crate::affine::Settings { warp, ..settings };
        if next.mapping(source).is_err() {
            self.status = "That shape cannot be converted; reset the transform first".into();
            return;
        }
        self.affine_settings = next;
        self.transform_mode = mode;
    }
    /// Change the mesh grid size, resampling the current warp so its shape
    /// carries over.
    pub(crate) fn set_mesh_grid(&mut self, columns: u8, rows: u8) {
        self.mesh_grid = [columns, rows];
        if !matches!(
            self.affine_settings.warp,
            Some(crate::affine::Warp::Mesh(_))
        ) {
            return;
        }
        let source = self
            .affine_preview
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .map_or(&self.doc, |s| s.engine.original());
        match self.affine_settings.mesh(source, columns, rows) {
            Ok(mesh) => self.affine_settings.warp = Some(crate::affine::Warp::Mesh(mesh)),
            Err(error) => self.status = error,
        }
    }
    /// Ctrl+T: start free transform from any tool, or apply the open one.
    pub(crate) fn toggle_free_transform(&mut self) {
        if self.affine_preview.is_some() {
            self.finish_affine_preview();
        } else {
            self.affine_settings = crate::affine::Settings {
                sampling: self.affine_settings.sampling,
                mirror: self.affine_settings.mirror,
                ..crate::affine::Settings::default()
            };
            self.transform_mode = Mode::ScaleRotate;
            self.start_affine_preview();
        }
    }
    fn paint_warp(
        &self,
        painter: &crate::egui::Painter,
        view: crate::view::View,
        source: &crate::document::Document,
    ) {
        use crate::egui::{Color32, Shape, Stroke, pos2};
        let settings = self.affine_settings;
        let Ok(mapping) = settings.mapping(source) else {
            return;
        };
        let accent = Color32::from_rgb(75, 180, 255);
        let stroke = Stroke::new(1.5_f32, accent);
        let b = bounds(source);
        let (l, t) = (b.left as f64, b.top as f64);
        let (w, h) = ((b.right - b.left) as f64, (b.bottom - b.top) as f64);
        let screen = |x: f64, y: f64| {
            let [x, y] = mapping.forward(b, x, y);
            view.to_screen(pos2(x as f32, y as f32))
        };
        // Mesh: the warped grid lines; corners: the border.
        let (columns, rows) = match settings.warp {
            Some(crate::affine::Warp::Mesh(mesh)) => (mesh.columns(), mesh.rows()),
            _ => (2, 2),
        };
        let mut lines = Vec::with_capacity(columns + rows);
        for k in 0..rows {
            let f = k as f64 / (rows - 1) as f64;
            lines.push(
                (0..=32)
                    .map(|i| screen(l + w * i as f64 / 32.0, t + h * f))
                    .collect::<Vec<_>>(),
            );
        }
        for k in 0..columns {
            let f = k as f64 / (columns - 1) as f64;
            lines.push(
                (0..=32)
                    .map(|i| screen(l + w * f, t + h * i as f64 / 32.0))
                    .collect::<Vec<_>>(),
            );
        }
        for line in lines {
            painter.add(Shape::line(
                line.clone(),
                Stroke::new(3.0_f32, Color32::from_black_alpha(90)),
            ));
            painter.add(Shape::line(line, stroke));
        }
        for p in warp_points(settings) {
            painter.circle(
                view.to_screen(pos2(p[0] as f32, p[1] as f32)),
                5.0,
                Color32::WHITE,
                stroke,
            );
        }
    }
    /// Upload the selection as the drag-preview texture once per session.
    pub(crate) fn prepare_transform_overlay(&mut self, ctx: &crate::egui::Context) {
        if self.affine_settings.mirror.is_some() {
            return;
        }
        let Some(preview) = self.affine_preview.as_mut() else {
            return;
        };
        if preview.overlay.is_some() || preview.drag.is_none() {
            return;
        }
        let Some(session) = &preview.session else {
            return;
        };
        let source = session.engine.original();
        if !can_overlay(source) || self.reference_layers.layers.iter().any(|b| b.visible) {
            return;
        }
        let b = bounds(source);
        let (w, h) = (b.right - b.left, b.bottom - b.top);
        if w == 0 || h == 0 {
            return;
        }
        let scale = (OVERLAY_SIDE as f64 / w.max(h) as f64).min(1.0);
        let (tw, th) = (
            ((w as f64 * scale).round() as usize).max(1),
            ((h as f64 * scale).round() as usize).max(1),
        );
        let layer = &source.layers[source.active];
        let mut rgba = vec![0u8; tw * th * 4];
        for ty in 0..th {
            let y = b.top + ((ty as f64 + 0.5) / scale) as usize;
            for tx in 0..tw {
                let x = b.left + ((tx as f64 + 0.5) / scale) as usize;
                let (x, y) = (x.min(b.right - 1), y.min(b.bottom - 1));
                let coverage = source
                    .selection_mask
                    .as_ref()
                    .map_or(255, |mask| mask.at(x, y));
                let mut pixel = layer.pixels.rgba(y * source.width + x);
                pixel[3] = (pixel[3] as u16 * coverage as u16 / 255) as u8;
                rgba[(ty * tw + tx) * 4..(ty * tw + tx) * 4 + 4].copy_from_slice(&pixel);
            }
        }
        let image = crate::egui::ColorImage::from_rgba_unmultiplied([tw, th], &rgba);
        preview.overlay = Some(ctx.load_texture(
            "transform-overlay",
            image,
            crate::egui::TextureOptions::LINEAR,
        ));
    }
    /// Whether the GPU overlay is standing in for the full-quality preview.
    #[cfg(test)]
    pub(crate) fn transform_overlay_active(&self) -> bool {
        self.affine_preview
            .as_ref()
            .is_some_and(|p| p.overlay_active && p.overlay.is_some())
    }
    fn paint_transform_overlay(&self, painter: &crate::egui::Painter, view: crate::view::View) {
        use crate::egui::{Color32, Mesh, Shape, epaint::Vertex, pos2};
        let Some(preview) = self.affine_preview.as_ref() else {
            return;
        };
        let (Some(texture), Some(session)) = (&preview.overlay, &preview.session) else {
            return;
        };
        if !preview.overlay_active {
            return;
        }
        let source = session.engine.original();
        let Ok(mapping) = self.affine_settings.mapping(source) else {
            return;
        };
        let layer = &source.layers[source.active];
        if !layer.visible {
            return;
        }
        let alpha = (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        let tint = Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha);
        let b = bounds(source);
        let (l, t) = (b.left as f64, b.top as f64);
        let (w, h) = ((b.right - b.left) as f64, (b.bottom - b.top) as f64);
        // Scale/rotate is exact with two triangles; warps need a grid so
        // perspective and mesh bends stay smooth.
        let n = if self.affine_settings.warp.is_some() {
            24
        } else {
            1
        };
        let mut mesh = Mesh::with_texture(texture.id());
        for j in 0..=n {
            for i in 0..=n {
                let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                let [x, y] = mapping.forward(b, l + u * w, t + v * h);
                mesh.vertices.push(Vertex {
                    pos: view.to_screen(pos2(x as f32, y as f32)),
                    uv: pos2(u as f32, v as f32),
                    color: tint,
                });
            }
        }
        let row = (n + 1) as u32;
        for j in 0..n as u32 {
            for i in 0..n as u32 {
                let a = j * row + i;
                mesh.indices
                    .extend_from_slice(&[a, a + 1, a + row + 1, a, a + row + 1, a + row]);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
    pub(crate) fn paint_affine_handles(
        &self,
        painter: &crate::egui::Painter,
        view: crate::view::View,
    ) {
        use crate::egui::{Color32, Rect, Shape, Stroke, vec2};
        let Some(session) = self
            .affine_preview
            .as_ref()
            .and_then(|p| p.session.as_ref())
        else {
            return;
        };
        self.paint_transform_overlay(painter, view);
        self.paint_mirror_partner(painter, view, session.engine.original());
        if self.affine_settings.warp.is_some() {
            self.paint_warp(painter, view, session.engine.original());
            return;
        }
        let Some(h) = handles(session.engine.original(), self.affine_settings, view) else {
            return;
        };
        let accent = Color32::from_rgb(75, 180, 255);
        let stroke = Stroke::new(1.5_f32, accent);
        let mut outline = h.corners.to_vec();
        outline.push(h.corners[0]);
        painter.add(Shape::line(
            outline.clone(),
            Stroke::new(3.0_f32, Color32::from_black_alpha(90)),
        ));
        painter.add(Shape::line(outline, stroke));
        painter.line_segment([h.top, h.rotation], stroke);
        for side in h.sides {
            painter.rect(
                Rect::from_center_size(side, vec2(8.0, 8.0)),
                1.0,
                Color32::WHITE,
                stroke,
                crate::egui::StrokeKind::Middle,
            );
        }
        for corner in h.corners {
            painter.circle(corner, 5.0, Color32::WHITE, stroke);
        }
        painter.circle(h.rotation, 6.0, Color32::WHITE, stroke);
        for d in [vec2(6.0, 0.0), vec2(0.0, 6.0)] {
            painter.line_segment([h.center - d, h.center + d], stroke);
        }
    }
}
