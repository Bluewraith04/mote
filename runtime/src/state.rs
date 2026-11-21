//! The runtime of one program: its heap, its fault table and what runs when the program stops.

use std::any::Any;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Instant;

use mote_abi::{FaultTable, HandleKind, Kind, Region, TaskContext, TypeId, TypeTable, ELEMENTS_OFFSET, TASK_FAULTED};
use mote_heap::{Config, Full, Heap, Object, World};

use mote_scheduler::TaskHandle;

use crate::channels::Channels;
use crate::collect::{CodeMap, Top, Visit};
use crate::locks::Locks;
use crate::ports::{release_handle, Ports};
use crate::report::FaultReport;
use crate::tasks::{Cleanup, Tasks};

/// What the host does with a fault; it ends the task or the process, and if it returns the process aborts.
pub type FaultHook = Arc<dyn Fn(&FaultReport) + Send + Sync>;

/// A mutator that has joined: its context by address, and where it waits while it is parked at a safepoint or in a blocking call.
pub(crate) struct Joined {
    pub(crate) context: usize,
    pub(crate) top: Option<Top>,
    /// A stop waits for it to poll; false while its task is blocked in a call of the runtime.
    pub(crate) counted: bool,
    /// The scheduler's task, when the mutator is one.
    pub(crate) task: Option<TaskHandle>,
    /// The number of the program's task the scheduler task runs.
    pub(crate) number: Option<usize>,
    /// The `with` and `scope` blocks the task is inside, innermost last.
    pub(crate) cleanups: Vec<Cleanup>,
}

pub struct Runtime {
    heap: Heap,
    world: World,
    /// The mutators, whose contexts a stop makes poll.
    pub(crate) mutators: Mutex<Vec<Joined>>,
    pub(crate) tasks: Mutex<Tasks>,
    pub(crate) channels: Mutex<Channels>,
    pub(crate) locks: Mutex<Locks>,
    /// The ports the program opened, in order.
    pub(crate) ports: Mutex<Ports>,
    /// The host feeds the ports on the scheduler's clock, so an open port is no source outside the scheduler.
    scripted: AtomicBool,
    code: OnceLock<CodeMap>,
    collections: AtomicUsize,
    problems: Mutex<Vec<String>>,
    faults: FaultTable,
    hook: Mutex<Option<FaultHook>>,
    output: Mutex<Vec<u8>>,
    sink: Mutex<Option<Box<dyn Write + Send>>>,
    data: OnceLock<Box<dyn Any + Send + Sync>>,
    /// The arguments the program is given, after the name of the program.
    args: Mutex<Vec<String>>,
    /// The code `exit` asked for, or the entry returned; 0 otherwise.
    exit_code: AtomicI32,
    /// When the runtime was constructed, for single-threaded `now()` clocks.
    started: Instant,
}

impl Runtime {
    pub fn new(types: Arc<TypeTable>, faults: FaultTable, config: Config) -> Arc<Self> {
        Arc::new_cyclic(|me: &Weak<Runtime>| {
            let heap = Heap::new(types, config);
            let me = me.clone();
            heap.set_finalizer(Box::new(move |object| {
                if let Some(runtime) = me.upgrade() {
                    runtime.finalize(object);
                }
            }));
            Self {
                heap,
                world: World::new(),
                mutators: Mutex::new(Vec::new()),
                tasks: Mutex::new(Tasks::default()),
                channels: Mutex::new(Channels::default()),
                locks: Mutex::new(Locks::default()),
                ports: Mutex::new(Ports::default()),
                scripted: AtomicBool::new(false),
                code: OnceLock::new(),
                collections: AtomicUsize::new(0),
                problems: Mutex::new(Vec::new()),
                faults,
                hook: Mutex::new(None),
                output: Mutex::new(Vec::new()),
                sink: Mutex::new(None),
                data: OnceLock::new(),
                args: Mutex::new(Vec::new()),
                exit_code: AtomicI32::new(0),
                started: Instant::now(),
            }
        })
    }

