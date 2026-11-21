use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{quick, run};
use crate::{current, foreign_source, on_os_stack, park, spawn, spawn_named, yield_now, Config, Scheduler, TaskState, Wake};

#[test]
fn queues_main_runs_as_a_task_and_its_value_is_returned() {
    assert_eq!(run(quick(2), || 6 * 7), 42);
    assert!(run(quick(1), crate::in_a_task));
    assert!(!crate::in_a_task());
}

#[test]
#[should_panic(expected = "boom in main")]
fn queues_a_panic_in_main_is_raised_by_run() {
    run(quick(2), || panic!("boom in main"));
}

#[test]
fn queues_every_spawned_task_runs_exactly_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let spawned = run(quick(4), move || {
        let mut handles = Vec::new();
        for _ in 0..200 {
            let count = seen.clone();
            handles.push(spawn(move || {
                count.fetch_add(1, Ordering::SeqCst);
            }));
        }
        for handle in &handles {
            handle.join().unwrap();
        }
        handles.len()
    });
    assert_eq!((spawned, count.load(Ordering::SeqCst)), (200, 200));
}

#[test]
fn queues_one_worker_runs_ready_tasks_in_order_and_a_yield_goes_to_the_back() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = log.clone();
    run(quick(1), move || {
        let make = |name: &'static str| {
            let log = shared.clone();
            spawn(move || {
                log.lock().unwrap().push(format!("{name}1"));
                yield_now();
                log.lock().unwrap().push(format!("{name}2"));
            })
        };
        let (a, b) = (make("a"), make("b"));
        a.join().unwrap();
        b.join().unwrap();
    });
    assert_eq!(*log.lock().unwrap(), ["a1", "b1", "a2", "b2"]);
}

#[test]
fn queues_idle_workers_steal_ready_tasks() {
    let threads = Arc::new(Mutex::new(HashSet::new()));
    let seen = threads.clone();
    let scheduler = Scheduler::new(quick(4)).unwrap();
    scheduler
        .run(move || {
            let handles: Vec<_> = (0..64)
                .map(|_| {
                    let threads = seen.clone();
                    spawn(move || {
                        let until = Instant::now() + Duration::from_millis(3);
                        while Instant::now() < until {
                            std::hint::spin_loop();
                        }
                        threads.lock().unwrap().insert(std::thread::current().name().unwrap().to_string());
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap();
            }
        })
        .unwrap();
    assert!(threads.lock().unwrap().len() > 1, "{:?}", threads.lock().unwrap());
    assert!(scheduler.stats().steals > 0);
}

#[test]
fn queues_a_task_woken_by_a_task_that_parks_next_stays_on_its_worker() {
    const TURNS: usize = 2000;
    let scheduler = Scheduler::new(quick(4)).unwrap();
    scheduler
        .run(|| {
            let main = current();
            let turn = Arc::new(AtomicUsize::new(0));
            let theirs = turn.clone();
            let partner = spawn(move || {
                for _ in 0..TURNS {
                    while theirs.load(Ordering::SeqCst) != 1 {
                        park();
                    }
                    theirs.store(0, Ordering::SeqCst);
                    main.unpark();
                }
            });
            for _ in 0..TURNS {
                turn.store(1, Ordering::SeqCst);
                partner.unpark();
                while turn.load(Ordering::SeqCst) != 0 {
                    park();
                }
            }
            partner.join().unwrap();
        })
        .unwrap();
    let steals = scheduler.stats().steals;
    assert!(steals < TURNS / 10, "{steals} steals in {TURNS} turns");
}

#[test]
fn queues_a_task_woken_by_a_task_that_keeps_running_is_stolen_by_an_idle_worker() {
    let config = Config { slice: Duration::from_millis(5), ..quick(4) };
    run(config, || {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let sleeper = spawn(move || {
            park();
            flag.store(true, Ordering::SeqCst);
        });
        while sleeper.state() != TaskState::Parked {
            std::thread::sleep(Duration::from_millis(1));
        }
        sleeper.unpark();
        let until = Instant::now() + Duration::from_secs(5);
        while !ran.load(Ordering::SeqCst) {
            assert!(Instant::now() < until, "the woken task never ran beside the running one");
            std::hint::spin_loop();
        }
    });
}

#[test]
fn queues_a_parked_task_waits_for_an_unpark_and_runs_on() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = log.clone();
    run(quick(2), move || {
        let waiter = {
            let log = shared.clone();
            spawn(move || {
                log.lock().unwrap().push("parking");
                assert_eq!(park(), Wake::Woken);
                log.lock().unwrap().push("woken");
            })
        };
        while waiter.state() != TaskState::Parked {
            yield_now();
        }
        shared.lock().unwrap().push("unparking");
        waiter.unpark();
        waiter.join().unwrap();
    });
    assert_eq!(*log.lock().unwrap(), ["parking", "unparking", "woken"]);
}

#[test]
fn queues_an_unpark_before_the_park_is_not_lost() {
    let woke = run(quick(2), || {
        current().unpark();
        park()
    });
    assert_eq!(woke, Wake::Woken);
}

#[test]
fn queues_a_parked_task_is_woken_from_a_thread_outside_the_scheduler() {
    let reached = Arc::new(AtomicBool::new(false));
    let seen = reached.clone();
    run(quick(2), move || {
        let me = current();
        let source = foreign_source();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            me.unpark();
        });
        assert_eq!(park(), Wake::Woken);
        drop(source);
        thread.join().unwrap();
        seen.store(true, Ordering::SeqCst);
    });
    assert!(reached.load(Ordering::SeqCst));
}

