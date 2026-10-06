//! Channels: a channel object is the `Receiver<T>`, and each `Sender<T>` is a handle counting as one of its senders.
//! The channel closes when its last sender is closed or its owner task ends. `send` and `recv` check `task.cancelled` first.

use std::collections::VecDeque;
use std::ptr::NonNull;
use std::sync::Mutex;

use isa::intrinsics::*;
use isa::value::{
    backing_slot_count, ObjectHeader, Value, BACKING_CAP_SLOT, BACKING_DATA_BASE, BACKING_TYPE_ID, CHANNEL_TYPE_ID,
    SENDER_TYPE_ID,
};

use crate::intrinsic::alloc_intrinsic_object;
use crate::{Runtime, TaskContext, VmStatus};

fn alloc_channel_backing(rt: &Runtime, capacity: usize) -> Value {
    let desc = rt
        .intrinsic_types
        .descriptor(BACKING_TYPE_ID)
        .expect("BACKING_TYPE_ID is registered in IntrinsicTypeTable::default");
    let mut obj = rt.with_mutator(|m| alloc_intrinsic_object(m, desc, backing_slot_count(capacity)));
    // SAFETY: freshly allocated with `backing_slot_count(capacity)` slots.
    unsafe { obj.as_mut().set_field(BACKING_CAP_SLOT, Value::uint(capacity as u64)) };
    Value::boxed(obj)
}

pub(crate) fn alloc_channel(rt: &Runtime, capacity: usize) -> NonNull<ObjectHeader> {
    let channel_id = {
        let mut st = rt.sched.lock().unwrap();
        st.next_channel_id += 1;
        st.next_channel_id
    };
    let physical = capacity.max(1);
    let backing = alloc_channel_backing(rt, physical);

    let desc = rt
        .intrinsic_types
        .descriptor(CHANNEL_TYPE_ID)
        .expect("CHANNEL_TYPE_ID is registered in IntrinsicTypeTable::default");
    let mut obj = rt.with_mutator(|m| alloc_intrinsic_object(m, desc, 9));
    // SAFETY: freshly allocated with 9 slots; no collection can run before the calling native
    // returns (the intrinsic module's GC-safety invariant).
    unsafe {
        let h = obj.as_mut();
        h.set_field(CHAN_SLOT_ID, Value::uint(channel_id));
        h.set_field(CHAN_SLOT_CAPACITY, Value::uint(physical as u64));
        h.set_field(CHAN_SLOT_HEAD, Value::uint(0));
        h.set_field(CHAN_SLOT_LEN, Value::uint(0));
        h.set_field(CHAN_SLOT_CLOSED, Value::small_int(0));
        h.set_field(CHAN_SLOT_SENDERS, Value::small_int(0));
        h.set_field(CHAN_SLOT_TAKEN, Value::uint(0));
        h.set_field(CHAN_SLOT_BACKING, backing);
        h.set_field(CHAN_SLOT_RENDEZVOUS, Value::small_int(i64::from(capacity == 0)));
    }
    obj
}

fn channel_lock(rt: &Runtime, channel_id: u64) -> &Mutex<()> {
    rt.channel_locks.get(channel_id)
}

