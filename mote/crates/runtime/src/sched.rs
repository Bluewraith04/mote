//! The task scheduler. [`SchedState`] holds the run queue, parked tasks, free pool and id counters behind one lock on [`Runtime::sched`]; each worker thread runs [`drive_loop`].
//! A task is either published (in `run_queue` or `blocked`) or checked out by one worker. Cross-task operations go through [`TaskShared`], kept in `SchedState::registry` while the task lives.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use isa::value::{ObjectHeader, Value};

use crate::{Frame, Runtime, RuntimeRoots, ScopeFrame, TaskContext, VmStatus, DEFAULT_REDUCTION_BUDGET};

pub(crate) struct TaskShared {
    pub parent: Option<u64>,
    pub scope_depth: usize,
    pub cancelled: AtomicBool,
    pub scopes: Mutex<Vec<ScopeFrame>>,
}

// SAFETY: the object pointers in a `ScopeFrame` are read and written only under `scopes`' lock and name heap objects the collector keeps alive.
unsafe impl Send for TaskShared {}
unsafe impl Sync for TaskShared {}

impl TaskShared {
    pub(crate) fn new(parent: Option<u64>, scope_depth: usize) -> Self {
        TaskShared {
            parent,
            scope_depth,
            cancelled: AtomicBool::new(false),
            scopes: Mutex::new(Vec::new()),
        }
    }
}

/// Everything the scheduler mutates as one unit, behind [`Runtime::sched`]'s single lock.
#[derive(Default)]
pub struct SchedState {
    /// Ready unpinned tasks, oldest first.
    pub run_queue: ReadyQueue,
    /// Ready pinned tasks, oldest first.
    pub home_queue: ReadyQueue,
    /// Tasks parked on `SCOPEEXIT`, `join` or a channel `send` / `recv`; scanned as GC roots.
    pub blocked: HashMap<u64, TaskContext>,
    pub(crate) free_pool: Vec<TaskContext>,
    pub(crate) next_task_id: u64,
    pub(crate) context_switches: u64,
    pub(crate) next_channel_id: u64,
    pub(crate) send_waiters: HashMap<u64, VecDeque<u64>>,
    pub(crate) recv_waiters: HashMap<u64, VecDeque<u64>>,
    pub(crate) handoff_waiters: HashMap<u64, VecDeque<u64>>,
    pub(crate) next_cell_id: u64,
    pub(crate) writer_waiters: HashMap<u64, VecDeque<u64>>,
    pub(crate) watchers: HashMap<u64, Vec<u64>>,
    pub(crate) woken_early: HashSet<u64>,
    /// `Task<T>` handles of every task `SPAWN` left detached.
    pub detached: HashSet<NonNull<ObjectHeader>>,
    pub(crate) detached_faults: HashMap<u64, String>,
    pub(crate) registry: HashMap<u64, Arc<TaskShared>>,
    pub(crate) idle_count: usize,
    pub(crate) offload_done: HashMap<u64, Result<crate::PlatformResult, String>>,
    pub(crate) offload_pending: HashMap<u64, crate::PlatformContinuation>,
    pub(crate) offload_inflight: usize,
    pub(crate) mem: TaskStats,
}

/// What tasks and their regions used, for `--mem-stats`.
#[derive(Clone, Debug, Default)]
pub struct TaskStats {
    /// Tasks started, `main` included.
    pub spawned: u64,
    /// The most tasks alive at once.
    pub peak_live: usize,
    /// The most register slots any finished task held.
    pub peak_register_slots: usize,
    pub regions: crate::arena::RegionStats,
}

impl TaskStats {
    fn record(&mut self, register_slots: usize, regions: &crate::arena::RegionStats) {
        self.peak_register_slots = self.peak_register_slots.max(register_slots);
        self.regions.merge(regions);
        contracts::mem_trace!("task ended with {} register slots", register_slots);
    }
}

/// The condition variables idle threads wait on: one for the pool's workers, one for the home worker.
#[derive(Default)]
pub(crate) struct Wake {
    pool: Condvar,
    home: Condvar,
}

impl Wake {
    /// Wakes one thread that can run a task with this `pinned` flag; a pinned wake also reaches the single-thread runs, which wait on the pool's condvar.
    pub(crate) fn task(&self, pinned: bool) {
        if pinned {
            self.home.notify_one();
            wake_home_loop();
        }
        self.pool.notify_one();
    }

