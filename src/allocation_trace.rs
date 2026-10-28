//! Bounded Windows-only large-request trace. Hooks never allocate or wait for
//! another tracer writer. Symbolization/serialization happen after recording stops.
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering::*},
};
use windows_sys::Win32::System::{Diagnostics::Debug::*, LibraryLoader::GetModuleHandleW};

const MINIMUM: usize = 64 * 1024;
const CAPACITY: usize = 524288;
const DEPTH: usize = 32;
const ALLOC: usize = 1;
const FREE: usize = 2;
const REALLOC_BEGIN: usize = 3;
const REALLOC_END: usize = 4;

struct Event {
    kind: AtomicUsize,
    pointer: AtomicUsize,
    size: AtomicUsize,
    new_size: AtomicUsize,
    transaction: AtomicUsize,
    depth: AtomicUsize,
    frames: [AtomicUsize; DEPTH],
}
impl Event {
    const fn new() -> Self {
        Self {
            kind: AtomicUsize::new(0),
            pointer: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
            new_size: AtomicUsize::new(0),
            transaction: AtomicUsize::new(0),
            depth: AtomicUsize::new(0),
            frames: [const { AtomicUsize::new(0) }; DEPTH],
        }
    }
}
struct Recorder<const N: usize> {
    started: AtomicBool,
    enabled: AtomicBool,
    writers: AtomicUsize,
    next: AtomicUsize,
    events: [Event; N],
}
impl<const N: usize> Recorder<N> {
    const fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            enabled: AtomicBool::new(false),
            writers: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
            events: [const { Event::new() }; N],
        }
    }
    fn start(&self) {
        if self
            .started
            .compare_exchange(false, true, AcqRel, Acquire)
            .is_ok()
        {
            self.enabled.store(true, Release);
        }
    }
    fn enter(&self) -> Guard<'_, N> {
        let mut active = self.enabled.load(SeqCst);
        if active {
            self.writers.fetch_add(1, SeqCst);
            active = self.enabled.load(SeqCst);
            if !active {
                self.writers.fetch_sub(1, SeqCst);
            }
        }
        Guard {
            recorder: self,
            active,
        }
    }
    fn stop(&self) {
        self.enabled.store(false, SeqCst);
        while self.writers.load(SeqCst) != 0 {
            std::thread::yield_now();
        }
    }
    fn push(
        &self,
        kind: usize,
        pointer: usize,
        size: usize,
        new_size: usize,
        transaction: usize,
        stack: bool,
    ) -> usize {
        let index = self.next.fetch_add(1, Relaxed);
        if let Some(event) = self.events.get(index) {
            event.kind.store(kind, Relaxed);
            event.pointer.store(pointer, Relaxed);
            event.size.store(size, Relaxed);
            event.new_size.store(new_size, Relaxed);
            event.transaction.store(transaction, Relaxed);
            if stack {
                let mut frames = [std::ptr::null_mut::<c_void>(); DEPTH];
                // SAFETY: fixed writable pointer storage with exactly DEPTH slots.
                let depth = unsafe {
                    RtlCaptureStackBackTrace(
                        0,
                        DEPTH as u32,
                        frames.as_mut_ptr(),
                        std::ptr::null_mut(),
                    )
                } as usize;
                let depth = depth.min(DEPTH);
                for (slot, frame) in event.frames.iter().zip(frames).take(depth) {
                    slot.store(frame as usize, Relaxed);
                }
                event.depth.store(depth, Relaxed);
            }
        }
        index + 1
    }
    fn snapshot(&self) -> Vec<TraceEvent> {
        assert!(!self.enabled.load(SeqCst) && self.writers.load(SeqCst) == 0);
        self.events
            .iter()
            .take(self.next.load(Relaxed).min(N))
            .enumerate()
            .map(|(index, event)| TraceEvent {
                sequence: index + 1,
                kind: event.kind.load(Relaxed),
                pointer: event.pointer.load(Relaxed),
                size: event.size.load(Relaxed),
                new_size: event.new_size.load(Relaxed),
                transaction: event.transaction.load(Relaxed),
                frames: event
                    .frames
                    .iter()
                    .take(event.depth.load(Relaxed))
                    .map(|frame| frame.load(Relaxed))
                    .collect(),
            })
            .collect()
    }
}
pub(crate) struct Guard<'a, const N: usize> {
    recorder: &'a Recorder<N>,
    active: bool,
}
impl<const N: usize> Drop for Guard<'_, N> {
    fn drop(&mut self) {
        if self.active {
            self.recorder.writers.fetch_sub(1, SeqCst);
        }
    }
}
impl<const N: usize> Guard<'_, N> {
    pub(crate) fn allocated(&self, pointer: usize, size: usize) {
        if self.active && size >= MINIMUM {
            self.recorder.push(ALLOC, pointer, size, 0, 0, true);
        }
    }
    pub(crate) fn freeing(&self, pointer: usize, size: usize) {
        if self.active && size >= MINIMUM {
            self.recorder.push(FREE, pointer, size, 0, 0, false);
        }
    }
    pub(crate) fn reallocating(&self, pointer: usize, old: usize, new: usize) -> Option<usize> {
        (self.active && (old >= MINIMUM || new >= MINIMUM)).then(|| {
            self.recorder
                .push(REALLOC_BEGIN, pointer, old, new, 0, new >= MINIMUM)
        })
    }
    pub(crate) fn reallocated(&self, transaction: Option<usize>, pointer: usize) {
        if let Some(transaction) = transaction {
            self.recorder
                .push(REALLOC_END, pointer, 0, 0, transaction, false);
        }
    }
}
static RECORDER: Recorder<CAPACITY> = Recorder::new();
static SIZE_LIVE: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
pub(crate) fn start() {
    RECORDER.start();
}
pub(crate) fn enter() -> Guard<'static, CAPACITY> {
    RECORDER.enter()
}
pub(crate) fn account_resize(old: usize, new: usize) {
    let bucket = |size: usize| ((usize::BITS - 1 - size.leading_zeros()) as usize).min(31);
    if old != 0 {
        SIZE_LIVE[bucket(old)].fetch_sub(old, Relaxed);
    }
    if new != 0 {
        SIZE_LIVE[bucket(new)].fetch_add(new, Relaxed);
    }
}
pub(crate) fn live_sizes() -> [usize; 32] {
    std::array::from_fn(|i| SIZE_LIVE[i].load(Relaxed))
}
#[derive(serde::Serialize)]
pub(crate) struct Checkpoint {
    event_position: usize,
    writers_in_flight: usize,
}
pub(crate) fn checkpoint() -> Checkpoint {
    Checkpoint {
        event_position: RECORDER.next.load(SeqCst),
        writers_in_flight: RECORDER.writers.load(SeqCst),
    }
}

