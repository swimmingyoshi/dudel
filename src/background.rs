//! Long-lived workers for one-off background jobs such as filters.
//!
//! Like the band pool in `parallel.rs`, this avoids starting a thread per job:
//! NVIDIA's DirectX driver keeps a 184-byte record for every thread that ever
//! starts. A job runs on an idle worker; when every worker is busy another
//! starts, up to `MAX_WORKERS`, so a long job does not hold up a short one.
//! Workers then wait for the next job instead of exiting.
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

const MAX_WORKERS: usize = 8;

type Task = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct State {
    queue: VecDeque<Task>,
    idle: usize,
    workers: usize,
}

#[derive(Default)]
struct Pool {
    state: Mutex<State>,
    ready: Condvar,
}

fn pool() -> &'static Arc<Pool> {
    static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
    POOL.get_or_init(Default::default)
}

fn work(pool: Arc<Pool>) {
    loop {
        let task = {
            let mut state = pool.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(task) = state.queue.pop_front() {
                    break task;
                }
                state.idle += 1;
                state = pool.ready.wait(state).unwrap_or_else(|e| e.into_inner());
                state.idle -= 1;
            }
        };
        task();
    }
}

/// A job's result, like [`std::thread::JoinHandle`].
pub(crate) struct Handle<R> {
    slot: Arc<(Mutex<Option<std::thread::Result<R>>>, Condvar)>,
}

impl<R> Handle<R> {
    pub(crate) fn is_finished(&self) -> bool {
        self.slot
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    /// Wait for the job. A panic inside it is returned as `Err`.
    pub(crate) fn join(self) -> std::thread::Result<R> {
        let mut result = self.slot.0.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(result) = result.take() {
                return result;
            }
            result = self.slot.1.wait(result).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// Run `job` on a background worker.
pub(crate) fn spawn<R: Send + 'static>(job: impl FnOnce() -> R + Send + 'static) -> Handle<R> {
    let slot = Arc::new((Mutex::new(None), Condvar::new()));
    let out = slot.clone();
    let task: Task = Box::new(move || {
        let result = catch_unwind(AssertUnwindSafe(job));
        *out.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
        out.1.notify_all();
    });
    let pool = pool();
    let mut state = pool.state.lock().unwrap_or_else(|e| e.into_inner());
    state.queue.push_back(task);
    if state.queue.len() > state.idle && state.workers < MAX_WORKERS {
        let worker = pool.clone();
        let started = std::thread::Builder::new()
            .name(format!("dudel-background-{}", state.workers))
            .spawn(move || work(worker));
        if started.is_ok() {
            state.workers += 1;
        } else if state.workers == 0 {
            // No worker can take it: run it here rather than never.
            let task = state.queue.pop_back().unwrap();
            drop(state);
            task();
            return Handle { slot };
        }
    }
    pool.ready.notify_one();
    Handle { slot }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn jobs_return_results_and_panics_and_reuse_a_bounded_set_of_threads() {
        assert_eq!(spawn(|| 2 + 2).join().unwrap(), 4);
        assert!(spawn(|| panic!("filter failed")).join().is_err());
        let mut names = std::collections::BTreeSet::new();
        for _ in 0..50 {
            names.insert(
                spawn(|| std::thread::current().name().unwrap().to_owned())
                    .join()
                    .unwrap(),
            );
        }
        assert!(names.len() <= MAX_WORKERS, "{names:?}");
        assert!(names.iter().all(|n| n.starts_with("dudel-background-")));
    }

    #[test]
    fn a_long_job_does_not_hold_up_a_short_one() {
        let (release, wait) = mpsc::channel::<()>();
        let long = spawn(move || wait.recv().unwrap());
        let short = spawn(|| 7);
        for _ in 0..500 {
            if short.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(short.is_finished() && !long.is_finished());
        assert_eq!(short.join().unwrap(), 7);
        release.send(()).unwrap();
        long.join().unwrap();
    }
}
