//! Generators: `MKGEN`, `RESUME`, `YIELD`, and the generator half of `RET`.
//! A generator body runs as an ordinary frame on the resuming task; its register window is copied in on
//! `RESUME` and back out on `YIELD`.

use std::ptr::NonNull;

use isa::value::{GENERATOR_TYPE_ID, ObjectHeader, Value};

use crate::intrinsic::alloc_intrinsic_object;
use crate::util::{decode_r2, decode_r3};
use crate::arena::ArenaStack;
use crate::{Frame, GenLink, Runtime, TaskContext, VmStatus};

const GEN_STATE: usize = 1;
const GEN_CODE: usize = 2;
const GEN_PC: usize = 3;
const GEN_CLOSURE: usize = 4;
const GEN_ARENA: usize = 5;
const GEN_WINDOW: usize = 6;

const STATE_SUSPENDED: i64 = 1;
const STATE_RUNNING: i64 = 2;
const STATE_DONE: i64 = 3;

/// `MKGEN` — pack the running frame into a `Generator` and return it to the caller.
pub(crate) fn mkgen(rt: &Runtime, task: &mut TaskContext) -> Result<VmStatus, String> {
    if task.call_stack.len() < 2 {
        return Err("MKGEN in the entry frame: a generator function must be called".to_string());
    }
    let frame = task.call_stack.last().cloned().unwrap();
    let reg_count = rt.code_objects[task.current_code].register_count as usize;
    let desc = rt
        .intrinsic_types
        .descriptor(GENERATOR_TYPE_ID)
        .expect("GENERATOR_TYPE_ID is registered in IntrinsicTypeTable::default");
    let slots = GEN_WINDOW + reg_count;
    let mut obj = rt.with_mutator(|m| alloc_intrinsic_object(m, desc, slots));
    // SAFETY: freshly allocated with `slots` slots; no collection runs before this handler returns.
    unsafe {
        let h = obj.as_mut();
        h.set_field(0, Value::uint((slots - 1) as u64));
        h.set_field(GEN_STATE, Value::small_int(0));
        h.set_field(GEN_CODE, Value::small_int(task.current_code as i64));
        h.set_field(GEN_PC, Value::small_int(task.pc as i64 + 1));
        h.set_field(GEN_CLOSURE, frame.closure.map_or_else(Value::null, Value::boxed));
        h.set_field(GEN_ARENA, Value::null());
        for i in 0..reg_count {
            h.set_field(GEN_WINDOW + i, task.registers[frame.base + i]);
        }
    }
    task.call_stack.pop();
    task.registers[frame.dest_reg] = Value::boxed(obj);
    task.current_code = frame.caller_code;
    task.pc = frame.return_pc;
    Ok(VmStatus::Running)
}

/// `RESUME rA, rB, rC` — run generator `rB` until it yields (`rA` = value, `rC` = 1) or ends (`rA` = null, `rC` = 0).
pub(crate) fn resume(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    rt.safepoint_poll(task)?;
    let (a, b, c) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    let gv = task.registers[base + b as usize];
    let mut obj = generator_object(gv).ok_or_else(|| format!("RESUME target in r{b} is not a generator: {gv:?}"))?;
    // SAFETY: a generator object always has the header slots and its saved window.
    let (state, code_idx, pc, closure, arena_key) = unsafe {
        let h = obj.as_ref();
        (
            h.get_field(GEN_STATE).as_int().unwrap_or(0),
            h.get_field(GEN_CODE).as_int().unwrap_or(0) as usize,
            h.get_field(GEN_PC).as_int().unwrap_or(0) as usize,
            h.get_field(GEN_CLOSURE).as_object_ptr(),
            h.get_field(GEN_ARENA).as_int(),
        )
    };
    match state {
        STATE_DONE => {
            task.registers[base + a as usize] = Value::null();
            task.registers[base + c as usize] = Value::small_int(0);
            task.pc += 1;
            return Ok(VmStatus::Running);
        }
        STATE_RUNNING => return Err("cannot resume a generator that is already running".to_string()),
        _ => {}
    }

    let caller_reg_count = rt.code_objects[task.current_code].register_count as usize;
    let callee_base = base + caller_reg_count;
    let reg_count = rt.code_objects[code_idx].register_count as usize;
    rt.check_stack(callee_base + reg_count, task.region_bytes(), task.call_stack.len())?;
    if task.registers.len() < callee_base + reg_count {
        task.registers.resize(callee_base + reg_count, Value::null());
    }
    // SAFETY: the window has `reg_count` slots starting at `GEN_WINDOW`.
    unsafe {
        for i in 0..reg_count {
            task.registers[callee_base + i] = obj.as_ref().get_field(GEN_WINDOW + i);
        }
        obj.as_mut().set_field(GEN_STATE, Value::small_int(STATE_RUNNING));
    }
    task.call_stack.push(Frame {
        return_pc: task.pc + 1,
        base: callee_base,
        caller_code: task.current_code,
        dest_reg: base + a as usize,
        closure,
        generator: Some(GenLink { object: obj, status_dest: base + c as usize }),
    });
    if let Some(stack) = arena_key.and_then(|key| rt.gen_regions.take(key)) {
        swap_in(task, stack);
    }
    task.current_code = code_idx;
    task.pc = pc;
    Ok(VmStatus::Running)
}

