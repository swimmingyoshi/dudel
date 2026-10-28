//! Bounded, persistent workers for owned display-compositing jobs.
//!
//! Kept separate from the borrowed band pool: display jobs wait for band work.
//! Submission never waits for space; callers retain their sliced fallback on
//! exhaustion. These workers must not synchronously wait on other display jobs.
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

type Task = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    sender: mpsc::SyncSender<Task>,
}

impl Pool {
    fn new(threads: usize, queued: usize) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Task>(queued);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut started = 0;
        for index in 0..threads {
            let receiver = receiver.clone();
            if std::thread::Builder::new()
                .name(format!("dudel-display-{index}"))
                .spawn(move || {
                    loop {
                        let task = receiver.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        match task {
                            Ok(task) => task(),
                            Err(_) => break,
                        }
                    }
                })
                .is_ok()
            {
                started += 1;
            }
        }
        if started == 0 {
            return Err(io::Error::other("No display workers could be started"));
        }
        Ok(Self { sender })
    }

    fn submit<T: Send + 'static>(
        &self,
        run: impl FnOnce() -> T + Send + 'static,
    ) -> io::Result<Job<T>> {
        let (sender, result) = mpsc::sync_channel(1);
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        let task = Box::new(move || {
            // Fully consume/drop captured snapshots before publishing completion.
            let output = catch_unwind(AssertUnwindSafe(run));
            let _ = sender.send(output);
            done.store(true, Ordering::Release);
        });
        self.sender.try_send(task).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                io::Error::new(io::ErrorKind::WouldBlock, "Display worker queue is full")
            }
            mpsc::TrySendError::Disconnected(_) => io::Error::other("Display workers stopped"),
        })?;
        Ok(Job { result, finished })
    }
}

pub(crate) struct Job<T> {
    result: mpsc::Receiver<std::thread::Result<T>>,
    finished: Arc<AtomicBool>,
}

impl<T> Job<T> {
    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(crate) fn join(self) -> std::thread::Result<T> {
        self.result
            .recv()
            .unwrap_or_else(|_| Err(Box::new("Display worker disconnected")))
    }
}

pub(crate) fn spawn<T: Send + 'static>(
    run: impl FnOnce() -> T + Send + 'static,
) -> io::Result<Job<T>> {
    static POOL: OnceLock<io::Result<Pool>> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(4, 64))
        .as_ref()
        .map_err(|error| io::Error::other(error.to_string()))?
        .submit(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn repeated_jobs_reuse_thread_and_survive_panic_and_abandoned_result() {
        let pool = Pool::new(1, 2).unwrap();
        let id = pool
            .submit(|| std::thread::current().id())
            .unwrap()
            .join()
            .unwrap();
        for _ in 0..200 {
            assert_eq!(
                pool.submit(|| std::thread::current().id())
                    .unwrap()
                    .join()
                    .unwrap(),
                id
            );
        }
        assert!(
            pool.submit(|| panic!("injected display failure"))
                .unwrap()
                .join()
                .is_err()
        );
        drop(pool.submit(|| vec![0u8; 1024]).unwrap());
        assert_eq!(pool.submit(|| 42).unwrap().join().unwrap(), 42);
    }

    #[test]
    fn full_queue_returns_immediately_and_releases_rejected_capture() {
        let pool = Pool::new(1, 1).unwrap();
        let (started, running) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let active = pool
            .submit(move || {
                started.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(10)).unwrap();
            })
            .unwrap();
        running.recv_timeout(Duration::from_secs(10)).unwrap();
        let queued = pool.submit(|| 7).unwrap();
        let captured = Arc::new(());
        let weak = Arc::downgrade(&captured);
        let rejected = pool.submit(move || drop(captured));
        assert!(matches!(rejected, Err(ref error) if error.kind() == io::ErrorKind::WouldBlock));
        assert!(weak.upgrade().is_none());
        release.send(()).unwrap();
        active.join().unwrap();
        assert_eq!(queued.join().unwrap(), 7);
    }

    #[test]
    fn completed_job_no_longer_retains_its_source_capture() {
        let pool = Pool::new(1, 1).unwrap();
        let source = Arc::new(vec![0u8; 4096]);
        let weak = Arc::downgrade(&source);
        let job = pool
            .submit(move || {
                assert_eq!(source.len(), 4096);
            })
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !job.is_finished() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(weak.upgrade().is_none());
        job.join().unwrap();
    }
}
