//! The one Linux call the runtime makes: reading memory that may not be mapped, as a raw system call.

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

/// The two words at `address` in this process, or `None` when they are not readable; the kernel checks, so a wild address does not fault.
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
