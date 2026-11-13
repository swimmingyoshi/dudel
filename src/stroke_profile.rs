//! `--profile-stroke REPORT.json [FILTER] [IMAGES]`: where a long wet stroke's
//! time goes.
//!
//! Each case paints one scripted stroke through the app's own canvas frame
//! (pen packets, the painting queue, display compositing) without a window,
//! in real time: packets arrive on a 200 Hz pen clock and frames are shown
//! on a 60 Hz display, a frame that takes longer waiting for the refresh
//! after (one case runs its frames back to back). The report has every
//! frame (`stage_timer::Frame`) and a summary per case. With a folder as
//! IMAGES, each case's finished canvas is saved there as a PNG, to compare
//! the look before and after a change to the painter.
//!
//! Not measured: the GPU (texture upload and presenting), the pen driver and
//! the rest of the window's interface. Texture bytes handed to the GPU are
//! counted instead.
use crate::{
    Dudel,
    brush::{Blend, Brush},
    egui::{self, pos2, vec2},
    input::Packet,
    material::{Kind, Material, Settings},
    stage_timer::{self, Frame},
    stroke::Sample,
};
use std::time::{Duration, Instant};

/// The canvas of every case that does not name its own.
const CANVAS: [usize; 2] = [3000, 2000];
const WINDOW: [f32; 2] = [1600.0, 1000.0];
/// Pen reports per second.
const PEN_HZ: f64 = 200.0;
/// Canvas pixels per second unless the case says otherwise: a brisk stroke
/// at this zoom (about half).
const SPEED: f64 = 1800.0;
const FRAME: Duration = Duration::from_micros(16_667);
/// What a frame costs after its CPU work when frames follow one another at
/// once: the gap between a frame's CPU time and the time to the next frame
/// in the trace of 2026-11-13 from a real pen (2.1 ms on each of three
/// strokes).
const PRESENT: Duration = Duration::from_micros(2_100);

struct Case {
    name: &'static str,
    /// A built-in brush by name, or the default round brush.
    preset: Option<&'static str>,
    size: Option<f32>,
    /// Size follows pressure (the presets' own setting when `None`).
    pressure_size: Option<bool>,
    layer: Option<Kind>,
    paper: bool,
    /// Paint a wash first and leave it wet under the measured stroke.
    over_wet: bool,
    canvas: [usize; 2],
    /// What the dabs do, and the water of a wet brush (the preset's own
    /// when `None`).
    blend: Option<Blend>,
    wetness: Option<f32>,
    /// Canvas pixels the pen covers per second.
    speed: f64,
    /// Frames wait for a 60 Hz display, as with Low latency off. Otherwise
    /// they follow one another after `PRESENT`, as with it on (the default).
    display_paced: bool,
}

const fn case(name: &'static str, preset: Option<&'static str>, size: Option<f32>) -> Case {
    Case {
        name,
        preset,
        size,
        pressure_size: None,
        layer: None,
        paper: true,
        over_wet: false,
        canvas: CANVAS,
        blend: None,
        wetness: None,
        speed: SPEED,
        display_paced: true,
    }
}

/// The owner's report of 2026-11-12: the Chisel marker made wet, at 300 px
/// on a 2048 px canvas. Its narrow tip puts dabs 4.5 px apart.
fn chisel(name: &'static str) -> Case {
    Case {
        canvas: [2048, 2048],
        blend: Some(Blend::Wet),
        wetness: Some(0.5),
        ..case(name, Some("Chisel marker"), Some(300.0))
    }
}

/// The Chisel marker cases the way the trace from a real pen ran them.
fn traced(name: &'static str) -> Case {
    Case {
        paper: false,
        display_paced: false,
        speed: 4000.0,
        ..chisel(name)
    }
}