/// Whether the generator frame on top of the stack runs on its own region stack.
pub(crate) fn is_swapped(task: &TaskContext) -> bool {
    task.arena_saves.last().is_some_and(|(depth, _)| *depth == task.call_stack.len())
}

/// Runs the generator frame on top on `stack`, setting the task's own region stack aside until the generator yields or ends.
pub(crate) fn swap_in(task: &mut TaskContext, stack: Box<ArenaStack>) {
    let own = std::mem::replace(&mut task.arenas, *stack);
    task.saved_arena_bytes += own.segment_bytes();
    task.arena_saves.push((task.call_stack.len(), Box::new(own)));
}

fn swap_out(task: &mut TaskContext) -> ArenaStack {
    let (_, own) = task.arena_saves.pop().expect("a swapped generator frame has a saved stack");
    task.saved_arena_bytes -= own.segment_bytes();
    let mut generator_stack = std::mem::replace(&mut task.arenas, *own);
    task.arenas.absorb_stats(&generator_stack.take_stats());
    generator_stack
}

/// `YIELD rA` — save the window and `pc`, then hand `rA` to the `RESUME` that started this run.
pub(crate) fn yield_(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, _) = decode_r2(operands);
    let frame = task.call_stack.last().cloned().unwrap();
    let link = frame.generator.ok_or_else(|| "YIELD outside a generator body".to_string())?;
    let value = task.registers[frame.base + a as usize];
    let reg_count = rt.code_objects[task.current_code].register_count as usize;
    let mut obj = link.object;
    // SAFETY: the window has `reg_count` slots starting at `GEN_WINDOW`.
    unsafe {
        let h = obj.as_mut();
        for i in 0..reg_count {
            let v = task.registers[frame.base + i];
            h.set_field(GEN_WINDOW + i, v);
        }
        h.set_field(GEN_PC, Value::small_int(task.pc as i64 + 1));
        h.set_field(GEN_STATE, Value::small_int(STATE_SUSPENDED));
    }
    if is_swapped(task) {
        keep_regions(rt, task, obj);
    }
    task.call_stack.pop();
    task.registers[frame.dest_reg] = value;
    task.registers[link.status_dest] = Value::small_int(1);
    task.current_code = frame.caller_code;
    task.pc = frame.return_pc;
    Ok(VmStatus::Running)
}

fn keep_regions(rt: &Runtime, task: &mut TaskContext, mut obj: NonNull<ObjectHeader>) {
    let stack = swap_out(task);
    // SAFETY: a generator object has its header slots.
    let old_key = unsafe { obj.as_ref().get_field(GEN_ARENA) }.as_int();
    if stack.is_empty() {
        if let Some(key) = old_key {
            rt.gen_regions.release(key);
            unsafe { obj.as_mut().set_field(GEN_ARENA, Value::null()) };
        }
        return;
    }
    let key = old_key.unwrap_or_else(|| {
        let key = rt.gen_regions.new_key();
        rt.with_mutator(|m| m.release_on_collect(obj, contracts::RELEASE_GENERATOR, key));
        key
    });
    // SAFETY: `obj` is the live generator object just allocated.
    unsafe { obj.as_mut().set_field(GEN_ARENA, Value::small_int(key)) };
    rt.gen_regions.put(key, Box::new(stack));
}

/// `RET` inside a generator body: mark it done, drop its window, and end the `RESUME` with status 0.
pub fn finish(rt: &Runtime, task: &mut TaskContext, link: GenLink) -> Result<VmStatus, String> {
    if is_swapped(task) {
        drop(swap_out(task));
    }
    let frame = task.call_stack.pop().unwrap();
    let reg_count = rt.code_objects[task.current_code].register_count as usize;
    let mut obj = link.object;
    // SAFETY: the window has `reg_count` slots starting at `GEN_WINDOW`.
    unsafe {
        let h = obj.as_mut();
        for i in 0..reg_count {
            h.set_field(GEN_WINDOW + i, Value::null());
        }
        h.set_field(GEN_STATE, Value::small_int(STATE_DONE));
    }
    task.registers[frame.dest_reg] = Value::null();
    task.registers[link.status_dest] = Value::small_int(0);
    task.current_code = frame.caller_code;
    task.pc = frame.return_pc;
    Ok(VmStatus::Running)
}

fn generator_object(v: Value) -> Option<NonNull<ObjectHeader>> {
    let ptr = v.as_object_ptr()?;
    // SAFETY: a boxed value in a register points at a live header with a live descriptor.
    let id = unsafe { ptr.as_ref().type_ptr.as_ref().id };
    (id == GENERATOR_TYPE_ID).then_some(ptr)
}
