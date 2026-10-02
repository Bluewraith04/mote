//! Tier 2 region allocation: `ENTERARENA EXITARENA ARENAALLOC`.

use std::ptr::NonNull;

use isa::value::Value;

use crate::util::decode_ri;
use crate::machine::{ObjectStore, TypeTable};
use crate::{TaskContext, VmStatus};

/// Open a region. The operand (once a capacity hint) is ignored: segments grow on demand.
pub fn enter(task: &mut TaskContext, _operands: u32) -> Result<VmStatus, String> {
    if task.call_stack.last().is_some_and(|f| f.generator.is_some()) && !super::generator::is_swapped(task) {
        super::generator::swap_in(task, Box::default());
    }
    task.arenas.enter_arena();
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// Close the innermost region and free what it holds.
pub fn exit(task: &mut TaskContext) -> Result<VmStatus, String> {
    task.arenas.exit_arena()?;
    task.pc += 1;
    Ok(VmStatus::Running)
}

/// Allocate an object of `type_descriptors[bx]` in the current arena.
pub fn alloc<M: TypeTable + ObjectStore>(rt: &M, task: &mut TaskContext, operands: u32) -> Result<VmStatus, String> {
    let (dest_reg, type_idx) = decode_ri(operands);
    let type_idx = type_idx as usize;
    let types = rt.type_descriptors();
    if type_idx >= types.len() {
        return Err(format!("Invalid type descriptor index: {}", type_idx));
    }
    let type_ptr = NonNull::from(&types[type_idx]);
    let obj_ptr = if task.arenas.too_deep() {
        task.arenas.count_too_deep();
        rt.alloc_object(type_ptr, types[type_idx].slots as usize)
    } else {
        task.arenas.alloc_object(type_ptr)?
    };
    let dest = task.call_stack.last().unwrap().base + (dest_reg as usize);
    task.registers[dest] = Value::boxed(obj_ptr);
    task.pc += 1;
    Ok(VmStatus::Running)
}
