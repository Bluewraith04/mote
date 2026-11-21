//! Tasks on the scheduler: the `Task` handle objects, the waits on them, scopes, and what the collector needs from them.

use mote_abi::{TaskContext, TypeId, TASK_CANCELLED, TASK_CONVERT_OFFSET, TASK_DONE, TASK_FAULTED, TASK_NUMBER_OFFSET, TASK_PENDING};
use mote_heap::{Full, Object};
use mote_scheduler::{self as scheduler, Poll, TaskHandle, Wake};

use crate::channels::NANOS;
use crate::collect::Top;
use crate::report::FaultReport;
use crate::state::Runtime;

/// What the thunk of a task calls: the task's context, its closure and its handle, whose payload the thunk fills with the result.
pub type TaskBody = extern "C" fn(*mut TaskContext, usize, usize);

/// The offset of the word that holds the task's number in its handle.
const NUMBER: u32 = TASK_NUMBER_OFFSET;
/// The offset of the word that holds the task's state in its handle.
const STATE: u32 = 8;

#[derive(Default)]
struct TaskRec {
    state: u64,
    /// Counts the tasks this place has held, so a word that names a task by place and generation tells a task that was freed.
    generation: u32,
    /// The handle object and the closure, which the collector keeps alive until the task ends.
    handle: usize,
    closure: usize,
    task: Option<TaskHandle>,
    scope: Option<usize>,
    awaited: bool,
    /// The handle object died; the record goes when nothing else needs it.
    dead: bool,
    waiters: Vec<TaskHandle>,
    fault: Option<FaultReport>,
}

#[derive(Default)]
struct ScopeRec {
    /// False for a place that was freed.
    open: bool,
    tasks: Vec<usize>,
    cancelled: bool,
    /// The task that opened it ended without closing it; it goes when its last task ends.
    orphan: bool,
    waiters: Vec<TaskHandle>,
}

/// Every task the program started and every scope it opened, with the places of those that were freed.
#[derive(Default)]
pub(crate) struct Tasks {
    recs: Vec<TaskRec>,
    scopes: Vec<ScopeRec>,
    free_recs: Vec<usize>,
    free_scopes: Vec<usize>,
}

impl Tasks {
    /// The objects that tasks which have not ended hold.
    pub(crate) fn roots(&self) -> Vec<usize> {
        self.recs.iter().filter(|rec| rec.state == TASK_PENDING).flat_map(|rec| [rec.handle, rec.closure]).filter(|&address| address != 0).collect()
    }

    /// The place of a new pending task: a freed one when there is one.
    fn start(&mut self, handle: usize, closure: usize, scope: Option<usize>) -> usize {
        let fresh = TaskRec { state: TASK_PENDING, handle, closure, scope, ..TaskRec::default() };
        match self.free_recs.pop() {
            Some(id) => {
                self.recs[id] = TaskRec { generation: self.recs[id].generation, ..fresh };
                id
            }
            None => {
                self.recs.push(fresh);
                self.recs.len() - 1
            }
        }
    }

    /// Frees the record of a task whose handle died, once it has ended and no scope or report needs it.
    fn release(&mut self, id: usize) {
        let rec = &mut self.recs[id];
        let reported = rec.state == TASK_FAULTED && !rec.awaited && rec.scope.is_none();
        if !rec.dead || rec.state == TASK_PENDING || rec.scope.is_some() || reported {
            return;
        }
        *rec = TaskRec { state: TASK_DONE, generation: rec.generation.wrapping_add(1), ..TaskRec::default() };
        self.free_recs.push(id);
    }

    /// Opens a scope in a freed place when there is one.
    fn open_scope(&mut self) -> usize {
        let index = self.free_scopes.pop().unwrap_or_else(|| {
            self.scopes.push(ScopeRec::default());
            self.scopes.len() - 1
        });
        self.scopes[index].open = true;
        index
    }

    /// The scope of the index, when it is open.
    fn scope(&self, index: usize) -> Option<&ScopeRec> {
        self.scopes.get(index).filter(|scope| scope.open)
    }

    /// Frees an orphaned scope once none of its tasks is pending.
    fn close_orphan(&mut self, index: usize) {
        let Some(scope) = self.scope(index).filter(|scope| scope.orphan) else { return };
        if scope.tasks.iter().all(|&id| self.recs[id].state != TASK_PENDING) {
            self.close_scope(index);
        }
    }

    /// Frees a scope that has ended; its tasks stop belonging to it, and a fault of theirs it did not report is its own.
    fn close_scope(&mut self, index: usize) {
        let Some(scope) = self.scopes.get_mut(index).filter(|scope| scope.open) else { return };
        let ids = std::mem::take(&mut scope.tasks);
        *scope = ScopeRec::default();
        self.free_scopes.push(index);
        for id in ids {
            let rec = &mut self.recs[id];
            rec.scope = None;
            rec.awaited |= rec.state == TASK_FAULTED;
            self.release(id);
        }
    }
}

/// A block a task is inside that a cancellation undoes.
pub(crate) enum Cleanup {
    /// A `with`: the box that holds the resource, and the code that closes the resource from the box.
    Close { resource: usize, thunk: usize },
    /// A `scope`, by its token.
    Scope(u64),
}

impl Cleanup {
    /// The object the cleanup keeps alive.
    pub(crate) fn resource(&self) -> Option<usize> {
        match self {
            Cleanup::Close { resource, .. } => Some(*resource),
            Cleanup::Scope(_) => None,
        }
    }
}

/// How a `scope` ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeEnd {
    Clean,
    /// A task of the scope faulted and nobody awaited it.
    Faulted(FaultReport),
    /// The waiting task was cancelled, and so were the tasks of the scope.
    Cancelled,
}

impl Runtime {
    fn number(handle: Object) -> usize {
        handle.word(NUMBER) as usize
    }

    /// Starts a task that runs `body(context, closure, handle)` and returns its `Task` handle object of type `ty`.
    ///
    /// The calling code is a task of the scheduler. A task started in a scope that was cancelled ends cancelled without running.
    pub fn spawn(&self, context: &mut TaskContext, ty: TypeId, body: TaskBody, closure: usize, scope: Option<u64>) -> Result<Object, Full> {
        let handle = self.alloc(context, ty, 0)?;
        let (id, cancelled) = {
            let mut tasks = self.tasks.lock().expect("the tasks");
            let scope = scope.map(|token| token as usize).filter(|&token| tasks.scope(token).is_some());
            let id = tasks.start(handle.address(), closure, scope);
            handle.set_word(NUMBER, id as u64);
            if let Some(scope) = scope {
                tasks.scopes[scope].tasks.push(id);
            }
            (id, scope.is_some_and(|scope| tasks.scopes[scope].cancelled))
        };
        self.own_captured(id, closure);
        let runtime = self as *const Runtime as usize;
        let address = handle.address();
        let started = scheduler::spawn(move || run_task(runtime, id, body, closure, address));
        if cancelled {
            started.cancel();
        }
        self.tasks.lock().expect("the tasks").recs[id].task = Some(started);
        Ok(handle)
    }

