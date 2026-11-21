//! The netpoller: OS readiness monitor (epoll on Linux, WSAPoll + loopback wakeup on Windows) that wakes the token of a waiter when its descriptor is ready.

#[cfg(test)]
use crate::sys::SysError;

/// What a waiter wants of a descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interest {
    Readable,
    Writable,
}

/// A second task waits for the same thing on the same descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use crate::sys::{self, EpollEvent, SysError, EPOLLERR, EPOLLHUP, EPOLLIN, EPOLLONESHOT, EPOLLOUT, EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};

    use super::{Busy, Interest};

    const WAKE_DATA: u64 = u64::MAX;
    const EEXIST: i32 = 17;
    const ENOENT: i32 = 2;

    struct Waiters<T> {
        read: Option<T>,
        write: Option<T>,
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

        pub fn wake(&self) {
            sys::eventfd_write(self.wake, 1);
        }

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
}

#[cfg(target_os = "windows")]
mod imp {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::windows::io::AsRawSocket;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Mutex, Once};
    use std::time::Instant;

    use crate::sys::SysError;

    use super::{Busy, Interest};

    const POLLRDNORM: i16 = 0x0100;
    const POLLWRNORM: i16 = 0x0010;
    const POLLERR: i16 = 0x0001;
    const POLLHUP: i16 = 0x0002;
    const POLLNVAL: i16 = 0x0004;
    const SOL_SOCKET: i32 = 0xffff;
    const SO_TYPE: i32 = 0x1008;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WsaPollFd {
        fd: usize,
        events: i16,
        revents: i16,
    }

    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAStartup(version: u16, data: *mut [u8; 512]) -> i32;
        fn WSAPoll(fds: *mut WsaPollFd, count: u32, timeout_ms: i32) -> i32;
        fn getsockopt(socket: usize, level: i32, optname: i32, optval: *mut i32, optlen: *mut i32) -> i32;
    }

    static WSA_INIT: Once = Once::new();

    fn ensure_wsa() {
        WSA_INIT.call_once(|| {
            let mut data = [0u8; 512];
            // SAFETY: requests Winsock 2.2 into a 512-byte buffer larger than `WSADATA`.
            unsafe { WSAStartup(0x0202, &mut data) };
        });
    }

    struct Waiters<T> {
        read: Option<T>,
        write: Option<T>,
    }

    impl<T> Waiters<T> {
        fn events(&self) -> i16 {
            let mut bits = 0i16;
            if self.read.is_some() {
                bits |= POLLRDNORM;
            }
            if self.write.is_some() {
                bits |= POLLWRNORM;
            }
            bits
        }

        fn is_empty(&self) -> bool {
            self.read.is_none() && self.write.is_none()
        }
    }

    struct PollGuard<'a>(&'a AtomicUsize);
    impl Drop for PollGuard<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub struct Poller<T> {
        wake_rx: Mutex<TcpStream>,
        wake_tx: Mutex<TcpStream>,
        wake_fd: usize,
        wake_pending: AtomicBool,
        explicit_wake: AtomicBool,
        polling: AtomicUsize,
        waiters: Mutex<HashMap<i32, Waiters<T>>>,
        count: AtomicUsize,
    }

    impl<T> Poller<T> {
        pub fn new() -> Result<Self, SysError> {
            ensure_wsa();
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| SysError(e.raw_os_error().unwrap_or(1)))?;
            let addr = listener.local_addr().map_err(|e| SysError(e.raw_os_error().unwrap_or(1)))?;
            let tx = TcpStream::connect(addr).map_err(|e| SysError(e.raw_os_error().unwrap_or(1)))?;
            let (rx, _) = listener.accept().map_err(|e| SysError(e.raw_os_error().unwrap_or(1)))?;
            let _ = rx.set_nonblocking(true);
            let _ = tx.set_nonblocking(true);
            let _ = tx.set_nodelay(true);
            let wake_fd = rx.as_raw_socket() as usize;
            Ok(Self {
                wake_rx: Mutex::new(rx),
                wake_tx: Mutex::new(tx),
                wake_fd,
                wake_pending: AtomicBool::new(false),
                explicit_wake: AtomicBool::new(false),
                polling: AtomicUsize::new(0),
                waiters: Mutex::new(HashMap::new()),
                count: AtomicUsize::new(0),
            })
        }

        pub fn waiting(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }

        fn is_valid_socket(fd: i32) -> bool {
            if fd < 0 {
                return false;
            }
            let mut kind = 0i32;
            let mut len = std::mem::size_of::<i32>() as i32;
            // SAFETY: checks whether `fd` is an open Winsock socket.
            unsafe { getsockopt(fd as usize, SOL_SOCKET, SO_TYPE, &mut kind, &mut len) == 0 }
        }

        fn send_wake_byte(&self) {
            let mut tx = self.wake_tx.lock().expect("wake_tx");
            if !self.wake_pending.load(Ordering::SeqCst) {
                let _ = tx.write(&[1]);
                self.wake_pending.store(true, Ordering::SeqCst);
            }
        }

        fn drain_wake(&self) {
            let _tx = self.wake_tx.lock().expect("wake_tx");
            let mut rx = self.wake_rx.lock().expect("wake_rx");
            let mut buf = [0u8; 64];
            loop {
                match rx.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => continue,
                }
            }
            self.wake_pending.store(false, Ordering::SeqCst);
        }

        pub fn register(&self, fd: i32, interest: Interest, token: T) -> Result<Result<(), Busy>, SysError> {
            if !Self::is_valid_socket(fd) {
                return Err(SysError(9));
            }
            let mut waiters = self.waiters.lock().expect("the poller");
            let entry = waiters.entry(fd).or_insert(Waiters { read: None, write: None });
            let place = if interest == Interest::Readable { &mut entry.read } else { &mut entry.write };
            if place.is_some() {
                return Ok(Err(Busy));
            }
            *place = Some(token);
            self.count.fetch_add(1, Ordering::SeqCst);
            drop(waiters);
            if self.polling.load(Ordering::SeqCst) > 0 {
                self.send_wake_byte();
            }
            Ok(Ok(()))
        }

        pub fn cancel(&self, fd: i32, interest: Interest) -> Option<T> {
            let mut waiters = self.waiters.lock().expect("the poller");
            let entry = waiters.get_mut(&fd)?;
            let token = if interest == Interest::Readable { entry.read.take() } else { entry.write.take() }?;
            self.count.fetch_sub(1, Ordering::SeqCst);
            if entry.is_empty() {
                waiters.remove(&fd);
            }
            drop(waiters);
            if self.polling.load(Ordering::SeqCst) > 0 {
                self.send_wake_byte();
            }
            Some(token)
        }

        pub fn wake(&self) {
            self.explicit_wake.store(true, Ordering::SeqCst);
            self.send_wake_byte();
        }

        pub fn poll(&self, timeout_ms: i32, ready: &mut Vec<T>) {
            self.polling.fetch_add(1, Ordering::SeqCst);
            let _guard = PollGuard(&self.polling);
            let started = Instant::now();
            loop {
                let mut fds = {
                    let waiters = self.waiters.lock().expect("the poller");
                    let mut list = Vec::with_capacity(waiters.len() + 1);
                    list.push(WsaPollFd { fd: self.wake_fd, events: POLLRDNORM, revents: 0 });
                    for (&fd, entry) in waiters.iter() {
                        let events = entry.events();
                        if events != 0 {
                            list.push(WsaPollFd { fd: fd as usize, events, revents: 0 });
                        }
                    }
                    list
                };
                let left = if timeout_ms < 0 {
                    -1
                } else {
                    let elapsed = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
                    (timeout_ms - elapsed).max(0)
                };
                // SAFETY: `fds` has at least `wake_fd` and all descriptors belong to active waiters or the wakeup socket.
                let rc = unsafe { WSAPoll(fds.as_mut_ptr(), fds.len() as u32, left) };
                if rc <= 0 {
                    return;
                }
                let wake_hit = fds[0].revents != 0;
                if wake_hit {
                    self.drain_wake();
                }
                let was_explicit = self.explicit_wake.swap(false, Ordering::SeqCst);
                let mut fired = false;
                if fds.len() > 1 {
                    let mut waiters = self.waiters.lock().expect("the poller");
                    for item in &fds[1..] {
                        if item.revents == 0 {
                            continue;
                        }
                        let fd = item.fd as i32;
                        let Some(entry) = waiters.get_mut(&fd) else { continue };
                        let failed = item.revents & (POLLERR | POLLHUP | POLLNVAL) != 0;
                        if (item.revents & POLLRDNORM != 0 || failed) && let Some(token) = entry.read.take() {
                            ready.push(token);
                            fired = true;
                        }
                        if (item.revents & POLLWRNORM != 0 || failed) && let Some(token) = entry.write.take() {
                            ready.push(token);
                            fired = true;
                        }
                        if entry.is_empty() {
                            waiters.remove(&fd);
                        }
                    }
                }
                if fired || was_explicit || timeout_ms == 0 {
                    return;
                }
                if !wake_hit {
                    return;
                }
            }
        }

        pub fn settle(&self, woken: usize) {
            self.count.fetch_sub(woken, Ordering::SeqCst);
        }
    }
}

