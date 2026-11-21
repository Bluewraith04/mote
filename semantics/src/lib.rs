//! Resolves names and types and produces the typed expression tree.

mod analyze;
pub mod builtins;
mod check;
pub mod comptime;
mod cx;
mod defs;
#[cfg(any(test, feature = "testing"))]
mod harness;
mod instantiate;
mod native;
mod program;
mod resolve;
#[cfg(test)]
mod std_tests;
#[cfg(any(test, feature = "testing"))]
mod testing;
mod tree;
mod types;
mod walk;

pub use analyze::{analyze, PARKING_BUILTINS};
pub use check::{check, plan, view_coercion_fn};
pub use comptime::{layout_align, layout_size, lower_body as comptime_lower};
pub use defs::*;
#[cfg(any(test, feature = "testing"))]
pub use harness::{compile, compile_all, std_sources, Compiled};
pub use instantiate::instantiate;
pub use native::{c_type, signature, NativeError, NativeFailure, NativeReason};
pub use program::{Init, Instance, Program, StdItems};
pub use resolve::{resolve, Input, ModuleInput, KNOWN as ATTRIBUTES};
pub use tree::*;
pub use types::{BuiltinTy, ConstArg, ConstExpr, ConstVal, FloatTy, FnParam, FnTy, GenericArg, IntTy, TyId, TyKind, TypeNames, Types};
