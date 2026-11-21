//! Mutexes and onces on the scheduler: the handles are heap objects; the queues of waiting tasks are kept here.

use std::collections::VecDeque;
use std::sync::MutexGuard;

use mote_abi::{HandleKind, Kind, TaskContext, TypeId, ONCE_DONE, ONCE_RUNNING, TASK_CANCELLED};
use mote_heap::Object;
use mote_scheduler::{self as scheduler, TaskHandle, Wake};

use crate::channels::Failure;
use crate::collect::Top;
use crate::state::Runtime;

/// The words of a `Mutex` handle.
const OWNER: u32 = 8;
const MUTEX_WAITERS: u32 = 16;

/// The words of a `Once` handle.
const STATE: u32 = 8;
const RUNNER: u32 = 16;
const ONCE_WAITERS: u32 = 24;

/// What holds a mutex or runs a once: 0 for nothing, 1 for the program's first task, and 2 more than the name (place and generation) of any other.
const FIRST: u64 = 1;

/// The tasks that wait on one mutex or once.
#[derive(Default)]
struct Waiting {
    /// The tasks that wait for a mutex, oldest first; the first one takes it next.
    queue: VecDeque<u64>,
    /// Every task that sleeps until something changes.
    parked: Vec<TaskHandle>,
}

/// The waits on every mutex and once.
#[derive(Default)]
pub(crate) struct Locks {
    lists: Vec<Waiting>,
    /// The places of the lists whose mutex or once died.
    free: Vec<usize>,
}

impl Locks {
    /// A list for a new mutex or once, in a freed place when there is one.
    fn open(&mut self) -> usize {
        self.free.pop().unwrap_or_else(|| {
            self.lists.push(Waiting::default());
            self.lists.len() - 1
        })
    }

    /// Frees the list of a mutex or once that died.
    fn close(&mut self, number: usize) {
        if let Some(waiting) = self.lists.get_mut(number) {
            *waiting = Waiting::default();
            self.free.push(number);
        }
    }

    /// Every task that sleeps on a mutex or a once, which a task that stopped may have been holding up.
    pub(crate) fn sleepers(&mut self) -> Vec<TaskHandle> {
        self.lists.iter_mut().flat_map(|waiting| std::mem::take(&mut waiting.parked)).collect()
    }
}