fn cases() -> Vec<Case> {
    let wash = Some("Watercolor wash");
    vec![
        Case {
            paper: false,
            ..case("plain brush 60 px, no paper", None, Some(60.0))
        },
        case("plain brush 60 px, paper", None, Some(60.0)),
        case("Watercolor wash 60 px (as built in)", wash, None),
        Case {
            paper: false,
            ..case("Watercolor wash 60 px, no paper", wash, None)
        },
        case(
            "Watercolor round 18 px (as built in)",
            Some("Watercolor round"),
            None,
        ),
        case("Water brush 44 px (as built in)", Some("Water brush"), None),
        case("Watercolor wash 150 px", wash, Some(150.0)),
        Case {
            pressure_size: Some(true),
            ..case(
                "Watercolor wash 150 px, size by pressure",
                wash,
                Some(150.0),
            )
        },
        case("Watercolor wash 300 px", wash, Some(300.0)),
        Case {
            paper: false,
            ..case("Watercolor wash 300 px, no paper", wash, Some(300.0))
        },
        Case {
            pressure_size: Some(true),
            ..case(
                "Watercolor wash 300 px, size by pressure",
                wash,
                Some(300.0),
            )
        },
        Case {
            layer: Some(Kind::Watercolor),
            ..case("Watercolor wash 60 px, watercolor layer", wash, None)
        },
        Case {
            layer: Some(Kind::Watercolor),
            pressure_size: Some(true),
            ..case(
                "Watercolor wash 150 px, size by pressure, watercolor layer",
                wash,
                Some(150.0),
            )
        },
        Case {
            over_wet: true,
            ..case("Watercolor wash 150 px, over a wet wash", wash, Some(150.0))
        },
        Case {
            blend: None,
            ..chisel("Chisel marker 300 px, paint, 2048 canvas")
        },
        chisel("Chisel marker 300 px, wet 50%, 2048 canvas"),
        Case {
            paper: false,
            ..chisel("Chisel marker 300 px, wet 50%, 2048 canvas, no paper")
        },
        Case {
            pressure_size: Some(true),
            ..chisel("Chisel marker 300 px, wet 50%, 2048 canvas, size by pressure")
        },
        Case {
            layer: Some(Kind::Watercolor),
            ..chisel("Chisel marker 300 px, wet 50%, 2048 canvas, watercolor layer")
        },
        Case {
            speed: 600.0,
            ..chisel("Chisel marker 300 px, wet 50%, 2048 canvas, unhurried pen")
        },
        Case {
            display_paced: false,
            ..chisel("Chisel marker 300 px, wet 50%, 2048 canvas, frames back to back")
        },
        // The trace of 2026-11-13 from a real pen: no paper, Low latency on,
        // the pen at 2,900 to 5,000 px/s, and plain paint.
        Case {
            blend: None,
            ..traced("Chisel marker 300 px, paint, as traced (4,000 px/s)")
        },
        traced("Chisel marker 300 px, wet 50%, as traced (4,000 px/s)"),
        Case {
            speed: 2900.0,
            ..traced("Chisel marker 300 px, wet 50%, as traced (2,900 px/s)")
        },
    ]
}

/// Pen positions on the canvas with pressure: three sweeps across, each a
/// little below the last so they overlap, pressure rising and falling.
fn path(canvas: [usize; 2], size: f32, offset: f32, speed: f64) -> Vec<(f64, [f32; 2], f32)> {
    let (width, height) = (canvas[0] as f32, canvas[1] as f32);
    let (left, right) = (width * 0.1, width * 0.9);
    let step = (size * 0.6).max(12.0);
    let top = height * 0.35 + offset;
    let mut corners = Vec::new();
    for sweep in 0..3 {
        let y = top + sweep as f32 * step;
        let (from, to) = if sweep % 2 == 0 {
            (left, right)
        } else {
            (right, left)
        };
        corners.push([from, y]);
        corners.push([to, y]);
    }
    let lengths: Vec<f32> = corners
        .windows(2)
        .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
        .collect();
    let total: f32 = lengths.iter().sum();
    let count = (f64::from(total) / speed * PEN_HZ).ceil() as usize;
    (0..=count)
        .map(|i| {
            let u = i as f32 / count as f32;
            let mut along = u * total;
            let mut at = corners[corners.len() - 1];
            for (pair, length) in corners.windows(2).zip(&lengths) {
                if along <= *length {
                    let t = along / length.max(1e-6);
                    at = [
                        pair[0][0] + (pair[1][0] - pair[0][0]) * t,
                        pair[0][1] + (pair[1][1] - pair[0][1]) * t,
                    ];
                    break;
                }
                along -= length;
            }
            let wave = (u * 3.0 * std::f32::consts::PI).sin();
            (i as f64 / PEN_HZ, at, 0.2 + 0.8 * wave * wave)
        })
        .collect()
}

