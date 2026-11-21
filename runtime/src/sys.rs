//! Safe OS memory probing: reading two words that may not be mapped without faulting.

#[cfg(target_os = "linux")]
mod imp {
    use std::arch::asm;

    mod number {
        pub const GETPID: usize = 39;
        pub const PROCESS_VM_READV: usize = 310;
    }

    #[repr(C)]
    struct IoVec {
        base: usize,
        len: usize,
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

    pub fn read_two_words(address: usize) -> Option<[usize; 2]> {
        let mut words = [0usize; 2];
        let local = IoVec { base: words.as_mut_ptr() as usize, len: 16 };
        let remote = IoVec { base: address, len: 16 };
        // SAFETY: `getpid` takes no pointer; the vectors name this process's own buffers, and the kernel returns an error for an address it cannot read.
        let read = unsafe {
            let pid = call(number::GETPID, [0; 6]) as usize;
            call(number::PROCESS_VM_READV, [pid, &raw const local as usize, 1, &raw const remote as usize, 1, 0])
        };
        (read == 16).then_some(words)
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;

    const MEM_COMMIT: u32 = 0x1000;
    const PAGE_GUARD: u32 = 0x100;
    const PAGE_READ_MASK: u32 = 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;

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
        fn VirtualQuery(address: *const c_void, buffer: *mut MemoryBasicInformation, length: usize) -> usize;
    }

    fn is_readable_range(start: usize, bytes: usize) -> bool {
        let Some(end) = start.checked_add(bytes) else { return false };
        let mut cursor = start;
        while cursor < end {
            let mut info = std::mem::MaybeUninit::<MemoryBasicInformation>::uninit();
            // SAFETY: querying arbitrary addresses is safe; Windows returns failure for invalid addresses.
            let got = unsafe { VirtualQuery(cursor as *const c_void, info.as_mut_ptr(), size_of::<MemoryBasicInformation>()) };
            if got != size_of::<MemoryBasicInformation>() {
                return false;
            }
            let info = unsafe { info.assume_init() };
            let readable = info.state == MEM_COMMIT && (info.protect & PAGE_GUARD) == 0 && (info.protect & PAGE_READ_MASK) != 0;
            if !readable {
                return false;
            }
            let region_end = (info.base_address as usize).saturating_add(info.region_size);
            if region_end <= cursor {
                return false;
            }
            cursor = region_end;
        }
        true
    }

    pub fn read_two_words(address: usize) -> Option<[usize; 2]> {
        if address == 0 || !is_readable_range(address, 16) {
            return None;
        }
        // SAFETY: `is_readable_range` confirmed every page of `[address, address + 16)` is committed and readable.
        unsafe { Some([*(address as *const usize), *((address + 8) as *const usize)]) }
    }
}

/// The two words at `address` in this process, or `None` when they are not readable.
pub fn read_two_words(address: usize) -> Option<[usize; 2]> {
    imp::read_two_words(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_a_readable_address_gives_its_words_and_a_wild_one_gives_none() {
        let words = [7usize, 9];
        assert_eq!(read_two_words(words.as_ptr() as usize), Some([7, 9]));
        assert_eq!(read_two_words(0), None);
        assert_eq!(read_two_words(8), None);
        assert_eq!(read_two_words(usize::MAX - 7), None);
    }
}
