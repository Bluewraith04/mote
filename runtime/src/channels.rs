//! Channels on the scheduler: the ends and the buffer are heap objects; the queues of waiting tasks and the owners of the `Sender` ends are kept here.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, MutexGuard};

use mote_abi::{align_up, HandleKind, Kind, PairPayload, TaskContext, TypeId, TypeTable, BOX_VALUE_OFFSET, ELEMENTS_OFFSET, TASK_CANCELLED};
use mote_heap::{Full, Object};
use mote_scheduler::{self as scheduler, TaskHandle, Wake};

use crate::collect::Top;
use crate::ports::PortShared;
use crate::state::Runtime;

/// The words of a channel handle.
const SENDERS: u32 = 8;
const RECEIVERS: u32 = 16;
const CAPACITY: u32 = 24;
const HEAD: u32 = 32;
const LENGTH: u32 = 40;
const NUMBER: u32 = 48;
const BUFFER: u32 = 56;

/// The words of a `Sender` or `Receiver` handle.
const CHANNEL: u32 = 8;
const OPEN: u32 = 16;

/// How deep a message is searched for `Sender` ends.
const DEPTH: usize = 8;

/// Nanoseconds in a millisecond, the unit of the program's timeouts.
pub(crate) const NANOS: u64 = 1_000_000;

/// The owner of the ends the program's first task makes.
const MAIN: usize = usize::MAX;

/// Why a channel call stopped the program.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Raised(&'static str),
    Full,
}

impl From<Full> for Failure {
    fn from(_: Full) -> Self {
        Failure::Full
    }
}

/// The tasks that wait on one channel.
#[derive(Default)]
struct Waits {
    /// The tasks that wait to take a message, oldest first; the first one takes the next.
    receivers: VecDeque<u64>,
    /// The tasks that wait to put a message, oldest first.
    writers: VecDeque<u64>,
    /// Every task that sleeps until something changes.
    parked: Vec<TaskHandle>,
    /// Messages put and taken since the channel began.
    pushed: u64,
    popped: u64,
    /// The port that fills the channel, when it is one.
    port: Option<Arc<PortShared>>,
}

/// The waits on every channel and the `Sender` ends each task owns.
#[derive(Default)]
pub(crate) struct Channels {
    waits: Vec<Waits>,
    /// The places of the waits whose channel died.
    free: Vec<usize>,
    /// The task that owns an end, by the end's address; an end in a message has none.
    owners: HashMap<usize, usize>,
    /// The open ends each task owns, which stay alive until it ends and are closed then.
    ends: HashMap<usize, HashSet<usize>>,
}

impl Channels {
    /// The waits of a new channel, in a freed place when there is one.
    fn open(&mut self) -> usize {
        self.free.pop().unwrap_or_else(|| {
            self.waits.push(Waits::default());
            self.waits.len() - 1
        })
    }

    /// The tasks that wait to take a message from the channel, oldest first.
    pub(crate) fn receivers(&mut self, number: usize) -> &mut VecDeque<u64> {
        &mut self.waits[number].receivers
    }

    /// The ends that tasks which have not ended own.
    pub(crate) fn roots(&self) -> Vec<usize> {
        self.ends.values().flatten().copied().collect()
    }

    /// Gives the ends to the task, taking them from the task that owned them.
    fn adopt(&mut self, task: usize, found: Vec<usize>) {
        for end in found {
            self.release(end);
            self.owners.insert(end, task);
            self.ends.entry(task).or_default().insert(end);
        }
    }

    /// Takes the end from its owner.
    fn release(&mut self, end: usize) {
        if let Some(owner) = self.owners.remove(&end)
            && let Some(owned) = self.ends.get_mut(&owner)
        {
            owned.remove(&end);
        }
    }
}

fn read(address: usize) -> usize {
    // SAFETY: the callers read words inside live objects and values.
    unsafe { (address as *const usize).read() }
}

fn read_tag(address: usize, bytes: u32) -> usize {
    // SAFETY: as above; a tag is aligned to its size.
    unsafe {
        match bytes {
            1 => (address as *const u8).read() as usize,
            2 => (address as *const u16).read() as usize,
            4 => (address as *const u32).read() as usize,
            _ => (address as *const u64).read() as usize,
        }
    }
}

