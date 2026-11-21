//! The OS calls the scheduler makes (Linux raw syscalls; Windows Win32 virtual memory).

use std::ptr::NonNull;

/// Bytes of an OS page.
pub const PAGE_SIZE: usize = 4096;

/// The call failed; the value is the OS error number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysError(pub i32);

#[cfg(target_os = "linux")]
mod imp {
    use std::arch::asm;
    use std::ptr::NonNull;

    #[cfg(test)]
    use super::PAGE_SIZE;
    use super::SysError;

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

    pub fn map_slots(bytes: usize, _slot_bytes: usize) -> Result<NonNull<u8>, SysError> {
        // SAFETY: an anonymous mapping at no fixed address touches no existing memory.
        let result = unsafe { call(number::MMAP, [0, bytes, PROT_READ_WRITE, MAP_PRIVATE_ANONYMOUS_NORESERVE, usize::MAX, 0]) };
        let address = check(result)?;
        Ok(NonNull::new(address as *mut u8).expect("a mapping is not at address 0"))
    }

    pub fn prepare_stack_window(_base: usize, _sp: usize, _top: usize) {}

    pub fn initial_stack_limit(base: usize, _top: usize) -> usize {
        base
    }

    pub unsafe fn unmap(address: NonNull<u8>, bytes: usize) {
        // SAFETY: the caller's promise.
        let result = unsafe { call(number::MUNMAP, [address.as_ptr() as usize, bytes, 0, 0, 0, 0]) };
        debug_assert_eq!(result, 0, "munmap failed");
    }

    pub unsafe fn discard(address: usize, bytes: usize) {
        // SAFETY: the caller's promise.
        let result = unsafe { call(number::MADVISE, [address, bytes, MADV_DONTNEED, 0, 0, 0]) };
        debug_assert_eq!(result, 0, "madvise failed");
    }

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

    pub fn epoll_wait(epoll: i32, events: &mut [EpollEvent], timeout_ms: i32) -> Result<usize, SysError> {
        // SAFETY: `events` is writable for its length; a null signal mask keeps the mask.
        let result = unsafe { call(number::EPOLL_PWAIT, [epoll as usize, events.as_mut_ptr() as usize, events.len(), timeout_ms as isize as usize, 0, 8]) };
        match check(result) {
            Err(SysError(EINTR)) => Ok(0),
            other => other,
        }
    }

    pub fn eventfd() -> Result<i32, SysError> {
        // SAFETY: takes no pointer.
        check(unsafe { call(number::EVENTFD2, [0, EFD_CLOEXEC | EFD_NONBLOCK, 0, 0, 0, 0]) }).map(|fd| fd as i32)
    }

    pub fn eventfd_write(fd: i32, value: u64) {
        // SAFETY: eight readable bytes.
        unsafe { call(number::WRITE, [fd as usize, &raw const value as usize, 8, 0, 0, 0]) };
    }

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
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::{null_mut, NonNull};
    use std::sync::{Once, RwLock};

