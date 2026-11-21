//! Ports: a library's own threads push byte messages that a `Receiver` takes, and the program is not called a deadlock while a port is open.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use mote_abi::{FaultKind, MotePort, TaskContext, TypeId, ELEMENTS_OFFSET, PUSH_CLOSED, PUSH_FULL, PUSH_QUEUED, TASK_CANCELLED};
use mote_heap::Object;
use mote_scheduler::{self as scheduler, ForeignSource, TaskHandle, Wake};

use crate::channels::{first_in, join, leave, Failure};
use crate::collect::Top;
use crate::report::{backtrace, FaultReport};
use crate::state::Runtime;

/// The word of a `Port` handle that holds the reference to its `PortShared`.
const PORT: u32 = 8;

/// How a port keeps the messages the program has not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortMode {
    Queue(usize),
    Latest,
}

#[derive(Default)]
struct PortState {
    messages: VecDeque<Vec<u8>>,
    /// The library finished: no more messages come.
    closed: bool,
    /// The `Receiver` closed: no message is taken.
    receiver_closed: bool,
    /// The tasks that wait for a message.
    parked: Vec<TaskHandle>,
    /// Keeps a wait from being a deadlock while the port is open.
    source: Option<ForeignSource>,
}

/// What a library holds a `MotePort *` to: the messages and the state of one port.
pub struct PortShared {
    mode: PortMode,
    state: Mutex<PortState>,
}

impl PortShared {
    fn new(mode: PortMode, source: Option<ForeignSource>) -> Arc<Self> {
        Arc::new(Self { mode, state: Mutex::new(PortState { source, ..PortState::default() }) })
    }

    fn lock(&self) -> MutexGuard<'_, PortState> {
        self.state.lock().expect("the port")
    }

    /// Queues a copy of the message; one of the `PUSH_*` results.
    pub fn push(&self, bytes: &[u8]) -> i32 {
        let woken = {
            let mut state = self.lock();
            if state.closed || state.receiver_closed {
                return PUSH_CLOSED;
            }
            match self.mode {
                PortMode::Latest => state.messages.clear(),
                PortMode::Queue(n) if state.messages.len() >= n => return PUSH_FULL,
                PortMode::Queue(_) => {}
            }
            state.messages.push_back(bytes.to_vec());
            std::mem::take(&mut state.parked)
        };
        woken.iter().for_each(TaskHandle::unpark);
        PUSH_QUEUED
    }

    /// The library finished.
    pub fn close(&self) {
        let (woken, source) = {
            let mut state = self.lock();
            state.closed = true;
            (std::mem::take(&mut state.parked), state.source.take())
        };
        woken.iter().for_each(TaskHandle::unpark);
        drop(source);
    }

    /// The `Receiver` closed: the messages left are dropped and no more are taken.
    pub fn receiver_close(&self) {
        let (woken, source) = {
            let mut state = self.lock();
            state.receiver_closed = true;
            state.messages.clear();
            (std::mem::take(&mut state.parked), state.source.take())
        };
        woken.iter().for_each(TaskHandle::unpark);
        drop(source);
    }

    /// Whether no message can come and none is left.
    pub fn finished(&self) -> bool {
        let state = self.lock();
        (state.closed || state.receiver_closed) && state.messages.is_empty()
    }
}

/// The ports the program opened, which the host reaches by the order they were opened in; one nobody holds any more is dropped from the list.
pub(crate) struct Ports {
    opened: usize,
    live: Vec<(usize, Weak<PortShared>)>,
    /// The size of the list at which the dropped ports are removed.
    prune_at: usize,
}

impl Default for Ports {
    fn default() -> Self {
        Self { opened: 0, live: Vec::new(), prune_at: PRUNE_FROM }
    }
}

/// The smallest list of ports that is pruned.
const PRUNE_FROM: usize = 16;

impl Ports {
    fn add(&mut self, port: &Arc<PortShared>) {
        if self.live.len() >= self.prune_at {
            self.live.retain(|(_, port)| port.strong_count() > 0);
            self.prune_at = (self.live.len() * 2).max(PRUNE_FROM);
        }
        self.live.push((self.opened, Arc::downgrade(port)));
        self.opened += 1;
    }

    /// The port opened `index`th: `None` before the program opens it, and `Some(None)` after it was dropped.
    fn get(&self, index: usize) -> Option<Option<Arc<PortShared>>> {
        if index >= self.opened {
            return None;
        }
        Some(self.live.binary_search_by_key(&index, |(opened, _)| *opened).ok().and_then(|at| self.live[at].1.upgrade()))
    }
}

