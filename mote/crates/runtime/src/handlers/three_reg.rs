use isa::opcode::Opcode::{self, *};
use isa::value::Value;
use isa::equality::values_equal;

use crate::util::decode_r3;
use crate::{Runtime, TaskContext, VmStatus};

/// The `r3` group: arithmetic, boolean, bitwise, shift and comparison opcodes. Reads `rB` and `rC`, writes `rA`.
pub fn binary(
    rt: &Runtime,
    task: &mut TaskContext,
    opcode: Opcode,
    operands: u32,
) -> Result<VmStatus, String> {
    let (a, b, c) = decode_r3(operands);
    let base = task.call_stack.last().unwrap().base;
    let src1 = base + (b as usize);
    let src2 = base + (c as usize);
    let dest = base + (a as usize);

    if opcode == ADD
        && let Some(mut a) = task.registers[src1].as_heap_string()
        && let Some(b) = task.registers[src2].as_heap_string()
    {
        a.push_str(&b);
        let v = crate::handlers::strings::alloc_heap_string(rt, a.as_bytes());
        task.registers[dest] = v;
        task.pc += 1;
        return Ok(VmStatus::Running);
    }

    if matches!(opcode, DIV | MOD) && is_integer_zero(task.registers[src2]) {
        let what = if opcode == DIV { "division" } else { "remainder" };
        return Err(format!("arithmetic error: {what} by zero"));
    }
    if matches!(opcode, DIV | MOD) && is_min_by_minus_one(task.registers[src1], task.registers[src2]) {
        return Err("arithmetic error: integer overflow".to_string());
    }

    let registers = &mut task.registers;
    match opcode {
        ADD => add(registers, src1, src2, dest),
        SUB => sub(registers, src1, src2, dest),
        MUL => mul(registers, src1, src2, dest),
        DIV => div(registers, src1, src2, dest),
        MOD => modulo(registers, src1, src2, dest),
        AND => and(registers, src1, src2, dest),
        OR => or(registers, src1, src2, dest),
        XOR => xor(registers, src1, src2, dest),
        BAND => bitand(registers, src1, src2, dest),
        BOR => bitor(registers, src1, src2, dest),
        BXOR => bitxor(registers, src1, src2, dest),
        SHL => shl(registers, src1, src2, dest),
        SHR => shr(registers, src1, src2, dest),
        EQ => eq(registers, src1, src2, dest),
        NE => ne(registers, src1, src2, dest),
        LT => lt(registers, src1, src2, dest),
        LE => le(registers, src1, src2, dest),
        GT => gt(registers, src1, src2, dest),
        GE => ge(registers, src1, src2, dest),
        _ => unreachable!("three_reg::binary called with {opcode:?}"),
    }
    task.pc += 1;
    Ok(VmStatus::Running)
}

fn is_integer_zero(v: Value) -> bool {
    matches!(v, Value::Int(0) | Value::UInt(0))
}

fn is_min_by_minus_one(lhs: Value, rhs: Value) -> bool {
    matches!((lhs, rhs), (Value::Int(i64::MIN), Value::Int(-1)))
}

pub fn add(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a.wrapping_add(b)),
        (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
        _ => panic!("Unexpected Types in ADD"),
    };
}

pub fn sub(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a - b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a.wrapping_sub(b)),
        (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
        _ => panic!("Unexpected Types in SUB"),
    };
}

pub fn mul(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a * b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a.wrapping_mul(b)),
        (Value::Float(a), Value::Float(b)) => Value::Float(a * b),
        _ => panic!("Unexpected Types in MUL"),
    };
}

pub(crate) fn div(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a / b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a / b),
        (Value::Float(a), Value::Float(b)) => Value::Float(a / b),
        _ => panic!("Unexpected Types in DIV"),
    };
}

pub(crate) fn modulo(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a % b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a % b),
        (Value::Float(a), Value::Float(b)) => Value::Float(a % b),
        _ => panic!("Unexpected Types in MOD"),
    };
}

pub fn and(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Bool(a), Value::Bool(b)) => Value::Bool(a && b),
        _ => panic!("Who passes non boolean stuff into AND?"),
    };
}

pub fn or(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Bool(a), Value::Bool(b)) => Value::Bool(a || b),
        _ => panic!("Who passes non boolean stuff into OR?"),
    };
}

pub(crate) fn xor(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Bool(a), Value::Bool(b)) => Value::Bool(a != b),
        _ => panic!("Who passes non boolean stuff into XOR?"),
    };
}