#[derive(serde::Serialize)]
struct Row {
    #[serde(flatten)]
    frame: Frame,
    /// Pen packets delivered this frame.
    packets: usize,
    /// Texture bytes this frame hands to the GPU.
    upload_bytes: usize,
}

#[derive(serde::Serialize)]
struct Third {
    frames: usize,
    dabs: usize,
    mean_frame_ms: f64,
    pump_ms_per_dab: f64,
    display_ms_per_frame: f64,
    expand_ms_per_frame: f64,
}

#[derive(serde::Serialize)]
struct Summary {
    name: &'static str,
    canvas: [usize; 2],
    brush: String,
    blend: String,
    size_px: f32,
    spacing_px: f32,
    pressure_size: bool,
    layer: String,
    paper: bool,
    over_wet: bool,
    stroke_px: f32,
    pen_px_per_second: f64,
    pen_seconds: f64,
    dabs: usize,
    /// Dabs the pen asks for per second, and what the painter managed.
    dabs_asked_per_second: f64,
    frames: usize,
    frame_ms_p50: f64,
    frame_ms_p95: f64,
    frame_ms_max: f64,
    frames_over_17_ms: usize,
    /// The most dabs waiting behind the pen, and how far that is on canvas.
    queued_max: usize,
    behind_px_max: f32,
    queued_at_lift: usize,
    /// After the pen lifts, until the stroke is fully painted.
    catch_up_ms: f64,
    upload_mib: f64,
    /// Milliseconds per stage over the whole stroke.
    stage_ms: std::collections::BTreeMap<&'static str, f64>,
    /// Microseconds per painted dab, per stage.
    stage_us_per_dab: std::collections::BTreeMap<&'static str, f64>,
    /// Frame time not in the pump, paper view, expansion or display: the
    /// canvas's own interface and bookkeeping.
    other_ms: f64,
    /// The stroke in three equal parts of its frames, first to last.
    thirds: Vec<Third>,
    rows: Vec<Row>,
}

fn raw(seconds: f64) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            vec2(WINDOW[0], WINDOW[1]),
        )),
        time: Some(seconds),
        ..Default::default()
    }
}

/// One frame of the canvas with these packets; its record.
fn frame(
    app: &mut Dudel,
    ctx: &egui::Context,
    began: Instant,
    previous: &mut Instant,
    packets: &[Packet],
    display_paced: bool,
) -> Row {
    let start = Instant::now();
    app.performance.begin_frame();
    let output = ctx.run(raw(start.duration_since(began).as_secs_f64()), |ctx| {
        app.canvas(ctx, packets)
    });
    let frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    let interval = start.duration_since(*previous).as_secs_f64() * 1000.0;
    *previous = start;
    let upload_bytes = output
        .textures_delta
        .set
        .iter()
        .filter(|(id, _)| *id != egui::TextureId::Managed(0))
        .map(|(_, delta)| {
            let size = delta.image.size();
            size[0] * size[1] * 4
        })
        .sum();
    let row = Row {
        frame: app.stage_frame(
            start.duration_since(began).as_secs_f64() * 1000.0,
            interval,
            frame_ms,
        ),
        packets: packets.len(),
        upload_bytes,
    };
    // The display's turn: the frame is shown at the next refresh, which a
    // frame longer than one refresh misses.
    if display_paced {
        let elapsed = began.elapsed();
        let refreshes = elapsed.as_nanos() / FRAME.as_nanos() + 1;
        std::thread::sleep(FRAME * refreshes as u32 - elapsed);
    } else {
        std::thread::sleep(PRESENT);
    }
    row
}

