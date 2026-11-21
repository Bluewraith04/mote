//! Resolves names and types and produces the typed expression tree.

mod analyze;
mod check;
mod cx;
mod defs;
#[cfg(any(test, feature = "testing"))]
mod harness;
mod instantiate;
mod native;
mod program;
mod reflect;
mod resolve;
#[cfg(test)]
mod std_tests;
#[cfg(any(test, feature = "testing"))]
mod testing;
mod tree;
mod types;
mod walk;

pub use analyze::{analyze, PARKING_INTRINSICS};
pub use check::{check, plan};
pub use defs::*;
#[cfg(any(test, feature = "testing"))]
pub use harness::{compile, compile_all, std_sources, Compiled};
pub use instantiate::instantiate;
pub use native::{c_type, signature, NativeError, NativeFailure, NativeReason};
pub use program::{Init, Instance, Program};
pub use reflect::{field_flag, method_flag, AttributeInfo, FieldInfo, Kind, MethodInfo, ReflectError, Reflected, Shape, VariantInfo, MAX_REFLECTED};
pub use resolve::{resolve, Input, ModuleInput, KNOWN as ATTRIBUTES};
pub use tree::*;
pub use types::{FloatTy, FnParam, FnTy, IntTy, TyId, TyKind, TypeNames, Types};
