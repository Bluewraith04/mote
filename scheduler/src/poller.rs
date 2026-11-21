//! The netpoller: one epoll instance that wakes the token of a waiter when its descriptor is ready, and an eventfd that interrupts the wait.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::sys::{self, EpollEvent, SysError, EPOLLERR, EPOLLHUP, EPOLLIN, EPOLLONESHOT, EPOLLOUT, EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};

const WAKE_DATA: u64 = u64::MAX;
const EEXIST: i32 = 17;
const ENOENT: i32 = 2;

/// What a waiter wants of a descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interest {
    Readable,
    Writable,
}

/// A second task waits for the same thing on the same descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

struct Waiters<T> {
    read: Option<T>,
    write: Option<T>,
    /// The descriptor is in the epoll set.
    armed: bool,
}

impl<T> Waiters<T> {
    fn events(&self) -> u32 {
        EPOLLONESHOT | if self.read.is_some() { EPOLLIN } else { 0 } | if self.write.is_some() { EPOLLOUT } else { 0 }
    }

    fn is_empty(&self) -> bool {
        self.read.is_none() && self.write.is_none()
    }
}

pub struct Poller<T> {
    epoll: i32,
    wake: i32,
    waiters: Mutex<HashMap<i32, Waiters<T>>>,
    count: AtomicUsize,
}

impl<T> Poller<T> {
    pub fn new() -> Result<Self, SysError> {
        let epoll = sys::epoll_create()?;
        let wake = sys::eventfd().inspect_err(|_| sys::close(epoll))?;
        sys::epoll_ctl(epoll, EPOLL_CTL_ADD, wake, Some(EpollEvent::new(EPOLLIN, WAKE_DATA))).inspect_err(|_| {
            sys::close(wake);
            sys::close(epoll);
        })?;
        Ok(Self { epoll, wake, waiters: Mutex::new(HashMap::new()), count: AtomicUsize::new(0) })
    }

    /// Tasks waiting on a descriptor now.
    pub fn waiting(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn arm(&self, fd: i32, entry: &mut Waiters<T>) {
        let event = Some(EpollEvent::new(entry.events(), fd as u64));
        let operation = if entry.armed { EPOLL_CTL_MOD } else { EPOLL_CTL_ADD };
        let result = match sys::epoll_ctl(self.epoll, operation, fd, event) {
            Err(SysError(ENOENT)) => sys::epoll_ctl(self.epoll, EPOLL_CTL_ADD, fd, event),
            Err(SysError(EEXIST)) => sys::epoll_ctl(self.epoll, EPOLL_CTL_MOD, fd, event),
            other => other,
        };
        entry.armed = result.is_ok();
    }

    /// Makes `token` the waiter for the descriptor; a bad descriptor is reported as its error, a taken place as `Busy`.
    pub fn register(&self, fd: i32, interest: Interest, token: T) -> Result<Result<(), Busy>, SysError> {
        let mut waiters = self.waiters.lock().expect("the poller");
        let entry = waiters.entry(fd).or_insert(Waiters { read: None, write: None, armed: false });
        let place = if interest == Interest::Readable { &mut entry.read } else { &mut entry.write };
        if place.is_some() {
            return Ok(Err(Busy));
        }
        *place = Some(token);
        self.arm(fd, entry);
        if !entry.armed {
            let place = if interest == Interest::Readable { &mut entry.read } else { &mut entry.write };
            *place = None;
            if entry.is_empty() {
                waiters.remove(&fd);
            }
            return Err(SysError(9));
        }
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(Ok(()))
    }

    /// Takes the waiter back; its token, or `None` when it already woke.
    pub fn cancel(&self, fd: i32, interest: Interest) -> Option<T> {
        let mut waiters = self.waiters.lock().expect("the poller");
        let entry = waiters.get_mut(&fd)?;
        let token = if interest == Interest::Readable { entry.read.take() } else { entry.write.take() }?;
        self.count.fetch_sub(1, Ordering::SeqCst);
        if entry.is_empty() {
            if entry.armed {
                let _ = sys::epoll_ctl(self.epoll, EPOLL_CTL_DEL, fd, None);
            }
            waiters.remove(&fd);
        } else {
            self.arm(fd, entry);
        }
        Some(token)
    }

    /// Interrupts a wait in progress or the next one.
    pub fn wake(&self) {
        sys::eventfd_write(self.wake, 1);
    }

    /// Waits up to `timeout_ms` (negative for no limit) and appends the tokens of the waiters whose descriptors are ready; they stay counted in `waiting` until `settle`.
    pub fn poll(&self, timeout_ms: i32, ready: &mut Vec<T>) {
        let mut events = [EpollEvent::new(0, 0); 64];
        let count = sys::epoll_wait(self.epoll, &mut events, timeout_ms).unwrap_or(0);
        let mut waiters = self.waiters.lock().expect("the poller");
        for event in &events[..count] {
            let (bits, data) = (event.events, event.data);
            if data == WAKE_DATA {
                sys::eventfd_read(self.wake);
                continue;
            }
            let fd = data as i32;
            let Some(entry) = waiters.get_mut(&fd) else { continue };
            let failed = bits & (EPOLLERR | EPOLLHUP) != 0;
            if bits & EPOLLIN != 0 || failed {
                ready.extend(entry.read.take());
            }
            if bits & EPOLLOUT != 0 || failed {
                ready.extend(entry.write.take());
            }
            if entry.is_empty() {
                waiters.remove(&fd);
            } else {
                self.arm(fd, waiters.get_mut(&fd).expect("still there"));
            }
        }
    }

    /// Stops counting `woken` waiters whose tokens `poll` returned; the caller does this after waking them, so no moment shows them as neither waiting nor awake.
    pub fn settle(&self, woken: usize) {
        self.count.fetch_sub(woken, Ordering::SeqCst);
    }
}

impl<T> Drop for Poller<T> {
    fn drop(&mut self) {
        sys::close(self.wake);
        sys::close(self.epoll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    fn pair() -> (UnixStream, UnixStream) {
        UnixStream::pair().unwrap()
    }

    #[test]
    fn poller_a_readable_descriptor_wakes_its_waiter() {
        let poller = Poller::new().unwrap();
        let (mut left, right) = pair();
        assert_eq!(poller.register(right.as_raw_fd(), Interest::Readable, 7), Ok(Ok(())));
        assert_eq!(poller.waiting(), 1);
        let mut ready = Vec::new();
        poller.poll(10, &mut ready);
        assert!(ready.is_empty());
        left.write_all(b"x").unwrap();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [7]);
        assert_eq!(poller.waiting(), 1, "a woken waiter counts until it is settled");
        poller.settle(ready.len());
        assert_eq!(poller.waiting(), 0);
    }

    #[test]
    fn poller_a_waiter_wakes_once_and_registers_again_for_the_next_wake() {
        let poller = Poller::new().unwrap();
        let (mut left, mut right) = pair();
        let fd = right.as_raw_fd();
        for round in 0..3 {
            assert_eq!(poller.register(fd, Interest::Readable, round), Ok(Ok(())));
            left.write_all(b"x").unwrap();
            let mut ready = Vec::new();
            poller.poll(1000, &mut ready);
            assert_eq!(ready, [round]);
            let mut byte = [0u8];
            right.read_exact(&mut byte).unwrap();
        }
    }

    #[test]
    fn poller_a_writable_descriptor_wakes_at_once() {
        let poller = Poller::new().unwrap();
        let (left, _right) = pair();
        assert_eq!(poller.register(left.as_raw_fd(), Interest::Writable, 1), Ok(Ok(())));
        let mut ready = Vec::new();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [1]);
    }

