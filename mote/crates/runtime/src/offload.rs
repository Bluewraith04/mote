//! Blocking platform requests run on a pool thread while their task is parked.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use contracts::PlatformResult;

use crate::sched::{SchedState, Wake};

/// A blocking request's execution: runs on a pool thread, touches no heap.
pub type OffloadWork = Box<dyn FnOnce() -> PlatformResult + Send>;

const MAX_POOL_THREADS: usize = 1024;

struct Job {
    task_id: u64,
    work: OffloadWork,
}

struct Queue {
    jobs: VecDeque<Job>,
    idle: usize,
    threads: usize,
}

struct Inner {
    queue: Mutex<Queue>,
    cv: Condvar,
    shutdown: AtomicBool,
    spinners: AtomicUsize,
    sched: Arc<Mutex<SchedState>>,
    wake: Arc<Wake>,
}

/// Lazily-grown detached thread pool; dropped with its `Runtime`.
pub struct OffloadPool {
    inner: Arc<Inner>,
}

#[derive(Clone)]
pub(crate) struct OffloadHandle {
    inner: Arc<Inner>,
}

impl OffloadHandle {
    pub(crate) fn submit(&self, task_id: u64, work: OffloadWork) {
        self.inner.submit(task_id, work);
    }

    pub(crate) fn complete(&self, task_id: u64, out: Result<PlatformResult, String>) {
        self.inner.complete(task_id, out);
    }
}

impl Inner {
    /// Queues `work` and answers whether a spinning pool thread took it.
    fn submit(self: &Arc<Self>, task_id: u64, work: OffloadWork) -> bool {
        let hot = self.spinners.load(Ordering::Acquire) > 0;
        let mut q = self.queue.lock().unwrap();
        q.jobs.push_back(Job { task_id, work });
        if q.jobs.len() > q.idle && q.threads < MAX_POOL_THREADS {
            q.threads += 1;
            let inner = self.clone();
            std::thread::spawn(move || pool_loop(inner));
        }
        drop(q);
        self.cv.notify_one();
        hot
    }

    fn complete(&self, task_id: u64, out: Result<PlatformResult, String>) {
        let mut st = self.sched.lock().unwrap();
        st.offload_cancel.remove(&task_id);
        st.offload_done.insert(task_id, out);
        st.offload_inflight -= 1;
        let woken = st.wake_or_note_early(task_id);
        drop(st);
        if let Some(pinned) = woken {
            self.wake.task_from_pool(pinned);
        }
    }
}

impl OffloadPool {
    pub(crate) fn new(sched: Arc<Mutex<SchedState>>, wake: Arc<Wake>) -> Self {
        OffloadPool {
            inner: Arc::new(Inner {
                queue: Mutex::new(Queue { jobs: VecDeque::new(), idle: 0, threads: 0 }),
                cv: Condvar::new(),
                shutdown: AtomicBool::new(false),
                spinners: AtomicUsize::new(0),
                sched,
                wake,
            }),
        }
    }

    /// Queues `work`; answers whether a pool thread was already spinning for it.
    pub(crate) fn submit(&self, task_id: u64, work: OffloadWork) -> bool {
        self.inner.submit(task_id, work)
    }

    pub(crate) fn handle(&self) -> OffloadHandle {
        OffloadHandle { inner: self.inner.clone() }
    }
}

impl Drop for OffloadPool {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.cv.notify_all();
    }
}

/// How long a pool thread that just finished a job waits for the next one before it sleeps.
const THREAD_SPIN: Duration = Duration::from_micros(200);

/// Waits `THREAD_SPIN` for a job to be queued.
fn spin_for_job(inner: &Inner) {
    if inner.spinners.fetch_add(1, Ordering::AcqRel) > 0 {
        inner.spinners.fetch_sub(1, Ordering::AcqRel);
        return;
    }
    inner.queue.lock().unwrap().idle += 1;
    let end = Instant::now() + THREAD_SPIN;
    loop {
        if inner.shutdown.load(Ordering::Relaxed) || Instant::now() >= end {
            break;
        }
        if inner.queue.try_lock().is_ok_and(|q| !q.jobs.is_empty()) {
            break;
        }
        std::hint::spin_loop();
    }
    inner.queue.lock().unwrap().idle -= 1;
    inner.spinners.fetch_sub(1, Ordering::AcqRel);
}

fn pool_loop(inner: Arc<Inner>) {
    loop {
        spin_for_job(&inner);
        let job = {
            let mut q = inner.queue.lock().unwrap();
            loop {
                if inner.shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(j) = q.jobs.pop_front() {
                    break j;
                }
                q.idle += 1;
                q = inner.cv.wait(q).unwrap();
                q.idle -= 1;
            }
        };
        let out = catch_unwind(AssertUnwindSafe(job.work))
            .map_err(|_| "a blocking native panicked".to_string());
        inner.complete(job.task_id, out);
    }
}