/// `MoteHost::port_push`: copies the message into the port.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn host_port_push(port: *mut MotePort, bytes: *const u8, length: i64) -> i32 {
    // SAFETY: the library holds a reference it was given and has not released.
    let shared = unsafe { &*(port as *const PortShared) };
    let message = if bytes.is_null() || length <= 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(bytes, length as usize) } };
    shared.push(message)
}

/// `MoteHost::port_close`: the library is finished and releases its reference.
pub extern "C" fn host_port_close(port: *mut MotePort) {
    // SAFETY: the library holds a reference it was given and releases it here.
    let shared = unsafe { Arc::from_raw(port as *const PortShared) };
    shared.close();
}

/// Releases the reference a dying `Port` handle holds.
pub(crate) fn release_handle(handle: Object) {
    let address = handle.word(PORT) as usize;
    if address != 0 {
        handle.set_word(PORT, 0);
        // SAFETY: the handle owns the reference `port_new` stored in it.
        drop(unsafe { Arc::from_raw(address as *const PortShared) });
    }
}

impl Runtime {
    /// The `PortShared` of a `Port` handle.
    fn shared_of(handle: Object) -> Arc<PortShared> {
        let address = handle.word(PORT) as usize;
        // SAFETY: the handle holds a reference until it dies; this one is another.
        unsafe {
            Arc::increment_strong_count(address as *const PortShared);
            Arc::from_raw(address as *const PortShared)
        }
    }

    /// The `Port` handle of the type for the channel; `queue` is the number of messages it keeps and 0 means the newest only.
    pub fn port_new(&self, context: &mut TaskContext, channel: Object, ty: TypeId, queue: i64) -> Result<Object, Failure> {
        let handle = self.alloc(context, ty, 0)?;
        let mode = if queue <= 0 { PortMode::Latest } else { PortMode::Queue(queue as usize) };
        let shared = PortShared::new(mode, (!self.is_scripted()).then(scheduler::foreign_source));
        handle.set_word(PORT, Arc::into_raw(shared.clone()) as u64);
        self.ports.lock().expect("the ports").add(&shared);
        self.attach_port(channel, shared);
        Ok(handle)
    }

    /// A `MotePort *` for a library, which owns one reference to release with `port_close`.
    pub fn port_ref(&self, handle: Object) -> usize {
        Arc::into_raw(Self::shared_of(handle)) as usize
    }

    /// Pushes a message to the port opened `index`th; none when the program has not opened it yet, and a port nobody holds any more is closed.
    pub fn feed_port(&self, index: usize, bytes: &[u8]) -> Option<i32> {
        let port = self.ports.lock().expect("the ports").get(index)?;
        Some(port.map_or(PUSH_CLOSED, |port| port.push(bytes)))
    }

    /// Closes the port opened `index`th; false when the program has not opened it yet.
    pub fn close_port(&self, index: usize) -> bool {
        let port = self.ports.lock().expect("the ports").get(index);
        port.inspect(|port| port.iter().for_each(|port| port.close())).is_some()
    }

    /// How many ports the program has opened.
    pub fn ports_opened(&self) -> usize {
        self.ports.lock().expect("the ports").opened
    }

