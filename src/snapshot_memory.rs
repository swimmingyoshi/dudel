//! Non-owning allocation descriptors. Callers must keep the described snapshot
//! alive; identities alone neither retain nor dereference artwork allocations.
use crate::{
    document::{Document, Layer},
    pixels::AllocationId,
};

#[derive(Default)]
pub(crate) struct Snapshot {
    metadata_bytes: usize,
    frame_allocation: (usize, usize),
    allocations: Vec<(AllocationId, usize)>,
}
#[derive(Default, serde::Serialize)]
pub(crate) struct Memory {
    pub snapshots: usize,
    pub metadata_bytes: usize,
    pub pixel_bytes_beyond_live: usize,
}
/// A worker owns the document; its descriptor retains only allocation identities.
/// Finished handles no longer retain source pixels, even before owner polling.
pub(crate) struct SourceWorker<T> {
    worker: std::thread::JoinHandle<T>,
    snapshot: Snapshot,
}
impl<T: Send + 'static> SourceWorker<T> {
    pub(crate) fn spawn(
        document: Document,
        run: impl FnOnce(Document) -> T + Send + 'static,
    ) -> Self {
        let snapshot = Snapshot::capture(&document);
        let worker = std::thread::spawn(move || run(document));
        Self { worker, snapshot }
    }
    pub(crate) fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }
    pub(crate) fn join(self) -> std::thread::Result<T> {
        self.worker.join()
    }
    /// Keep the same source descriptor while a detached continuation waits for
    /// completion. This conservatively counts the source until cleanup finishes.
    pub(crate) fn after<R: Send + 'static>(
        self,
        run: impl FnOnce(std::thread::Result<T>) -> R + Send + 'static,
    ) -> SourceWorker<R> {
        let Self { worker, snapshot } = self;
        SourceWorker {
            worker: std::thread::spawn(move || run(worker.join())),
            snapshot,
        }
    }
    pub(crate) fn register_source(&self, registry: &mut Registry) {
        if !self.is_finished() {
            registry.add_snapshot(&self.snapshot);
        }
    }
}
impl Snapshot {
    pub(crate) fn capture(doc: &Document) -> Self {
        let metadata_bytes = document_metadata(doc);
        let mut allocations = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if let Some(mask) = &doc.selection_mask {
            let (id, bytes) = mask.allocation();
            let id = AllocationId::Tile(id);
            seen.insert(id);
            allocations.push((id, bytes));
        }
        for layer in &doc.layers {
            if let Some(scene) = &layer.display_reference {
                scene.visit_allocations(|id, bytes| {
                    let id = AllocationId::Tile(id);
                    if seen.insert(id) {
                        allocations.push((id, bytes));
                    }
                });
            }
            if let Some(mask) = &layer.mask {
                mask.coverage.visit_allocations(|id, bytes| {
                    if seen.insert(id) {
                        allocations.push((id, bytes));
                    }
                });
            }
            layer.pixels.visit_allocations(|id, bytes| {
                if seen.insert(id) {
                    allocations.push((id, bytes));
                }
            });
        }
        Self {
            metadata_bytes,
            frame_allocation: (
                std::sync::Arc::as_ptr(&doc.canvas_frame) as usize,
                std::mem::size_of::<crate::canvas_size::Frame>(),
            ),
            allocations,
        }
    }
    pub(crate) fn descriptor_bytes(&self) -> usize {
        self.allocations.capacity() * std::mem::size_of::<(AllocationId, usize)>()
    }
}
fn document_metadata(doc: &Document) -> usize {
    std::mem::size_of::<Document>()
        + doc.layers.capacity() * std::mem::size_of::<Layer>()
        + doc
            .layers
            .iter()
            .map(|layer| layer.name.capacity())
            .sum::<usize>()
}
#[derive(Hash, PartialEq, Eq)]
enum Identity {
    Storage(usize),
    Bytes(usize),
    Metadata(usize),
}
#[derive(Default)]
pub(crate) struct Registry {
    seen: std::collections::HashSet<Identity>,
    live_bytes: usize,
    additional_bytes: usize,
    shared_metadata_bytes: usize,
}
impl Registry {
    pub(crate) fn new(live: &Document) -> Self {
        let mut registry = Self::default();
        registry.add_document(live);
        registry.live_bytes = registry.additional_bytes;
        registry.additional_bytes = 0;
        registry.shared_metadata_bytes = 0;
        registry
    }
    pub(crate) fn live_bytes(&self) -> usize {
        self.live_bytes
    }
    pub(crate) fn additional_bytes(&self) -> usize {
        self.additional_bytes
    }
    pub(crate) fn add_owned(&mut self, bytes: usize) {
        self.additional_bytes += bytes;
    }
    pub(crate) fn add_pixels(&mut self, id: AllocationId, bytes: usize) {
        let id = match id {
            AllocationId::Storage(id) => Identity::Storage(id),
            AllocationId::Tile(id) => Identity::Bytes(id),
        };
        if self.seen.insert(id) {
            self.additional_bytes += bytes;
        }
    }
    pub(crate) fn add_document(&mut self, doc: &Document) {
        self.add_owned(document_metadata(doc));
        self.add_shared_metadata(
            std::sync::Arc::as_ptr(&doc.canvas_frame) as usize,
            std::mem::size_of::<crate::canvas_size::Frame>(),
        );
        if let Some(mask) = &doc.selection_mask {
            let (id, bytes) = mask.allocation();
            self.add_buffer(id, bytes);
        }
        for layer in &doc.layers {
            if let Some(scene) = &layer.display_reference {
                scene.register_allocations(self);
            }
            if let Some(mask) = &layer.mask {
                mask.coverage
                    .visit_allocations(|id, bytes| self.add_pixels(id, bytes));
            }
            layer
                .pixels
                .visit_allocations(|id, bytes| self.add_pixels(id, bytes));
        }
    }
    pub(crate) fn add_snapshot(&mut self, snapshot: &Snapshot) {
        self.add_owned(snapshot.metadata_bytes);
        self.add_shared_metadata(snapshot.frame_allocation.0, snapshot.frame_allocation.1);
        for &(id, bytes) in &snapshot.allocations {
            self.add_pixels(id, bytes);
        }
    }
    pub(crate) fn add_buffer(&mut self, id: usize, bytes: usize) {
        if self.seen.insert(Identity::Bytes(id)) {
            self.additional_bytes += bytes;
        }
    }
    fn add_shared_metadata(&mut self, id: usize, bytes: usize) {
        if self.seen.insert(Identity::Metadata(id)) {
            self.additional_bytes += bytes;
            self.shared_metadata_bytes += bytes;
        }
    }
}
pub(crate) fn beyond_live<'a>(
    live: &Document,
    snapshots: impl IntoIterator<Item = &'a Snapshot>,
) -> Memory {
    let mut registry = Registry::new(live);
    let mut memory = Memory::default();
    for snapshot in snapshots {
        memory.snapshots += 1;
        memory.metadata_bytes += snapshot.metadata_bytes;
        registry.add_snapshot(snapshot);
    }
    memory.metadata_bytes += registry.shared_metadata_bytes;
    memory.pixel_bytes_beyond_live = registry.additional_bytes() - memory.metadata_bytes;
    memory
}

