//! A task: its state machine, its stack, its context block and what waits for it.

use std::any::Any;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use mote_abi::TaskContext;

use crate::stacks::Stack;

/// Queued and not running.
pub(crate) const READY: u8 = 0;
pub(crate) const RUNNING: u8 = 1;
/// Decided to park and still on its stack; the worker finishes the move.
pub(crate) const PARKING: u8 = 2;
pub(crate) const PARKED: u8 = 3;
/// Woken while running or parking: the next park returns at once.
pub(crate) const WOKEN: u8 = 4;
pub(crate) const DONE: u8 = 5;
/// Parked and having its stack copied out; a wake in this state is left for the compactor to carry out.
pub(crate) const COMPACTING: u8 = 6;

/// The state of a task by name, for reports.
pub(crate) fn state_name(state: u8) -> &'static str {
    ["ready", "running", "parking", "parked", "woken", "done", "parked"][state as usize]
}

/// Who waits for the end of a task.
#[derive(Default)]
pub(crate) struct Completion {
    pub finished: bool,
    pub joiners: Vec<Arc<Task>>,
}

pub(crate) struct Task {
    pub id: u64,
    pub name: String,
    pub is_main: bool,
    pub state: AtomicU8,
    /// The saved stack pointer while the task is not running.
    pub sp: UnsafeCell<usize>,
    /// The slot, taken when the task first runs and given back when it ends.
    pub stack: UnsafeCell<Option<Stack>>,
    /// The bytes from the saved stack pointer to the top of the slot while a cold task's pages are released.
    pub snapshot: UnsafeCell<Option<Box<[u8]>>>,
    pub entry: UnsafeCell<Option<Box<dyn FnOnce() + Send>>>,
    /// The block compiled code reads through the context pointer; it never moves.
    pub context: Box<UnsafeCell<TaskContext>>,
    /// The stack limit before cancellation poisons the word; zero until the task has a stack.
    pub real_limit: AtomicUsize,
    /// The scheduler's clock when the task last parked.
    pub parked_at: AtomicU64,
    /// The scheduler's clock when a worker queued the task for itself without waking another; zero when any worker may steal it.
    pub ready_at: AtomicU64,
    /// The parked stack was already handled by the compactor.
    pub cold: AtomicBool,
    pub cancelled: AtomicBool,
    /// The task runs the cleanups of a cancellation, which no further `cancel` interrupts.
    pub cleaning: AtomicBool,
    /// Orders `cancel` against the start of the cleanups.
    pub cancel_lock: Mutex<()>,
    /// What the task waits for while it is parked, for the deadlock report.
    pub waiting: Mutex<String>,
    pub completion: Mutex<Completion>,
    pub finished: Condvar,
    pub panic: Mutex<Option<Box<dyn Any + Send>>>,
    /// Polls left before the seeded scheduler preempts the task.
    pub polls_left: AtomicU64,
}

// SAFETY: the cells are touched by the worker that runs the task, one at a time, handed over through the state machine.
unsafe impl Send for Task {}
// SAFETY: as above; everything else is atomic or locked.
unsafe impl Sync for Task {}

impl Task {
    pub fn poll_word(&self) -> &AtomicU64 {
        // SAFETY: the context is a live, 8-aligned block; other threads write this word only atomically.
        unsafe { AtomicU64::from_ptr(&raw mut (*self.context.get()).poll) }
    }

    pub fn stack_limit_word(&self) -> &AtomicU64 {
        // SAFETY: as above.
        unsafe { AtomicU64::from_ptr(&raw mut (*self.context.get()).stack_limit) }
    }

    pub fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }
}