/// Paint `points` in real time; rows from the first packet until the stroke
/// is fully painted, and the index of the row that delivered the pen lift.
fn stroke(
    app: &mut Dudel,
    ctx: &egui::Context,
    points: &[(f64, [f32; 2], f32)],
    display_paced: bool,
) -> Result<(Vec<Row>, usize), String> {
    let view = app.view(app.canvas_area);
    let began = Instant::now();
    let mut previous = began;
    let (mut next, mut rows, mut lift) = (0, Vec::new(), None);
    loop {
        let now = began.elapsed().as_secs_f64();
        let mut packets = Vec::new();
        while next < points.len() && points[next].0 <= now {
            let (time, [x, y], pressure) = points[next];
            let p = view.to_screen(pos2(x, y));
            let sample = Sample {
                x: p.x,
                y: p.y,
                pressure,
                pressure_known: true,
                time: Some(1000.0 + time),
                ..Default::default()
            };
            packets.push(if next == 0 {
                Packet::Start(sample)
            } else if next + 1 == points.len() {
                Packet::End(sample)
            } else {
                Packet::Move(sample)
            });
            next += 1;
        }
        if next == points.len() && lift.is_none() {
            lift = Some(rows.len());
        }
        rows.push(frame(
            app,
            ctx,
            began,
            &mut previous,
            &packets,
            display_paced,
        ));
        let last = rows.last().unwrap();
        if let Some(lift) = lift
            && !last.frame.stroking
            && last.frame.queued == 0
            && !app.render_queue.busy()
        {
            return Ok((rows, lift));
        }
        if began.elapsed() > Duration::from_secs(120) {
            return Err(format!("Stroke did not finish: {}", app.status));
        }
    }
}

fn percentile(sorted: &[f64], p: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(sorted.len() * p).div_ceil(100).max(1) - 1]
}

/// The finished canvas of a case, as `IMAGES/case-name.png`.
fn save_image(app: &Dudel, folder: &std::path::Path, name: &str) -> Result<(), String> {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let (width, height) = (app.doc.width as u32, app.doc.height as u32);
    let rgba = app.doc.composite();
    crate::storage::atomic_stream(&folder.join(format!("{slug}.png")), |file| {
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())
    })
}

