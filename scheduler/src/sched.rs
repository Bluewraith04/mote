//! The scheduler: workers with their queues, the task state machine, parking, timers, descriptors, preemption, seeded schedules and the deadlock report.

use std::any::Any;
use std::cell::{Cell, UnsafeCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use mote_abi::TaskContext;

use crate::poller::{Interest, Poller};
use crate::stacks::{Slab, MAIN_STACK_BYTES, TASK_STACK_BYTES};
use crate::switch;
use crate::sys::SysError;
use crate::task::{state_name, Completion, Task, COMPACTING, DONE, PARKED, PARKING, READY, RUNNING, WOKEN};
use crate::timers::{Clock, TimerId, Wheel};

const NO_TIMER: u64 = u64::MAX;

/// Bytes of the OS stack of a worker, on which runtime and native calls run.
const WORKER_STACK_BYTES: usize = 8 << 20;

/// The most polls a seeded task runs before it is preempted.
const MAX_SEEDED_POLLS: u64 = 16;

/// Nanoseconds a task a worker queued for itself waits before another worker may steal it.
const STEAL_AFTER: u64 = 50_000;

/// The most live stack bytes of a cold task that are copied out; a deeper stack keeps its pages.
const COLD_LIVE_CAP: usize = 64 * 1024;

/// How a scheduler is made.
#[derive(Clone, Debug)]
pub struct Config {
    pub workers: usize,
    /// Stack slots for spawned tasks; a task takes one when it first runs.
    pub spawn_slots: usize,
    pub spawn_stack_bytes: usize,
    pub main_stack_bytes: usize,
    /// Run time after which a task is asked to yield; zero turns preemption off.
    pub slice: Duration,
    /// One worker, a virtual clock, and a seeded order: one seed gives one schedule.
    pub seed: Option<u64>,
    /// Record a trace of the schedule.
    pub trace: bool,
    /// Parked time after which a task's stack is copied out and its pages are released; `None` never.
    pub cold_after: Option<Duration>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            workers: std::thread::available_parallelism().map_or(1, |count| count.get()),
            spawn_slots: 100_000,
            spawn_stack_bytes: TASK_STACK_BYTES,
            main_stack_bytes: MAIN_STACK_BYTES,
            slice: Duration::from_millis(10),
            seed: None,
            trace: false,
            cold_after: Some(Duration::from_secs(1)),
        }
    }

    pub fn seeded(seed: u64) -> Self {
        Self { workers: 1, slice: Duration::ZERO, seed: Some(seed), trace: true, cold_after: None, ..Self::new() }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a park returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    Woken,
    TimedOut,
    Cancelled,
}

/// What a poll point found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    Continue,
    Cancelled,
}

/// Why compiled code entered the runtime from the stack-limit check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    /// The stack reached its real limit.
    Overflow,
    /// The task was cancelled and the limit poisoned.
    Cancelled,
    /// Neither.
    Spurious,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Parked,
    Done,
}

#[derive(Debug)]
pub enum FdError {
    /// Another task waits for the same thing on the descriptor.
    Busy,
    Sys(i32),
}

/// A task that is not done when the system can make no progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParkedTask {
    pub id: u64,
    pub name: String,
    pub state: &'static str,
    pub waiting: String,
}

/// All tasks are parked with nothing that could wake one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deadlock {
    pub tasks: Vec<ParkedTask>,
}

impl fmt::Display for Deadlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "deadlock: {} tasks wait and nothing can wake them", self.tasks.len())?;
        for task in &self.tasks {
            write!(f, "\n  task {} `{}` ({}): {}", task.id, task.name, task.state, task.waiting)?;
        }
        Ok(())
    }
}

/// One step of a schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TraceEvent {
    Spawn(u64),
    Run(u64),
    Yield(u64),
    Park(u64),
    Wake(u64),
    Done(u64),
    /// The virtual clock jumped to this time.
    Jump(u64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchedStats {
    pub spawned: usize,
    pub switches: usize,
    pub steals: usize,
    pub timers_fired: usize,
    /// Parked stacks the compactor handled.
    pub compacted: usize,
}

enum Outcome {
    MainDone,
    Deadlock(Deadlock),
}

#[derive(Default)]
struct Idle {
    polling: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    None,
    Yield,
    Park,
    Done,
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut rng = Rng(seed ^ 0x9E37_79B9_7F4A_7C15);
        rng.next(1);
        rng
    }

    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) % below.max(1)
    }
}

pub(crate) struct Shared {
    config: Config,
    clock: Clock,
    main_slab: Slab,
    spawn_slab: Slab,
    queues: Vec<Mutex<VecDeque<Arc<Task>>>>,
    injector: Mutex<VecDeque<Arc<Task>>>,
    /// Tasks that were due to start and found every slot in use, oldest first.
    stack_waiters: Mutex<VecDeque<Arc<Task>>>,
    /// Tasks in a queue.
    queued: AtomicUsize,
    /// Tasks that are ready or running.
    runnable: AtomicUsize,
    live: AtomicUsize,
    next_id: AtomicU64,
    registry: Mutex<HashMap<u64, Arc<Task>>>,
    timers: Mutex<Wheel<Arc<Task>>>,
    next_timer: AtomicU64,
    poller: Poller<Arc<Task>>,
    idle: Mutex<Idle>,
    idle_cv: Condvar,
    sleepers: AtomicUsize,
    polling: AtomicBool,
    foreign: AtomicUsize,
    /// Threads that took timers off the wheel and are waking their tasks.
    waking: AtomicUsize,
    shutdown: AtomicBool,
    outcome: Mutex<Option<Outcome>>,
    outcome_cv: Condvar,
    rng: Mutex<Rng>,
    trace: Mutex<Vec<TraceEvent>>,
    current_context: Vec<AtomicPtr<TaskContext>>,
    switch_count: Vec<AtomicU64>,
    spawned: AtomicUsize,
    steals: AtomicUsize,
    timers_fired: AtomicUsize,
    compacted: AtomicUsize,
    /// Held while a stack is copied out or back, and by whoever reads a stack that is not running.
    stack_guard: Mutex<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().expect("a scheduler lock")
}

