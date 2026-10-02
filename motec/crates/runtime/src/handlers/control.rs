//! Constant loads and control flow: `LOADI LOADK JMP JMPIF JMPIFNOT CALL CALLV
//! GETCAPTURE RET NOP`. `HALT` is handled inline in [`super::execute`].

use std::ptr::NonNull;

use isa::opcode::Opcode::{self, *};
use isa::value::{ObjectHeader, Value};

use crate::handlers::sched_intrinsics;
use crate::util::{decode_jc, decode_ju, decode_r2, decode_r3, decode_ri};
use crate::{Frame, Runtime, ScopeFrame, TaskContext, VmStatus};

/// `LOADI` (16-bit immediate) / `LOADK` (constant-pool index).
pub fn load(
    rt: &Runtime,
    task: &mut TaskContext,
    opcode: Opcode,
    operands: u32,
) -> Result<VmStatus, String> {
    let (a, bx) = decode_ri(operands);
    let dest = task.call_stack.last().unwrap().base + (a as usize);
    match opcode {
        LOADI => {
            task.registers[dest] = Value::small_int(bx as i64);
        }
        LOADK => {
            task.registers[dest] = rt.code_objects[task.current_code].constants[bx as usize];
        }
        _ => unreachable!("control::load called with {opcode:?}"),
    }
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// Unconditional jump. A backward jump polls a safepoint first.
pub(crate) fn jmp(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let sbx = decode_ju(operands);
    if sbx < 0 {
        rt.safepoint_poll(task)?;
    }
    task.pc = (task.pc as isize + sbx as isize) as usize;
    Ok(VmStatus::Running)
}

/// Branch when `rA` is truthy; a taken backward branch polls a safepoint.
pub(crate) fn jmpif(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, sbx) = decode_jc(operands);
    let cond = task.call_stack.last().unwrap().base + (a as usize);
    if task.registers[cond].is_truthy() {
        if sbx < 0 {
            rt.safepoint_poll(task)?;
        }
        task.pc = (task.pc as isize + sbx as isize) as usize;
    } else {
        task.pc += 1;
    }
    Ok(VmStatus::Running)
}

/// Branch when `rA` is falsy; a taken backward branch polls a safepoint.
pub(crate) fn jmpifnot(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (a, sbx) = decode_jc(operands);
    let cond = task.call_stack.last().unwrap().base + (a as usize);
    if !task.registers[cond].is_truthy() {
        if sbx < 0 {
            rt.safepoint_poll(task)?;
        }
        task.pc = (task.pc as isize + sbx as isize) as usize;
    } else {
        task.pc += 1;
    }
    Ok(VmStatus::Running)
}

/// Calls a code object by static index: polls a safepoint, lays out the callee's register window after the caller's, copies the arguments and pushes a frame.
pub fn call(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    rt.safepoint_poll(task)?;
    let (dest_reg, callee_idx) = decode_ri(operands);
    enter_call(rt, task, dest_reg, callee_idx as usize, dest_reg as usize + 1, None)
}

/// Indirect call: `rB` holds a function value; calls the code object named by its slot 0 and records it on the frame for `GETCAPTURE`.
pub(crate) fn callv(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    rt.safepoint_poll(task)?;
    let (dest_reg, callee_reg, arg_start) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    let callee_val = task.registers[base + (callee_reg as usize)];
    let callee_code_idx = callee_val.as_function_code_idx().ok_or_else(|| {
        format!("CALLV target in r{callee_reg} is not a callable value: {callee_val:?}")
    })?;
    enter_call(rt, task, dest_reg, callee_code_idx, arg_start as usize, callee_val.as_object_ptr())
}

