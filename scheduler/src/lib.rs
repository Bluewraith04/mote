//! Runs vthreads with a netpoller, a timer wheel and work stealing.

pub mod poller;
mod sched;
pub mod stacks;
pub mod switch;
mod sys;
mod task;
pub mod timers;

#[cfg(test)]
mod tests;

pub use poller::Interest;
pub use sched::{
    cleanup_begin, current, direct_stack_limit, exit, foreign_source, in_a_task, now, on_os_stack, park, park_on, park_timeout, poll_point, run_single, scavenge, sleep, spawn, spawn_named, stack_trap, wait_fd,
    wait_readable, wait_writable, yield_now, Config, Deadlock, FdError, ForeignSource, ParkedTask, Poll, SchedStats, Scheduler, TaskHandle, TaskState, TraceEvent, Trap, Wake,
};
pub use stacks::{Slab, Stack, MAIN_STACK_BYTES, RED_ZONE, TASK_STACK_BYTES};
pub use sys::SysError;
