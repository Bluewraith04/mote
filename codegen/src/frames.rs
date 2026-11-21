//! The frame descriptors of compiled functions, read from the stack maps Cranelift made.

use cranelift_codegen::CompiledCode;
use mote_abi::{FrameDescriptor, Slot, TypeTable};

use crate::function::Frame;
use crate::CompileError;

/// The descriptor of each call site of a compiled function, by the code offset of the return address.
///
/// A descriptor's offsets are from the base of the frame, which is the frame pointer less the frame size; the stack maps count from the stack pointer, which is the size of the outgoing arguments lower.
pub(crate) fn describe(code: &CompiledCode, frame: &Frame, types: &TypeTable) -> Result<Vec<(u32, FrameDescriptor)>, CompileError> {
    let size = code.frame_size;
    let mut refs = Vec::new();
    let mut dynamic = Vec::new();
    for (slot, ty) in &frame.memory {
        let base = code.sized_stackslot_offsets[*slot];
        for inner in &types.get(*ty).slots {
            match inner.shifted(base) {
                Slot::Ref(offset) => refs.push(offset),
                other => dynamic.push(other),
            }
        }
    }
    refs.extend(frame.words.iter().map(|slot| code.sized_stackslot_offsets[*slot]));
    let mut sites = Vec::new();
    for (at, span, map) in code.buffer.user_stack_maps() {
        let outgoing = span.checked_sub(size).ok_or_else(|| CompileError::internal("a stack map spans less than its frame"))?;
        let mut live = refs.clone();
        for (_, from_sp) in map.entries() {
            let offset = from_sp.checked_sub(outgoing).ok_or_else(|| CompileError::internal("a stack map entry lies below the frame"))?;
            // The probe is the first slot of the frame.
            if offset != 0 {
                live.push(offset);
            }
        }
        let descriptor = FrameDescriptor::new(size, live, dynamic.clone()).map_err(|error| CompileError::internal(format!("a frame descriptor: {error:?}")))?;
        sites.push((*at, descriptor));
    }
    Ok(sites)
}
