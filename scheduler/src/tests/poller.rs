use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{quick, run};
use crate::{current, foreign_source, park, spawn, wait_fd, wait_readable, wait_writable, yield_now, FdError, Interest, TaskState, Wake};

#[cfg(target_os = "linux")]
fn stream_pair() -> (std::os::unix::net::UnixStream, std::os::unix::net::UnixStream, i32, i32) {
    use std::os::fd::AsRawFd;
    let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
    let (fa, fb) = (a.as_raw_fd(), b.as_raw_fd());
    (a, b, fa, fb)
}

#[cfg(target_os = "windows")]
fn stream_pair() -> (std::net::TcpStream, std::net::TcpStream, i32, i32) {
    use std::net::{TcpListener, TcpStream};
    use std::os::windows::io::AsRawSocket;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let a = TcpStream::connect(addr).unwrap();
    let (b, _) = listener.accept().unwrap();
    let (fa, fb) = (a.as_raw_socket() as i32, b.as_raw_socket() as i32);
    (a, b, fa, fb)
}

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
        let (mut left, mut right, _, fd) = stream_pair();
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
        let (_left, _right, fd, _) = stream_pair();
        wait_writable(fd).unwrap()
    });
    assert_eq!(wake, Wake::Woken);
}

#[test]
fn poller_a_wait_with_a_timeout_ends_at_the_timeout() {
    let (wake, elapsed) = run(quick(1), || {
        let (_left, _right, _, fd) = stream_pair();
        let start = Instant::now();
        (wait_fd(fd, Interest::Readable, Some(20 * MS)).unwrap(), start.elapsed())
    });
    assert_eq!(wake, Wake::TimedOut);
    assert!(elapsed >= Duration::from_millis(20) && elapsed < Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn poller_a_thread_outside_the_scheduler_writes_and_the_task_wakes() {
    let wake = run(quick(2), || {
        let (mut left, _right, _, fd) = stream_pair();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            left.write_all(b"x").unwrap();
            left
        });
        let wake = wait_readable(fd).unwrap();
        let _left = writer.join().unwrap();
        wake
    });
    assert_eq!(wake, Wake::Woken);
}

#[test]
fn poller_a_second_task_for_the_same_thing_is_refused_and_a_closed_peer_wakes() {
    run(quick(2), || {
        let (left, _right, _, fd) = stream_pair();
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
        let (_left, _right, _, fd) = stream_pair();
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
        let pairs: Vec<_> = (0..40).map(|_| stream_pair()).collect();
        let waiters: Vec<_> = pairs
            .iter()
            .map(|(_, _, _, fd)| {
                let fd = *fd;
                spawn(move || {
                    wait_readable(fd).unwrap();
                })
            })
            .collect();
        for waiter in &waiters {
            wait_until_parked(waiter);
        }
        for (mut left, _, _, _) in pairs.iter().map(|(left, right, fa, fb)| (left.try_clone().unwrap(), right, *fa, *fb)) {
            left.write_all(b"x").unwrap();
        }
        waiters.iter().for_each(|waiter| waiter.join().unwrap());
        waiters.len()
    });
    assert_eq!(total, 40);
}