fn run_case(case: &Case, images: Option<&std::path::Path>) -> Result<Summary, String> {
    let mut app = Dudel {
        doc: crate::document::Document::new(case.canvas[0], case.canvas[1]),
        ..Default::default()
    };
    let mut brush = match case.preset {
        Some(name) => app
            .brushes
            .iter()
            .find(|b| b.name == name)
            .cloned()
            .ok_or_else(|| format!("No built-in brush named {name}"))?,
        None => Brush::default(),
    };
    if let Some(size) = case.size {
        brush.size = size;
    }
    if let Some(pressure_size) = case.pressure_size {
        brush.pressure_size = pressure_size;
    }
    if let Some(blend) = case.blend {
        brush.blend = blend;
    }
    if let Some(wetness) = case.wetness {
        brush.wetness = wetness;
    }
    app.doc.layers[0].material = case.layer.map(|kind| Material::new(Settings::new(kind)));
    if case.paper {
        app.doc.paper = crate::paper::Paper::presets()
            .into_iter()
            .find(|(name, _)| *name == "Cold-pressed")
            .map(|(_, paper)| paper);
    }
    app.select_tool(crate::tools::Tool::Brush);
    app.brush = brush.clone();
    app.color = [200, 40, 30];
    app.changed();
    let ctx = egui::Context::default();
    let began = Instant::now();
    let mut previous = began;
    // The first frames fit the canvas to the window and show it; the paper
    // surface arrives a few tiles a frame. Wait until nothing more uploads.
    let mut quiet = 0;
    for _ in 0..600 {
        let row = frame(&mut app, &ctx, began, &mut previous, &[], true);
        quiet = if row.upload_bytes == 0 { quiet + 1 } else { 0 };
        if quiet == 4 {
            break;
        }
    }
    if case.over_wet {
        // A wash under the measured stroke, still wet when it starts.
        stroke(
            &mut app,
            &ctx,
            &path(case.canvas, brush.size, -brush.size * 0.5, case.speed),
            case.display_paced,
        )?;
        for _ in 0..12 {
            frame(&mut app, &ctx, began, &mut previous, &[], true);
        }
    }
    let points = path(case.canvas, brush.size, 0.0, case.speed);
    let stroke_px: f32 = points
        .windows(2)
        .map(|pair| (pair[1].1[0] - pair[0].1[0]).hypot(pair[1].1[1] - pair[0].1[1]))
        .sum();
    let pen_seconds = points.last().unwrap().0;
    stage_timer::take();
    let (rows, lift) = stroke(&mut app, &ctx, &points, case.display_paced)?;
    if let Some(folder) = images {
        save_image(&app, folder, case.name)?;
    }

    let dabs: usize = rows.iter().map(|r| r.frame.dabs).sum();
    let mut frame_ms: Vec<f64> = rows.iter().map(|r| r.frame.frame_ms).collect();
    frame_ms.sort_unstable_by(f64::total_cmp);
    let mut stage_ms = std::collections::BTreeMap::new();
    for row in &rows {
        for (name, ms) in &row.frame.stages {
            *stage_ms.entry(*name).or_insert(0.0) += ms;
        }
    }
    let stage_us_per_dab = stage_ms
        .iter()
        .map(|(name, ms)| (*name, (ms * 1000.0 / dabs.max(1) as f64).round()))
        .collect();
    let total_ms: f64 = frame_ms.iter().sum();
    let of = |name: &str| stage_ms.get(name).copied().unwrap_or(0.0);
    let other_ms = total_ms - of("pump") - of("paper_view") - of("expand") - of("display");
    let queued_max = rows.iter().map(|r| r.frame.queued).max().unwrap_or(0);
    // At full pressure, as the stroke spaces them.
    let spacing_px = brush.dab_spacing(&brush.factors(&Sample {
        pressure: 1.0,
        ..Default::default()
    }))[1];
    let thirds = rows
        .chunks(rows.len().div_ceil(3).max(1))
        .map(|part| {
            let dabs: usize = part.iter().map(|r| r.frame.dabs).sum();
            let sum = |name: &str| -> f64 {
                part.iter()
                    .map(|r| r.frame.stages.get(name).copied().unwrap_or(0.0))
                    .sum()
            };
            let frames = part.len() as f64;
            Third {
                frames: part.len(),
                dabs,
                mean_frame_ms: part.iter().map(|r| r.frame.frame_ms).sum::<f64>() / frames,
                pump_ms_per_dab: sum("pump") / dabs.max(1) as f64,
                display_ms_per_frame: sum("display") / frames,
                expand_ms_per_frame: sum("expand") / frames,
            }
        })
        .collect();
    Ok(Summary {
        name: case.name,
        canvas: case.canvas,
        brush: if brush.name.is_empty() {
            "default round".into()
        } else {
            brush.name.clone()
        },
        blend: format!("{:?}", brush.blend),
        size_px: brush.size,
        spacing_px,
        pressure_size: brush.pressure_size,
        layer: case
            .layer
            .map_or_else(|| "plain".into(), |kind| format!("{kind:?}")),
        paper: case.paper,
        over_wet: case.over_wet,
        stroke_px,
        pen_px_per_second: case.speed,
        pen_seconds,
        dabs,
        dabs_asked_per_second: dabs as f64 / pen_seconds,
        frames: rows.len(),
        frame_ms_p50: percentile(&frame_ms, 50),
        frame_ms_p95: percentile(&frame_ms, 95),
        frame_ms_max: frame_ms.last().copied().unwrap_or(0.0),
        frames_over_17_ms: frame_ms.iter().filter(|ms| **ms > 17.0).count(),
        queued_max,
        behind_px_max: queued_max as f32 * spacing_px,
        queued_at_lift: rows[lift].frame.queued,
        catch_up_ms: rows.last().unwrap().frame.t_ms - rows[lift].frame.t_ms,
        upload_mib: rows.iter().map(|r| r.upload_bytes).sum::<usize>() as f64 / (1024.0 * 1024.0),
        stage_ms,
        stage_us_per_dab,
        other_ms,
        thirds,
        rows,
    })
}

