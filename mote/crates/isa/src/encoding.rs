use crate::opcode::Opcode;

pub type Instruction = u32;

pub fn encode_r3(op: Opcode, a: u8, b: u8, c: u8) -> Instruction {
    (op as u32) | ((a as u32) << 8) | ((b as u32) << 16) | ((c as u32) << 24)
}

pub fn encode_r2(op: Opcode, a: u8, b: u8) -> Instruction {
    (op as u32) | ((a as u32) << 8) | ((b as u32) << 16)
}

pub fn encode_ri(op: Opcode, a: u8, bx: u16) -> Instruction {
    (op as u32) | ((a as u32) << 8) | ((bx as u32) << 16)
}

pub fn encode_ju(op: Opcode, sbx: i32) -> Instruction {
    let raw = (sbx as u32) & 0x00FF_FFFF;
    (op as u32) | (raw << 8)
}

pub fn encode_jc(op: Opcode, a: u8, sbx: i16) -> Instruction {
    let raw = (sbx as u16 as u32) & 0xFFFF;
    (op as u32) | ((a as u32) << 8) | (raw << 16)
}

pub fn encode_none(op: Opcode) -> Instruction {
    op as u32
}

/// `CALL rA, bx` — code object `bx` over the window `rA+1..`; the result lands in `rA`.
pub fn encode_call(dest: u8, func_idx: u16) -> Instruction {
    encode_ri(Opcode::CALL, dest, func_idx)
}

pub fn encode_ret(ret_reg: u8) -> Instruction {
    encode_r2(Opcode::RET, ret_reg, 0)
}

/// `SCOPEEXIT rA` — `rA` receives the close-out outcome: `null`, or
/// an unobserved fault message as a heap `String`.
pub fn encode_scopeexit(dest: u8) -> Instruction {
    encode_r2(Opcode::SCOPEEXIT, dest, 0)
}

pub fn encode_newobj(dest: u8, type_idx: u16) -> Instruction {
    encode_ri(Opcode::NEWOBJ, dest, type_idx)
}

pub fn encode_newstr(dest: u8, str_idx: u16) -> Instruction {
    encode_ri(Opcode::NEWSTR, dest, str_idx)
}

pub fn encode_callnative(dest: u8, func_idx: u8, arg_base: u8) -> Instruction {
    encode_r3(Opcode::CALLNATIVE, dest, func_idx, arg_base)
}

/// `CALLINTRINSIC rA, id, rC`.
pub fn encode_callintrinsic(dest: u8, id: u8, arg_base: u8) -> Instruction {
    encode_r3(Opcode::CALLINTRINSIC, dest, id, arg_base)
}

/// `CALLNATIVEW rA, table_idx` — args at `rA+1..`, result in `rA`.
pub fn encode_callnativew(dest: u8, table_idx: u16) -> Instruction {
    encode_ri(Opcode::CALLNATIVEW, dest, table_idx)
}

/// `CALLNATIVEF rA, table_idx` — `CALLNATIVEW` for a fallible native.
pub fn encode_callnativef(dest: u8, table_idx: u16) -> Instruction {
    encode_ri(Opcode::CALLNATIVEF, dest, table_idx)
}

pub fn encode_callv(dest: u8, callee_reg: u8, arg_base: u8) -> Instruction {
    encode_r3(Opcode::CALLV, dest, callee_reg, arg_base)
}

pub fn encode_getcapture(dest: u8, capture_idx: u16) -> Instruction {
    encode_ri(Opcode::GETCAPTURE, dest, capture_idx)
}

pub fn encode_getglobal(dest: u8, global_idx: u16) -> Instruction {
    encode_ri(Opcode::GETGLOBAL, dest, global_idx)
}

pub fn encode_setglobal(src: u8, global_idx: u16) -> Instruction {
    encode_ri(Opcode::SETGLOBAL, src, global_idx)
}

pub fn encode_getfield(dest: u8, obj_reg: u8, field_slot: u8) -> Instruction {
    encode_r3(Opcode::GETFIELD, dest, obj_reg, field_slot)
}

pub fn encode_setfield(obj_reg: u8, field_slot: u8, val_reg: u8) -> Instruction {
    encode_r3(Opcode::SETFIELD, obj_reg, field_slot, val_reg)
}

pub fn encode_typeof(dest: u8, src_reg: u8) -> Instruction {
    encode_r2(Opcode::TYPEOF, dest, src_reg)
}