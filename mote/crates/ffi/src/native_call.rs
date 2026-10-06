use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use contracts::{
    NativeCtx, NativeEntry, NativeError, NativeFn, NativeId, NativeOutcome, NativeRegistry, Overflow, PlatformRequest, PlatformResult,
    SourceDecode, SourceRequest,
};
use isa::value::Value;
use crate::local_refs::LocalReferenceScope;

/// Context passed into a native function call from the VM.
pub struct NativeCallContext<'a> {
    pub args: &'a [Value],
    pub local_scope: &'a mut LocalReferenceScope,
    /// The allocation seam: collection and channel natives allocate GC objects through it; leaf natives ignore it.
    pub heap: &'a mut dyn NativeCtx,
}

impl<'a> NativeCallContext<'a> {
    pub fn new(
        args: &'a [Value],
        local_scope: &'a mut LocalReferenceScope,
        heap: &'a mut dyn NativeCtx,
    ) -> Self {
        Self { args, local_scope, heap }
    }

    #[inline(always)]
    pub fn arg(&self, idx: usize) -> Option<Value> {
        self.args.get(idx).copied()
    }
}

/// A plain native body: reads its arguments, returns a value or a fault message.
pub type PlainNativeFn = fn(&mut NativeCallContext) -> Result<Value, String>;

/// A fallible native body: a value, or a [`NativeError`] that is a recoverable failure or a fault.
pub type FallibleNativeFn = fn(&mut NativeCallContext) -> Result<Value, NativeError>;

/// Turns the platform's response into the native's result.
pub type ContinuationFn = Box<dyn FnOnce(&mut NativeCallContext, PlatformResult) -> Result<Value, NativeError> + Send>;
/// Reads the arguments and names the platform request plus its continuation.
pub type RequestFn = fn(&mut NativeCallContext) -> Result<(PlatformRequest, ContinuationFn), String>;

/// What a source native opens: the request, the channel capacity, the overflow policy and the payload decoder.
pub struct SourceSpec {
    pub request: SourceRequest,
    pub capacity: usize,
    pub overflow: Overflow,
    pub decode: SourceDecode,
}

/// Names the source to open; the native's result is the `Channel` its events arrive on.
pub type SourceFn = fn(&mut NativeCallContext) -> Result<SourceSpec, String>;

/// Names the exit code; the runtime stops the whole program.
pub type ExitFn = fn(&mut NativeCallContext) -> Result<i32, String>;

#[derive(Clone, Copy)]
/// How a native runs: plainly, on the platform, as an event source, or by ending the program.
pub enum NativeBody {
    Plain(PlainNativeFn),
    Fallible(FallibleNativeFn),
    Platform(RequestFn),
    Exit(ExitFn),
    Source(SourceFn),
}

/// A builtin as a [`NativeFn`]: runs its body under a fresh [`NativeCallContext`] and reports the outcome.
pub struct BuiltinNative {
    body: NativeBody,
}

impl NativeFn for BuiltinNative {
    fn call(&self, cx: &mut dyn NativeCtx, args: &[Value]) -> NativeOutcome {
        let mut scope = LocalReferenceScope::new();
        let mut ctx = NativeCallContext::new(args, &mut scope, cx);
        match self.body {
            NativeBody::Plain(func) => func(&mut ctx).into(),
            NativeBody::Fallible(func) => match func(&mut ctx) {
                Ok(value) => NativeOutcome::Done(value),
                Err(error) => NativeOutcome::Fail(error),
            },
            NativeBody::Exit(exit) => match exit(&mut ctx) {
                Ok(code) => NativeOutcome::Exit(code),
                Err(message) => NativeOutcome::Fail(NativeError::fault(message)),
            },
            NativeBody::Source(source) => match source(&mut ctx) {
                Ok(SourceSpec { request, capacity, overflow, decode }) => {
                    NativeOutcome::OpenSource { request, capacity, overflow, decode }
                }
                Err(message) => NativeOutcome::Fail(NativeError::fault(message)),
            },
            NativeBody::Platform(request) => match request(&mut ctx) {
                Ok((request, continuation)) => NativeOutcome::Platform(
                    request,
                    Box::new(move |cx, result| {
                        let mut scope = LocalReferenceScope::new();
                        let mut ctx = NativeCallContext::new(&[], &mut scope, cx);
                        continuation(&mut ctx, result)
                    }),
                ),
                Err(message) => NativeOutcome::Fail(NativeError::fault(message)),
            },
        }
    }
}

/// Thread-safe native registry; entries are in registration order.
#[derive(Default, Clone)]
pub struct NativeFunctionRegistry {
    entries: Arc<RwLock<Vec<NativeEntry>>>,
    name_map: Arc<RwLock<HashMap<String, u16>>>,
}

impl NativeFunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `callable` under `name` and return its index.
    pub fn register_fn(&self, name: &str, arity: Option<u8>, callable: Arc<dyn NativeFn>) -> u16 {
        let mut entries = self.entries.write();
        let idx = entries.len() as u16;
        entries.push(NativeEntry { name: name.to_string(), arity, callable });
        self.name_map.write().insert(name.to_string(), idx);
        idx
    }

    fn add(&self, name: &str, body: NativeBody) -> u16 {
        self.register_fn(name, None, Arc::new(BuiltinNative { body }))
    }

    pub fn register(&self, name: &str, func: PlainNativeFn) -> u16 {
        self.add(name, NativeBody::Plain(func))
    }

    /// Register a native declared `-> __Fallible<T>`: it may fail with a recoverable [`NativeError`].
    pub(crate) fn register_fallible(&self, name: &str, func: FallibleNativeFn) -> u16 {
        self.add(name, NativeBody::Fallible(func))
    }

    /// Register a native that asks the platform; takes one registry slot, like [`register`](Self::register).
    pub(crate) fn register_platform(&self, name: &str, request: RequestFn) -> u16 {
        self.add(name, NativeBody::Platform(request))
    }

    /// Register a native that opens an event source.
    pub fn register_source(&self, name: &str, source: SourceFn) -> u16 {
        self.add(name, NativeBody::Source(source))
    }

    /// Register a native that ends the program.
    pub(crate) fn register_exit(&self, name: &str, exit: ExitFn) -> u16 {
        self.add(name, NativeBody::Exit(exit))
    }

    pub fn get(&self, idx: u16) -> Option<NativeEntry> {
        self.entries.read().get(idx as usize).cloned()
    }

    pub fn get_by_name(&self, name: &str) -> Option<u16> {
        self.name_map.read().get(name).copied()
    }

    /// Invokes a registered native by index. A platform native returns its request and continuation
    /// for the runtime to run (inline, or off the worker while the task parks).
    pub fn call(&self, idx: u16, args: &[Value], cx: &mut dyn NativeCtx) -> NativeOutcome {
        match self.get(idx) {
            Some(entry) => entry.callable.call(cx, args),
            None => NativeOutcome::Fail(NativeError::fault(format!("Invalid native function index: {idx}"))),
        }
    }
}

impl NativeRegistry for NativeFunctionRegistry {
    fn resolve(&self, name: &str) -> Option<NativeId> {
        self.get_by_name(name)
    }

    fn entry(&self, id: NativeId) -> Option<NativeEntry> {
        self.get(id)
    }
}
