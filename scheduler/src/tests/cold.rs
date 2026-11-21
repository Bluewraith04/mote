use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{quick, run};
use crate::sys::resident_pages;
use crate::{park, park_timeout, scavenge, spawn, yield_now, Config, Scheduler, TaskHandle, TaskState, Wake};

/// A run that treats every parked task as cold.
fn cold(workers: usize) -> Config {
    Config { cold_after: Some(Duration::ZERO), ..quick(workers) }
}

fn wait_until_parked(task: &TaskHandle) {
    while task.state() != TaskState::Parked {
        yield_now();
    }
}

fn resident(task: &TaskHandle) -> usize {
    let (base, top) = task.stack_bounds().expect("the task has a stack");
    resident_pages(base, top - base)
}

/// Scavenges until the task's resident pages fall to `at_most`, or five seconds pass; the pages left.
fn scavenge_until(task: &TaskHandle, at_most: usize) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    while resident(task) > at_most && Instant::now() < deadline {
        scavenge();
        yield_now();
    }
    resident(task)
}

/// Touches about `kib` KiB of stack and returns a checksum of it.
#[inline(never)]
fn dig(kib: usize) -> usize {
    let mut page = [0u8; 1024];
    page[0] = kib as u8;
    let page = black_box(page);
    if kib == 0 { page[0] as usize } else { dig(kib - 1) + page[0] as usize }
}

/// Parks at the bottom of `depth` frames that each use their data after the park.
#[inline(never)]
fn dive_and_park(depth: usize) -> usize {
    let mut page = [depth as u8; 1024];
    page[1] = 7;
    let page = black_box(page);
    let below = if depth == 0 {
        park();
        0
    } else {
        dive_and_park(depth - 1)
    };
    below + page[0] as usize + page[1] as usize
}

fn dive_value(depth: usize) -> usize {
    (0..=depth).map(|level| level as u8 as usize + 7).sum()
}

#[test]
fn queues_a_cold_parked_task_gives_back_its_pages_and_resumes_intact() {
    let (before, after, intact) = run(cold(2), || {
        let result = Arc::new(Mutex::new(None));
        let slot = result.clone();
        let task = spawn(move || {
            let warm = dig(200);
            let mut local = [0u64; 32];
            for (at, word) in local.iter_mut().enumerate() {
                *word = at as u64 * 3 + warm as u64;
            }
            park();
            *slot.lock().unwrap() = Some(local.iter().enumerate().all(|(at, word)| *word == at as u64 * 3 + warm as u64));
        });
        wait_until_parked(&task);
        let before = resident(&task);
        let after = scavenge_until(&task, 0);
        task.unpark();
        task.join().unwrap();
        let intact = *result.lock().unwrap();
        (before, after, intact)
    });
    assert!(before > 40, "the task touched {before} pages");
    assert_eq!((after, intact), (0, Some(true)));
}

#[test]
fn queues_a_cold_task_with_a_deep_stack_keeps_its_live_pages_and_drops_the_rest() {
    let (before, after, value) = run(cold(2), || {
        let result = Arc::new(Mutex::new(0));
        let slot = result.clone();
        let task = spawn(move || {
            dig(400);
            *slot.lock().unwrap() = dive_and_park(100);
        });
        wait_until_parked(&task);
        let before = resident(&task);
        let after = scavenge_until(&task, before - 1);
        task.unpark();
        task.join().unwrap();
        let value = *result.lock().unwrap();
        (before, after, value)
    });
    assert!(after < before && after >= 100 * 1024 / 4096, "{before} pages before, {after} after");
    assert_eq!(value, dive_value(100));
}

#[test]
fn queues_a_wake_during_compaction_is_not_lost() {
    let scheduler = Arc::new(Scheduler::new(cold(4)).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let hammer = {
        let (scheduler, stop) = (scheduler.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                scheduler.scavenge();
                std::thread::yield_now();
            }
        })
    };
    let total = scheduler
        .run(|| {
            let tasks: Vec<_> = (0..64u64)
                .map(|seed| {
                    spawn(move || {
                        let mut mine = [0u64; 16];
                        for (at, word) in mine.iter_mut().enumerate() {
                            *word = seed * 100 + at as u64;
                        }
                        for _ in 0..40 {
                            park_timeout(300_000, "a short wait");
                            assert_eq!(mine.iter().sum::<u64>(), seed * 100 * 16 + 120);
                        }
                    })
                })
                .collect();
            tasks.iter().for_each(|task| task.join().unwrap());
            tasks.len()
        })
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    hammer.join().unwrap();
    assert_eq!(total, 64);
    assert!(scheduler.stats().compacted > 0);
}

#[test]
fn queues_a_seeded_run_compacts_only_when_asked_and_resumes_intact() {
    assert_eq!(Config::seeded(1).cold_after, None);
    let run_one = |config: Config| {
        run(config, || {
            let result = Arc::new(Mutex::new(false));
            let slot = result.clone();
            let task = spawn(move || {
                let value = dive_and_park(10);
                *slot.lock().unwrap() = value == dive_value(10);
            });
            wait_until_parked(&task);
            let handled = scavenge() + scavenge();
            let pages = resident(&task);
            task.unpark();
            task.join().unwrap();
            let intact = *result.lock().unwrap();
            (handled, pages > 0, intact)
        })
    };
    assert_eq!(run_one(Config::seeded(1)), (0, true, true));
    assert_eq!(run_one(Config { cold_after: Some(Duration::ZERO), ..Config::seeded(1) }), (1, false, true));
}

#[test]
fn queues_a_cancelled_cold_task_resumes_and_ends() {
    run(cold(2), || {
        let task = spawn(|| assert_eq!(park(), Wake::Cancelled));
        wait_until_parked(&task);
        assert_eq!(scavenge_until(&task, 0), 0);
        task.cancel();
        task.join().unwrap();
    });
}
