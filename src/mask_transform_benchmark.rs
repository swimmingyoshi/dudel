//! Headless application geometry timings, verified independently of transform code.
use crate::{
    affine::{Sampling, Settings},
    document::{Document, LayerId},
    layer_mask::{LayerMask, Mask},
    paint_target::Target,
    selection::Selection,
};
use std::{path::Path, time::Instant};
const OFFSET: [i64; 2] = [17, -9];
const PROFILES: [&str; 5] = [
    "whole-sparse",
    "whole-dense",
    "rectangle-dense",
    "soft-dense",
    "folder-rectangle",
];
#[derive(Clone, Copy)]
enum Operation {
    Move,
    AffineInteger,
    AffineHalf,
}
impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Move => "integer-move",
            Self::AffineInteger => "nearest-affine-translation",
            Self::AffineHalf => "bilinear-half-pixel-translation",
        }
    }
    fn fractional(self) -> bool {
        matches!(self, Self::AffineHalf)
    }
}
pub(crate) fn fixture(w: usize, h: usize, profile: &str) -> Result<(Document, LayerId), String> {
    let mut doc = Document::new(w, h);
    doc.version = 2;
    if profile == "folder-rectangle" {
        doc.layers[0] = crate::document::Layer::new_folder("Masked group".into());
    } else {
        doc.layers[0].pixels = [17, 83, 201, 129].repeat(w * h).into();
    }
    let bounds = Selection {
        left: w / 2 - 32,
        top: h / 2 - 32,
        right: w / 2 + 32,
        bottom: h / 2 + 32,
    };
    let coverage = if profile == "whole-sparse" {
        let mut mask = Mask::new(w, h, 255)?;
        {
            let mut writer = mask.writer();
            for y in bounds.top..bounds.bottom {
                for x in bounds.left..bounds.right {
                    writer.set(x, y, 43);
                }
            }
        }
        mask
    } else {
        Mask::generate(w, h, 255, None, |x, y| {
            Ok(((x / 7 + y / 11) * 37 % 251) as u8)
        })?
    };
    let id = LayerId::new();
    doc.layers[0].mask = Some(LayerMask {
        id,
        enabled: true,
        linked: false,
        coverage,
    });
    doc.layers[0].alpha_locked = true;
    if profile.contains("rectangle") {
        doc.selection = Some(bounds);
    }
    if profile == "soft-dense" {
        let mut bytes = vec![0; w * h];
        for y in bounds.top..bounds.bottom {
            for x in bounds.left..bounds.right {
                let edge = (x - bounds.left)
                    .min(bounds.right - 1 - x)
                    .min(y - bounds.top)
                    .min(bounds.bottom - 1 - y);
                bytes[y * w + x] = [64, 128, 192, 255][edge.min(3)];
            }
        }
        let mask = crate::selection_mask::Mask::from_coverage(w, h, bytes)?;
        doc.selection = mask.bounds();
        doc.selection_mask = Some(mask);
    }
    doc.validate()?;
    Ok((doc, id))
}
fn selection(doc: &Document, x: usize, y: usize) -> u8 {
    if doc.selection.is_some_and(|r| !r.contains(x, y)) {
        0
    } else {
        doc.selection_mask.as_ref().map_or(255, |m| m.at(x, y))
    }
}
fn verify(source: &Document, result: &Document, op: Operation) -> Result<(), String> {
    result.validate()?;
    let before = source.layers[0].mask.as_ref().unwrap();
    let after = result.layers[0]
        .mask
        .as_ref()
        .ok_or("Missing transformed mask")?;
    if !result.layers[0].pixels.shares(&source.layers[0].pixels)
        || before.id != after.id
        || before.enabled != after.enabled
        || before.linked != after.linked
    {
        return Err("Mask probe changed artwork allocation or mask metadata".into());
    }
    let explicit = source.selection.is_some() || source.selection_mask.is_some();
    let tap = |sx: i64, sy: i64| -> (u32, u32) {
        if sx < 0 || sy < 0 || sx >= source.width as i64 || sy >= source.height as i64 {
            (255, 0)
        } else {
            (
                u32::from(before.coverage.at(sx as usize, sy as usize)),
                if explicit {
                    u32::from(selection(source, sx as usize, sy as usize))
                } else {
                    255
                },
            )
        }
    };
    for y in 0..source.height {
        for x in 0..source.width {
            let (gray, incoming) = if op.fractional() {
                // x-17.5,y+8.5: four equally weighted pixel-centre taps.
                let (mut weighted, mut covered, mut raw) = (0, 0, 0);
                for dy in 0..2 {
                    for dx in 0..2 {
                        let (v, c) = tap(x as i64 - 18 + dx, y as i64 + 8 + dy);
                        weighted += v * c;
                        covered += c;
                        raw += v;
                    }
                }
                if !explicit {
                    ((raw + 2) / 4, 255)
                } else if (covered + 2) / 4 == 0 {
                    (255, 0)
                } else {
                    ((weighted + covered / 2) / covered, (covered + 2) / 4)
                }
            } else {
                tap(x as i64 - OFFSET[0], y as i64 - OFFSET[1])
            };
            let expected = if explicit {
                let departing = u32::from(selection(source, x, y));
                let untouched = 255 - departing.max(incoming);
                let revealed = departing.saturating_sub(incoming);
                (u32::from(before.coverage.at(x, y)) * untouched
                    + gray * incoming
                    + 255 * revealed
                    + 127)
                    / 255
            } else {
                gray
            };
            if u32::from(after.coverage.at(x, y)) != expected
                || (explicit && u32::from(selection(result, x, y)) != incoming)
            {
                return Err(format!(
                    "Independent mask/selection oracle differs at {x},{y} ({})",
                    op.name()
                ));
            }
        }
    }
    if !explicit && (result.selection.is_some() || result.selection_mask.is_some()) {
        return Err("Whole-mask transform created a selection".into());
    }
    Ok(())
}
pub(crate) fn verify_half(source: &Document, result: &Document) -> Result<(), String> {
    verify(source, result, Operation::AffineHalf)
}
fn run_case(
    source: &Document,
    id: LayerId,
    op: Operation,
    pending: bool,
) -> Result<serde_json::Value, String> {
    let resident_before = crate::benchmark::resident_bytes();
    let target = Target::Mask(id);
    let (mut result, mut history, timing) = if matches!(op, Operation::Move) {
        crate::canvas::measure_move_target(source, OFFSET, target, pending)?
    } else {
        crate::affine_preview::measure_target(
            source,
            Settings {
                translation: if op.fractional() {
                    [17.5, -8.5]
                } else {
                    [17.0, -9.0]
                },
                sampling: if op.fractional() {
                    Sampling::Bilinear
                } else {
                    Sampling::Nearest
                },
                ..Default::default()
            },
            pending,
            target,
        )?
    };
    let resident_after = crate::benchmark::resident_bytes();
    verify(source, &result, op)?;
    let mask_bytes = result.layers[0]
        .mask
        .as_ref()
        .unwrap()
        .coverage
        .payload_bytes();
    let selection_bytes = result
        .selection_mask
        .as_ref()
        .map_or(0, |m| m.allocation().1);
    let after = result.clone();
    let start = Instant::now();
    let undone = history.undo(&mut result);
    let undo_ms = start.elapsed().as_secs_f64() * 1000.0;
    if !undone || !crate::document::same_snapshot(source, &result) || history.can_undo() {
        return Err("Mask probe undo differs or has extra actions".into());
    }
    let start = Instant::now();
    let redone = history.redo(&mut result);
    let redo_ms = start.elapsed().as_secs_f64() * 1000.0;
    if !redone || !crate::document::same_snapshot(&after, &result) {
        return Err("Mask probe redo differs".into());
    }
    Ok(
        serde_json::json!({"operation":op.name(),"phase":if pending{"pending-final-request"}else{"ready-preview"},"timing":timing,"undo_ms":undo_ms,"redo_ms":redo_ms,"resident_before_bytes":resident_before,"resident_after_bytes":resident_after,"result_mask_payload_bytes":mask_bytes,"result_selection_capacity_bytes":selection_bytes,"independently_verified_pixels":source.width*source.height,"artwork_shared_mask_flags_selection_and_one_action_undo_redo_verified":true}),
    )
}
pub(crate) fn run(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err("Benchmark destination already exists".into());
    }
    let mut cases = Vec::new();
    for [w, h] in [[1024, 1024], [2048, 2048], [4096, 1024]] {
        for profile in PROFILES {
            let (source, id) = fixture(w, h, profile)?;
            let mut measurements = Vec::new();
            for op in [
                Operation::Move,
                Operation::AffineInteger,
                Operation::AffineHalf,
            ] {
                for pending in [false, true] {
                    measurements.push(run_case(&source, id, op, pending)?);
                }
            }
            cases.push(serde_json::json!({"canvas":[w,h],"profile":profile,"source_mask_payload_bytes":source.layers[0].mask.as_ref().unwrap().coverage.payload_bytes(),"source_selection_capacity_bytes":source.selection_mask.as_ref().map_or(0,|m|m.allocation().1),"measurements":measurements}));
        }
    }
    let report = serde_json::json!({"version":1,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"scope":"Headless application acquisition, frozen preview publication and FIFO history settlement. Worker waits are outside owner-call timers. Excludes compositing/display/GPU, real frame scheduling, input drivers and tablet latency. Every source passes current document limits.","memory_scope":"Mask byte payload and selection capacity are storage counts, not total heap. Resident values are whole-process snapshots around each measurement, not a peak or isolated working set; they include retained allocator/runtime allocations.","cases":cases});
    write_report(path, &report)
}