    pub(crate) fn all(&self) {
        self.home.notify_all();
        self.pool.notify_all();
        wake_home_loop();
    }

    /// Waits on the condvar `role` listens to.
    fn wait<'a>(&self, st: std::sync::MutexGuard<'a, SchedState>, role: Role) -> std::sync::MutexGuard<'a, SchedState> {
        let cv = if role == Role::Home { &self.home } else { &self.pool };
        cv.wait(st).unwrap()
    }
}

/// How often a busy home worker lets the window system run its events.
const HOME_POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(4);

/// Pulls the home worker out of the window system's event loop, if it is waiting there.
fn wake_home_loop() {
    if let Some(hook) = contracts::home_loop() {
        hook.wake();
    }
}

thread_local! {
    static HOME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether this thread is the one `run_main_parallel` was called on.
pub(crate) fn on_home_thread() -> bool {
    HOME.with(std::cell::Cell::get)
}

/// Marks the current thread as the home thread until dropped.
struct HomeThread;

impl HomeThread {
    fn enter() -> Self {
        HOME.with(|h| h.set(true));
        HomeThread
    }
}

impl Drop for HomeThread {
    fn drop(&mut self) {
        HOME.with(|h| h.set(false));
    }
}

/// Which tasks a worker thread may run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Unpinned tasks only.
    Pool,
    /// Pinned tasks only: the thread that called `run_main_parallel`.
    Home,
    /// Every task: the single-thread runs.
    Both,
}

impl Role {
    fn runs(self, pinned: bool) -> bool {
        match self {
            Role::Pool => !pinned,
            Role::Home => pinned,
            Role::Both => true,
        }
    }
}

/// A ready queue; every queue adds to one shared counter so `safepoint_poll` can ask "is anything waiting" without the `sched` lock.
#[derive(Default)]
pub struct ReadyQueue {
    queue: VecDeque<TaskContext>,
    len: Arc<AtomicUsize>,
}

impl ReadyQueue {
    fn sharing(len: Arc<AtomicUsize>) -> Self {
        ReadyQueue { queue: VecDeque::new(), len }
    }

    pub fn push_back(&mut self, task: TaskContext) {
        self.queue.push_back(task);
        self.len.fetch_add(1, Ordering::Relaxed);
    }

    pub fn pop_front(&mut self) -> Option<TaskContext> {
        let task = self.queue.pop_front();
        if task.is_some() {
            self.len.fetch_sub(1, Ordering::Relaxed);
        }
        task
    }

    pub fn clear(&mut self) {
        self.len.fetch_sub(self.queue.len(), Ordering::Relaxed);
        self.queue.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut TaskContext> {
        self.queue.iter_mut()
    }

    pub(crate) fn len_handle(&self) -> Arc<AtomicUsize> {
        self.len.clone()
    }
}

impl SchedState {
    /// Empty queues that share one ready counter.
    pub(crate) fn new() -> Self {
        let len = Arc::new(AtomicUsize::new(0));
        SchedState {
            run_queue: ReadyQueue::sharing(len.clone()),
            home_queue: ReadyQueue::sharing(len),
            ..SchedState::default()
        }
    }

    /// Puts a ready task on the queue its `pinned` flag names; the caller wakes a thread with [`Wake::task`].
    pub(crate) fn enqueue(&mut self, task: TaskContext) {
        if task.pinned {
            self.home_queue.push_back(task);
        } else {
            self.run_queue.push_back(task);
        }
    }

    fn pop_for(&mut self, role: Role) -> Option<TaskContext> {
        match role {
            Role::Pool => self.run_queue.pop_front(),
            Role::Home => self.home_queue.pop_front(),
            Role::Both => self.run_queue.pop_front().or_else(|| self.home_queue.pop_front()),
        }
    }

    fn has_work_for(&self, role: Role) -> bool {
        match role {
            Role::Pool => !self.run_queue.is_empty(),
            Role::Home => !self.home_queue.is_empty(),
            Role::Both => !self.run_queue.is_empty() || !self.home_queue.is_empty(),
        }
    }

