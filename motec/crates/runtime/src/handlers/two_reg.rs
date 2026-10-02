use isa::opcode::Opcode::{self, *};
use isa::value::Value;

use crate::util::decode_r2;
use crate::{TaskContext, VmStatus};

/// The `r2` group: unary arithmetic and logical ops, `MOVE` and `DEBUGPRINT`. Reads `rB`, writes `rA`.
pub fn unary(task: &mut TaskContext, opcode: Opcode, operands: u32) -> Result<VmStatus, String> {
    let (a, b) = decode_r2(operands);
    let base = task.call_stack.last().unwrap().base;
    let src = base + (b as usize);
    let dest = base + (a as usize);
    match opcode {
        NEG => neg(&mut task.registers, src, dest),
        NOT => not(&mut task.registers, src, dest),
        BNOT => bitnot(&mut task.registers, src, dest),
        MOVE => move_(&mut task.registers, src, dest),
        DEBUGPRINT => debugprint(&mut task.registers, src),
        _ => unreachable!("two_reg::unary called with {opcode:?}"),
    }
    task.pc += 1;
    Ok(VmStatus::Running)
}

pub(crate) fn move_(registers: &mut [Value], src: usize, dest: usize) {
    registers[dest] = registers[src];
}

pub fn neg(registers: &mut [Value], src: usize, dest: usize) {
    registers[dest] = match registers[src] {
        Value::Int(n) => Value::Int(-n),
        Value::Float(f) => Value::Float(-f),
        _ => panic!("Unsupported Operand Type"),
    };
}

pub fn not(registers: &mut [Value], src: usize, dest: usize) {
    registers[dest] = match registers[src] {
        Value::Bool(b) => Value::Bool(!b),
        _ => panic!("Unsupported Operand Type"),
    };
}

pub(crate) fn bitnot(registers: &mut [Value], src: usize, dest: usize) {
    registers[dest] = match registers[src] {
        Value::Int(n) => Value::Int(!n),
        Value::UInt(n) => Value::UInt(!n),
        _ => panic!("Unsupported Operand Type"),
    };
}

pub(crate) fn debugprint(registers: &mut [Value], src: usize) {
    let v0 = registers[src];
    match v0 {
        Value::Int(n) => println!("Int({n})"),
        Value::UInt(n) => println!("UInt({n})"),
        Value::Float(f) => println!("Float({f})"),
        Value::Char(c) => println!("Char({c:?})"),
        Value::Symbol(id) => println!("Symbol({id})"),
        Value::NativeFn(p) => println!("NativeFn({p:?})"),
        Value::Bool(true) => println!("True"),
        Value::Bool(false) => println!("False"),
        Value::ObjectPtr(p) => match v0.as_heap_string() {
            Some(s) => println!("{s}"),
            None => println!("MoteObject(Heap) at <{p:?}>"),
        },
        Value::Inline(p) => match v0.as_short_str() {
            Some(s) => println!("{s}"),
            None => println!("Inline Payload {p:?}"),
        },
        Value::Null => println!("Null"),
    }
}

/// `dest` = the descriptor id of an object, or a negative code for a scalar, so the two never collide.
pub(crate) fn typeof_(registers: &mut [Value], src: usize, dest: usize) {
    registers[dest] = Value::int(match registers[src] {
        Value::Int(_) => -1,
        Value::Float(_) => -2,
        Value::Bool(_) => -3,
        Value::Null => -4,
        Value::Inline(_) => -5,
        Value::UInt(_) => -6,
        Value::Char(_) => -7,
        Value::Symbol(_) => -8,
        Value::NativeFn(_) => -9,
        // SAFETY: a boxed pointer names a live object header whose descriptor outlives it.
        Value::ObjectPtr(p) => unsafe { p.as_ref().type_ptr.as_ref().id as i64 },
    });
}