fn print(summary: &Summary) {
    println!("\n== {}", summary.name);
    println!(
        "   {} px stroke in {:.1} s; {} dabs ({:.0} a second asked), spacing {:.1} px",
        summary.stroke_px.round(),
        summary.pen_seconds,
        summary.dabs,
        summary.dabs_asked_per_second,
        summary.spacing_px,
    );
    println!(
        "   frames: {} (p50 {:.1} / p95 {:.1} / max {:.1} ms), {} over 17 ms",
        summary.frames,
        summary.frame_ms_p50,
        summary.frame_ms_p95,
        summary.frame_ms_max,
        summary.frames_over_17_ms,
    );
    println!(
        "   behind the pen: up to {} dabs ({:.0} px); {} at pen lift, painted {:.0} ms later",
        summary.queued_max, summary.behind_px_max, summary.queued_at_lift, summary.catch_up_ms,
    );
    let total: f64 = summary.rows.iter().map(|r| r.frame.frame_ms).sum();
    let mut stages: Vec<_> = summary.stage_ms.iter().collect();
    stages.sort_by(|a, b| b.1.total_cmp(a.1));
    let line: Vec<String> = stages
        .iter()
        .map(|(name, ms)| {
            format!(
                "{name} {:.0} ms ({:.0}%, {:.0} us/dab)",
                ms,
                *ms / total.max(1e-9) * 100.0,
                summary.stage_us_per_dab[*name],
            )
        })
        .collect();
    println!("   frame time {total:.0} ms: {}", line.join("; "));
    println!(
        "   other (interface, bookkeeping) {:.0} ms; textures to GPU {:.1} MiB",
        summary.other_ms, summary.upload_mib
    );
    for (third, name) in summary.thirds.iter().zip(["first", "middle", "last"]) {
        println!(
            "   {name} third: frame {:.1} ms, pump {:.2} ms/dab, display {:.2} ms/frame, expand {:.2} ms/frame, {} dabs",
            third.mean_frame_ms,
            third.pump_ms_per_dab,
            third.display_ms_per_frame,
            third.expand_ms_per_frame,
            third.dabs,
        );
    }
}

pub(crate) fn run(
    path: &std::path::Path,
    filter: Option<&str>,
    images: Option<&std::path::Path>,
) -> Result<(), String> {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err("The report must have a .json extension".into());
    }
    stage_timer::enable();
    let mut summaries = Vec::new();
    for case in cases() {
        if filter.is_some_and(|f| !case.name.to_lowercase().contains(&f.to_lowercase())) {
            continue;
        }
        let summary = run_case(&case, images)?;
        print(&summary);
        summaries.push(summary);
    }
    if summaries.is_empty() {
        return Err("No case matches the filter".into());
    }
    let report = serde_json::json!({
        "version": 2,
        "window": WINDOW,
        "pen_hz": PEN_HZ,
        "pen_speed_px_per_second": SPEED,
        "threads": std::thread::available_parallelism().map_or(0, |n| n.get()),
        "scope": "Scripted strokes through the canvas frame without a window, in real time. \
                  CPU only: no GPU upload or presenting, no pen driver, no side panels.",
        "cases": summaries,
    });
    let bytes = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
    crate::storage::atomic_write(path, &bytes)
}