    /// Drops every waiter entry of task `id`.
    pub(crate) fn forget_waiter(&mut self, id: u64) {
        for map in [&mut self.send_waiters, &mut self.recv_waiters, &mut self.handoff_waiters, &mut self.writer_waiters] {
            map.retain(|_, waiters| {
                waiters.retain(|&w| w != id);
                !waiters.is_empty()
            });
        }
        self.watchers.retain(|_, watchers| {
            watchers.retain(|&w| w != id);
            !watchers.is_empty()
        });
    }

    /// Moves a parked task to its ready queue and returns whether it was pinned, or notes an early wake.
    pub(crate) fn wake_or_note_early(&mut self, id: u64) -> Option<bool> {
        match self.blocked.remove(&id) {
            Some(t) => {
                let pinned = t.pinned;
                self.enqueue(t);
                Some(pinned)
            }
            None => {
                self.woken_early.insert(id);
                None
            }
        }
    }
}

// SAFETY: `NonNull<ObjectHeader>` (in `detached`, and reachable through a
// `TaskContext`'s own fields) is never dereferenced except by code that
// already holds the appropriate lock or exclusive ownership of the task it
// came from — see `Runtime`'s own `Send`/`Sync` safety comment in lib.rs,
// which this state lives behind.
unsafe impl Send for SchedState {}

fn self_scan_root_ptrs(task: &mut TaskContext, code_objects: &[crate::CodeObject]) -> Vec<NonNull<ObjectHeader>> {
    let mut out = Vec::new();
    let extent = task.live_register_extent(code_objects);
    for reg in &task.registers[..extent] {
        if let Some(ptr) = reg.as_object_ptr() {
            out.push(ptr);
        }
    }
    task.clear_dead_registers(code_objects);
    for frame in &task.call_stack {
        if let Some(ptr) = frame.closure {
            out.push(ptr);
        }
        if let Some(link) = frame.generator {
            out.push(link.object);
        }
    }
    if let Some(ptr) = task.handle {
        out.push(ptr);
    }
    out.extend_from_slice(&task.held_turns);
    out.extend_from_slice(&task.owned_senders);
    task.arenas.for_each_heap_pointer(|ptr| out.push(ptr));
    for (_, saved) in &task.arena_saves {
        saved.for_each_heap_pointer(|ptr| out.push(ptr));
    }
    out
}

struct CheckedOutGuard<'a> {
    rt: &'a Runtime,
}

impl CheckedOutGuard<'_> {
    fn finish_main(self, task: &mut TaskContext) {
        let rt = self.rt;
        std::mem::forget(self);
        rt.sched.lock().unwrap().mem.record(task.registers.len(), &task.arenas.take_stats());
        rt.safepoint.leave_reporting(&mut || self_scan_root_ptrs(task, &rt.code_objects));
    }
}

impl Drop for CheckedOutGuard<'_> {
    fn drop(&mut self) {
        self.rt.safepoint.leave();
    }
}

impl Runtime {
    /// Publishes a task: registers its [`TaskShared`], pushes it on the ready queue and wakes a sleeping worker.
    pub fn schedule(&self, task: TaskContext) {
        let mut st = self.sched.lock().unwrap();
        st.registry.insert(task.task_id, Arc::clone(&task.shared));
        st.mem.peak_live = st.mem.peak_live.max(st.registry.len());
        let pinned = task.pinned;
        st.enqueue(task);
        drop(st);
        self.wake.task(pinned);
    }

    /// Task and region measurements so far.
    pub fn task_stats(&self) -> TaskStats {
        let st = self.sched.lock().unwrap();
        TaskStats { spawned: st.next_task_id + 1, ..st.mem.clone() }
    }

    /// How many `TaskContext`s are held in the free pool.
    pub fn pooled_tasks(&self) -> usize {
        self.sched.lock().unwrap().free_pool.len()
    }

    /// A `TaskContext` for a task entering code object `code_idx`, reusing a pooled one when available.
    pub fn acquire_task(
        &self,
        code_idx: usize,
        reg_count: usize,
        parent: Option<u64>,
        scope_depth: usize,
    ) -> TaskContext {
        let regs = reg_count.max(16);
        let (task_id, mut t) = {
            let mut st = self.sched.lock().unwrap();
            st.next_task_id += 1;
            let task_id = st.next_task_id;
            (task_id, st.free_pool.pop().unwrap_or_default())
        };
        t.call_stack.clear();
        t.call_stack.push(entry_frame(code_idx));
        t.registers.clear();
        t.registers.resize(regs, Value::null());
        t.pc = 0;
        t.current_code = code_idx;
        t.arenas.clear();
        t.arena_saves.clear();
        t.saved_arena_bytes = 0;
        t.shared = Arc::new(TaskShared::new(parent, scope_depth));
        t.handle = None;
        t.handoff = None;
        t.pinned = false;
        t.scope_parked = false;
        t.task_id = task_id;
        t.budget = DEFAULT_REDUCTION_BUDGET;
        t.instrs_since_safepoint = 0;
        t
    }

