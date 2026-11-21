use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{quick, run};
use crate::{current, foreign_source, park, spawn, wait_fd, wait_readable, wait_writable, yield_now, FdError, Interest, TaskState, Wake};

const MS: u64 = 1_000_000;

fn wait_until_parked(task: &crate::TaskHandle) {
    while task.state() != TaskState::Parked {
        yield_now();
    }
}

#[test]
fn poller_a_task_parks_until_its_descriptor_is_readable() {
    let result = Arc::new(Mutex::new(None));
    let (slot, probe) = (result.clone(), result.clone());
    run(quick(2), move || {
        let (mut left, mut right) = UnixStream::pair().unwrap();
        let fd = right.as_raw_fd();
        let reader = spawn(move || {
            let wake = wait_readable(fd).unwrap();
            let mut byte = [0u8];
            right.read_exact(&mut byte).unwrap();
            *slot.lock().unwrap() = Some((wake, byte[0]));
        });
        wait_until_parked(&reader);
        assert!(probe.lock().unwrap().is_none());
        left.write_all(b"z").unwrap();
        reader.join().unwrap();
    });
    assert_eq!(*result.lock().unwrap(), Some((Wake::Woken, b'z')));
}

#[test]
fn poller_a_writable_descriptor_does_not_park_for_long() {
    let wake = run(quick(1), || {
        let (left, _right) = UnixStream::pair().unwrap();
        wait_writable(left.as_raw_fd()).unwrap()
    });
    assert_eq!(wake, Wake::Woken);
}

#[test]
fn poller_a_wait_with_a_timeout_ends_at_the_timeout() {
    let (wake, elapsed) = run(quick(1), || {
        let (_left, right) = UnixStream::pair().unwrap();
        let start = Instant::now();
        (wait_fd(right.as_raw_fd(), Interest::Readable, Some(20 * MS)).unwrap(), start.elapsed())
    });
    assert_eq!(wake, Wake::TimedOut);
    assert!(elapsed >= Duration::from_millis(20) && elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn poller_a_thread_outside_the_scheduler_writes_and_the_task_wakes() {
    let wake = run(quick(2), || {
        let (mut left, right) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            left.write_all(b"x").unwrap();
            left
        });
        let wake = wait_readable(right.as_raw_fd()).unwrap();
        let _left = writer.join().unwrap();
        wake
    });
    assert_eq!(wake, Wake::Woken);
}

#[test]
fn poller_a_second_task_for_the_same_thing_is_refused_and_a_closed_peer_wakes() {
    run(quick(2), || {
        let (left, right) = UnixStream::pair().unwrap();
        let fd = right.as_raw_fd();
        let first = spawn(move || {
            wait_readable(fd).unwrap();
        });
        wait_until_parked(&first);
        assert!(matches!(wait_readable(fd), Err(FdError::Busy)));
        drop(left);
        first.join().unwrap();
    });
}

#[test]
fn poller_a_bad_descriptor_is_an_error() {
    let error = run(quick(1), || wait_readable(-1));
    assert!(matches!(error, Err(FdError::Sys(9))), "{error:?}");
}

#[test]
fn poller_a_cancelled_task_leaves_its_descriptor_wait() {
    run(quick(2), || {
        let (_left, right) = UnixStream::pair().unwrap();
        let fd = right.as_raw_fd();
        let waiter = spawn(move || {
            assert!(matches!(wait_readable(fd), Ok(Wake::Cancelled)));
        });
        wait_until_parked(&waiter);
        waiter.cancel();
        waiter.join().unwrap();
        let again = spawn(move || {
            assert!(matches!(wait_fd(fd, Interest::Readable, Some(MS)), Ok(Wake::TimedOut)));
        });
        again.join().unwrap();
    });
}

#[test]
fn poller_a_parked_port_receiver_is_not_a_deadlock_while_a_foreign_source_lives() {
    let woke = run(quick(2), || {
        let me = current();
        let source = foreign_source();
        let native = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            me.unpark();
        });
        let wake = park();
        drop(source);
        native.join().unwrap();
        wake
    });
    assert_eq!(woke, Wake::Woken);
}

#[test]
fn poller_many_tasks_wait_on_many_descriptors() {
    let total = run(quick(4), || {
        let pairs: Vec<_> = (0..40).map(|_| UnixStream::pair().unwrap()).collect();
        let waiters: Vec<_> = pairs
            .iter()
            .map(|(_, right)| {
                let fd = right.as_raw_fd();
                spawn(move || {
                    wait_readable(fd).unwrap();
                })
            })
            .collect();
        for waiter in &waiters {
            wait_until_parked(waiter);
        }
        for (mut left, _) in pairs.iter().map(|(left, right)| (left.try_clone().unwrap(), right)) {
            left.write_all(b"x").unwrap();
        }
        waiters.iter().for_each(|waiter| waiter.join().unwrap());
        waiters.len()
    });
    assert_eq!(total, 40);
}