    /// Frees the record of a task whose handle died; a view of a task shares the record and frees nothing.
    pub(crate) fn task_free(&self, handle: Object) {
        if handle.word(TASK_CONVERT_OFFSET) != 0 {
            return;
        }
        let mut tasks = self.tasks.lock().expect("the tasks");
        let id = Self::number(handle);
        if let Some(rec) = tasks.recs.get_mut(id) {
            rec.dead = true;
            tasks.release(id);
        }
    }

    /// Takes the calling task off the waiters of the task, which a wait that did not end with the task's end leaves behind.
    fn stop_waiting(&self, id: usize) {
        let me = scheduler::current().id();
        self.tasks.lock().expect("the tasks").recs[id].waiters.retain(|waiter| waiter.id() != me);
    }

    /// Waits until the task has ended and returns its state, which is `TASK_PENDING` only when the waiting task was cancelled first.
    pub fn await_task(&self, context: &mut TaskContext, top: Top, handle: Object) -> u64 {
        let id = Self::number(handle);
        loop {
            {
                let mut tasks = self.tasks.lock().expect("the tasks");
                let rec = &mut tasks.recs[id];
                rec.awaited = true;
                if rec.state != TASK_PENDING {
                    return rec.state;
                }
                rec.waiters.push(scheduler::current());
            }
            if self.blocked(context, top, || scheduler::park_on("a task")) == Wake::Cancelled {
                self.stop_waiting(id);
                return TASK_PENDING;
            }
        }
    }

    /// Like `await_task`, giving up with `TASK_PENDING` after `ms` milliseconds of the scheduler's clock; a wait that was cancelled ends the calling task.
    pub fn await_task_timeout(&self, context: &mut TaskContext, top: Top, handle: Object, ms: i64) -> u64 {
        let id = Self::number(handle);
        let deadline = scheduler::now().saturating_add((ms.max(0) as u64).saturating_mul(NANOS));
        loop {
            {
                let mut tasks = self.tasks.lock().expect("the tasks");
                let rec = &mut tasks.recs[id];
                rec.awaited = true;
                if rec.state != TASK_PENDING {
                    return rec.state;
                }
                if scheduler::now() >= deadline {
                    return TASK_PENDING;
                }
                rec.waiters.push(scheduler::current());
            }
            let park = || scheduler::park_timeout(deadline.saturating_sub(scheduler::now()), "a task");
            let woke = self.blocked(context, top, park);
            self.stop_waiting(id);
            if woke == Wake::Cancelled {
                self.end_task(context, TASK_CANCELLED, None);
            }
        }
    }

    /// Sleeps `ms` milliseconds of the scheduler's clock; a cancelled task ends.
    pub fn sleep_ms(&self, context: &mut TaskContext, top: Top, ms: i64) {
        if ms <= 0 {
            return;
        }
        let sleep = || scheduler::sleep((ms as u64).saturating_mul(NANOS));
        if self.blocked(context, top, sleep) == Wake::Cancelled {
            self.end_task(context, TASK_CANCELLED, None);
        }
    }

    /// What names the task of the number in a lock word: its place and generation.
    pub(crate) fn task_name(&self, number: usize) -> u64 {
        u64::from(self.tasks.lock().expect("the tasks").recs[number].generation) << 32 | number as u64
    }

    /// Whether the task a `task_name` named has ended, which a freed task has.
    pub(crate) fn task_ended(&self, name: u64) -> bool {
        let tasks = self.tasks.lock().expect("the tasks");
        tasks.recs.get((name & 0xffff_ffff) as usize).is_none_or(|rec| u64::from(rec.generation) != name >> 32 || rec.state != TASK_PENDING)
    }

    /// Whether the task has ended.
    pub fn task_is_ready(&self, handle: Object) -> bool {
        self.tasks.lock().expect("the tasks").recs[Self::number(handle)].state != TASK_PENDING
    }

    /// Asks the task to stop at its next poll.
    pub fn cancel_task(&self, handle: Object) {
        let task = {
            let tasks = self.tasks.lock().expect("the tasks");
            let rec = &tasks.recs[Self::number(handle)];
            rec.task.clone().filter(|_| rec.state == TASK_PENDING)
        };
        if let Some(task) = task {
            task.cancel();
        }
    }

    /// Opens a `scope` for the task and returns its token.
    pub fn scope_begin(&self, context: *const TaskContext) -> u64 {
        let token = {
            self.tasks.lock().expect("the tasks").open_scope() as u64
        };
        self.update(context, |joined| joined.cleanups.push(Cleanup::Scope(token)));
        token
    }

    /// Records a `with` the task is inside.
    pub fn defer(&self, context: *const TaskContext, resource: usize, thunk: usize) {
        self.update(context, |joined| joined.cleanups.push(Cleanup::Close { resource, thunk }));
    }

    /// Forgets the innermost `with` of the task.
    pub fn undefer(&self, context: *const TaskContext) {
        self.update(context, |joined| {
            if let Some(at) = joined.cleanups.iter().rposition(|cleanup| matches!(cleanup, Cleanup::Close { .. })) {
                joined.cleanups.remove(at);
            }
        });
    }

    /// Forgets the `scope` of the token, which has ended.
    fn forget_scope(&self, context: *const TaskContext, token: u64) {
        self.update(context, |joined| {
            if let Some(at) = joined.cleanups.iter().rposition(|cleanup| matches!(cleanup, Cleanup::Scope(open) if *open == token)) {
                joined.cleanups.remove(at);
            }
        });
    }

