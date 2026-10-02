//! Traits and plain types the components agree on; depends only on `isa`.

pub mod events;
/// The heap and mutator traits and collector statistics.
pub mod heap;
/// The slot layout trait.
pub mod layout;
pub mod natives;
pub mod platform;
/// The root enumeration interface the collector calls.
pub mod roots;
pub mod safepoint;
/// A location holding a `Value`.
pub mod slot;

pub use events::{event_queue, EventPayload, EventQueue, EventSink, Overflow, PushResult, SourceHandle, SourceRequest};
pub use heap::{Heap, HeapStats, Mutator, OutOfMemory, Released, RELEASE_FILE, RELEASE_GENERATOR, RELEASE_DATABASE, RELEASE_LIBRARY, RELEASE_SOCKET, RELEASE_TRANSACTION};
pub use layout::TypeLayout;
pub use natives::{
    FakeNativeCtx, NativeCtx, NativeEntry, NativeError, NativeFn, NativeId, NativeOutcome, NativeRegistry, PlatformContinuation,
    PlatformResult, SourceDecode,
};
pub use platform::{
    CArg, FileKind, FileMode,FileStat, Platform, PlatformError, ProcessOutput, PlatformErrorKind, PlatformRequest, PlatformResponse, SqlMode, SqlValue, StdStream, Wake, Whence, Woken,
};
pub use roots::RootSource;
pub use safepoint::{PausedRoot, SafepointCoordinator};
pub use slot::ValueSlot;
