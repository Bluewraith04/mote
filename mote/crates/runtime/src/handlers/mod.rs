//! Per-opcode execution: `Runtime::step` delegates to [`execute`], and each match arm's body lives in a submodule grouped by instruction shape.

use isa::opcode::Opcode::{self, *};

use crate::{Runtime, TaskContext, VmStatus};

pub mod arena;
pub mod channel;
pub mod control;
pub mod generator;
#[cfg(test)]
mod generator_tests;
pub mod native;
pub mod object;
pub mod sched_intrinsics;
pub mod shared_cell;
pub mod strings;
#[cfg(test)]
mod tests;
/// The three-register opcodes.
pub mod three_reg;
/// The two-register opcodes.
pub mod two_reg;

/// Executes one decoded instruction; each arm advances `pc` itself, or assigns it for control flow.
pub fn execute(
    rt: &Runtime,
    task: &mut TaskContext,
    opcode: Opcode,
    operands: u32,
) -> Result<VmStatus, String> {
    match opcode {
        ADD | SUB | MUL | DIV | MOD | AND | OR | XOR | BAND | BOR | BXOR | SHL | SHR | EQ | NE
        | LT | LE | GT | GE => three_reg::binary(rt, task, opcode, operands),
        NEG | NOT | BNOT | MOVE | DEBUGPRINT => two_reg::unary(task, opcode, operands),
        LOADI | LOADK => control::load(rt, task, opcode, operands),
        JMP => control::jmp(rt, task, operands),
        JMPIF => control::jmpif(rt, task, operands),
        JMPIFNOT => control::jmpifnot(rt, task, operands),
        CALL => control::call(rt, task, operands),
        CALLV => control::callv(rt, task, operands),
        GETCAPTURE => control::getcapture(task, operands),
        GETGLOBAL => control::getglobal(rt, task, operands),
        SETGLOBAL => control::setglobal(rt, task, operands),
        SCOPEENTER => control::scopeenter(task),
        SCOPEEXIT => control::scopeexit(rt, task, operands),
        SPAWN => control::spawn(rt, task, operands),
        RET => control::ret(rt, task, operands),
        RETN => control::retn(task, operands),
        NOP => control::nop(task),
        HALT => Ok(VmStatus::Halted),
        NEWOBJ => object::newobj(rt, task, operands),
        GETFIELD => object::getfield(task, operands),
        SETFIELD => object::setfield(task, operands),
        TYPEOF => object::type_of(task, operands),
        COPYVAL => object::copyval(rt, task, operands),
        STAMP => object::stamp(rt, task, operands),
        LOADTYPE => object::loadtype(rt, task, operands),
        STAMPT => object::stampt(rt, task, operands),
        ASTYPE => object::astype(rt, task, operands),
        ISTYPE => object::istype(rt, task, operands),
        SOME => object::some(rt, task, operands),
        UNSOME => object::unsome(task, operands),
        ENTERARENA => arena::enter(task, operands),
        EXITARENA => arena::exit(task),
        ARENAALLOC => arena::alloc(rt, task, operands),
        CALLNATIVE => native::callnative(rt, task, operands),
        CALLNATIVEW => native::callnativew(rt, task, operands),
        CALLNATIVEF => native::callnativef(rt, task, operands),
        CALLINTRINSIC => native::callintrinsic(rt, task, operands),
        NEWSTR => strings::newstr(rt, task, operands),
        MKGEN => generator::mkgen(rt, task),
        RESUME => generator::resume(rt, task, operands),
        YIELD => generator::yield_(rt, task, operands),
    }
}
