//! `NEWSTR`: allocates a heap string from the current code object's `string_table`, through the worker's `Mutator`.

use std::ptr::NonNull;

use isa::value::{string_slot_count, TypeDescriptor, Value};

use crate::util::decode_ri;
use crate::machine::{CodeTable, ObjectStore, TypeTable};
use crate::{TaskContext, VmStatus};

pub(crate) fn alloc_heap_string<M: TypeTable + ObjectStore>(rt: &M, bytes: &[u8]) -> Value {
    let slot_count = string_slot_count(bytes.len());
    let type_ptr: NonNull<TypeDescriptor> = NonNull::from(rt.string_type());

    let obj_ptr = rt.alloc_object(type_ptr, slot_count);

    // SAFETY: `obj_ptr` has `slot_count` slots, enough for the length slot and the bytes.
    unsafe {
        let header = obj_ptr.as_ptr();
        (*header).set_field(0, Value::uint(bytes.len() as u64));
        if !bytes.is_empty() {
            let dst = (*header).field_ptr(1) as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        }
    }

    Value::boxed(obj_ptr)
}

/// `rA = new heap string from string_table[bx]`.
pub(crate) fn newstr<M: CodeTable + TypeTable + ObjectStore>(
    rt: &M,
    task: &mut TaskContext,
    operands: u32,
) -> Result<VmStatus, String> {
    let (dest_reg, str_idx) = decode_ri(operands);

    let bytes: Vec<u8> = rt
        .code_objects()
        .get(task.current_code)
        .and_then(|c| c.string_table.get(str_idx as usize))
        .map(|s| s.as_bytes().to_vec())
        .ok_or_else(|| {
            format!("NEWSTR: no string_table[{str_idx}] in code object {}", task.current_code)
        })?;

    let v = alloc_heap_string(rt, &bytes);
    let dest = task.call_stack.last().unwrap().base + (dest_reg as usize);
    task.registers[dest] = v;
    task.pc += 1;
    Ok(VmStatus::Running)
}