    use super::{SysError, PAGE_SIZE};

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_DECOMMIT: u32 = 0x4000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;
    const PAGE_GUARD: u32 = 0x100;
    const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC000_0005;
    const STATUS_GUARD_PAGE_VIOLATION: u32 = 0x8000_0001;
    const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
    const STACK_CHUNK_BYTES: usize = 64 * 1024;
    const STACK_CUSHION_BYTES: usize = 2 * PAGE_SIZE;

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *mut ExceptionRecord,
        address: *mut c_void,
        num_parameters: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        partition_id: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        ty: u32,
    }

    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, free_type: u32) -> i32;
        #[cfg(test)]
        fn VirtualQuery(address: *const c_void, buffer: *mut MemoryBasicInformation, length: usize) -> usize;
        fn AddVectoredExceptionHandler(first: u32, handler: extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
        fn GetLastError() -> u32;
    }

    static SLABS: RwLock<Vec<(usize, usize, usize)>> = RwLock::new(Vec::new());
    static HANDLER_INIT: Once = Once::new();

    fn find_slab(addr: usize) -> Option<(usize, usize, usize)> {
        let Ok(ranges) = SLABS.read() else { return None };
        ranges.iter().copied().find(|&(start, end, _)| addr >= start && addr < end)
    }

    /// Commits `[next_base .. chunk_end)` as writable stack memory, placing a guard page after `STACK_CUSHION_BYTES` when room remains above `slot_base`.
    /// Returns the safe `StackLimit` (just above the guard page, or `next_base` at the bottom).
    unsafe fn commit_stack_chunk(slot_base: usize, next_base: usize, chunk_end: usize, sp_page: usize) -> Option<usize> {
        if chunk_end > next_base {
            // SAFETY: caller ensures `[next_base, chunk_end)` lies within a reserved stack slot.
            if unsafe { VirtualAlloc(next_base as *mut c_void, chunk_end - next_base, MEM_COMMIT, PAGE_READWRITE) }.is_null() {
                return None;
            }
        }
        let guard = next_base + STACK_CUSHION_BYTES;
        if next_base > slot_base && guard + PAGE_SIZE <= sp_page.min(chunk_end) {
            // SAFETY: committing a single guard page with `STACK_CUSHION_BYTES` committed beneath it for exception dispatch.
            unsafe { VirtualAlloc(guard as *mut c_void, PAGE_SIZE, MEM_COMMIT, PAGE_READWRITE | PAGE_GUARD) };
            Some(guard + PAGE_SIZE)
        } else {
            Some(next_base)
        }
    }

    pub fn initial_stack_limit(base: usize, top: usize) -> usize {
        let chunk_start = (top.saturating_sub(STACK_CHUNK_BYTES) & !(PAGE_SIZE - 1)).max(base);
        let guard = chunk_start + STACK_CUSHION_BYTES;
        if chunk_start > base && guard + PAGE_SIZE <= top - PAGE_SIZE { guard + PAGE_SIZE } else { chunk_start }
    }

    /// Ensures a 64 KiB committed window below `sp` up to `top` (plus an 8 KiB cushion beneath a guard page) before switching to a Windows task stack.
    pub fn prepare_stack_window(base: usize, sp: usize, top: usize) {
        let sp_page = sp & !(PAGE_SIZE - 1);
        let commit_end = ((sp + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)).max(sp_page + PAGE_SIZE).min(top);
        let chunk_start = (sp.saturating_sub(STACK_CHUNK_BYTES) & !(PAGE_SIZE - 1)).max(base);
        // SAFETY: `[chunk_start, commit_end)` is inside the task's reserved `[base, top)` stack slot.
        unsafe { commit_stack_chunk(base, chunk_start, commit_end, sp_page) };
    }

    /// Commits reserved stack slab pages lazily on first touch or guard-page hit so large 16 MiB/256 MiB slabs take no physical RAM upfront.
    extern "system" fn commit_on_fault(info: *mut ExceptionPointers) -> i32 {
        // SAFETY: Windows passes a valid `ExceptionPointers` whose `record` describes the exception.
        unsafe {
            let record = &*(*info).record;
            if record.num_parameters < 2 {
                return EXCEPTION_CONTINUE_SEARCH;
            }
            let addr = record.information[1];
            let Some((slab_start, _, slot_bytes)) = find_slab(addr) else {
                return EXCEPTION_CONTINUE_SEARCH;
            };
            if record.code == STATUS_GUARD_PAGE_VIOLATION {
                let slot_base = slab_start + ((addr - slab_start) / slot_bytes) * slot_bytes;
                let slot_end = slot_base + slot_bytes;
                let guard_page = addr & !(PAGE_SIZE - 1);
                let cur_chunk_base = guard_page.saturating_sub(STACK_CUSHION_BYTES).max(slot_base);
                let next_base = cur_chunk_base.saturating_sub(STACK_CHUNK_BYTES).max(slot_base);
                let Some(new_limit) = commit_stack_chunk(slot_base, next_base, cur_chunk_base, cur_chunk_base) else {
                    return EXCEPTION_CONTINUE_SEARCH;
                };
                let mut cur_limit: usize;
                std::arch::asm!("mov {}, gs:[0x10]", out(reg) cur_limit, options(nostack, readonly, preserves_flags));
                if cur_limit >= slot_base && cur_limit <= slot_end && new_limit < cur_limit {
                    std::arch::asm!("mov gs:[0x10], {}", in(reg) new_limit, options(nostack, preserves_flags));
                }
                return EXCEPTION_CONTINUE_EXECUTION;
            }
            if record.code != EXCEPTION_ACCESS_VIOLATION {
                return EXCEPTION_CONTINUE_SEARCH;
            }
            let page = (addr & !(PAGE_SIZE - 1)) as *mut c_void;
            if VirtualAlloc(page, PAGE_SIZE, MEM_COMMIT, PAGE_READWRITE).is_null() {
                return EXCEPTION_CONTINUE_SEARCH;
            }
            EXCEPTION_CONTINUE_EXECUTION
        }
    }

    fn ensure_handler() {
        HANDLER_INIT.call_once(|| {
            // SAFETY: registers a process-wide vectored handler that only handles accesses within registered stack slabs.
            unsafe { AddVectoredExceptionHandler(1, commit_on_fault) };
        });
    }

    pub fn map_slots(bytes: usize, slot_bytes: usize) -> Result<NonNull<u8>, SysError> {
        ensure_handler();
        // SAFETY: reserves virtual address space only; pages commit on first touch via `commit_on_fault`.
        let ptr = unsafe { VirtualAlloc(null_mut(), bytes, MEM_RESERVE, PAGE_READWRITE) };
        let non_null: NonNull<u8> = NonNull::new(ptr.cast()).ok_or_else(|| SysError(unsafe { GetLastError() as i32 }))?;
        let start = non_null.as_ptr() as usize;
        SLABS.write().expect("slab registry").push((start, start + bytes, slot_bytes));
        Ok(non_null)
    }

    pub unsafe fn unmap(address: NonNull<u8>, _bytes: usize) {
        let start = address.as_ptr() as usize;
        if let Ok(mut ranges) = SLABS.write() {
            ranges.retain(|&(base, _, _)| base != start);
        }
        // SAFETY: the caller's promise; `address` is the exact `VirtualAlloc` reservation base.
        let ok = unsafe { VirtualFree(address.as_ptr().cast(), 0, MEM_RELEASE) };
        debug_assert_ne!(ok, 0, "VirtualFree(MEM_RELEASE) failed");
    }

    pub unsafe fn discard(address: usize, bytes: usize) {
        if bytes == 0 {
            return;
        }
        // SAFETY: `address` and `bytes` are whole pages inside a reserved slab range.
        let ok = unsafe { VirtualFree(address as *mut c_void, bytes, MEM_DECOMMIT) };
        debug_assert_ne!(ok, 0, "VirtualFree(MEM_DECOMMIT) failed");
    }

    #[cfg(test)]
    pub fn resident_pages(address: usize, bytes: usize) -> usize {
        let end = address + bytes;
        let mut cursor = address;
        let mut resident = 0usize;
        while cursor < end {
            let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::uninit();
            // SAFETY: `cursor` is within a mapped virtual reservation.
            let got = unsafe { VirtualQuery(cursor as *const c_void, info.as_mut_ptr(), size_of::<MemoryBasicInformation>()) };
            if got != size_of::<MemoryBasicInformation>() {
                break;
            }
            let info = unsafe { info.assume_init() };
            let next = (info.base_address as usize).saturating_add(info.region_size).min(end);
            if info.state == MEM_COMMIT && next > cursor {
                resident += (next - cursor) / PAGE_SIZE;
            }
            if next <= cursor {
                break;
            }
            cursor = next;
        }
        resident
    }

    const _: usize = size_of::<MemoryBasicInformation>();
}

