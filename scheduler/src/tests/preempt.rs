use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mote_abi::{TaskContext, CONTEXT_POLL, CONTEXT_STACK_LIMIT};

use super::{quick, run};
use crate::{current, park, poll_point, sleep, spawn, stack_trap, yield_now, Config, Poll, TaskHandle, TaskState, Trap, Wake, RED_ZONE};

const MS: u64 = 1_000_000;

/// The word of the task's context at `offset`, read as compiled code would.
fn word(task: &TaskHandle, offset: u32) -> u64 {
    // SAFETY: the context is a live block of the task and the word is 8-aligned.
    unsafe { AtomicU64::from_ptr((task.context() as usize + offset as usize) as *mut u64) }.load(Ordering::SeqCst)
}

fn wait_until_parked(task: &TaskHandle) {
    while task.state() != TaskState::Parked {
        yield_now();
    }
}

#[test]
fn preempt_a_context_starts_with_its_stack_limit_set_and_no_poll_request() {
    run(quick(1), || {
        let me = current();
        let (base, _) = me.stack_bounds().unwrap();
        assert_eq!(word(&me, CONTEXT_STACK_LIMIT), (base + RED_ZONE) as u64);
        assert_eq!(word(&me, CONTEXT_POLL), 0);
        assert_eq!(std::mem::size_of::<TaskContext>(), 48);
    });
}

#[test]
fn preempt_the_ticker_sets_the_poll_flag_of_a_task_that_runs_a_whole_slice() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = log.clone();
    run(Config { slice: Duration::from_millis(5), ..quick(1) }, move || {
        let spinner = {
            let log = shared.clone();
            spawn(move || {
                let me = current();
                let give_up = Instant::now() + Duration::from_secs(10);
                while word(&me, CONTEXT_POLL) == 0 && Instant::now() < give_up {
                    std::hint::spin_loop();
                }
                log.lock().unwrap().push("flagged");
                assert_eq!(poll_point(), Poll::Continue);
                log.lock().unwrap().push("resumed");
            })
        };
        let other = {
            let log = shared.clone();
            spawn(move || log.lock().unwrap().push("other"))
        };
        spinner.join().unwrap();
        other.join().unwrap();
    });
    assert_eq!(*log.lock().unwrap(), ["flagged", "other", "resumed"]);
}

#[test]
fn preempt_a_poll_point_without_a_request_does_not_yield() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = log.clone();
    run(quick(1), move || {
        let other = {
            let log = shared.clone();
            spawn(move || log.lock().unwrap().push("other"))
        };
        assert_eq!(poll_point(), Poll::Continue);
        shared.lock().unwrap().push("main");
        other.join().unwrap();
    });
    assert_eq!(*log.lock().unwrap(), ["main", "other"]);
}

#[test]
fn preempt_cancel_marks_the_task_sets_its_poll_flag_and_poisons_its_stack_limit() {
    let seen = Arc::new(Mutex::new(None));
    let slot = seen.clone();
    run(quick(2), move || {
        let task = spawn(move || {
            let woke = park();
            let polled = poll_point();
            let trap = stack_trap(0x1000);
            *slot.lock().unwrap() = Some((woke, polled, trap, current().is_cancelled()));
        });
        wait_until_parked(&task);
        assert_eq!((word(&task, CONTEXT_POLL), word(&task, CONTEXT_STACK_LIMIT) == u64::MAX), (0, false));
        task.cancel();
        assert_eq!((word(&task, CONTEXT_POLL), word(&task, CONTEXT_STACK_LIMIT)), (1, u64::MAX));
        assert!(task.is_cancelled());
        task.join().unwrap();
    });
    assert_eq!(*seen.lock().unwrap(), Some((Wake::Cancelled, Poll::Cancelled, Trap::Cancelled, true)));
}

#[test]
fn preempt_a_stack_trap_tells_an_overflow_from_a_spurious_entry() {
    let traps = run(quick(1), || {
        let (base, top) = current().stack_bounds().unwrap();
        (stack_trap(base + 16), stack_trap(base + RED_ZONE - 1), stack_trap(top - 256))
    });
    assert_eq!(traps, (Trap::Overflow, Trap::Overflow, Trap::Spurious));
}

#[test]
fn preempt_cancelling_a_sleeping_task_ends_its_sleep() {
    let (woke, elapsed) = run(quick(2), || {
        let result = Arc::new(Mutex::new(None));
        let slot = result.clone();
        let sleeper = spawn(move || *slot.lock().unwrap() = Some(sleep(60_000 * MS)));
        wait_until_parked(&sleeper);
        let start = Instant::now();
        sleeper.cancel();
        sleeper.join().unwrap();
        let woke = result.lock().unwrap().take().unwrap();
        (woke, start.elapsed())
    });
    assert_eq!(woke, Wake::Cancelled);
    assert!(elapsed < Duration::from_secs(5));
}

#[test]
fn preempt_a_cleaning_task_is_not_cancelled_again_and_has_its_stack_limit_back() {
    let seen = run(quick(1), || {
        let me = current();
        let limit = word(&me, CONTEXT_STACK_LIMIT);
        me.cancel();
        let cancelled = (poll_point(), word(&me, CONTEXT_STACK_LIMIT));
        crate::cleanup_begin();
        me.cancel();
        let cleaning = (poll_point(), word(&me, CONTEXT_STACK_LIMIT) == limit, me.is_cancelled(), word(&me, CONTEXT_POLL));
        (cancelled, cleaning, sleep(MS))
    });
    assert_eq!(seen, ((Poll::Cancelled, u64::MAX), (Poll::Continue, true, false, 0), Wake::TimedOut));
}

#[test]
fn preempt_a_cancelled_task_keeps_getting_cancelled_at_every_poll() {
    let polls = run(quick(1), || {
        let me = current();
        me.cancel();
        (poll_point(), poll_point(), park())
    });
    assert_eq!(polls, (Poll::Cancelled, Poll::Cancelled, Wake::Cancelled));
}
