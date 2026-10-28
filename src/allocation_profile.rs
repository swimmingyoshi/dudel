//! Opt-in allocation accounting. Hooks never allocate, format, or take locks.
//! Requested layout bytes exclude heap backing/overhead and external allocations.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Default)]
struct Counters {
    live: AtomicU64,
    peak: AtomicU64,
    acquired: AtomicU64,
    released: AtomicU64,
    allocations: AtomicU64,
    reallocations: AtomicU64,
    deallocations: AtomicU64,
}

impl Counters {
    fn resize(&self, old: usize, new: usize) {
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        crate::allocation_trace::account_resize(old, new);
        self.acquired.fetch_add(new as u64, Relaxed);
        self.released.fetch_add(old as u64, Relaxed);
        let live = if new >= old {
            self.live.fetch_add((new - old) as u64, Relaxed) + (new - old) as u64
        } else {
            self.live.fetch_sub((old - new) as u64, Relaxed) - (old - new) as u64
        };
        self.peak.fetch_max(live, Relaxed);
    }
}

#[derive(Default)]
struct ProfiledSystem(Counters);

// SAFETY: Every operation delegates to System with its original layout/pointer.
// Accounting is infallible and does not inspect or retain allocated memory.
unsafe impl GlobalAlloc for ProfiledSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        let trace = crate::allocation_trace::enter();
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            self.0.resize(0, layout.size());
            self.0.allocations.fetch_add(1, Relaxed);
            #[cfg(all(feature = "allocation-stack-profile", windows))]
            trace.allocated(result as usize, layout.size());
        }
        result
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        let trace = crate::allocation_trace::enter();
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            self.0.resize(0, layout.size());
            self.0.allocations.fetch_add(1, Relaxed);
            #[cfg(all(feature = "allocation-stack-profile", windows))]
            trace.allocated(result as usize, layout.size());
        }
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        let trace = crate::allocation_trace::enter();
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        trace.freeing(ptr as usize, layout.size());
        unsafe { System.dealloc(ptr, layout) };
        self.0.resize(layout.size(), 0);
        self.0.deallocations.fetch_add(1, Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        let trace = crate::allocation_trace::enter();
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        let transaction = trace.reallocating(ptr as usize, layout.size(), new_size);
        let result = unsafe { System.realloc(ptr, layout, new_size) };
        #[cfg(all(feature = "allocation-stack-profile", windows))]
        trace.reallocated(transaction, result as usize);
        if !result.is_null() {
            self.0.resize(layout.size(), new_size);
            self.0.reallocations.fetch_add(1, Relaxed);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: ProfiledSystem = ProfiledSystem(Counters {
    live: AtomicU64::new(0),
    peak: AtomicU64::new(0),
    acquired: AtomicU64::new(0),
    released: AtomicU64::new(0),
    allocations: AtomicU64::new(0),
    reallocations: AtomicU64::new(0),
    deallocations: AtomicU64::new(0),
});

#[derive(serde::Serialize)]
pub(crate) struct Snapshot {
    live_requested_bytes: u64,
    lifetime_peak_requested_bytes: u64,
    cumulative_acquired_requested_bytes: u64,
    cumulative_released_requested_bytes: u64,
    allocations: u64,
    reallocations: u64,
    deallocations: u64,
}

pub(crate) fn snapshot() -> Snapshot {
    let c = &ALLOCATOR.0;
    // Concurrent counters are individually sampled, not a stop-the-world census.
    Snapshot {
        live_requested_bytes: c.live.load(Relaxed),
        lifetime_peak_requested_bytes: c.peak.load(Relaxed),
        cumulative_acquired_requested_bytes: c.acquired.load(Relaxed),
        cumulative_released_requested_bytes: c.released.load(Relaxed),
        allocations: c.allocations.load(Relaxed),
        reallocations: c.reallocations.load(Relaxed),
        deallocations: c.deallocations.load(Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_shrink_and_release_balance_and_keep_peak() {
        let counters = Counters::default();
        counters.resize(0, 128);
        counters.resize(128, 512);
        counters.resize(512, 32);
        counters.resize(32, 0);
        assert_eq!(counters.live.load(Relaxed), 0);
        assert_eq!(counters.peak.load(Relaxed), 512);
        assert_eq!(counters.acquired.load(Relaxed), 672);
        assert_eq!(counters.released.load(Relaxed), 672);
    }

    #[test]
    fn concurrent_accounting_returns_to_zero() {
        let counters = Counters::default();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..10_000 {
                        counters.resize(0, 128);
                        counters.resize(128, 256);
                        counters.resize(256, 0);
                    }
                });
            }
        });
        assert_eq!(counters.live.load(Relaxed), 0);
        assert_eq!(counters.acquired.load(Relaxed), 40_000 * 384);
        assert_eq!(counters.released.load(Relaxed), 40_000 * 384);
        assert!((256..=4 * 256).contains(&counters.peak.load(Relaxed)));
    }

    #[test]
    fn allocator_preserves_zeroes_alignment_and_bytes_through_realloc() {
        let allocator = ProfiledSystem::default();
        let layout = Layout::from_size_align(128, 64).unwrap();
        // SAFETY: Layouts match each successful allocation/reallocation. Pointers
        // are checked and accessed only within their current allocation.
        unsafe {
            let ptr = allocator.alloc_zeroed(layout);
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % 64, 0);
            assert!(std::slice::from_raw_parts(ptr, 128).iter().all(|b| *b == 0));
            ptr.write(73);
            let ptr = allocator.realloc(ptr, layout, 512);
            assert!(!ptr.is_null());
            assert_eq!(ptr.read(), 73);
            let grown = Layout::from_size_align(512, 64).unwrap();
            let ptr = allocator.realloc(ptr, grown, 32);
            assert!(!ptr.is_null());
            assert_eq!(ptr.read(), 73);
            allocator.dealloc(ptr, Layout::from_size_align(32, 64).unwrap());
        }
        assert_eq!(allocator.0.live.load(Relaxed), 0);
        assert_eq!(allocator.0.peak.load(Relaxed), 512);
        assert_eq!(allocator.0.allocations.load(Relaxed), 1);
        assert_eq!(allocator.0.reallocations.load(Relaxed), 2);
        assert_eq!(allocator.0.deallocations.load(Relaxed), 1);
    }
}