    /// Waits for every task of the scope; a task that faulted and nobody awaited is reported once.
    pub fn scope_end(&self, context: &mut TaskContext, top: Top, token: u64) -> ScopeEnd {
        let index = token as usize;
        let ended = loop {
            {
                let mut tasks = self.tasks.lock().expect("the tasks");
                let Some(scope) = tasks.scope(index) else { break ScopeEnd::Clean };
                let ids = scope.tasks.clone();
                if ids.iter().all(|&id| tasks.recs[id].state != TASK_PENDING) {
                    let mut ended = ScopeEnd::Clean;
                    for id in ids {
                        let rec = &mut tasks.recs[id];
                        if rec.state == TASK_FAULTED && !rec.awaited {
                            rec.awaited = true;
                            if let Some(fault) = rec.fault.clone() {
                                ended = ScopeEnd::Faulted(fault);
                                break;
                            }
                        }
                    }
                    break ended;
                }
                tasks.scopes[index].waiters.push(scheduler::current());
            }
            if self.blocked(context, top, || scheduler::park_on("a scope")) == Wake::Cancelled {
                return ScopeEnd::Cancelled;
            }
        };
        self.forget_scope(context, token);
        self.tasks.lock().expect("the tasks").close_scope(index);
        ended
    }

    /// Cancels the tasks of the scope that have not ended and waits for them.
    fn cancel_scope(&self, context: *mut TaskContext, index: usize) {
        loop {
            let pending: Vec<TaskHandle> = {
                let mut tasks = self.tasks.lock().expect("the tasks");
                let Some(scope) = tasks.scope(index) else { return };
                let pending: Vec<TaskHandle> = scope.tasks.iter().filter(|&&id| tasks.recs[id].state == TASK_PENDING).filter_map(|&id| tasks.recs[id].task.clone()).collect();
                if pending.is_empty() {
                    tasks.close_scope(index);
                    return;
                }
                tasks.scopes[index].waiters.push(scheduler::current());
                pending
            };
            pending.iter().for_each(TaskHandle::cancel);
            self.blocked(context, Top { fp: 0, pc: 0 }, || scheduler::park_on("a scope"));
        }
    }

    /// Undoes the `with` and `scope` blocks the cancelled calling task is inside, innermost first.
    fn unwind(&self, context: *mut TaskContext) {
        scheduler::cleanup_begin();
        loop {
            let mut next = None;
            self.update(context, |joined| next = joined.cleanups.pop());
            match next {
                None => return,
                Some(Cleanup::Close { resource, thunk }) => {
                    // SAFETY: the thunk is the address of the function the code generator made for the `with`, which takes the context and the box.
                    let close: extern "C" fn(*mut TaskContext, usize) = unsafe { std::mem::transmute::<usize, extern "C" fn(*mut TaskContext, usize)>(thunk) };
                    close(context, resource);
                }
                Some(Cleanup::Scope(token)) => self.cancel_scope(context, token as usize),
            }
        }
    }

    /// The text `await` gives in its `Err` for a task that did not return.
    pub fn task_message(&self, handle: Object) -> String {
        let tasks = self.tasks.lock().expect("the tasks");
        let rec = &tasks.recs[Self::number(handle)];
        match (&rec.fault, rec.state) {
            (Some(fault), TASK_FAULTED) => fault.message.clone(),
            _ => "the task was cancelled".to_string(),
        }
    }

    /// Whether the context belongs to a task of the program that has been asked to stop.
    pub(crate) fn is_cancelled_task(&self, context: *mut TaskContext) -> bool {
        let mutators = self.mutators.lock().expect("the mutators");
        mutators.iter().any(|joined| joined.context == context as usize && joined.number.is_some() && joined.task.as_ref().is_some_and(TaskHandle::is_cancelled))
    }

    /// A fault of a task nobody awaited and that no scope holds, which the program's end reports.
    pub fn unawaited_fault(&self) -> Option<FaultReport> {
        let tasks = self.tasks.lock().expect("the tasks");
        tasks.recs.iter().find(|rec| rec.state == TASK_FAULTED && rec.scope.is_none() && !rec.awaited).and_then(|rec| rec.fault.clone())
    }

    /// Ends the calling task here with the state, recording the fault when there is one; its frames are abandoned.
    ///
    /// A cancelled task first undoes the `with` and `scope` blocks it is inside.
    pub fn end_task(&self, context: *mut TaskContext, state: u64, fault: Option<FaultReport>) -> ! {
        let id = self.running_task(context);
        if state == TASK_CANCELLED {
            self.unwind(context);
        }
        self.finish(context, id, state, fault);
        scheduler::exit()
    }

    /// The number of the task that runs on the context.
    fn running_task(&self, context: *mut TaskContext) -> usize {
        self.spawned(context).expect("a task of the program")
    }

    /// The number of the task of the program that runs on the context; none for the program's first task.
    pub(crate) fn spawned(&self, context: *mut TaskContext) -> Option<usize> {
        let mutators = self.mutators.lock().expect("the mutators");
        mutators.iter().find(|joined| joined.context == context as usize).and_then(|joined| joined.number)
    }

    /// Records the end of a task, wakes whoever waits for it, and cancels the rest of its scope when it faulted.
    fn finish(&self, context: *mut TaskContext, id: usize, state: u64, fault: Option<FaultReport>) {
        self.close_ends(id);
        let mut left_open = Vec::new();
        self.update(context, |joined| {
            left_open = joined.cleanups.drain(..).filter_map(|cleanup| if let Cleanup::Scope(token) = cleanup { Some(token as usize) } else { None }).collect();
        });
        let (woken, siblings) = {
            let mut tasks = self.tasks.lock().expect("the tasks");
            let rec = &mut tasks.recs[id];
            rec.state = state;
            rec.fault = fault;
            Object::from_address(rec.handle).expect("a handle").set_word(STATE, state);
            rec.handle = 0;
            rec.closure = 0;
            let mut woken = std::mem::take(&mut rec.waiters);
            let scope = rec.scope;
            let mut siblings = Vec::new();
            if let Some(scope) = scope {
                woken.append(&mut tasks.scopes[scope].waiters);
                if state == TASK_FAULTED {
                    tasks.scopes[scope].cancelled = true;
                    let ids = tasks.scopes[scope].tasks.clone();
                    siblings = ids.into_iter().filter(|&other| other != id).filter_map(|other| tasks.recs[other].task.clone().filter(|_| tasks.recs[other].state == TASK_PENDING)).collect();
                }
            }
            for index in left_open {
                if let Some(scope) = tasks.scopes.get_mut(index).filter(|scope| scope.open) {
                    scope.orphan = true;
                }
                tasks.close_orphan(index);
            }
            if let Some(scope) = scope {
                tasks.close_orphan(scope);
            }
            (woken, siblings)
        };
        for task in siblings {
            task.cancel();
        }
        for task in woken {
            task.unpark();
        }
        self.wake_locks();
        self.task_exit(context);
    }