#[derive(serde::Serialize)]
struct TraceEvent {
    sequence: usize,
    kind: usize,
    pointer: usize,
    size: usize,
    new_size: usize,
    transaction: usize,
    frames: Vec<usize>,
}
#[derive(serde::Serialize)]
struct Symbol {
    name: String,
    module_base: u64,
    displacement: u64,
}
#[repr(C)]
struct SymbolBuffer {
    info: SYMBOL_INFOW,
    extra: [u16; 1024],
}

fn symbols(events: &[TraceEvent], directory: &Path) -> BTreeMap<usize, Symbol> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
    }
    let addresses: BTreeSet<_> = events
        .iter()
        .flat_map(|e| e.frames.iter().copied())
        .collect();
    let path: Vec<u16> = directory
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: DbgHelp is used only here, on the stopped diagnostic's main thread.
    // Search path and symbol buffer storage remain valid throughout each call.
    unsafe {
        let process = GetCurrentProcess();
        SymSetOptions(
            SYMOPT_UNDNAME
                | SYMOPT_DEFERRED_LOADS
                | SYMOPT_EXACT_SYMBOLS
                | SYMOPT_FAIL_CRITICAL_ERRORS
                | SYMOPT_NO_PROMPTS,
        );
        if SymInitializeW(process, path.as_ptr(), 1) == 0 {
            return BTreeMap::new();
        }
        let mut result = BTreeMap::new();
        for address in addresses {
            let mut buffer: SymbolBuffer = std::mem::zeroed();
            buffer.info.SizeOfStruct = std::mem::size_of::<SYMBOL_INFOW>() as u32;
            buffer.info.MaxNameLen = 1024;
            let mut displacement = 0;
            if SymFromAddrW(process, address as u64, &mut displacement, &mut buffer.info) != 0 {
                let length = (buffer.info.NameLen as usize).min(1023);
                let name = std::slice::from_raw_parts(buffer.info.Name.as_ptr(), length);
                result.insert(
                    address,
                    Symbol {
                        name: format!(
                            "{:#}",
                            rustc_demangle::demangle(&String::from_utf16_lossy(name))
                        ),
                        module_base: buffer.info.ModBase,
                        displacement,
                    },
                );
            }
        }
        SymCleanup(process);
        result
    }
}
pub(crate) fn finish_to(path: &Path) -> Result<(), String> {
    RECORDER.stop();
    let events = RECORDER.snapshot();
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let symbols = symbols(&events, executable.parent().unwrap_or(Path::new(".")));
    // SAFETY: a null module name requests the current process's executable module.
    let image_base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    let report = serde_json::json!({"version":1,"minimum_bytes":MINIMUM,
        "capacity_events":CAPACITY,"recorded_events":events.len(),
        "dropped_events":RECORDER.next.load(Relaxed).saturating_sub(CAPACITY),
        "trace_storage_bytes":std::mem::size_of_val(&RECORDER),
        "executable":executable,"image_base":image_base,
        "scope":"Requested Rust blocks >=64 KiB, starting before the native fixture. Fixed atomic event storage; tracing stops before output. All allocator calls in flight finish before extraction. Begin/end realloc transactions preserve failure and address-reuse ordering. Earlier allocations and smaller blocks are outside event coverage. Trace storage is static and not counted as requested Rust heap bytes; touched trace pages may affect process memory. Symbols use only local matching PDBs; unresolved addresses remain raw. Instrumented timings are not baseline latency.",
        "symbols":symbols,"events":events});
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, &report).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_waits_for_inflight_realloc_and_rejects_new_guards() {
        let recorder = Recorder::<4>::new();
        recorder.start();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let recorder_ref = &recorder;
            scope.spawn(move || {
                let guard = recorder_ref.enter();
                let transaction = guard.reallocating(123, MINIMUM, MINIMUM * 2);
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                guard.reallocated(transaction, 456);
            });
            started_rx.recv().unwrap();
            scope.spawn(|| {
                recorder.stop();
                finished_tx.send(recorder.snapshot().len()).unwrap();
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while recorder.enabled.load(SeqCst) && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert!(!recorder.enabled.load(SeqCst));
            assert!(!recorder.enter().active);
            assert!(matches!(
                finished_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            release_tx.send(()).unwrap();
            assert_eq!(finished_rx.recv().unwrap(), 2);
        });
        let events = recorder.snapshot();
        assert_eq!(events[1].transaction, events[0].sequence);
    }
    #[test]
    fn concurrent_events_are_complete_and_stop_disables_new_writers() {
        let recorder = Recorder::<512>::new();
        recorder.start();
        std::thread::scope(|scope| {
            for thread in 0..4 {
                let recorder = &recorder;
                scope.spawn(move || {
                    for i in 0..100 {
                        let guard = recorder.enter();
                        guard.freeing(thread * 100 + i + 1, MINIMUM);
                    }
                });
            }
        });
        recorder.stop();
        recorder.enter().freeing(9999, MINIMUM);
        let events = recorder.snapshot();
        assert_eq!(events.len(), 400);
        let pointers: BTreeSet<_> = events.iter().map(|event| event.pointer).collect();
        assert_eq!(pointers.len(), 400);
        assert_eq!(recorder.writers.load(SeqCst), 0);
    }
    #[test]
    fn overflow_is_visible_and_realloc_keeps_its_transaction() {
        let recorder = Recorder::<3>::new();
        recorder.start();
        {
            let guard = recorder.enter();
            let transaction = guard.reallocating(123, MINIMUM, MINIMUM * 2);
            guard.reallocated(transaction, 0);
            guard.freeing(123, MINIMUM);
            guard.freeing(456, MINIMUM);
        }
        recorder.stop();
        let events = recorder.snapshot();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].transaction, events[0].sequence);
        assert_eq!(events[1].pointer, 0);
        assert_eq!(recorder.next.load(Relaxed) - 3, 1);
    }
}