    fn recycle(&self, mut task: TaskContext) {
        let register_slots = task.registers.len();
        let regions = task.arenas.take_stats();
        task.call_stack.clear();
        task.registers.clear();
        task.arenas.clear();
        task.arena_saves.clear();
        task.saved_arena_bytes = 0;
        task.handle = None;
        task.pinned = false;
        let task_id = task.task_id;
        let cancelled = task.shared.cancelled.load(Ordering::SeqCst);
        task.task_id = 0;
        task.pc = 0;
        task.current_code = 0;
        task.budget = DEFAULT_REDUCTION_BUDGET;
        task.instrs_since_safepoint = 0;
        let mut guard = self.sched.lock().unwrap();
        let st = &mut *guard;
        st.registry.remove(&task_id);
        st.mem.record(register_slots, &regions);
        if cancelled {
            st.forget_waiter(task_id);
        }
        st.woken_early.remove(&task_id);
        if st.free_pool.len() < crate::FREE_POOL_LIMIT {
            task.registers.shrink_to(crate::RECYCLED_REGISTER_SLOTS);
            task.call_stack.shrink_to(crate::RECYCLED_FRAMES);
            st.free_pool.push(task);
        }
    }

    pub(crate) fn task_shared(&self, id: u64) -> Option<Arc<TaskShared>> {
        self.sched.lock().unwrap().registry.get(&id).cloned()
    }

    pub(crate) fn unblock(&self, id: u64) {
        let mut st = self.sched.lock().unwrap();
        if let Some(t) = st.blocked.remove(&id) {
            let pinned = t.pinned;
            st.enqueue(t);
            drop(st);
            self.wake.task(pinned);
        }
    }

    /// Wakes a task parked at its `SCOPEEXIT`; one parked on anything else, or still running, is left alone.
    pub(crate) fn unblock_scope(&self, id: u64) {
        let mut st = self.sched.lock().unwrap();
        if st.blocked.get(&id).is_some_and(|t| t.scope_parked) {
            let t = st.blocked.remove(&id).expect("checked above");
            let pinned = t.pinned;
            st.enqueue(t);
            drop(st);
            self.wake.task(pinned);
        }
    }


    pub(crate) fn unblock_or_note_early(&self, id: u64) {
        let mut st = self.sched.lock().unwrap();
        if let Some(pinned) = st.wake_or_note_early(id) {
            drop(st);
            self.wake.task(pinned);
        }
    }

    /// Runs every task in the run queue, and anything they spawn, to completion; single worker only.
    pub fn run_scheduled(&mut self) -> Result<(), String> {
        let done = AtomicBool::new(false);
        let result_slot: Mutex<Option<Result<TaskContext, String>>> = Mutex::new(None);
        let _home = HomeThread::enter();
        drive_loop(self, Role::Both, None, 1, &done, &result_slot);
        Ok(())
    }

    /// Runs `main` and every task it spawns on this thread, and returns the finished main task, whose `registers[0]` is the program result.
    pub fn run_main(&mut self, main: TaskContext) -> Result<TaskContext, String> {
        let main_id = main.task_id;
        self.schedule(main);
        let done = AtomicBool::new(false);
        let result_slot: Mutex<Option<Result<TaskContext, String>>> = Mutex::new(None);
        let _home = HomeThread::enter();
        drive_loop(self, Role::Both, Some(main_id), 1, &done, &result_slot);
        if self.exit_requested() {
            return Ok(self.finish_exited(result_slot));
        }
        let finished = result_slot
            .into_inner()
            .unwrap()
            .unwrap_or_else(|| Err("scheduler drained without the main task completing".to_string()))?;
        self.shutdown_detached()?;
        Ok(finished)
    }