impl Runtime {
    fn locks(&self) -> MutexGuard<'_, Locks> {
        self.locks.lock().expect("the locks")
    }

    /// What the calling task is called in a `Mutex` owner word and a `Once` runner word.
    fn holder(&self, context: *mut TaskContext) -> u64 {
        self.spawned(context).map_or(FIRST, |number| self.task_name(number) + 2)
    }

    /// Whether the task a word names has ended; the program's first task is still running.
    fn holder_ended(&self, holder: u64) -> bool {
        holder > FIRST && self.task_ended(holder - 2)
    }

    /// The offset and size of the value a `Mutex` or a `Once` holds.
    fn held(&self, handle: Object) -> (usize, usize) {
        let table = self.heap().table();
        let field = table.get_raw(handle.header().type_id().raw()).and_then(|info| info.object.as_ref()).and_then(|shape| shape.fields.first().copied()).expect("a lock holds a value");
        (field.offset as usize, table.layout(field.ty).size as usize)
    }

    /// Wakes the tasks that sleep on the list.
    fn wake_list(&self, number: usize) {
        let woken = std::mem::take(&mut self.locks().lists[number].parked);
        woken.iter().for_each(TaskHandle::unpark);
    }

    /// Wakes every task that sleeps on a mutex or a once, as a task that ended may have held it.
    pub(crate) fn wake_locks(&self) {
        let woken = self.locks().sleepers();
        woken.iter().for_each(TaskHandle::unpark);
    }

    /// Sleeps until something changes on the list; a cancelled task leaves its queue and ends.
    fn sleep_on_lock(&self, context: &mut TaskContext, top: Top, number: usize, me: u64) {
        if self.blocked(context, top, || scheduler::park_on("a lock")) == Wake::Cancelled {
            self.locks().lists[number].queue.retain(|&other| other != me);
            self.wake_list(number);
            self.end_task(context, TASK_CANCELLED, None);
        }
    }

    fn new_list(&self) -> u64 {
        self.locks().open() as u64
    }

    /// Frees the list of a `Mutex` that died.
    pub(crate) fn mutex_free(&self, mutex: Object) {
        self.locks().close(mutex.word(MUTEX_WAITERS) as usize);
    }

    /// Frees the list of a `Once` that died.
    pub(crate) fn once_free(&self, once: Object) {
        self.locks().close(once.word(ONCE_WAITERS) as usize);
    }

    /// A `Mutex` of the type that is not locked.
    pub fn mutex_new(&self, context: &mut TaskContext, ty: TypeId) -> Result<Object, Failure> {
        if !matches!(self.heap().table().get_raw(ty.raw()).map(|info| &info.kind), Some(Kind::Handle(HandleKind::Mutex, _))) {
            return Err(Failure::Raised("a mutex of a type that is not a mutex"));
        }
        let mutex = self.alloc(context, ty, 0)?;
        mutex.set_word(MUTEX_WAITERS, self.new_list());
        Ok(mutex)
    }

    /// Locks the mutex for the calling task, waiting while another holds it, and copies its value to `dest`.
    pub fn mutex_take(&self, context: &mut TaskContext, top: Top, mutex: Object, dest: usize) -> Result<(), Failure> {
        let me = self.holder(context);
        let queued = Self::waiter();
        let number = mutex.word(MUTEX_WAITERS) as usize;
        loop {
            let mut locks = self.locks();
            let owner = mutex.word(OWNER);
            if owner == me {
                return Err(Failure::Raised("Mutex is already locked by this task"));
            }
            if owner != 0 && self.holder_ended(owner) {
                return Err(Failure::Raised("Mutex was left locked by a task that stopped"));
            }
            let waiting = &mut locks.lists[number];
            if owner == 0 && waiting.queue.front().is_none_or(|&front| front == queued) {
                mutex.set_word(OWNER, me);
                waiting.queue.retain(|&other| other != queued);
                let (offset, size) = self.held(mutex);
                // SAFETY: the value is inside the handle, which is live, and `dest` has room for it.
                unsafe { std::ptr::copy_nonoverlapping((mutex.address() + offset) as *const u8, dest as *mut u8, size) };
                return Ok(());
            }
            if !waiting.queue.contains(&queued) {
                waiting.queue.push_back(queued);
            }
            waiting.parked.push(scheduler::current());
            drop(locks);
            self.sleep_on_lock(context, top, number, queued);
        }
    }

    /// Stores the value at `source` in the mutex and unlocks it; the calling task holds it.
    pub fn mutex_put(&self, context: &mut TaskContext, mutex: Object, source: usize) -> Result<(), Failure> {
        let me = self.holder(context);
        let locks = self.locks();
        if mutex.word(OWNER) != me {
            return Err(Failure::Raised("Mutex is not locked by this task"));
        }
        let (offset, size) = self.held(mutex);
        // SAFETY: the value is inside the handle, which is live, and `source` holds a value.
        unsafe { std::ptr::copy_nonoverlapping(source as *const u8, (mutex.address() + offset) as *mut u8, size) };
        mutex.set_word(OWNER, 0);
        drop(locks);
        self.wake_list(mutex.word(MUTEX_WAITERS) as usize);
        Ok(())
    }

    /// A `Once` of the type that has no value.
    pub fn once_new(&self, context: &mut TaskContext, ty: TypeId) -> Result<Object, Failure> {
        if !matches!(self.heap().table().get_raw(ty.raw()).map(|info| &info.kind), Some(Kind::Handle(HandleKind::Once, _))) {
            return Err(Failure::Raised("a once of a type that is not a once"));
        }
        let once = self.alloc(context, ty, 0)?;
        once.set_word(ONCE_WAITERS, self.new_list());
        Ok(once)
    }

    /// Whether the calling task is to run the initializer: false when the once has a value, also after waiting for the task that runs it.
    pub fn once_begin(&self, context: &mut TaskContext, top: Top, once: Object) -> Result<bool, Failure> {
        let me = self.holder(context);
        let number = once.word(ONCE_WAITERS) as usize;
        loop {
            let mut locks = self.locks();
            match once.word(STATE) {
                ONCE_DONE => return Ok(false),
                ONCE_RUNNING => {
                    let runner = once.word(RUNNER);
                    if runner == me {
                        return Err(Failure::Raised("Once is used inside its own initializer"));
                    }
                    if !self.holder_ended(runner) {
                        locks.lists[number].parked.push(scheduler::current());
                        drop(locks);
                        self.sleep_on_lock(context, top, number, Self::waiter());
                        continue;
                    }
                }
                _ => {}
            }
            once.set_word(STATE, ONCE_RUNNING);
            once.set_word(RUNNER, me);
            return Ok(true);
        }
    }

    /// Stores the value at `source` in the once, which then has a value.
    pub fn once_finish(&self, once: Object, source: usize) {
        let locks = self.locks();
        let (offset, size) = self.held(once);
        // SAFETY: the value is inside the handle, which is live, and `source` holds a value.
        unsafe { std::ptr::copy_nonoverlapping(source as *const u8, (once.address() + offset) as *mut u8, size) };
        once.set_word(STATE, ONCE_DONE);
        once.set_word(RUNNER, 0);
        drop(locks);
        self.wake_list(once.word(ONCE_WAITERS) as usize);
    }

    /// Copies the value of the once to `dest`.
    pub fn once_value(&self, once: Object, dest: usize) -> Result<(), Failure> {
        let _locks = self.locks();
        if once.word(STATE) != ONCE_DONE {
            return Err(Failure::Raised("Once has no value"));
        }
        let (offset, size) = self.held(once);
        // SAFETY: the value is inside the handle, which is live, and `dest` has room for it.
        unsafe { std::ptr::copy_nonoverlapping((once.address() + offset) as *const u8, dest as *mut u8, size) };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use mote_abi::{FaultTable, Scalar, TypeTable, TASK_DONE};
    use mote_heap::Config as HeapConfig;

    use super::*;

    const TOP: Top = Top { fp: 0, pc: 0 };
    const VALUE: u32 = 24;
    const ONCE_VALUE: u32 = 32;

    struct Types {
        table: Arc<TypeTable>,
        mutex: TypeId,
        once: TypeId,
        task: TypeId,
    }

    fn types() -> Types {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let mutex = table.add("Mutex<Int>", Kind::Handle(HandleKind::Mutex, vec![int]));
        let once = table.add("Once<Int>", Kind::Handle(HandleKind::Once, vec![int]));
        let task = table.add("Task<Int>", Kind::Handle(HandleKind::Task, vec![int]));
        Types { table: Arc::new(table), mutex, once, task }
    }

    fn runtime(ty: &Types) -> Arc<Runtime> {
        Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig::default())
    }

    /// Runs `f` as the main task of a seeded scheduler, as a mutator of the runtime.
    fn run<R: Send + 'static>(runtime: &Arc<Runtime>, f: impl FnOnce(&Runtime, &mut TaskContext) -> R + Send + 'static) -> R {
        let runtime = runtime.clone();
        scheduler::Scheduler::new(scheduler::Config::seeded(1)).expect("a scheduler").run(move || {
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

    fn take(rt: &Runtime, context: &mut TaskContext, mutex: Object) -> Result<i64, Failure> {
        let mut value = 0i64;
        rt.mutex_take(context, TOP, mutex, &mut value as *mut i64 as usize)?;
        Ok(value)
    }

    fn put(rt: &Runtime, context: &mut TaskContext, mutex: Object, value: i64) -> Result<(), Failure> {
        rt.mutex_put(context, mutex, &value as *const i64 as usize)
    }

    fn mutex_of(rt: &Runtime, context: &mut TaskContext, ty: &Types, value: i64) -> Object {
        let mutex = rt.mutex_new(context, ty.mutex).expect("room");
        mutex.set_word(VALUE, value as u64);
        mutex
    }

    #[test]
    fn locks_a_mutex_gives_its_value_to_the_task_that_takes_it_and_keeps_the_value_put_back() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let mutex = mutex_of(rt, context, &ty, 7);
            let owner_before = mutex.word(OWNER);
            let first = take(rt, context, mutex);
            let owner_while = mutex.word(OWNER);
            put(rt, context, mutex, 8).expect("held");
            (owner_before, first, owner_while, mutex.word(OWNER), take(rt, context, mutex))
        });
        assert_eq!(got, (0, Ok(7), FIRST, 0, Ok(8)));
    }

    #[test]
    fn locks_taking_a_mutex_twice_or_putting_one_that_is_not_held_is_refused() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let mutex = mutex_of(rt, context, &ty, 1);
            let unheld = put(rt, context, mutex, 2);
            take(rt, context, mutex).expect("free");
            (unheld, take(rt, context, mutex))
        });
        assert_eq!(got, (Err(Failure::Raised("Mutex is not locked by this task")), Err(Failure::Raised("Mutex is already locked by this task"))));
    }

    #[test]
    fn locks_a_task_that_waits_for_a_mutex_takes_it_when_the_holder_puts_it_back() {
        static SHARED: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn adder(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let mutex = Object::from_address(SHARED.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            let context = unsafe { &mut *context };
            let value = take(rt, context, mutex).expect("free in the end");
            put(rt, context, mutex, value + 1).expect("held");
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let mutex = mutex_of(rt, context, &ty, 5);
            SHARED.store(mutex.address(), Ordering::SeqCst);
            take(rt, context, mutex).expect("free");
            let handle = rt.spawn(context, task, adder, 0, None).expect("room");
            let list = mutex.word(MUTEX_WAITERS) as usize;
            while rt.locks().lists[list].queue.is_empty() {
                scheduler::yield_now();
            }
            put(rt, context, mutex, 10).expect("held");
            let state = rt.await_task(context, TOP, handle);
            (state, take(rt, context, mutex))
        });
        assert_eq!(got, (TASK_DONE, Ok(11)));
    }

    #[test]
    fn locks_a_mutex_left_held_by_a_task_that_ended_is_refused() {
        static SHARED: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn keeper(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let mutex = Object::from_address(SHARED.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            take(rt, unsafe { &mut *context }, mutex).expect("free");
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let mutex = mutex_of(rt, context, &ty, 5);
            SHARED.store(mutex.address(), Ordering::SeqCst);
            let handle = rt.spawn(context, task, keeper, 0, None).expect("room");
            rt.await_task(context, TOP, handle);
            take(rt, context, mutex)
        });
        assert_eq!(got, Err(Failure::Raised("Mutex was left locked by a task that stopped")));
    }

    #[test]
    fn locks_a_task_cancelled_while_it_waits_for_a_mutex_leaves_the_queue() {
        static SHARED: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn waiter(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let mutex = Object::from_address(SHARED.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            let _ = take(rt, unsafe { &mut *context }, mutex);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let mutex = mutex_of(rt, context, &ty, 5);
            SHARED.store(mutex.address(), Ordering::SeqCst);
            take(rt, context, mutex).expect("free");
            let handle = rt.spawn(context, task, waiter, 0, None).expect("room");
            let list = mutex.word(MUTEX_WAITERS) as usize;
            while rt.locks().lists[list].queue.is_empty() {
                scheduler::yield_now();
            }
            rt.cancel_task(handle);
            let state = rt.await_task(context, TOP, handle);
            let queued = rt.locks().lists[list].queue.len();
            put(rt, context, mutex, 6).expect("held");
            (state, queued, take(rt, context, mutex))
        });
        assert_eq!(got, (mote_abi::TASK_CANCELLED, 0, Ok(6)));
    }

    #[test]
    fn locks_a_once_runs_its_initializer_once_and_then_gives_the_value() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let once = rt.once_new(context, ty.once).expect("room");
            let mut value = 0i64;
            let empty = rt.once_value(once, &mut value as *mut i64 as usize);
            let first = rt.once_begin(context, TOP, once);
            let inside = rt.once_begin(context, TOP, once);
            let seven = 7i64;
            rt.once_finish(once, &seven as *const i64 as usize);
            let later = rt.once_begin(context, TOP, once);
            let found = rt.once_value(once, &mut value as *mut i64 as usize);
            (empty, first, inside, later, found, value, once.word(STATE), once.word(RUNNER), once.word(ONCE_VALUE))
        });
        assert_eq!(got, (Err(Failure::Raised("Once has no value")), Ok(true), Err(Failure::Raised("Once is used inside its own initializer")), Ok(false), Ok(()), 7, ONCE_DONE, 0, 7));
    }

    #[test]
    fn locks_a_task_that_asks_a_once_that_is_running_waits_for_the_value() {
        static SHARED: AtomicUsize = AtomicUsize::new(0);
        static SEEN: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn asker(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let once = Object::from_address(SHARED.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            let ran = rt.once_begin(unsafe { &mut *context }, TOP, once).expect("not inside");
            let mut value = 0i64;
            rt.once_value(once, &mut value as *mut i64 as usize).expect("done");
            SEEN.store(value as usize + usize::from(ran) * 100, Ordering::SeqCst);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        run(&rt, move |rt, context| {
            let once = rt.once_new(context, ty.once).expect("room");
            SHARED.store(once.address(), Ordering::SeqCst);
            assert_eq!(rt.once_begin(context, TOP, once), Ok(true));
            let handle = rt.spawn(context, task, asker, 0, None).expect("room");
            let list = once.word(ONCE_WAITERS) as usize;
            while rt.locks().lists[list].parked.is_empty() {
                scheduler::yield_now();
            }
            let nine = 9i64;
            rt.once_finish(once, &nine as *const i64 as usize);
            rt.await_task(context, TOP, handle);
        });
        assert_eq!(SEEN.load(Ordering::SeqCst), 9);
    }

    #[test]
    fn locks_a_once_whose_runner_ended_without_a_value_is_run_again() {
        static SHARED: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn runner(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let once = Object::from_address(SHARED.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            let _ = rt.once_begin(unsafe { &mut *context }, TOP, once);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let once = rt.once_new(context, ty.once).expect("room");
            SHARED.store(once.address(), Ordering::SeqCst);
            let handle = rt.spawn(context, task, runner, 0, None).expect("room");
            rt.await_task(context, TOP, handle);
            rt.once_begin(context, TOP, once)
        });
        assert_eq!(got, Ok(true));
    }

    #[test]
    fn locks_a_handle_of_the_wrong_type_is_not_made() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| (rt.mutex_new(context, ty.once).unwrap_err(), rt.once_new(context, ty.mutex).unwrap_err()));
        assert_eq!(got, (Failure::Raised("a mutex of a type that is not a mutex"), Failure::Raised("a once of a type that is not a once")));
    }

    #[test]
    fn locks_the_list_of_a_mutex_or_once_that_died_is_freed_and_its_place_is_used_again() {
        let ty = types();
        let rt = Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig { torture: true, ..HeapConfig::default() });
        let got = run(&rt, move |rt, context| {
            rt.mutex_new(context, ty.mutex).expect("room");
            rt.once_new(context, ty.once).expect("room");
            rt.poll(context, TOP);
            let freed = rt.locks().free.len();
            rt.mutex_new(context, ty.mutex).expect("room");
            rt.once_new(context, ty.once).expect("room");
            let locks = rt.locks();
            (freed, locks.lists.len(), locks.free.len())
        });
        assert_eq!(got, (2, 2, 0));
    }

    #[test]
    fn locks_a_mutex_left_locked_by_a_freed_task_is_refused_while_another_task_has_its_place() {
        static GO: AtomicBool = AtomicBool::new(false);
        extern "C" fn hold(context: *mut TaskContext, closure: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            // SAFETY: as above.
            let _ = take(rt, unsafe { &mut *context }, Object::from_address(closure).unwrap());
        }
        extern "C" fn waiting(context: *mut TaskContext, _: usize, _: usize) {
            while !GO.load(Ordering::SeqCst) {
                // SAFETY: the context is the calling task's own and attached.
                unsafe {
                    TaskContext::request_poll(context);
                    Runtime::of(context).poll(&mut *context, TOP);
                }
            }
        }
        let ty = types();
        let rt = Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig { torture: true, ..HeapConfig::default() });
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let mutex = mutex_of(rt, context, &ty, 1);
            let holder = rt.spawn(context, task, hold, mutex.address(), None).expect("room");
            rt.await_task(context, TOP, holder);
            let keeper = rt.spawn(context, task, waiting, mutex.address(), None).expect("room");
            rt.poll(context, TOP);
            let other = rt.spawn(context, task, waiting, mutex.address(), None).expect("room");
            let taken = take(rt, context, mutex);
            GO.store(true, Ordering::SeqCst);
            rt.await_task(context, TOP, keeper);
            rt.await_task(context, TOP, other);
            taken
        });
        assert_eq!(got, Err(Failure::Raised("Mutex was left locked by a task that stopped")));
    }
}