/// The `Sender` ends held by a value of the type at the address.
fn ends_in(table: &TypeTable, ty: TypeId, at: usize, depth: usize, out: &mut Vec<usize>) {
    let Some(info) = table.get_raw(ty.raw()) else { return };
    if depth >= DEPTH {
        return;
    }
    match &info.kind {
        Kind::Handle(HandleKind::Sender, _) => {
            let end = read(at);
            if end != 0 {
                out.push(end);
            }
        }
        Kind::Struct(_) => info.fields.iter().for_each(|field| ends_in(table, field.ty, at + field.offset as usize, depth + 1, out)),
        Kind::Enum(_) if !info.variants.is_empty() => {
            let tag = read_tag(at, info.tag_size);
            if let Some(variant) = info.variants.get(tag) {
                variant.fields.iter().for_each(|field| ends_in(table, field.ty, at + field.offset as usize, depth + 1, out));
            }
        }
        Kind::Optional(inner) if info.is_reference() => in_object(table, *inner, read(at), depth + 1, out),
        Kind::Optional(_) => {
            if read(at) != 0
                && let Some(variant) = info.variants.get(1)
            {
                variant.fields.iter().for_each(|field| ends_in(table, field.ty, at + field.offset as usize, depth + 1, out));
            }
        }
        Kind::Pair => {
            let word = read(at);
            if word != 0 {
                let held = TypeId::from_raw(word as u32);
                if table.pair_payload(held) == PairPayload::Boxed {
                    in_object(table, held, read(at + 8) + BOX_VALUE_OFFSET as usize, depth + 1, out);
                } else {
                    ends_in(table, held, at + 8, depth + 1, out);
                }
            }
        }
        Kind::FixedArray(element, count) => {
            let layout = table.layout(*element);
            let stride = align_up(layout.size, layout.align) as usize;
            for index in 0..*count as usize {
                ends_in(table, *element, at + index * stride, depth + 1, out);
            }
        }
        Kind::Class(_) | Kind::Box(_) | Kind::Closure(_) => in_object(table, ty, read(at), depth + 1, out),
        _ => {}
    }
}

/// The `Sender` ends in the fields of the object of the type; a null reference has none.
fn in_object(table: &TypeTable, ty: TypeId, object: usize, depth: usize, out: &mut Vec<usize>) {
    if object == 0 || depth >= DEPTH {
        return;
    }
    let Some(shape) = table.get_raw(ty.raw()).and_then(|info| info.object.as_ref()) else { return };
    shape.fields.iter().for_each(|field| ends_in(table, field.ty, object + field.offset as usize, depth + 1, out));
}

/// The object a `Sender` or `Receiver` end holds as its channel.
fn channel_of(end: Object) -> Object {
    Object::from_address(end.word(CHANNEL) as usize).expect("an end holds its channel")
}

/// Whether nothing more can come: every sender is gone or the receiving side closed.
fn closed(channel: Object) -> bool {
    channel.word(SENDERS) == 0 || channel.word(RECEIVERS) == 0
}

pub(crate) fn join(queue: &mut VecDeque<u64>, me: u64) {
    if !queue.contains(&me) {
        queue.push_back(me);
    }
}

pub(crate) fn first_in(queue: &VecDeque<u64>, me: u64) -> bool {
    queue.front().is_none_or(|&front| front == me)
}

pub(crate) fn leave(queue: &mut VecDeque<u64>, me: u64) {
    queue.retain(|&other| other != me);
}