    /// Runs `main` on `num_workers` OS threads that pull tasks from one shared, locked run queue; the calling thread also runs the tasks that pinned themselves to it.
    pub fn run_main_parallel(&self, main: TaskContext, num_workers: usize) -> Result<TaskContext, String> {
        assert!(num_workers >= 1, "run_main_parallel: num_workers must be at least 1");
        let main_id = main.task_id;
        self.schedule(main);
        let done = AtomicBool::new(false);
        let result_slot: Mutex<Option<Result<TaskContext, String>>> = Mutex::new(None);
        let threads = num_workers + 1;
        std::thread::scope(|scope| {
            for _ in 0..num_workers {
                scope.spawn(|| drive_loop(self, Role::Pool, Some(main_id), threads, &done, &result_slot));
            }
            let _home = HomeThread::enter();
            drive_loop(self, Role::Home, Some(main_id), threads, &done, &result_slot);
        });
        if self.exit_requested() {
            return Ok(self.finish_exited(result_slot));
        }
        let finished = result_slot
            .into_inner()
            .unwrap()
            .unwrap_or_else(|| Err("scheduler drained without the main task completing".to_string()))?;
        self.shutdown_detached()?;
        Ok(finished)
    }

    fn finish_exited(&self, result_slot: Mutex<Option<Result<TaskContext, String>>>) -> TaskContext {
        let _ = self.shutdown_detached();
        result_slot.into_inner().unwrap().and_then(Result::ok).unwrap_or_default()
    }

    fn shutdown_detached(&self) -> Result<(), String> {
        use crate::handlers::sched_intrinsics::{TASK_SLOT_ID, TASK_SLOT_OBSERVED, TASK_SLOT_STATUS, STATUS_ERR};

        let mut st = self.sched.lock().unwrap();
        let detached = std::mem::take(&mut st.detached);
        let mut fault: Option<String> = None;
        for handle in &detached {
            // SAFETY: every handle in `detached` was allocated by
            // `sched_intrinsics::alloc_task_handle` and kept alive as a GC
            // root (`crate::roots::scan_roots`) for as long as it's in this
            // list.
            let (status, observed, task_id) = unsafe {
                let h = handle.as_ref();
                (
                    h.get_field(TASK_SLOT_STATUS).as_int().unwrap_or(STATUS_ERR),
                    h.get_field(TASK_SLOT_OBSERVED).is_truthy(),
                    h.get_field(TASK_SLOT_ID).as_uint().unwrap_or(0),
                )
            };
            if fault.is_none() && status == STATUS_ERR && !observed {
                fault = st.detached_faults.get(&task_id).cloned();
            }
        }

        st.run_queue.clear();
        st.home_queue.clear();
        st.blocked.clear();
        st.detached_faults.clear();
        st.registry.clear();

        match fault {
            Some(msg) => Err(msg),
            None => Ok(()),
        }
    }

    fn complete_child(&self, mut child: TaskContext, err: Option<String>) {
        crate::handlers::shared_cell::release_held_turns(self, &mut child);
        crate::handlers::sched_intrinsics::close_owned_senders(self, &mut child);
        if let Some(handle) = child.handle {
            let ok_value = child.registers.first().copied().unwrap_or_else(Value::null);
            crate::handlers::sched_intrinsics::complete_handle(self, handle, ok_value, err.as_deref());
        }

        if child.shared.parent.is_none() {
            let mut st = self.sched.lock().unwrap();
            match &err {
                Some(e) => {
                    st.detached_faults.insert(child.task_id, e.clone());
                }
                None => {
                    if let Some(handle) = child.handle {
                        st.detached.remove(&handle);
                    }
                }
            }
        }

        let faulted = err.is_some();
        if let Some(pid) = child.shared.parent {
            let depth = child.shared.scope_depth;
            let mut wake = false;
            if let Some(parent_shared) = self.task_shared(pid) {
                let mut scopes = parent_shared.scopes.lock().unwrap();
                if let Some(sf) = scopes.get_mut(depth) {
                    sf.pending = sf.pending.saturating_sub(1);
                    if let Some(e) = &err
                        && sf.first_error.is_none()
                    {
                        sf.first_error = Some(e.clone());
                        sf.first_error_handle = child.handle;
                    }
                    wake = sf.pending == 0;
                }
            }
            if faulted {
                self.cancel_children(pid, Some(depth));
            }
            if wake {
                // A plain unblock: a parent that has not parked yet re-checks `pending` as it parks, and one still running needs no wake.
                self.unblock_scope(pid);
            }
        }
        self.recycle(child);
    }

