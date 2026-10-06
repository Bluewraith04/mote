//! Blocking platform requests: a request runs on a pool thread while its task is parked. The pool grows on demand; waits for time or readiness hold no thread.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

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
    fn submit(self: &Arc<Self>, task_id: u64, work: OffloadWork) {
        let mut q = self.queue.lock().unwrap();
        q.jobs.push_back(Job { task_id, work });
        if q.jobs.len() > q.idle && q.threads < MAX_POOL_THREADS {
            q.threads += 1;
            let inner = self.clone();
            std::thread::spawn(move || pool_loop(inner));
        }
        drop(q);
        self.cv.notify_one();
    }

    fn complete(&self, task_id: u64, out: Result<PlatformResult, String>) {
        let mut st = self.sched.lock().unwrap();
        st.offload_done.insert(task_id, out);
        st.offload_inflight -= 1;
        st.wake_or_note_early(task_id);
        drop(st);
        self.wake.all();
    }
}

impl OffloadPool {
    pub(crate) fn new(sched: Arc<Mutex<SchedState>>, wake: Arc<Wake>) -> Self {
        OffloadPool {
            inner: Arc::new(Inner {
                queue: Mutex::new(Queue { jobs: VecDeque::new(), idle: 0, threads: 0 }),
                cv: Condvar::new(),
                shutdown: AtomicBool::new(false),
                sched,
                wake,
            }),
        }
    }

    pub(crate) fn submit(&self, task_id: u64, work: OffloadWork) {
        self.inner.submit(task_id, work);
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

fn pool_loop(inner: Arc<Inner>) {
    loop {
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
