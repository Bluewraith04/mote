//! The Linux calls the scheduler makes, as raw system calls.

use std::arch::asm;
use std::ptr::NonNull;

/// Bytes of an OS page.
pub const PAGE_SIZE: usize = 4096;

mod number {
    pub const MMAP: usize = 9;
    pub const MUNMAP: usize = 11;
    pub const MADVISE: usize = 28;
    #[cfg(test)]
    pub const MINCORE: usize = 27;
    pub const EPOLL_CREATE1: usize = 291;
    pub const EPOLL_CTL: usize = 233;
    pub const EPOLL_PWAIT: usize = 281;
    pub const EVENTFD2: usize = 290;
    pub const READ: usize = 0;
    pub const WRITE: usize = 1;
    pub const CLOSE: usize = 3;
}

const PROT_READ_WRITE: usize = 0x1 | 0x2;
const MAP_PRIVATE_ANONYMOUS_NORESERVE: usize = 0x2 | 0x20 | 0x4000;
const MADV_DONTNEED: usize = 4;
const EINTR: i32 = 4;

pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;
pub const EPOLLIN: u32 = 0x1;
pub const EPOLLOUT: u32 = 0x4;
pub const EPOLLERR: u32 = 0x8;
pub const EPOLLHUP: u32 = 0x10;
pub const EPOLLONESHOT: u32 = 1 << 30;
const EPOLL_CLOEXEC: usize = 0o2000000;
const EFD_CLOEXEC: usize = 0o2000000;
const EFD_NONBLOCK: usize = 0o4000;

/// The call failed; the value is the Linux error number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysError(pub i32);

/// One ready event of an epoll instance.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EpollEvent {
    pub events: u32,
    pub data: u64,
}

impl EpollEvent {
    pub const fn new(events: u32, data: u64) -> Self {
        Self { events, data }
    }
}

unsafe fn call(number: usize, args: [usize; 6]) -> isize {
    let result;
    // SAFETY: the Linux system call convention; the caller vouches for the arguments.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") args[0], in("rsi") args[1], in("rdx") args[2],
            in("r10") args[3], in("r8") args[4], in("r9") args[5],
            lateout("rcx") _, lateout("r11") _,
            options(nostack)
        );
    }
    result
}

fn check(result: isize) -> Result<usize, SysError> {
    if (-4095..0).contains(&result) { Err(SysError(-result as i32)) } else { Ok(result as usize) }
}

/// Zeroed read-write memory of `bytes` (whole pages) that the system commits when it is touched.
pub fn map(bytes: usize) -> Result<NonNull<u8>, SysError> {
    assert!(bytes > 0 && bytes.is_multiple_of(PAGE_SIZE), "a mapping is whole pages");
    // SAFETY: an anonymous mapping at no fixed address touches no existing memory.
    let result = unsafe { call(number::MMAP, [0, bytes, PROT_READ_WRITE, MAP_PRIVATE_ANONYMOUS_NORESERVE, usize::MAX, 0]) };
    let address = check(result)?;
    Ok(NonNull::new(address as *mut u8).expect("a mapping is not at address 0"))
}

/// Gives the pages back to the system.
///
/// # Safety
/// `address` and `bytes` are whole pages of one mapping made by this module that nothing uses any more.
pub unsafe fn unmap(address: NonNull<u8>, bytes: usize) {
    // SAFETY: the caller's promise.
    let result = unsafe { call(number::MUNMAP, [address.as_ptr() as usize, bytes, 0, 0, 0, 0]) };
    debug_assert_eq!(result, 0, "munmap failed");
}

/// Drops the contents of whole pages: the memory reads as zero again and the system may reuse the pages.
///
/// # Safety
/// `address` and `bytes` are whole pages of a mapping made by this module, and nothing needs their contents.
pub unsafe fn discard(address: usize, bytes: usize) {
    // SAFETY: the caller's promise.
    let result = unsafe { call(number::MADVISE, [address, bytes, MADV_DONTNEED, 0, 0, 0]) };
    debug_assert_eq!(result, 0, "madvise failed");
}

/// How many of the pages of the range hold memory now.
#[cfg(test)]
pub fn resident_pages(address: usize, bytes: usize) -> usize {
    let pages = bytes.div_ceil(PAGE_SIZE);
    let mut vector = vec![0u8; pages];
    // SAFETY: the range is mapped and `vector` has a byte for each page.
    let result = unsafe { call(number::MINCORE, [address, bytes, vector.as_mut_ptr() as usize, 0, 0, 0]) };
    assert_eq!(result, 0, "mincore failed");
    vector.iter().filter(|byte| **byte & 1 != 0).count()
}

pub fn epoll_create() -> Result<i32, SysError> {
    // SAFETY: takes no pointer.
    check(unsafe { call(number::EPOLL_CREATE1, [EPOLL_CLOEXEC, 0, 0, 0, 0, 0]) }).map(|fd| fd as i32)
}

pub fn epoll_ctl(epoll: i32, operation: i32, fd: i32, event: Option<EpollEvent>) -> Result<(), SysError> {
    let event = event.unwrap_or(EpollEvent::new(0, 0));
    // SAFETY: `event` lives through the call.
    check(unsafe { call(number::EPOLL_CTL, [epoll as usize, operation as usize, fd as usize, &raw const event as usize, 0, 0]) }).map(|_| ())
}

/// Waits up to `timeout_ms` (negative for no limit); the number of events written to `events`.
pub fn epoll_wait(epoll: i32, events: &mut [EpollEvent], timeout_ms: i32) -> Result<usize, SysError> {
    // SAFETY: `events` is writable for its length; a null signal mask keeps the mask.
    let result = unsafe { call(number::EPOLL_PWAIT, [epoll as usize, events.as_mut_ptr() as usize, events.len(), timeout_ms as isize as usize, 0, 8]) };
    match check(result) {
        Err(SysError(EINTR)) => Ok(0),
        other => other,
    }
}

/// A non-blocking counter descriptor.
pub fn eventfd() -> Result<i32, SysError> {
    // SAFETY: takes no pointer.
    check(unsafe { call(number::EVENTFD2, [0, EFD_CLOEXEC | EFD_NONBLOCK, 0, 0, 0, 0]) }).map(|fd| fd as i32)
}

/// Adds `value` to the counter of an eventfd.
pub fn eventfd_write(fd: i32, value: u64) {
    // SAFETY: eight readable bytes.
    unsafe { call(number::WRITE, [fd as usize, &raw const value as usize, 8, 0, 0, 0]) };
}

/// Resets the counter of an eventfd; what it held.
pub fn eventfd_read(fd: i32) -> u64 {
    let mut value = 0u64;
    // SAFETY: eight writable bytes.
    unsafe { call(number::READ, [fd as usize, &raw mut value as usize, 8, 0, 0, 0]) };
    value
}

pub fn close(fd: i32) {
    // SAFETY: takes no pointer.
    unsafe { call(number::CLOSE, [fd as usize, 0, 0, 0, 0, 0]) };
}
