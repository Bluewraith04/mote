use std::time::Duration;

use crate::{Config, Scheduler};

mod cold;
mod deadlock;
mod poller;
mod preempt;
mod queues;
mod seeded;
mod timers;

/// A small scheduler for a test: no preemption ticker, room for a few hundred tasks.
fn quick(workers: usize) -> Config {
    Config { workers, spawn_slots: 256, main_stack_bytes: 8 << 20, slice: Duration::ZERO, ..Config::new() }
}

/// Runs `main` and expects it to end.
fn run<R: Send + 'static>(config: Config, main: impl FnOnce() -> R + Send + 'static) -> R {
    Scheduler::new(config).unwrap().run(main).expect("no deadlock")
}
