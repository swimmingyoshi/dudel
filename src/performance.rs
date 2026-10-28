use crate::{Dudel, egui};

/// Bounded recent-update samples; sorting happens only in the diagnostic window.
#[derive(Default)]
pub(crate) struct Latencies {
    samples: std::collections::VecDeque<f64>,
}
impl Latencies {
    pub(crate) fn record(&mut self, ms: f64) {
        if !ms.is_finite() || ms < 0.0 {
            return;
        }
        if self.samples.len() == 256 {
            self.samples.pop_front();
        }
        self.samples.push_back(ms);
    }
    fn summary(&self) -> Option<[f64; 3]> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort_unstable_by(f64::total_cmp);
        Some([50usize, 95, 99].map(|p| sorted[(sorted.len() * p).div_ceil(100) - 1]))
    }
}

#[derive(Default)]
pub struct Performance {
    pub open: bool,
    pub(crate) frames: Latencies,
    pub(crate) drawing: Latencies,
    pub(crate) phase_max_ms: [f64; 8],
    last_drawing: Option<std::time::Instant>,
    pub frame_ms: f64,
    pub dabs: usize,
    pub composite_ms: f64,
    pub upload_ms: f64,
    pub display_queued_bytes: usize,
    pub display_preparation_peak_bytes: usize,
    pub display_worker_ms: f64,
}
impl Performance {
    pub(crate) fn note_drawing(&mut self) {
        self.last_drawing = Some(std::time::Instant::now());
    }
    pub(crate) fn defer_optional(&self) -> bool {
        self.last_drawing
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(150))
    }
    pub fn begin_frame(&mut self) {
        self.dabs = 0;
        self.composite_ms = 0.0;
        self.upload_ms = 0.0;
        self.display_queued_bytes = 0;
        self.display_preparation_peak_bytes = 0;
        self.display_worker_ms = 0.0;
    }
}
impl Dudel {
    pub(crate) fn performance_window(&mut self, ctx: &egui::Context) {
        let mut open = self.performance.open;
        egui::Window::new("Performance").open(&mut open).vscroll(true).default_width(520.0).default_height(540.0).show(ctx, |ui| {
            let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
            let projects = self.open_projects_memory();
            ui.label(format!("All {} open projects tracked CPU: {:.2} MiB", projects.projects, mib(projects.tracked_cpu_bytes)));
            ui.label(format!("All projects canvas / reference and thumbnail RGBA: {:.2} / {:.2} MiB",
                mib(projects.canvas_rgba_bytes), mib(projects.auxiliary_rgba_bytes)));
            ui.small("Includes parked projects and shared reference pixels counted once. RGBA payload estimates exclude GPU allocation overhead, auxiliary mipmaps and preview-window textures; not process memory or VRAM commitment.");
            ui.separator();
            ui.label(format!("Retained history: {:.2} MiB ({} compressed regions)", self.history.retained_bytes() as f64 / (1024.0 * 1024.0), self.history.compressed_regions()));
            ui.label(format!("Prefetched undo: {:.2} / {} MiB", self.history.prefetched_bytes() as f64 / (1024.0*1024.0), crate::document::History::PREFETCH_MIB));
            ui.label(format!("Previous UI update: {:.2} ms", self.performance.frame_ms));
            for (name, samples) in [("UI updates", &self.performance.frames), ("Drawing queue pumps", &self.performance.drawing)] {
                if let Some([p50,p95,p99]) = samples.summary() {
                    ui.label(format!("{name}: p50 {p50:.2} / p95 {p95:.2} / p99 {p99:.2} ms (last {} samples)", samples.samples.len()));
                }
            }
            ui.small("Queue samples include controller work; these are CPU update times, not pen-to-screen latency.");
            for (name, maximum) in crate::render_queue::PHASES.iter().zip(self.performance.phase_max_ms) {
                if maximum > 0.0 { ui.label(format!("Longest {name} unit: {maximum:.3} ms")); }
            }
            if ui.small_button("Reset timing samples").clicked() {
                self.performance.frames = Default::default();
                self.performance.drawing = Default::default();
                self.performance.phase_max_ms = [0.0;8];
            }
            ui.label(format!("Painted dabs this update: {}", self.performance.dabs));
            ui.label(format!("CPU composite: {:.2} ms", self.performance.composite_ms));
            ui.label(format!("Completed display worker (elapsed): {:.2} ms", self.performance.display_worker_ms));
            let snapshot = self.texture.snapshot_memory(&self.doc);
            ui.label(format!("Frozen display sources: {}; {:.2} MiB pixels beyond live + {:.3} MiB metadata", snapshot.snapshots,
                snapshot.pixel_bytes_beyond_live as f64 / (1024.0 * 1024.0), snapshot.metadata_bytes as f64 / (1024.0 * 1024.0)));
            ui.small("Snapshot pixels may also be shared with undo/save; do not add these estimates as exclusive allocations.");
            let mut registry = crate::snapshot_memory::Registry::new(&self.doc);
            self.register_project_extras(&mut registry);
            if let Some(contour) = self.doc.selection_mask.as_ref().and_then(|mask|self.selection_contour.contour(mask)) {
                ui.label(format!("Selection outline grid: {} document px", contour.step));
            }
            if self.experimental_indexed_storage {
                let descriptors = self.indexed_session.as_ref().map_or(0, |s| s.descriptor_bytes())
                    + self.indexed_pending.as_ref().map_or(0, |p| p.descriptor_bytes());
                ui.label(format!("Indexed session descriptors (idle/last confirmed): {:.3} MiB", descriptors as f64 / (1024.0 * 1024.0)));
                ui.small("An indexed worker can hold old/pending proof inventories up to 4 MiB plus keys/index/codec scratch; weak headers and allocator overhead excluded. Source snapshots are tracked below.");
            }
            ui.label(format!("Active project tracked CPU: {:.2} MiB live + {:.2} MiB additional",
                registry.live_bytes() as f64 / (1024.0 * 1024.0), registry.additional_bytes() as f64 / (1024.0 * 1024.0)));
            ui.small("Includes stroke cursors/buffers and settlement patches. Shared stroke settings, fill working buffers, worker output/scratch, map controls, descriptor bookkeeping and allocator overhead are excluded.");
            ui.label(format!("Texture preparation / queue: {:.2} ms", self.performance.upload_ms));
            let memory = self.texture.memory();
            ui.label(format!("Display textures: {} / {:.2} MiB RGBA", memory.resident_tiles, mib(memory.texture_base_rgba_bytes)));
            ui.label(format!("With mip levels (estimate): {:.2} MiB", mib(memory.texture_with_mips_rgba_estimate_bytes)));
            ui.label(format!("Display metadata: {:.2} MiB", mib(memory.metadata_heap_bytes)));
            let cache = self.texture.cache_stats();
            ui.label(format!("Cache: {} hits / {} misses / {} bypasses / {} evictions", cache.hits, cache.misses, cache.bypasses, cache.evictions));
            ui.label(format!("Composite state cache: {:.2} / {} MiB", mib(memory.prefix_heap_bytes), crate::prefix_cache::BUDGET_MIB));
            ui.label(format!("Queued display pixels: {:.2} MiB; preparation peak: {:.2} MiB", mib(self.performance.display_queued_bytes), mib(self.performance.display_preparation_peak_bytes)));
            ui.small("Allocation estimates exclude driver/allocator overhead, weak allocation headers and other UI textures; not total process memory.");
            ui.small("CPU timings exclude GPU execution and pen-to-display latency. Idle updates are event-driven.");
        });
        self.performance.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latency_window_evicts_old_spikes_and_rejects_invalid_samples() {
        let mut samples = Latencies::default();
        samples.record(10_000.0);
        for i in 1..=256 {
            samples.record(i as f64);
        }
        samples.record(f64::NAN);
        samples.record(-1.0);
        assert_eq!(samples.samples.len(), 256);
        assert_eq!(samples.summary(), Some([128.0, 244.0, 254.0]));
    }
}