impl Shared {
    fn new(config: Config) -> Result<Arc<Self>, SysError> {
        let workers = config.workers.max(1);
        let seeded = config.seed.is_some();
        let clock = if seeded { Clock::virtual_time() } else { Clock::real() };
        Ok(Arc::new(Self {
            main_slab: Slab::new(1, config.main_stack_bytes)?,
            spawn_slab: Slab::new(config.spawn_slots, config.spawn_stack_bytes)?,
            queues: (0..workers).map(|_| Mutex::new(VecDeque::new())).collect(),
            injector: Mutex::new(VecDeque::new()),
            stack_waiters: Mutex::new(VecDeque::new()),
            queued: AtomicUsize::new(0),
            runnable: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            next_id: AtomicU64::new(1),
            registry: Mutex::new(HashMap::new()),
            timers: Mutex::new(Wheel::new(0)),
            next_timer: AtomicU64::new(NO_TIMER),
            poller: Poller::new()?,
            idle: Mutex::new(Idle::default()),
            idle_cv: Condvar::new(),
            sleepers: AtomicUsize::new(0),
            polling: AtomicBool::new(false),
            foreign: AtomicUsize::new(0),
            waking: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            outcome: Mutex::new(None),
            outcome_cv: Condvar::new(),
            rng: Mutex::new(Rng::new(config.seed.unwrap_or(0))),
            trace: Mutex::new(Vec::new()),
            current_context: (0..workers).map(|_| AtomicPtr::new(ptr::null_mut())).collect(),
            switch_count: (0..workers).map(|_| AtomicU64::new(0)).collect(),
            spawned: AtomicUsize::new(0),
            steals: AtomicUsize::new(0),
            timers_fired: AtomicUsize::new(0),
            compacted: AtomicUsize::new(0),
            stack_guard: Mutex::new(()),
            clock,
            config,
        }))
    }

    fn seeded(&self) -> bool {
        self.config.seed.is_some()
    }

    fn trace(&self, event: TraceEvent) {
        if self.config.trace {
            lock(&self.trace).push(event);
        }
    }

