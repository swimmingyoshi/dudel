//! Persistent worker threads for data-parallel bands.
//!
//! Spawning threads per call costs tens of microseconds per thread, and on
//! NVIDIA's DirectX driver every thread that ever starts leaves a 184-byte
//! record in the process heap. Display refreshes composited row bands on new
//! scoped threads thousands of times per minute, which grew memory steadily.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

type Task = Box<dyn FnOnce() + Send + 'static>;

/// Tasks wait in one queue that every idle helper watches, so a call wakes
/// all the helpers it needs at once instead of each waking the next.
struct Pool {
    queue: Mutex<std::collections::VecDeque<Task>>,
    ready: Condvar,
    /// Queued tasks, readable without the lock while a helper spins.
    pending: std::sync::atomic::AtomicUsize,
    threads: usize,
}

/// How long a helper keeps looking for work after a task before sleeping.
/// Calls often come in quick succession (a dab's coverage, then its paint),
/// and waking a sleeping core takes longer than this.
const LINGER: std::time::Duration = std::time::Duration::from_micros(50);

fn pool() -> Option<&'static Pool> {
    static POOL: OnceLock<Option<Pool>> = OnceLock::new();
    POOL.get_or_init(|| {
        // The calling thread also works, so one fewer helper than cores.
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get().min(16))
            .saturating_sub(1);
        let mut started = 0;
        for index in 0..threads {
            // Helpers wait here until the pool below is stored.
            let spawned = std::thread::Builder::new()
                .name(format!("dudel-band-{index}"))
                .spawn(|| {
                    let Some(pool) = pool() else { return };
                    loop {
                        let linger = std::time::Instant::now();
                        while pool.pending.load(Ordering::Relaxed) == 0 && linger.elapsed() < LINGER
                        {
                            std::hint::spin_loop();
                        }
                        let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
                        let task = loop {
                            if let Some(task) = queue.pop_front() {
                                pool.pending.fetch_sub(1, Ordering::Relaxed);
                                break task;
                            }
                            queue = pool.ready.wait(queue).unwrap_or_else(|e| e.into_inner());
                        };
                        drop(queue);
                        task();
                    }
                });
            // Spawn failure only reduces throughput; the caller still works.
            started += usize::from(spawned.is_ok());
        }
        (started > 0).then(|| Pool {
            queue: Mutex::new(std::collections::VecDeque::new()),
            ready: Condvar::new(),
            pending: Default::default(),
            threads: started,
        })
    })
    .as_ref()
}

#[derive(Default)]
struct Progress {
    started: usize,
    finished: usize,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    progress: Mutex<Progress>,
    done: Condvar,
    /// Helpers inside `work`, readable without the lock while the caller spins.
    running: std::sync::atomic::AtomicUsize,
}

