//! The context switch: callee-saved registers pushed on the old stack, the stack pointer swapped, the new registers popped.

use std::arch::global_asm;

#[cfg(target_os = "linux")]
global_asm!(
    ".text",
    ".p2align 4",
    ".global mote_sched_switch",
    "mote_sched_switch:",
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdi], rsp",
    "mov rsp, rsi",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",
    ".p2align 4",
    ".global mote_sched_trampoline",
    "mote_sched_trampoline:",
    "mov rdi, r12",
    "call r13",
    "ud2",
    ".p2align 4",
    ".global mote_sched_call_on",
    "mote_sched_call_on:",
    "push rbp",
    "mov rbp, rsp",
    "mov rax, rdi",
    "mov rdi, rsi",
    "mov rsp, rdx",
    "call rax",
    "mov rsp, rbp",
    "pop rbp",
    "ret",
);

#[cfg(target_os = "windows")]
global_asm!(
    ".text",
    ".p2align 4",
    ".global mote_sched_switch",
    "mote_sched_switch:",
    "push rbp",
    "push rbx",
    "push rdi",
    "push rsi",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov rax, gs:[0x08]",
    "push rax",
    "mov rax, gs:[0x10]",
    "push rax",
    "sub rsp, 160",
    "movups [rsp + 0], xmm6",
    "movups [rsp + 16], xmm7",
    "movups [rsp + 32], xmm8",
    "movups [rsp + 48], xmm9",
    "movups [rsp + 64], xmm10",
    "movups [rsp + 80], xmm11",
    "movups [rsp + 96], xmm12",
    "movups [rsp + 112], xmm13",
    "movups [rsp + 128], xmm14",
    "movups [rsp + 144], xmm15",
    "mov [rcx], rsp",
    "mov rsp, rdx",
    "movups xmm15, [rsp + 144]",
    "movups xmm14, [rsp + 128]",
    "movups xmm13, [rsp + 112]",
    "movups xmm12, [rsp + 96]",
    "movups xmm11, [rsp + 80]",
    "movups xmm10, [rsp + 64]",
    "movups xmm9, [rsp + 48]",
    "movups xmm8, [rsp + 32]",
    "movups xmm7, [rsp + 16]",
    "movups xmm6, [rsp + 0]",
    "add rsp, 160",
    "pop rax",
    "mov gs:[0x10], rax",
    "pop rax",
    "mov gs:[0x08], rax",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rsi",
    "pop rdi",
    "pop rbx",
    "pop rbp",
    "ret",
    ".p2align 4",
    ".global mote_sched_trampoline",
    "mote_sched_trampoline:",
    "sub rsp, 32",
    "mov rcx, r12",
    "call r13",
    "ud2",
    ".p2align 4",
    ".global mote_sched_call_on",
    "mote_sched_call_on:",
    "push rbp",
    "mov rbp, rsp",
    "push qword ptr gs:[0x08]",
    "push qword ptr gs:[0x10]",
    "mov rax, [r8 + 168]",
    "mov gs:[0x08], rax",
    "mov rax, [r8 + 160]",
    "mov gs:[0x10], rax",
    "sub r8, 256",
    "and r8, -16",
    "sub r8, 32",
    "mov rax, rcx",
    "mov rcx, rdx",
    "mov rsp, r8",
    "call rax",
    "mov rax, [rbp - 8]",
    "mov gs:[0x08], rax",
    "mov rax, [rbp - 16]",
    "mov gs:[0x10], rax",
    "mov rsp, rbp",
    "pop rbp",
    "ret",
);

unsafe extern "C" {
    fn mote_sched_switch(save: *mut usize, load: usize);
    fn mote_sched_trampoline();
    fn mote_sched_call_on(f: extern "C" fn(usize), arg: usize, sp: usize);
}

/// Bytes `prepare` writes below the top of a stack.
#[cfg(target_os = "linux")]
const INITIAL_FRAME: usize = 56;
#[cfg(target_os = "windows")]
const INITIAL_FRAME: usize = 248;

/// Lays out a stack spanning `[base, top)` so the first `switch` calls `entry(arg)` with a zero frame pointer.
///
/// # Safety
/// `top` is the 16-aligned end of writable memory of at least a page, `base < top`, and `entry` never returns.
pub unsafe fn prepare_range(base: usize, top: usize, entry: extern "C" fn(usize) -> !, arg: usize) -> usize {
    debug_assert!(top.is_multiple_of(16));
    let sp = top - INITIAL_FRAME;
    crate::sys::prepare_stack_window(base, sp, top);
    let words = sp as *mut usize;
    // SAFETY: the caller's promise: the frame lies inside the stack.
    unsafe {
        for at in 0..INITIAL_FRAME / 8 {
            words.add(at).write(0);
        }
        #[cfg(target_os = "linux")]
        {
            let _ = base;
            // r15, r14, r13, r12, rbx, rbp, return address.
            words.add(2).write(entry as usize);
            words.add(3).write(arg);
            words.add(6).write(mote_sched_trampoline as *const () as usize);
        }
        #[cfg(target_os = "windows")]
        {
            // xmm6..xmm15 (20 words), StackLimit, StackBase, r15, r14, r13, r12, rsi, rdi, rbx, rbp, return address.
            words.add(20).write(crate::sys::initial_stack_limit(base, top));
            words.add(21).write(top);
            words.add(24).write(entry as usize);
            words.add(25).write(arg);
            words.add(30).write(mote_sched_trampoline as *const () as usize);
        }
    }
    sp
}