    pub(crate) fn cancel_children(&self, parent_id: u64, depth: Option<usize>) {
        let newly_cancelled: Vec<u64> = {
            let mut st = self.sched.lock().unwrap();
            let ids: Vec<u64> = st
                .registry
                .iter()
                .filter(|(_, shared)| {
                    shared.parent == Some(parent_id)
                        && depth.is_none_or(|d| shared.scope_depth == d)
                        && !shared.cancelled.load(Ordering::SeqCst)
                })
                .map(|(id, shared)| {
                    shared.cancelled.store(true, Ordering::SeqCst);
                    *id
                })
                .collect();
            for &id in &ids {
                st.forget_waiter(id);
            }
            ids
        };
        for id in newly_cancelled {
            self.unblock(id);
        }
    }

    /// Cancels another task: sets its flag and drops its waiter entries in one section, so a send or unlock cannot spend its wake on a task that no longer waits, then wakes it.
    pub(crate) fn cancel_task(&self, id: u64) {
        {
            let mut st = self.sched.lock().unwrap();
            if let Some(shared) = st.registry.get(&id) {
                shared.cancelled.store(true, Ordering::SeqCst);
            }
            st.forget_waiter(id);
        }
        self.unblock(id);
    }

    /// Count of cooperative task switches performed across every worker.
    pub fn context_switches(&self) -> u64 {
        self.sched.lock().unwrap().context_switches

    }
    pub(crate) fn gc_safepoint(&self, task: &mut TaskContext) -> Result<(), String> {
        let own_roots = |task: &mut TaskContext| self_scan_root_ptrs(task, &self.code_objects);
        if self.safepoint.pause_pending() {
            self.safepoint.participate(&mut || own_roots(task));
            return Ok(());
        }
        if !self.heap.should_collect() {
            return Ok(());
        }
        let started = self.safepoint.stop_the_world(&mut || self.heap.collect(&mut RuntimeRoots::new(self, task)));
        if !started {
            self.safepoint.participate(&mut || own_roots(task));
            return Ok(());
        }
        crate::arena::decay_segments();
        self.release_resources();
        self.heap.limit_exceeded().map_or(Ok(()), Err)
    }

    fn release_resources(&self) {
        use contracts::{PlatformRequest, RELEASE_FILE, RELEASE_GENERATOR, RELEASE_LIBRARY, RELEASE_SOCKET};
        for r in self.heap.take_released() {
            let request = match r.kind {
                RELEASE_GENERATOR => {
                    self.gen_regions.release(r.key);
                    continue;
                }
                RELEASE_FILE => PlatformRequest::FileClose { id: r.key },
                RELEASE_SOCKET => PlatformRequest::SocketClose { id: r.key },
                RELEASE_LIBRARY => PlatformRequest::LibClose { lib: r.key },
                _ => continue,
            };
            if let Some(platform) = &self.platform {
                let _ = platform.execute(request);
            }
        }
    }
}

