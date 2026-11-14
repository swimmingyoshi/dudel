//! Command-line modes that do their work and exit without opening a window:
//! installing, checks for scripts, file inspection and the benchmarks.

use crate::{
    affine, affine_benchmark, benchmark, canvas, canvas_textures, clip_blocks, clip_container,
    clip_metadata, clipboard, extensions, fill, import_report, indexed_commit, layer_mask,
    mask_transform_benchmark, prefix_cache, psd_export, recovery_probe, render_queue, replay,
    selection_contour, selection_mask, settings_store, stroke_profile, tip_import, transform,
    update,
};
use std::path::{Path, PathBuf};

/// Benchmarks that take one optional report path: the flag, the report's
/// default name, the label for an error, and the benchmark.
type Report = (
    &'static str,
    &'static str,
    &'static str,
    fn(&Path) -> Result<(), String>,
);
const REPORTS: &[Report] = &[
    (
        "--benchmark-prefix-plan",
        "prefix-plan.json",
        "Prefix plan benchmark",
        prefix_cache::benchmark_plan,
    ),
    (
        "--benchmark-display-memory",
        "display-memory.json",
        "Display memory benchmark",
        canvas_textures::benchmark,
    ),
    (
        "--benchmark-clipboard",
        "clipboard.json",
        "Clipboard benchmark",
        clipboard::benchmark,
    ),
    (
        "--benchmark-move-preview",
        "move-preview.json",
        "Move preview benchmark",
        canvas::benchmark_moves,
    ),
    (
        "--benchmark-mask-transform",
        "mask-transform.json",
        "Mask transform benchmark",
        transform::benchmark_masks,
    ),
    (
        "--benchmark-selection-contour",
        "selection-contour.json",
        "Selection contour benchmark",
        selection_contour::benchmark,
    ),
    (
        "--benchmark-affine-dense",
        "affine-dense.json",
        "Dense affine benchmark",
        affine_benchmark::run,
    ),
    (
        "--benchmark-affine",
        "affine.json",
        "Affine benchmark",
        affine::benchmark,
    ),
    (
        "--benchmark-selection",
        "selection.json",
        "Selection benchmark",
        selection_mask::benchmark,
    ),
    (
        "--benchmark-fill",
        "fill.json",
        "Fill benchmark",
        fill::benchmark,
    ),
    (
        "--benchmark-render-queue",
        "render-queue.json",
        "Render queue",
        render_queue::benchmark,
    ),
];

