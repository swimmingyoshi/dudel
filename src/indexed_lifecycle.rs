//! Generated supported-workload storage measurements. No user project input.
use super::{CompactStats, Journal, Result, StreamStats};
use crate::{
    document::{Document, MAX_RASTER_LAYERS},
    indexed_document,
    pixels::SIDE,
};
use std::{path::Path, time::Instant};

#[derive(Clone, Copy)]
enum Content {
    Sparse,
    Pattern,
    Noise,
}
fn fixture(size: [usize; 2], layers: usize, content: Content) -> Result<Document> {
    crate::selection_mask::pixels(size[0], size[1])?;
    if !(1..=MAX_RASTER_LAYERS).contains(&layers) {
        return Err("Unsupported lifecycle layer count".into());
    }
    let [width, height] = size;
    let mut doc = Document::new(width, height);
    for _ in 1..layers {
        doc.add_layer();
    }
    let mut random = 0x13579bdfu32;
    for (number, layer) in doc.layers.iter_mut().enumerate() {
        layer.name = format!("Generated lifecycle leaf {number}");
        if number % 3 == 0 {
            layer.csp_byte_opacity = true;
            layer.opacity = 156. / 256.;
        }
        if matches!(content, Content::Sparse) {
            layer
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[number as u8 + 1, 70, 130, 128]);
            layer
                .pixels
                .writer()
                .rgba_mut(width * height - 1)
                .copy_from_slice(&[33, number as u8 + 1, 99, 0]);
            continue;
        }
        for top in (0..height).step_by(SIDE) {
            for left in (0..width).step_by(SIDE) {
                let right = (left + SIDE).min(width);
                let bottom = (top + SIDE).min(height);
                let mut tile = Vec::with_capacity((right - left) * (bottom - top) * 4);
                for y in top..bottom {
                    for x in left..right {
                        let rgb = if matches!(content, Content::Pattern) {
                            [
                                (x + number * 17) as u8,
                                (y + number * 29) as u8,
                                ((x ^ y) + number * 7) as u8,
                            ]
                        } else {
                            random ^= random << 13;
                            random ^= random >> 17;
                            random ^= random << 5;
                            [random as u8, (random >> 8) as u8, (random >> 16) as u8]
                        };
                        tile.extend(rgb);
                        tile.push(if (x + y + number).is_multiple_of(17) {
                            0
                        } else if (x + y + number).is_multiple_of(5) {
                            128
                        } else {
                            255
                        });
                    }
                }
                layer
                    .pixels
                    .writer()
                    .write_region(width, [left, top, right, bottom], &tile);
            }
        }
    }
    let mut coverage = vec![0; width * height];
    coverage[0] = 127;
    *coverage.last_mut().unwrap() = 255;
    let mask = crate::selection_mask::Mask::from_coverage(width, height, coverage)?;
    doc.selection = mask.bounds();
    doc.selection_mask = Some(mask);
    doc.validate()?;
    Ok(doc)
}
fn milliseconds(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}
fn capacity(doc: &Document) -> usize {
    doc.layers.iter().map(|layer| layer.pixels.capacity()).sum()
}
fn verify(path: &Path, expected: &Document, indexed: bool) -> Result<f64> {
    let start = Instant::now();
    let actual = if indexed {
        let mut journal = Journal::open(path, false)?;
        let objects = journal.index.entries.keys().copied().collect::<Vec<_>>();
        indexed_document::decode(
            &objects,
            &mut |key| journal.read(key)?.ok_or("Missing lifecycle record".into()),
            &|| false,
        )?
        .0
    } else {
        crate::storage::load_project(path)?
    };
    let elapsed = milliseconds(start);
    if !indexed_document::equivalent(expected, &actual)? {
        return Err("Lifecycle document bytes/properties differ".into());
    }
    Ok(elapsed)
}
#[derive(serde::Serialize)]
struct Case {
    name: &'static str,
    canvas: [usize; 2],
    layers: usize,
    logical_rgba_bytes: usize,
    source_pixel_capacity_bytes: usize,
    source_registry_live_bytes: usize,
    source_pixel_capacity_after_bytes: usize,
    v4_save_ms: f64,
    v4_load_ms: f64,
    v4_bytes: u64,
    indexed_first_save_ms: f64,
    indexed_load_ms: f64,
    indexed_bytes: u64,
    thumbnail_load_ms: f64,
    unchanged_save_ms: f64,
    unchanged_append_bytes: u64,
    unchanged: StreamStats,
    edit_save_ms: f64,
    edit_load_ms: f64,
    edit_append_bytes: u64,
    edit: StreamStats,
    compaction_ms: f64,
    compaction: CompactStats,
    compacted_load_ms: f64,
    warm_reopened_save_ms: f64,
    warm_unchanged_worker_ms: f64,
    warm_unchanged: StreamStats,
    warm_edit_worker_ms: f64,
    warm_edit: StreamStats,
    warm_edit_load_ms: f64,
    exact_round_trips: bool,
}
fn measure(name: &'static str, mut doc: Document) -> Result<Case> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let v4_path = directory.path().join("baseline.dudel");
    let indexed_path = directory.path().join("candidate.index-study");
    let source_capacity = capacity(&doc);
    let registry_bytes = crate::snapshot_memory::Registry::new(&doc).live_bytes();
    eprintln!("Indexed lifecycle {name}: v4 save/open");
    let start = Instant::now();
    crate::storage::save_project(&v4_path, &doc)?;
    let v4_save = milliseconds(start);
    let v4_bytes = std::fs::metadata(&v4_path)
        .map_err(|e| e.to_string())?
        .len();
    let v4_load = verify(&v4_path, &doc, false)?;
    eprintln!("Indexed lifecycle {name}: indexed initial save/open");
    let start = Instant::now();
    let mut journal = Journal::create(&indexed_path)?;
    let (keys, _) = journal.replace_document(&doc, &Default::default(), &|| false, |_| Ok(()))?;
    let indexed_save = milliseconds(start);
    let indexed_bytes = journal.index.end;
    drop(journal);
    let indexed_load = verify(&indexed_path, &doc, true)?;
    let start = Instant::now();
    let mut journal = Journal::open(&indexed_path, false)?;
    let objects = journal.index.entries.keys().copied().collect::<Vec<_>>();
    let preview = crate::indexed_document::read_thumbnail(
        &objects,
        &mut |key| {
            journal
                .read(key)?
                .ok_or("Missing lifecycle thumbnail".into())
        },
        &|| false,
    )?
    .ok_or("Missing lifecycle thumbnail")?;
    let thumbnail_load = milliseconds(start);
    drop(journal);
    let expected = crate::indexed_thumbnail::decode(
        &crate::indexed_thumbnail::encode(&doc, &|| false)?,
        [doc.width, doc.height],
    )?;
    if preview.rgba != expected.rgba || preview.size != expected.size {
        return Err("Lifecycle thumbnail differs".into());
    }
    eprintln!("Indexed lifecycle {name}: unchanged save");
    let start = Instant::now();
    let mut journal = Journal::open(&indexed_path, true)?;
    let (keys, unchanged) = journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    let unchanged_ms = milliseconds(start);
    let unchanged_end = journal.index.end;
    if unchanged.changed != 0 || unchanged.removed != 0 || unchanged.raster_records_encoded != 0 {
        return Err("Unchanged lifecycle save rewrote records".into());
    }
    drop(journal);
    let [tw, th] = crate::indexed_thumbnail::size([doc.width, doc.height])?;
    let point = doc.height / (2 * th as usize) * doc.width + doc.width / (2 * tw as usize);
    let active = doc.active;
    let mut pixel = doc.layers[active].pixels.rgba(point);
    pixel[0] ^= 0x55;
    if pixel[3] == 0 {
        pixel[3] = 128;
    }
    doc.layers[active]
        .pixels
        .writer()
        .rgba_mut(point)
        .copy_from_slice(&pixel);
    doc.layers[active].name = "Generated lifecycle edited leaf".into();
    eprintln!("Indexed lifecycle {name}: one-tile edit save/open");
    let start = Instant::now();
    let mut journal = Journal::open(&indexed_path, true)?;
    let (_, edit) = journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    if edit.raster_records_encoded != 1 {
        return Err("One-tile lifecycle edit recompressed additional rasters".into());
    }
    let edit_ms = milliseconds(start);
    let edit_end = journal.index.end;
    drop(journal);
    let edit_load = verify(&indexed_path, &doc, true)?;
    eprintln!("Indexed lifecycle {name}: verify/compact/open");
    let start = Instant::now();
    let mut journal = Journal::open(&indexed_path, true)?;
    let compact = journal.compact_document(&indexed_path, &|| false, |_| Ok(()))?;
    let compact_ms = milliseconds(start);
    drop(journal);
    let compacted_load = verify(&indexed_path, &doc, true)?;
    eprintln!("Indexed lifecycle {name}: locked-session background saves");
    let start = Instant::now();
    let mut journal = Journal::open(&indexed_path, true)?;
    let (keys, warmed) = journal.replace_document(&doc, &keys, &|| false, |_| Ok(()))?;
    let warm_reopened_save_ms = milliseconds(start);
    if warmed.raster_records_reused_without_read != 0 {
        return Err("Reopened lifecycle save trusted stale proofs".into());
    }
    let start = Instant::now();
    let worker = crate::snapshot_memory::SourceWorker::spawn(doc.clone(), move |snapshot| {
        let (keys, stats) = journal.replace_document(&snapshot, &keys, &|| false, |_| Ok(()))?;
        Ok::<_, String>((journal, keys, stats))
    });
    let (mut journal, keys, warm_unchanged) = worker
        .join()
        .map_err(|_| "Warm unchanged worker panicked")??;
    let warm_unchanged_worker_ms = milliseconds(start);
    if warm_unchanged.raster_records_read_for_comparison != 0
        || warm_unchanged.raster_records_encoded != 0
        || warm_unchanged.raster_records_reused_without_read
            != warmed.raster_records_read_for_comparison
    {
        return Err("Warm lifecycle save copied/read unchanged rasters".into());
    }
    let active = doc.active;
    let mut pixel = doc.layers[active].pixels.rgba(0);
    pixel[0] ^= 1;
    doc.layers[active]
        .pixels
        .writer()
        .rgba_mut(0)
        .copy_from_slice(&pixel);
    let start = Instant::now();
    let worker = crate::snapshot_memory::SourceWorker::spawn(doc.clone(), move |snapshot| {
        let (_, stats) = journal.replace_document(&snapshot, &keys, &|| false, |_| Ok(()))?;
        Ok::<_, String>((journal, stats))
    });
    let (journal, warm_edit) = worker.join().map_err(|_| "Warm edit worker panicked")??;
    let warm_edit_worker_ms = milliseconds(start);
    if warm_edit.raster_records_read_for_comparison != 1
        || warm_edit.raster_records_encoded != 1
        || warm_edit.raster_records_reused_without_read + 1
            != warm_unchanged.raster_records_reused_without_read
    {
        return Err("Warm lifecycle edit read/encoded more than the changed raster".into());
    }
    drop(journal);
    let warm_edit_load_ms = verify(&indexed_path, &doc, true)?;
    let after_capacity = capacity(&doc);
    if after_capacity != source_capacity {
        return Err("Lifecycle storage populated or changed source pixel capacity".into());
    }
    eprintln!("Indexed lifecycle {name}: exact checks passed");
    Ok(Case {
        name,
        canvas: [doc.width, doc.height],
        layers: doc.layers.len(),
        logical_rgba_bytes: doc.width * doc.height * 4 * doc.raster_count(),
        source_pixel_capacity_bytes: source_capacity,
        source_registry_live_bytes: registry_bytes,
        source_pixel_capacity_after_bytes: after_capacity,
        v4_save_ms: v4_save,
        v4_load_ms: v4_load,
        v4_bytes,
        indexed_first_save_ms: indexed_save,
        indexed_load_ms: indexed_load,
        indexed_bytes,
        thumbnail_load_ms: thumbnail_load,
        unchanged_save_ms: unchanged_ms,
        unchanged_append_bytes: unchanged_end - indexed_bytes,
        unchanged,
        edit_save_ms: edit_ms,
        edit_load_ms: edit_load,
        edit_append_bytes: edit_end - unchanged_end,
        edit,
        compaction_ms: compact_ms,
        compaction: compact,
        compacted_load_ms: compacted_load,
        warm_reopened_save_ms,
        warm_unchanged_worker_ms,
        warm_unchanged,
        warm_edit_worker_ms,
        warm_edit,
        warm_edit_load_ms,
        exact_round_trips: true,
    })
}
pub(super) fn run(output: Option<&Path>) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Report {
        scope: &'static str,
        cases: Vec<Case>,
    }
    let mut cases = Vec::new();
    for (name, size, layers, content) in [
        (
            "supported_sparse_4mp_20_layers",
            [2048, 2048],
            20,
            Content::Sparse,
        ),
        (
            "supported_dense_pattern_4mp_20_layers",
            [2048, 2048],
            20,
            Content::Pattern,
        ),
        (
            "maximum_supported_dense_noise_4mp_32_layers",
            [2048, 2048],
            32,
            Content::Noise,
        ),
    ] {
        cases.push(measure(name, fixture(size, layers, content)?)?);
    }
    crate::clip_container::write_report(
        &Report {
            scope: "Single-run generated supported storage lifecycle versus v4, with OS caches affected by prior writes/reads. No cache flush, cold-disk claim, UI/tablet latency, process RSS/peak memory or power-loss certification. Source/registry capacities are component accounting, sink allocations exclude encoder/index/source memory. Append bytes exclude root overwrites. All source files are temporary generated data. Reopened saves validate old rasters; separate warm measurements move the same locked journal through background workers. Worker times include spawn/join and snapshots; they are not reopen latency.",
            cases,
        },
        output,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_edge_lifecycle_covers_both_formats_and_preserves_source_capacity() {
        let report = measure("small_edge", fixture([17, 19], 2, Content::Noise).unwrap()).unwrap();
        assert!(report.exact_round_trips);
        assert_eq!(
            report.source_pixel_capacity_bytes,
            report.source_pixel_capacity_after_bytes
        );
        assert_eq!(report.unchanged.changed, 0);
        assert!(report.edit.changed > 0);
        assert!(report.compaction.after_bytes < report.compaction.before_bytes);
        assert!(fixture([4096, 4097], 1, Content::Sparse).is_err());
        assert!(fixture([1, 1], MAX_RASTER_LAYERS + 1, Content::Sparse).is_err());
    }
}