pub(crate) fn channel_new(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let capacity = task.registers[args_start]
        .as_int()
        .ok_or_else(|| "Channel(capacity): capacity is not an Int".to_string())?;
    if capacity < 0 {
        return Err(format!("Channel(capacity): capacity can't be negative, got {capacity}"));
    }
    let obj = alloc_channel(rt, capacity as usize);
    task.registers[dest] = Value::boxed(obj);
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn open_sender(rt: &Runtime, task: &mut TaskContext, mut channel: NonNull<ObjectHeader>) -> NonNull<ObjectHeader> {
    let desc = rt
        .intrinsic_types
        .descriptor(SENDER_TYPE_ID)
        .expect("SENDER_TYPE_ID is registered in IntrinsicTypeTable::default");
    let mut sender = rt.with_mutator(|m| alloc_intrinsic_object(m, desc, 2));
    // SAFETY: freshly allocated with 2 slots; the channel is live (the caller holds it in a register).
    unsafe {
        sender.as_mut().set_field(SENDER_SLOT_CHANNEL, Value::boxed(channel));
        sender.as_mut().set_field(SENDER_SLOT_CLOSED, Value::small_int(0));
        let channel_id = channel.as_ref().get_field(CHAN_SLOT_ID).as_uint().unwrap_or(0);
        let lock = channel_lock(rt, channel_id);
        let _guard = lock.lock().unwrap();
        let senders = channel.as_ref().get_field(CHAN_SLOT_SENDERS).as_int().unwrap_or(0);
        channel.as_mut().set_field(CHAN_SLOT_SENDERS, Value::small_int(senders + 1));
    }
    task.owned_senders.push(sender);
    sender
}

fn sender_parts(value: Value, what: &str) -> Result<(NonNull<ObjectHeader>, NonNull<ObjectHeader>, bool), String> {
    let sender = value.as_object_ptr().ok_or_else(|| format!("{what}: not a Sender"))?;
    // SAFETY: a `Sender` handle's slots are initialised by `open_sender`.
    let (channel, closed) = unsafe {
        let s = sender.as_ref();
        (s.get_field(SENDER_SLOT_CHANNEL).as_object_ptr(), s.get_field(SENDER_SLOT_CLOSED).as_int().unwrap_or(0) != 0)
    };
    let channel = channel.ok_or_else(|| format!("{what}: the Sender has no channel"))?;
    Ok((sender, channel, closed))
}

pub(crate) fn channel_sender(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let channel = task.registers[args_start]
        .as_object_ptr()
        .ok_or_else(|| "Channel: not a channel".to_string())?;
    let sender = open_sender(rt, task, channel);
    task.registers[dest] = Value::boxed(sender);
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(crate) fn sender_clone(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let (_, channel, closed) = sender_parts(task.registers[args_start], "clone")?;
    if closed {
        return Err("clone of a closed Sender".to_string());
    }
    let sender = open_sender(rt, task, channel);
    task.registers[dest] = Value::boxed(sender);
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(crate) fn channel_send(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    if task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        task.handoff = None;
        pass_wake_on(rt, sender_parts(task.registers[args_start], "send").ok().map(|(_, channel, _)| channel), false);
        return Err("task was cancelled".to_string());
    }
    let (_, mut channel, sender_closed) = sender_parts(task.registers[args_start], "send")?;
    let value = task.registers[args_start + 1];

    // SAFETY: `CHAN_SLOT_ID` is written once, in `alloc_channel`, and never mutated afterward — reading it needs no lock.
    let channel_id = unsafe { channel.as_ref().get_field(CHAN_SLOT_ID) }.as_uint().unwrap_or(0);

    let lock = channel_lock(rt, channel_id);
    let receiver_to_wake;
    let mut park = false;
    {
        let _guard = lock.lock().unwrap();

        // SAFETY: a live channel's slots are all initialised by `alloc_channel`.
        let (capacity, closed, head, len, taken, rendezvous) = unsafe {
            let h = channel.as_ref();
            (
                h.get_field(CHAN_SLOT_CAPACITY).as_uint().unwrap_or(0) as usize,
                h.get_field(CHAN_SLOT_CLOSED).as_int().unwrap_or(0) != 0,
                h.get_field(CHAN_SLOT_HEAD).as_uint().unwrap_or(0) as usize,
                h.get_field(CHAN_SLOT_LEN).as_uint().unwrap_or(0) as usize,
                h.get_field(CHAN_SLOT_TAKEN).as_uint().unwrap_or(0),
                h.get_field(CHAN_SLOT_RENDEZVOUS).as_int().unwrap_or(0) != 0,
            )
        };

        if let Some((handoff_channel, seq)) = task.handoff.filter(|(c, _)| *c == channel_id) {
            if taken >= seq || closed {
                task.handoff = None;
                receiver_to_wake = None;
            } else {
                rt.sched.lock().unwrap().handoff_waiters.entry(handoff_channel).or_default().push_back(task.task_id);
                return Ok(VmStatus::Parked);
            }
        } else {
            if sender_closed {
                return Err("send on a closed Sender".to_string());
            }
            if closed {
                return Err("send on a closed Channel".to_string());
            }
            if len == capacity {
                rt.sched.lock().unwrap().send_waiters.entry(channel_id).or_default().push_back(task.task_id);
                return Ok(VmStatus::Parked);
            }

            // SAFETY: `channel` is a live channel object.
            let backing_val = unsafe { channel.as_ref().get_field(CHAN_SLOT_BACKING) };
            let mut backing = backing_val
                .as_object_ptr()
                .ok_or_else(|| "send: channel backing is corrupt".to_string())?;
            let slot = BACKING_DATA_BASE + (head + len) % capacity;
            // SAFETY: `slot` is within `1..=capacity` by construction (`len < capacity` here).
            unsafe { backing.as_mut().set_field(slot, value) };
            unsafe { channel.as_mut().set_field(CHAN_SLOT_LEN, Value::uint((len + 1) as u64)) };

            let mut st = rt.sched.lock().unwrap();
            receiver_to_wake = st.recv_waiters.get_mut(&channel_id).and_then(VecDeque::pop_front);
            if rendezvous {
                task.handoff = Some((channel_id, taken + (len + 1) as u64));
                st.handoff_waiters.entry(channel_id).or_default().push_back(task.task_id);
                park = true;
            }
        }
    }
    if let Some(receiver) = receiver_to_wake {
        rt.unblock_or_note_early(receiver);
    }
    if park {
        return Ok(VmStatus::Parked);
    }

    task.registers[dest] = Value::null();
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// A cancelled task leaving `send` or `recv` hands on the wake it may have been given, so the next waiter takes the free slot or the item.
fn pass_wake_on(rt: &Runtime, channel: Option<NonNull<ObjectHeader>>, receiving: bool) {
    let Some(channel) = channel else { return };
    // SAFETY: `CHAN_SLOT_ID` is written once, in `alloc_channel`.
    let channel_id = unsafe { channel.as_ref().get_field(CHAN_SLOT_ID) }.as_uint().unwrap_or(0);
    let next = {
        let _guard = channel_lock(rt, channel_id).lock().unwrap();
        // SAFETY: a live channel's slots are initialised by `alloc_channel`, and its lock is held.
        let (capacity, len) = unsafe {
            let h = channel.as_ref();
            (h.get_field(CHAN_SLOT_CAPACITY).as_uint().unwrap_or(0), h.get_field(CHAN_SLOT_LEN).as_uint().unwrap_or(0))
        };
        let mut st = rt.sched.lock().unwrap();
        let (ready, waiters) = if receiving { (len > 0, &mut st.recv_waiters) } else { (len < capacity, &mut st.send_waiters) };
        if ready { waiters.get_mut(&channel_id).and_then(VecDeque::pop_front) } else { None }
    };
    if let Some(id) = next {
        rt.unblock_or_note_early(id);
    }
}

pub(crate) fn channel_recv(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    if task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        pass_wake_on(rt, task.registers[args_start].as_object_ptr(), true);
        return Err("task was cancelled".to_string());
    }
    let handle_val = task.registers[args_start];
    let mut handle = handle_val
        .as_object_ptr()
        .ok_or_else(|| "recv: receiver is not a Channel".to_string())?;

    // SAFETY: as `channel_send`.
    let channel_id = unsafe { handle.as_ref().get_field(CHAN_SLOT_ID) }.as_uint().unwrap_or(0);

    let lock = channel_lock(rt, channel_id);
    let recv_result;
    let source = rt.source(channel_id);
    let mut source_over = false;
    let to_wake: Vec<u64> = {
        let _guard = lock.lock().unwrap();

        if let Some(src) = &source {
            crate::sources::pump(rt, src, &mut handle)?;
        }

        // SAFETY: as `channel_send`.
        let (capacity, mut closed, head, len, taken, rendezvous) = unsafe {
            let h = handle.as_ref();
            (
                h.get_field(CHAN_SLOT_CAPACITY).as_uint().unwrap_or(0) as usize,
                h.get_field(CHAN_SLOT_CLOSED).as_int().unwrap_or(0) != 0,
                h.get_field(CHAN_SLOT_HEAD).as_uint().unwrap_or(0) as usize,
                h.get_field(CHAN_SLOT_LEN).as_uint().unwrap_or(0) as usize,
                h.get_field(CHAN_SLOT_TAKEN).as_uint().unwrap_or(0),
                h.get_field(CHAN_SLOT_RENDEZVOUS).as_int().unwrap_or(0) != 0,
            )
        };

        if len == 0 && !closed && source.as_ref().is_some_and(|s| s.queue.is_finished()) {
            // SAFETY: `handle` is a live channel object.
            unsafe { handle.as_mut().set_field(CHAN_SLOT_CLOSED, Value::small_int(1)) };
            closed = true;
            source_over = true;
        }

        if len == 0 && !closed {
            let mut st = rt.sched.lock().unwrap();
            if source.as_ref().is_some_and(|s| s.has_news()) {
                return Ok(VmStatus::Running);
            }
            st.recv_waiters.entry(channel_id).or_default().push_back(task.task_id);
            return Ok(VmStatus::Parked);
        }

        let (to_wake, status, value) = if len == 0 {
            (Vec::new(), RECV_EMPTY, Value::null())
        } else {
            // SAFETY: `handle` is a live channel object.
            let backing_val = unsafe { handle.as_ref().get_field(CHAN_SLOT_BACKING) };
            let backing = backing_val
                .as_object_ptr()
                .ok_or_else(|| "recv: channel backing is corrupt".to_string())?;
            let slot = BACKING_DATA_BASE + head;
            // SAFETY: `slot` is within `1..=capacity` by construction (`len > 0` here).
            let v = unsafe { backing.as_ref().get_field(slot) };
            let new_head = (head + 1) % capacity;
            unsafe {
                handle.as_mut().set_field(CHAN_SLOT_HEAD, Value::uint(new_head as u64));
                handle.as_mut().set_field(CHAN_SLOT_LEN, Value::uint((len - 1) as u64));
                handle.as_mut().set_field(CHAN_SLOT_TAKEN, Value::uint(taken + 1));
            }
            let mut st = rt.sched.lock().unwrap();
            let mut to_wake: Vec<u64> = st.send_waiters.get_mut(&channel_id).and_then(VecDeque::pop_front).into_iter().collect();
            if rendezvous {
                to_wake.extend(st.handoff_waiters.remove(&channel_id).into_iter().flatten());
            }
            (to_wake, RECV_GOT_VALUE, v)
        };

        recv_result = (status, value);
        to_wake
    };
    if source_over {
        rt.close_source(channel_id);
    }
    for id in to_wake {
        rt.unblock_or_note_early(id);
    }

    task.registers[dest] = recv_result.1;
    task.registers[args_start + 1] = Value::small_int(recv_result.0);
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(crate) fn close_sender(rt: &Runtime, mut sender: NonNull<ObjectHeader>) {
    // SAFETY: a `Sender` handle's slots are initialised by `open_sender`.
    let mut channel = unsafe {
        if sender.as_ref().get_field(SENDER_SLOT_CLOSED).as_int().unwrap_or(0) != 0 {
            return;
        }
        sender.as_mut().set_field(SENDER_SLOT_CLOSED, Value::small_int(1));
        match sender.as_ref().get_field(SENDER_SLOT_CHANNEL).as_object_ptr() {
            Some(c) => c,
            None => return,
        }
    };
    // SAFETY: `channel` is a live channel object.
    let channel_id = unsafe { channel.as_ref().get_field(CHAN_SLOT_ID) }.as_uint().unwrap_or(0);
    let lock = channel_lock(rt, channel_id);
    let waiters: Vec<u64> = {
        let _guard = lock.lock().unwrap();
        // SAFETY: as above, and the channel's lock is held.
        let senders = unsafe { channel.as_ref().get_field(CHAN_SLOT_SENDERS) }.as_int().unwrap_or(1) - 1;
        // SAFETY: as above, and the channel's lock is held.
        unsafe { channel.as_mut().set_field(CHAN_SLOT_SENDERS, Value::small_int(senders.max(0))) };
        if senders > 0 {
            return;
        }
        // SAFETY: as above, and the channel's lock is held.
        unsafe { channel.as_mut().set_field(CHAN_SLOT_CLOSED, Value::small_int(1)) };

        let mut st = rt.sched.lock().unwrap();
        let mut waiters: Vec<u64> = st.send_waiters.remove(&channel_id).into_iter().flatten().collect();
        waiters.extend(st.recv_waiters.remove(&channel_id).into_iter().flatten());
        waiters.extend(st.handoff_waiters.remove(&channel_id).into_iter().flatten());
        waiters
    };
    for id in waiters {
        rt.unblock_or_note_early(id);
    }
}

pub(crate) fn channel_close(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let (sender, _, _) = sender_parts(task.registers[args_start], "close")?;
    close_sender(rt, sender);
    if let Some(i) = task.owned_senders.iter().position(|&s| s == sender) {
        task.owned_senders.swap_remove(i);
    }
    task.registers[dest] = Value::null();
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(crate) fn close_owned_senders(rt: &Runtime, task: &mut TaskContext) {
    task.handoff = None;
    for sender in std::mem::take(&mut task.owned_senders) {
        close_sender(rt, sender);
    }
}

pub(crate) fn move_captured_senders(parent: &mut TaskContext, child: &mut TaskContext, closure: NonNull<ObjectHeader>) {
    for sender in isa::seal::collect_of_type(closure, SENDER_TYPE_ID) {
        if let Some(i) = parent.owned_senders.iter().position(|&s| s == sender) {
            parent.owned_senders.swap_remove(i);
            child.owned_senders.push(sender);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A channel with `len` buffered items and a task parked on it, as a receiver or a sender.
    fn channel_with_waiter(rt: &Runtime, capacity: usize, len: u64, receiving: bool) -> (NonNull<ObjectHeader>, u64) {
        let mut channel = alloc_channel(rt, capacity);
        // SAFETY: freshly allocated by `alloc_channel`.
        let channel_id = unsafe {
            channel.as_mut().set_field(CHAN_SLOT_LEN, Value::uint(len));
            channel.as_ref().get_field(CHAN_SLOT_ID).as_uint().unwrap()
        };
        let waiter = rt.acquire_task(0, 8, None, 0);
        let id = waiter.task_id;
        let mut st = rt.sched.lock().unwrap();
        let waiters = if receiving { &mut st.recv_waiters } else { &mut st.send_waiters };
        waiters.entry(channel_id).or_default().push_back(id);
        st.blocked.insert(id, waiter);
        (channel, id)
    }

    #[test]
    fn a_cancelled_receiver_passes_a_buffered_item_to_the_next_receiver() {
        let rt = Runtime::new(vec![]);
        let (channel, _) = channel_with_waiter(&rt, 2, 1, true);
        pass_wake_on(&rt, Some(channel), true);
        let st = rt.sched.lock().unwrap();
        assert!(st.blocked.is_empty() && !st.run_queue.is_empty());
    }

    #[test]
    fn a_cancelled_receiver_wakes_no_one_when_nothing_is_buffered() {
        let rt = Runtime::new(vec![]);
        let (channel, id) = channel_with_waiter(&rt, 2, 0, true);
        pass_wake_on(&rt, Some(channel), true);
        assert!(rt.sched.lock().unwrap().blocked.contains_key(&id));
    }

    #[test]
    fn a_cancelled_sender_passes_a_free_slot_to_the_next_sender() {
        let rt = Runtime::new(vec![]);
        let (channel, _) = channel_with_waiter(&rt, 2, 1, false);
        pass_wake_on(&rt, Some(channel), false);
        let st = rt.sched.lock().unwrap();
        assert!(st.blocked.is_empty() && !st.run_queue.is_empty());
    }

    #[test]
    fn a_cancelled_sender_wakes_no_one_when_the_channel_is_full() {
        let rt = Runtime::new(vec![]);
        let (channel, id) = channel_with_waiter(&rt, 2, 2, false);
        pass_wake_on(&rt, Some(channel), false);
        assert!(rt.sched.lock().unwrap().blocked.contains_key(&id));
    }
}
