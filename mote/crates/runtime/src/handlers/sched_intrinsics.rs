//! The scheduler-aware natives: `Task<T>.join()` and `.cancel()`; channels are in [`super::channel`].
//! Routed here from [`super::native`] before the leaf dispatcher, since they need full `&mut Runtime` access to park a task or reach another's state.
//! `join`, `send` and `recv` check `task.cancelled` first.

use std::ptr::NonNull;

use isa::value::{ObjectHeader, Value, BACKING_DATA_BASE, HEADER_BACKING_SLOT, HEADER_LEN_SLOT, TASK_TYPE_ID};

use super::channel;
use crate::intrinsic::alloc_intrinsic_object;
use crate::{Runtime, TaskContext, VmStatus};

pub use isa::intrinsics::*;
pub(crate) use channel::{alloc_channel, close_owned_senders};

/// Dispatches a `CALLINTRINSIC`; the id indexes [`INTRINSIC_NAMES`] and `dest` / `args_start` are frame-relative registers.
pub fn dispatch(
    rt: &Runtime,
    task: &mut TaskContext,
    func_idx: u8,
    dest: usize,
    args_start: usize,
) -> Result<VmStatus, String> {
    match func_idx {
        TASK_JOIN_INTRINSIC => join(rt, task, dest, args_start),
        TASK_CANCEL_INTRINSIC => cancel(rt, task, dest, args_start),
        TASK_ANY_INTRINSIC => task_any(rt, task, dest, args_start),
        CHANNEL_NEW_INTRINSIC => channel::channel_new(rt, task, dest, args_start),
        CHANNEL_SEND_INTRINSIC => channel::channel_send(rt, task, dest, args_start),
        CHANNEL_RECV_INTRINSIC => channel::channel_recv(rt, task, dest, args_start),
        CHANNEL_CLOSE_INTRINSIC => channel::channel_close(rt, task, dest, args_start),
        CHANNEL_SENDER_INTRINSIC => channel::channel_sender(rt, task, dest, args_start),
        SENDER_CLONE_INTRINSIC => channel::sender_clone(rt, task, dest, args_start),
        SHARED_NEW_INTRINSIC => super::shared_cell::shared_new(rt, task, dest, args_start),
        SHARED_GET_INTRINSIC => super::shared_cell::shared_get(rt, task, dest, args_start),
        SHARED_BEGIN_INTRINSIC => super::shared_cell::shared_begin(rt, task, dest, args_start),
        SHARED_COMMIT_INTRINSIC => super::shared_cell::shared_commit(rt, task, dest, args_start),
        SHARED_WAIT_INTRINSIC => super::shared_cell::shared_wait(rt, task, dest, args_start),
        TASK_IS_READY_INTRINSIC => is_ready(rt, task, dest, args_start),
        TASK_PIN_INTRINSIC => pin(task, dest, args_start),
        _ => Err(format!("CALLINTRINSIC: unknown intrinsic {func_idx}")),
    }
}

pub(crate) fn alloc_task_handle(rt: &Runtime, task_id: u64) -> NonNull<ObjectHeader> {
    let desc = rt
        .intrinsic_types
        .descriptor(TASK_TYPE_ID)
        .expect("TASK_TYPE_ID is registered in IntrinsicTypeTable::default");
    let mut obj = rt.with_mutator(|m| alloc_intrinsic_object(m, desc, 5));
    // SAFETY: freshly allocated with 5 slots; no collection can run before
    // `SPAWN`'s handler returns (the intrinsic module's GC-safety invariant).
    unsafe {
        let h = obj.as_mut();
        h.set_field(TASK_SLOT_ID, Value::uint(task_id));
        h.set_field(TASK_SLOT_STATUS, Value::small_int(STATUS_PENDING));
        h.set_field(TASK_SLOT_RESULT, Value::null());
        h.set_field(TASK_SLOT_WAITER, Value::null());
        h.set_field(TASK_SLOT_OBSERVED, Value::small_int(0));
    }
    obj
}

pub(crate) fn complete_handle(
    rt: &Runtime,
    mut handle: NonNull<ObjectHeader>,
    ok_value: Value,
    err: Option<&str>,
) {
    let (status, result) = match err {
        Some(e) => (STATUS_ERR, rt.alloc_heap_string(e)),
        None => (STATUS_OK, ok_value),
    };
    let waiter = {
        let _st = rt.sched.lock().unwrap();
        // SAFETY: `handle` was allocated by `alloc_task_handle` and is kept alive
        // by the GC root in `TaskContext::handle` (scanned by `roots`)
        // for as long as this task's `TaskContext` exists — which it still does;
        // the caller recycles it only after this call returns.
        unsafe {
            handle.as_mut().set_field(TASK_SLOT_STATUS, Value::small_int(status));
            handle.as_mut().set_field(TASK_SLOT_RESULT, result);
            handle.as_ref().get_field(TASK_SLOT_WAITER).as_uint()
        }
    };
    if let Some(waiter) = waiter {
        rt.unblock_or_note_early(waiter);
    }
}