#[test]
fn queues_the_state_machine_goes_ready_running_parked_done() {
    let states = Arc::new(Mutex::new(Vec::new()));
    let shared = states.clone();
    run(quick(1), move || {
        let inner = shared.clone();
        let task = spawn(move || {
            inner.lock().unwrap().push(current().state());
            park();
        });
        shared.lock().unwrap().push(task.state());
        yield_now();
        shared.lock().unwrap().push(task.state());
        task.unpark();
        task.join().unwrap();
        shared.lock().unwrap().push(task.state());
    });
    assert_eq!(*states.lock().unwrap(), [TaskState::Ready, TaskState::Running, TaskState::Parked, TaskState::Done]);
}

#[test]
fn queues_join_gives_the_panic_message_of_a_task_that_panicked_and_the_worker_lives_on() {
    let outcome = run(quick(1), || {
        let bad = spawn_named("bad", || panic!("task went wrong"));
        let first = bad.join();
        let good = spawn(|| {});
        (first, good.join(), bad.name().to_string())
    });
    assert_eq!(outcome.0, Err("task went wrong".to_string()));
    assert_eq!(outcome.1, Ok(()));
    assert_eq!(outcome.2, "bad");
}

#[test]
fn queues_stack_slots_are_reused_as_tasks_end() {
    let scheduler = Scheduler::new(Config { spawn_slots: 4, ..quick(2) }).unwrap();
    scheduler
        .run(|| {
            for _ in 0..100 {
                spawn(|| {}).join().unwrap();
            }
        })
        .unwrap();
    assert_eq!(scheduler.stack_slots_in_use(), 0);
}

#[test]
fn queues_a_task_takes_its_stack_when_it_first_runs() {
    let scheduler = Arc::new(Scheduler::new(Config { spawn_slots: 2, ..quick(1) }).unwrap());
    let inside = scheduler.clone();
    let (queued, finished) = scheduler
        .run(move || {
            let tasks: Vec<_> = (0..50).map(|_| spawn(|| {})).collect();
            let queued = inside.stack_slots_in_use();
            tasks.iter().for_each(|task| task.join().unwrap());
            (queued, tasks.len())
        })
        .unwrap();
    assert_eq!((queued, finished), (0, 50));
}