    fn spawn_task(self: &Arc<Self>, name: String, entry: Box<dyn FnOnce() + Send>, is_main: bool) -> Arc<Task> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let polls_left = if self.seeded() { lock(&self.rng).next(MAX_SEEDED_POLLS) + 1 } else { 0 };
        let task = Arc::new(Task {
            id,
            name,
            is_main,
            state: std::sync::atomic::AtomicU8::new(READY),
            sp: UnsafeCell::new(0),
            stack: UnsafeCell::new(None),
            snapshot: UnsafeCell::new(None),
            entry: UnsafeCell::new(Some(entry)),
            context: Box::new(UnsafeCell::new(TaskContext::default())),
            real_limit: AtomicUsize::new(0),
            parked_at: AtomicU64::new(0),
            ready_at: AtomicU64::new(0),
            cold: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            cleaning: AtomicBool::new(false),
            cancel_lock: Mutex::new(()),
            waiting: Mutex::new(String::new()),
            completion: Mutex::new(Completion::default()),
            finished: Condvar::new(),
            panic: Mutex::new(None),
            polls_left: AtomicU64::new(polls_left),
        });
        if self.seeded() {
            task.poll_word().store(1, Ordering::SeqCst);
        }
        lock(&self.registry).insert(id, task.clone());
        self.spawned.fetch_add(1, Ordering::SeqCst);
        self.live.fetch_add(1, Ordering::SeqCst);
        self.runnable.fetch_add(1, Ordering::SeqCst);
        self.trace(TraceEvent::Spawn(id));
        self.push_ready(task.clone(), true);
        task
    }

    fn slab_of(&self, task: &Task) -> &Slab {
        if task.is_main { &self.main_slab } else { &self.spawn_slab }
    }

    /// Gives a task that has not run its slot and first frame; false when every slot is in use, and the task then waits for one.
    fn start(&self, task: &Arc<Task>) -> bool {
        let slab = self.slab_of(task);
        let stack = match slab.acquire() {
            Some(stack) => stack,
            None => {
                // Looking again under the lock keeps a slot released meanwhile from being missed: `finish` takes the lock after releasing.
                let mut waiters = lock(&self.stack_waiters);
                match slab.acquire() {
                    Some(stack) => stack,
                    None => {
                        *lock(&task.waiting) = "a free stack".to_string();
                        waiters.push_back(task.clone());
                        self.runnable.fetch_sub(1, Ordering::SeqCst);
                        return false;
                    }
                }
            }
        };
        let limit = stack.limit();
        task.real_limit.store(limit, Ordering::SeqCst);
        // A cancellation that came first has poisoned the word, and stays.
        let _ = task.stack_limit_word().compare_exchange(0, limit as u64, Ordering::SeqCst, Ordering::SeqCst);
        // SAFETY: the stack is a fresh slot with a 16-aligned top, `task_entry` never returns, and the task is on no queue or worker but this one.
        unsafe {
            *task.sp.get() = switch::prepare_range(stack.base(), stack.top(), task_entry, Arc::as_ptr(task) as usize);
            *task.stack.get() = Some(stack);
        }
        true
    }

    /// Queues a ready task: on the queue of the worker that calls this, else on the shared one. A task that is not `eager` stays with the calling worker, which most often parks next; an idle worker is woken only when the queue holds a second task, and steals the first once it has waited `STEAL_AFTER`.
    fn push_ready(self: &Arc<Self>, task: Arc<Task>, eager: bool) {
        let own = match current_worker() {
            Some(worker) if !self.seeded() && Arc::ptr_eq(&worker.shared, self) => Some(worker.index),
            _ => None,
        };
        let mut wake = true;
        match own {
            Some(index) => {
                task.ready_at.store(if eager { 0 } else { self.clock.now().max(1) }, Ordering::SeqCst);
                let mut queue = lock(&self.queues[index]);
                queue.push_back(task);
                wake = eager || queue.len() > 1;
            }
            None => {
                task.ready_at.store(0, Ordering::SeqCst);
                lock(&self.injector).push_back(task);
            }
        }
        self.queued.fetch_add(1, Ordering::SeqCst);
        if wake {
            self.wake_one();
        }
    }

    /// Makes sure a worker that is idle looks for work.
    fn wake_one(&self) {
        if self.sleepers.load(Ordering::SeqCst) > 0 {
            let _idle = lock(&self.idle);
            self.idle_cv.notify_one();
        } else if self.polling.load(Ordering::SeqCst) {
            self.poller.wake();
        }
    }

    fn wake_all(&self) {
        {
            let _idle = lock(&self.idle);
            self.idle_cv.notify_all();
        }
        self.poller.wake();
    }

    /// Wakes a parked task, or leaves a token for the park of a running one; callable from any thread.
    pub(crate) fn unpark(self: &Arc<Self>, task: &Arc<Task>) {
        loop {
            let state = task.state();
            match state {
                PARKED => {
                    if task.state.compare_exchange(PARKED, READY, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                        self.runnable.fetch_add(1, Ordering::SeqCst);
                        self.trace(TraceEvent::Wake(task.id));
                        self.push_ready(task.clone(), false);
                        return;
                    }
                }
                RUNNING | PARKING | COMPACTING => {
                    if task.state.compare_exchange(state, WOKEN, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                        return;
                    }
                }
                _ => return,
            }
        }
    }

    fn find_task(&self, index: usize, tick: u64) -> Option<Arc<Task>> {
        if self.seeded() {
            let mut injector = lock(&self.injector);
            if injector.is_empty() {
                return None;
            }
            let at = lock(&self.rng).next(injector.len() as u64) as usize;
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return injector.remove(at);
        }
        if tick.is_multiple_of(61) && let Some(task) = lock(&self.injector).pop_front() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return Some(task);
        }
        if let Some(task) = lock(&self.queues[index]).pop_front() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return Some(task);
        }
        if let Some(task) = lock(&self.injector).pop_front() {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return Some(task);
        }
        let workers = self.queues.len();
        for step in 1..workers {
            let victim = (index + step) % workers;
            let mut stolen: Vec<Arc<Task>> = {
                let mut queue = lock(&self.queues[victim]);
                if queue.front().is_some_and(|oldest| {
                    let since = oldest.ready_at.load(Ordering::SeqCst);
                    since != 0 && self.clock.now().saturating_sub(since) < STEAL_AFTER
                }) {
                    continue;
                }
                let take = queue.len().div_ceil(2);
                let from = queue.len() - take;
                queue.drain(from..).collect()
            };
            if stolen.is_empty() {
                continue;
            }
            self.steals.fetch_add(1, Ordering::SeqCst);
            let first = stolen.remove(0);
            self.queued.fetch_sub(1, Ordering::SeqCst);
            lock(&self.queues[index]).extend(stolen);
            return Some(first);
        }
        None
    }

    fn timers_due(&self) -> bool {
        let next = self.next_timer.load(Ordering::SeqCst);
        next != NO_TIMER && next <= self.clock.now()
    }

    fn refresh_next_timer(&self, wheel: &Wheel<Arc<Task>>) {
        self.next_timer.store(wheel.next_deadline().unwrap_or(NO_TIMER), Ordering::SeqCst);
    }

    fn service_timers(self: &Arc<Self>) {
        if !self.timers_due() {
            return;
        }
        self.waking.fetch_add(1, Ordering::SeqCst);
        let fired = {
            let mut wheel = lock(&self.timers);
            let fired = wheel.advance(self.clock.now());
            self.refresh_next_timer(&wheel);
            fired
        };
        self.timers_fired.fetch_add(fired.len(), Ordering::SeqCst);
        for task in fired {
            self.unpark(&task);
        }
        self.waking.fetch_sub(1, Ordering::SeqCst);
    }

    /// Copies out the stacks of tasks parked past `cold_after` and gives their pages back; the number handled.
    pub(crate) fn scavenge(self: &Arc<Self>) -> usize {
        let Some(after) = self.config.cold_after else { return 0 };
        let (now, after) = (self.clock.now(), after.as_nanos() as u64);
        let cold: Vec<Arc<Task>> = lock(&self.registry)
            .values()
            .filter(|task| task.state() == PARKED && !task.cold.load(Ordering::SeqCst) && now.saturating_sub(task.parked_at.load(Ordering::SeqCst)) >= after)
            .cloned()
            .collect();
        cold.iter().filter(|task| self.compact(task)).count()
    }

    /// Releases the pages of one parked task, keeping what it needs to resume; false when it is no longer parked.
    fn compact(self: &Arc<Self>, task: &Arc<Task>) -> bool {
        // Counted before the state changes, so a wake that lands while the stack is copied is never seen as no wake at all.
        self.waking.fetch_add(1, Ordering::SeqCst);
        if task.state.compare_exchange(PARKED, COMPACTING, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            self.waking.fetch_sub(1, Ordering::SeqCst);
            return false;
        }
        // Another pass may have handled this park since the candidates were chosen; copying the emptied stack again would lose the snapshot.
        let handled = !task.cold.load(Ordering::SeqCst);
        if handled {
            let _guard = lock(&self.stack_guard);
            // SAFETY: the task is off every worker and only this thread holds it in `COMPACTING`; a wake only changes the state.
            unsafe {
                if let Some(stack) = (*task.stack.get()).as_ref() {
                    let sp = *task.sp.get();
                    if stack.top() - sp <= COLD_LIVE_CAP {
                        *task.snapshot.get() = Some(std::slice::from_raw_parts(sp as *const u8, stack.top() - sp).into());
                        stack.discard_all();
                    } else {
                        stack.discard_below(sp);
                    }
                }
            }
            task.cold.store(true, Ordering::SeqCst);
            self.compacted.fetch_add(1, Ordering::SeqCst);
        }
        if task.state.compare_exchange(COMPACTING, PARKED, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            task.state.store(READY, Ordering::SeqCst);
            self.runnable.fetch_add(1, Ordering::SeqCst);
            self.trace(TraceEvent::Wake(task.id));
            self.push_ready(task.clone(), true);
        }
        self.waking.fetch_sub(1, Ordering::SeqCst);
        handled
    }

    fn add_timer(&self, deadline: u64, task: Arc<Task>) -> TimerId {
        let id = {
            let mut wheel = lock(&self.timers);
            let id = wheel.insert(deadline, task);
            self.refresh_next_timer(&wheel);
            id
        };
        if self.polling.load(Ordering::SeqCst) {
            self.poller.wake();
        }
        id
    }

    /// Disarms a timer; whether it was still armed.
    fn cancel_timer(&self, id: TimerId) -> bool {
        let mut wheel = lock(&self.timers);
        let armed = wheel.cancel(id).is_some();
        self.refresh_next_timer(&wheel);
        armed
    }

    fn poll_timeout_ms(&self) -> i32 {
        let next = self.next_timer.load(Ordering::SeqCst);
        if next == NO_TIMER || self.clock.is_virtual() {
            return -1;
        }
        let delta = next.saturating_sub(self.clock.now());
        i32::try_from(delta.div_ceil(1_000_000)).unwrap_or(i32::MAX)
    }

    fn deadlock(&self) -> Option<Deadlock> {
        let stuck = self.runnable.load(Ordering::SeqCst) == 0
            && self.queued.load(Ordering::SeqCst) == 0
            && self.next_timer.load(Ordering::SeqCst) == NO_TIMER
            && self.poller.waiting() == 0
            && self.foreign.load(Ordering::SeqCst) == 0
            && self.waking.load(Ordering::SeqCst) == 0
            && self.live.load(Ordering::SeqCst) > 0;
        if !stuck {
            return None;
        }
        let mut tasks: Vec<ParkedTask> = lock(&self.registry)
            .values()
            .filter(|task| task.state() != DONE)
            .map(|task| ParkedTask { id: task.id, name: task.name.clone(), state: state_name(task.state()), waiting: lock(&task.waiting).clone() })
            .collect();
        tasks.sort_by_key(|task| task.id);
        Some(Deadlock { tasks })
    }

    /// Waits for work; false when the scheduler is over.
    fn idle(self: &Arc<Self>) -> bool {
        let mut idle = lock(&self.idle);
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                return false;
            }
            if self.queued.load(Ordering::SeqCst) > 0 || self.timers_due() {
                return true;
            }
            if let Some(report) = self.deadlock() {
                lock(&self.outcome).get_or_insert(Outcome::Deadlock(report));
                self.outcome_cv.notify_all();
                self.shutdown.store(true, Ordering::SeqCst);
                drop(idle);
                self.wake_all();
                return false;
            }
            let next = self.next_timer.load(Ordering::SeqCst);
            if self.clock.is_virtual() && next != NO_TIMER && self.poller.waiting() == 0 && self.foreign.load(Ordering::SeqCst) == 0 {
                self.clock.advance_to(next);
                self.trace(TraceEvent::Jump(next));
                return true;
            }
            if !idle.polling {
                idle.polling = true;
                self.polling.store(true, Ordering::SeqCst);
                let skip = self.queued.load(Ordering::SeqCst) > 0 || self.shutdown.load(Ordering::SeqCst);
                let timeout = self.poll_timeout_ms();
                drop(idle);
                let mut ready = Vec::new();
                if !skip {
                    self.poller.poll(timeout, &mut ready);
                }
                let woken = ready.len();
                for task in ready {
                    self.unpark(&task);
                }
                self.poller.settle(woken);
                idle = lock(&self.idle);
                idle.polling = false;
                self.polling.store(false, Ordering::SeqCst);
                if self.sleepers.load(Ordering::SeqCst) > 0 {
                    self.idle_cv.notify_one();
                }
                return true;
            }
            self.sleepers.fetch_add(1, Ordering::SeqCst);
            if self.queued.load(Ordering::SeqCst) > 0 || self.shutdown.load(Ordering::SeqCst) {
                self.sleepers.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            idle = self.idle_cv.wait(idle).expect("the idle lock");
            self.sleepers.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Ends a task that has switched away for the last time.
    fn finish(self: &Arc<Self>, task: &Arc<Task>) {
        task.state.store(DONE, Ordering::SeqCst);
        if task.is_main {
            // First, so that the count of runnable tasks falling to zero is not read as a deadlock.
            *lock(&self.outcome) = Some(Outcome::MainDone);
            self.outcome_cv.notify_all();
        }
        let joiners = {
            let mut completion = lock(&task.completion);
            completion.finished = true;
            std::mem::take(&mut completion.joiners)
        };
        for joiner in &joiners {
            self.unpark(joiner);
        }
        task.finished.notify_all();
        // SAFETY: the task is off its stack for good and its worker is the only one touching the cell.
        if let Some(stack) = unsafe { (*task.stack.get()).take() } {
            self.slab_of(task).release(stack);
            if !task.is_main && let Some(waiter) = lock(&self.stack_waiters).pop_front() {
                // Counted before this task stops being counted, so the pair never reads as nothing runnable.
                self.runnable.fetch_add(1, Ordering::SeqCst);
                self.push_ready(waiter, true);
            }
        }
        lock(&self.registry).remove(&task.id);
        self.trace(TraceEvent::Done(task.id));
        self.live.fetch_sub(1, Ordering::SeqCst);
        self.runnable.fetch_sub(1, Ordering::SeqCst);
        if !task.is_main && self.live.load(Ordering::SeqCst) > 0 {
            // Let an idle worker see whether this was the last thing that could run.
            self.wake_one();
        }
    }
}

