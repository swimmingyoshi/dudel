//! Headless engine workload; deliberately excludes driver, UI and GPU timings.
use crate::{
    brush::Brush,
    document::Document,
    stroke::{Sample, Stroke},
};
use serde::Serialize;
use std::{hint::black_box, time::Instant};

#[derive(Serialize)]
struct Measurement {
    brush: String,
    frames: usize,
    dabs: usize,
    paint_ms: f64,
    composite_ms: f64,
    max_engine_frame_ms: f64,
    buffer_payload_capacity_bytes: usize,
    tail_peak_dabs: usize,
    tail_finish_ms: f64,
}

#[derive(Serialize)]
struct Report {
    profile: &'static str,
    incremental: bool,
    buffered: bool,
    buffer_memory_scope: &'static str,
    canvas: [usize; 2],
    scope: &'static str,
    measurements: Vec<Measurement>,
    brush_settings: Vec<Brush>,
    tail_workloads: Vec<TailWorkload>,
    filters: Vec<FilterMeasurement>,
    cold_history: Vec<ColdHistoryMeasurement>,
    cold_history_scope: &'static str,
    lifecycle: Vec<Lifecycle>,
    clear: Vec<Clear>,
    transforms: Vec<TransformMeasurement>,
    project_storage: Vec<ProjectStorage>,
    display: Vec<Display>,
}

