use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::quick;
use crate::{current, foreign_source, park, park_on, sleep, spawn, spawn_named, yield_now, Config, Scheduler, TaskHandle};

const MS: u64 = 1_000_000;

fn stuck_program() {
    let receiver = spawn_named("receiver", || {
        park_on("a channel receive");
    });
    let _locker = spawn_named("locker", || {
        park_on("a mutex lock");
    });
    receiver.join().unwrap();
}

#[test]
fn deadlock_all_tasks_parked_with_nothing_to_wake_them_is_reported_with_what_each_waits_on() {
    let report = Scheduler::new(quick(2)).unwrap().run(stuck_program).unwrap_err();
    let rows: Vec<(&str, &str, &str)> = report.tasks.iter().map(|task| (task.name.as_str(), task.state, task.waiting.as_str())).collect();
    assert_eq!(rows, [("main", "parked", "join"), ("receiver", "parked", "a channel receive"), ("locker", "parked", "a mutex lock")]);
    assert_eq!(report.tasks.iter().map(|task| task.id).collect::<Vec<_>>(), [1, 2, 3]);
}

#[test]
fn deadlock_tasks_waiting_for_a_stack_are_reported() {
    for config in [Config { spawn_slots: 1, ..quick(2) }, Config { spawn_slots: 1, ..Config::seeded(5) }] {
        let report = Scheduler::new(config)
            .unwrap()
            .run(|| {
                let first = spawn_named("first", || {
                    park_on("a mutex lock");
                });
                let _second = spawn_named("second", || {
                    park_on("a mutex lock");
                });
                first.join().unwrap();
            })
            .unwrap_err();
        let waiting: Vec<&str> = report.tasks.iter().filter(|task| task.waiting == "a free stack").map(|task| task.name.as_str()).collect();
        assert_eq!((report.tasks.len(), waiting.len()), (3, 1), "{report}");
    }
}

#[test]
fn deadlock_the_report_prints_one_line_for_each_task() {
    let report = Scheduler::new(quick(1)).unwrap().run(stuck_program).unwrap_err();
    let text = report.to_string();
    assert!(text.starts_with("deadlock: 3 tasks wait"), "{text}");
    assert!(text.contains("task 2 `receiver` (parked): a channel receive"), "{text}");
    assert_eq!(text.lines().count(), 4);
}

#[test]
fn deadlock_is_found_with_one_worker_many_workers_and_a_seed() {
    for config in [quick(1), quick(4), Config::seeded(3)] {
        let report = Scheduler::new(config).unwrap().run(stuck_program).unwrap_err();
        assert_eq!(report.tasks.len(), 3);
    }
}

#[test]
fn deadlock_a_pending_timer_is_not_one() {
    Scheduler::new(quick(2))
        .unwrap()
        .run(|| {
            let sleeper = spawn(|| {
                park_on("a wait with a timeout");
            });
            let waker = sleeper.clone();
            let _timed = spawn(move || {
                sleep(40 * MS);
                waker.unpark();
            });
            sleeper.join().unwrap();
        })
        .unwrap();
}

#[test]
fn deadlock_a_foreign_source_is_not_one_until_it_goes_away_without_waking_anyone() {
    let report = Scheduler::new(quick(2))
        .unwrap()
        .run(|| {
            let source = foreign_source();
            let thread = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(40));
                drop(source);
            });
            park_on("a port receive");
            thread.join().unwrap();
        })
        .unwrap_err();
    assert_eq!(report.tasks.len(), 1);
    assert_eq!(report.tasks[0].waiting, "a port receive");
}

#[test]
fn deadlock_a_main_that_returns_while_other_tasks_are_parked_is_not_one() {
    let value = Scheduler::new(quick(2))
        .unwrap()
        .run(|| {
            for _ in 0..3 {
                spawn(|| {
                    park();
                });
            }
            yield_now();
            7
        })
        .unwrap();
    assert_eq!(value, 7);
}

#[test]
fn deadlock_a_panicking_task_does_not_hide_one() {
    let report = Scheduler::new(quick(2))
        .unwrap()
        .run(|| {
            let _bad = spawn(|| panic!("expected panic"));
            park_on("waiting for the bad task");
        })
        .unwrap_err();
    assert_eq!(report.tasks.len(), 1);
}

#[test]
fn deadlock_ping_pong_between_tasks_never_looks_like_one() {
    for round in 0..40 {
        let turn = Arc::new(AtomicUsize::new(0));
        let handles: Arc<Mutex<Vec<TaskHandle>>> = Arc::default();
        let (turn_a, handles_a) = (turn.clone(), handles.clone());
        let finished = Scheduler::new(quick(1 + round % 4))
            .unwrap()
            .run(move || {
                let players: Vec<_> = (0..2usize)
                    .map(|me| {
                        let (turn, handles) = (turn_a.clone(), handles_a.clone());
                        spawn(move || {
                            for _ in 0..100 {
                                while turn.load(Ordering::SeqCst) != me {
                                    park_on("its turn");
                                }
                                turn.store(1 - me, Ordering::SeqCst);
                                let other = handles.lock().unwrap().get(1 - me).cloned();
                                if let Some(other) = other {
                                    other.unpark();
                                }
                            }
                        })
                    })
                    .collect();
                handles_a.lock().unwrap().extend(players.iter().cloned());
                players.iter().for_each(|player| player.unpark());
                players.iter().for_each(|player| player.join().unwrap());
                current().id()
            })
            .expect("ping-pong ends");
        assert_eq!(finished, 1);
    }
}