/// The thread of one worker: its saved stack pointer, the task it runs and what that task asked for.
pub(crate) struct Worker {
    shared: Arc<Shared>,
    index: usize,
    sched_sp: UnsafeCell<usize>,
    current: Cell<*const Task>,
    action: Cell<Action>,
}

thread_local! {
    static WORKER: Cell<*const Worker> = const { Cell::new(ptr::null()) };
}

/// The worker of this thread; read afresh each call because a task may move between threads at a switch.
#[inline(never)]
fn worker_pointer() -> *const Worker {
    WORKER.with(Cell::get)
}

fn current_worker<'a>() -> Option<&'a Worker> {
    // SAFETY: the pointer is the address of a `Worker` that lives on the stack of this thread's loop.
    unsafe { worker_pointer().as_ref() }
}

fn current_task<'a>() -> Option<&'a Task> {
    // SAFETY: the pointer is a task the worker holds an `Arc` of while it runs.
    current_worker().and_then(|worker| unsafe { worker.current.get().as_ref() })
}

fn current_arc() -> Option<Arc<Task>> {
    let task = current_task()?;
    let pointer = ptr::from_ref(task);
    // SAFETY: `pointer` came from an `Arc` the worker holds, so the count is live.
    unsafe {
        Arc::increment_strong_count(pointer);
        Some(Arc::from_raw(pointer))
    }
}