    /// Elapsed nanoseconds since this runtime started, for single-threaded clocks.
    pub fn monotonic_now(&self) -> u64 {
        self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    /// Frees what the runtime keeps for a handle object the collector found dead.
    fn finalize(&self, object: Object) {
        if let Kind::Handle(handle, _) = &self.heap.table().get(object.header().type_id()).kind {
            match handle {
                HandleKind::Port => release_handle(object),
                HandleKind::Task => self.task_free(object),
                HandleKind::Channel => self.channel_free(object),
                HandleKind::Mutex => self.mutex_free(object),
                HandleKind::Once => self.once_free(object),
                _ => {}
            }
        }
    }

    /// Makes the host the only source of port messages, fed on the scheduler's clock, so a virtual clock moves while a port is open.
    pub fn set_scripted(&self, scripted: bool) {
        self.scripted.store(scripted, Ordering::SeqCst);
    }

    pub(crate) fn is_scripted(&self) -> bool {
        self.scripted.load(Ordering::SeqCst)
    }

    /// Keeps the frame descriptors and the static roots of the compiled code; the first call wins.
    pub fn set_code_map(&self, code: CodeMap) {
        let _ = self.code.set(code);
    }

    /// The code map, once it is set.
    pub fn code_map(&self) -> Option<&CodeMap> {
        self.code.get()
    }

    /// Collections made so far.
    pub fn collections(&self) -> usize {
        self.collections.load(Ordering::Relaxed)
    }

    /// What the checks of the collections found wrong, which the heap's `verify` setting turns on; the list is emptied.
    pub fn take_problems(&self) -> Vec<String> {
        std::mem::take(&mut *self.problems.lock().expect("the problems"))
    }

    /// A zeroed object from the context's region; a collection is asked for at the task's next poll when the heap wants one.
    pub fn alloc(&self, context: &mut TaskContext, ty: TypeId, count: u64) -> Result<Object, Full> {
        let mut region = context.region();
        let result = self.heap.alloc(&mut region, ty, count);
        if self.heap.config().torture {
            // The next object is not bumped inline, so each allocation reaches this function and the poll that follows it.
            region = Region { cursor: 0, limit: 0 };
        }
        context.set_region(region);
        if self.wants_collection() {
            // SAFETY: the context is borrowed, so it is alive.
            unsafe { TaskContext::request_poll(context) };
        }
        result
    }

    /// A string object of the text from the context's region.
    pub fn alloc_string(&self, context: &mut TaskContext, text: &str) -> Result<Object, Full> {
        let string = self.heap.table().iter().find(|(_, info)| info.kind == Kind::String).map(|(id, _)| id).ok_or(Full)?;
        let object = self.alloc(context, string, text.len() as u64)?;
        // SAFETY: a string of `len` bytes has room for them from `ELEMENTS_OFFSET`.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), (object.address() + ELEMENTS_OFFSET as usize) as *mut u8, text.len()) };
        Ok(object)
    }

    fn wants_collection(&self) -> bool {
        self.heap.config().torture || self.heap.wants_collection()
    }

    /// Keeps what the code generator knows about the program for the helpers its code calls; the first call wins.
    pub fn set_data<T: Any + Send + Sync>(&self, data: T) {
        let _ = self.data.set(Box::new(data));
    }

    /// What `set_data` kept, when it was a `T`.
    pub fn data<T: Any>(&self) -> Option<&T> {
        self.data.get().and_then(|data| (**data).downcast_ref::<T>())
    }

    /// Sends what the program prints to the writer as it is printed, instead of keeping it for `take_output`.
    pub fn set_sink(&self, sink: Box<dyn Write + Send>) {
        *self.sink.lock().expect("the sink") = Some(sink);
    }

    /// Flushes the writer of `set_sink`.
    pub fn flush_sink(&self) {
        if let Some(sink) = self.sink.lock().expect("the sink").as_mut() {
            let _ = sink.flush();
        }
    }

    /// Adds to the text the program printed.
    pub fn write_output(&self, bytes: &[u8]) {
        if let Some(sink) = self.sink.lock().expect("the sink").as_mut() {
            let _ = sink.write_all(bytes);
            return;
        }
        self.output.lock().expect("the output").extend_from_slice(bytes);
    }

    /// Writes to standard error.
    pub fn write_error(&self, bytes: &[u8]) {
        let _ = std::io::stderr().write_all(bytes);
    }

    /// Sets the arguments the program is given.
    pub fn set_program_args(&self, args: Vec<String>) {
        *self.args.lock().expect("the args") = args;
    }

    /// The arguments the program is given.
    pub fn program_args(&self) -> Vec<String> {
        self.args.lock().expect("the args").clone()
    }

    /// Sets the exit code of the run.
    pub fn set_exit_code(&self, code: i32) {
        self.exit_code.store(code, Ordering::Relaxed);
    }

    /// The exit code of the run: what `exit` asked for, else what the entry returned, else 0.
    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Relaxed)
    }

    /// Ends the run with the code, from the calling task.
    pub fn exit(&self, context: *mut TaskContext, code: i32) -> ! {
        self.set_exit_code(code);
        // SAFETY: the context is the calling task's own block.
        self.leave(unsafe { &mut *context });
        mote_scheduler::exit()
    }

    /// The text the program printed since the last call.
    pub fn take_output(&self) -> Vec<u8> {
        std::mem::take(&mut *self.output.lock().expect("the output"))
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    pub fn faults(&self) -> &FaultTable {
        &self.faults
    }

    /// Sets what runs on a fault; without one the report goes to standard error and the process exits with code 1.
    pub fn set_fault_hook(&self, hook: FaultHook) {
        *self.hook.lock().expect("the hook") = Some(hook);
    }

    /// Points the context at this runtime, which must outlive every use of the context.
    pub fn attach(self: &Arc<Self>, context: &mut TaskContext) {
        context.runtime = Arc::as_ptr(self) as u64;
    }

    /// Counts the attached context's task as a mutator: a stop waits until its code polls.
    pub fn join(&self, context: &mut TaskContext) {
        self.world.enter();
        self.mutators.lock().expect("the mutators").push(Joined { context: context as *mut TaskContext as usize, top: None, counted: true, task: None, number: None, cleanups: Vec::new() });
    }

    /// Counts the program's first task as a mutator, as `join` does for a thread; a stop scans its stack through the scheduler's handle while it is blocked in the runtime.
    pub fn join_task(&self, context: &mut TaskContext, task: TaskHandle) {
        self.task_enter(context, task, None);
    }

    /// Stops counting every mutator that is left, which a run does when it ends with tasks still waiting.
    pub fn leave_all(&self) {
        let left = std::mem::take(&mut *self.mutators.lock().expect("the mutators"));
        for _ in left.iter().filter(|joined| joined.counted) {
            self.world.leave();
        }
    }

    /// Counts the calling scheduler task as a mutator, as `join` does for a thread's program.
    pub(crate) fn task_enter(&self, context: *mut TaskContext, task: TaskHandle, number: Option<usize>) {
        self.world.enter();
        self.mutators.lock().expect("the mutators").push(Joined { context: context as usize, top: None, counted: true, task: Some(task), number, cleanups: Vec::new() });
    }

    /// Stops counting a scheduler task, whose context is gone from the list.
    pub(crate) fn task_exit(&self, context: *mut TaskContext) {
        let found = {
            let mut mutators = self.mutators.lock().expect("the mutators");
            mutators.iter().position(|found| found.context == context as usize).map(|at| mutators.swap_remove(at))
        };
        if found.is_some_and(|found| found.counted) {
            self.world.leave();
        }
    }

    pub(crate) fn update(&self, context: *const TaskContext, change: impl FnOnce(&mut Joined)) {
        if let Some(joined) = self.mutators.lock().expect("the mutators").iter_mut().find(|joined| joined.context == context as usize) {
            change(joined);
        }
    }

    /// Runs `f`, which blocks and touches no object, with the task counted as parked and its stack scanned from `top`.
    pub(crate) fn blocked<R>(&self, context: *const TaskContext, top: Top, f: impl FnOnce() -> R) -> R {
        self.update(context, |joined| {
            joined.top = Some(top);
            joined.counted = false;
        });
        self.world.leave();
        let result = f();
        // Counted before the wait for a stop to end, so a stop that begins meanwhile asks this task to poll.
        self.update(context, |joined| joined.counted = true);
        self.world.enter();
        self.update(context, |joined| joined.top = None);
        result
    }

    /// Stops counting the context's task as a mutator.
    pub fn leave(&self, context: &mut TaskContext) {
        let address = context as *mut TaskContext as usize;
        let found = {
            let mut mutators = self.mutators.lock().expect("the mutators");
            mutators.iter().position(|found| found.context == address).map(|at| mutators.swap_remove(at))
        };
        if found.is_some() {
            self.world.leave();
        }
    }

    /// Asks every joined context but `except` to poll.
    ///
    /// # Safety
    /// Each context that `join` counted and `leave` has not removed is alive.
    unsafe fn request_polls(&self, except: usize) {
        for joined in self.mutators.lock().expect("the mutators").iter().filter(|joined| joined.context != except && joined.counted) {
            // SAFETY: the caller's promise; the word is atomic because the task reads it without a lock.
            unsafe { TaskContext::request_poll(joined.context as *mut TaskContext) };
        }
    }

    /// Runs `f` while every mutator waits at a safepoint; for a thread that is not a mutator.
    ///
    /// # Safety
    /// Each context that `join` counted and `leave` has not removed is alive.
    pub unsafe fn stop_world<R>(&self, f: impl FnOnce() -> R) -> R {
        // SAFETY: the caller's promise.
        self.world.stop_requesting(|| unsafe { self.request_polls(0) }, f)
    }

    fn set_top(&self, context: &TaskContext, top: Option<Top>) {
        let address = context as *const TaskContext as usize;
        if let Some(joined) = self.mutators.lock().expect("the mutators").iter_mut().find(|joined| joined.context == address) {
            joined.top = top;
        }
    }

    /// The safepoint of the task whose context this is: clears its poll word, collects when the heap wants it, and waits while a stop is in progress.
    pub(crate) fn poll(&self, context: &mut TaskContext, top: Top) {
        let flagged = context.take_poll();
        self.set_top(context, Some(top));
        if self.wants_collection() {
            let own = context as *mut TaskContext as usize;
            // SAFETY: the joined contexts are alive until their tasks leave, and this one is parked.
            self.world.stop_from_mutator(|| unsafe { self.request_polls(own) }, || self.collect_stopped());
        } else {
            self.world.poll();
        }
        self.set_top(context, None);
        if flagged && mote_scheduler::in_a_task() {
            self.preempt(context, top);
        }
    }

    /// Collects from the globals and the stacks of the joined mutators, which are all parked.
    fn collect_stopped(&self) {
        if !self.wants_collection() {
            return;
        }
        let (contexts, stacks, resources) = {
            let mutators = self.mutators.lock().expect("the mutators");
            let stacks = mutators.iter().filter_map(|joined| joined.top.map(|top| (top, joined.task.clone().filter(|_| !joined.counted)))).collect();
            let resources: Vec<usize> = mutators.iter().flat_map(|joined| joined.cleanups.iter().filter_map(Cleanup::resource)).collect();
            (mutators.iter().map(|joined| joined.context).collect(), stacks, resources)
        };
        let mut objects = self.tasks.lock().expect("the tasks").roots();
        objects.extend(resources);
        objects.extend(self.channels.lock().expect("the channels").roots());
        let mut visit = Visit { code: self.code.get(), stacks, objects, contexts };
        let found = self.heap.collect(&mut visit);
        self.collections.fetch_add(1, Ordering::Relaxed);
        self.problems.lock().expect("the problems").extend(found.problems);
    }

    /// The runtime a context is attached to.
    ///
    /// # Safety
    /// `context` points to a live context that `attach` set, and the runtime is alive.
    pub unsafe fn of<'a>(context: *mut TaskContext) -> &'a Runtime {
        // SAFETY: the caller's promise.
        let pointer = unsafe { (*context).runtime } as *const Runtime;
        assert!(!pointer.is_null(), "a context is attached before its code runs");
        // SAFETY: the same.
        unsafe { &*pointer }
    }

    /// Stops the program: records the site in the context, tells the host, and never returns.
    pub(crate) fn stop(&self, context: &mut TaskContext, report: FaultReport) -> ! {
        if let Some(site) = report.site {
            context.fault_site = u64::from(site) + 1;
        }
        if self.spawned(context).is_some() {
            self.end_task(context, TASK_FAULTED, Some(report));
        }
        let hook = self.hook.lock().expect("the hook").clone();
        match hook {
            Some(hook) => hook(&report),
            None => {
                eprintln!("{report}");
                std::process::exit(1);
            }
        }
        std::process::abort()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_the_text_a_program_prints_is_kept_until_the_host_takes_it() {
        let runtime = Runtime::new(Arc::new(TypeTable::new()), FaultTable::new(), Config::default());
        runtime.write_output(b"one\n");
        runtime.write_output(b"two\n");
        assert_eq!(runtime.take_output(), b"one\ntwo\n");
        assert_eq!(runtime.take_output(), b"");
    }

    #[test]
    fn entries_a_stop_asks_each_joined_context_to_poll_and_a_left_one_is_not_asked() {
        let runtime = Runtime::new(Arc::new(TypeTable::new()), FaultTable::new(), Config::default());
        let mut kept = TaskContext::default();
        let mut left = TaskContext::default();
        runtime.join(&mut left);
        runtime.leave(&mut left);
        runtime.mutators.lock().expect("the mutators").push(Joined { context: &mut kept as *mut TaskContext as usize, top: None, counted: true, task: None, number: None, cleanups: Vec::new() });
        // The kept context is not a mutator of the world, so the stop does not wait for it.
        // SAFETY: both contexts are alive.
        unsafe { runtime.stop_world(|| ()) };
        assert_eq!((kept.poll, left.poll), (1, 0));
        runtime.mutators.lock().expect("the mutators").clear();
    }

    #[test]
    fn entries_a_context_finds_the_runtime_it_was_attached_to() {
        let runtime = Runtime::new(Arc::new(TypeTable::new()), FaultTable::new(), Config::default());
        let mut context = TaskContext::default();
        runtime.attach(&mut context);
        // SAFETY: the context was attached and the runtime is alive.
        let found = unsafe { Runtime::of(&mut context) };
        assert!(std::ptr::eq(found, Arc::as_ptr(&runtime)));
    }
}
