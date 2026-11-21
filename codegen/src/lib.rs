//! Generates machine code from the intermediate representation with Cranelift.

mod calls;
mod channels;
mod collect;
mod debug;
#[cfg(not(target_os = "windows"))]
mod dwarf;
mod closures;
mod emit;
mod frames;
mod function;
mod host;
mod jit;
mod locks;
mod mem;
mod meta;
mod native;
mod numeric;
mod objects;
#[cfg(test)]
mod refusals;
mod rows;
mod scalar;
mod seqs;
mod streams;
mod system;
mod tasks;
mod text;
pub mod typetable;
mod values;

pub use emit::{compile_object, init_symbol, ObjectLibrary, Resolve};
pub use host::{Fault, Feed, Ran};
pub use mote_runtime::Located;
pub use jit::{compile, compile_start, compile_with, CompileError, Compiled};
pub use mote_runtime::loader::LinkError;
pub use rows::FOLDED_ROWS;
pub use typetable::{build as build_types, TypeMap};