pub use imp::Poller;

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    use super::*;

    #[cfg(target_os = "linux")]
    fn pair() -> (std::os::unix::net::UnixStream, std::os::unix::net::UnixStream, i32, i32) {
        use std::os::fd::AsRawFd;
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let (fa, fb) = (a.as_raw_fd(), b.as_raw_fd());
        (a, b, fa, fb)
    }

    #[cfg(target_os = "windows")]
    fn pair() -> (std::net::TcpStream, std::net::TcpStream, i32, i32) {
        use std::net::{TcpListener, TcpStream};
        use std::os::windows::io::AsRawSocket;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let a = TcpStream::connect(addr).unwrap();
        let (b, _) = listener.accept().unwrap();
        let (fa, fb) = (a.as_raw_socket() as i32, b.as_raw_socket() as i32);
        (a, b, fa, fb)
    }

    #[test]
    fn poller_a_readable_descriptor_wakes_its_waiter() {
        let poller = Poller::new().unwrap();
        let (mut left, _right, _, fd) = pair();
        assert_eq!(poller.register(fd, Interest::Readable, 7), Ok(Ok(())));
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
        let (mut left, mut right, _, fd) = pair();
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
        let (_left, _right, fd, _) = pair();
        assert_eq!(poller.register(fd, Interest::Writable, 1), Ok(Ok(())));
        let mut ready = Vec::new();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [1]);
    }

    #[test]
    fn poller_a_reader_and_a_writer_of_one_descriptor_wake_apart() {
        let poller = Poller::new().unwrap();
        let (mut left, _right, _, fd) = pair();
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
        let (_left, _right, _, fd) = pair();
        assert_eq!(poller.register(fd, Interest::Readable, 1), Ok(Ok(())));
        assert_eq!(poller.register(fd, Interest::Readable, 2), Ok(Err(Busy)));
        assert_eq!(poller.waiting(), 1);
    }

    #[test]
    fn poller_a_closed_peer_wakes_the_waiters() {
        let poller = Poller::new().unwrap();
        let (left, _right, _, fd) = pair();
        assert_eq!(poller.register(fd, Interest::Readable, 1), Ok(Ok(())));
        drop(left);
        let mut ready = Vec::new();
        poller.poll(1000, &mut ready);
        assert_eq!(ready, [1]);
    }

    #[test]
    fn poller_a_cancelled_waiter_does_not_wake() {
        let poller = Poller::new().unwrap();
        let (mut left, _right, _, fd) = pair();
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