fn drive_loop(
    rt: &Runtime,
    role: Role,
    main_id: Option<u64>,
    threads: usize,
    done: &AtomicBool,
    result_slot: &Mutex<Option<Result<TaskContext, String>>>,
) {
    let mut last_poll = std::time::Instant::now();
    loop {
        if done.load(Ordering::SeqCst) {
            return;
        }
        if role != Role::Pool && last_poll.elapsed() >= HOME_POLL_EVERY {
            if let Some(hook) = contracts::home_loop().filter(|h| h.is_open()) {
                hook.wait(Some(std::time::Duration::ZERO));
            }
            last_poll = std::time::Instant::now();
        }
        let mut task = loop {
            rt.safepoint.enter();
            let mut st = rt.sched.lock().unwrap();
            if let Some(t) = st.pop_for(role) {
                break t;
            }
            rt.safepoint.leave();
            if done.load(Ordering::SeqCst) {
                return;
            }
            if main_id.is_none() {
                return;
            }
            st.idle_count += 1;
            let nothing_ready = st.run_queue.is_empty() && st.home_queue.is_empty();
            let window_system = contracts::home_loop().filter(|h| h.is_open());
            if st.idle_count == threads && nothing_ready && st.offload_inflight == 0 && !rt.sources_can_wake(&st) && window_system.is_none() {
                let msg = if !st.blocked.is_empty() {
                    format!("deadlock: {} task(s) blocked, none runnable", st.blocked.len())
                } else {
                    "scheduler drained without the main task completing".to_string()
                };
                *result_slot.lock().unwrap() = Some(Err(msg));
                done.store(true, Ordering::SeqCst);
                st.idle_count -= 1;
                drop(st);
                rt.wake.all();
                return;
            }
            st = match window_system.filter(|_| role != Role::Pool) {
                Some(hook) => {
                    drop(st);
                    hook.wait(None);
                    rt.sched.lock().unwrap()
                }
                None => rt.wake.wait(st, role),
            };
            st.idle_count -= 1;
        };
        let checked_out_guard = CheckedOutGuard { rt };
        task.budget = rt.next_budget();
        let is_main = Some(task.task_id) == main_id;
        loop {
            let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.step(&mut task)))
                .unwrap_or_else(|payload| match payload.downcast_ref::<contracts::OutOfMemory>() {
                    Some(oom) => Err(oom.0.clone()),
                    None => Err(format!("internal panic: {}", panic_message(&*payload))),
                })
                .map_err(|e| rt.locate_fault(&task, e));
            match stepped {
                Ok(VmStatus::Running) => {}
                Ok(VmStatus::Yielded) => {
                    if !role.runs(task.pinned) {
                        rt.schedule(task);
                        break;
                    }
                    if !rt.sched.lock().unwrap().has_work_for(role) {
                        continue;
                    }
                    rt.sched.lock().unwrap().context_switches += 1;
                    rt.schedule(task);
                    break;
                }
                Ok(VmStatus::Parked) => {
                    let mut st = rt.sched.lock().unwrap();
                    let cancelled = !task.awaiting_platform && task.shared.cancelled.load(Ordering::SeqCst);
                    let scope_done = task.scope_parked && task.shared.scopes.lock().unwrap().last().is_some_and(|f| f.pending == 0);
                    if cancelled {
                        st.forget_waiter(task.task_id);
                    }
                    if st.woken_early.remove(&task.task_id) || cancelled || scope_done {
                        let pinned = task.pinned;
                        st.enqueue(task);
                        drop(st);
                        rt.wake.task(pinned);
                    } else {
                        st.blocked.insert(task.task_id, task);
                    }
                    break;
                }
                Ok(VmStatus::Halted) => {
                    if is_main {
                        checked_out_guard.finish_main(&mut task);
                        *result_slot.lock().unwrap() = Some(Ok(task));
                        signal_done(rt, done);
                        return;
                    }
                    rt.complete_child(task, None);
                    break;
                }
                Ok(VmStatus::Exited(_)) => {
                    signal_done(rt, done);
                    return;
                }
                Err(e) => {
                    if rt.exit_requested() {
                        signal_done(rt, done);
                        return;
                    }
                    if is_main {
                        checked_out_guard.finish_main(&mut task);
                        *result_slot.lock().unwrap() = Some(Err(e));
                        signal_done(rt, done);
                        return;
                    }
                    rt.complete_child(task, Some(e));
                    break;
                }
            }
        }
    }
}