    /// Takes a message of the port into a new array at `dest`: 1 for a message, 0 when none can come or none is there and the call does not wait, and 2 when a wait ended with none.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn port_recv(&self, context: &mut TaskContext, top: Top, channel: Object, number: usize, port: &Arc<PortShared>, dest: usize, wait: bool, deadline: Option<u64>) -> u64 {
        let me = Self::waiter();
        let (message, others) = {
            let mut channels = self.channels.lock().expect("the channels");
            let mut state = port.lock();
            let turn = !wait || first_in(channels.receivers(number), me);
            match turn.then(|| state.messages.pop_front()).flatten() {
                Some(message) => {
                    leave(channels.receivers(number), me);
                    let others = if state.messages.is_empty() { Vec::new() } else { std::mem::take(&mut state.parked) };
                    (message, others)
                }
                None => {
                    let over = deadline.is_some_and(|at| scheduler::now() >= at);
                    if state.closed && state.messages.is_empty() || state.receiver_closed || !wait || over {
                        leave(channels.receivers(number), me);
                        let others = if state.messages.is_empty() { Vec::new() } else { std::mem::take(&mut state.parked) };
                        drop(state);
                        drop(channels);
                        others.iter().for_each(TaskHandle::unpark);
                        return 0;
                    }
                    join(channels.receivers(number), me);
                    state.parked.push(scheduler::current());
                    drop(state);
                    drop(channels);
                    let park = || match deadline {
                        Some(at) => scheduler::park_timeout(at.saturating_sub(scheduler::now()), "a port"),
                        None => scheduler::park_on("a port"),
                    };
                    if self.blocked(context, top, park) == Wake::Cancelled {
                        let others = {
                            let mut channels = self.channels.lock().expect("the channels");
                            leave(channels.receivers(number), me);
                            std::mem::take(&mut port.lock().parked)
                        };
                        others.iter().for_each(TaskHandle::unpark);
                        self.end_task(context, TASK_CANCELLED, None);
                    }
                    return 2;
                }
            }
        };
        others.iter().for_each(TaskHandle::unpark);
        let (element, ..) = self.buffer(channel);
        match self.alloc(context, element, message.len() as u64) {
            Ok(array) => {
                // SAFETY: the array has room for `length` bytes from `ELEMENTS_OFFSET`, and `dest` has room for a reference.
                unsafe {
                    std::ptr::copy_nonoverlapping(message.as_ptr(), (array.address() + ELEMENTS_OFFSET as usize) as *mut u8, message.len());
                    (dest as *mut usize).write(array.address());
                }
                1
            }
            Err(_) => {
                let report = FaultReport { kind: FaultKind::MemoryLimit, message: FaultKind::MemoryLimit.message().to_string(), site: None, span: None, backtrace: backtrace(top.fp, top.pc) };
                self.stop(context, report)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use mote_abi::{FaultTable, HandleKind, Kind, Scalar, TypeTable, TASK_DONE};
    use mote_heap::Config as HeapConfig;

    use super::*;
    use crate::channels::NANOS as MILLIS;

    const TOP: Top = Top { fp: 0, pc: 0 };

    struct Types {
        table: Arc<TypeTable>,
        channel: TypeId,
        receiver: TypeId,
        port: TypeId,
        task: TypeId,
    }

    fn types() -> Types {
        let mut table = TypeTable::new();
        let byte = table.add("UInt8", Kind::Scalar(Scalar::I8));
        let message = table.add("[UInt8]", Kind::Array(byte));
        let ring = table.add("[[UInt8]]", Kind::Array(message));
        let channel = table.add("Channel<[UInt8]>", Kind::Handle(HandleKind::Channel, vec![ring]));
        let receiver = table.add("Receiver<[UInt8]>", Kind::Handle(HandleKind::Receiver, vec![]));
        let port = table.add("PortEnd", Kind::Handle(HandleKind::Port, vec![]));
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let task = table.add("Task<Int>", Kind::Handle(HandleKind::Task, vec![int]));
        Types { table: Arc::new(table), channel, receiver, port, task }
    }

    fn runtime(ty: &Types) -> Arc<Runtime> {
        Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig::default())
    }

    /// Runs `f` as the main task of a scheduler with the real clock, as a mutator of the runtime; an open port keeps a virtual clock from moving.
    fn run<R: Send + 'static>(runtime: &Arc<Runtime>, f: impl FnOnce(&Runtime, &mut TaskContext) -> R + Send + 'static) -> R {
        let runtime = runtime.clone();
        scheduler::Scheduler::new(scheduler::Config::new()).expect("a scheduler").run(move || {
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

    /// A port of the mode with its channel and receiver.
    fn open(rt: &Runtime, context: &mut TaskContext, ty: &Types, queue: i64) -> (Object, Object, Object) {
        let channel = rt.channel_new(context, ty.channel, 1).expect("room");
        let rx = rt.channel_end(context, channel, ty.receiver).expect("room");
        let port = rt.port_new(context, channel, ty.port, queue).expect("room");
        (channel, port, rx)
    }

    fn bytes_of(array: usize) -> Vec<u8> {
        let array = Object::from_address(array).expect("an array");
        // SAFETY: the array holds `length` bytes from `ELEMENTS_OFFSET`.
        unsafe { std::slice::from_raw_parts((array.address() + ELEMENTS_OFFSET as usize) as *const u8, array.length() as usize).to_vec() }
    }

    fn recv(rt: &Runtime, context: &mut TaskContext, rx: Object, wait: bool) -> Option<Vec<u8>> {
        loop {
            let mut slot = 0usize;
            match rt.channel_recv(context, TOP, rx, &mut slot as *mut usize as usize, wait) {
                1 => return Some(bytes_of(slot)),
                0 => return None,
                _ => {}
            }
        }
    }

    #[test]
    fn ports_a_queue_keeps_up_to_its_size_and_a_latest_port_keeps_the_newest() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, queue, _) = open(rt, context, &ty, 2);
            let (_, latest, lrx) = open(rt, context, &ty, 0);
            let queue = Runtime::shared_of(queue);
            let pushed = [queue.push(&[1]), queue.push(&[2]), queue.push(&[3])];
            let latest = Runtime::shared_of(latest);
            latest.push(&[1, 1]);
            latest.push(&[2, 2]);
            (pushed, recv(rt, context, lrx, false), recv(rt, context, lrx, false))
        });
        assert_eq!(got, ([PUSH_QUEUED, PUSH_QUEUED, PUSH_FULL], Some(vec![2, 2]), None));
    }

    #[test]
    fn ports_a_receiver_takes_the_messages_in_order_as_new_byte_arrays() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, port, rx) = open(rt, context, &ty, 4);
            let shared = Runtime::shared_of(port);
            shared.push(&[1, 2, 3]);
            shared.push(&[]);
            shared.push(&[9]);
            shared.close();
            let first = recv(rt, context, rx, true);
            let second = recv(rt, context, rx, true);
            let third = recv(rt, context, rx, true);
            let end = (recv(rt, context, rx, true), rt.receiver_closed(rx));
            (first, second, third, end)
        });
        assert_eq!(got, (Some(vec![1, 2, 3]), Some(vec![]), Some(vec![9]), (None, true)));
    }

    #[test]
    fn ports_a_closed_port_refuses_pushes_and_a_closed_receiver_does_too() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, port, _) = open(rt, context, &ty, 4);
            let (_, other, rx) = open(rt, context, &ty, 4);
            let (shared, other) = (Runtime::shared_of(port), Runtime::shared_of(other));
            shared.close();
            other.push(&[1]);
            rt.receiver_close(rx);
            (shared.push(&[1]), other.push(&[1]), recv(rt, context, rx, false), rt.receiver_closed(rx))
        });
        assert_eq!(got, (PUSH_CLOSED, PUSH_CLOSED, None, true));
    }

    #[test]
    fn ports_a_receiver_waits_for_a_thread_outside_the_scheduler_and_that_is_no_deadlock() {
        static PORT: AtomicUsize = AtomicUsize::new(0);
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, port, rx) = open(rt, context, &ty, 4);
            let address = rt.port_ref(port);
            PORT.store(address, Ordering::SeqCst);
            let feeder = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                let address = PORT.load(Ordering::SeqCst) as *mut MotePort;
                let first = host_port_push(address, [7u8, 8].as_ptr(), 2);
                std::thread::sleep(Duration::from_millis(10));
                host_port_close(address);
                first
            });
            let first = recv(rt, context, rx, true);
            let end = recv(rt, context, rx, true);
            (feeder.join().expect("the feeder"), first, end)
        });
        assert_eq!(got, (PUSH_QUEUED, Some(vec![7, 8]), None));
    }

    #[test]
    fn ports_a_timed_receive_gives_up_on_a_port_that_stays_quiet() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, port, rx) = open(rt, context, &ty, 4);
            let mut slot = 0usize;
            let start = scheduler::now();
            let status = rt.channel_recv_timeout(context, TOP, rx, &mut slot as *mut usize as usize, 50);
            let waited = (scheduler::now() - start) / MILLIS;
            Runtime::shared_of(port).close();
            (status, (50..1000).contains(&waited))
        });
        assert_eq!(got, (0, true));
    }

    #[test]
    fn ports_the_runtime_feeds_and_closes_the_port_opened_nth() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let before = (rt.feed_port(0, &[1]), rt.close_port(0), rt.ports_opened());
            let (_, _, rx) = open(rt, context, &ty, 4);
            let after = (rt.feed_port(0, &[5]), rt.ports_opened());
            rt.close_port(0);
            (before, after, recv(rt, context, rx, true), recv(rt, context, rx, true))
        });
        assert_eq!(got, ((None, false, 0), (Some(PUSH_QUEUED), 1), Some(vec![5]), None));
    }

    #[test]
    fn ports_a_native_reference_outlives_the_handle_and_the_handle_releases_its_own() {
        let ty = types();
        let rt = runtime(&ty);
        let counts = run(&rt, move |rt, context| {
            let (_, port, _) = open(rt, context, &ty, 4);
            let held = rt.port_ref(port);
            let with_both = Arc::strong_count(&Runtime::shared_of(port)) - 1;
            release_handle(port);
            release_handle(port);
            // SAFETY: `held` is the reference `port_ref` made.
            let shared = unsafe { Arc::from_raw(held as *const PortShared) };
            let alone = Arc::strong_count(&shared);
            (with_both, alone, shared.push(&[1]))
        });
        // The channel's record holds one more.
        assert_eq!(counts, (3, 2, PUSH_QUEUED));
    }

    #[test]
    fn ports_a_task_cancelled_while_it_waits_for_a_message_ends_cancelled() {
        static RX: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn waiter(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let rx = Object::from_address(RX.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            recv(rt, unsafe { &mut *context }, rx, true);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let (_, port, rx) = open(rt, context, &ty, 4);
            RX.store(rx.address(), Ordering::SeqCst);
            let handle = rt.spawn(context, task, waiter, 0, None).expect("room");
            scheduler::sleep(10 * MILLIS);
            rt.cancel_task(handle);
            let state = rt.await_task(context, TOP, handle);
            Runtime::shared_of(port).close();
            state
        });
        assert_ne!(got, TASK_DONE);
    }

    #[test]
    fn ports_waiting_receivers_of_one_port_take_the_messages_oldest_first() {
        static RX: AtomicUsize = AtomicUsize::new(0);
        static FIRST: AtomicUsize = AtomicUsize::new(0);
        static SECOND: AtomicUsize = AtomicUsize::new(0);
        fn take(context: *mut TaskContext, into: &AtomicUsize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let rx = Object::from_address(RX.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            into.store(recv(rt, unsafe { &mut *context }, rx, true).unwrap()[0] as usize, Ordering::SeqCst);
        }
        extern "C" fn first(context: *mut TaskContext, _: usize, _: usize) {
            take(context, &FIRST);
        }
        extern "C" fn second(context: *mut TaskContext, _: usize, _: usize) {
            take(context, &SECOND);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        run(&rt, move |rt, context| {
            let (_, port, rx) = open(rt, context, &ty, 4);
            RX.store(rx.address(), Ordering::SeqCst);
            let a = rt.spawn(context, task, first, 0, None).expect("room");
            while rt.receivers_waiting(rx) < 1 {
                scheduler::yield_now();
            }
            let b = rt.spawn(context, task, second, 0, None).expect("room");
            while rt.receivers_waiting(rx) < 2 {
                scheduler::yield_now();
            }
            let shared = Runtime::shared_of(port);
            shared.push(&[1]);
            shared.push(&[2]);
            rt.await_task(context, TOP, a);
            rt.await_task(context, TOP, b);
            shared.close();
        });
        assert_eq!((FIRST.load(Ordering::SeqCst), SECOND.load(Ordering::SeqCst)), (1, 2));
    }

    #[test]
    fn ports_a_port_whose_channel_died_is_closed_to_the_library() {
        let ty = types();
        let rt = Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig { torture: true, ..HeapConfig::default() });
        let got = run(&rt, move |rt, context| {
            let (_, port, _) = open(rt, context, &ty, 4);
            let held = rt.port_ref(port);
            let before = host_port_push(held as *mut MotePort, [1u8].as_ptr(), 1);
            rt.poll(context, TOP);
            let after = host_port_push(held as *mut MotePort, [1u8].as_ptr(), 1);
            host_port_close(held as *mut MotePort);
            (before, after)
        });
        assert_eq!(got, (PUSH_QUEUED, PUSH_CLOSED));
    }

    #[test]
    fn ports_the_list_of_ports_drops_those_nobody_holds_and_a_dropped_one_is_closed() {
        let ty = types();
        let rt = Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig { torture: true, ..HeapConfig::default() });
        let got = run(&rt, move |rt, context| {
            for _ in 0..100 {
                open(rt, context, &ty, 1);
                rt.poll(context, TOP);
            }
            let listed = rt.ports.lock().expect("the ports").live.len();
            (rt.ports_opened(), listed < 40, rt.feed_port(0, &[1]), rt.close_port(0), rt.feed_port(100, &[1]))
        });
        assert_eq!(got, (100, true, Some(PUSH_CLOSED), true, None));
    }
}
