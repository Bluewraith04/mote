//! [`StopTheWorld`]: the one [`SafepointCoordinator`]; its lock is never the scheduler's.

use contracts::{PausedRoot, SafepointCoordinator};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

struct Root(PausedRoot);

// SAFETY: a root is a raw heap pointer only read while every mutator is stopped.
unsafe impl Send for Root {}

#[derive(Default)]
struct Pause {
    entered: usize,
    stopping: bool,
    reported: usize,
    generation: u64,
    roots: Vec<Root>,
}

#[derive(Default)]
/// The stop-the-world handshake between workers and the collector.
pub struct StopTheWorld {
    state: Mutex<Pause>,
    changed: Condvar,
    pending: AtomicBool,
}

impl StopTheWorld {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SafepointCoordinator for StopTheWorld {
    fn enter(&self) {
        let mut p = self.state.lock().unwrap();
        while p.stopping {
            p = self.changed.wait(p).unwrap();
        }
        p.entered += 1;
    }

    fn leave(&self) {
        self.state.lock().unwrap().entered -= 1;
        self.changed.notify_all();
    }

    fn leave_reporting(&self, roots: &mut dyn FnMut() -> Vec<PausedRoot>) {
        let mut p = self.state.lock().unwrap();
        p.entered -= 1;
        if p.stopping {
            p.roots.extend(roots().into_iter().map(Root));
            p.reported += 1;
        }
        drop(p);
        self.changed.notify_all();
    }

    fn pause_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    fn participate(&self, roots: &mut dyn FnMut() -> Vec<PausedRoot>) {
        let mut p = self.state.lock().unwrap();
        let mut reported_for = None;
        while p.stopping {
            if reported_for != Some(p.generation) {
                reported_for = Some(p.generation);
                p.roots.extend(roots().into_iter().map(Root));
                p.reported += 1;
                self.changed.notify_all();
            }
            p = self.changed.wait(p).unwrap();
        }
    }

    fn stop_the_world(&self, collect: &mut dyn FnMut()) -> bool {
        {
            let mut p = self.state.lock().unwrap();
            if p.stopping {
                return false;
            }
            p.stopping = true;
            p.reported = 1;
            p.generation += 1;
            p.roots.clear();
            self.pending.store(true, Ordering::SeqCst);
            while p.reported < p.entered {
                p = self.changed.wait(p).unwrap();
            }
        }
        collect();
        let mut p = self.state.lock().unwrap();
        p.stopping = false;
        p.reported = 0;
        p.roots.clear();
        self.pending.store(false, Ordering::SeqCst);
        drop(p);
        self.changed.notify_all();
        true
    }

    fn take_reported_roots(&self) -> Vec<PausedRoot> {
        std::mem::take(&mut self.state.lock().unwrap().roots).into_iter().map(|r| r.0).collect()
    }
}
