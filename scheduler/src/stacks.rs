//! The stack slab: one reservation cut into equal slots, committed on first touch and released with `madvise`.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::sys::{self, SysError, PAGE_SIZE};

/// Bytes of a spawned task's stack slot.
pub const TASK_STACK_BYTES: usize = 16 << 20;

/// Bytes of the main task's stack slot, as virtual space.
pub const MAIN_STACK_BYTES: usize = 256 << 20;

/// Bytes at the low end of a slot that the stack limit keeps free for the runtime's own frames.
/// On Windows, must exceed the kernel's ~20 KiB guaranteed stack floor plus fault-reporting frames.
#[cfg(target_os = "windows")]
pub const RED_ZONE: usize = 64 * 1024;
#[cfg(not(target_os = "windows"))]
pub const RED_ZONE: usize = 16 * 1024;

/// One slot of a slab; it is returned with `Slab::release`.
#[derive(Debug, PartialEq, Eq)]
pub struct Stack {
    slot: usize,
    base: usize,
    bytes: usize,
}

impl Stack {
    /// The lowest address of the slot.
    pub fn base(&self) -> usize {
        self.base
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The address just past the slot, where the stack starts; a multiple of 16.
    pub fn top(&self) -> usize {
        self.base + self.bytes
    }

    /// The lowest address a frame may reach before the prologue check enters the runtime.
    pub fn limit(&self) -> usize {
        self.base + RED_ZONE
    }

    pub fn contains(&self, address: usize) -> bool {
        (self.base..self.top()).contains(&address)
    }

    /// Gives back the whole pages below `sp`, which hold nothing live.
    ///
    /// # Safety
    /// The task of this slot is not running and `sp` is its saved stack pointer.
    pub unsafe fn discard_below(&self, sp: usize) {
        let end = sp & !(PAGE_SIZE - 1);
        if end > self.base {
            // SAFETY: whole pages of the slot, and the caller's promise that nothing below `sp` is live.
            unsafe { sys::discard(self.base, end - self.base) };
        }
    }

    /// Gives back every page of the slot; the caller has saved what it needs.
    ///
    /// # Safety
    /// The task of this slot is not running and its live bytes are saved elsewhere.
    pub unsafe fn discard_all(&self) {
        // SAFETY: whole pages of the slot, and the caller's promise.
        unsafe { sys::discard(self.base, self.bytes) };
    }
}

struct State {
    /// The first slot never handed out.
    next: usize,
    free: Vec<usize>,
}

/// A reservation of `count` slots of `slot_bytes` each.
pub struct Slab {
    base: NonNull<u8>,
    slot_bytes: usize,
    count: usize,
    state: Mutex<State>,
    mapped: AtomicBool,
}

// SAFETY: the slab owns its mapping; the slots it hands out are used by one task at a time.
unsafe impl Send for Slab {}
// SAFETY: as above; the free list is behind a lock.
unsafe impl Sync for Slab {}

impl Slab {
    /// Reserves the address space; no memory is committed until a stack is touched.
    pub fn new(count: usize, slot_bytes: usize) -> Result<Self, SysError> {
        assert!(count > 0 && slot_bytes.is_multiple_of(PAGE_SIZE) && slot_bytes >= 2 * RED_ZONE, "a slot is whole pages with room beyond the red zone");
        let base = sys::map_slots(count.checked_mul(slot_bytes).ok_or(SysError(12))?, slot_bytes)?;
        Ok(Self { base, slot_bytes, count, state: Mutex::new(State { next: 0, free: Vec::new() }), mapped: AtomicBool::new(true) })
    }

    pub fn slots(&self) -> usize {
        self.count
    }

    pub fn slot_bytes(&self) -> usize {
        self.slot_bytes
    }

    /// The slots handed out and not released.
    pub fn in_use(&self) -> usize {
        let state = self.state.lock().expect("the slab");
        state.next - state.free.len()
    }

    /// A slot from the free list, else the next unused one; `None` when all are in use.
    pub fn acquire(&self) -> Option<Stack> {
        let mut state = self.state.lock().expect("the slab");
        let slot = match state.free.pop() {
            Some(slot) => slot,
            None if state.next < self.count => {
                state.next += 1;
                state.next - 1
            }
            None => return None,
        };
        Some(Stack { slot, base: self.base.as_ptr() as usize + slot * self.slot_bytes, bytes: self.slot_bytes })
    }

    /// Returns the slot: its memory goes back to the system and it is the next one handed out.
    pub fn release(&self, stack: Stack) {
        assert!(stack.base == self.base.as_ptr() as usize + stack.slot * self.slot_bytes, "a stack of another slab");
        // SAFETY: the slot is whole pages of this slab's mapping and its task has ended.
        unsafe { sys::discard(stack.base, stack.bytes) };
        self.state.lock().expect("the slab").free.push(stack.slot);
    }
}

impl Slab {
    pub fn is_mapped(&self) -> bool {
        self.mapped.load(Ordering::SeqCst)
    }