impl Worker {
    /// Runs the task until it switches back, then does what it asked.
    fn run(&self, task: Arc<Task>) {
        let shared = &self.shared;
        // SAFETY: a task off every worker has no one else reading its cells; a task without a stack has not run.
        if unsafe { (*task.stack.get()).is_none() } && !shared.start(&task) {
            return;
        }
        // SAFETY: as above; the snapshot is the bytes the compactor took from the stack at the saved pointer, which is mapped and empty now.
        unsafe {
            let _guard = lock(&shared.stack_guard);
            let snapshot = (*task.snapshot.get()).take();
            if (snapshot.is_some() || task.cold.load(Ordering::SeqCst))
                && let Some(stack) = (*task.stack.get()).as_ref()
            {
                crate::sys::prepare_stack_window(stack.base(), *task.sp.get(), stack.top());
            }
            if let Some(bytes) = snapshot {
                ptr::copy_nonoverlapping(bytes.as_ptr(), *task.sp.get() as *mut u8, bytes.len());
            }
        }
        task.state.store(RUNNING, Ordering::SeqCst);
        self.current.set(Arc::as_ptr(&task));
        shared.current_context[self.index].store(task.context.get(), Ordering::SeqCst);
        shared.switch_count[self.index].fetch_add(1, Ordering::SeqCst);
        shared.trace(TraceEvent::Run(task.id));
        // SAFETY: the task's saved pointer is a prepared or suspended stack nothing runs on, and `sched_sp` outlives the call.
        unsafe { switch::switch(self.sched_sp.get(), *task.sp.get()) };
        shared.current_context[self.index].store(ptr::null_mut(), Ordering::SeqCst);
        self.current.set(ptr::null());
        match self.action.replace(Action::None) {
            Action::Yield => {
                task.state.store(READY, Ordering::SeqCst);
                shared.trace(TraceEvent::Yield(task.id));
                shared.push_ready(task, false);
            }
            Action::Park => {
                task.parked_at.store(shared.clock.now(), Ordering::SeqCst);
                task.cold.store(false, Ordering::SeqCst);
                if task.state.compare_exchange(PARKING, PARKED, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    shared.trace(TraceEvent::Park(task.id));
                    shared.runnable.fetch_sub(1, Ordering::SeqCst);
                } else {
                    task.state.store(READY, Ordering::SeqCst);
                    shared.push_ready(task, false);
                }
            }
            Action::Done => shared.finish(&task),
            Action::None => unreachable!("a task switches back with a request"),
        }
    }
}

fn worker_main(shared: Arc<Shared>, index: usize) {
    let worker = Worker { shared: shared.clone(), index, sched_sp: UnsafeCell::new(0), current: Cell::new(ptr::null()), action: Cell::new(Action::None) };
    WORKER.with(|slot| slot.set(&raw const worker));
    let mut tick = 0u64;
    while !shared.shutdown.load(Ordering::SeqCst) {
        shared.service_timers();
        tick += 1;
        match shared.find_task(index, tick) {
            Some(task) => worker.run(task),
            None => {
                if !shared.idle() {
                    break;
                }
            }
        }
    }
    WORKER.with(|slot| slot.set(ptr::null()));
}

/// Sets every running task that has run a whole slice to ask for a yield at its next poll.
fn ticker(shared: Arc<Shared>) {
    let workers = shared.current_context.len();
    let mut last = vec![0u64; workers];
    while !shared.shutdown.load(Ordering::SeqCst) {
        std::thread::sleep(shared.config.slice);
        if shared.queued.load(Ordering::SeqCst) > 0 {
            shared.wake_one();
        }
        for (index, seen) in last.iter_mut().enumerate() {
            let switches = shared.switch_count[index].load(Ordering::SeqCst);
            let context = shared.current_context[index].load(Ordering::SeqCst);
            if switches == *seen && !context.is_null() {
                // SAFETY: a context stays allocated while its task is current on the worker; the word is shared atomically.
                unsafe { AtomicU64::from_ptr(&raw mut (*context).poll) }.store(1, Ordering::SeqCst);
            }
            *seen = switches;
        }
    }
}

/// Releases the stacks of tasks that stay parked, a pass every half of `cold_after`.
fn scavenger(shared: Arc<Shared>, after: Duration) {
    const STEP: Duration = Duration::from_millis(10);
    let every = (after / 2).clamp(STEP, Duration::from_millis(500));
    let mut waited = Duration::ZERO;
    while !shared.shutdown.load(Ordering::SeqCst) {
        std::thread::sleep(STEP);
        waited += STEP;
        if waited >= every {
            waited = Duration::ZERO;
            shared.scavenge();
        }
    }
}

/// Where every task starts, on its own stack.
extern "C" fn task_entry(arg: usize) -> ! {
    // SAFETY: `arg` is the address of the task, which the worker keeps alive while it runs.
    let task = unsafe { &*(arg as *const Task) };
    // SAFETY: only this task's own start takes the closure.
    let entry = unsafe { (*task.entry.get()).take() }.expect("a task starts once");
    if let Err(payload) = catch_unwind(AssertUnwindSafe(entry)) {
        *lock(&task.panic) = Some(payload);
    }
    block(Action::Done);
    unreachable!("a finished task is never resumed");
}

/// Ends the calling task here: its frames are abandoned and no destructor above this call runs.
pub fn exit() -> ! {
    if current_worker().is_some() {
        block(Action::Done);
    } else if !DIRECT.with(Cell::get).is_null() {
        direct_return();
    } else {
        panic!("a task runs on a worker");
    }
    unreachable!("a finished task is never resumed");
}

struct DirectState {
    return_sp: usize,
    limit: usize,
}

thread_local! {
    static DIRECT: Cell<*mut DirectState> = const { Cell::new(ptr::null_mut()) };
}

struct DirectFrame<F> {
    func: Option<F>,
    panic: Option<Box<dyn Any + Send>>,
}

extern "C" fn direct_entry<F: FnOnce()>(arg: usize) -> ! {
    // SAFETY: `arg` points to `DirectFrame<F>` live in `run_single`'s suspended stack frame.
    let frame = unsafe { &mut *(arg as *mut DirectFrame<F>) };
    let func = frame.func.take().expect("a single task starts once");
    if let Err(payload) = catch_unwind(AssertUnwindSafe(func)) {
        frame.panic = Some(payload);
    }
    direct_return();
}

fn direct_return() -> ! {
    let direct = DIRECT.with(Cell::get);
    assert!(!direct.is_null(), "exit called outside a task");
    let mut dummy_sp = 0usize;
    // SAFETY: `direct.return_sp` holds the OS stack pointer saved by `run_single`; `dummy_sp` discards the abandoned task stack.
    unsafe { switch::switch(&raw mut dummy_sp, (*direct).return_sp) };
    unreachable!("a finished task is never resumed");
}

