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

#[test]
fn single_runs_directly_supports_exit_and_checks_stack_limit() {
    use crate::{direct_stack_limit, exit, in_a_task, run_single, stack_trap, Trap, MAIN_STACK_BYTES};

    let got = run_single(MAIN_STACK_BYTES, || {
        assert!(!in_a_task());
        let limit = direct_stack_limit().expect("direct limit set");
        assert_eq!(stack_trap(limit - 1), Trap::Overflow);
        assert_eq!(stack_trap(limit), Trap::Spurious);
        42
    })
    .unwrap();
    assert_eq!(got, Some(42));

    let exited: Option<i32> = run_single(MAIN_STACK_BYTES, || {
        exit();
    })
    .unwrap();
    assert_eq!(exited, None);

    let panicked = std::panic::catch_unwind(|| {
        let _ = run_single(MAIN_STACK_BYTES, || -> i32 {
            panic!("boom");
        });
    });
    assert!(panicked.is_err());
}
