//! Safepoints: threads that run program code register as mutators and a stop waits until every one has parked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};

#[derive(Default)]
struct State {
    mutators: usize,
    parked: usize,
    stopping: bool,
}

/// The threads that run program code, and the handshake that stops them.
#[derive(Default)]
pub struct World {
    state: Mutex<State>,
    /// Signals that a mutator parked or left.
    reached: Condvar,
    /// Signals that a stop ended.
    resumed: Condvar,
    requested: AtomicBool,
}

/// One registered thread; it polls at its safepoints and leaves the world when dropped.
pub struct Mutator<'w> {
    world: &'w World,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("the world")
    }

    /// Adds the calling thread; it waits while a stop is in progress.
    pub fn register(&self) -> Mutator<'_> {
        self.enter();
        Mutator { world: self }
    }

    /// Counts one more mutator, as `register` does, until a matching `leave`; for a mutator that is not a thread's guard, such as a task.
    pub fn enter(&self) {
        let mut state = self.lock();
        while state.stopping {
            state = self.resumed.wait(state).expect("the world");
        }
        state.mutators += 1;
    }

    /// Counts one mutator fewer.
    pub fn leave(&self) {
        let mut state = self.lock();
        state.mutators -= 1;
        self.reached.notify_all();
    }

    /// The safepoint of a mutator counted by `enter`: parks while a stop is in progress.
    pub fn poll(&self) {
        if self.requested.load(Ordering::Acquire) {
            self.park();
        }
    }

    fn park(&self) {
        let mut state = self.lock();
        if !state.stopping {
            return;
        }
        state.parked += 1;
        self.reached.notify_all();
        while state.stopping {
            state = self.resumed.wait(state).expect("the world");
        }
        state.parked -= 1;
    }

    /// `stop`, where `request` runs once the stop is asked for and before the wait, to make each mutator poll.
    pub fn stop_requesting<R>(&self, request: impl FnOnce(), f: impl FnOnce() -> R) -> R {
        self.stopped_with(false, request, f)
    }

    /// `Mutator::stop` for a mutator counted by `enter`, where `request` runs once the stop is asked for.
    pub fn stop_from_mutator<R>(&self, request: impl FnOnce(), f: impl FnOnce() -> R) -> R {
        self.stopped_with(true, request, f)
    }

    /// Runs `f` while every mutator is parked; for a thread that is not a mutator.
    pub fn stop<R>(&self, f: impl FnOnce() -> R) -> R {
        self.stopped(false, f)
    }

    fn stopped<R>(&self, mutator: bool, f: impl FnOnce() -> R) -> R {
        self.stopped_with(mutator, || {}, f)
    }

    fn stopped_with<R>(&self, mutator: bool, request: impl FnOnce(), f: impl FnOnce() -> R) -> R {
        struct Resume<'w>(&'w World, bool);
        impl Drop for Resume<'_> {
            fn drop(&mut self) {
                let mut state = self.0.lock();
                state.stopping = false;
                if self.1 {
                    state.parked -= 1;
                }
                self.0.requested.store(false, Ordering::Release);
                self.0.resumed.notify_all();
            }
        }
        let mut state = self.lock();
        if mutator {
            state.parked += 1;
            self.reached.notify_all();
        }
        while state.stopping {
            state = self.resumed.wait(state).expect("the world");
        }
        state.stopping = true;
        self.requested.store(true, Ordering::Release);
        request();
        while state.parked < state.mutators {
            state = self.reached.wait(state).expect("the world");
        }
        drop(state);
        let _resume = Resume(self, mutator);
        f()
    }

    /// Whether a stop is asked for.
    pub fn stop_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    /// Mutators registered now.
    pub fn mutators(&self) -> usize {
        self.lock().mutators
    }
}

impl Mutator<'_> {
    /// The safepoint: parks while a stop is in progress. One atomic load when none is.
    pub fn poll(&self) {
        self.world.poll();
    }

    /// Runs `f`, which blocks and touches no object, with this thread counted as parked.
    pub fn blocking<R>(&self, f: impl FnOnce() -> R) -> R {
        {
            let mut state = self.world.lock();
            state.parked += 1;
            self.world.reached.notify_all();
        }
        let result = f();
        let mut state = self.world.lock();
        while state.stopping {
            state = self.world.resumed.wait(state).expect("the world");
        }
        state.parked -= 1;
        result
    }

    /// Runs `f` while every other mutator is parked; this thread is at a safepoint.
    pub fn stop<R>(&self, f: impl FnOnce() -> R) -> R {
        self.world.stopped(true, f)
    }
}

impl Drop for Mutator<'_> {
    fn drop(&mut self) {
        self.world.leave();
    }
}
