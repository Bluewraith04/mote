//! Lowers the typed tree into a control-flow graph of mutable registers and optimizes it.

mod builder;
pub mod facts;
mod generators;
pub mod intrinsics;
mod ir;
mod lower;
pub mod opt;
mod print;
pub mod reflect;
pub mod requirements;
#[cfg(test)]
mod tests;
mod uses;
mod verify;

pub use builder::FunctionBuilder;
pub use facts::{analyze, Allocation, Facts, FunctionFacts};
pub use ir::{Block, BlockId, Callee, Const, Entry, EntryKind, FrameSlot, FuncId, Function, FunctionKind, Inst, Module, Op, Operand, Reg, RegId, Terminator};
pub use lower::{lower, Lowered, Unsupported};
pub use opt::optimize;
pub use print::{print_function, print_module};
pub use uses::string_constants;
pub use verify::{verify, VerifyError};