fn soak_cycle(w: usize, h: usize) -> Result<serde_json::Value, String> {
    let mut measurements_verified = 0;
    let mut max_observed_resident = 0;
    for profile in ["whole-dense", "soft-dense", "whole-sparse"] {
        let (source, id) = fixture(w, h, profile)?;
        for op in [
            Operation::Move,
            Operation::AffineInteger,
            Operation::AffineHalf,
        ] {
            for pending in [false, true] {
                let measurement = run_case(&source, id, op, pending)?;
                max_observed_resident = max_observed_resident
                    .max(measurement["resident_after_bytes"].as_u64().unwrap_or(0));
                measurements_verified += 1;
            }
        }
        // Every result, history, worker and source is dropped before returning.
        // Only scalar JSON observations survive; no artwork is retained by this probe.
    }
    Ok(
        serde_json::json!({"post_drop_memory":crate::benchmark::process_memory(),"measurements_verified":measurements_verified,"max_observed_resident_bytes":max_observed_resident}),
    )
}

pub(crate) fn soak(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err("Benchmark destination already exists".into());
    }
    let before = crate::benchmark::process_memory();
    let start = Instant::now();
    let mut cycles = Vec::new();
    for cycle in 0..64 {
        cycles.push(soak_cycle(2048, 2048)?);
        eprintln!(
            "Mask lifecycle soak: {}/64 cycles verified ({:.1}s)",
            cycle + 1,
            start.elapsed().as_secs_f64()
        );
    }
    let report = serde_json::json!({
        "version":1,
        "profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "canvas":[2048,2048],
        "elapsed_seconds":start.elapsed().as_secs_f64(),
        "before_memory":before,
        "after_memory":crate::benchmark::process_memory(),
        "scope":"Same-process repeated headless mask application lifecycles, not a persistent native GUI session. Each cycle creates dense, fractional-selection and sparse fixtures, runs ready and pending-final Move/integer affine/bilinear transforms with independent pixel verification and Undo/Redo, then drops all source/result/history/worker ownership. No forced heap trimming or working-set eviction.",
        "memory_scope":"Windows working-set and private committed process snapshots, not live heap allocations or peaks. Scalar JSON observations accumulate; documents do not. A plateau can bound growth in this workload but cannot prove leak freedom or native-session stability.",
        "cycles":cycles
    });
    write_report(path, &report)
}

pub(crate) fn write_report(path: &Path, report: &impl serde::Serialize) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    use std::io::Write;
    {
        let mut writer = std::io::BufWriter::new(file.as_file_mut());
        serde_json::to_writer_pretty(&mut writer, report).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
    }
    file.persist_noclobber(path)
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn benchmark_oracles_validate_application_move_affine_and_single_action_history() {
        for profile in PROFILES {
            let (doc, id) = fixture(95, 89, profile).unwrap();
            for op in [
                Operation::Move,
                Operation::AffineInteger,
                Operation::AffineHalf,
            ] {
                for pending in [false, true] {
                    run_case(&doc, id, op, pending).unwrap();
                }
            }
        }
        assert_eq!(soak_cycle(95, 89).unwrap()["measurements_verified"], 18);
    }
}