pub(crate) fn bitand(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a & b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a & b),
        _ => panic!("Bitwise Operations only defined for Int and UInt"),
    };
}

pub(crate) fn bitor(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a | b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a | b),
        _ => panic!("Bitwise Operations only defined for Int and UInt"),
    };
}

pub(crate) fn bitxor(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a ^ b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a ^ b),
        _ => panic!("Bitwise Operations only defined for Int and UInt"),
    };
}

pub(crate) fn shl(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a << b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a << b),
        _ => panic!("Bitwise Operations only defined for Int and UInt"),
    };
}

pub(crate) fn shr(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    registers[dest] = match (registers[src1], registers[src2]) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a >> b),
        (Value::UInt(a), Value::UInt(b)) => Value::UInt(a >> b),
        _ => panic!("Bitwise Operations only defined for Int and UInt"),
    };
}

pub fn eq(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    let is_eq = values_equal(registers[src1], registers[src2]);
    registers[dest] = Value::Bool(is_eq);
}

pub fn ne(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    let is_eq = values_equal(registers[src1], registers[src2]);
    registers[dest] = Value::Bool(!is_eq);
}

fn string_order(lhs: Value, rhs: Value) -> Option<std::cmp::Ordering> {
    Some(lhs.heap_bytes()?.cmp(rhs.heap_bytes()?))
}

fn incomparable() -> bool {
    panic!("Cannot compare ordering between incompatible types")
}

pub fn lt(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    let (lhs, rhs) = (registers[src1], registers[src2]);
    let v = match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a < b,
        (Value::UInt(a), Value::UInt(b)) => a < b,
        (Value::Float(a), Value::Float(b)) => a < b,
        (Value::Char(a), Value::Char(b)) => a < b,
        _ => string_order(lhs, rhs).map_or_else(incomparable, |o| o.is_lt()),
    };
    registers[dest] = Value::Bool(v);
}

pub(crate) fn le(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    let (lhs, rhs) = (registers[src1], registers[src2]);
    let v = match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a <= b,
        (Value::UInt(a), Value::UInt(b)) => a <= b,
        (Value::Float(a), Value::Float(b)) => a <= b,
        (Value::Char(a), Value::Char(b)) => a <= b,
        _ => string_order(lhs, rhs).map_or_else(incomparable, |o| o.is_le()),
    };
    registers[dest] = Value::Bool(v);
}

pub fn gt(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    let (lhs, rhs) = (registers[src1], registers[src2]);
    let v = match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a > b,
        (Value::UInt(a), Value::UInt(b)) => a > b,
        (Value::Float(a), Value::Float(b)) => a > b,
        (Value::Char(a), Value::Char(b)) => a > b,
        _ => string_order(lhs, rhs).map_or_else(incomparable, |o| o.is_gt()),
    };
    registers[dest] = Value::Bool(v);
}

pub(crate) fn ge(registers: &mut [Value], src1: usize, src2: usize, dest: usize) {
    let (lhs, rhs) = (registers[src1], registers[src2]);
    let v = match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a >= b,
        (Value::UInt(a), Value::UInt(b)) => a >= b,
        (Value::Float(a), Value::Float(b)) => a >= b,
        (Value::Char(a), Value::Char(b)) => a >= b,
        _ => string_order(lhs, rhs).map_or_else(incomparable, |o| o.is_ge()),
    };
    registers[dest] = Value::Bool(v);
}

pub(crate) fn getfield(registers: &mut [Value], src_obj: usize, field_idx: usize, dest: usize) {
    let Value::ObjectPtr(ptr) = registers[src_obj] else {
        panic!("GETFIELD expected ObjectPtr, found {:?}", registers[src_obj]);
    };
    // SAFETY: a boxed pointer names a live object header.
    let val = unsafe { ptr.as_ref().get_field(field_idx) };
    registers[dest] = val;
}

/// Stores `registers[val_reg]` into a field; a fault if it points to a region object deeper than the target.
pub fn setfield(registers: &mut [Value], obj_reg: usize, field_idx: usize, val_reg: usize) -> Result<(), String> {
    let Value::ObjectPtr(ptr) = registers[obj_reg] else {
        panic!("SETFIELD expected ObjectPtr, found {:?}", registers[obj_reg]);
    };

    let val = registers[val_reg];
    // SAFETY: a boxed pointer names a live object header.
    let header_mut = unsafe { &mut *ptr.as_ptr() };
    isa::value::check_store(header_mut, val)?;
    // SAFETY: `field_idx` is within the object's slots.
    unsafe {
        header_mut.set_field(field_idx, val);
    }
    Ok(())
}