/// Lays out a stack so that the first `switch` to it calls `entry(arg)` with the frame pointer zero; the stack pointer to switch to.
///
/// # Safety
/// `top` is the 16-aligned end of writable memory of at least a page, and `entry` never returns.
pub unsafe fn prepare(top: usize, entry: extern "C" fn(usize) -> !, arg: usize) -> usize {
    // SAFETY: the caller's promise.
    unsafe { prepare_range(top.saturating_sub(crate::stacks::TASK_STACK_BYTES), top, entry, arg) }
}

/// Saves the registers on this stack, stores its pointer in `save`, and continues on the stack at `load`.
///
/// # Safety
/// `load` is a pointer stored by `switch` or returned by `prepare`, whose stack nothing else is running on; `save` stays valid until that stack is resumed.
pub unsafe fn switch(save: *mut usize, load: usize) {
    // SAFETY: the caller's promise.
    unsafe { mote_sched_switch(save, load) };
}

/// Runs `f(arg)` on the stack whose pointer `os_sp` was stored by `switch`, below what that stack holds, and returns here.
///
/// # Safety
/// `os_sp` belongs to a stack that is suspended and has at least 256 bytes plus what `f` needs free below it; `f` does not switch.
pub unsafe fn call_on(os_sp: usize, f: extern "C" fn(usize), arg: usize) {
    #[cfg(target_os = "linux")]
    let target = (os_sp - 256) & !15;
    #[cfg(target_os = "windows")]
    let target = os_sp;
    // SAFETY: the caller's promise.
    unsafe { mote_sched_call_on(f, arg, target) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stacks::{Slab, TASK_STACK_BYTES};
    use std::hint::black_box;

    /// What a test task and the test thread share.
    #[repr(C)]
    struct Link {
        /// First, because the probe entry stores the frame pointer at the argument.
        frame_pointer: usize,
        task_sp: usize,
        main_sp: usize,
        counter: u64,
        sum: f64,
        seen_sp: usize,
        finished: bool,
    }

    fn link() -> Box<Link> {
        Box::new(Link { frame_pointer: usize::MAX, task_sp: 0, main_sp: 0, counter: 0, sum: 0.0, seen_sp: 0, finished: false })
    }

    fn here() -> usize {
        let marker = 0u8;
        black_box(&marker as *const u8 as usize)
    }

    extern "C" fn counting(arg: usize) -> ! {
        // SAFETY: the test keeps the link alive while the task runs.
        let link = unsafe { &mut *(arg as *mut Link) };
        link.seen_sp = here();
        let mut local = 100u64;
        let mut float = 0.5f64;
        loop {
            local += 1;
            float *= 1.5;
            link.counter = local;
            link.sum = float;
            // SAFETY: the main stack is suspended in a `switch` that stored `main_sp`.
            unsafe { switch(&raw mut link.task_sp, link.main_sp) };
            assert_eq!(link.counter, local, "a local survives a switch");
        }
    }

    extern "C" fn ending(arg: usize) -> ! {
        // SAFETY: as above.
        let link = unsafe { &mut *(arg as *mut Link) };
        link.finished = true;
        // SAFETY: as above.
        unsafe { switch(&raw mut link.task_sp, link.main_sp) };
        unreachable!("a finished task is never resumed");
    }

    fn start(slab: &Slab, entry: extern "C" fn(usize) -> !, link: &mut Link) -> crate::stacks::Stack {
        let stack = slab.acquire().unwrap();
        // SAFETY: the slot is writable memory with a 16-aligned top and the entry never returns.
        link.task_sp = unsafe { prepare_range(stack.base(), stack.top(), entry, link as *mut Link as usize) };
        stack
    }

    fn resume(link: &mut Link) {
        // SAFETY: the task is suspended or new, and `main_sp` is stored for it to come back.
        unsafe { switch(&raw mut link.main_sp, link.task_sp) };
    }

    #[test]
    fn switch_the_entry_runs_on_the_new_stack_with_its_argument_and_comes_back() {
        let slab = Slab::new(1, TASK_STACK_BYTES).unwrap();
        let mut link = link();
        let stack = start(&slab, ending, &mut link);
        resume(&mut link);
        assert!(link.finished);
        assert!(stack.contains(link.task_sp), "the saved stack pointer is on the task's stack");
        assert!(!stack.contains(here()));
    }

    #[test]
    fn switch_a_task_resumes_where_it_left_off_with_its_locals_and_floats() {
        let slab = Slab::new(1, TASK_STACK_BYTES).unwrap();
        let mut link = link();
        let stack = start(&slab, counting, &mut link);
        let mut expected = 0.5f64;
        for round in 1..=40u64 {
            resume(&mut link);
            expected *= 1.5;
            assert_eq!((link.counter, link.sum), (100 + round, expected));
        }
        assert!(stack.contains(link.seen_sp));
    }

    #[test]
    fn switch_two_tasks_keep_separate_stacks_and_state() {
        let slab = Slab::new(2, TASK_STACK_BYTES).unwrap();
        let (mut one, mut two) = (link(), link());
        let first = start(&slab, counting, &mut one);
        let second = start(&slab, counting, &mut two);
        for _ in 0..5 {
            resume(&mut one);
            resume(&mut two);
            resume(&mut two);
        }
        assert_eq!((one.counter, two.counter), (105, 110));
        assert!(first.contains(one.seen_sp) && second.contains(two.seen_sp));
        assert_ne!(one.task_sp, two.task_sp);
    }

    #[cfg(target_os = "linux")]
    global_asm!(".text", ".global mote_sched_probe", "mote_sched_probe:", "mov [rdi], rbp", "jmp {done}", done = sym probe_done);
    #[cfg(target_os = "windows")]
    global_asm!(".text", ".global mote_sched_probe", "mote_sched_probe:", "mov [rcx], rbp", "jmp {done}", done = sym probe_done);

    unsafe extern "C" {
        safe fn mote_sched_probe(arg: usize) -> !;
    }

    extern "C" fn probe_done(arg: usize) -> ! {
        // SAFETY: as above.
        let link = unsafe { &mut *(arg as *mut Link) };
        link.finished = true;
        // SAFETY: as above.
        unsafe { switch(&raw mut link.task_sp, link.main_sp) };
        unreachable!("a finished task is never resumed");
    }

    #[test]
    fn switch_the_frame_pointer_chain_of_a_new_task_starts_at_zero() {
        let slab = Slab::new(1, TASK_STACK_BYTES).unwrap();
        let mut link = link();
        let _stack = start(&slab, mote_sched_probe, &mut link);
        resume(&mut link);
        assert_eq!(link.frame_pointer, 0);
    }

    struct OsCall {
        task_stack_range: (usize, usize),
        inside: usize,
        deep: u64,
        before: usize,
        after: usize,
    }

    extern "C" fn deep_call(arg: usize) {
        // SAFETY: the test owns the record and runs it on one thread.
        let call = unsafe { &mut *(arg as *mut OsCall) };
        call.inside = here();
        let mut buffer = [0u8; 512 * 1024];
        for at in (0..buffer.len()).step_by(4096) {
            buffer[at] = 1;
        }
        call.deep = black_box(&buffer).iter().step_by(4096).map(|byte| u64::from(*byte)).sum();
    }

    extern "C" fn calling(arg: usize) -> ! {
        // SAFETY: as above.
        let link = unsafe { &mut *(arg as *mut Link) };
        // SAFETY: the test keeps the record alive.
        let call = unsafe { &mut *(link.counter as *mut OsCall) };
        call.before = here();
        // SAFETY: `main_sp` is the suspended thread stack with room below it, and `deep_call` does not switch.
        unsafe { call_on(link.main_sp, deep_call, link.counter as usize) };
        call.after = here();
        link.finished = true;
        // SAFETY: as above.
        unsafe { switch(&raw mut link.task_sp, link.main_sp) };
        unreachable!("a finished task is never resumed");
    }

    #[test]
    fn switch_a_call_on_the_os_stack_leaves_the_task_stack_and_returns_to_it() {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| {
                let slab = Slab::new(1, TASK_STACK_BYTES).unwrap();
                let mut link = link();
                let mut call = OsCall { task_stack_range: (0, 0), inside: 0, deep: 0, before: 0, after: 0 };
                link.counter = &raw mut call as u64;
                let stack = start(&slab, calling, &mut link);
                call.task_stack_range = (stack.base(), stack.top());
                resume(&mut link);
                assert!(link.finished);
                assert!(stack.contains(call.before) && stack.contains(call.after));
                assert!(!stack.contains(call.inside), "the call ran off the task stack");
                assert_eq!(call.deep, 128, "it used more stack than a task's slot holds");
                assert!(call.task_stack_range.0 < call.task_stack_range.1);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
