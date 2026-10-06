//! The native functions: their registry, the call context they run in and the built-in set.
/// The built-in native functions.
pub mod builtins;
/// Storage for values created during one native call.
pub mod local_refs;
/// The native function types and the registry.
pub mod native_call;

pub use local_refs::LocalReferenceScope;
pub use native_call::{BuiltinNative, ContinuationFn, ExitFn, NativeBody, NativeCallContext, NativeFunctionRegistry, PlainNativeFn, RequestFn};