fn signal_done(rt: &Runtime, done: &AtomicBool) {
    {
        let _st = rt.sched.lock().unwrap();
        done.store(true, Ordering::SeqCst);
    }
    rt.wake.all();
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn entry_frame(code_idx: usize) -> Frame {
    Frame {
        return_pc: 0,
        base: 0,
        caller_code: code_idx,
        dest_reg: 0,
        closure: None,
        generator: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::sched_intrinsics::alloc_task_handle;

    fn detached_child(rt: &Runtime) -> TaskContext {
        let mut child = rt.acquire_task(0, 8, None, 0);
        let handle = alloc_task_handle(rt, child.task_id);
        child.handle = Some(handle);
        rt.sched.lock().unwrap().detached.insert(handle);
        child
    }

    #[test]
    fn a_detached_task_that_finishes_cleanly_stops_being_a_root() {
        let rt = Runtime::new(vec![]);
        let child = detached_child(&rt);
        assert_eq!(rt.sched.lock().unwrap().detached.len(), 1);
        rt.complete_child(child, None);
        assert!(rt.sched.lock().unwrap().detached.is_empty());
    }

    #[test]
    fn a_detached_task_that_faulted_stays_until_the_program_ends() {
        let rt = Runtime::new(vec![]);
        let child = detached_child(&rt);
        rt.complete_child(child, Some("boom".to_string()));
        let st = rt.sched.lock().unwrap();
        assert_eq!(st.detached.len(), 1);
        assert_eq!(st.detached_faults.len(), 1);
    }

    fn parent_with_one_child(rt: &Runtime) -> (TaskContext, TaskContext) {
        let parent = rt.acquire_task(0, 8, None, 0);
        parent.shared.scopes.lock().unwrap().push(crate::ScopeFrame { pending: 1, ..crate::ScopeFrame::default() });
        rt.sched.lock().unwrap().registry.insert(parent.task_id, Arc::clone(&parent.shared));
        let child = rt.acquire_task(0, 8, Some(parent.task_id), 0);
        (parent, child)
    }

    #[test]
    fn a_child_finishing_under_a_running_parent_leaves_no_early_wake() {
        let rt = Runtime::new(vec![]);
        let (parent, child) = parent_with_one_child(&rt);
        rt.complete_child(child, None);
        assert!(rt.sched.lock().unwrap().woken_early.is_empty());
        assert_eq!(parent.shared.scopes.lock().unwrap()[0].pending, 0);
    }

    #[test]
    fn the_last_child_finishing_wakes_a_parent_parked_at_the_scope_exit() {
        let rt = Runtime::new(vec![]);
        let (mut parent, child) = parent_with_one_child(&rt);
        let id = parent.task_id;
        parent.scope_parked = true;
        rt.sched.lock().unwrap().blocked.insert(id, parent);
        rt.complete_child(child, None);
        let st = rt.sched.lock().unwrap();
        assert!(st.blocked.is_empty() && !st.run_queue.is_empty());
        assert!(st.woken_early.is_empty());
    }

    #[test]
    fn a_child_finishing_leaves_a_parent_parked_on_something_else_asleep() {
        let rt = Runtime::new(vec![]);
        let (parent, child) = parent_with_one_child(&rt);
        let id = parent.task_id;
        rt.sched.lock().unwrap().blocked.insert(id, parent);
        rt.complete_child(child, None);
        let st = rt.sched.lock().unwrap();
        assert!(st.blocked.contains_key(&id) && st.run_queue.is_empty());
    }

    #[test]
    fn cancelling_a_parked_task_drops_its_waiter_entries_but_not_another_tasks() {
        let rt = Runtime::new(vec![]);
        let task = rt.acquire_task(0, 8, None, 0);
        let id = task.task_id;
        let shared = Arc::clone(&task.shared);
        {
            let mut st = rt.sched.lock().unwrap();
            st.registry.insert(id, Arc::clone(&shared));
            st.recv_waiters.entry(8).or_default().push_back(id);
            st.recv_waiters.entry(8).or_default().push_back(id + 100);
            st.blocked.insert(id, task);
        }
        rt.cancel_task(id);
        let st = rt.sched.lock().unwrap();
        assert_eq!(st.recv_waiters[&8], VecDeque::from([id + 100]));
        assert!(st.blocked.is_empty() && !st.run_queue.is_empty());
        assert!(shared.cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn a_finished_task_leaves_no_empty_waiter_queues() {
        let rt = Runtime::new(vec![]);
        let task = rt.acquire_task(0, 8, None, 0);
        let id = task.task_id;
        task.shared.cancelled.store(true, Ordering::SeqCst);
        {
            let mut st = rt.sched.lock().unwrap();
            st.send_waiters.entry(7).or_default().push_back(id);
            st.recv_waiters.entry(8).or_default().push_back(id);
            st.recv_waiters.entry(8).or_default().push_back(id + 100);
            st.writer_waiters.entry(9).or_default().push_back(id);
            st.watchers.entry(10).or_default().push(id);
        }
        rt.recycle(task);
        let st = rt.sched.lock().unwrap();
        assert!(st.send_waiters.is_empty() && st.writer_waiters.is_empty() && st.watchers.is_empty());
        assert_eq!(st.recv_waiters.len(), 1, "another task's queue stays");
    }
}