    #[test]
    fn poller_a_reader_and_a_writer_of_one_descriptor_wake_apart() {
        let poller = Poller::new().unwrap();
        let (mut left, right) = pair();
        let fd = right.as_raw_fd();
        assert_eq!(poller.register(fd, Interest::Readable, 1), Ok(Ok(())));
        assert_eq!(poller.register(fd, Interest::Writable, 2), Ok(Ok(())));
        let mut ready = Vec::new();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [2]);
        poller.settle(1);
        assert_eq!(poller.waiting(), 1);
        left.write_all(b"x").unwrap();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [2, 1]);
    }

    #[test]
    fn poller_a_second_waiter_for_the_same_thing_is_refused() {
        let poller = Poller::new().unwrap();
        let (_left, right) = pair();
        assert_eq!(poller.register(right.as_raw_fd(), Interest::Readable, 1), Ok(Ok(())));
        assert_eq!(poller.register(right.as_raw_fd(), Interest::Readable, 2), Ok(Err(Busy)));
        assert_eq!(poller.waiting(), 1);
    }

    #[test]
    fn poller_a_closed_peer_wakes_the_waiters() {
        let poller = Poller::new().unwrap();
        let (left, right) = pair();
        assert_eq!(poller.register(right.as_raw_fd(), Interest::Readable, 1), Ok(Ok(())));
        drop(left);
        let mut ready = Vec::new();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [1]);
    }

    #[test]
    fn poller_a_cancelled_waiter_does_not_wake() {
        let poller = Poller::new().unwrap();
        let (mut left, right) = pair();
        let fd = right.as_raw_fd();
        assert_eq!(poller.register(fd, Interest::Readable, 5), Ok(Ok(())));
        assert_eq!(poller.cancel(fd, Interest::Readable), Some(5));
        assert_eq!(poller.cancel(fd, Interest::Readable), None);
        assert_eq!(poller.waiting(), 0);
        left.write_all(b"x").unwrap();
        let mut ready = Vec::new();
        poller.poll(20, &mut ready);
        assert!(ready.is_empty());
    }

    #[test]
    fn poller_a_bad_descriptor_is_an_error() {
        let poller: Poller<u32> = Poller::new().unwrap();
        assert_eq!(poller.register(-1, Interest::Readable, 1), Err(SysError(9)));
        assert_eq!(poller.waiting(), 0);
    }

    #[test]
    fn poller_the_wait_times_out_and_a_wake_interrupts_it() {
        let poller: Poller<u32> = Poller::new().unwrap();
        let mut ready = Vec::new();
        let start = Instant::now();
        poller.poll(30, &mut ready);
        assert!(start.elapsed() >= Duration::from_millis(25) && ready.is_empty());
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(30));
                poller.wake();
            });
            let start = Instant::now();
            poller.poll(10_000, &mut ready);
            assert!(start.elapsed() < Duration::from_secs(5));
        });
        poller.wake();
        let start = Instant::now();
        poller.poll(10_000, &mut ready);
        assert!(start.elapsed() < Duration::from_secs(5), "a wake before the wait is not lost");
    }
}
