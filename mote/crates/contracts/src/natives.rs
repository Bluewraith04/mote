//! The native-function contract: what a native is given, what it returns, and how one is looked up.

use std::sync::Arc;

use isa::value::{TypeDescriptor, Value};

use crate::platform::{PlatformError, PlatformRequest, PlatformResponse};

pub type NativeId = u16;

/// What executing a [`PlatformRequest`] gives back.
pub type PlatformResult = Result<PlatformResponse, PlatformError>;

/// Turns the platform's response into the native's result.
pub type PlatformContinuation =
    Box<dyn FnOnce(&mut dyn NativeCtx, PlatformResult) -> Result<Value, NativeError> + Send>;

/// Turns one event payload into the source channel's element.
pub type SourceDecode = fn(&mut dyn NativeCtx, crate::EventPayload) -> Result<Value, String>;

/// A native's failure: a kind (0 is a plain fault, `n > 0` a recoverable `ErrorKind` code) and a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeError {
    pub kind: i32,
    pub message: String,
}

impl NativeError {
    /// A plain fault with no kind.
    pub fn fault(message: impl Into<String>) -> Self {
        Self { kind: 0, message: message.into() }
    }

    /// A recoverable failure of `ErrorKind` code `code`.
    pub fn failure(code: i32, message: impl Into<String>) -> Self {
        Self { kind: code + 1, message: message.into() }
    }

    /// The `ErrorKind` code of a recoverable failure, or `None` for a plain fault.
    pub fn code(&self) -> Option<i32> {
        (self.kind > 0).then(|| self.kind - 1)
    }
}

/// What a native call produces.
pub enum NativeOutcome {
    Done(Value),
    Fail(NativeError),
    /// Ask the platform, then run the continuation.
    Platform(PlatformRequest, PlatformContinuation),
    /// End the program with this exit code.
    Exit(i32),
    /// Open an event source and answer a `Channel` of `capacity` that it feeds.
    OpenSource { request: crate::SourceRequest, capacity: usize, overflow: crate::Overflow, decode: SourceDecode },
}

impl From<String> for NativeError {
    fn from(message: String) -> Self {
        Self::fault(message)
    }
}

impl From<&str> for NativeError {
    fn from(message: &str) -> Self {
        Self::fault(message)
    }
}

impl From<Result<Value, String>> for NativeOutcome {
    fn from(result: Result<Value, String>) -> Self {
        match result {
            Ok(value) => NativeOutcome::Done(value),
            Err(message) => NativeOutcome::Fail(NativeError::fault(message)),
        }
    }
}

/// One native function.
pub trait NativeFn: Send + Sync {
    fn call(&self, cx: &mut dyn NativeCtx, args: &[Value]) -> NativeOutcome;
}

/// A registry entry: the name a program resolves, its argument count if fixed, and the callable.
#[derive(Clone)]
pub struct NativeEntry {
    pub name: String,
    pub arity: Option<u8>,
    pub callable: Arc<dyn NativeFn>,
}

/// Resolves natives by name.
pub trait NativeRegistry {
    fn resolve(&self, name: &str) -> Option<NativeId>;
    fn entry(&self, id: NativeId) -> Option<NativeEntry>;
}

/// The allocation and slot-access capability handed to a native call.
pub trait NativeCtx {
    /// The outside world; an error if no platform is installed.
    fn platform(&self) -> Result<&dyn crate::Platform, String>;

    /// Allocates a fixed-layout intrinsic header of `slot_count` null slots.
    fn alloc_header(&mut self, type_id: u64, slot_count: usize) -> Result<Value, String>;

    /// Allocates a growable pointer backing with room for `capacity` elements.
    fn alloc_backing(&mut self, capacity: usize) -> Result<Value, String>;

    /// Allocates a growable raw-byte backing with room for `capacity` bytes.
    fn alloc_bytes_backing(&mut self, capacity: usize) -> Result<Value, String>;

    /// Allocates an open-addressed `Map` backing with `buckets` buckets.
    fn alloc_map_backing(&mut self, buckets: usize) -> Result<Value, String>;

