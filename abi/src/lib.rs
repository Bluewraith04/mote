//! Defines the value layout and runtime contract shared by the compiler and the runtime.

mod bytes;
mod callconv;
mod code_table;
mod debug;
mod entry;
mod fault;
mod frame;
mod handle;
mod header;
mod host;
mod kind;
mod layout;
mod meta;
mod native;
mod object;
mod passing;
mod region;
#[cfg(test)]
mod sample;
mod scalar;
mod table;
mod table_bytes;

pub use bytes::{BytesError, Reader, Writer};
pub use mote_base::Span;
pub use callconv::{
    closure_signature, signature, Signature, TaskContext, CONTEXT, CONTEXT_FAULT_SITE, CONTEXT_FIELDS, CONTEXT_POLL, CONTEXT_REGION_CURSOR, CONTEXT_REGION_LIMIT,
    CONTEXT_RUNTIME, CONTEXT_SIZE, CONTEXT_STACK_LIMIT,
};
pub use code_table::CodeTable;
pub use debug::{DebugFunction, DebugTable, Place, NO_PLACE};
pub use entry::Entry;
pub use fault::{raw as fault_site_raw, FaultInfo, FaultKind, FaultSite, FaultTable};
pub use frame::FrameDescriptor;
pub use handle::{HandleKind, ONCE_DONE, ONCE_EMPTY, ONCE_RUNNING, TASK_CANCELLED, TASK_CONVERT_OFFSET, TASK_DONE, TASK_FAULTED, TASK_NUMBER_OFFSET, TASK_PENDING};
pub use header::{Header, FLAGS_RESERVED, FLAG_FINALIZE, FLAG_LARGE, FLAG_STATIC};
pub use host::{header as host_header, MoteHost, MotePort, PUSH_CLOSED, PUSH_FULL, PUSH_QUEUED, WAIT_FAILED, WAIT_READABLE, WAIT_READY, WAIT_TIMED_OUT, WAIT_WRITABLE};
pub use kind::{Element, Field, Kind, Object, PairPayload, Slot, TypeId, Variant};
pub use layout::{
    align_up, Layout, BOX_VALUE_OFFSET, CLOSURE_CAPTURES_OFFSET, CLOSURE_CODE_OFFSET, ELEMENTS_OFFSET, HEADER_SIZE, LENGTH_OFFSET,
    LIST_BUFFER_OFFSET, LIST_CAPACITY_OFFSET, PAIR_PAYLOAD_OFFSET, WORD,
};
pub use meta::{Carries, Described, Meta, Reflected, Shown};
pub use native::{CParam, CType, LinkImage, LinkTable, LinkedFn, NativeSig};
pub use object::{
    DIRECTORY_CODE, DIRECTORY_DEBUG, DIRECTORY_FAILURE, DIRECTORY_FAULTS, DIRECTORY_FLAGS, DIRECTORY_FLAG_SINGLE_THREADED, DIRECTORY_INITS,
    DIRECTORY_LINK, DIRECTORY_META, DIRECTORY_NATIVES, DIRECTORY_TYPES, DIRECTORY_WORDS, DIRECTORY_WORDS_OF_CODE,
};
pub use passing::{passing, Passing, MAX_PASSED_SCALARS};
pub use region::{inline_size, is_finalized, Region, MAX_INLINE_ALLOC};
pub use scalar::Scalar;
pub use table::{TypeInfo, TypeTable};
pub use table_bytes::TypeImage;
