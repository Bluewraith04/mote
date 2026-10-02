//! `Shared<T>` cells: readers take the sealed version and never wait; writers take turns.
//! A cell's id picks its `cell_locks` stripe, keys `writer_waiters` (first in first out) and `watchers` (tasks in `wait_until`).

use std::collections::VecDeque;
use std::ptr::NonNull;
use std::sync::Mutex;

use isa::intrinsics::{SEAL_CLAIM, SEAL_COPY, SEAL_IN_PLACE, SHARED_SLOT_HOLDER, SHARED_SLOT_ID, SHARED_SLOT_VERSION};
use isa::seal::{self, CopyKind};
use isa::value::{ObjectHeader, Value, FUNCTION_TYPE_ID, GENERATOR_TYPE_ID, SHARED_TYPE_ID};

use crate::intrinsic::alloc_intrinsic_object;
use crate::{Runtime, TaskContext, VmStatus};

fn cell(v: Value) -> Result<NonNull<ObjectHeader>, String> {
    v.as_object_ptr().ok_or_else(|| "the receiver is not a `Shared`".to_string())
}

fn cell_id(h: NonNull<ObjectHeader>) -> u64 {
    // SAFETY: `SHARED_SLOT_ID` is written once, in `shared_new`.
    unsafe { h.as_ref().get_field(SHARED_SLOT_ID) }.as_uint().unwrap_or(0)
}

fn cell_lock(rt: &Runtime, id: u64) -> &Mutex<()> {
    rt.cell_locks.get(id)
}

fn seal_value(rt: &Runtime, v: Value, mode: i64) -> Result<Value, String> {
    let Some(ptr) = v.as_object_ptr() else { return Ok(v) };
    let sealed = match mode {
        SEAL_IN_PLACE => {
            if seal::reaches_type(ptr, FUNCTION_TYPE_ID) || seal::reaches_type(ptr, GENERATOR_TYPE_ID) {
                return Err("cannot share a function or generator: it is not copyable".to_string());
            }
            seal::seal_deep(ptr);
            ptr
        }
        SEAL_CLAIM => rt.with_mutator(|m| seal::claim(ptr, &mut |t, s| m.alloc(t, s)))?,
        _ => rt.with_mutator(|m| seal::copy_deep(ptr, &mut |t, s| m.alloc(t, s)))?,
    };
    Ok(Value::boxed(sealed))
}

