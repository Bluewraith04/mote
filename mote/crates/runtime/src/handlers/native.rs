//! `CALLNATIVE`/`CALLNATIVEW` dispatch natives through the installed [`crate::NativeDispatchHook`]; `CALLINTRINSIC` runs scheduler-aware intrinsics.

use super::sched_intrinsics;
use crate::util::{decode_r3, decode_ri};
use crate::{NativeOutcome, PlatformContinuation, Runtime, TaskContext, VmStatus};
use contracts::{NativeCtx, NativeError, PlatformRequest, Wake, Woken};

/// `rA = native_fn[b](rC..)`: an 8-bit registry id, or a slot of the program's native table when it has one.
pub(crate) fn callnative(
    rt: &Runtime,
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (dest_reg, func_idx, arg_base) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    let dest = base + (dest_reg as usize);
    let args_start = base + (arg_base as usize);

    if task.awaiting_platform {
        return resume_platform(rt, task, dest, false);
    }

    let id = if rt.native_ids.is_empty() {
        func_idx as u16
    } else {
        *rt.native_ids
            .get(func_idx as usize)
            .ok_or_else(|| format!("CALLNATIVE {func_idx} is outside the program's native table"))?
    };
    call_leaf(rt, task, id, dest, args_start, false)
}

/// `rA = intrinsic[b](rC..)`: a scheduler-aware runtime operation.
pub(crate) fn callintrinsic(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (dest_reg, id, arg_base) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    sched_intrinsics::dispatch(rt, task, id, base + dest_reg as usize, base + arg_base as usize)
}

/// `rA = native_table[bx](rA+1..)` — a builtin by its program-table slot.
pub(crate) fn callnativew(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    call_table_native(rt, task, operands, false)
}

/// `CALLNATIVEW` for a fallible native: a recoverable failure lands in `rA+1` and `rA+2`.
pub(crate) fn callnativef(rt: &Runtime, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    call_table_native(rt, task, operands, true)
}

fn call_table_native(rt: &Runtime, task: &mut TaskContext, operands: u32, fallible: bool) -> Result<VmStatus, String> {
    let (dest_reg, slot) = decode_ri(operands);
    let base = task.call_stack.last().unwrap().base;
    let dest = base + (dest_reg as usize);
    if task.awaiting_platform {
        return resume_platform(rt, task, dest, fallible);
    }
    let id = *rt
        .native_ids
        .get(slot as usize)
        .ok_or_else(|| format!("native {slot} is outside the program's native table"))?;
    call_leaf(rt, task, id, dest, dest + 1, fallible)
}

fn finish_ok(task: &mut TaskContext, dest: usize, value: isa::value::Value, fallible: bool) -> Result<VmStatus, String> {
    if fallible {
        task.registers[dest + 1] = isa::value::Value::int(-1);
    }
    finish(task, dest, value)
}

fn finish_failure(rt: &Runtime, task: &mut TaskContext, dest: usize, code: i32, message: &str) -> Result<VmStatus, String> {
    let text = rt.with_intrinsic_ctx(|ctx| ctx.alloc_string(message.as_bytes()))?;
    task.registers[dest + 1] = isa::value::Value::int(code as i64);
    task.registers[dest + 2] = text;
    finish(task, dest, isa::value::Value::null())
}

fn finish_result(
    rt: &Runtime,
    task: &mut TaskContext,
    dest: usize,
    result: Result<isa::value::Value, NativeError>,
    fallible: bool,
) -> Result<VmStatus, String> {
    match result {
        Ok(value) => finish_ok(task, dest, value, fallible),
        Err(error) => match error.code().filter(|_| fallible) {
            Some(code) => finish_failure(rt, task, dest, code, &error.message),
            None => Err(error.message),
        },
    }
}

fn call_leaf(rt: &Runtime, task: &mut TaskContext, id: u16, dest: usize, args_start: usize, fallible: bool) -> Result<VmStatus, String> {
    let args = &task.registers[args_start..];
    let dispatcher = rt
        .native_dispatcher
        .clone()
        .ok_or_else(|| format!("No native dispatcher registered for CALLNATIVE (native {})", id))?;
    let outcome = rt.with_intrinsic_ctx(|ctx| dispatcher(id, ctx, args));
    match outcome {
        NativeOutcome::Done(value) => finish_ok(task, dest, value, fallible),
        NativeOutcome::Fail(error) => finish_result(rt, task, dest, Err(error), fallible),
        NativeOutcome::Platform(request, continuation) => platform_call(rt, task, dest, request, continuation, fallible),
        NativeOutcome::OpenSource { request, capacity, overflow, decode } => {
            let channel = rt.open_source(request, capacity, overflow, decode)?;
            finish(task, dest, channel)
        }
        NativeOutcome::Exit(code) => {
            rt.request_exit(code);
            Ok(VmStatus::Exited(code))
        }
    }
}

fn finish(task: &mut TaskContext, dest: usize, value: isa::value::Value) -> Result<VmStatus, String> {
    task.registers[dest] = value;
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn platform_call(
    rt: &Runtime,
    task: &mut TaskContext,
    dest: usize,
    request: PlatformRequest,
    continuation: PlatformContinuation,
    fallible: bool,
) -> Result<VmStatus, String> {
    let platform = rt.platform.clone().ok_or("no platform installed")?;
    if !platform.blocking(&request) {
        let result = platform.execute(request);
        let result = rt.with_intrinsic_ctx(|ctx| continuation(ctx, result));
        return finish_result(rt, task, dest, result, fallible);
    }
    let id = task.task_id;
    {
        let mut st = rt.sched.lock().unwrap();
        st.offload_pending.insert(id, continuation);
        st.offload_inflight += 1;
    }
    task.awaiting_platform = true;
    let handle = rt.offload.handle();
    let runner = platform.clone();
    let wake: Wake = Box::new(move |woken| match woken {
        Woken::Run(request) => handle.submit(id, Box::new(move || runner.execute(request))),
        Woken::Done(output) => handle.complete(id, Ok(output)),
    });
    if let Err(unwaited) = platform.wait(request, wake) {
        let (request, _) = *unwaited;
        rt.offload.submit(id, Box::new(move || platform.execute(request)));
    }
    Ok(VmStatus::Parked)
}

fn resume_platform(rt: &Runtime, task: &mut TaskContext, dest: usize, fallible: bool) -> Result<VmStatus, String> {
    let id = task.task_id;
    let (output, continuation) = {
        let mut st = rt.sched.lock().unwrap();
        match st.offload_done.remove(&id) {
            Some(out) => (out, st.offload_pending.remove(&id)),
            None => return Ok(VmStatus::Parked),
        }
    };
    task.awaiting_platform = false;
    let output = output?;
    let continuation = continuation.ok_or("platform response without a continuation")?;
    let result = rt.with_intrinsic_ctx(|ctx| continuation(ctx, output));
    finish_result(rt, task, dest, result, fallible)
}