fn join(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    if task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("task was cancelled".to_string());
    }
    let handle_val = task.registers[args_start];
    let mut handle = handle_val
        .as_object_ptr()
        .ok_or_else(|| "join: receiver is not a Task handle".to_string())?;
    {
        let _st = rt.sched.lock().unwrap();
        // SAFETY: a live `Task<T>` handle's slots are all initialised by
        // `alloc_task_handle`.
        let status = unsafe { handle.as_ref().get_field(TASK_SLOT_STATUS) }
            .as_int()
            .unwrap_or(STATUS_PENDING);
        if status == STATUS_PENDING {
            // SAFETY: `handle` is a live `Task<T>` object.
            unsafe { handle.as_mut().set_field(TASK_SLOT_WAITER, Value::uint(task.task_id)) };
            return Ok(VmStatus::Parked);
        }
    }
    // SAFETY: `handle` is a live `Task<T>` object.
    unsafe { handle.as_mut().set_field(TASK_SLOT_OBSERVED, Value::small_int(1)) };
    task.registers[dest] = Value::null();
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn is_ready(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let handle = task.registers[args_start]
        .as_object_ptr()
        .ok_or_else(|| "is_ready: receiver is not a Task handle".to_string())?;
    let status = {
        let _st = rt.sched.lock().unwrap();
        // SAFETY: as `join`.
        unsafe { handle.as_ref().get_field(TASK_SLOT_STATUS) }.as_int().unwrap_or(STATUS_PENDING)
    };
    task.registers[dest] = if status == STATUS_PENDING { Value::false_() } else { Value::true_() };
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `std.task.pin` / `unpin` / `is_pinned`: a change of mode yields so the scheduler moves the task to its new queue.
fn pin(task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let mode = task.registers[args_start].as_int().ok_or_else(|| "task.pin: mode is not an Int".to_string())?;
    let wanted = match mode {
        PIN_OFF => false,
        PIN_ON => true,
        _ => task.pinned,
    };
    let moved = wanted != task.pinned;
    task.pinned = wanted;
    task.registers[dest] = if wanted { Value::true_() } else { Value::false_() };
    task.pc += 1;
    Ok(if moved { VmStatus::Yielded } else { VmStatus::Running })
}

fn cancel(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let handle_val = task.registers[args_start];
    let handle = handle_val
        .as_object_ptr()
        .ok_or_else(|| "cancel: receiver is not a Task handle".to_string())?;
    // SAFETY: as `join`.
    let target_id = unsafe { handle.as_ref().get_field(TASK_SLOT_ID) }
        .as_uint()
        .unwrap_or(0);
    cancel_target(rt, task, target_id);
    task.registers[dest] = Value::null();
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn cancel_target(rt: &Runtime, task: &TaskContext, target_id: u64) {
    if target_id == task.task_id {
        task.shared.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
    } else {
        rt.cancel_task(target_id);
    }
}

fn list_task_handles(list: Value) -> Result<Vec<NonNull<ObjectHeader>>, String> {
    let list_ptr = list.as_object_ptr().ok_or_else(|| "task.any: argument is not a List".to_string())?;
    // SAFETY: a live `List`'s header slots are initialised by `list_new`.
    let (len, backing) = unsafe {
        let h = list_ptr.as_ref();
        (h.get_field(HEADER_LEN_SLOT).as_uint().unwrap_or(0) as usize, h.get_field(HEADER_BACKING_SLOT))
    };
    let backing_ptr = backing
        .as_object_ptr()
        .ok_or_else(|| "task.any: corrupt List backing".to_string())?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        // SAFETY: `i < len <= capacity` by construction.
        let v = unsafe { backing_ptr.as_ref().get_field(BACKING_DATA_BASE + i) };
        let h = v
            .as_object_ptr()
            .ok_or_else(|| "task.any: list element is not a Task handle".to_string())?;
        out.push(h);
    }
    Ok(out)
}

fn task_any(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    if task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("task was cancelled".to_string());
    }
    let handles = list_task_handles(task.registers[args_start])?;
    if handles.is_empty() {
        return Err("task.any: the list is empty".to_string());
    }
    let winner = {
        let _st = rt.sched.lock().unwrap();
        let resolved = handles.iter().find(|h| {
            // SAFETY: a live `Task<T>` handle's slots are all initialised by `alloc_task_handle`.
            let status = unsafe { h.as_ref().get_field(TASK_SLOT_STATUS) }.as_int().unwrap_or(STATUS_PENDING);
            status != STATUS_PENDING
        });
        match resolved {
            Some(&h) => h,
            None => {
                for &h in &handles {
                    let mut h = h;
                    // SAFETY: as above.
                    unsafe { h.as_mut().set_field(TASK_SLOT_WAITER, Value::uint(task.task_id)) };
                }
                return Ok(VmStatus::Parked);
            }
        }
    };
    for mut h in handles {
        if h != winner {
            // SAFETY: `h` is a live `Task<T>` object.
            let target_id = unsafe { h.as_ref().get_field(TASK_SLOT_ID) }.as_uint().unwrap_or(0);
            cancel_target(rt, task, target_id);
            // SAFETY: as above.
            unsafe { h.as_mut().set_field(TASK_SLOT_OBSERVED, Value::small_int(1)) };
        }
    }
    task.registers[dest] = Value::boxed(winner);
    task.pc += 1;
    Ok(VmStatus::Running)
}
