use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{quick, run};
use crate::{current, now, park_timeout, sleep, spawn, yield_now, Config, Scheduler, TaskState, Wake};

const MS: u64 = 1_000_000;

#[test]
fn timers_a_task_sleeps_on_the_real_clock_while_others_run() {
    let (elapsed, other_ran) = run(quick(1), || {
        let ran = Arc::new(Mutex::new(false));
        let flag = ran.clone();
        let other = spawn(move || *flag.lock().unwrap() = true);
        let start = Instant::now();
        assert_eq!(sleep(30 * MS), Wake::TimedOut);
        other.join().unwrap();
        let ran = *ran.lock().unwrap();
        (start.elapsed(), ran)
    });
    assert!(elapsed >= Duration::from_millis(30) && elapsed < Duration::from_secs(2), "{elapsed:?}");
    assert!(other_ran);
}

#[test]
fn timers_a_timed_park_times_out_and_an_unpark_beats_the_timer() {
    let (timed_out, woken) = run(quick(2), || {
        let timed_out = park_timeout(10 * MS, "a test wait");
        let me = current();
        let helper = spawn(move || me.unpark());
        let woken = park_timeout(10_000 * MS, "a test wait");
        helper.join().unwrap();
        (timed_out, woken)
    });
    assert_eq!((timed_out, woken), (Wake::TimedOut, Wake::Woken));
}

#[test]
fn timers_sleepers_wake_in_deadline_order_whatever_the_spawn_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let shared = order.clone();
    run(quick(2), move || {
        let tasks: Vec<_> = [40u64, 10, 30, 20]
            .into_iter()
            .map(|ms| {
                let order = shared.clone();
                spawn(move || {
                    sleep(ms * MS);
                    order.lock().unwrap().push(ms);
                })
            })
            .collect();
        tasks.iter().for_each(|task| task.join().unwrap());
    });
    assert_eq!(*order.lock().unwrap(), [10, 20, 30, 40]);
}

#[test]
fn timers_a_timed_park_cancels_its_timer_when_woken_early() {
    let scheduler = Scheduler::new(quick(1)).unwrap();
    scheduler
        .run(|| {
            let me = current();
            let helper = spawn(move || me.unpark());
            assert_eq!(park_timeout(60_000 * MS, "a long wait"), Wake::Woken);
            helper.join().unwrap();
        })
        .unwrap();
    assert_eq!(scheduler.stats().timers_fired, 0);
}

#[test]
fn timers_a_virtual_clock_makes_an_hour_pass_at_once() {
    let scheduler = Scheduler::new(Config::seeded(1)).unwrap();
    let start = Instant::now();
    let slept = scheduler
        .run(|| {
            let before = now();
            sleep(3600 * 1_000_000_000);
            now() - before
        })
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!((3600 * 1_000_000_000..3600 * 1_000_000_000 + MS).contains(&slept), "{slept}");
    assert_eq!(scheduler.now(), slept);
}

#[test]
fn timers_a_virtual_clock_does_not_move_while_tasks_can_run() {
    let reads = run(Config::seeded(2), || {
        let first = now();
        for _ in 0..100 {
            yield_now();
        }
        (first, now())
    });
    assert_eq!(reads, (0, 0));
}

#[test]
fn timers_virtual_sleepers_wake_in_order_at_their_times() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = log.clone();
    run(Config::seeded(3), move || {
        let tasks: Vec<_> = [300u64, 100, 200]
            .into_iter()
            .map(|ms| {
                let log = shared.clone();
                spawn(move || {
                    sleep(ms * MS);
                    log.lock().unwrap().push((ms, now() / MS));
                })
            })
            .collect();
        tasks.iter().for_each(|task| task.join().unwrap());
    });
    assert_eq!(*log.lock().unwrap(), [(100, 100), (200, 200), (300, 300)]);
}

#[test]
fn timers_a_sleeping_task_shows_as_parked() {
    run(quick(1), || {
        let sleeper = spawn(|| {
            sleep(20 * MS);
        });
        while sleeper.state() != TaskState::Parked {
            yield_now();
        }
        sleeper.join().unwrap();
        assert!(sleeper.is_done());
    });
}

#[test]
fn timers_an_unpark_makes_a_sleep_resume_its_wait_until_the_deadline() {
    let elapsed = run(quick(2), || {
        let me = current();
        let start = Instant::now();
        let helper = spawn(move || me.unpark());
        let wake = sleep(25 * MS);
        helper.join().unwrap();
        park_timeout(1, "settle");
        (wake, start.elapsed())
    });
    assert_eq!(elapsed.0, Wake::TimedOut);
    assert!(elapsed.1 >= Duration::from_millis(25));
}