/// The stack limit of the active single-threaded run, when running inside `run_single`.
pub fn direct_stack_limit() -> Option<usize> {
    let direct = DIRECT.with(Cell::get);
    if direct.is_null() {
        None
    } else {
        // SAFETY: `direct` points to `DirectState` live on `run_single`'s suspended stack.
        Some(unsafe { (*direct).limit })
    }
}

fn direct_entry_for<F: FnOnce()>(_frame: &DirectFrame<F>) -> extern "C" fn(usize) -> ! {
    direct_entry::<F>
}

/// Runs `main` directly on one mapped stack without worker threads, ticker, scavenger, or poller; returns `None` when `main` ends via `exit`.
pub fn run_single<R>(main_stack_bytes: usize, main: impl FnOnce() -> R) -> Result<Option<R>, SysError> {
    assert!(current_worker().is_none() && DIRECT.with(Cell::get).is_null(), "run_single cannot nest inside a task");
    let slab = Slab::new(1, main_stack_bytes)?;
    let stack = slab.acquire().expect("a single-task slab holds one stack");
    let limit = stack.limit();
    let mut result: Option<R> = None;
    let mut frame = DirectFrame {
        func: Some(|| {
            result = Some(main());
        }),
        panic: None,
    };
    let entry = direct_entry_for(&frame);
    // SAFETY: `stack` is freshly mapped with a 16-aligned top, `direct_entry` never returns, and `frame` stays live while `switch` runs.
    let task_sp = unsafe { switch::prepare_range(stack.base(), stack.top(), entry, &raw mut frame as usize) };
    let mut direct = DirectState { return_sp: 0, limit };
    struct DirectGuard(*mut DirectState);
    impl Drop for DirectGuard {
        fn drop(&mut self) {
            DIRECT.with(|slot| slot.set(self.0));
        }
    }
    let guard = DirectGuard(DIRECT.with(|slot| {
        let prev = slot.get();
        slot.set(&raw mut direct);
        prev
    }));
    // SAFETY: `direct.return_sp` stores this thread's stack pointer; `task_sp` enters `direct_entry` on `stack`.
    unsafe { switch::switch(&raw mut direct.return_sp, task_sp) };
    drop(guard);
    slab.release(stack);
    // SAFETY: the direct task has finished and nothing runs on this slab again.
    unsafe { slab.unmap() };
    if let Some(payload) = frame.panic {
        resume_unwind(payload);
    }
    Ok(result)
}

/// Switches from the running task to its worker with a request.
fn block(action: Action) {
    let worker = current_worker().expect("a task runs on a worker");
    let task = current_task().expect("a task is current");
    worker.action.set(action);
    // SAFETY: the worker saved its pointer in `sched_sp` when it switched here; this task's own pointer is stored for its next resume.
    unsafe { switch::switch(task.sp.get(), *worker.sched_sp.get()) };
}

fn in_task() -> &'static Task {
    current_task().expect("this call needs a task")
}

/// What a scheduler task can be told from anywhere.
#[derive(Clone)]
pub struct TaskHandle {
    task: Arc<Task>,
    shared: Arc<Shared>,
}

impl fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TaskHandle({} `{}`)", self.task.id, self.task.name)
    }
}

impl TaskHandle {
    pub fn id(&self) -> u64 {
        self.task.id
    }

    pub fn name(&self) -> &str {
        &self.task.name
    }

    pub fn state(&self) -> TaskState {
        match self.task.state() {
            READY => TaskState::Ready,
            RUNNING | PARKING | WOKEN => TaskState::Running,
            PARKED | COMPACTING => TaskState::Parked,
            _ => TaskState::Done,
        }
    }

    pub fn is_done(&self) -> bool {
        self.task.state() == DONE
    }

    /// Wakes the task if it is parked; if it is running, its next park returns at once. Callable from any thread.
    pub fn unpark(&self) {
        self.shared.unpark(&self.task);
    }

    /// Marks the task cancelled, sets its poll flag, poisons its stack limit and wakes it.
    pub fn cancel(&self) {
        {
            let _order = lock(&self.task.cancel_lock);
            if self.task.cleaning.load(Ordering::SeqCst) {
                return;
            }
            self.task.cancelled.store(true, Ordering::SeqCst);
            self.task.poll_word().store(1, Ordering::SeqCst);
            self.task.stack_limit_word().store(u64::MAX, Ordering::SeqCst);
        }
        self.shared.unpark(&self.task);
    }

    pub fn is_cancelled(&self) -> bool {
        self.task.cancelled.load(Ordering::SeqCst)
    }

    /// Parks the calling task until this one ends; the panic message when it panicked.
    pub fn join(&self) -> Result<(), String> {
        let me = current_arc().expect("join needs a task; a thread uses wait");
        loop {
            {
                let mut completion = lock(&self.task.completion);
                if completion.finished {
                    break;
                }
                completion.joiners.push(me.clone());
            }
            if park_on("join") == Wake::Cancelled {
                return Err("the joining task was cancelled".to_string());
            }
        }
        self.outcome()
    }

    /// Blocks the calling thread until the task ends.
    pub fn wait(&self) -> Result<(), String> {
        let mut completion = lock(&self.task.completion);
        while !completion.finished {
            completion = self.task.finished.wait(completion).expect("the completion lock");
        }
        drop(completion);
        self.outcome()
    }