/// Runs `mode` when it is one of these commands; `false` when the app itself
/// should start. A command that fails says why and exits with status 1.
pub(crate) fn run(mode: &str, args: &mut impl Iterator<Item = String>) -> bool {
    if let Some((flag, default, label, benchmark)) = REPORTS.iter().find(|row| row.0 == mode) {
        let path = args
            .next()
            .map_or_else(|| PathBuf::from(default), PathBuf::from);
        if args.next().is_some() {
            eprintln!("Usage: {flag} [FILE.json]");
            std::process::exit(1);
        }
        if let Err(error) = benchmark(&path) {
            eprintln!("{label}: {error}");
            std::process::exit(1);
        }
        return true;
    }
    #[cfg(windows)]
    if mode == "--uninstall" {
        // Windows runs this from Settings > Apps for the installed copy.
        let quiet = args.next().as_deref() == Some("--quiet");
        if let Err(error) = crate::install::uninstall_command(quiet) {
            eprintln!("Remove Dudel: {error}");
            if !quiet {
                rfd::MessageDialog::new()
                    .set_title("Remove Dudel")
                    .set_description(&error)
                    .set_level(rfd::MessageLevel::Error)
                    .show();
            }
            std::process::exit(1);
        }
        return true;
    }
    #[cfg(windows)]
    if mode == "--install" {
        // What Install on this PC does, for scripts: every extra is on
        // unless its --no- switch is given.
        let switches: Vec<String> = args.collect();
        let off = |switch: &str| switches.iter().any(|given| given == switch);
        let options = crate::install::Options {
            desktop: !off("--no-desktop"),
            open_projects: !off("--no-file-type"),
            thumbnails: !off("--no-previews"),
        };
        match crate::install::install(options) {
            Ok(program) => println!("Installed {}", program.display()),
            Err(error) => {
                eprintln!("Install: {error}");
                std::process::exit(1);
            }
        }
        return true;
    }
    if mode == "--check-update" {
        // What Help > Check for updates would find, for scripts and support.
        match update::check() {
            Ok(Some(release)) => {
                println!("Dudel {} is available: {}", release.version, release.page)
            }
            Ok(None) => println!("Dudel {} is the newest version", env!("CARGO_PKG_VERSION")),
            Err(error) => {
                eprintln!("Update check: {error}");
                std::process::exit(1);
            }
        }
        return true;
    }
    if mode == "--check-plugin" {
        let path = args.next().map(PathBuf::from);
        let result = match path {
            Some(path) if args.next().is_none() => extensions::check_package(&path),
            _ => Err("Usage: --check-plugin PACKAGE.dudelplugin".into()),
        };
        if let Err(error) = result {
            eprintln!("Plugin validation: {error}");
            std::process::exit(1);
        }
        println!("Plugin API, metadata and program validation passed");
        return true;
    }
    if mode == "--brush-texture" {
        // For plugin packaging: a PNG as the brush tip or grain JSON a
        // manifest embeds, converted exactly as Brush settings import it.
        let usage = "Usage: --brush-texture tip|grain IMAGE.png";
        let (Some(kind), Some(path), None) = (args.next(), args.next(), args.next()) else {
            eprintln!("{usage}");
            std::process::exit(1);
        };
        let path = PathBuf::from(path);
        let texture = match kind.as_str() {
            "tip" => tip_import::from_png(&path),
            "grain" => tip_import::grain_from_png(&path),
            _ => Err(usage.into()),
        };
        match texture.and_then(|t| serde_json::to_string(&t).map_err(|e| e.to_string())) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("Brush texture: {error}");
                std::process::exit(1);
            }
        }
        return true;
    }
    if mode == "--probe-artwork-recovery" {
        let root = args.next().map(PathBuf::from);
        let phase = args.next();
        if root.is_none() || phase.is_none() || args.next().is_some() {
            eprintln!(
                "Usage: --probe-artwork-recovery NEW_ISOLATED_FOLDER write-v4|write-v5|restore"
            );
            std::process::exit(1);
        }
        if let Err(error) = recovery_probe::run(&root.unwrap(), &phase.unwrap()) {
            eprintln!("Artwork recovery probe failed: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if mode == "--probe-settings-store" {
        let root = args.next().map(PathBuf::from);
        let phase = args.next();
        if root.is_none() || phase.is_none() || args.next().is_some() {
            eprintln!(
                "Usage: --probe-settings-store ISOLATED_FOLDER first|reopen|corrupt|repair|failure|retry"
            );
            std::process::exit(1);
        }
        if let Err(error) = settings_store::probe(&root.unwrap(), &phase.unwrap()) {
            eprintln!("Settings probe failed: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if matches!(
        mode,
        "--study-indexed-commits"
            | "--study-indexed-tiles"
            | "--study-indexed-documents"
            | "--study-indexed-stream"
            | "--study-indexed-thumbnails"
            | "--study-indexed-compaction"
            | "--benchmark-indexed-lifecycle"
    ) {
        let report = args.next().map(PathBuf::from);
        if args.next().is_some() {
            eprintln!("Usage: {} [NEW_REPORT.json]", mode);
            std::process::exit(1);
        }
        let result = if mode == "--benchmark-indexed-lifecycle" {
            indexed_commit::benchmark_lifecycle(report.as_deref())
        } else if mode == "--study-indexed-compaction" {
            indexed_commit::study_compaction(report.as_deref())
        } else if mode == "--study-indexed-thumbnails" {
            indexed_commit::study_thumbnails(report.as_deref())
        } else if mode == "--study-indexed-stream" {
            indexed_commit::study_stream(report.as_deref())
        } else if mode == "--study-indexed-documents" {
            indexed_commit::study_documents(report.as_deref())
        } else if mode == "--study-indexed-tiles" {
            indexed_commit::study_tiles(report.as_deref())
        } else {
            indexed_commit::study(report.as_deref())
        };
        if let Err(error) = result {
            eprintln!("Indexed storage study: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if mode == "--compare-clip-export" {
        let input = args.next().map(PathBuf::from);
        let reference = args.next().map(PathBuf::from);
        let report = args.next().map(PathBuf::from);
        let (Some(input), Some(reference)) = (input, reference) else {
            eprintln!("Usage: --compare-clip-export FILE.clip REFERENCE.png [NEW_REPORT.json]");
            std::process::exit(1);
        };
        if args.next().is_some() {
            eprintln!("Usage: --compare-clip-export FILE.clip REFERENCE.png [NEW_REPORT.json]");
            std::process::exit(1);
        }
        if let Err(error) = clip_metadata::compare_export(&input, &reference, report.as_deref()) {
            eprintln!("Clip export comparison: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if matches!(mode, "--roundtrip-psd" | "--roundtrip-psd-effect-layers") {
        let usage = "Usage: --roundtrip-psd[-effect-layers] INPUT OUTPUT.psd [NEW_REPORT.json]";
        let effects = if mode == "--roundtrip-psd" {
            psd_export::Effects::Editable
        } else {
            psd_export::Effects::AsLayers
        };
        let (Some(input), Some(psd)) = (args.next(), args.next()) else {
            eprintln!("{usage}");
            std::process::exit(1);
        };
        let report = args.next().map(PathBuf::from);
        if args.next().is_some() {
            eprintln!("{usage}");
            std::process::exit(1);
        }
        let (input, psd) = (PathBuf::from(input), PathBuf::from(psd));
        if let Err(error) = import_report::roundtrip_psd(&input, &psd, report.as_deref(), effects) {
            eprintln!("PSD round trip: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if matches!(
        mode,
        "--inspect-clip"
            | "--inspect-clip-metadata"
            | "--inspect-clip-blocks"
            | "--verify-clip-raster"
            | "--verify-clip-document"
            | "--verify-import"
            | "--verify-clip-preview"
            | "--compare-clip-rendering"
            | "--compare-clip-folder-caches"
    ) {
        let Some(input) = args.next().map(PathBuf::from) else {
            eprintln!("Usage: {} FILE.clip [NEW_REPORT.json]", mode);
            std::process::exit(1);
        };
        let report = args.next().map(PathBuf::from);
        // --verify-import also takes a reference image to compare against.
        let reference = if mode == "--verify-import" {
            args.next().map(PathBuf::from)
        } else {
            None
        };
        if args.next().is_some() {
            eprintln!(
                "Usage: {} FILE.clip [NEW_REPORT.json]{}",
                mode,
                if mode == "--verify-import" {
                    " [REFERENCE.png]"
                } else {
                    ""
                }
            );
            std::process::exit(1);
        }
        let result = if matches!(mode, "--verify-import" | "--verify-clip-preview") {
            import_report::verify(
                &input,
                report.as_deref(),
                reference.as_deref(),
                mode == "--verify-clip-preview",
            )
        } else if mode == "--inspect-clip-metadata" {
            clip_metadata::inspect(&input, report.as_deref())
        } else if mode == "--compare-clip-folder-caches" {
            clip_metadata::compare_folder_caches(&input, report.as_deref())
        } else if mode == "--compare-clip-rendering" {
            clip_metadata::compare_rendering(&input, report.as_deref())
        } else if mode == "--verify-clip-document" {
            clip_metadata::verify_document(&input, report.as_deref())
        } else if mode == "--verify-clip-raster" {
            clip_metadata::verify_rasters(&input, report.as_deref())
        } else if mode == "--inspect-clip-blocks" {
            clip_blocks::inspect(&input, report.as_deref())
        } else {
            clip_container::inspect(&input, report.as_deref())
        };
        if let Err(error) = result {
            eprintln!("Clip inspection: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if matches!(
        mode,
        "--benchmark-live-display" | "--benchmark-moving-display"
    ) {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("live-display.json"));
        if args.next().is_some() {
            eprintln!("Usage: --benchmark-live-display / --benchmark-moving-display [FILE.json]");
            std::process::exit(1);
        }
        let result = if mode == "--benchmark-moving-display" {
            canvas_textures::benchmark_live_motion(&path)
        } else {
            canvas_textures::benchmark_live(&path)
        };
        if let Err(error) = result {
            eprintln!("Live display benchmark: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if mode == "--profile-stroke" {
        let Some(report) = args.next().map(PathBuf::from) else {
            eprintln!("Usage: --profile-stroke REPORT.json [CASE NAME FILTER] [IMAGE FOLDER]");
            std::process::exit(1);
        };
        let filter = args.next();
        let images = args.next().map(PathBuf::from);
        if let Err(error) = stroke_profile::run(&report, filter.as_deref(), images.as_deref()) {
            eprintln!("Stroke profile: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if mode == "--replay-input" {
        let result = (|| {
            let path = args.next().map(PathBuf::from).ok_or("Usage: --replay-input FILE [--batch-size N] [--work-budget N] [--output FILE.dudel] [--report FILE.json]")?;
            let mut batch_size = 64;
            let mut work_budget = 0;
            let mut output = None;
            let mut report = None;
            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("Missing value for {flag}"))?;
                match flag.as_str() {
                    "--batch-size" => {
                        batch_size = value
                            .parse::<usize>()
                            .map_err(|_| "Invalid replay batch size")?
                    }
                    "--output" => output = Some(PathBuf::from(value)),
                    "--work-budget" => {
                        work_budget = value
                            .parse::<usize>()
                            .map_err(|_| "Invalid rendering work budget")?
                    }
                    "--report" => report = Some(PathBuf::from(value)),
                    _ => return Err(format!("Unknown replay flag: {flag}")),
                }
            }
            replay::run(
                &path,
                batch_size,
                work_budget,
                output.as_deref(),
                report.as_deref(),
            )
        })();
        if let Err(error) = result {
            eprintln!("Replay: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if mode == "--benchmark" {
        let path = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("benchmark.json"));
        let flags: Vec<String> = args.collect();
        let incremental = flags.iter().any(|flag| flag == "--incremental");
        let buffered = !flags.iter().any(|flag| flag == "--unbuffered");
        if flags
            .iter()
            .any(|flag| flag != "--incremental" && flag != "--unbuffered")
        {
            eprintln!("Unknown benchmark flag; supported: --incremental, --unbuffered");
            std::process::exit(1);
        }
        if let Err(error) = benchmark::run(&path, incremental, buffered) {
            eprintln!("Benchmark: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if mode == "--benchmark-layer-masks" {
        let report = args.next().map(PathBuf::from);
        if report.is_none() || args.next().is_some() {
            eprintln!("Usage: --benchmark-layer-masks NEW_REPORT.json");
            std::process::exit(1);
        }
        if let Err(error) = layer_mask::benchmark(&report.unwrap()) {
            eprintln!("Layer mask engine: {error}");
            std::process::exit(1);
        }
        return true;
    }
    if matches!(
        mode,
        "--benchmark-mask-transforms" | "--soak-mask-transforms"
    ) {
        let path = args.next().map(PathBuf::from);
        if path.is_none() || args.next().is_some() {
            eprintln!("Usage: {} NEW_REPORT.json", mode);
            std::process::exit(1);
        }
        let path = path.unwrap();
        let result = if mode == "--soak-mask-transforms" {
            mask_transform_benchmark::soak(&path)
        } else {
            mask_transform_benchmark::run(&path)
        };
        if let Err(error) = result {
            eprintln!("Mask transforms: {error}");
            std::process::exit(1);
        }
        return true;
    }
    false
}
