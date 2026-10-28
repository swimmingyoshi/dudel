//! Optional diagnostic census of this process's virtual memory, for locating
//! committed memory outside Rust allocation requests. Sampling uses fixed
//! storage only: it walks VirtualQuery regions and Windows heap summaries.
//! Region types overlap with heap totals; they are views, not additive budgets.

/// Private reservations are bucketed by their whole reserved span:
/// <64 KiB, <256 KiB, <1 MiB, <4 MiB, <16 MiB, <64 MiB, <256 MiB, larger.
const BUCKETS: usize = 8;
const HEAPS: usize = 32;

#[derive(Clone, Copy, Default, serde::Serialize)]
pub(crate) struct Heap {
    handle: usize,
    committed_bytes: usize,
    allocated_bytes: usize,
}

#[derive(Clone, Copy, Default, serde::Serialize)]
pub(crate) struct Snapshot {
    private_committed_bytes: usize,
    private_reserved_only_bytes: usize,
    mapped_committed_bytes: usize,
    image_committed_bytes: usize,
    private_allocations: usize,
    private_bucket_committed_bytes: [usize; BUCKETS],
    private_bucket_allocations: [usize; BUCKETS],
    heap_count: usize,
    heaps_omitted: usize,
    heaps: [Heap; HEAPS],
}

#[cfg(any(windows, test))]
fn bucket(span: usize) -> usize {
    let mut limit = 64 * 1024;
    for index in 0..BUCKETS - 1 {
        if span < limit {
            return index;
        }
        limit *= 4;
    }
    BUCKETS - 1
}

#[derive(Default)]
#[cfg(windows)]
struct Reservation {
    base: usize,
    span: usize,
    committed: usize,
}

#[cfg(windows)]
impl Snapshot {
    fn close(&mut self, reservation: &Reservation) {
        if reservation.span == 0 {
            return;
        }
        let index = bucket(reservation.span);
        self.private_allocations += 1;
        self.private_bucket_allocations[index] += 1;
        self.private_bucket_committed_bytes[index] += reservation.committed;
    }
}