    fn outcome(&self) -> Result<(), String> {
        match lock(&self.task.panic).as_ref() {
            None => Ok(()),
            Some(payload) => Err(payload.downcast_ref::<&str>().map(|text| text.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic".to_string())),
        }
    }

    /// The context block compiled code reads for this task.
    pub fn context(&self) -> *mut TaskContext {
        self.task.context.get()
    }

    /// The lowest and highest address of the task's stack; `None` once it has ended.
    pub fn stack_bounds(&self) -> Option<(usize, usize)> {
        // SAFETY: the cell changes only when the task ends, and a stale read is a stack that is about to go.
        unsafe { (*self.task.stack.get()).as_ref() }.map(|stack| (stack.base(), stack.top()))
    }

    /// Calls `f` with the saved stack pointer and the bytes from it to the top of the slot when a compactor holds them in place of the pages; the stack is not copied out or back meanwhile.
    pub fn with_snapshot<R>(&self, f: impl FnOnce(Option<(usize, &[u8])>) -> R) -> R {
        let _guard = lock(&self.shared.stack_guard);
        // SAFETY: the guard keeps the compactor and the worker that restores from changing the cells.
        let found = unsafe { (*self.task.snapshot.get()).as_deref().map(|bytes| (*self.task.sp.get(), bytes)) };
        f(found)
    }

    /// The stack pointer a parked task was suspended at: the live part of its stack is from here to the top.
    pub fn saved_sp(&self) -> Option<usize> {
        // SAFETY: a parked task is off every worker, so nothing writes its saved pointer.
        (self.task.state() == PARKED).then(|| unsafe { *self.task.sp.get() })
    }
}

/// Keeps the scheduler from calling a wait a deadlock while a thread outside it may still wake a task.
pub struct ForeignSource {
    shared: Arc<Shared>,
}

impl Drop for ForeignSource {
    fn drop(&mut self) {
        self.shared.foreign.fetch_sub(1, Ordering::SeqCst);
        self.shared.wake_all();
    }
}

fn foreign_source_of(shared: &Arc<Shared>) -> ForeignSource {
    shared.foreign.fetch_add(1, Ordering::SeqCst);
    ForeignSource { shared: shared.clone() }
}

/// A scheduler: workers, a clock and a set of tasks.
pub struct Scheduler {
    shared: Arc<Shared>,
    started: AtomicBool,
}

impl Scheduler {
    pub fn new(config: Config) -> Result<Self, SysError> {
        Ok(Self { shared: Shared::new(config)?, started: AtomicBool::new(false) })
    }

    /// Runs `main` as the main task on the workers; it returns when `main` does, or with the report when nothing can make progress.
    pub fn run<R: Send + 'static>(&self, main: impl FnOnce() -> R + Send + 'static) -> Result<R, Deadlock> {
        self.run_or_exit(main).map(|result| result.expect("main stored its result"))
    }

    /// Like `run`, and `None` when `main` ended through `exit`.
    pub fn run_or_exit<R: Send + 'static>(&self, main: impl FnOnce() -> R + Send + 'static) -> Result<Option<R>, Deadlock> {
        assert!(!self.started.swap(true, Ordering::SeqCst), "a scheduler runs once");
        let shared = &self.shared;
        let result: Arc<Mutex<Option<R>>> = Arc::new(Mutex::new(None));
        let slot = result.clone();
        let main_task = shared.spawn_task("main".to_string(), Box::new(move || *lock(&slot) = Some(main())), true);
        let workers = shared.queues.len();
        std::thread::scope(|scope| {
            for index in 0..workers {
                let shared = shared.clone();
                std::thread::Builder::new().name(format!("mote-worker-{index}")).stack_size(WORKER_STACK_BYTES).spawn_scoped(scope, move || worker_main(shared, index)).expect("a worker thread");
            }
            if shared.config.slice > Duration::ZERO && !shared.seeded() {
                let shared = shared.clone();
                scope.spawn(move || ticker(shared));
            }
            if let Some(after) = shared.config.cold_after && !shared.seeded() {
                let shared = shared.clone();
                scope.spawn(move || scavenger(shared, after));
            }
            let mut outcome = lock(&shared.outcome);
            while outcome.is_none() {
                outcome = shared.outcome_cv.wait(outcome).expect("the outcome lock");
            }
            drop(outcome);
            shared.shutdown.store(true, Ordering::SeqCst);
            shared.wake_all();
        });
        // SAFETY: the workers have stopped, so no task runs again; the frames of abandoned tasks are never read.
        unsafe {
            shared.main_slab.unmap();
            shared.spawn_slab.unmap();
        }
        let outcome = lock(&shared.outcome).take().expect("an outcome");
        match outcome {
            Outcome::Deadlock(report) => Err(report),
            Outcome::MainDone => {
                if let Some(payload) = lock(&main_task.panic).take() {
                    resume_unwind(payload);
                }
                Ok(lock(&result).take())
            }
        }
    }

    /// The schedule recorded when `Config::trace` is on.
    pub fn trace(&self) -> Vec<TraceEvent> {
        lock(&self.shared.trace).clone()
    }

    pub fn stats(&self) -> SchedStats {
        let shared = &self.shared;
        SchedStats {
            spawned: shared.spawned.load(Ordering::SeqCst),
            switches: shared.switch_count.iter().map(|count| count.load(Ordering::SeqCst) as usize).sum(),
            steals: shared.steals.load(Ordering::SeqCst),
            timers_fired: shared.timers_fired.load(Ordering::SeqCst),
            compacted: shared.compacted.load(Ordering::SeqCst),
        }
    }

    /// One pass of the compactor over the parked tasks; the number handled.
    pub fn scavenge(&self) -> usize {
        self.shared.scavenge()
    }

    /// Nanoseconds on the scheduler's clock.
    pub fn now(&self) -> u64 {
        self.shared.clock.now()
    }

    pub fn foreign_source(&self) -> ForeignSource {
        foreign_source_of(&self.shared)
    }

    pub fn stack_slots_in_use(&self) -> usize {
        self.shared.spawn_slab.in_use()
    }

    /// Whether the stacks of the run are still mapped.
    pub fn stacks_mapped(&self) -> bool {
        self.shared.spawn_slab.is_mapped() || self.shared.main_slab.is_mapped()
    }
}

fn shared_of_current() -> Arc<Shared> {
    current_worker().expect("this call needs a task").shared.clone()
}

/// A handle to the calling task.
pub fn current() -> TaskHandle {
    TaskHandle { task: current_arc().expect("this call needs a task"), shared: shared_of_current() }
}

/// Whether the calling code runs in a task.
pub fn in_a_task() -> bool {
    current_task().is_some()
}

pub fn spawn(f: impl FnOnce() + Send + 'static) -> TaskHandle {
    spawn_named("task", f)
}

pub fn spawn_named(name: &str, f: impl FnOnce() + Send + 'static) -> TaskHandle {
    let shared = shared_of_current();
    let task = shared.spawn_task(name.to_string(), Box::new(f), false);
    TaskHandle { task, shared }
}

/// One pass of the compactor: the stacks of tasks parked past `cold_after` are copied out and their pages released; the number handled.
pub fn scavenge() -> usize {
    shared_of_current().scavenge()
}

/// Keeps a wait from being called a deadlock while a thread outside the scheduler can still wake a task.
pub fn foreign_source() -> ForeignSource {
    foreign_source_of(&shared_of_current())
}

/// Gives the worker to another ready task and comes back.
pub fn yield_now() {
    block(Action::Yield);
}

