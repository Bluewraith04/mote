//! The calls a C host makes to set a runtime up: create it from the byte forms of a program's tables, attach a context, and set the fault handler.

use std::sync::Arc;

use mote_abi::{FaultTable, TaskContext, TypeImage};
use mote_heap::Config;

use crate::report::FaultReport;
use crate::state::Runtime;

/// A fault as a C handler sees it; it is valid only during the call.
#[repr(C)]
pub struct MoteFaultReport {
    /// The position of the kind in `FaultKind::ALL`.
    pub kind: u32,
    /// The raw id of the fault site, or -1 when the fault has a message of its own.
    pub site: i64,
    /// The two ends of the span of the check, or -1 and -1.
    pub span_start: i64,
    pub span_end: i64,
    pub message: *const u8,
    pub message_len: usize,
    /// Return addresses, innermost first.
    pub frames: *const usize,
    pub frame_count: usize,
}

/// A C function that ends the task or the process; if it returns, the process aborts.
pub type MoteFaultHandler = extern "C" fn(*const MoteFaultReport);

/// A runtime for the type table and fault table in their byte forms and a heap limit in bytes (0 for the default); null when a table is refused.
///
/// # Safety
/// Each pointer is valid for its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_runtime_new(types: *const u8, types_len: usize, faults: *const u8, faults_len: usize, heap_limit: usize) -> *const Runtime {
    // SAFETY: the caller's promise.
    let (types, faults) = unsafe { (std::slice::from_raw_parts(types, types_len), std::slice::from_raw_parts(faults, faults_len)) };
    let (Ok(image), Ok(faults)) = (TypeImage::from_bytes(types), FaultTable::from_bytes(faults)) else { return std::ptr::null() };
    let defaults = Config::default();
    let config = Config { limit: if heap_limit == 0 { defaults.limit } else { heap_limit }, ..defaults };
    Arc::into_raw(Runtime::new(Arc::new(image.table), faults, config))
}

/// Frees a runtime made by `mote_runtime_new`; no context may be used afterwards.
///
/// # Safety
/// `runtime` came from `mote_runtime_new` and is freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_runtime_free(runtime: *const Runtime) {
    // SAFETY: the caller's promise.
    drop(unsafe { Arc::from_raw(runtime) });
}

/// Points the context at the runtime.
///
/// # Safety
/// Both pointers are live, and the runtime outlives the context's use.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_runtime_attach(runtime: *const Runtime, context: *mut TaskContext) {
    // SAFETY: the caller's promise; the pointer is not freed here.
    let runtime = unsafe { std::mem::ManuallyDrop::new(Arc::from_raw(runtime)) };
    // SAFETY: the same.
    runtime.attach(unsafe { &mut *context });
}

/// Sets what runs on a fault.
///
/// # Safety
/// `runtime` is live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_runtime_on_fault(runtime: *const Runtime, handler: MoteFaultHandler) {
    // SAFETY: the caller's promise.
    unsafe { &*runtime }.set_fault_hook(Arc::new(move |report: &FaultReport| {
        let at = |value: Option<u32>| value.map_or(-1, i64::from);
        let view = MoteFaultReport {
            kind: u32::from(report.kind.tag()),
            site: at(report.site),
            span_start: at(report.span.map(|span| span.0)),
            span_end: at(report.span.map(|span| span.1)),
            message: report.message.as_ptr(),
            message_len: report.message.len(),
            frames: report.backtrace.as_ptr(),
            frame_count: report.backtrace.len(),
        };
        handler(&view);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_a_runtime_is_refused_for_tables_that_do_not_read() {
        let bad = [1u8, 2, 3];
        // SAFETY: the pointers are valid for their lengths.
        let runtime = unsafe { mote_runtime_new(bad.as_ptr(), bad.len(), bad.as_ptr(), bad.len(), 0) };
        assert!(runtime.is_null());
    }
}
