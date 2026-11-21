//! The OS memory calls the heap makes.

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

    use super::{SysError, PAGE_SIZE};

    mod number {
        pub const MMAP: usize = 9;
        pub const MUNMAP: usize = 11;
        pub const MADVISE: usize = 28;
    }

    const PROT_READ_WRITE: usize = 0x1 | 0x2;
    const MAP_PRIVATE_ANONYMOUS_NORESERVE: usize = 0x2 | 0x20 | 0x4000;
    const MADV_DONTNEED: usize = 4;

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

    pub fn map(bytes: usize) -> Result<NonNull<u8>, SysError> {
        // SAFETY: an anonymous mapping at no fixed address touches no existing memory.
        let result = unsafe { call(number::MMAP, [0, bytes, PROT_READ_WRITE, MAP_PRIVATE_ANONYMOUS_NORESERVE, usize::MAX, 0]) };
        let address = check(result)?;
        Ok(NonNull::new(address as *mut u8).expect("a mapping is not at address 0"))
    }

    pub fn map_aligned(bytes: usize, align: usize) -> Result<NonNull<u8>, SysError> {
        let whole = map(bytes + align)?;
        let start = whole.as_ptr() as usize;
        let aligned = start.next_multiple_of(align);
        let head = aligned - start;
        let tail = align - head;
        // SAFETY: the head and the tail are parts of the mapping made above and nothing refers to them.
        unsafe {
            if head > 0 {
                unmap(whole, head);
            }
            if tail > 0 {
                unmap(NonNull::new((aligned + bytes) as *mut u8).expect("inside the mapping"), tail);
            }
        }
        Ok(NonNull::new(aligned as *mut u8).expect("inside the mapping"))
    }

    pub unsafe fn unmap(address: NonNull<u8>, bytes: usize) {
        // SAFETY: the caller's promise.
        let result = unsafe { call(number::MUNMAP, [address.as_ptr() as usize, bytes, 0, 0, 0, 0]) };
        debug_assert_eq!(result, 0, "munmap failed");
    }

    pub unsafe fn discard(address: NonNull<u8>, bytes: usize) {
        // SAFETY: the caller's promise.
        let result = unsafe { call(number::MADVISE, [address.as_ptr() as usize, bytes, MADV_DONTNEED, 0, 0, 0]) };
        debug_assert_eq!(result, 0, "madvise failed");
    }

    const _: usize = PAGE_SIZE;
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::{null_mut, NonNull};

    use super::SysError;

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_DECOMMIT: u32 = 0x4000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;
    const ALLOCATION_GRANULARITY: usize = 64 * 1024;

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
        fn VirtualQuery(address: *const c_void, buffer: *mut MemoryBasicInformation, length: usize) -> usize;
        fn GetLastError() -> u32;
    }

    pub fn map(bytes: usize) -> Result<NonNull<u8>, SysError> {
        // SAFETY: reserves and commits private zeroed pages at a system-chosen address.
        let ptr = unsafe { VirtualAlloc(null_mut(), bytes, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        NonNull::new(ptr.cast()).ok_or_else(|| SysError(unsafe { GetLastError() as i32 }))
    }

    pub fn map_aligned(bytes: usize, align: usize) -> Result<NonNull<u8>, SysError> {
        // VirtualAlloc base addresses are guaranteed aligned to Windows' 64 KiB allocation granularity.
        if align <= ALLOCATION_GRANULARITY {
            return map(bytes);
        }
        let whole = map(bytes + align)?;
        let aligned = (whole.as_ptr() as usize).next_multiple_of(align);
        Ok(NonNull::new(aligned as *mut u8).expect("inside the mapping"))
    }

    pub unsafe fn unmap(address: NonNull<u8>, _bytes: usize) {
        let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::uninit();
        // SAFETY: `address` lies inside a VirtualAlloc allocation; MEM_RELEASE frees the entire allocation base.
        let queried = unsafe { VirtualQuery(address.as_ptr().cast(), info.as_mut_ptr(), size_of::<MemoryBasicInformation>()) };
        let base = if queried == size_of::<MemoryBasicInformation>() {
            unsafe { info.assume_init().allocation_base }
        } else {
            address.as_ptr().cast()
        };
        let ok = unsafe { VirtualFree(base, 0, MEM_RELEASE) };
        debug_assert_ne!(ok, 0, "VirtualFree(MEM_RELEASE) failed");
    }

    pub unsafe fn discard(address: NonNull<u8>, bytes: usize) {
        // Decommit drops physical pages; immediate MEM_COMMIT preserves virtual readability with fresh zeroed demand pages.
        let ok = unsafe { VirtualFree(address.as_ptr().cast(), bytes, MEM_DECOMMIT) };
        debug_assert_ne!(ok, 0, "VirtualFree(MEM_DECOMMIT) failed");
        let ptr = unsafe { VirtualAlloc(address.as_ptr().cast(), bytes, MEM_COMMIT, PAGE_READWRITE) };
        debug_assert!(!ptr.is_null(), "VirtualAlloc(MEM_COMMIT) after decommit failed");
    }
}

/// Zeroed read-write memory of `bytes` (a multiple of the page size) at an address the system picks.
pub fn map(bytes: usize) -> Result<NonNull<u8>, SysError> {
    assert!(bytes > 0 && bytes.is_multiple_of(PAGE_SIZE), "a mapping is whole pages");
    imp::map(bytes)
}

/// Zeroed memory of `bytes` whose address is a multiple of `align`; both are powers of two and `align` is at least a page.
pub fn map_aligned(bytes: usize, align: usize) -> Result<NonNull<u8>, SysError> {
    assert!(align.is_power_of_two() && align >= PAGE_SIZE, "an alignment is a power of two of at least a page");
    imp::map_aligned(bytes, align)
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
pub unsafe fn discard(address: NonNull<u8>, bytes: usize) {
    // SAFETY: the caller's promise.
    unsafe { imp::discard(address, bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_a_mapping_is_zeroed_and_writable() {
        let memory = map(2 * PAGE_SIZE).unwrap();
        let words = memory.as_ptr().cast::<u64>();
        // SAFETY: two pages of fresh memory.
        unsafe {
            assert_eq!(words.read(), 0);
            assert_eq!(words.add(2 * PAGE_SIZE / 8 - 1).read(), 0);
            words.write(7);
            assert_eq!(words.read(), 7);
            unmap(memory, 2 * PAGE_SIZE);
        }
    }

    #[test]
    fn blocks_an_aligned_mapping_starts_on_its_alignment() {
        let align = 1 << 16;
        let memory = map_aligned(3 * align, align).unwrap();
        assert_eq!(memory.as_ptr() as usize % align, 0);
        // SAFETY: the whole aligned range is mapped.
        unsafe {
            memory.as_ptr().add(3 * align - 1).write(1);
            unmap(memory, 3 * align);
        }
    }

    #[test]
    fn blocks_discarded_pages_read_as_zero() {
        let memory = map(PAGE_SIZE).unwrap();
        // SAFETY: one page of fresh memory.
        unsafe {
            memory.as_ptr().write(9);
            discard(memory, PAGE_SIZE);
            assert_eq!(memory.as_ptr().read(), 0);
            unmap(memory, PAGE_SIZE);
        }
    }
}