    /// Answers a poll that asked for a preemption: the task yields, or ends cancelled.
    pub(crate) fn preempt(&self, context: &mut TaskContext, top: Top) {
        // SAFETY: the context is borrowed, so it is alive.
        unsafe { TaskContext::request_poll(context) };
        if self.blocked(context, top, scheduler::poll_point) == Poll::Cancelled {
            self.end_task(context, TASK_CANCELLED, None);
        }
    }
}

/// The scheduler task of a task of the program.
fn run_task(runtime: usize, id: usize, body: TaskBody, closure: usize, handle: usize) {
    // SAFETY: the runtime outlives the scheduler run that holds its tasks.
    let runtime = unsafe { &*(runtime as *const Runtime) };
    let me = scheduler::current();
    let context = me.context();
    // SAFETY: the context is this task's own block, which nothing else writes yet.
    unsafe { (*context).runtime = runtime as *const Runtime as u64 };
    runtime.task_enter(context, me.clone(), Some(id));
    if me.is_cancelled() {
        runtime.finish(context, id, TASK_CANCELLED, None);
        return;
    }
    body(context, closure, handle);
    runtime.finish(context, id, TASK_DONE, None);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mote_abi::{FaultKind, FaultTable, FrameDescriptor, HandleKind, Kind, Scalar, TypeTable};
    use mote_heap::Config as HeapConfig;

    use super::*;
    use crate::collect::CodeMap;

    const TOP: Top = Top { fp: 0, pc: 0 };
    const RESULT: u32 = 32;

    struct Types {
        table: Arc<TypeTable>,
        task: TypeId,
        node: TypeId,
    }

    fn types() -> Types {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let node = table.add("Node", Kind::Class(vec![int, int]));
        let task = table.add("Task<Int>", Kind::Handle(HandleKind::Task, vec![int]));
        table.add("String", Kind::String);
        Types { table: Arc::new(table), task, node }
    }

    /// Site 0 is a division by zero and site 1 a use of the whole stack.
    fn faults() -> FaultTable {
        let mut faults = FaultTable::new();
        faults.add(FaultKind::DivideByZero, mote_abi::Span::new(1, 2));
        faults.add(FaultKind::StackDepth, mote_abi::Span::new(3, 4));
        faults
    }

    fn runtime(ty: &Types, heap: HeapConfig) -> Arc<Runtime> {
        Runtime::new(ty.table.clone(), faults(), heap)
    }

    /// Runs `f` as the main task of a scheduler, as a mutator of the runtime.
    fn run<R: Send + 'static>(config: scheduler::Config, runtime: &Arc<Runtime>, f: impl FnOnce(&Runtime, &mut TaskContext) -> R + Send + 'static) -> R {
        let runtime = runtime.clone();
        scheduler::Scheduler::new(config).expect("a scheduler").run(move || {
            let me = scheduler::current();
            let context = me.context();
            // SAFETY: the context is this task's own block.
            let context = unsafe { &mut *context };
            runtime.attach(context);
            runtime.task_enter(context, me, None);
            let result = f(&runtime, context);
            runtime.task_exit(context);
            result
        }).expect("no deadlock")
    }

    fn fault() -> FaultReport {
        FaultReport { kind: FaultKind::DivideByZero, message: "division by zero".into(), site: None, span: None, backtrace: Vec::new() }
    }

    /// Code in a task: a poll that yields at once, as compiled code does when a slice ends.
    fn yield_poll(context: *mut TaskContext) {
        // SAFETY: the context is the calling task's own.
        unsafe {
            TaskContext::request_poll(context);
            Runtime::of(context).poll(&mut *context, TOP);
        }
    }

    #[test]
    fn tasks_a_spawned_task_runs_its_body_and_await_returns_done_with_the_result_in_the_handle() {
        extern "C" fn body(_: *mut TaskContext, _: usize, handle: usize) {
            Object::from_address(handle).expect("a handle").set_word(RESULT, 7);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let (state, result, word) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let handle = rt.spawn(context, task, body, 0, None).expect("room");
            let state = rt.await_task(context, TOP, handle);
            (state, handle.word(RESULT), handle.word(STATE))
        });
        assert_eq!((state, result, word), (TASK_DONE, 7, TASK_DONE));
    }

    #[test]
    fn tasks_a_handle_is_pending_and_numbered_until_its_task_runs() {
        extern "C" fn body(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let seen = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let (first, second) = (rt.spawn(context, task, body, 0, None).expect("room"), rt.spawn(context, task, body, 0, None).expect("room"));
            let seen = (first.word(STATE), first.word(NUMBER), second.word(NUMBER), rt.task_is_ready(first));
            rt.await_task(context, TOP, first);
            rt.await_task(context, TOP, second);
            (seen, rt.task_is_ready(first), rt.task_is_ready(second))
        });
        assert_eq!(seen, ((TASK_PENDING, 0, 1, false), true, true));
    }

    #[test]
    fn tasks_awaiting_a_task_that_has_ended_returns_its_state_at_once() {
        extern "C" fn body(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let states = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let handle = rt.spawn(context, task, body, 0, None).expect("room");
            (rt.await_task(context, TOP, handle), rt.await_task(context, TOP, handle))
        });
        assert_eq!(states, (TASK_DONE, TASK_DONE));
    }

    #[test]
    fn tasks_cancel_ends_a_task_that_polls_as_cancelled() {
        static STARTED: AtomicBool = AtomicBool::new(false);
        static CANCELLED: AtomicBool = AtomicBool::new(false);
        static AFTER: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn spin(context: *mut TaskContext, _: usize, _: usize) {
            STARTED.store(true, Ordering::SeqCst);
            loop {
                yield_poll(context);
                if CANCELLED.load(Ordering::SeqCst) {
                    AFTER.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let (state, handle_state) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let handle = rt.spawn(context, task, spin, 0, None).expect("room");
            while !STARTED.load(Ordering::SeqCst) {
                scheduler::yield_now();
            }
            CANCELLED.store(true, Ordering::SeqCst);
            rt.cancel_task(handle);
            (rt.await_task(context, TOP, handle), handle.word(STATE))
        });
        assert_eq!((state, handle_state), (TASK_CANCELLED, TASK_CANCELLED));
        assert!(AFTER.load(Ordering::SeqCst) <= 1);
    }

    #[test]
    fn tasks_a_task_cancelled_before_it_starts_does_not_run_its_body() {
        static RAN: AtomicBool = AtomicBool::new(false);
        extern "C" fn body(_: *mut TaskContext, _: usize, _: usize) {
            RAN.store(true, Ordering::SeqCst);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let state = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let handle = rt.spawn(context, task, body, 0, None).expect("room");
            rt.cancel_task(handle);
            rt.await_task(context, TOP, handle)
        });
        assert_eq!(state, TASK_CANCELLED);
        assert!(!RAN.load(Ordering::SeqCst));
    }

    #[test]
    fn tasks_a_scope_waits_for_every_task_started_in_it() {
        static DONE: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn body(context: *mut TaskContext, _: usize, _: usize) {
            yield_poll(context);
            yield_poll(context);
            DONE.fetch_add(1, Ordering::SeqCst);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let (end, done) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let scope = rt.scope_begin(context);
            for _ in 0..3 {
                rt.spawn(context, task, body, 0, Some(scope)).expect("room");
            }
            (rt.scope_end(context, TOP, scope), DONE.load(Ordering::SeqCst))
        });
        assert_eq!((end, done), (ScopeEnd::Clean, 3));
    }

    #[test]
    fn tasks_a_fault_ends_the_task_faulted_cancels_its_scope_and_the_scope_reports_it_once() {
        static STARTED: AtomicBool = AtomicBool::new(false);
        extern "C" fn spin(context: *mut TaskContext, _: usize, _: usize) {
            STARTED.store(true, Ordering::SeqCst);
            loop {
                yield_poll(context);
            }
        }
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let (first, second, spinner, culprit) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let scope = rt.scope_begin(context);
            let spinner = rt.spawn(context, task, spin, 0, Some(scope)).expect("room");
            while !STARTED.load(Ordering::SeqCst) {
                scheduler::yield_now();
            }
            let culprit = rt.spawn(context, task, faulting, 0, Some(scope)).expect("room");
            let first = rt.scope_end(context, TOP, scope);
            (first, rt.scope_end(context, TOP, scope), spinner.word(STATE), culprit.word(STATE))
        });
        assert_eq!((first, second), (ScopeEnd::Faulted(fault()), ScopeEnd::Clean));
        assert_eq!((spinner, culprit), (TASK_CANCELLED, TASK_FAULTED));
    }

    #[test]
    fn tasks_a_fault_that_was_awaited_is_not_reported_by_its_scope() {
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let (state, end) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let scope = rt.scope_begin(context);
            let handle = rt.spawn(context, task, faulting, 0, Some(scope)).expect("room");
            (rt.await_task(context, TOP, handle), rt.scope_end(context, TOP, scope))
        });
        assert_eq!((state, end), (TASK_FAULTED, ScopeEnd::Clean));
    }

    #[test]
    fn tasks_a_task_started_in_a_cancelled_scope_ends_cancelled_without_running() {
        static RAN: AtomicBool = AtomicBool::new(false);
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        extern "C" fn late(_: *mut TaskContext, _: usize, _: usize) {
            RAN.store(true, Ordering::SeqCst);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let state = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let scope = rt.scope_begin(context);
            rt.spawn(context, task, faulting, 0, Some(scope)).expect("room");
            while !rt.tasks.lock().expect("the tasks").scopes[scope as usize].cancelled {
                scheduler::yield_now();
            }
            let handle = rt.spawn(context, task, late, 0, Some(scope)).expect("room");
            let state = rt.await_task(context, TOP, handle);
            rt.scope_end(context, TOP, scope);
            state
        });
        assert_eq!(state, TASK_CANCELLED);
        assert!(!RAN.load(Ordering::SeqCst));
    }

    #[test]
    fn tasks_a_fault_nobody_awaited_and_no_scope_holds_is_reported_until_it_is_awaited() {
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let (before, after) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let handle = rt.spawn(context, task, faulting, 0, None).expect("room");
            while !rt.task_is_ready(handle) {
                scheduler::yield_now();
            }
            let before = rt.unawaited_fault();
            rt.await_task(context, TOP, handle);
            (before, rt.unawaited_fault())
        });
        assert_eq!((before, after), (Some(fault()), None));
    }

    #[test]
    fn tasks_a_pending_task_keeps_its_handle_and_closure_alive_and_an_ended_one_does_not() {
        static GO: AtomicBool = AtomicBool::new(false);
        extern "C" fn waiting(context: *mut TaskContext, _: usize, _: usize) {
            while !GO.load(Ordering::SeqCst) {
                yield_poll(context);
            }
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let (task, node) = (ty.task, ty.node);
        let (while_pending, after_end) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let dead = rt.alloc(context, node, 0).expect("room");
            let closure = rt.alloc(context, node, 0).expect("room");
            let handle = rt.spawn(context, task, waiting, closure.address(), None).expect("room");
            rt.poll(context, TOP);
            let while_pending = (rt.heap().is_marked(handle), rt.heap().is_marked(closure), rt.heap().is_marked(dead));
            GO.store(true, Ordering::SeqCst);
            rt.await_task(context, TOP, handle);
            rt.poll(context, TOP);
            (while_pending, (rt.heap().is_marked(handle), rt.heap().is_marked(closure)))
        });
        assert_eq!(while_pending, (true, true, false));
        assert_eq!(after_end, (false, false));
        assert!(rt.take_problems().is_empty());
    }

    static PARKED: Mutex<Option<TaskHandle>> = Mutex::new(None);
    static HELD: AtomicUsize = AtomicUsize::new(0);
    static INTACT: AtomicBool = AtomicBool::new(false);
    const FRAME_PC: usize = 0x4000;

    /// A task that keeps an object only in a frame of its own and parks in the runtime.
    extern "C" fn holder(context: *mut TaskContext, _: usize, _: usize) {
        // SAFETY: the context is the calling task's own and attached.
        let rt = unsafe { Runtime::of(context) };
        let node = rt.alloc(unsafe { &mut *context }, TypeId::from_raw(NODE.load(Ordering::SeqCst) as u32), 0).expect("room");
        let mut frame = [node.address(), 0usize, 0, 0];
        HELD.store(node.address(), Ordering::SeqCst);
        *PARKED.lock().expect("parked") = Some(scheduler::current());
        let top = Top { fp: std::hint::black_box(&mut frame[2]) as *mut usize as usize, pc: FRAME_PC };
        rt.blocked(context, top, || scheduler::park_on("a test"));
        INTACT.store(std::hint::black_box(&frame)[0] == node.address(), Ordering::SeqCst);
    }

    static NODE: AtomicUsize = AtomicUsize::new(0);
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Whether the object a parked task holds only in its frame survives a collection while the task is parked.
    fn parked_frame_survives(compact: bool) -> (bool, bool) {
        let ty = types();
        let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        NODE.store(ty.node.raw() as usize, Ordering::SeqCst);
        INTACT.store(false, Ordering::SeqCst);
        *PARKED.lock().expect("parked") = None;
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let mut code = CodeMap::new();
        code.add_frame(FRAME_PC, FrameDescriptor::new(16, vec![0], vec![]).expect("a frame"));
        rt.set_code_map(code);
        let task = ty.task;
        let config = scheduler::Config { cold_after: compact.then_some(Duration::ZERO), ..scheduler::Config::seeded(3) };
        let (survived, compacted) = run(config, &rt, move |rt, context| {
            let handle = rt.spawn(context, task, holder, 0, None).expect("room");
            let parked = loop {
                if let Some(parked) = PARKED.lock().expect("parked").clone() {
                    break parked;
                }
                scheduler::yield_now();
            };
            // The task has stored its handle and goes on to park before main runs again.
            let compacted = if compact { scheduler::scavenge() > 0 } else { false };
            rt.poll(context, TOP);
            let held = Object::from_address(HELD.load(Ordering::SeqCst)).expect("an object");
            let survived = rt.heap().is_marked(held);
            parked.unpark();
            rt.await_task(context, TOP, handle);
            (survived, compacted)
        });
        assert!(INTACT.load(Ordering::SeqCst));
        assert!(rt.take_problems().is_empty());
        (survived, compacted)
    }

    #[test]
    fn tasks_the_frames_of_a_task_parked_in_the_runtime_are_roots() {
        assert_eq!(parked_frame_survives(false), (true, false));
    }

    #[test]
    fn tasks_the_frames_of_a_parked_task_are_roots_when_a_compactor_holds_its_stack() {
        assert_eq!(parked_frame_survives(true), (true, true));
    }

    /// The bytes of a string object.
    fn text(string: usize) -> String {
        let object = Object::from_address(string).expect("a string");
        // SAFETY: a string object holds `length` bytes from `ELEMENTS_OFFSET`.
        let bytes = unsafe { std::slice::from_raw_parts((string + mote_abi::ELEMENTS_OFFSET as usize) as *const u8, object.length() as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    }

    #[test]
    fn tasks_the_entries_start_wait_for_and_ask_a_task_through_their_symbols() {
        use crate::entries::symbols::*;
        extern "C" fn body(_: *mut TaskContext, _: usize, handle: usize) {
            Object::from_address(handle).expect("a handle").set_word(RESULT, 11);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task.raw();
        let seen = run(scheduler::Config::seeded(1), &rt, move |_, context| {
            let context = context as *mut TaskContext;
            // SAFETY: the context is this task's own and attached to a live runtime; the handles come from the entries.
            unsafe {
                let token = mote_scope_begin(context);
                let first = mote_task_spawn(context, task, body as *const () as usize, 0, token as i64);
                let second = mote_task_spawn(context, task, body as *const () as usize, 0, -1);
                let before = mote_task_ready(context, first);
                mote_scope_end(context, token);
                let (after, state) = (mote_task_ready(context, first), mote_task_await(context, second));
                mote_task_cancel(context, second);
                (before, after, state, Object::from_address(first).expect("a handle").word(RESULT), Object::from_address(second).expect("a handle").word(RESULT))
            }
        });
        assert_eq!(seen, (0, 1, TASK_DONE, 11, 11));
    }

    #[test]
    fn tasks_a_fault_in_a_task_ends_the_task_and_not_the_program_and_await_reads_its_message() {
        use crate::entries::symbols::*;
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            unsafe { mote_fault(context, 0) }
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        rt.set_fault_hook(Arc::new(|_| panic!("the program was stopped")));
        let task = ty.task.raw();
        let (state, message) = run(scheduler::Config::seeded(1), &rt, move |_, context| {
            let context = context as *mut TaskContext;
            // SAFETY: as above.
            unsafe {
                let handle = mote_task_spawn(context, task, faulting as *const () as usize, 0, -1);
                let state = mote_task_await(context, handle);
                (state, text(mote_task_message(context, handle)))
            }
        });
        assert_eq!((state, message.as_str()), (TASK_FAULTED, "division by zero"));
    }

    #[test]
    fn tasks_the_message_of_a_task_that_was_cancelled_says_so() {
        use crate::entries::symbols::*;
        extern "C" fn waiting(context: *mut TaskContext, _: usize, _: usize) {
            loop {
                yield_poll(context);
            }
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task.raw();
        let (state, message) = run(scheduler::Config::seeded(1), &rt, move |_, context| {
            let context = context as *mut TaskContext;
            // SAFETY: as above.
            unsafe {
                let handle = mote_task_spawn(context, task, waiting as *const () as usize, 0, -1);
                mote_task_cancel(context, handle);
                let state = mote_task_await(context, handle);
                (state, text(mote_task_message(context, handle)))
            }
        });
        assert_eq!((state, message.as_str()), (TASK_CANCELLED, "the task was cancelled"));
    }

    #[test]
    fn tasks_a_cancelled_task_that_meets_the_stack_limit_check_ends_cancelled_and_another_ends_faulted() {
        use crate::entries::symbols::*;
        static CANCELLED: AtomicBool = AtomicBool::new(false);
        extern "C" fn overflowing(context: *mut TaskContext, _: usize, _: usize) {
            while !CANCELLED.load(Ordering::SeqCst) {
                scheduler::yield_now();
            }
            // SAFETY: the context is the calling task's own and attached.
            unsafe { mote_fault(context, 1) }
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        rt.set_fault_hook(Arc::new(|_| panic!("the program was stopped")));
        let task = ty.task.raw();
        let states = run(scheduler::Config::seeded(1), &rt, move |_, context| {
            let context = context as *mut TaskContext;
            // SAFETY: as above.
            unsafe {
                let uncancelled = mote_task_spawn(context, task, overflowing as *const () as usize, 0, -1);
                let cancelled = mote_task_spawn(context, task, overflowing as *const () as usize, 0, -1);
                mote_task_cancel(context, cancelled);
                CANCELLED.store(true, Ordering::SeqCst);
                (mote_task_await(context, cancelled), mote_task_await(context, uncancelled))
            }
        });
        assert_eq!(states, (TASK_CANCELLED, TASK_FAULTED));
    }

    #[test]
    fn tasks_a_task_cancelled_while_it_waits_for_its_scope_ends_cancelled_and_cancels_the_scope() {
        use crate::entries::symbols::*;
        static TASK: AtomicUsize = AtomicUsize::new(0);
        static CHILD: AtomicUsize = AtomicUsize::new(0);
        static AFTER: AtomicBool = AtomicBool::new(false);
        extern "C" fn spin(context: *mut TaskContext, _: usize, _: usize) {
            loop {
                yield_poll(context);
            }
        }
        extern "C" fn waiter(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            unsafe {
                let token = mote_scope_begin(context);
                CHILD.store(mote_task_spawn(context, TASK.load(Ordering::SeqCst) as u32, spin as *const () as usize, 0, token as i64), Ordering::SeqCst);
                mote_scope_end(context, token);
            }
            AFTER.store(true, Ordering::SeqCst);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        TASK.store(ty.task.raw() as usize, Ordering::SeqCst);
        let states = run(scheduler::Config::seeded(1), &rt, move |_, context| {
            let context = context as *mut TaskContext;
            // SAFETY: as above.
            unsafe {
                let handle = mote_task_spawn(context, ty.task.raw(), waiter as *const () as usize, 0, -1);
                while CHILD.load(Ordering::SeqCst) == 0 {
                    scheduler::yield_now();
                }
                mote_task_cancel(context, handle);
                (mote_task_await(context, handle), mote_task_await(context, CHILD.load(Ordering::SeqCst)))
            }
        });
        assert_eq!(states, (TASK_CANCELLED, TASK_CANCELLED));
        assert!(!AFTER.load(Ordering::SeqCst));
    }

    #[test]
    fn tasks_the_return_address_reaches_the_spawn_entry_on_the_stack() {
        use crate::entries::symbols::*;
        #[inline(never)]
        fn spawn_a_bad_type(context: *mut TaskContext) -> usize {
            // SAFETY: the context is attached; type 0 is not a task type, which stops the program.
            std::hint::black_box(unsafe { mote_task_spawn(context, 0, 0, 0, -1) })
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let reports = Arc::new(Mutex::new(Vec::new()));
        let kept = reports.clone();
        rt.set_fault_hook(Arc::new(move |report| {
            kept.lock().expect("the reports").push(report.clone());
            scheduler::exit()
        }));
        let shared = rt.clone();
        let outcome = scheduler::Scheduler::new(scheduler::Config::seeded(1)).expect("a scheduler").run_or_exit(move || {
            let me = scheduler::current();
            let context = me.context();
            // SAFETY: the context is this task's own block.
            shared.attach(unsafe { &mut *context });
            spawn_a_bad_type(context);
        });
        assert_eq!(outcome, Ok(None));
        let reports = reports.lock().expect("the reports");
        let start = spawn_a_bad_type as *const () as usize;
        assert!(reports[0].message.starts_with("a task of type 0"), "{}", reports[0].message);
        assert!((1..512).contains(&reports[0].backtrace[0].wrapping_sub(start)), "{:#x} against {start:#x}", reports[0].backtrace[0]);
    }

    #[test]
    fn tasks_sleep_moves_the_scheduler_clock_by_the_milliseconds_and_a_time_of_zero_does_not() {
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let elapsed = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let start = scheduler::now();
            rt.sleep_ms(context, TOP, 0);
            rt.sleep_ms(context, TOP, -5);
            let unmoved = scheduler::now() - start;
            rt.sleep_ms(context, TOP, 100);
            (unmoved, (scheduler::now() - start) / NANOS)
        });
        assert_eq!(elapsed, (0, 100));
    }

    #[test]
    fn tasks_a_timed_await_gives_up_at_its_time_and_a_later_one_gets_the_state() {
        extern "C" fn slow(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            // SAFETY: as above.
            rt.sleep_ms(unsafe { &mut *context }, TOP, 500);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let start = scheduler::now();
            let handle = rt.spawn(context, task, slow, 0, None).expect("room");
            let none = rt.await_task_timeout(context, TOP, handle, 0);
            let gave_up = rt.await_task_timeout(context, TOP, handle, 100);
            let at = (scheduler::now() - start) / NANOS;
            let done = rt.await_task_timeout(context, TOP, handle, 1000);
            let again = rt.await_task_timeout(context, TOP, handle, 0);
            (none, gave_up, at, done, again, (scheduler::now() - start) / NANOS)
        });
        assert_eq!(got, (TASK_PENDING, TASK_PENDING, 100, TASK_DONE, TASK_DONE, 500));
    }

    #[test]
    fn tasks_a_task_cancelled_in_a_timed_await_or_a_sleep_ends_cancelled() {
        static HANDLE: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn forever(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            // SAFETY: as above.
            rt.sleep_ms(unsafe { &mut *context }, TOP, 100_000);
        }
        extern "C" fn waiter(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: as above.
            let rt = unsafe { Runtime::of(context) };
            let other = Object::from_address(HANDLE.load(Ordering::SeqCst)).expect("a handle");
            // SAFETY: as above.
            rt.await_task_timeout(unsafe { &mut *context }, TOP, other, 100_000);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let sleeper = rt.spawn(context, task, forever, 0, None).expect("room");
            HANDLE.store(sleeper.address(), Ordering::SeqCst);
            let waiting = rt.spawn(context, task, waiter, 0, None).expect("room");
            scheduler::sleep(10 * NANOS);
            rt.cancel_task(waiting);
            rt.cancel_task(sleeper);
            (rt.await_task(context, TOP, waiting), rt.await_task(context, TOP, sleeper))
        });
        assert_eq!(got, (TASK_CANCELLED, TASK_CANCELLED));
    }

    #[test]
    fn tasks_a_wait_that_gave_up_or_was_cancelled_leaves_the_waiters_of_the_task() {
        static HANDLE: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn slow(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            // SAFETY: as above.
            rt.sleep_ms(unsafe { &mut *context }, TOP, 500);
        }
        extern "C" fn waiter(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: as above.
            let rt = unsafe { Runtime::of(context) };
            let other = Object::from_address(HANDLE.load(Ordering::SeqCst)).expect("a handle");
            // SAFETY: as above.
            rt.await_task(unsafe { &mut *context }, TOP, other);
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let slow = rt.spawn(context, task, slow, 0, None).expect("room");
            let id = Runtime::number(slow);
            let waiters = |rt: &Runtime| rt.tasks.lock().expect("the tasks").recs[id].waiters.len();
            rt.await_task_timeout(context, TOP, slow, 100);
            let after_timeout = waiters(rt);
            HANDLE.store(slow.address(), Ordering::SeqCst);
            let waiting = rt.spawn(context, task, waiter, 0, None).expect("room");
            scheduler::sleep(10 * NANOS);
            let while_waiting = waiters(rt);
            rt.cancel_task(waiting);
            rt.await_task(context, TOP, waiting);
            (after_timeout, while_waiting, waiters(rt))
        });
        assert_eq!(got, (0, 1, 0));
    }

    #[test]
    fn tasks_the_record_of_a_task_is_freed_with_its_handle_and_its_place_is_used_again() {
        extern "C" fn quiet(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let first = rt.spawn(context, task, quiet, 0, None).expect("room");
            let id = Runtime::number(first);
            let name = rt.task_name(id);
            rt.await_task(context, TOP, first);
            let before = rt.task_ended(name);
            rt.poll(context, TOP);
            let second = rt.spawn(context, task, quiet, 0, None).expect("room");
            let (reused, recs) = (Runtime::number(second) == id, rt.tasks.lock().expect("the tasks").recs.len());
            (before, reused, recs, rt.task_name(id) != name, rt.task_ended(name))
        });
        assert_eq!(got, (true, true, 1, true, true));
    }

    #[test]
    fn tasks_a_view_of_a_task_shares_its_record_and_frees_nothing() {
        extern "C" fn quiet(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let first = rt.spawn(context, task, quiet, 0, None).expect("room");
            let view = rt.alloc(context, task, 0).expect("room");
            view.set_word(NUMBER, first.word(NUMBER));
            view.set_word(TASK_CONVERT_OFFSET, first.address() as u64);
            let id = Runtime::number(first);
            rt.await_task(context, TOP, view);
            rt.task_free(view);
            let after_view = rt.tasks.lock().expect("the tasks").recs[id].dead;
            rt.task_free(first);
            (Runtime::number(view) == id, rt.task_is_ready(view), after_view, rt.tasks.lock().expect("the tasks").recs[id].generation)
        });
        assert_eq!(got, (true, true, false, 1));
    }

    #[test]
    fn tasks_the_name_of_a_task_that_was_freed_is_ended_while_a_task_in_its_place_runs() {
        static GO: AtomicBool = AtomicBool::new(false);
        extern "C" fn quiet(_: *mut TaskContext, _: usize, _: usize) {}
        extern "C" fn waiting(context: *mut TaskContext, _: usize, _: usize) {
            while !GO.load(Ordering::SeqCst) {
                yield_poll(context);
            }
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let first = rt.spawn(context, task, quiet, 0, None).expect("room");
            let old = rt.task_name(Runtime::number(first));
            rt.await_task(context, TOP, first);
            rt.poll(context, TOP);
            let second = rt.spawn(context, task, waiting, 0, None).expect("room");
            let new = rt.task_name(Runtime::number(second));
            let running = (rt.task_ended(old), rt.task_ended(new));
            GO.store(true, Ordering::SeqCst);
            rt.await_task(context, TOP, second);
            (running, rt.task_ended(new))
        });
        assert_eq!(got, ((true, false), true));
    }

    #[test]
    fn tasks_a_task_that_faulted_unawaited_keeps_its_record_after_its_handle_died() {
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let handle = rt.spawn(context, task, faulting, 0, None).expect("room");
            while !rt.task_is_ready(handle) {
                scheduler::yield_now();
            }
            rt.poll(context, TOP);
            rt.unawaited_fault()
        });
        assert_eq!(got, Some(fault()));
    }

    #[test]
    fn tasks_a_scope_is_freed_when_it_ends_and_its_place_is_used_again() {
        extern "C" fn quiet(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let first = rt.scope_begin(context);
            rt.spawn(context, task, quiet, 0, Some(first)).expect("room");
            let ended = rt.scope_end(context, TOP, first);
            rt.poll(context, TOP);
            let second = rt.scope_begin(context);
            let ended_again = rt.scope_end(context, TOP, second);
            let tasks = rt.tasks.lock().expect("the tasks");
            (ended, ended_again, first == second, tasks.scopes.len(), tasks.recs.len(), tasks.free_recs.len())
        });
        assert_eq!(got, (ScopeEnd::Clean, ScopeEnd::Clean, true, 1, 1, 1));
    }

    #[test]
    fn tasks_a_fault_a_scope_did_not_report_is_not_reported_by_the_program_either() {
        extern "C" fn faulting(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig { torture: true, ..HeapConfig::default() });
        let task = ty.task;
        let got = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let scope = rt.scope_begin(context);
            rt.spawn(context, task, faulting, 0, Some(scope)).expect("room");
            rt.spawn(context, task, faulting, 0, Some(scope)).expect("room");
            let ended = rt.scope_end(context, TOP, scope);
            rt.poll(context, TOP);
            (ended, rt.unawaited_fault())
        });
        assert_eq!(got, (ScopeEnd::Faulted(fault()), None));
    }

    #[test]
    fn tasks_a_scope_of_a_task_that_faulted() {
        static TASK: AtomicUsize = AtomicUsize::new(0);
        static OPENED: AtomicBool = AtomicBool::new(false);
        static GO: AtomicBool = AtomicBool::new(false);
        extern "C" fn child(context: *mut TaskContext, _: usize, _: usize) {
            while !GO.load(Ordering::SeqCst) {
                yield_poll(context);
            }
        }
        extern "C" fn opener(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let scope = rt.scope_begin(context);
            // SAFETY: as above, and nothing else uses the context while it is spawning.
            rt.spawn(unsafe { &mut *context }, TypeId::from_raw(TASK.load(Ordering::SeqCst) as u32), child, 0, Some(scope)).expect("room");
            OPENED.store(true, Ordering::SeqCst);
            rt.end_task(context, TASK_FAULTED, Some(fault()));
        }
        let ty = types();
        let rt = runtime(&ty, HeapConfig::default());
        TASK.store(ty.task.raw() as usize, Ordering::SeqCst);
        let task = ty.task;
        let (while_running, after) = run(scheduler::Config::seeded(1), &rt, move |rt, context| {
            let opener = rt.spawn(context, task, opener, 0, None).expect("room");
            while !OPENED.load(Ordering::SeqCst) || !rt.task_is_ready(opener) {
                scheduler::yield_now();
            }
            let open = |rt: &Runtime| rt.tasks.lock().expect("the tasks").scopes.iter().filter(|scope| scope.open).count();
            let while_running = open(rt);
            GO.store(true, Ordering::SeqCst);
            while rt.tasks.lock().expect("the tasks").recs.iter().any(|rec| rec.state == TASK_PENDING) {
                scheduler::yield_now();
            }
            (while_running, open(rt))
        });
        assert_eq!((while_running, after), (1, 0));
    }
}