pub(crate) fn probe() -> Result<Vec<serde_json::Value>, String> {
    let mut cases = Vec::new();
    for size in [512usize, 2048] {
        for tiled in [false, true] {
            for layers in [1usize, 8, 32] {
                let mut live = Document::new(size, size);
                let pixels = [17, 83, 201, 128].repeat(size * size);
                if tiled {
                    live.layers[0]
                        .pixels
                        .writer()
                        .write_region(size, [0, 0, size, size], &pixels);
                } else {
                    live.layers[0].pixels = pixels.into();
                }
                for index in 1..layers {
                    live.add_layer();
                    live.layers[index]
                        .pixels
                        .writer()
                        .rgba_mut(index * size + index)
                        .copy_from_slice(&[201, 17, 83, 127]);
                }
                live.validate()?;
                let first = live.clone();
                let second = live.clone();
                let a = Snapshot::capture(&first);
                let b = Snapshot::capture(&second);
                let shared = beyond_live(&live, [&a, &b]);
                let mut history = crate::document::History::new();
                history.checkpoint(&live);
                let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
                let project_path = directory.path().join("source.dudel");
                let recovery_path = directory.path().join("source.gz");
                let (save_send, save_receive) = std::sync::mpsc::channel();
                let save_path = project_path.clone();
                let save = SourceWorker::spawn(live.clone(), move |document| {
                    save_receive.recv().map_err(|error| error.to_string())?;
                    crate::storage::save_project(&save_path, &document)
                });
                let (recovery_send, recovery_receive) = std::sync::mpsc::channel();
                let recovery_file = recovery_path.clone();
                let recover = SourceWorker::spawn(live.clone(), move |document| {
                    recovery_receive.recv().map_err(|error| error.to_string())?;
                    crate::storage::save_recovery(
                        &recovery_file,
                        &crate::storage::Recovery {
                            document,
                            original_path: None,
                        },
                    )
                });
                if shared.pixel_bytes_beyond_live != 0 {
                    return Err("Shared snapshots duplicated pixel accounting".into());
                }
                live.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(0)
                    .copy_from_slice(&[201, 17, 83, 255]);
                let one = beyond_live(&live, [&a]);
                let two = beyond_live(&live, [&a, &b]);
                history.refresh_memory(&live);
                let mut registry = Registry::new(&live);
                history.register_allocations(&mut registry);
                let history_bytes = registry.additional_bytes();
                registry.add_snapshot(&a);
                registry.add_snapshot(&b);
                if history_bytes != history.retained_bytes()
                    || registry.additional_bytes() != history_bytes + two.metadata_bytes
                {
                    return Err("Combined history and snapshot accounting duplicated pixels".into());
                }
                if one.pixel_bytes_beyond_live != two.pixel_bytes_beyond_live
                    || first.layers[0].pixels.rgba(0) != [17, 83, 201, 128]
                    || second.layers[0].pixels.rgba(0) != [17, 83, 201, 128]
                {
                    return Err("COW snapshots lost pixels or deduplication".into());
                }
                let combined_snapshots = registry.additional_bytes();
                save.register_source(&mut registry);
                recover.register_source(&mut registry);
                let all_sources = registry.additional_bytes();
                if all_sources != combined_snapshots + two.metadata_bytes {
                    return Err("Save/recovery sources duplicated shared history pixels".into());
                }
                let io_started = std::time::Instant::now();
                save_send.send(()).map_err(|error| error.to_string())?;
                recovery_send.send(()).map_err(|error| error.to_string())?;
                while !save.is_finished() || !recover.is_finished() {
                    if io_started.elapsed().as_secs() >= 30 {
                        return Err("Snapshot source writes exceeded probe deadline".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let io_poll_wall_ms = io_started.elapsed().as_secs_f64() * 1000.0;
                let mut completed = Registry::new(&live);
                save.register_source(&mut completed);
                recover.register_source(&mut completed);
                if completed.additional_bytes() != 0 {
                    return Err("Completed source workers retained snapshot accounting".into());
                }
                save.join().map_err(|_| "Save source worker panicked")??;
                recover
                    .join()
                    .map_err(|_| "Recovery source worker panicked")??;
                let saved = crate::storage::load_project(&project_path)?;
                let recovered = crate::storage::read_recovery(&recovery_path)?.document;
                for restored in [&saved, &recovered] {
                    if restored.layers.len() != first.layers.len()
                        || restored
                            .layers
                            .iter()
                            .zip(&first.layers)
                            .any(|(actual, expected)| actual.pixels != expected.pixels)
                    {
                        return Err(
                            "Save/recovery source pixels differ from frozen revision".into()
                        );
                    }
                }
                cases.push(serde_json::json!({"size":size, "layers":layers,
                    "storage":if tiled {"tiled"} else {"flat-compatibility"},
                    "shared":shared, "after_one_pixel_edit_one_snapshot":one,
                    "after_one_pixel_edit_two_snapshots":two,
                    "tracked_live_document_bytes":registry.live_bytes(),
                    "history_bytes_beyond_live":history_bytes,
                    "combined_history_and_snapshots_bytes_beyond_live":combined_snapshots,
                    "combined_history_display_save_recovery_bytes_beyond_live":all_sources,
                    "completed_source_worker_bytes":completed.additional_bytes(),
                    "source_write_poll_wall_ms":io_poll_wall_ms,
                    "descriptor_vector_bytes":a.descriptor_bytes() + b.descriptor_bytes()}));
            }
        }
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn await_source<T: Send + 'static>(worker: &SourceWorker<T>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !worker.is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn save_and_recovery_sources_share_history_pixels_and_release_before_polling() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("snapshot.dudel");
        let recovery = directory.path().join("snapshot.gz");
        let mut live = Document::new(513, 257);
        live.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[9, 11, 13, 0]);
        live.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[17, 83, 201, 128]);
        let frozen = live.clone();
        let descriptor = Snapshot::capture(&frozen);
        let mut history = crate::document::History::new();
        history.checkpoint(&live);
        let (save_send, save_receive) = std::sync::mpsc::channel();
        let project_path = project.clone();
        let save = SourceWorker::spawn(live.clone(), move |document| {
            save_receive.recv().unwrap();
            crate::storage::save_project(&project_path, &document)
        });
        let (recovery_send, recovery_receive) = std::sync::mpsc::channel();
        let recovery_path = recovery.clone();
        let original_path = project.clone();
        let recover = SourceWorker::spawn(live.clone(), move |document| {
            recovery_receive.recv().unwrap();
            crate::storage::save_recovery(
                &recovery_path,
                &crate::storage::Recovery {
                    document,
                    original_path: Some(original_path),
                },
            )
        });
        live.layers[0]
            .pixels
            .writer()
            .rgba_mut(1)
            .copy_from_slice(&[201, 17, 83, 255]);
        history.refresh_memory(&live);
        let mut registry = Registry::new(&live);
        history.register_allocations(&mut registry);
        let history_bytes = registry.additional_bytes();
        let metadata = beyond_live(&live, [&descriptor]).metadata_bytes;
        save.register_source(&mut registry);
        recover.register_source(&mut registry);
        assert_eq!(registry.additional_bytes(), history_bytes + metadata * 2);
        save_send.send(()).unwrap();
        recovery_send.send(()).unwrap();
        await_source(&save);
        await_source(&recover);
        let mut completed = Registry::new(&live);
        save.register_source(&mut completed);
        recover.register_source(&mut completed);
        assert_eq!(completed.additional_bytes(), 0);
        save.join().unwrap().unwrap();
        recover.join().unwrap().unwrap();
        let saved = crate::storage::load_project(&project).unwrap();
        let recovered = crate::storage::read_recovery(&recovery).unwrap();
        assert_eq!(recovered.original_path, Some(project));
        assert_eq!(saved.layers[0].pixels, frozen.layers[0].pixels);
        assert_eq!(recovered.document.layers[0].pixels, frozen.layers[0].pixels);
        assert_eq!(saved.layers[0].pixels.rgba(0), [9, 11, 13, 0]);
        assert_eq!(live.layers[0].pixels.rgba(1), [201, 17, 83, 255]);
        assert!(history.undo(&mut live));
        assert_eq!(live.layers[0].pixels, frozen.layers[0].pixels);
    }
    #[test]
    fn failed_and_panicked_source_workers_stop_accounting_without_changing_artwork() {
        let mut live = Document::new(32, 32);
        live.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 128]);
        let failed = SourceWorker::spawn(live.clone(), |_| Err::<(), _>("test failure"));
        let panicked = SourceWorker::spawn(live.clone(), |_| -> () { panic!("test source panic") });
        await_source(&failed);
        await_source(&panicked);
        let mut registry = Registry::new(&live);
        failed.register_source(&mut registry);
        panicked.register_source(&mut registry);
        assert_eq!(registry.additional_bytes(), 0);
        assert_eq!(failed.join().unwrap(), Err("test failure"));
        assert!(panicked.join().is_err());
        assert_eq!(live.layers[0].pixels.rgba(0), [17, 83, 201, 128]);
    }
    #[test]
    fn descriptors_deduplicate_shared_snapshots_and_distinguish_flat_and_tiled_cow() {
        for flat in [false, true] {
            let mut live = Document::new(513, 257);
            if flat {
                live.layers[0].pixels = [17, 83, 201, 128].repeat(513 * 257).into();
            } else {
                live.layers[0]
                    .pixels
                    .writer()
                    .rgba_mut(0)
                    .copy_from_slice(&[17, 83, 201, 128]);
            }
            let first = live.clone();
            let second = live.clone();
            let a = Snapshot::capture(&first);
            let b = Snapshot::capture(&second);
            assert_eq!(beyond_live(&live, [&a, &b]).pixel_bytes_beyond_live, 0);
            live.layers[0]
                .pixels
                .writer()
                .rgba_mut(0)
                .copy_from_slice(&[201, 17, 83, 255]);
            let one = beyond_live(&live, [&a]);
            let two = beyond_live(&live, [&a, &b]);
            assert!(
                one.pixel_bytes_beyond_live >= if flat { 513 * 257 * 4 } else { 256 * 256 * 4 }
            );
            assert!(
                one.pixel_bytes_beyond_live
                    < if flat {
                        513 * 257 * 4 + 4096
                    } else {
                        256 * 256 * 4 + 4096
                    }
            );
            assert_eq!(one.pixel_bytes_beyond_live, two.pixel_bytes_beyond_live);
            assert_eq!(two.metadata_bytes, one.metadata_bytes * 2);
            assert_eq!(two.snapshots, 2);
            assert_eq!(first.layers[0].pixels.rgba(0), [17, 83, 201, 128]);
            assert_eq!(second.layers[0].pixels.rgba(0), [17, 83, 201, 128]);
        }
    }
    #[test]
    fn descriptors_do_not_retain_pixels_or_force_copy_on_write() {
        let mut doc = Document::new(513, 257);
        doc.layers[0]
            .pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[17, 83, 201, 128]);
        let descriptor = Snapshot::capture(&doc);
        let mut pixels = doc.layers[0].pixels.clone();
        let mut before = Vec::new();
        pixels.visit_allocations(|id, _| before.push(id));
        drop(doc);
        pixels
            .writer()
            .rgba_mut(0)
            .copy_from_slice(&[201, 17, 83, 255]);
        let mut after = Vec::new();
        pixels.visit_allocations(|id, _| after.push(id));
        assert!(
            before == after,
            "Only the remaining pixel owner should exist"
        );
        assert!(descriptor.descriptor_bytes() > 0);
    }
}