#[cfg(target_os = "linux")]
pub use imp::{epoll_create, epoll_ctl, epoll_wait, eventfd, eventfd_read, eventfd_write, close, EpollEvent, EPOLLERR, EPOLLHUP, EPOLLIN, EPOLLONESHOT, EPOLLOUT, EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD};

/// Zeroed read-write memory of `bytes` (whole pages) cut into `slot_bytes` slots that the system commits when touched.
pub fn map_slots(bytes: usize, slot_bytes: usize) -> Result<NonNull<u8>, SysError> {
    assert!(bytes > 0 && bytes.is_multiple_of(PAGE_SIZE), "a mapping is whole pages");
    imp::map_slots(bytes, slot_bytes)
}

/// Zeroed read-write memory of `bytes` (whole pages) that the system commits when it is touched.
#[allow(dead_code)]
pub fn map(bytes: usize) -> Result<NonNull<u8>, SysError> {
    map_slots(bytes, bytes)
}

/// Ensures the stack window below `sp` up to `top` is ready for execution on this OS.
pub fn prepare_stack_window(base: usize, sp: usize, top: usize) {
    imp::prepare_stack_window(base, sp, top);
}

/// Initial `StackLimit` written into a newly prepared task stack.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn initial_stack_limit(base: usize, top: usize) -> usize {
    imp::initial_stack_limit(base, top)
}

/// Gives the pages back to the system.
///
/// # Safety
/// `address` and `bytes` are whole pages of one mapping made by this module that nothing uses any more.
pub unsafe fn unmap(address: NonNull<u8>, bytes: usize) {
    // SAFETY: the caller's promise.
    unsafe { imp::unmap(address, bytes) }
}

/// Drops the contents of whole pages: the memory reads as zero again and the system may reuse the pages.
///
/// # Safety
/// `address` and `bytes` are whole pages of a mapping made by this module, and nothing needs their contents.
pub unsafe fn discard(address: usize, bytes: usize) {
    // SAFETY: the caller's promise.
    unsafe { imp::discard(address, bytes) }
}

/// How many of the pages of the range hold memory now.
#[cfg(test)]
pub fn resident_pages(address: usize, bytes: usize) -> usize {
    imp::resident_pages(address, bytes)
}