    /// Gives the whole reservation back, once; a task that was abandoned in it never runs again.
    ///
    /// # Safety
    /// Nothing runs on or reads any slot of the slab after this.
    pub unsafe fn unmap(&self) {
        if self.mapped.swap(false, Ordering::SeqCst) {
            // SAFETY: the mapping was made by `new`, the slab owns it, and the caller's promise.
            unsafe { sys::unmap(self.base, self.count * self.slot_bytes) };
        }
    }
}

impl Drop for Slab {
    fn drop(&mut self) {
        // SAFETY: nothing uses the slab once it is dropped.
        unsafe { self.unmap() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_the_slots_are_equal_adjacent_and_aligned() {
        let slab = Slab::new(4, TASK_STACK_BYTES).unwrap();
        let stacks: Vec<Stack> = (0..4).map(|_| slab.acquire().unwrap()).collect();
        for pair in stacks.windows(2) {
            assert_eq!(pair[1].base(), pair[0].top());
        }
        assert!(stacks.iter().all(|stack| stack.bytes() == TASK_STACK_BYTES && stack.top() % 16 == 0 && stack.base() % PAGE_SIZE == 0));
        assert_eq!(slab.in_use(), 4);
    }

    #[test]
    fn stacks_the_limit_leaves_the_red_zone_above_the_bottom() {
        let slab = Slab::new(1, TASK_STACK_BYTES).unwrap();
        let stack = slab.acquire().unwrap();
        assert_eq!(stack.limit(), stack.base() + RED_ZONE);
        assert!(stack.contains(stack.limit()) && stack.contains(stack.top() - 1) && !stack.contains(stack.top()));
    }

    #[test]
    fn stacks_a_slab_hands_out_no_more_slots_than_it_has() {
        let slab = Slab::new(2, TASK_STACK_BYTES).unwrap();
        let first = slab.acquire().unwrap();
        let _second = slab.acquire().unwrap();
        assert_eq!(slab.acquire(), None);
        slab.release(first);
        assert!(slab.acquire().is_some());
    }

    #[test]
    fn stacks_a_released_slot_is_the_next_one_handed_out_and_reads_zero() {
        let slab = Slab::new(3, TASK_STACK_BYTES).unwrap();
        let first = slab.acquire().unwrap();
        let _second = slab.acquire().unwrap();
        let base = first.base();
        // SAFETY: the last word of the slot is mapped and this test owns the slot.
        unsafe { ((first.top() - 8) as *mut u64).write(0xdead) };
        slab.release(first);
        let again = slab.acquire().unwrap();
        assert_eq!(again.base(), base);
        // SAFETY: as above.
        assert_eq!(unsafe { ((again.top() - 8) as *const u64).read() }, 0);
        assert_eq!(slab.in_use(), 2);
    }

    #[test]
    fn stacks_memory_is_committed_when_touched_and_given_back_on_release() {
        let slab = Slab::new(64, TASK_STACK_BYTES).unwrap();
        let stacks: Vec<Stack> = (0..64).map(|_| slab.acquire().unwrap()).collect();
        let resident = |stack: &Stack| sys::resident_pages(stack.base(), stack.bytes());
        assert!(stacks.iter().all(|stack| resident(stack) == 0), "nothing is committed by acquiring");
        // SAFETY: the top words of the slots are mapped and this test owns them.
        unsafe { ((stacks[7].top() - 8) as *mut u64).write(1) };
        assert_eq!(resident(&stacks[7]), 1);
        assert_eq!(resident(&stacks[8]), 0);
        let seven = stacks.into_iter().nth(7).unwrap();
        let base = seven.base();
        slab.release(seven);
        assert_eq!(sys::resident_pages(base, TASK_STACK_BYTES), 0);
    }

    #[test]
    fn stacks_the_main_stack_is_256_mib_of_virtual_space() {
        let slab = Slab::new(1, MAIN_STACK_BYTES).unwrap();
        let stack = slab.acquire().unwrap();
        assert_eq!(stack.bytes(), 256 << 20);
        // SAFETY: the top word is mapped and this test owns it.
        unsafe { ((stack.top() - 8) as *mut u64).write(1) };
        assert_eq!(sys::resident_pages(stack.base(), stack.bytes()), 1);
    }

    #[test]
    fn stacks_a_spawned_task_has_16_mib_and_the_default_is_100000_slots() {
        assert_eq!(TASK_STACK_BYTES, 16 << 20);
        assert_eq!(crate::Config::new().spawn_slots, 100_000);
        let slab = Slab::new(crate::Config::new().spawn_slots, TASK_STACK_BYTES).unwrap();
        assert_eq!(slab.slots() * slab.slot_bytes(), 100_000 * (16 << 20));
    }

    #[test]
    fn stacks_the_pages_below_a_stack_pointer_can_be_given_back() {
        let slab = Slab::new(1, TASK_STACK_BYTES).unwrap();
        let stack = slab.acquire().unwrap();
        for page in [1, 40, 200] {
            // SAFETY: the words are inside the slot and this test owns it.
            unsafe { ((stack.base() + page * PAGE_SIZE) as *mut u64).write(7) };
        }
        // SAFETY: the same.
        unsafe { ((stack.top() - 8) as *mut u64).write(9) };
        assert_eq!(sys::resident_pages(stack.base(), stack.bytes()), 4);
        let sp = stack.top() - 64;
        // SAFETY: nothing runs on the slot.
        unsafe { stack.discard_below(sp) };
        assert_eq!(sys::resident_pages(stack.base(), stack.bytes()), 1);
        // SAFETY: the same.
        assert_eq!(unsafe { ((stack.top() - 8) as *const u64).read() }, 9);
        // SAFETY: the same.
        unsafe { stack.discard_all() };
        assert_eq!(sys::resident_pages(stack.base(), stack.bytes()), 0);
    }

    #[test]
    fn stacks_a_slab_is_shared_between_threads() {
        let slab = Slab::new(64, TASK_STACK_BYTES).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        let stack = slab.acquire().unwrap();
                        slab.release(stack);
                    }
                });
            }
        });
        assert_eq!(slab.in_use(), 0);
    }
}
