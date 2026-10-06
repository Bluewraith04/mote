//! Conformance for [`contracts::SafepointCoordinator`]; every implementation must pass `suite`.

use contracts::{PausedRoot, SafepointCoordinator};
use runtime::safepoint::StopTheWorld;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

fn root(n: usize) -> PausedRoot {
    NonNull::new(n as *mut _).unwrap()
}

fn addrs(roots: Vec<PausedRoot>) -> Vec<usize> {
    let mut v: Vec<usize> = roots.iter().map(|r| r.as_ptr() as usize).collect();
    v.sort();
    v
}

fn suite(make: fn() -> Arc<dyn SafepointCoordinator>) {
    a_lone_mutator_collects_without_waiting(make());
    every_entered_mutator_reports_before_the_collection(make());
    a_leaving_mutator_is_not_waited_for(make());
    a_leaver_hands_over_roots_during_a_pause(make());
    a_second_claimant_loses_and_runs_nothing(make());
    enter_blocks_until_the_pause_ends(make());
    pauses_repeat_and_reports_do_not_carry_over(make());
}

fn a_lone_mutator_collects_without_waiting(c: Arc<dyn SafepointCoordinator>) {
    c.enter();
    assert!(!c.pause_pending());
    let mut ran = false;
    assert!(c.stop_the_world(&mut || ran = true));
    assert!(ran && !c.pause_pending());
    c.leave();
}

fn every_entered_mutator_reports_before_the_collection(c: Arc<dyn SafepointCoordinator>) {
    let workers = 3;
    let ready = Arc::new(Barrier::new(workers + 1));
    let stop = Arc::new(AtomicBool::new(false));
    let handles: Vec<_> = (0..workers)
        .map(|i| {
            let (c, ready, stop) = (c.clone(), ready.clone(), stop.clone());
            thread::spawn(move || {
                c.enter();
                ready.wait();
                while !stop.load(Ordering::SeqCst) {
                    if c.pause_pending() {
                        c.participate(&mut || vec![root(0x1000 * (i + 1))]);
                    }
                    thread::yield_now();
                }
                c.leave();
            })
        })
        .collect();
    c.enter();
    ready.wait();
    let mut seen = Vec::new();
    assert!(c.stop_the_world(&mut || seen = addrs(c.take_reported_roots())));
    assert_eq!(seen, vec![0x1000, 0x2000, 0x3000]);
    stop.store(true, Ordering::SeqCst);
    c.leave();
    handles.into_iter().for_each(|h| h.join().unwrap());
}

fn a_leaving_mutator_is_not_waited_for(c: Arc<dyn SafepointCoordinator>) {
    c.enter();
    let other = c.clone();
    let idle = thread::spawn(move || {
        other.enter();
        thread::sleep(Duration::from_millis(30));
        other.leave();
    });
    thread::sleep(Duration::from_millis(5));
    assert!(c.stop_the_world(&mut || {}));
    c.leave();
    idle.join().unwrap();
}

fn a_leaver_hands_over_roots_during_a_pause(c: Arc<dyn SafepointCoordinator>) {
    c.enter();
    let other = c.clone();
    let entered = Arc::new(Barrier::new(2));
    let e2 = entered.clone();
    let leaver = thread::spawn(move || {
        other.enter();
        e2.wait();
        while !other.pause_pending() {
            thread::yield_now();
        }
        other.leave_reporting(&mut || vec![root(0x77)]);
    });
    entered.wait();
    let mut seen = Vec::new();
    assert!(c.stop_the_world(&mut || seen = addrs(c.take_reported_roots())));
    assert_eq!(seen, vec![0x77]);
    c.leave();
    leaver.join().unwrap();
}

fn a_second_claimant_loses_and_runs_nothing(c: Arc<dyn SafepointCoordinator>) {
    c.enter();
    let (c2, inside) = (c.clone(), Arc::new(Barrier::new(2)));
    let i2 = inside.clone();
    let loser = thread::spawn(move || {
        i2.wait();
        let mut ran = false;
        let won = c2.stop_the_world(&mut || ran = true);
        (won, ran)
    });
    assert!(c.stop_the_world(&mut || {
        inside.wait();
        thread::sleep(Duration::from_millis(30));
    }));
    assert_eq!(loser.join().unwrap(), (false, false));
    c.leave();
}

fn enter_blocks_until_the_pause_ends(c: Arc<dyn SafepointCoordinator>) {
    c.enter();
    let entered = Arc::new(AtomicBool::new(false));
    let (c2, e2) = (c.clone(), entered.clone());
    let late = thread::spawn(move || {
        while !c2.pause_pending() {
            thread::yield_now();
        }
        c2.enter();
        e2.store(true, Ordering::SeqCst);
        c2.leave();
    });
    assert!(c.stop_the_world(&mut || {
        thread::sleep(Duration::from_millis(30));
        assert!(!entered.load(Ordering::SeqCst), "enter returned during the pause");
    }));
    c.leave();
    late.join().unwrap();
    assert!(entered.load(Ordering::SeqCst));
}

fn pauses_repeat_and_reports_do_not_carry_over(c: Arc<dyn SafepointCoordinator>) {
    let cycles = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(Barrier::new(2));
    let (c2, stop2, ready2) = (c.clone(), stop.clone(), ready.clone());
    let worker = thread::spawn(move || {
        c2.enter();
        ready2.wait();
        while !stop2.load(Ordering::SeqCst) {
            if c2.pause_pending() {
                c2.participate(&mut || vec![root(0x55)]);
            }
            thread::yield_now();
        }
        c2.leave();
    });
    c.enter();
    ready.wait();
    for _ in 0..50 {
        let mut seen = Vec::new();
        assert!(c.stop_the_world(&mut || seen = addrs(c.take_reported_roots())));
        assert_eq!(seen, vec![0x55], "each pause reports exactly once");
        cycles.fetch_add(1, Ordering::SeqCst);
    }
    stop.store(true, Ordering::SeqCst);
    c.leave();
    worker.join().unwrap();
    assert_eq!(cycles.load(Ordering::SeqCst), 50);
}

#[test]
fn stop_the_world_conforms() {
    suite(|| Arc::new(StopTheWorld::new()));
}