    /// Frees a backing the caller has just replaced in its header.
    fn release_backing(&mut self, _backing: Value) {}

    /// Has the collector close the handle in `obj`'s slot 0 (a `RELEASE_*` kind) once no object holds it.
    fn release_on_collect(&mut self, _obj: Value, _kind: u8) -> Result<(), String> {
        Ok(())
    }

    /// A raw pointer to `obj`'s byte payload, valid only while `obj` stays reachable.
    fn bytes_ptr(&self, obj: Value) -> Result<*mut u8, String>;

    /// Reads slot `i` of a heap object; `i` must be within its slot count.
    fn get_slot(&self, obj: Value, i: usize) -> Result<Value, String>;

    /// Writes slot `i` of a heap object and fires the GC write barrier.
    fn set_slot(&mut self, obj: Value, i: usize, val: Value) -> Result<(), String>;

    /// Copies `len` bytes out of a raw-byte backing starting at `start`.
    fn read_bytes(&self, backing: Value, start: usize, len: usize) -> Result<Vec<u8>, String>;

    /// Copies `src` into a raw-byte backing at byte offset `start`.
    fn write_bytes(&mut self, backing: Value, start: usize, src: &[u8]) -> Result<(), String>;

    /// Allocates a heap `String` holding `bytes`, which must be valid UTF-8.
    fn alloc_string(&mut self, bytes: &[u8]) -> Result<Value, String>;

    /// A mutable deep copy of `obj`; a non-heap value is returned as is.
    fn clone_value(&mut self, obj: Value) -> Result<Value, String>;

    /// A new zeroed object of the struct `desc` (a field's embedded struct), which must outlive the object.
    fn alloc_struct(&mut self, desc: &TypeDescriptor) -> Result<Value, String>;

    /// Whether this call runs on the program's main thread.
    fn on_home_thread(&self) -> bool {
        true
    }
}

/// A heapless [`NativeCtx`] for testing natives.
#[derive(Default)]
pub struct FakeNativeCtx {
    platform: Option<Box<dyn crate::Platform>>,
}

impl FakeNativeCtx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_platform(platform: impl crate::Platform + 'static) -> Self {
        Self { platform: Some(Box::new(platform)) }
    }
}

fn no_heap<T>() -> Result<T, String> {
    Err("FakeNativeCtx has no heap".to_string())
}

impl NativeCtx for FakeNativeCtx {
    fn platform(&self) -> Result<&dyn crate::Platform, String> {
        self.platform.as_deref().ok_or_else(|| "no platform installed".to_string())
    }
    fn alloc_header(&mut self, _: u64, _: usize) -> Result<Value, String> { no_heap() }
    fn alloc_backing(&mut self, _: usize) -> Result<Value, String> { no_heap() }
    fn alloc_bytes_backing(&mut self, _: usize) -> Result<Value, String> { no_heap() }
    fn alloc_map_backing(&mut self, _: usize) -> Result<Value, String> { no_heap() }
    fn bytes_ptr(&self, _: Value) -> Result<*mut u8, String> { no_heap() }
    fn get_slot(&self, _: Value, _: usize) -> Result<Value, String> { no_heap() }
    fn set_slot(&mut self, _: Value, _: usize, _: Value) -> Result<(), String> { no_heap() }
    fn read_bytes(&self, _: Value, _: usize, _: usize) -> Result<Vec<u8>, String> { no_heap() }
    fn write_bytes(&mut self, _: Value, _: usize, _: &[u8]) -> Result<(), String> { no_heap() }
    fn alloc_string(&mut self, _: &[u8]) -> Result<Value, String> { no_heap() }
    fn clone_value(&mut self, _: Value) -> Result<Value, String> { no_heap() }
    fn alloc_struct(&mut self, _: &TypeDescriptor) -> Result<Value, String> { no_heap() }
}

impl std::fmt::Debug for NativeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeEntry").field("name", &self.name).field("arity", &self.arity).finish()
    }
}