/// Parks the calling task until it is woken; a wake may come without its cause, so callers check their condition.
pub fn park() -> Wake {
    park_on("an unnamed wait")
}

/// Like `park`, with what the task waits for, which the deadlock report shows.
pub fn park_on(reason: &str) -> Wake {
    let task = in_task();
    if task.cancelled.load(Ordering::SeqCst) {
        return Wake::Cancelled;
    }
    *lock(&task.waiting) = reason.to_string();
    match task.state.compare_exchange(RUNNING, PARKING, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => block(Action::Park),
        Err(WOKEN) => task.state.store(RUNNING, Ordering::SeqCst),
        Err(other) => unreachable!("a running task is not {}", state_name(other)),
    }
    // The task may be on another worker now; nothing from before the switch is reused.
    if in_task().cancelled.load(Ordering::SeqCst) { Wake::Cancelled } else { Wake::Woken }
}

/// Parks for at most `ns` nanoseconds of the scheduler's clock.
pub fn park_timeout(ns: u64, reason: &str) -> Wake {
    let shared = shared_of_current();
    let me = current_arc().expect("this call needs a task");
    let timer = shared.add_timer(shared.clock.now().saturating_add(ns), me);
    let wake = park_on(reason);
    let still_armed = shared.cancel_timer(timer);
    match wake {
        Wake::Cancelled => Wake::Cancelled,
        _ if !still_armed => Wake::TimedOut,
        _ => Wake::Woken,
    }
}

/// Sleeps `ns` nanoseconds of the scheduler's clock; `Cancelled` if the task is cancelled first.
pub fn sleep(ns: u64) -> Wake {
    let shared = shared_of_current();
    let deadline = shared.clock.now().saturating_add(ns);
    loop {
        let now = shared.clock.now();
        if now >= deadline {
            return Wake::TimedOut;
        }
        if park_timeout(deadline - now, "a sleep") == Wake::Cancelled {
            return Wake::Cancelled;
        }
    }
}

/// Nanoseconds on the scheduler's clock.
pub fn now() -> u64 {
    shared_of_current().clock.now()
}

/// Lets the calling cancelled task run code before it ends: it is no longer cancelled, its stack limit is restored, and a later `cancel` of it does nothing.
pub fn cleanup_begin() {
    let task = in_task();
    let _order = lock(&task.cancel_lock);
    task.cleaning.store(true, Ordering::SeqCst);
    task.cancelled.store(false, Ordering::SeqCst);
    task.poll_word().store(u64::from(shared_of_current().seeded()), Ordering::SeqCst);
    task.stack_limit_word().store(task.real_limit.load(Ordering::SeqCst) as u64, Ordering::SeqCst);
}

/// What compiled code's runtime entry calls when the poll flag is set: yields when it is a preemption, and reports a cancellation.
pub fn poll_point() -> Poll {
    let task = in_task();
    let flagged = task.poll_word().swap(0, Ordering::SeqCst) != 0;
    if task.cancelled.load(Ordering::SeqCst) {
        task.poll_word().store(1, Ordering::SeqCst);
        return Poll::Cancelled;
    }
    let shared = shared_of_current();
    let preempt = if shared.seeded() {
        task.poll_word().store(1, Ordering::SeqCst);
        let left = task.polls_left.load(Ordering::SeqCst);
        if left <= 1 {
            task.polls_left.store(lock(&shared.rng).next(MAX_SEEDED_POLLS) + 1, Ordering::SeqCst);
            true
        } else {
            task.polls_left.store(left - 1, Ordering::SeqCst);
            false
        }
    } else {
        flagged
    };
    if preempt {
        yield_now();
    }
    Poll::Continue
}

/// What compiled code's runtime entry calls when the stack-limit check fails with the stack pointer `sp`.
pub fn stack_trap(sp: usize) -> Trap {
    if let Some(task) = current_task() {
        if task.cancelled.load(Ordering::SeqCst) {
            Trap::Cancelled
        } else if sp < task.real_limit.load(Ordering::SeqCst) {
            Trap::Overflow
        } else {
            Trap::Spurious
        }
    } else if let Some(limit) = direct_stack_limit() {
        if sp < limit { Trap::Overflow } else { Trap::Spurious }
    } else {
        panic!("this call needs a task");
    }
}

/// Runs `f(arg)` on the worker's OS stack, for a runtime or native call that needs more stack than a task has; `f` does not park or yield.
pub fn on_os_stack(f: extern "C" fn(usize), arg: usize) {
    let worker = current_worker().expect("this call needs a task");
    // SAFETY: the worker is suspended in its switch with its pointer stored, the OS stack has room below it, and `f` does not switch.
    unsafe { switch::call_on(*worker.sched_sp.get(), f, arg) };
}

/// Parks until the descriptor is readable.
pub fn wait_readable(fd: i32) -> Result<Wake, FdError> {
    wait_fd(fd, Interest::Readable, None)
}

/// Parks until the descriptor is writable.
pub fn wait_writable(fd: i32) -> Result<Wake, FdError> {
    wait_fd(fd, Interest::Writable, None)
}

/// Parks until the descriptor is ready, `ns` have passed (`TimedOut`) or the task is cancelled.
pub fn wait_fd(fd: i32, interest: Interest, timeout: Option<u64>) -> Result<Wake, FdError> {
    let shared = shared_of_current();
    let me = current_arc().expect("this call needs a task");
    let deadline = timeout.map(|ns| shared.clock.now().saturating_add(ns));
    let reason = format!("{} on descriptor {fd}", if interest == Interest::Readable { "readable" } else { "writable" });
    loop {
        match shared.poller.register(fd, interest, me.clone()) {
            Err(error) => return Err(FdError::Sys(error.0)),
            Ok(Err(_)) => return Err(FdError::Busy),
            Ok(Ok(())) => {}
        }
        let timer = deadline.map(|at| shared.add_timer(at, me.clone()));
        let wake = park_on(&reason);
        let timed_out = timer.is_some_and(|timer| !shared.cancel_timer(timer));
        if shared.poller.cancel(fd, interest).is_none() {
            return Ok(Wake::Woken);
        }
        if wake == Wake::Cancelled {
            return Ok(Wake::Cancelled);
        }
        if timed_out || deadline.is_some_and(|at| shared.clock.now() >= at) {
            return Ok(Wake::TimedOut);
        }
    }
}