pub(super) fn shared_new(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let mode = task.registers[args_start + 1].as_int().unwrap_or(SEAL_COPY);
    let version = seal_value(rt, task.registers[args_start], mode)?;
    let id = {
        let mut st = rt.sched.lock().unwrap();
        st.next_cell_id += 1;
        st.next_cell_id
    };
    let desc = rt.intrinsic_types.descriptor(SHARED_TYPE_ID).expect("SHARED_TYPE_ID is registered");
    let mut obj = rt.with_mutator(|m| alloc_intrinsic_object(m, desc, 3));
    // SAFETY: freshly allocated with 3 slots; no collection runs before this intrinsic returns.
    unsafe {
        let h = obj.as_mut();
        h.set_field(SHARED_SLOT_ID, Value::uint(id));
        h.set_field(SHARED_SLOT_VERSION, version);
        h.set_field(SHARED_SLOT_HOLDER, Value::null());
    }
    task.registers[dest] = Value::boxed(obj);
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(super) fn shared_get(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let h = cell(task.registers[args_start])?;
    let lock = cell_lock(rt, cell_id(h));
    let version = {
        let _guard = lock.lock().unwrap();
        // SAFETY: a live cell's slots are initialised by `shared_new`.
        unsafe { h.as_ref().get_field(SHARED_SLOT_VERSION) }
    };
    task.registers[dest] = version;
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn same_version(a: Value, b: Value) -> bool {
    match (a.as_float(), b.as_float()) {
        (Some(x), Some(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

pub(super) fn shared_wait(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    if task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("task was cancelled".to_string());
    }
    let h = cell(task.registers[args_start])?;
    let seen = task.registers[args_start + 1];
    let first = task.registers[args_start + 2].as_int().unwrap_or(0) != 0;
    let id = cell_id(h);
    let lock = cell_lock(rt, id);
    let version = {
        let _guard = lock.lock().unwrap();
        // SAFETY: as `shared_get`.
        let version = unsafe { h.as_ref().get_field(SHARED_SLOT_VERSION) };
        if !first && same_version(version, seen) {
            rt.sched.lock().unwrap().watchers.entry(id).or_default().push(task.task_id);
            return Ok(VmStatus::Parked);
        }
        version
    };
    task.registers[dest] = version;
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(super) fn shared_begin(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    if task.shared.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("task was cancelled".to_string());
    }
    let mut h = cell(task.registers[args_start])?;
    let want_copy = task.registers[args_start + 1].as_int().unwrap_or(0) != 0;
    let id = cell_id(h);
    let lock = cell_lock(rt, id);
    let version = {
        let _guard = lock.lock().unwrap();
        // SAFETY: as `shared_get`.
        match unsafe { h.as_ref().get_field(SHARED_SLOT_HOLDER) }.as_uint() {
            Some(holder) if holder == task.task_id => return Err("a `Shared` was written inside its own `update`".to_string()),
            Some(_) => {
                rt.sched.lock().unwrap().writer_waiters.entry(id).or_default().push_back(task.task_id);
                return Ok(VmStatus::Parked);
            }
            // SAFETY: as above, and the cell's stripe lock is held.
            None => unsafe {
                h.as_mut().set_field(SHARED_SLOT_HOLDER, Value::uint(task.task_id));
                h.as_ref().get_field(SHARED_SLOT_VERSION)
            },
        }
    };
    task.held_turns.push(h);
    let working = match version.as_object_ptr().filter(|_| want_copy) {
        Some(ptr) => Value::boxed(rt.with_mutator(|m| seal::copy_deep_as(ptr, &mut |t, s| m.alloc(t, s), CopyKind::Working))?),
        None if want_copy => version,
        None => Value::null(),
    };
    task.registers[dest] = working;
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(super) fn shared_commit(rt: &Runtime, task: &mut TaskContext, dest: usize, args_start: usize) -> Result<VmStatus, String> {
    let h = cell(task.registers[args_start])?;
    let mode = task.registers[args_start + 2].as_int().unwrap_or(SEAL_CLAIM);
    let version = seal_value(rt, task.registers[args_start + 1], mode)?;
    if !release(rt, h, task.task_id, Some(version)) {
        return Err("a `Shared` was committed by a task without its turn".to_string());
    }
    if let Some(i) = task.held_turns.iter().position(|&x| x == h) {
        task.held_turns.swap_remove(i);
    }
    task.registers[dest] = Value::null();
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(crate) fn release(rt: &Runtime, mut h: NonNull<ObjectHeader>, owner: u64, version: Option<Value>) -> bool {
    let id = cell_id(h);
    let lock = cell_lock(rt, id);
    let woken = {
        let _guard = lock.lock().unwrap();
        // SAFETY: as `shared_get`.
        if unsafe { h.as_ref().get_field(SHARED_SLOT_HOLDER) }.as_uint() != Some(owner) {
            return false;
        }
        unsafe {
            if let Some(v) = version {
                h.as_mut().set_field(SHARED_SLOT_VERSION, v);
            }
            h.as_mut().set_field(SHARED_SLOT_HOLDER, Value::null());
        }
        let mut st = rt.sched.lock().unwrap();
        let mut woken: Vec<u64> = st.writer_waiters.get_mut(&id).and_then(VecDeque::pop_front).into_iter().collect();
        if version.is_some() {
            woken.extend(st.watchers.remove(&id).unwrap_or_default());
        }
        woken
    };
    for id in woken {
        rt.unblock_or_note_early(id);
    }
    true
}

pub(crate) fn release_held_turns(rt: &Runtime, task: &mut TaskContext) {
    for h in std::mem::take(&mut task.held_turns) {
        release(rt, h, task.task_id, None);
    }
}