#[cfg(windows)]
pub(crate) fn snapshot() -> Snapshot {
    use windows_sys::Win32::System::Memory::{
        GetProcessHeaps, HEAP_SUMMARY, HeapSummary, MEM_COMMIT, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE,
        MEM_RESERVE, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    let mut census = Snapshot::default();
    let mut open = Reservation::default();
    let mut address = 0usize;
    loop {
        // Zeroed output storage is valid; VirtualQuery reports 0 past the
        // highest user address, which ends the walk.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let written = unsafe {
            VirtualQuery(
                address as *const _,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 || info.RegionSize == 0 {
            break;
        }
        let size = info.RegionSize;
        let committed = info.State == MEM_COMMIT;
        match info.Type {
            MEM_PRIVATE => {
                let base = info.AllocationBase as usize;
                if base != open.base {
                    census.close(&open);
                    open = Reservation {
                        base,
                        ..Reservation::default()
                    };
                }
                open.span += size;
                if committed {
                    open.committed += size;
                    census.private_committed_bytes += size;
                } else if info.State == MEM_RESERVE {
                    census.private_reserved_only_bytes += size;
                }
            }
            MEM_MAPPED if committed => census.mapped_committed_bytes += size,
            MEM_IMAGE if committed => census.image_committed_bytes += size,
            _ => {}
        }
        match address.checked_add(size) {
            Some(next) => address = next,
            None => break,
        }
    }
    census.close(&open);

    let mut handles = [std::ptr::null_mut(); HEAPS];
    // The fixed buffer is passed with its exact capacity; a larger total is
    // reported back and recorded as omitted heaps.
    let count = unsafe { GetProcessHeaps(HEAPS as u32, handles.as_mut_ptr()) } as usize;
    census.heap_count = count;
    census.heaps_omitted = count.saturating_sub(HEAPS);
    for (slot, handle) in census.heaps.iter_mut().zip(&handles[..count.min(HEAPS)]) {
        let mut summary: HEAP_SUMMARY = unsafe { std::mem::zeroed() };
        summary.cb = std::mem::size_of::<HEAP_SUMMARY>() as u32;
        // Handles come from GetProcessHeaps for this process; a heap destroyed
        // concurrently makes the call fail, leaving that slot's sizes zero.
        if unsafe { HeapSummary(*handle, 0, &mut summary) } != 0 {
            *slot = Heap {
                handle: *handle as usize,
                committed_bytes: summary.cbCommitted,
                allocated_bytes: summary.cbAllocated,
            };
        } else {
            slot.handle = *handle as usize;
        }
    }
    census
}

#[cfg(not(windows))]
pub(crate) fn snapshot() -> Snapshot {
    Snapshot::default()
}

/// Blocks keyed by their first pointer-sized word when it points into a loaded
/// image (typically a C++ vtable), else by exact block size with the top bit set.
#[cfg(windows)]
const SIZE_KEY: u64 = 1 << 63;
const SLOTS: usize = 1 << 15;

#[derive(Clone, Copy, Default, serde::Serialize)]
pub(crate) struct Slot {
    key: u64,
    count: u32,
    bytes: u64,
}

/// One preallocated census of busy blocks in the default process heap.
#[derive(serde::Serialize)]
pub(crate) struct HeapWalk {
    round: usize,
    walked: bool,
    busy_blocks: usize,
    busy_bytes: usize,
    image_keyed_blocks: usize,
    overflow_blocks: usize,
    #[serde(serialize_with = "used_slots")]
    slots: Box<[Slot]>,
    /// Optional contents of up to `SAMPLES` blocks of `sample_size` bytes,
    /// chosen with DUDEL_HEAP_SAMPLE_SIZE to identify an unknown owner.
    sample_size: usize,
    #[serde(serialize_with = "used_samples")]
    samples: Box<[[u64; SAMPLE_WORDS]]>,
    sampled: usize,
}

const SAMPLES: usize = 256;
const SAMPLE_WORDS: usize = 32;

fn used_samples<S: serde::Serializer>(
    samples: &[[u64; SAMPLE_WORDS]],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(None)?;
    for sample in samples
        .iter()
        .filter(|sample| sample.iter().any(|word| *word != 0))
    {
        seq.serialize_element(&sample[..])?;
    }
    seq.end()
}

fn used_slots<S: serde::Serializer>(slots: &[Slot], serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(None)?;
    for slot in slots.iter().filter(|slot| slot.count > 0) {
        seq.serialize_element(slot)?;
    }
    seq.end()
}

impl HeapWalk {
    pub(crate) fn reserve() -> Self {
        Self {
            round: 0,
            walked: false,
            busy_blocks: 0,
            busy_bytes: 0,
            image_keyed_blocks: 0,
            overflow_blocks: 0,
            slots: vec![Slot::default(); SLOTS].into_boxed_slice(),
            sample_size: std::env::var("DUDEL_HEAP_SAMPLE_SIZE")
                .ok()
                .and_then(|size| size.parse().ok())
                .unwrap_or(0),
            samples: vec![[0; SAMPLE_WORDS]; SAMPLES].into_boxed_slice(),
            sampled: 0,
        }
    }

    #[cfg(windows)]
    fn add(&mut self, key: u64, size: usize) {
        let mut index = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 49) as usize;
        for _ in 0..SLOTS {
            let slot = &mut self.slots[index];
            if slot.count == 0 || slot.key == key {
                slot.key = key;
                slot.count += 1;
                slot.bytes += size as u64;
                return;
            }
            index = (index + 1) & (SLOTS - 1);
        }
        self.overflow_blocks += 1;
    }
}

#[cfg(windows)]
mod images {
    use windows_sys::Win32::System::ProcessStatus::{
        K32EnumProcessModules, K32GetModuleInformation, MODULEINFO,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    pub(super) const MODULES: usize = 1024;

    /// Sorted [start, end) ranges of loaded images, in caller-owned storage.
    pub(super) fn ranges(storage: &mut [(usize, usize); MODULES]) -> usize {
        let mut handles = [std::ptr::null_mut(); MODULES];
        let mut needed = 0u32;
        let process = unsafe { GetCurrentProcess() };
        // The handle buffer is passed with its exact byte size.
        let listed = unsafe {
            K32EnumProcessModules(
                process,
                handles.as_mut_ptr(),
                std::mem::size_of_val(&handles) as u32,
                &mut needed,
            )
        };
        if listed == 0 {
            return 0;
        }
        let count = (needed as usize / std::mem::size_of::<*mut ()>()).min(MODULES);
        let mut used = 0;
        for handle in &handles[..count] {
            let mut info: MODULEINFO = unsafe { std::mem::zeroed() };
            // A module unloaded since enumeration fails and is skipped.
            let found = unsafe {
                K32GetModuleInformation(
                    process,
                    *handle,
                    &mut info,
                    std::mem::size_of::<MODULEINFO>() as u32,
                )
            };
            if found != 0 {
                let start = info.lpBaseOfDll as usize;
                storage[used] = (start, start + info.SizeOfImage as usize);
                used += 1;
            }
        }
        storage[..used].sort_unstable();
        used
    }

    pub(super) fn contains(ranges: &[(usize, usize)], address: usize) -> bool {
        let index = ranges.partition_point(|range| range.0 <= address);
        index > 0 && address < ranges[index - 1].1
    }
}

/// Walk the default process heap into `walk` without allocating. The heap is
/// locked for the duration, so other threads' heap calls wait.
#[cfg(windows)]
pub(crate) fn walk_default_heap(walk: &mut HeapWalk, round: usize) {
    use windows_sys::Win32::System::Memory::{
        GetProcessHeap, HeapLock, HeapUnlock, HeapWalk as Walk, PROCESS_HEAP_ENTRY,
    };
    const BUSY: u16 = 4;
    let mut ranges = [(0usize, 0usize); images::MODULES];
    let used = images::ranges(&mut ranges);
    let ranges = &ranges[..used];
    walk.round = round;
    let heap = unsafe { GetProcessHeap() };
    if unsafe { HeapLock(heap) } == 0 {
        return;
    }
    let mut entry: PROCESS_HEAP_ENTRY = unsafe { std::mem::zeroed() };
    // HeapWalk starts from a null lpData and reports false at the end.
    while unsafe { Walk(heap, &mut entry) } != 0 {
        if entry.wFlags & BUSY == 0 {
            continue;
        }
        let size = entry.cbData as usize;
        walk.busy_blocks += 1;
        walk.busy_bytes += size;
        let mut key = SIZE_KEY | size as u64;
        if size >= std::mem::size_of::<usize>() {
            // Busy blocks are committed heap memory; this reads its first word
            // while the heap lock prevents it being freed.
            let first = unsafe { (entry.lpData as *const usize).read_unaligned() };
            if images::contains(ranges, first) {
                key = first as u64;
                walk.image_keyed_blocks += 1;
            }
        }
        walk.add(key, size);
        if size == walk.sample_size && walk.sampled < SAMPLES {
            let words = (size / 8).min(SAMPLE_WORDS);
            // The same committed busy block as above, read in whole words.
            let block = unsafe { std::slice::from_raw_parts(entry.lpData as *const u8, words * 8) };
            for (word, bytes) in walk.samples[walk.sampled]
                .iter_mut()
                .zip(block.chunks_exact(8))
            {
                *word = u64::from_le_bytes(bytes.try_into().unwrap());
            }
            walk.sampled += 1;
        }
    }
    unsafe { HeapUnlock(heap) };
    walk.walked = true;
}

#[cfg(not(windows))]
pub(crate) fn walk_default_heap(_walk: &mut HeapWalk, _round: usize) {}

/// Loaded image names and ranges for resolving image keys after a session.
pub(crate) fn modules() -> serde_json::Value {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
        let mut ranges = [(0usize, 0usize); images::MODULES];
        let used = images::ranges(&mut ranges);
        let modules: Vec<_> = ranges[..used]
            .iter()
            .map(|&(start, end)| {
                let mut name = [0u16; 1024];
                // A module's base address is its HMODULE; the call writes at
                // most the buffer length.
                let length = unsafe {
                    GetModuleFileNameW(start as *mut _, name.as_mut_ptr(), name.len() as u32)
                } as usize;
                serde_json::json!({
                    "name": String::from_utf16_lossy(&name[..length]),
                    "start": start,
                    "end": end,
                })
            })
            .collect();
        serde_json::Value::Array(modules)
    }
    #[cfg(not(windows))]
    serde_json::Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_split_reservation_spans_by_powers_of_four() {
        assert_eq!(bucket(4096), 0);
        assert_eq!(bucket(64 * 1024), 1);
        assert_eq!(bucket(1024 * 1024 - 1), 2);
        assert_eq!(bucket(1024 * 1024), 3);
        assert_eq!(bucket(256 * 1024 * 1024), 7);
        assert_eq!(bucket(usize::MAX), 7);
    }

    #[cfg(windows)]
    #[test]
    fn census_sees_a_new_committed_private_reservation() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        const SIZE: usize = 32 * 1024 * 1024;
        // A fresh reservation of a known size; released before asserting.
        // Other tests allocate concurrently, so only lower bounds are checked.
        let pointer = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                SIZE,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        assert!(!pointer.is_null());
        let during = snapshot();
        assert!(unsafe { VirtualFree(pointer, 0, MEM_RELEASE) } != 0);
        assert!(during.private_bucket_committed_bytes[bucket(SIZE)] >= SIZE);
        assert!(during.private_committed_bytes >= SIZE);
        assert!(during.heap_count >= 1);
        assert!(during.heaps[0].committed_bytes > 0);
    }

    #[cfg(windows)]
    #[test]
    fn heap_walk_counts_busy_blocks_and_image_pointers() {
        // A heap block whose first word points into this executable's image.
        static MARKER: u8 = 0;
        let held = Box::new([std::ptr::addr_of!(MARKER) as usize, 0, 0, 0]);
        let mut walk = HeapWalk::reserve();
        walk_default_heap(&mut walk, 3);
        let key = std::ptr::addr_of!(MARKER) as u64;
        assert!(walk.walked && walk.round == 3);
        assert!(walk.busy_blocks > 0 && walk.busy_bytes >= 32);
        assert!(
            walk.slots
                .iter()
                .any(|slot| slot.key == key && slot.count >= 1)
        );
        drop(held);
        assert!(modules().as_array().is_some_and(|list| !list.is_empty()));
    }
}