impl Runtime {
    fn channels(&self) -> MutexGuard<'_, Channels> {
        self.channels.lock().expect("the channels")
    }

    /// The task whose ends a call makes: the number of the program's task on the context, or the first task.
    fn owner(&self, context: *mut TaskContext) -> usize {
        self.spawned(context).unwrap_or(MAIN)
    }

    /// The element type, its size and its stride, and the buffer of a channel.
    pub(crate) fn buffer(&self, channel: Object) -> (TypeId, usize, usize, Object) {
        let table = self.heap().table();
        let Some(Kind::Handle(HandleKind::Channel, payloads)) = table.get_raw(channel.header().type_id().raw()).map(|info| &info.kind) else { panic!("a channel") };
        let Some(Kind::Array(element)) = table.get_raw(payloads[0].raw()).map(|info| &info.kind) else { panic!("a channel buffer") };
        let layout = table.layout(*element);
        let buffer = Object::from_address(channel.word(BUFFER) as usize).expect("a channel holds its buffer");
        (*element, layout.size as usize, align_up(layout.size, layout.align) as usize, buffer)
    }

    /// The number the queues know the calling task by; a thread that is not a task of the scheduler has none and never waits.
    pub(crate) fn waiter() -> u64 {
        if scheduler::in_a_task() { scheduler::current().id() } else { u64::MAX }
    }

    /// The address of a message place of the buffer.
    fn place(buffer: Object, stride: usize, index: u64) -> usize {
        buffer.address() + ELEMENTS_OFFSET as usize + index as usize * stride
    }

    /// A channel of the type with room for `capacity` messages, which a rendezvous channel does not have.
    pub fn channel_new(&self, context: &mut TaskContext, ty: TypeId, capacity: i64) -> Result<Object, Failure> {
        if capacity < 0 {
            return Err(Failure::Raised("a channel buffer is not negative"));
        }
        let Some(Kind::Handle(HandleKind::Channel, payloads)) = self.heap().table().get_raw(ty.raw()).map(|info| &info.kind) else { return Err(Failure::Raised("a channel of a type that is not a channel")) };
        let buffer = self.alloc(context, payloads[0], (capacity as u64).max(1))?;
        let channel = self.alloc(context, ty, 0)?;
        let mut channels = self.channels();
        channel.set_word(NUMBER, channels.open() as u64);
        channel.set_word(RECEIVERS, 1);
        channel.set_word(CAPACITY, capacity as u64);
        channel.set_word(BUFFER, buffer.address() as u64);
        Ok(channel)
    }

    /// A new `Sender` or `Receiver` of the channel; a `Sender` belongs to the running task.
    pub fn channel_end(&self, context: &mut TaskContext, channel: Object, ty: TypeId) -> Result<Object, Failure> {
        let end = self.alloc(context, ty, 0)?;
        end.set_word(CHANNEL, channel.address() as u64);
        end.set_word(OPEN, 1);
        if matches!(self.heap().table().get_raw(ty.raw()).map(|info| &info.kind), Some(Kind::Handle(HandleKind::Sender, _))) {
            channel.set_word(SENDERS, channel.word(SENDERS) + 1);
            let owner = self.owner(context);
            self.channels().adopt(owner, vec![end.address()]);
        }
        Ok(end)
    }

    /// Wakes every task that sleeps on the channel.
    fn wake(waits: &mut Waits) -> Vec<TaskHandle> {
        std::mem::take(&mut waits.parked)
    }

    fn unpark(woken: Vec<TaskHandle>) {
        woken.iter().for_each(TaskHandle::unpark);
    }

    /// Sleeps until something changes on the channel; a cancelled task leaves the queues with `cleanup` and ends.
    fn sleep(&self, context: &mut TaskContext, top: Top, number: usize, cleanup: impl FnOnce(&mut Waits)) {
        self.sleep_until(context, top, number, None, cleanup);
    }

    /// Like `sleep`, for at most until the scheduler's clock reaches the deadline in nanoseconds.
    fn sleep_until(&self, context: &mut TaskContext, top: Top, number: usize, deadline: Option<u64>, cleanup: impl FnOnce(&mut Waits)) {
        let park = || match deadline {
            Some(at) => scheduler::park_timeout(at.saturating_sub(scheduler::now()), "a channel"),
            None => scheduler::park_on("a channel"),
        };
        if self.blocked(context, top, park) == Wake::Cancelled {
            let woken = {
                let mut channels = self.channels();
                let waits = &mut channels.waits[number];
                cleanup(waits);
                Self::wake(waits)
            };
            Self::unpark(woken);
            self.end_task(context, TASK_CANCELLED, None);
        }
    }

    /// Puts the message at `source` in the buffer; there is room.
    fn put(&self, channels: &mut Channels, channel: Object, source: usize) {
        let (element, size, stride, buffer) = self.buffer(channel);
        let (head, length) = (channel.word(HEAD), channel.word(LENGTH));
        let place = Self::place(buffer, stride, (head + length) % buffer.length());
        // SAFETY: the place is inside the buffer, which holds messages of `size` bytes, and `source` holds a message.
        unsafe { std::ptr::copy_nonoverlapping(source as *const u8, place as *mut u8, size) };
        channel.set_word(LENGTH, length + 1);
        let mut found = Vec::new();
        ends_in(self.heap().table(), element, source, 0, &mut found);
        for end in found {
            channels.release(end);
        }
        channels.waits[channel.word(NUMBER) as usize].pushed += 1;
    }

    /// Takes the oldest message to `dest`; there is one.
    fn take(&self, channels: &mut Channels, channel: Object, dest: usize, owner: usize) {
        let (element, size, stride, buffer) = self.buffer(channel);
        let (head, length) = (channel.word(HEAD), channel.word(LENGTH));
        let place = Self::place(buffer, stride, head);
        // SAFETY: the place is inside the buffer and `dest` has room for a message.
        unsafe {
            std::ptr::copy_nonoverlapping(place as *const u8, dest as *mut u8, size);
            std::ptr::write_bytes(place as *mut u8, 0, stride);
        }
        channel.set_word(HEAD, (head + 1) % buffer.length());
        channel.set_word(LENGTH, length - 1);
        let mut found = Vec::new();
        ends_in(self.heap().table(), element, dest, 0, &mut found);
        channels.adopt(owner, found);
        channels.waits[channel.word(NUMBER) as usize].popped += 1;
    }

    /// Sends the message at `source` through the sender. With `wait` the call sleeps until it can and answers false when a wait ended and it must be asked again; without, it answers whether there was room.
    pub fn channel_send(&self, context: &mut TaskContext, top: Top, tx: Object, source: usize, wait: bool) -> Result<bool, Failure> {
        let channel = channel_of(tx);
        let number = channel.word(NUMBER) as usize;
        let me = Self::waiter();
        let capacity = channel.word(CAPACITY);
        let mut channels = self.channels();
        let (open, taking) = (tx.word(OPEN) != 0, channel.word(RECEIVERS) != 0);
        if wait && !open {
            return Err(Failure::Raised("send on a closed Sender"));
        }
        if wait && !taking {
            return Err(Failure::Raised("send on a closed Receiver"));
        }
        let length = channel.word(LENGTH);
        let room = if capacity > 0 { length < capacity } else { length == 0 };
        if !wait {
            let waits = &channels.waits[number];
            let room = open && taking && room && (capacity > 0 || !waits.receivers.is_empty());
            if !(room && waits.writers.is_empty()) {
                return Ok(false);
            }
            self.put(&mut channels, channel, source);
            let woken = Self::wake(&mut channels.waits[number]);
            drop(channels);
            Self::unpark(woken);
            return Ok(true);
        }
        let waits = &mut channels.waits[number];
        if !(room && first_in(&waits.writers, me)) {
            join(&mut waits.writers, me);
            waits.parked.push(scheduler::current());
            drop(channels);
            self.sleep(context, top, number, |waits| leave(&mut waits.writers, me));
            return Ok(false);
        }
        self.put(&mut channels, channel, source);
        let ticket = channels.waits[number].pushed - 1;
        if capacity > 0 {
            let waits = &mut channels.waits[number];
            leave(&mut waits.writers, me);
            let woken = Self::wake(waits);
            drop(channels);
            Self::unpark(woken);
            return Ok(true);
        }
        // A rendezvous message is handed over when a receiver has taken it.
        join(&mut channels.waits[number].writers, me);
        let woken = Self::wake(&mut channels.waits[number]);
        Self::unpark(woken);
        loop {
            let waits = &mut channels.waits[number];
            if waits.popped > ticket {
                leave(&mut waits.writers, me);
                let woken = Self::wake(waits);
                drop(channels);
                Self::unpark(woken);
                return Ok(true);
            }
            waits.parked.push(scheduler::current());
            drop(channels);
            self.sleep(context, top, number, |waits| {
                leave(&mut waits.writers, me);
                if waits.popped <= ticket {
                    // The message was never taken: it goes with its sender.
                    waits.popped = waits.pushed;
                    channel.set_word(LENGTH, 0);
                    channel.set_word(HEAD, 0);
                    let (_, _, stride, buffer) = self.buffer(channel);
                    // SAFETY: the buffer is live and holds `length` places of `stride` bytes.
                    unsafe { std::ptr::write_bytes(Self::place(buffer, stride, 0) as *mut u8, 0, stride * buffer.length() as usize) };
                }
            });
            channels = self.channels();
        }
    }

    /// Takes a message through the receiver to `dest`: 1 for a message, 0 when the channel is closed and empty. With `wait` the call sleeps until one of them holds and answers 2 when a wait ended and it must be asked again.
    pub fn channel_recv(&self, context: &mut TaskContext, top: Top, rx: Object, dest: usize, wait: bool) -> u64 {
        self.recv_until(context, top, rx, dest, wait, None)
    }

    /// Like a waiting `channel_recv` that gives up with 0 after `ms` milliseconds of the scheduler's clock.
    pub fn channel_recv_timeout(&self, context: &mut TaskContext, top: Top, rx: Object, dest: usize, ms: i64) -> u64 {
        let deadline = scheduler::now().saturating_add((ms.max(0) as u64).saturating_mul(NANOS));
        loop {
            match self.recv_until(context, top, rx, dest, true, Some(deadline)) {
                2 => {}
                status => return status,
            }
        }
    }

    fn recv_until(&self, context: &mut TaskContext, top: Top, rx: Object, dest: usize, wait: bool, deadline: Option<u64>) -> u64 {
        let channel = channel_of(rx);
        let number = channel.word(NUMBER) as usize;
        let me = Self::waiter();
        let owner = self.owner(context);
        let mut channels = self.channels();
        if let Some(port) = channels.waits[number].port.clone() {
            drop(channels);
            return self.port_recv(context, top, channel, number, &port, dest, wait, deadline);
        }
        let length = channel.word(LENGTH);
        let waits = &channels.waits[number];
        let ready = length > 0 && (!wait || first_in(&waits.receivers, me));
        if ready {
            self.take(&mut channels, channel, dest, owner);
            let waits = &mut channels.waits[number];
            leave(&mut waits.receivers, me);
            let woken = Self::wake(waits);
            drop(channels);
            Self::unpark(woken);
            return 1;
        }
        if !wait || (length == 0 && closed(channel)) || deadline.is_some_and(|at| scheduler::now() >= at) {
            leave(&mut channels.waits[number].receivers, me);
            return 0;
        }
        let waits = &mut channels.waits[number];
        join(&mut waits.receivers, me);
        waits.parked.push(scheduler::current());
        drop(channels);
        self.sleep_until(context, top, number, deadline, |waits| leave(&mut waits.receivers, me));
        2
    }

    /// Closes the sender; the channel closes with its last sender.
    pub fn sender_close(&self, tx: Object) {
        let channel = channel_of(tx);
        let woken = {
            let mut channels = self.channels();
            if tx.word(OPEN) != 0 {
                tx.set_word(OPEN, 0);
                channel.set_word(SENDERS, channel.word(SENDERS).saturating_sub(1));
                channels.release(tx.address());
            }
            Self::wake(&mut channels.waits[channel.word(NUMBER) as usize])
        };
        Self::unpark(woken);
    }

    /// Another sender of the channel, owned by the running task.
    pub fn sender_clone(&self, context: &mut TaskContext, tx: Object) -> Result<Object, Failure> {
        if tx.word(OPEN) == 0 {
            return Err(Failure::Raised("clone of a closed Sender"));
        }
        self.channel_end(context, channel_of(tx), tx.header().type_id())
    }

    /// Closes the receiving side: the messages that are left are dropped and no more can be sent.
    pub fn receiver_close(&self, rx: Object) {
        let channel = channel_of(rx);
        let port = self.channels().waits[channel.word(NUMBER) as usize].port.clone();
        if let Some(port) = port {
            port.receiver_close();
            return;
        }
        let woken = {
            let mut channels = self.channels();
            channel.set_word(RECEIVERS, 0);
            let (_, _, stride, buffer) = self.buffer(channel);
            // SAFETY: the buffer is live and holds `length` places of `stride` bytes.
            unsafe { std::ptr::write_bytes(Self::place(buffer, stride, 0) as *mut u8, 0, stride * buffer.length() as usize) };
            channel.set_word(HEAD, 0);
            channel.set_word(LENGTH, 0);
            let waits = &mut channels.waits[channel.word(NUMBER) as usize];
            waits.popped = waits.pushed;
            Self::wake(waits)
        };
        Self::unpark(woken);
    }

    /// Another receiver of the channel.
    pub fn receiver_clone(&self, context: &mut TaskContext, rx: Object) -> Result<Object, Failure> {
        self.channel_end(context, channel_of(rx), rx.header().type_id())
    }

    /// Whether no message can come and none is left.
    pub fn receiver_closed(&self, rx: Object) -> bool {
        let channel = channel_of(rx);
        let channels = self.channels();
        if let Some(port) = &channels.waits[channel.word(NUMBER) as usize].port {
            return port.finished();
        }
        closed(channel) && channel.word(LENGTH) == 0
    }

    /// Makes the channel the one a port fills.
    pub(crate) fn attach_port(&self, channel: Object, port: Arc<PortShared>) {
        self.channels().waits[channel.word(NUMBER) as usize].port = Some(port);
    }

    /// Frees the waits of a channel that died, and closes the port that filled it: nothing can take its messages now.
    pub(crate) fn channel_free(&self, channel: Object) {
        let port = {
            let mut channels = self.channels();
            let number = channel.word(NUMBER) as usize;
            let Some(waits) = channels.waits.get_mut(number) else { return };
            let port = std::mem::take(waits).port;
            channels.free.push(number);
            port
        };
        if let Some(port) = port {
            port.receiver_close();
        }
    }

    /// Makes the running task the owner of the `Sender` ends its closure holds.
    pub(crate) fn own_captured(&self, task: usize, closure: usize) {
        let mut found = Vec::new();
        if let Some(object) = Object::from_address(closure) {
            in_object(self.heap().table(), object.header().type_id(), closure, 0, &mut found);
        }
        if !found.is_empty() {
            self.channels().adopt(task, found);
        }
    }

    /// Closes the ends the task owns, as it ends.
    pub(crate) fn close_ends(&self, task: usize) {
        let woken = {
            let mut channels = self.channels();
            let ends = channels.ends.remove(&task).unwrap_or_default();
            let mut woken = Vec::new();
            for address in ends {
                channels.owners.remove(&address);
                let end = Object::from_address(address).expect("an end");
                if end.word(OPEN) != 0 {
                    end.set_word(OPEN, 0);
                    let channel = channel_of(end);
                    channel.set_word(SENDERS, channel.word(SENDERS).saturating_sub(1));
                    woken.extend(Self::wake(&mut channels.waits[channel.word(NUMBER) as usize]));
                }
            }
            woken
        };
        Self::unpark(woken);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use mote_abi::{FaultTable, Scalar, CLOSURE_CAPTURES_OFFSET, TASK_CANCELLED, TASK_DONE};
    use mote_heap::Config as HeapConfig;

    use super::*;

    const TOP: Top = Top { fp: 0, pc: 0 };

    struct Types {
        table: Arc<TypeTable>,
        channel: TypeId,
        sender: TypeId,
        receiver: TypeId,
        closure: TypeId,
        task: TypeId,
        pair_channel: TypeId,
    }

    fn types() -> Types {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let array = table.add("[Int]", Kind::Array(int));
        let channel = table.add("Channel<Int>", Kind::Handle(HandleKind::Channel, vec![array]));
        let sender = table.add("Sender<Int>", Kind::Handle(HandleKind::Sender, vec![]));
        let receiver = table.add("Receiver<Int>", Kind::Handle(HandleKind::Receiver, vec![]));
        let closure = table.add("Closure", Kind::Closure(vec![sender]));
        let task = table.add("Task<Int>", Kind::Handle(HandleKind::Task, vec![int]));
        let pair = table.add("Pair", Kind::Struct(vec![int, sender]));
        let pairs = table.add("[Pair]", Kind::Array(pair));
        let pair_channel = table.add("Channel<Pair>", Kind::Handle(HandleKind::Channel, vec![pairs]));
        Types { table: Arc::new(table), channel, sender, receiver, closure, task, pair_channel }
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

    /// A channel of the type's channel type with its two ends.
    fn open(rt: &Runtime, context: &mut TaskContext, ty: &Types, capacity: i64) -> (Object, Object, Object) {
        let channel = rt.channel_new(context, ty.channel, capacity).expect("room");
        let tx = rt.channel_end(context, channel, ty.sender).expect("room");
        let rx = rt.channel_end(context, channel, ty.receiver).expect("room");
        (channel, tx, rx)
    }

    fn send(rt: &Runtime, context: &mut TaskContext, tx: Object, value: i64) {
        let mut message = value;
        while !rt.channel_send(context, TOP, tx, &mut message as *mut i64 as usize, true).expect("open") {}
    }

    fn recv(rt: &Runtime, context: &mut TaskContext, rx: Object) -> Option<i64> {
        loop {
            let mut message = 0i64;
            match rt.channel_recv(context, TOP, rx, &mut message as *mut i64 as usize, true) {
                1 => return Some(message),
                0 => return None,
                _ => {}
            }
        }
    }

    fn try_recv(rt: &Runtime, context: &mut TaskContext, rx: Object) -> Option<i64> {
        let mut message = 0i64;
        (rt.channel_recv(context, TOP, rx, &mut message as *mut i64 as usize, false) == 1).then_some(message)
    }

    fn try_send(rt: &Runtime, context: &mut TaskContext, tx: Object, value: i64) -> bool {
        let mut message = value;
        rt.channel_send(context, TOP, tx, &mut message as *mut i64 as usize, false).expect("open")
    }

    #[test]
    fn channels_a_new_channel_has_one_sender_one_receiver_and_a_buffer_of_its_capacity() {
        let ty = types();
        let rt = runtime(&ty);
        let words = run(&rt, move |rt, context| {
            let (channel, tx, rx) = open(rt, context, &ty, 3);
            let buffer = Object::from_address(channel.word(BUFFER) as usize).unwrap();
            ((channel.word(SENDERS), channel.word(RECEIVERS), channel.word(CAPACITY), buffer.length()), (tx.word(CHANNEL) == channel.address() as u64, tx.word(OPEN), rx.word(OPEN)))
        });
        assert_eq!(words, ((1, 1, 3, 3), (true, 1, 1)));
    }

    #[test]
    fn channels_a_rendezvous_channel_keeps_one_place_and_a_negative_capacity_is_refused() {
        let ty = types();
        let rt = runtime(&ty);
        let (length, refused) = run(&rt, move |rt, context| {
            let (channel, _, _) = open(rt, context, &ty, 0);
            let buffer = Object::from_address(channel.word(BUFFER) as usize).unwrap();
            (buffer.length(), rt.channel_new(context, ty.channel, -1).unwrap_err())
        });
        assert_eq!((length, refused), (1, Failure::Raised("a channel buffer is not negative")));
    }

    #[test]
    fn channels_a_buffered_channel_gives_its_messages_in_the_order_they_came() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 3);
            for value in [1, 2, 3] {
                send(rt, context, tx, value);
            }
            vec![recv(rt, context, rx), recv(rt, context, rx), recv(rt, context, rx)]
        });
        assert_eq!(got, [Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn channels_a_ring_that_wrapped_keeps_the_order() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 2);
            let mut got = Vec::new();
            for value in 0..7 {
                send(rt, context, tx, value);
                if value % 2 == 1 {
                    got.push(recv(rt, context, rx));
                    got.push(recv(rt, context, rx));
                }
            }
            got
        });
        assert_eq!(got, [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]);
    }

    #[test]
    fn channels_a_closed_channel_gives_none_once_it_is_drained() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 2);
            send(rt, context, tx, 7);
            rt.sender_close(tx);
            let before = rt.receiver_closed(rx);
            (before, recv(rt, context, rx), recv(rt, context, rx), rt.receiver_closed(rx))
        });
        assert_eq!(got, (false, Some(7), None, true));
    }

    #[test]
    fn channels_try_send_and_try_recv_never_wait() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 1);
            let empty = try_recv(rt, context, rx);
            let first = try_send(rt, context, tx, 1);
            let full = try_send(rt, context, tx, 2);
            (empty, first, full, try_recv(rt, context, rx))
        });
        assert_eq!(got, (None, true, false, Some(1)));
    }

    #[test]
    fn channels_try_send_on_a_rendezvous_channel_needs_a_receiver_that_waits() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, _) = open(rt, context, &ty, 0);
            try_send(rt, context, tx, 1)
        });
        assert!(!got);
    }

    #[test]
    fn channels_a_rendezvous_send_waits_until_a_receiver_takes_the_message() {
        static HANDOVER: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn sender(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let tx = Object::from_address(HANDOVER.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            send(rt, unsafe { &mut *context }, tx, 42);
            rt.sender_close(tx);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 0);
            HANDOVER.store(tx.address(), Ordering::SeqCst);
            let handle = rt.spawn(context, task, sender, 0, None).expect("room");
            let got = (recv(rt, context, rx), recv(rt, context, rx));
            (got, rt.await_task(context, TOP, handle))
        });
        assert_eq!(got, ((Some(42), None), TASK_DONE));
    }

    #[test]
    fn channels_a_sender_that_is_closed_or_whose_receiver_is_closed_cannot_send() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 2);
            let other = rt.sender_clone(context, tx).expect("room");
            rt.sender_close(other);
            let mut message = 1i64;
            let closed_sender = rt.channel_send(context, TOP, other, &mut message as *mut i64 as usize, true).unwrap_err();
            let clone = rt.sender_clone(context, other).unwrap_err();
            rt.receiver_close(rx);
            let closed_receiver = rt.channel_send(context, TOP, tx, &mut message as *mut i64 as usize, true).unwrap_err();
            (closed_sender, clone, closed_receiver)
        });
        assert_eq!(got, (Failure::Raised("send on a closed Sender"), Failure::Raised("clone of a closed Sender"), Failure::Raised("send on a closed Receiver")));
    }

    #[test]
    fn channels_closing_the_receiver_drops_the_messages_and_the_channel_is_closed() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (channel, tx, rx) = open(rt, context, &ty, 2);
            send(rt, context, tx, 1);
            let other = rt.receiver_clone(context, rx).expect("room");
            rt.receiver_close(other);
            (channel.word(LENGTH), rt.receiver_closed(rx), try_recv(rt, context, rx))
        });
        assert_eq!(got, (0, true, None));
    }

    #[test]
    fn channels_a_sender_given_to_a_task_closes_when_that_task_ends() {
        extern "C" fn quiet(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let (channel, tx, rx) = open(rt, context, &ty, 2);
            let other = rt.sender_clone(context, tx).expect("room");
            let closure = rt.alloc(context, ty.closure, 0).expect("room");
            closure.set_word(CLOSURE_CAPTURES_OFFSET, other.address() as u64);
            let handle = rt.spawn(context, task, quiet, closure.address(), None).expect("room");
            rt.sender_close(tx);
            let got = recv(rt, context, rx);
            (got, rt.await_task(context, TOP, handle), channel.word(SENDERS), other.word(OPEN))
        });
        assert_eq!(got, (None, TASK_DONE, 0, 0));
    }

    #[test]
    fn channels_a_sender_sent_in_a_message_belongs_to_the_task_that_takes_it() {
        static REQUESTS: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn taker(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let rx = Object::from_address(REQUESTS.load(Ordering::SeqCst)).unwrap();
            let mut message = [0usize; 2];
            // SAFETY: as above.
            while rt.channel_recv(unsafe { &mut *context }, TOP, rx, message.as_mut_ptr() as usize, true) == 2 {}
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let (_, reply, replies) = open(rt, context, &ty, 1);
            let requests = rt.channel_new(context, ty.pair_channel, 1).expect("room");
            let asking = rt.channel_end(context, requests, ty.sender).expect("room");
            let taking = rt.channel_end(context, requests, ty.receiver).expect("room");
            REQUESTS.store(taking.address(), Ordering::SeqCst);
            let mut message = [1usize, reply.address()];
            rt.channel_send(context, TOP, asking, message.as_mut_ptr() as usize, true).expect("open");
            let owned_in_flight = rt.channels().owners.get(&reply.address()).copied();
            let handle = rt.spawn(context, task, taker, 0, None).expect("room");
            let state = rt.await_task(context, TOP, handle);
            (owned_in_flight, state, reply.word(OPEN), recv(rt, context, replies))
        });
        assert_eq!(got, (None, TASK_DONE, 0, None));
    }

    #[test]
    fn channels_a_task_cancelled_while_it_waits_leaves_the_queue() {
        static WAITING: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn waiter(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let rx = Object::from_address(WAITING.load(Ordering::SeqCst)).unwrap();
            let mut message = 0i64;
            // SAFETY: as above.
            while rt.channel_recv(unsafe { &mut *context }, TOP, rx, &mut message as *mut i64 as usize, true) == 2 {}
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let (channel, tx, rx) = open(rt, context, &ty, 1);
            WAITING.store(rx.address(), Ordering::SeqCst);
            let handle = rt.spawn(context, task, waiter, 0, None).expect("room");
            while rt.channels().waits[channel.word(NUMBER) as usize].receivers.is_empty() {
                scheduler::yield_now();
            }
            rt.cancel_task(handle);
            let state = rt.await_task(context, TOP, handle);
            send(rt, context, tx, 5);
            (state, recv(rt, context, rx), rt.channels().waits[channel.word(NUMBER) as usize].receivers.len())
        });
        assert_eq!(got, (TASK_CANCELLED, Some(5), 0));
    }

    #[test]
    fn channels_waiting_receivers_take_the_messages_oldest_first() {
        static QUEUE: AtomicUsize = AtomicUsize::new(0);
        static FIRST: AtomicUsize = AtomicUsize::new(0);
        static SECOND: AtomicUsize = AtomicUsize::new(0);
        fn take(context: *mut TaskContext, into: &AtomicUsize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let rx = Object::from_address(QUEUE.load(Ordering::SeqCst)).unwrap();
            // SAFETY: as above.
            into.store(recv(rt, unsafe { &mut *context }, rx).unwrap() as usize, Ordering::SeqCst);
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
            let (channel, tx, rx) = open(rt, context, &ty, 2);
            QUEUE.store(rx.address(), Ordering::SeqCst);
            let a = rt.spawn(context, task, first, 0, None).expect("room");
            while rt.channels().waits[channel.word(NUMBER) as usize].receivers.is_empty() {
                scheduler::yield_now();
            }
            let b = rt.spawn(context, task, second, 0, None).expect("room");
            while rt.channels().waits[channel.word(NUMBER) as usize].receivers.len() < 2 {
                scheduler::yield_now();
            }
            send(rt, context, tx, 1);
            send(rt, context, tx, 2);
            rt.await_task(context, TOP, a);
            rt.await_task(context, TOP, b);
        });
        assert_eq!((FIRST.load(Ordering::SeqCst), SECOND.load(Ordering::SeqCst)), (1, 2));
    }

    #[test]
    fn channels_the_ends_a_task_owns_are_roots_until_it_ends() {
        extern "C" fn quiet(_: *mut TaskContext, _: usize, _: usize) {}
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let (before, after) = run(&rt, move |rt, context| {
            let (_, tx, _) = open(rt, context, &ty, 1);
            let other = rt.sender_clone(context, tx).expect("room");
            let closure = rt.alloc(context, ty.closure, 0).expect("room");
            closure.set_word(CLOSURE_CAPTURES_OFFSET, other.address() as u64);
            let handle = rt.spawn(context, task, quiet, closure.address(), None).expect("room");
            let before = rt.channels().roots().contains(&other.address());
            rt.await_task(context, TOP, handle);
            let roots = rt.channels().roots();
            (before, (roots.contains(&tx.address()), roots.contains(&other.address())))
        });
        assert_eq!((before, after), (true, (true, false)));
    }

    fn recv_for(rt: &Runtime, context: &mut TaskContext, rx: Object, ms: i64) -> Option<i64> {
        let mut message = 0i64;
        (rt.channel_recv_timeout(context, TOP, rx, &mut message as *mut i64 as usize, ms) == 1).then_some(message)
    }

    #[test]
    fn channels_a_timed_receive_gives_up_at_its_time_and_takes_a_message_that_comes_before_it() {
        static SENDER: AtomicUsize = AtomicUsize::new(0);
        extern "C" fn late(context: *mut TaskContext, _: usize, _: usize) {
            // SAFETY: the context is the calling task's own and attached.
            let rt = unsafe { Runtime::of(context) };
            let tx = Object::from_address(SENDER.load(Ordering::SeqCst)).unwrap();
            scheduler::sleep(300 * crate::channels::NANOS);
            // SAFETY: as above.
            send(rt, unsafe { &mut *context }, tx, 9);
        }
        let ty = types();
        let rt = runtime(&ty);
        let task = ty.task;
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 1);
            SENDER.store(tx.address(), Ordering::SeqCst);
            let start = scheduler::now();
            let none = recv_for(rt, context, rx, 100);
            let at = (scheduler::now() - start) / NANOS;
            let handle = rt.spawn(context, task, late, 0, None).expect("room");
            let some = recv_for(rt, context, rx, 1000);
            rt.await_task(context, TOP, handle);
            (none, at, some, (scheduler::now() - start) / NANOS)
        });
        assert_eq!(got, (None, 100, Some(9), 400));
    }

    #[test]
    fn channels_a_timed_receive_with_no_time_takes_only_what_is_there_and_a_closed_channel_ends_it() {
        let ty = types();
        let rt = runtime(&ty);
        let got = run(&rt, move |rt, context| {
            let (_, tx, rx) = open(rt, context, &ty, 2);
            let empty = recv_for(rt, context, rx, 0);
            send(rt, context, tx, 4);
            let ready = recv_for(rt, context, rx, 0);
            rt.sender_close(tx);
            let start = scheduler::now();
            let closed = recv_for(rt, context, rx, 5000);
            (empty, ready, closed, scheduler::now() - start)
        });
        assert_eq!(got, (None, Some(4), None, 0));
    }

    #[test]
    fn channels_the_waits_of_a_channel_that_died_are_freed_and_their_place_is_used_again() {
        let ty = types();
        let rt = Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig { torture: true, ..HeapConfig::default() });
        let got = run(&rt, move |rt, context| {
            let (first, tx, _) = open(rt, context, &ty, 1);
            let number = first.word(NUMBER);
            rt.sender_close(tx);
            rt.poll(context, TOP);
            let (second, ..) = open(rt, context, &ty, 1);
            let channels = rt.channels();
            (second.word(NUMBER) == number, channels.waits.len(), channels.free.len())
        });
        assert_eq!(got, (true, 1, 0));
    }

    #[test]
    fn channels_a_channel_whose_sender_is_open_stays_alive_with_its_waits() {
        let ty = types();
        let rt = Runtime::new(ty.table.clone(), FaultTable::new(), HeapConfig { torture: true, ..HeapConfig::default() });
        let got = run(&rt, move |rt, context| {
            let (first, ..) = open(rt, context, &ty, 1);
            rt.poll(context, TOP);
            let (second, ..) = open(rt, context, &ty, 1);
            (first.word(NUMBER) != second.word(NUMBER), rt.channels().waits.len())
        });
        assert_eq!(got, (true, 2));
    }

    impl Runtime {
        /// How many tasks wait to take a message from the channel the receiver belongs to.
        pub(crate) fn receivers_waiting(&self, rx: Object) -> usize {
            self.channels().waits[channel_of(rx).word(NUMBER) as usize].receivers.len()
        }
    }
}