/// `rA = globals[bx]`; an index past the end reads `null`.
pub(crate) fn getglobal(
    rt: &Runtime,
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (dest_reg, gidx) = decode_ri(operands);
    let base = task.call_stack.last().unwrap().base;
    let val = rt.globals.lock().unwrap().get(gidx as usize).copied().unwrap_or_else(Value::null);
    task.registers[base + (dest_reg as usize)] = val;
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `globals[bx] = rA`, growing the array to fit.
pub(crate) fn setglobal(
    rt: &Runtime,
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (src_reg, gidx) = decode_ri(operands);
    let base = task.call_stack.last().unwrap().base;
    let val = task.registers[base + (src_reg as usize)];
    isa::value::check_store_at(0, val)?;
    let gi = gidx as usize;
    let mut globals = rt.globals.lock().unwrap();
    if gi >= globals.len() {
        globals.resize(gi + 1, Value::null());
    }
    globals[gi] = val;
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `rA` = capture slot `1 + bx` of the function object the frame was entered through.
pub(crate) fn getcapture(task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (dest_reg, cap_idx) = decode_ri(operands);
    let (closure, base) = {
        let frame = task.call_stack.last().unwrap();
        (frame.closure, frame.base)
    };
    let closure = closure
        .ok_or_else(|| "GETCAPTURE in a frame that was not entered through a closure".to_string())?;
    // SAFETY: `closure` is the live function object this frame was entered through.
    let val = unsafe { closure.as_ref().get_field(1 + cap_idx as usize) };
    task.registers[base + (dest_reg as usize)] = val;
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn enter_call(
    rt: &Runtime,
    task: &mut TaskContext,
    dest_reg: u8,
    callee_code_idx: usize,
    arg_start: usize,
    closure: Option<NonNull<ObjectHeader>>,
) -> Result<VmStatus, String> {
    if callee_code_idx >= rt.code_objects.len() {
        return Err(format!("Invalid callee code object index: {}", callee_code_idx));
    }
    let caller_base = task.call_stack.last().unwrap().base;
    let caller_code_idx = task.current_code;
    let caller_reg_count = rt.code_objects[caller_code_idx].register_count as usize;

    let dest = caller_base + (dest_reg as usize);
    let return_pc = task.pc + 1;

    let callee_reg_count = rt.code_objects[callee_code_idx].register_count as usize;
    let param_count = rt.code_objects[callee_code_idx].param_count as usize;
    let callee_base = caller_base + caller_reg_count;

    let total_regs_needed = callee_base + callee_reg_count;
    rt.check_stack(total_regs_needed, task.region_bytes(), task.call_stack.len())?;
    if task.registers.len() < total_regs_needed {
        task.registers.resize(total_regs_needed, Value::null());
    }

    for i in 0..param_count {
        let caller_arg_slot = caller_base + arg_start + i;
        let callee_param_slot = callee_base + i;
        task.registers[callee_param_slot] = task.registers[caller_arg_slot];
    }

    task.call_stack.push(Frame {
        return_pc,
        base: callee_base,
        caller_code: caller_code_idx,
        dest_reg: dest,
        closure,
        generator: None,
    });

    task.current_code = callee_code_idx;
    task.pc = 0;
    Ok(VmStatus::Running)
}

/// Returns `rA` (or `null`) to the caller's destination register; at the bottom frame, writes `registers[0]` and halts.
pub fn ret(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (ret_reg, _) = decode_r2(operands);
    let current_frame = task.call_stack.last().unwrap();
    if let Some(link) = current_frame.generator {
        return crate::handlers::generator::finish(rt, task, link);
    }
    let ret_val = task.registers[current_frame.base + (ret_reg as usize)];
    isa::value::check_store_at(task.arenas.depth(), ret_val)?;

    if task.call_stack.len() == 1 {
        task.call_stack.pop();
        task.registers[0] = ret_val;
        return Ok(VmStatus::Halted);
    }

    let finished_frame = task.call_stack.pop().unwrap();
    task.registers[finished_frame.dest_reg] = ret_val;
    task.current_code = finished_frame.caller_code;
    task.pc = finished_frame.return_pc;
    Ok(VmStatus::Running)
}

/// `RETN rA, n`: the `n` registers from `rA` go to the caller's destination register and the ones after it.
pub(crate) fn retn(task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (first, count) = decode_r2(operands);
    let frame = task.call_stack.last().unwrap();
    if frame.generator.is_some() || task.call_stack.len() == 1 {
        return Err("RETN outside a plain call".to_string());
    }
    let from = frame.base + first as usize;
    let depth = task.arenas.depth();
    for i in 0..count as usize {
        isa::value::check_store_at(depth, task.registers[from + i])?;
    }
    let finished_frame = task.call_stack.pop().unwrap();
    for i in 0..count as usize {
        task.registers[finished_frame.dest_reg + i] = task.registers[from + i];
    }
    task.current_code = finished_frame.caller_code;
    task.pc = finished_frame.return_pc;
    Ok(VmStatus::Running)
}

/// No-op: advance `pc`.
pub(crate) fn nop(task: &mut TaskContext) -> Result<VmStatus, String> {
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `SCOPEENTER`: opens a structured-concurrency scope on the running task, recording the call-stack depth so `SPAWN` only attaches to a scope opened in its own function.
pub(crate) fn scopeenter(task: &mut TaskContext) -> Result<VmStatus, String> {
    task.shared.scopes.lock().unwrap().push(ScopeFrame {
        owner_call_depth: task.call_stack.len(),
        ..ScopeFrame::default()
    });
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `SCOPEEXIT rA`: closes the innermost scope. While children run, parks the task with `pc` left here; otherwise pops the scope and writes the outcome to `rA`.
pub(crate) fn scopeexit(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (dest_reg, _) = decode_r2(operands);
    let sf = {
        let mut scopes = task.shared.scopes.lock().unwrap();
        let pending = scopes
            .last()
            .ok_or_else(|| "SCOPEEXIT with no open scope".to_string())?
            .pending;
        if pending > 0 {
            return Ok(VmStatus::Parked);
        }
        scopes.pop().unwrap()
    };
    let observed = sf.first_error_handle.is_some_and(|h| {
        // SAFETY: a live `Task<T>` handle's slots are all initialised.
        unsafe { h.as_ref().get_field(sched_intrinsics::TASK_SLOT_OBSERVED) }.is_truthy()
    });
    let unobserved_fault = if observed { None } else { sf.first_error };
    let dest = task.call_stack.last().unwrap().base + (dest_reg as usize);
    task.registers[dest] = match unobserved_fault {
        Some(msg) => rt.alloc_heap_string(&msg),
        None => Value::null(),
    };
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// `SPAWN rA, rB`: starts the zero-argument closure in `rB` as a child task; `rA` receives its `Task<T>` handle.
pub fn spawn(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (dest_reg, closure_reg) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;

    let call_depth = task.call_stack.len();
    let attach_depth = {
        let scopes = task.shared.scopes.lock().unwrap();
        scopes
            .last()
            .filter(|sf| sf.owner_call_depth == call_depth)
            .map(|_| scopes.len() - 1)
    };

    let closure_val = task.registers[base + (closure_reg as usize)];
    let code_idx = closure_val
        .as_function_code_idx()
        .ok_or_else(|| format!("SPAWN target in r{closure_reg} is not a callable value"))?;
    let reg_count = rt
        .code_objects
        .get(code_idx)
        .ok_or_else(|| format!("SPAWN: invalid callee code object index {code_idx}"))?
        .register_count as usize;

    let (parent, scope_depth) = match attach_depth {
        Some(depth) => (Some(task.task_id), depth),
        None => (None, 0),
    };
    let mut child = rt.acquire_task(code_idx, reg_count, parent, scope_depth);
    child.call_stack[0].closure = closure_val.as_object_ptr();
    if let Some(closure) = closure_val.as_object_ptr() {
        super::channel::move_captured_senders(task, &mut child, closure);
    }
    let handle = crate::handlers::sched_intrinsics::alloc_task_handle(rt, child.task_id);
    child.handle = Some(handle);

    match attach_depth {
        Some(depth) => {
            task.shared.scopes.lock().unwrap()[depth].pending += 1;
        }
        None => {
            rt.sched.lock().unwrap().detached.insert(handle);
        }
    }
    rt.schedule(child);

    task.registers[base + (dest_reg as usize)] = Value::boxed(handle);
    task.pc += 1;
    Ok(VmStatus::Running)
}