/// Run `job(index)` for every index in `0..count` on the calling thread and the
/// persistent pool, returning each result in index order. A panicking job
/// yields `Err` for its index; every started job has finished before return.
pub(crate) fn run<R: Send>(
    count: usize,
    job: impl Fn(usize) -> R + Sync,
) -> Vec<std::thread::Result<R>> {
    let slots: Vec<Mutex<Option<std::thread::Result<R>>>> =
        (0..count).map(|_| Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let work = || {
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= count {
                return;
            }
            let result = catch_unwind(AssertUnwindSafe(|| job(index)));
            *slots[index].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        }
    };
    let shared = Arc::new(Shared::default());
    if let Some(pool) = pool().filter(|_| count > 1) {
        let helpers = pool.threads.min(count - 1);
        let work: &(dyn Fn() + Sync) = &work;
        // SAFETY: helpers only use `work` (and through it the borrowed job and
        // slots) after registering as started while the call is open. The
        // caller closes the call and waits until every started helper has
        // finished before these borrows end, and `work` never unwinds. Helpers
        // that start after closing return without touching borrowed data.
        let work: &'static (dyn Fn() + Sync) = unsafe { std::mem::transmute(work) };
        let mut queue = pool.queue.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..helpers {
            let shared = shared.clone();
            queue.push_back(Box::new(move || {
                {
                    let mut progress = shared.progress.lock().unwrap_or_else(|e| e.into_inner());
                    if progress.closed {
                        return;
                    }
                    progress.started += 1;
                    shared.running.fetch_add(1, Ordering::Relaxed);
                }
                work();
                let mut progress = shared.progress.lock().unwrap_or_else(|e| e.into_inner());
                progress.finished += 1;
                shared.running.fetch_sub(1, Ordering::Release);
                shared.done.notify_all();
            }));
        }
        pool.pending.fetch_add(helpers, Ordering::Relaxed);
        drop(queue);
        for _ in 0..helpers {
            pool.ready.notify_one();
        }
    }
    // The caller always participates, so queued helpers that never start
    // (for example, when called from a busy pool thread) cannot block it.
    work();
    // Helpers usually finish within moments of the caller; waiting for them
    // without sleeping avoids a wake-up that costs more than the wait.
    let linger = std::time::Instant::now();
    while shared.running.load(Ordering::Acquire) > 0 && linger.elapsed() < LINGER * 4 {
        std::hint::spin_loop();
    }
    let mut progress = shared.progress.lock().unwrap_or_else(|e| e.into_inner());
    progress.closed = true;
    while progress.finished < progress.started {
        progress = shared
            .done
            .wait(progress)
            .unwrap_or_else(|e| e.into_inner());
    }
    drop(progress);
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every index runs before return")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn results_keep_index_order_and_panics_stay_per_index() {
        let results = run(64, |index| {
            if index == 17 {
                panic!("injected");
            }
            index * 3
        });
        assert_eq!(results.len(), 64);
        for (index, result) in results.into_iter().enumerate() {
            match result {
                Ok(value) => assert_eq!(value, index * 3),
                Err(_) => assert_eq!(index, 17),
            }
        }
        assert!(run(0, |index| index).is_empty());
    }

    #[test]
    fn borrowed_mutable_bands_are_all_written_before_return() {
        let mut out = vec![0u32; 1000];
        let bands: Vec<_> = out.chunks_mut(64).map(Mutex::new).collect();
        for result in run(bands.len(), |index| {
            for value in bands[index].lock().unwrap().iter_mut() {
                *value = index as u32 + 1;
            }
        }) {
            result.unwrap();
        }
        drop(bands);
        for (position, value) in out.iter().enumerate() {
            assert_eq!(*value, position as u32 / 64 + 1);
        }
    }

    #[test]
    fn nested_calls_from_pool_threads_complete() {
        let total = AtomicUsize::new(0);
        for result in run(32, |_| {
            for inner in run(8, |value| value) {
                total.fetch_add(inner.unwrap(), Ordering::Relaxed);
            }
        }) {
            result.unwrap();
        }
        assert_eq!(total.load(Ordering::Relaxed), 32 * 28);
    }

    #[test]
    fn repeated_calls_create_no_threads_after_the_pool_starts() {
        run(4, |index| index);
        let names: Vec<_> = (0..200)
            .flat_map(|_| run(8, |_| std::thread::current().name().map(str::to_owned)))
            .map(Result::unwrap)
            .collect();
        // Every band ran on the caller or a named persistent pool thread.
        let caller = std::thread::current().name().map(str::to_owned);
        assert!(names.iter().all(|name| {
            *name == caller
                || name
                    .as_deref()
                    .is_some_and(|name| name.starts_with("dudel-band-"))
        }));
    }
    #[test]
    #[ignore]
    fn probe_short_job_latency() {
        let spin = |micros: u64| {
            let start = std::time::Instant::now();
            while start.elapsed().as_micros() < micros as u128 {
                std::hint::spin_loop();
            }
        };
        run(4, |_| ());
        for gap in [0u64, 2000] {
            let mut total = 0.0;
            for _ in 0..50 {
                if gap > 0 {
                    std::thread::sleep(std::time::Duration::from_micros(gap));
                }
                let start = std::time::Instant::now();
                run(10, |_| spin(50));
                total += start.elapsed().as_secs_f64() * 1e6;
            }
            eprintln!(
                "gap {gap} us: 10 x 50 us jobs took {:.0} us on average",
                total / 50.0
            );
        }
    }
}
