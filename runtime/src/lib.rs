//! Provides the functions that compiled code calls.

// The `extern "C"` helpers take pointers that only compiled code gives them.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod args;
mod capi;
mod channels;
mod collect;
mod entries;
#[cfg(feature = "executable")]
mod exe;
pub mod exec;
pub mod helpers;
pub mod loader;
mod locks;
mod ports;
mod report;
pub mod show;
mod state;
mod sys;
mod tasks;
mod walk;

pub use capi::{mote_runtime_attach, mote_runtime_free, mote_runtime_new, mote_runtime_on_fault, MoteFaultHandler, MoteFaultReport};
pub use channels::Failure;
pub use collect::{CodeMap, FunctionInfo, GlobalRoot, Located, Top};
pub use entries::address;
pub use mote_heap::{Config as HeapConfig, Stats as HeapStats};
pub use ports::{host_port_close, host_port_push, PortMode, PortShared};
pub use report::{backtrace, FaultReport, MAX_FRAMES};
pub use state::{FaultHook, Runtime};
pub use tasks::{ScopeEnd, TaskBody};
