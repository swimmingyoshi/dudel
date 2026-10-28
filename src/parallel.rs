//! Persistent worker threads for data-parallel bands.
//!
//! Spawning threads per call costs tens of microseconds per thread, and on
//! NVIDIA's DirectX driver every thread that ever starts leaves a 184-byte
//! record in the process heap. Display refreshes composited row bands on new
//! scoped threads thousands of times per minute, which grew memory steadily.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};

type Task = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    sender: Mutex<mpsc::Sender<Task>>,
    threads: usize,
}

fn pool() -> Option<&'static Pool> {
    static POOL: OnceLock<Option<Pool>> = OnceLock::new();
    POOL.get_or_init(|| {
        // The calling thread also works, so one fewer helper than cores.
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get().min(16))
            .saturating_sub(1);
        if threads == 0 {
            return None;
        }
        let (sender, receiver) = mpsc::channel::<Task>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut started = 0;
        for index in 0..threads {
            let receiver = receiver.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("dudel-band-{index}"))
                .spawn(move || {
                    loop {
                        let task = receiver.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        match task {
                            Ok(task) => task(),
                            Err(_) => return,
                        }
                    }
                });
            // Spawn failure only reduces throughput; the caller still works.
            started += usize::from(spawned.is_ok());
        }
        (started > 0).then(|| Pool {
            sender: Mutex::new(sender),
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
        use std::sync::atomic::Ordering;
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
        let sender = pool.sender.lock().unwrap_or_else(|e| e.into_inner());
        for _ in 0..helpers {
            let shared = shared.clone();
            let task: Task = Box::new(move || {
                {
                    let mut progress = shared.progress.lock().unwrap_or_else(|e| e.into_inner());
                    if progress.closed {
                        return;
                    }
                    progress.started += 1;
                }
                work();
                let mut progress = shared.progress.lock().unwrap_or_else(|e| e.into_inner());
                progress.finished += 1;
                shared.done.notify_all();
            });
            if sender.send(task).is_err() {
                break;
            }
        }
    }
    // The caller always participates, so queued helpers that never start
    // (for example, when called from a busy pool thread) cannot block it.
    work();
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
}