#[test]
fn queues_unstarted_tasks_wait_for_a_slot() {
    let seen = run(Config { spawn_slots: 2, ..quick(2) }, || {
        let parked: Vec<_> = (0..2).map(|_| spawn(|| { park(); })).collect();
        for task in &parked {
            while task.state() != TaskState::Parked {
                yield_now();
            }
        }
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let waiting = spawn(move || flag.store(true, Ordering::SeqCst));
        for _ in 0..200 {
            yield_now();
        }
        let before = (ran.load(Ordering::SeqCst), waiting.state());
        parked[0].unpark();
        parked[0].join().unwrap();
        waiting.join().unwrap();
        parked[1].unpark();
        parked[1].join().unwrap();
        (before, ran.load(Ordering::SeqCst))
    });
    assert_eq!(seen, ((false, TaskState::Ready), true));
}

#[test]
fn queues_a_task_keeps_its_stack_when_it_moves_between_workers() {
    let sums = run(quick(4), || {
        let handles: Vec<_> = (0..16u64)
            .map(|seed| {
                spawn(move || {
                    let mut buffer = [0u64; 512];
                    for (at, word) in buffer.iter_mut().enumerate() {
                        *word = seed * 1000 + at as u64;
                    }
                    for _ in 0..50 {
                        yield_now();
                    }
                    assert_eq!(buffer.iter().sum::<u64>(), seed * 1000 * 512 + (0..512u64).sum::<u64>());
                })
            })
            .collect();
        handles.iter().for_each(|handle| handle.join().unwrap());
        handles.len()
    });
    assert_eq!(sums, 16);
}

#[test]
fn queues_thousands_of_parked_tasks_cost_only_their_touched_pages() {
    let woken = Arc::new(AtomicUsize::new(0));
    let seen = woken.clone();
    run(Config { spawn_slots: 4096, ..quick(4) }, move || {
        let tasks: Vec<_> = (0..4000)
            .map(|_| {
                let woken = seen.clone();
                spawn(move || {
                    park();
                    woken.fetch_add(1, Ordering::SeqCst);
                })
            })
            .collect();
        for task in &tasks {
            while task.state() != TaskState::Parked {
                yield_now();
            }
        }
        for task in &tasks {
            task.unpark();
        }
        for task in &tasks {
            task.join().unwrap();
        }
    });
    assert_eq!(woken.load(Ordering::SeqCst), 4000);
}

extern "C" fn record_stack_pointer(arg: usize) {
    let marker = 0u8;
    // SAFETY: the test passes the address of a live `AtomicUsize`.
    unsafe { &*(arg as *const AtomicUsize) }.store(std::hint::black_box(&marker) as *const u8 as usize, Ordering::SeqCst);
}

#[test]
fn queues_a_call_on_the_os_stack_runs_off_the_task_stack_and_returns() {
    let inside = Arc::new(AtomicUsize::new(0));
    let seen = inside.clone();
    let (bounds, after) = run(quick(1), move || {
        let me = current();
        on_os_stack(record_stack_pointer, Arc::as_ptr(&seen) as usize);
        let marker = 0u8;
        (me.stack_bounds().unwrap(), std::hint::black_box(&marker) as *const u8 as usize)
    });
    let at = inside.load(Ordering::SeqCst);
    assert!(at != 0 && !(bounds.0..bounds.1).contains(&at));
    assert!((bounds.0..bounds.1).contains(&after));
}

#[test]
fn queues_a_parked_task_reports_where_its_live_stack_is() {
    run(quick(1), || {
        let task = spawn(|| {
            park();
        });
        assert_eq!(task.saved_sp(), None);
        while task.state() != TaskState::Parked {
            yield_now();
        }
        let (base, top) = task.stack_bounds().unwrap();
        let sp = task.saved_sp().unwrap();
        assert!(base < sp && sp < top && top - sp < 64 * 1024);
        task.unpark();
        task.join().unwrap();
        assert_eq!(task.stack_bounds(), None);
    });
}