#[derive(Serialize)]
struct ColdHistoryMeasurement {
    canvas: [usize; 2],
    layers: usize,
    all_layers_dense: bool,
    actions: usize,
    cooling_enabled: bool,
    prefetch_between_actions: bool,
    prefetch_wall_ms: f64,
    peak_prefetched_bytes: usize,
    history_bytes_before: usize,
    history_bytes_after: usize,
    compressed_regions: usize,
    cooling_elapsed_ms: f64,
    max_idle_call_ms: f64,
    max_undo_ms: f64,
    max_redo_ms: f64,
    live_pixel_capacity_bytes: usize,
}
fn cold_history_workload(
    layers: usize,
    cooling_enabled: bool,
    prefetch_between_actions: bool,
) -> Result<ColdHistoryMeasurement, String> {
    let side = 2048;
    let mut doc = Document::new(side, side);
    for _ in 1..layers {
        doc.add_layer();
    }
    // Supported four-megapixel canvas with every layer dense; no compatibility
    // cache. A reusable packed row is benchmark scratch, excluded from payload.
    let mut row = vec![0; side * 4];
    for (layer_index, layer) in doc.layers.iter_mut().enumerate() {
        for pixel in row.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[10 + layer_index as u8, 30, 200, 128]);
        }
        let mut writer = layer.pixels.writer();
        for y in 0..side {
            writer.write_region(side, [0, y, side, y + 1], &row);
        }
    }
    doc.validate()?;
    let mut history = crate::document::History::new();
    doc.active = 0;
    for step in 1..=6 {
        history.checkpoint(&doc);
        let mut writer = doc.layers[0].pixels.writer();
        for y in 0..side {
            for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                pixel.copy_from_slice(&[
                    10 + step * 17,
                    30,
                    200,
                    if (x + y) % 31 == 0 { 0 } else { 128 },
                ]);
            }
            writer.write_region(side, [0, y, side, y + 1], &row);
        }
        history.settle(&doc);
    }
    let history_bytes_before = history.retained_bytes();
    let started = Instant::now();
    let mut max_idle_call_ms = 0.0f64;
    if cooling_enabled {
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let call = Instant::now();
            let pending = history.cool_idle(&doc);
            max_idle_call_ms = max_idle_call_ms.max(call.elapsed().as_secs_f64() * 1000.0);
            if !pending {
                break;
            }
            if Instant::now() >= deadline {
                return Err("Cold history benchmark worker timed out".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let cooling_elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let history_bytes_after = history.retained_bytes();
    let compressed_regions = history.compressed_regions();
    let live_pixel_capacity_bytes = doc.layers.iter().map(|layer| layer.pixels.capacity()).sum();
    let mut max_undo_ms = 0.0f64;
    let mut max_redo_ms = 0.0f64;
    let mut prefetch_wall_ms = 0.0;
    let mut peak_prefetched_bytes = 0;
    for step in (0..6).rev() {
        let start = Instant::now();
        if !history.undo(&mut doc) {
            return Err(history
                .take_error()
                .unwrap_or("History benchmark lost an undo action".into()));
        }
        max_undo_ms = max_undo_ms.max(start.elapsed().as_secs_f64() * 1000.0);
        verify_history_pixels(&doc, step)?;
        if prefetch_between_actions {
            prefetch_wall_ms += prefetch_history(
                &mut history,
                &doc,
                &mut peak_prefetched_bytes,
                &mut max_idle_call_ms,
            )?;
        }
    }
    if history.can_undo() {
        return Err("Cold history retained an unexpected extra action".into());
    }
    for step in 1..=6 {
        let start = Instant::now();
        if !history.redo(&mut doc) {
            return Err(history
                .take_error()
                .unwrap_or("History benchmark lost a redo action".into()));
        }
        max_redo_ms = max_redo_ms.max(start.elapsed().as_secs_f64() * 1000.0);
        verify_history_pixels(&doc, step)?;
        if prefetch_between_actions {
            prefetch_wall_ms += prefetch_history(
                &mut history,
                &doc,
                &mut peak_prefetched_bytes,
                &mut max_idle_call_ms,
            )?;
        }
    }
    Ok(ColdHistoryMeasurement {
        canvas: [side, side],
        layers,
        all_layers_dense: true,
        actions: 6,
        cooling_enabled,
        prefetch_between_actions,
        prefetch_wall_ms,
        peak_prefetched_bytes,
        history_bytes_before,
        history_bytes_after,
        compressed_regions,
        cooling_elapsed_ms,
        max_idle_call_ms,
        max_undo_ms,
        max_redo_ms,
        live_pixel_capacity_bytes,
    })
}

fn prefetch_history(
    history: &mut crate::document::History,
    doc: &Document,
    peak: &mut usize,
    max_idle: &mut f64,
) -> Result<f64, String> {
    let start = Instant::now();
    let deadline = start + std::time::Duration::from_secs(30);
    loop {
        let call = Instant::now();
        let pending = history.cool_idle(doc);
        *max_idle = max_idle.max(call.elapsed().as_secs_f64() * 1000.0);
        *peak = (*peak).max(history.prefetched_bytes());
        if !pending {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Undo prefetch benchmark worker timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

fn verify_history_pixels(doc: &Document, step: u8) -> Result<(), String> {
    let mut count = 0;
    doc.layers[0].pixels.try_visit_rows(
        doc.width,
        doc.height,
        [0, 0, doc.width, doc.height],
        |left, y, row| -> Result<(), String> {
            for (offset, pixel) in row.chunks_exact(4).enumerate() {
                let expected = [
                    10 + step * 17,
                    30,
                    200,
                    if step > 0 && (left + offset + y) % 31 == 0 {
                        0
                    } else {
                        128
                    },
                ];
                if pixel != expected {
                    return Err("Cold history pixel verification failed".into());
                }
                count += 1;
            }
            Ok(())
        },
    )?;
    if count != doc.width * doc.height {
        return Err("Cold history lost a dense tile".into());
    }
    Ok(())
}

#[derive(Serialize)]
struct FilterMeasurement {
    engine: &'static str,
    explicit_flat_baseline: bool,
    canvas: [usize; 2],
    visible_pixels: usize,
    filter_ms: f64,
    source_pixel_capacity_before: usize,
    snapshot_pixel_capacity_after: usize,
    output_pixel_capacity_after: usize,
    logical_pixel_bytes: usize,
}

fn filter_workload(wasm: bool, dense: bool, flat: bool) -> Result<FilterMeasurement, String> {
    let side = if dense { 512 } else { 2048 };
    let mut doc = Document::new(side, side);
    let visible_pixels = if dense { side * side } else { 2 };
    {
        let mut writer = doc.layers[0].pixels.writer();
        if dense {
            for index in 0..side * side {
                writer.rgba_mut(index).copy_from_slice(&[10, 30, 200, 128]);
            }
        } else {
            writer.rgba_mut(0).copy_from_slice(&[10, 30, 200, 128]);
            writer
                .rgba_mut(side * side - 1)
                .copy_from_slice(&[40, 50, 60, 255]);
        }
    }
    let snapshot = doc.layers[0].pixels.clone();
    let source_pixel_capacity_before = snapshot.capacity();
    let start = Instant::now();
    if flat {
        // Explicit compatibility baseline: includes materializing/copying the
        // shared flat view, then the same filter runner. Not an old-build timing.
        doc.layers[0].pixels = doc.layers[0].pixels.to_vec().into();
    }
    let control = crate::plugins::FilterControl::default();
    if wasm {
        crate::wasm_filters::apply(
            &mut doc.layers[0],
            include_bytes!("../plugins/example-invert.wasm"),
            None,
            side,
            &control,
        )?;
    } else {
        crate::plugins::apply_filter_controlled(
            &mut doc.layers[0],
            "fn filter(r,g,b,a) {[255-r,255-g,255-b,a]}",
            None,
            side,
            &control,
        )?;
    }
    let filter_ms = start.elapsed().as_secs_f64() * 1000.0;
    if doc.layers[0].pixels.rgba(0) != [245, 225, 55, 128] || snapshot.rgba(0) != [10, 30, 200, 128]
    {
        return Err("Filter benchmark failed to preserve snapshot/output".into());
    }
    Ok(FilterMeasurement {
        engine: if wasm { "WASM" } else { "Rhai" },
        explicit_flat_baseline: flat,
        canvas: [side, side],
        visible_pixels,
        filter_ms,
        source_pixel_capacity_before,
        snapshot_pixel_capacity_after: snapshot.capacity(),
        output_pixel_capacity_after: doc.layers[0].pixels.capacity(),
        logical_pixel_bytes: side * side * 4,
    })
}

#[derive(Serialize)]
struct TailWorkload {
    cooperative: bool,
    live_step_calls: usize,
    max_live_step_ms: f64,
    live_cursor_peak_payload_bytes: usize,
    step_budget_work_units: usize,
    step_calls: usize,
    max_step_ms: f64,
    cursor_peak_payload_bytes: usize,
    rgba_fnv1a64: String,
    brush_settings: Brush,
    symmetry_segments: u8,
    emitted_dabs: usize,
    tail_peak_dabs: usize,
    live_paint_ms: f64,
    finish_ms: f64,
    buffer_payload_capacity_bytes: usize,
}

/// Worst retention interval, at the legal .5px minimum spacing. This measures
/// lift rasterization separately; it excludes history settlement and GPU upload.
fn tail_workload(
    soft: bool,
    segments: u8,
    cooperative: bool,
) -> (TailWorkload, crate::pixels::Pixels) {
    let brush = Brush {
        name: if soft {
            "Maximum tapered soft"
        } else {
            "Maximum tapered ink"
        }
        .into(),
        size: if soft { 300.0 } else { 18.0 },
        hardness: if soft { 0.0 } else { 1.0 },
        pressure_size: false,
        flow: 0.3,
        opacity: 0.5,
        taper_out: 300.0,
        dynamics: crate::dynamics::BrushDynamics {
            spacing: Some(crate::dynamics::Dynamics {
                inputs: vec![crate::dynamics::Effector {
                    input: crate::dynamics::Input::Pressure,
                    curve: crate::dynamics::Curve {
                        points: vec![[0.0, 0.0], [1.0, 0.0]],
                    },
                }],
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let settings = crate::stroke_buffer::Settings {
        target: crate::paint_target::Target::Raster,
        brush: brush.clone(),
        color: [38, 44, 59],
        erase: false,
        layer: 0,
        symmetry: crate::symmetry::Symmetry {
            mode: if segments > 1 {
                crate::symmetry::Mode::Radial
            } else {
                crate::symmetry::Mode::Off
            },
            segments,
            ..Default::default()
        },
    };
    let mut doc = Document::new(512, 512);
    let mut buffer = crate::stroke_buffer::StrokeBuffer::default();
    let mut stroke = Stroke::default();
    let mut emitted_dabs = 0;
    let shared_settings = std::sync::Arc::new(settings.clone());
    let mut live_step_calls = 0;
    let mut max_live_step_ms = 0.0f64;
    let mut live_cursor_peak_payload_bytes = 0;
    let live = Instant::now();
    for i in 0..602 {
        let sample = Sample {
            x: 105.5 + i as f32 * 0.5,
            y: 256.5,
            pressure: 1.0,
            ..Default::default()
        };
        let mut dabs = stroke.sample(sample, &brush);
        if i == 601 {
            dabs.extend(stroke.finish(sample, &brush));
        }
        emitted_dabs += dabs.len();
        for dab in dabs {
            if cooperative {
                let mut cursor = buffer
                    .begin_dab(&doc, dab, shared_settings.clone())
                    .unwrap();
                loop {
                    let started = Instant::now();
                    let step = cursor.step(&mut buffer, &mut doc, 64).unwrap();
                    max_live_step_ms =
                        max_live_step_ms.max(started.elapsed().as_secs_f64() * 1000.0);
                    live_step_calls += 1;
                    live_cursor_peak_payload_bytes =
                        live_cursor_peak_payload_bytes.max(cursor.peak_payload_capacity_bytes());
                    if step.complete {
                        break;
                    }
                }
            } else {
                buffer.paint(&mut doc, dab, &settings);
            }
        }
    }
    let live_paint_ms = live.elapsed().as_secs_f64() * 1000.0;
    let finish = Instant::now();
    let mut step_calls = 0;
    let mut max_step_ms = 0.0f64;
    let mut cursor_peak_payload_bytes = 0;
    let step_budget_work_units = if cooperative { 64 } else { 0 };
    if cooperative {
        let mut cursor = buffer.begin_tail(&doc, &settings).unwrap();
        cursor_peak_payload_bytes = cursor.peak_payload_capacity_bytes();
        loop {
            let started = Instant::now();
            let step = cursor
                .step(&mut buffer, &mut doc, step_budget_work_units)
                .unwrap();
            max_step_ms = max_step_ms.max(started.elapsed().as_secs_f64() * 1000.0);
            step_calls += 1;
            cursor_peak_payload_bytes =
                cursor_peak_payload_bytes.max(cursor.peak_payload_capacity_bytes());
            if step.complete {
                break;
            }
        }
    } else {
        buffer.finish_tail(&mut doc, &settings);
    }
    let finish_ms = finish.elapsed().as_secs_f64() * 1000.0;
    let mut checksum = 0xcbf29ce484222325u64;
    for index in 0..doc.width * doc.height {
        for byte in doc.layers[0].pixels.rgba(index) {
            checksum = (checksum ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
    (
        TailWorkload {
            cooperative,
            live_step_calls,
            max_live_step_ms,
            live_cursor_peak_payload_bytes,
            step_budget_work_units,
            step_calls,
            max_step_ms,
            cursor_peak_payload_bytes,
            rgba_fnv1a64: format!("{checksum:016x}"),
            brush_settings: brush,
            symmetry_segments: segments,
            emitted_dabs,
            tail_peak_dabs: buffer.tail_peak_dabs(),
            live_paint_ms,
            finish_ms,
            buffer_payload_capacity_bytes: buffer.payload_capacity_bytes(),
        },
        doc.layers[0].pixels.clone(),
    )
}

fn tail_workloads() -> Result<Vec<TailWorkload>, String> {
    let cases = [(false, 1), (true, 1), (true, 4)];
    let references: Vec<_> = cases
        .iter()
        .map(|&(soft, segments)| tail_workload(soft, segments, false))
        .collect();
    let mut raw = vec![];
    let mut cooperative = vec![];
    for ((soft, segments), (measurement, expected)) in cases.into_iter().zip(references) {
        let (sliced, actual) = tail_workload(soft, segments, true);
        // Sparse equality is exact, including hidden RGBA, and outside timers.
        if actual != expected {
            return Err("Cooperative tail fixture differs from synchronous pixels".into());
        }
        raw.push(measurement);
        cooperative.push(sliced);
    }
    raw.extend(cooperative);
    Ok(raw)
}

#[derive(Serialize)]
struct Lifecycle {
    canvas: [usize; 2],
    layers: usize,
    stroke_shape: &'static str,
    checkpoint_ms: f64,
    pen_down_ms: f64,
    pen_up_history_ms: f64,
    undo_ms: f64,
    redo_ms: f64,
    history_bytes: usize,
    allocated_pixel_bytes: usize,
    allocated_pixel_bytes_after_undo: usize,
    allocated_pixel_bytes_after_redo: usize,
    logical_pixel_bytes: usize,
    resident_bytes_after_paint: Option<usize>,
    buffer_payload_capacity_bytes: usize,
    buffer_release_ms: f64,
    buffer_tile_reclaim_ms: f64,
}

#[derive(Serialize)]
struct Clear {
    canvas: [usize; 2],
    painted: bool,
    clear_ms: f64,
    changed: bool,
    allocated_pixel_bytes_before: usize,
    allocated_pixel_bytes_after: usize,
    logical_pixel_bytes: usize,
}
fn clear_workload(painted: bool) -> Result<Clear, String> {
    let side = 4096;
    let mut doc = Document::new(side, side);
    if painted {
        let mut writer = doc.layers[0].pixels.writer();
        writer.rgba_mut(0).copy_from_slice(&[1, 2, 3, 255]);
        writer
            .rgba_mut(side * side - 1)
            .copy_from_slice(&[4, 5, 6, 128]);
    }
    // Match the undoable command's copy-on-write ownership without timing its
    // history settlement. This is a synthetic canvas above the shipping cap.
    let _snapshot = doc.clone();
    let allocated_pixel_bytes_before = doc.layers[0].pixels.capacity();
    let start = Instant::now();
    let changed = doc.clear_selected()?;
    let clear_ms = start.elapsed().as_secs_f64() * 1000.0;
    if changed != painted
        || doc.layers[0].pixels.rgba(0) != [0; 4]
        || doc.layers[0].pixels.rgba(side * side - 1) != [0; 4]
    {
        return Err("Clear benchmark failed to restore expected pixels".into());
    }
    Ok(Clear {
        canvas: [side, side],
        painted,
        clear_ms,
        changed,
        allocated_pixel_bytes_before,
        allocated_pixel_bytes_after: doc.layers[0].pixels.capacity(),
        logical_pixel_bytes: side * side * 4,
    })
}

#[derive(Serialize)]
struct TransformMeasurement {
    canvas: [usize; 2],
    operation: &'static str,
    transform_ms: f64,
    allocated_pixel_bytes_before: usize,
    allocated_pixel_bytes_after: usize,
    logical_pixel_bytes: usize,
}
fn transform_workload(rotate: bool) -> Result<TransformMeasurement, String> {
    let side = 4096;
    let mut doc = Document::new(side, side);
    {
        let mut writer = doc.layers[0].pixels.writer();
        writer
            .rgba_mut(200 * side + 200)
            .copy_from_slice(&[1, 2, 3, 255]);
        writer
            .rgba_mut((side - 201) * side + side - 201)
            .copy_from_slice(&[4, 5, 6, 128]);
    }
    let _snapshot = doc.clone();
    let allocated_pixel_bytes_before = doc.layers[0].pixels.capacity();
    let start = Instant::now();
    if rotate {
        doc.rotate_canvas_clockwise();
    } else if !doc.transform_active(crate::transform::Transform::FlipHorizontal)? {
        return Err("Transform benchmark unexpectedly made no change".into());
    }
    let transform_ms = start.elapsed().as_secs_f64() * 1000.0;
    let allocated_pixel_bytes_after = doc.layers[0].pixels.capacity();
    if doc.layers[0].pixels.rgba(200 * side + side - 201) != [1, 2, 3, 255] {
        return Err("Transform benchmark failed to move the first pixel".into());
    }
    Ok(TransformMeasurement {
        canvas: [side, side],
        operation: if rotate {
            "rotate_canvas_clockwise"
        } else {
            "flip_horizontal"
        },
        transform_ms,
        allocated_pixel_bytes_before,
        allocated_pixel_bytes_after,
        logical_pixel_bytes: side * side * 4,
    })
}

#[derive(Serialize)]
struct ProjectStorage {
    version: u8,
    canvas: [usize; 2],
    layers: usize,
    encode_ms: f64,
    decode_ms: f64,
    compressed_bytes: usize,
    loaded_pixel_capacity: usize,
    logical_pixel_bytes: usize,
}
fn project_workload(v4: bool) -> Result<ProjectStorage, String> {
    let side = 2048;
    let mut doc = Document::new(side, side);
    for _ in 1..20 {
        doc.add_layer();
    }
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&[1, 2, 3, 255]);
    doc.layers[0]
        .pixels
        .writer()
        .rgba_mut(side * side - 1)
        .copy_from_slice(&[4, 5, 6, 0]);
    doc.layers[19]
        .pixels
        .writer()
        .rgba_mut(side * side - 1)
        .copy_from_slice(&[7, 8, 9, 128]);
    let mut encoded = Vec::new();
    let start = Instant::now();
    if v4 {
        crate::tile_project::encode(&doc, &mut encoded)?;
    } else {
        crate::binary_project::encode(&doc, &mut encoded)?;
    }
    let encode_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let loaded = if v4 {
        crate::tile_project::decode(&encoded[crate::tile_project::MAGIC.len()..])?
    } else {
        crate::binary_project::decode(&encoded[crate::binary_project::MAGIC.len()..])?
    };
    let decode_ms = start.elapsed().as_secs_f64() * 1000.0;
    if loaded.layers[0].pixels.rgba(side * side - 1) != [4, 5, 6, 0]
        || loaded.layers[19].pixels.rgba(side * side - 1) != [7, 8, 9, 128]
    {
        return Err("Project storage benchmark failed round-trip verification".into());
    }
    Ok(ProjectStorage {
        version: if v4 { 4 } else { 3 },
        canvas: [side, side],
        layers: 20,
        encode_ms,
        decode_ms,
        compressed_bytes: encoded.len(),
        loaded_pixel_capacity: loaded
            .layers
            .iter()
            .map(|layer| layer.pixels.capacity())
            .sum(),
        logical_pixel_bytes: side * side * 4 * 20,
    })
}

#[derive(Serialize)]
struct Display {
    layers: usize,
    first_incremental_update_ms: f64,
    warm_incremental_update_ms: f64,
    unchanged_refresh_ms: f64,
    unchanged_refresh_upload_bytes: usize,
    tiled: bool,
    canvas: [usize; 2],
    initial_update_ms: f64,
    initial_upload_bytes: usize,
    initial_texture_count: usize,
    incremental_updates: usize,
    incremental_update_ms: f64,
    incremental_upload_bytes: usize,
}
fn display_workload(tiled: bool, layer_count: usize) -> Display {
    let ctx = crate::egui::Context::default();
    let mut doc = Document::new(1024, 768);
    let brush = Brush::default();
    for _ in 1..layer_count {
        doc.add_layer();
        doc.dab(256.0, 256.0, 1.0, &brush, [40, 90, 140], false);
    }
    doc.dab(256.0, 256.0, 1.0, &brush, [40, 90, 140], false);
    let mut tiles = crate::canvas_textures::CanvasTextures::default();
    let mut legacy: Option<crate::egui::TextureHandle> = None;
    let mut performance = crate::performance::Performance::default();
    let options = crate::egui::TextureOptions {
        mipmap_mode: Some(crate::egui::TextureFilter::Linear),
        ..crate::egui::TextureOptions::LINEAR
    };
    let mut update = |doc: &Document, full: bool, dirty: Option<[usize; 4]>| {
        ctx.run(Default::default(), |ctx| {
            if tiled {
                tiles.update(ctx, doc, full, dirty, options, &mut performance);
            } else if full {
                let image = crate::egui::ColorImage::from_rgba_unmultiplied(
                    [doc.width, doc.height],
                    &doc.composite(),
                );
                legacy = Some(ctx.load_texture("legacy-display-benchmark", image, options));
            } else if let Some([left, top, right, bottom]) = dirty {
                let image = crate::egui::ColorImage::from_rgba_unmultiplied(
                    [right - left, bottom - top],
                    &doc.composite_region(left, top, right, bottom),
                );
                legacy
                    .as_mut()
                    .unwrap()
                    .set_partial([left, top], image, options);
            }
        })
    };
    let bytes = |output: &crate::egui::FullOutput| {
        output
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| *id != crate::egui::TextureId::Managed(0))
            .map(|(_, delta)| {
                let size = delta.image.size();
                size[0] * size[1] * 4
            })
            .sum::<usize>()
    };
    let start = Instant::now();
    let initial = update(&doc, true, None);
    let initial_update_ms = start.elapsed().as_secs_f64() * 1000.0;
    let initial_upload_bytes = bytes(&initial);
    let initial_texture_count = initial
        .textures_delta
        .set
        .iter()
        .filter(|(id, _)| *id != crate::egui::TextureId::Managed(0))
        .count();
    let mut incremental_update_ms = 0.0;
    let start = Instant::now();
    let unchanged = update(&doc, true, None);
    let unchanged_refresh_ms = start.elapsed().as_secs_f64() * 1000.0;
    let unchanged_refresh_upload_bytes = bytes(&unchanged);
    let mut incremental_upload_bytes = 0;
    let mut first_incremental_update_ms = 0.0;
    let mut warm_incremental_update_ms = 0.0;
    for step in 0..8 {
        let x = 248.0 + step as f32 * 2.0;
        doc.dab(x, 255.5, 1.0, &brush, [180, 60, 90], false);
        let dirty = crate::render_region::dab_bounds(x, 255.5, &brush, doc.width, doc.height);
        let start = Instant::now();
        let output = update(&doc, false, Some(dirty));
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        incremental_update_ms += elapsed;
        if step == 0 {
            first_incremental_update_ms = elapsed;
        } else {
            warm_incremental_update_ms += elapsed;
        }
        incremental_upload_bytes += bytes(&output);
    }
    Display {
        layers: layer_count,
        first_incremental_update_ms,
        warm_incremental_update_ms,
        unchanged_refresh_ms,
        unchanged_refresh_upload_bytes,
        tiled,
        canvas: [doc.width, doc.height],
        initial_update_ms,
        initial_upload_bytes,
        initial_texture_count,
        incremental_updates: 8,
        incremental_update_ms,
        incremental_upload_bytes,
    }
}

#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct ProcessMemory {
    pub resident_bytes: usize,
    pub private_committed_bytes: usize,
    pub peak_resident_bytes: usize,
    pub peak_private_committed_bytes: usize,
}

#[cfg(windows)]
pub(crate) fn process_memory() -> Option<ProcessMemory> {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    // All-zero counters are valid output storage. The current-process pseudo
    // handle stays valid and the API receives the exact structure size.
    let mut counters: Counters = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<Counters>() as u32;
    let success =
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (success != 0).then_some(ProcessMemory {
        resident_bytes: counters.working_set,
        private_committed_bytes: counters.private_usage,
        peak_resident_bytes: counters.peak_working_set,
        peak_private_committed_bytes: counters.peak_pagefile,
    })
}
#[cfg(not(windows))]
pub(crate) fn process_memory() -> Option<ProcessMemory> {
    None
}

pub(crate) fn resident_bytes() -> Option<usize> {
    process_memory().map(|m| m.resident_bytes)
}

#[cfg(all(test, windows))]
#[test]
fn windows_process_peak_counters_cover_current_commitment_and_residency() {
    let memory = process_memory().expect("Windows process memory counters");
    assert!(memory.private_committed_bytes > 0 && memory.resident_bytes > 0);
    assert!(memory.peak_private_committed_bytes >= memory.private_committed_bytes);
    assert!(memory.peak_resident_bytes >= memory.resident_bytes);
}

fn lifecycle(
    side: usize,
    layers: usize,
    buffered: bool,
    diagonal: bool,
) -> Result<Lifecycle, String> {
    let mut doc = Document::new(side, side);
    for _ in 1..layers {
        doc.add_layer();
    }
    // Deliberately exercise the planned 4096-square workload above today's
    // project pixel cap. Do not save it or imply it is a supported project.
    let mut history = crate::document::History::new();
    let brush = Brush::default();
    let mut buffer = buffered.then(crate::stroke_buffer::StrokeBuffer::default);
    let start = Instant::now();
    history.checkpoint(&doc);
    let checkpoint_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    doc.dab_buffered(
        200.0,
        200.0,
        1.0,
        &brush,
        [38, 44, 59],
        false,
        crate::symmetry::IDENTITY,
        buffer.as_mut(),
    );
    let pen_down_ms = start.elapsed().as_secs_f64() * 1000.0;
    for x in 201..400 {
        doc.dab_buffered(
            if diagonal {
                200.0 + (x - 200) as f32 * (side - 400) as f32 / 199.0
            } else {
                x as f32
            },
            if diagonal {
                200.0 + (x - 200) as f32 * (side - 400) as f32 / 199.0
            } else {
                200.0
            },
            1.0,
            &brush,
            [38, 44, 59],
            false,
            crate::symmetry::IDENTITY,
            buffer.as_mut(),
        );
    }
    let resident_bytes_after_paint = resident_bytes();
    let allocated_pixel_bytes = doc.layers.iter().map(|layer| layer.pixels.capacity()).sum();
    let buffer_payload_capacity_bytes = buffer
        .as_ref()
        .map_or(0, |buffer| buffer.payload_capacity_bytes());
    let reclaim = Instant::now();
    if let Some(buffer) = &buffer {
        buffer.reclaim_empty_tiles(&mut doc);
    }
    let buffer_tile_reclaim_ms = reclaim.elapsed().as_secs_f64() * 1000.0;
    let release = Instant::now();
    drop(buffer);
    let buffer_release_ms = release.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    history.settle(&doc);
    let pen_up_history_ms = start.elapsed().as_secs_f64() * 1000.0;
    let history_bytes = history.retained_bytes();
    let start = Instant::now();
    if !history.undo(&mut doc) {
        return Err("Benchmark undo was unavailable".into());
    }
    let undo_ms = start.elapsed().as_secs_f64() * 1000.0;
    let allocated_pixel_bytes_after_undo =
        doc.layers.iter().map(|layer| layer.pixels.capacity()).sum();
    if (0..side * side).any(|index| doc.layers[doc.active].pixels.rgba(index) != [0; 4]) {
        return Err("Benchmark undo failed to restore blank layer".into());
    }
    let start = Instant::now();
    if !history.redo(&mut doc) {
        return Err("Benchmark redo was unavailable".into());
    }
    let redo_ms = start.elapsed().as_secs_f64() * 1000.0;
    let allocated_pixel_bytes_after_redo =
        doc.layers.iter().map(|layer| layer.pixels.capacity()).sum();
    if doc.layers[doc.active].pixels.rgba(200 * side + 200)[3] == 0 {
        return Err("Benchmark redo failed to restore painting".into());
    }
    Ok(Lifecycle {
        canvas: [side, side],
        layers,
        stroke_shape: if diagonal { "diagonal" } else { "horizontal" },
        checkpoint_ms,
        pen_down_ms,
        pen_up_history_ms,
        undo_ms,
        redo_ms,
        history_bytes,
        allocated_pixel_bytes,
        allocated_pixel_bytes_after_undo,
        allocated_pixel_bytes_after_redo,
        logical_pixel_bytes: side * side * 4 * layers,
        resident_bytes_after_paint,
        buffer_payload_capacity_bytes,
        buffer_release_ms,
        buffer_tile_reclaim_ms,
    })
}

pub fn run(path: &std::path::Path, incremental: bool, buffered: bool) -> Result<(), String> {
    let mut brushes = vec![
        Brush::default(),
        Brush {
            name: "Grain pencil".into(),
            size: 18.0,
            grain_amount: 0.8,
            ..Brush::default()
        },
        Brush {
            name: "Large soft brush".into(),
            size: 150.0,
            hardness: 0.0,
            opacity: 0.3,
            ..Brush::default()
        },
    ];
    brushes.push(Brush {
        name: "Large eased soft brush".into(),
        size: 150.0,
        hardness: 0.0,
        opacity: 0.3,
        softness_profile: crate::brush::SoftnessProfile::Eased,
        ..Default::default()
    });
    brushes.push(Brush {
        name: "Compensated airbrush".into(),
        size: 150.0,
        hardness: 0.0,
        opacity: 0.3,
        flow: 0.1,
        pressure_size: false,
        adjust_flow_by_spacing: true,
        softness_profile: crate::brush::SoftnessProfile::Eased,
        ..Default::default()
    });
    let mut dynamic_ink = Brush {
        name: "Speed dynamics ink".into(),
        taper_in: 20.0,
        stabilization: 4.0,
        stabilization_space: crate::brush::StabilizationSpace::Screen,
        stabilization_by_speed: true,
        ..Default::default()
    };
    dynamic_ink.dynamics.size = Some(crate::dynamics::Dynamics {
        minimum: 0.2,
        minimum_mode: crate::dynamics::MinimumMode::Clamp,
        inputs: vec![crate::dynamics::Effector {
            input: crate::dynamics::Input::Speed,
            curve: crate::dynamics::Curve {
                points: vec![[0.0, 1.0], [1.0, 0.3]],
            },
        }],
    });
    brushes.push(dynamic_ink);
    if buffered {
        brushes.push(Brush {
            name: "Tapered ink".into(),
            taper_in: 20.0,
            taper_out: 20.0,
            opacity: 0.5,
            flow: 0.3,
            ..Default::default()
        });
    }
    let brush_settings = brushes.clone();
    let mut measurements = Vec::new();
    for brush in brushes {
        let mut doc = Document::new(2048, 2048);
        let mut stroke = Stroke::default();
        let mut buffer = buffered.then(crate::stroke_buffer::StrokeBuffer::default);
        let settings = crate::stroke_buffer::Settings {
            target: crate::paint_target::Target::Raster,
            brush: brush.clone(),
            color: [38, 44, 59],
            erase: false,
            layer: 0,
            symmetry: Default::default(),
        };
        let mut measurement = Measurement {
            brush: brush.name.clone(),
            frames: 12,
            dabs: 0,
            paint_ms: 0.0,
            composite_ms: 0.0,
            max_engine_frame_ms: 0.0,
            buffer_payload_capacity_bytes: 0,
            tail_peak_dabs: 0,
            tail_finish_ms: 0.0,
        };
        for frame in 0..measurement.frames {
            let start = Instant::now();
            let mut region = None;
            // Eight ordered 480 Hz samples per simulated 60 Hz frame.
            for packet in 0..8 {
                let t = (frame * 8 + packet) as f32 / 95.0;
                let sample = Sample {
                    x: 300.0 + t * 1400.0,
                    y: 1024.0 + (t * std::f32::consts::TAU).sin() * 400.0,
                    pressure: 0.15 + t * 0.85,
                    time: Some((frame * 8 + packet) as f64 / 480.0),
                    ..Default::default()
                };
                let mut dabs = stroke.sample(sample, &brush);
                if frame == 11 && packet == 7 {
                    dabs.extend(stroke.finish(sample, &brush));
                }
                measurement.dabs += dabs.len();
                for dab in dabs {
                    crate::render_region::include(
                        &mut region,
                        crate::render_region::dab_bounds(
                            dab.x, dab.y, &brush, doc.width, doc.height,
                        ),
                    );
                    if let Some(buffer) = &mut buffer {
                        let painted = buffer.paint(&mut doc, dab, &settings);
                        if let Some(bounds) = painted.region.bounds() {
                            crate::render_region::include(&mut region, bounds);
                        }
                    } else {
                        doc.dab_sample_buffered(
                            dab,
                            &brush,
                            [38, 44, 59],
                            false,
                            crate::symmetry::IDENTITY,
                            None,
                        );
                    }
                }
            }
            if frame + 1 == measurement.frames
                && let Some(buffer) = &mut buffer
            {
                let finish = Instant::now();
                let painted = buffer.finish_tail(&mut doc, &settings);
                measurement.tail_finish_ms = finish.elapsed().as_secs_f64() * 1000.0;
                if let Some(bounds) = painted.region.bounds() {
                    crate::render_region::include(&mut region, bounds);
                }
            }
            measurement.paint_ms += start.elapsed().as_secs_f64() * 1000.0;
            let composite = Instant::now();
            if incremental {
                if let Some([left, top, right, bottom]) = region {
                    black_box(doc.composite_region(left, top, right, bottom));
                }
            } else {
                black_box(doc.composite());
            }
            measurement.composite_ms += composite.elapsed().as_secs_f64() * 1000.0;
            measurement.max_engine_frame_ms = measurement
                .max_engine_frame_ms
                .max(start.elapsed().as_secs_f64() * 1000.0);
        }
        measurement.buffer_payload_capacity_bytes = buffer
            .as_ref()
            .map_or(0, |buffer| buffer.payload_capacity_bytes());
        measurement.tail_peak_dabs = buffer.as_ref().map_or(0, |buffer| buffer.tail_peak_dabs());
        measurements.push(measurement);
    }
    let report = Report {
        incremental,
        buffered,
        buffer_memory_scope: "Sparse 32x32 original/coverage blocks, finalized-prefix coverage, block-index and pending-tail payload capacity; excludes map control bytes, original-block vector capacity, allocator overhead and captured brush assets. Buffer release measured separately before history settlement. 4096-square workload is synthetic and exceeds the current supported project pixel cap.",
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        canvas: [2048, 2048],
        scope: "Measurements: CPU stroke rasterization and composite, synthetic 12-frame workload. Lifecycle: one checkpoint, first dab (including copy-on-write), a 200-dab stroke (horizontal, plus a separate 4096-square diagonal case), history settle, undo and redo; 2048 square/one layer and 4096 square/20 layers. Clear: blank/two-visible-pixel 4096-square layer with a shared snapshot, excluding history settlement. Transforms: two visible pixels on a 4096-square layer with a shared snapshot, excluding history settlement. Project storage: v3/v4 encode to memory and decode a supported 2048-square/20-layer three-pixel document, excluding filesystem commits and UI. Display: legacy single texture vs padded tiles on 1024x768/two layers, CPU preparation and queued RGBA bytes; excludes GPU execution. Resident bytes are the entire process working set sampled after painting, not isolated document memory. Excludes textures, UI, GPU, tablet driver and input latency; not a 60 FPS certification.",
        measurements,
        brush_settings,
        tail_workloads: if buffered { tail_workloads()? } else { vec![] },
        filters: vec![
            filter_workload(false, false, false)?,
            filter_workload(false, false, true)?,
            filter_workload(true, false, false)?,
            filter_workload(true, false, true)?,
            filter_workload(false, true, false)?,
            filter_workload(false, true, true)?,
            filter_workload(true, true, false)?,
            filter_workload(true, true, true)?,
        ],
        cold_history: vec![
            cold_history_workload(1, false, false)?,
            cold_history_workload(1, true, false)?,
            cold_history_workload(20, false, false)?,
            cold_history_workload(20, true, false)?,
            cold_history_workload(20, true, true)?,
        ],
        cold_history_scope: "Supported 2048-square canvases validated through Document::validate, with one or twenty dense layers and six complete edits of layer zero. Raw and cooled history in the same build; one shared compression/prefetch worker, 1ms benchmark polling (the UI requests 16ms repaint while idle). The additional twenty-layer prefetch mode drains idle jobs between actions under 32MiB and available history space; rapid modes have no idle opportunity between actions. Timings include remaining cold decoding before applying an action. Every active-layer pixel is checked after each undo/redo outside its timer. Excludes dense fixture setup, GPU, composition, save/recovery and physical input latency. Retained-byte estimates include patch/grid metadata and raw/packed Arc headers, but exclude allocator bookkeeping, codec workspace, worker stack and transient worker output. Live pixel capacity excludes these history allocations.",
        lifecycle: vec![
            lifecycle(2048, 1, buffered, false)?,
            lifecycle(4096, 20, buffered, false)?,
            lifecycle(4096, 20, buffered, true)?,
        ],
        clear: vec![clear_workload(false)?, clear_workload(true)?],
        transforms: vec![transform_workload(false)?, transform_workload(true)?],
        project_storage: vec![project_workload(false)?, project_workload(true)?],
        display: vec![
            display_workload(false, 2),
            display_workload(true, 2),
            display_workload(false, 20),
            display_workload(true, 20),
        ],
    };
    let data = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    crate::storage::atomic_write(path, &data)
}
