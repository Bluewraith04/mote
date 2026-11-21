use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::run;
use crate::{poll_point, sleep, spawn, Config, Scheduler, TraceEvent};

const MS: u64 = 1_000_000;

/// Five tasks that poll and sleep, and the order in which their steps ran.
fn program(order: Arc<Mutex<Vec<(u64, u64)>>>) -> impl FnOnce() + Send + 'static {
    move || {
        let tasks: Vec<_> = (0..5u64)
            .map(|id| {
                let order = order.clone();
                spawn(move || {
                    for step in 0..6u64 {
                        poll_point();
                        order.lock().unwrap().push((id, step));
                        if (id + step) % 3 == 0 {
                            sleep((id + 1) * MS);
                        }
                    }
                })
            })
            .collect();
        tasks.iter().for_each(|task| task.join().unwrap());
    }
}

fn replay(seed: u64) -> (Vec<TraceEvent>, Vec<(u64, u64)>) {
    let order = Arc::new(Mutex::new(Vec::new()));
    let scheduler = Scheduler::new(Config::seeded(seed)).unwrap();
    scheduler.run(program(order.clone())).unwrap();
    let steps = order.lock().unwrap().clone();
    (scheduler.trace(), steps)
}

#[test]
fn seeded_one_seed_gives_one_schedule_every_time() {
    for seed in [0, 1, 7, 12345] {
        let first = replay(seed);
        for _ in 0..4 {
            assert_eq!(replay(seed), first, "seed {seed}");
        }
    }
}

#[test]
fn seeded_different_seeds_give_different_schedules() {
    let traces: HashSet<Vec<TraceEvent>> = (0..12).map(|seed| replay(seed).0).collect();
    assert!(traces.len() >= 6, "{} distinct schedules from 12 seeds", traces.len());
}

#[test]
fn seeded_the_next_ready_task_is_chosen_by_the_seed_not_in_spawn_order() {
    let first_runs: HashSet<Vec<u64>> = (0..20)
        .map(|seed| {
            replay(seed)
                .0
                .iter()
                .filter_map(|event| match event {
                    TraceEvent::Run(id) if *id != 1 => Some(*id),
                    _ => None,
                })
                .take(5)
                .collect()
        })
        .collect();
    assert!(first_runs.len() > 1, "{first_runs:?}");
}

#[test]
fn seeded_the_preemption_counter_yields_a_task_at_its_poll_points() {
    let (trace, _) = replay(5);
    let yields = trace.iter().filter(|event| matches!(event, TraceEvent::Yield(_))).count();
    assert!(yields > 0, "{trace:?}");
}

#[test]
fn seeded_the_virtual_clock_jumps_forward_between_events() {
    let (trace, _) = replay(9);
    let jumps: Vec<u64> = trace.iter().filter_map(|event| if let TraceEvent::Jump(ns) = event { Some(*ns) } else { None }).collect();
    assert!(!jumps.is_empty());
    assert!(jumps.windows(2).all(|pair| pair[0] < pair[1]), "{jumps:?}");
}

#[test]
fn seeded_a_schedule_starts_with_the_main_task_and_ends_when_it_is_done() {
    let (trace, _) = replay(3);
    assert_eq!(trace[..2], [TraceEvent::Spawn(1), TraceEvent::Run(1)]);
    assert_eq!(trace.last(), Some(&TraceEvent::Done(1)));
    assert_eq!(trace.iter().filter(|event| matches!(event, TraceEvent::Spawn(_))).count(), 6);
    assert_eq!(trace.iter().filter(|event| matches!(event, TraceEvent::Done(_))).count(), 6);
}

#[test]
fn seeded_every_schedule_runs_every_step_once() {
    for seed in 0..10 {
        let (_, mut steps) = replay(seed);
        steps.sort();
        assert_eq!(steps, (0..5u64).flat_map(|id| (0..6u64).map(move |step| (id, step))).collect::<Vec<_>>(), "seed {seed}");
    }
}

#[test]
fn seeded_a_task_starts_with_its_poll_flag_set_and_every_poll_sets_it_again() {
    let (before, after) = run(Config::seeded(2), || {
        let word = || unsafe { (*crate::current().context()).poll };
        let before = word();
        poll_point();
        (before, word())
    });
    assert_eq!((before, after), (1, 1));
}

#[test]
fn seeded_a_run_gives_its_stacks_back_when_it_ends_though_a_task_was_abandoned() {
    let scheduler = Scheduler::new(Config { spawn_slots: 4, ..Config::seeded(1) }).unwrap();
    assert!(scheduler.stacks_mapped());
    scheduler.run(|| drop(spawn(|| crate::exit()).join())).unwrap();
    assert!(!scheduler.stacks_mapped());
}

#[test]
fn seeded_a_run_uses_one_worker_and_a_clock_that_starts_at_zero() {
    let scheduler = Scheduler::new(Config::seeded(4)).unwrap();
    assert_eq!(scheduler.now(), 0);
    let ran_on = scheduler.run(|| std::thread::current().name().map(str::to_string)).unwrap();
    assert_eq!(ran_on.as_deref(), Some("mote-worker-0"));
    assert_eq!(run(Config::seeded(4), || 1), 1);
}
