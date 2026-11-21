//! The functions compiled code calls: one symbol for each `Entry`.
//!
//! Each symbol is a stub that passes the caller's frame pointer and return address on to a Rust function, which builds the backtrace from them.

use std::arch::global_asm;

use mote_abi::{Entry, FaultKind, HandleKind, Kind, TaskContext, TypeId, ELEMENTS_OFFSET, TASK_CANCELLED, TASK_PENDING};
use mote_heap::{Full, Object};

use crate::channels::Failure;
use crate::collect::Top;
use crate::report::{backtrace, FaultReport};
use crate::state::Runtime;
use crate::tasks::{ScopeEnd, TaskBody};

/// Defines a symbol that moves the frame pointer and return address into the two registers after the entry's parameters and jumps to `inner`.
#[cfg(target_os = "linux")]
macro_rules! stub {
    ($entry:literal => $inner:literal, $fp:literal, $pc:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            concat!("mov ", $fp, ", rbp"),
            concat!("mov ", $pc, ", [rsp]"),
            concat!("jmp ", $inner),
        );
    };
}

/// Like `stub!` for an entry whose parameters leave one register: the frame pointer goes there and the return address is the seventh argument, on the stack.
#[cfg(target_os = "linux")]
macro_rules! stub_stacked {
    ($entry:literal => $inner:literal, $fp:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            concat!("mov ", $fp, ", rbp"),
            "push qword ptr [rsp]",
            concat!("call ", $inner),
            "add rsp, 8",
            "ret",
        );
    };
}

#[cfg(target_os = "linux")]
mod stubs {
    use super::*;
    stub!("mote_fault" => "mote_entry_fault", "rdx", "rcx");
    stub!("mote_alloc" => "mote_entry_alloc", "rdx", "rcx");
    stub!("mote_alloc_array" => "mote_entry_alloc_array", "rcx", "r8");
    stub!("mote_fault_message" => "mote_entry_fault_message", "rdx", "rcx");
    stub!("mote_poll" => "mote_entry_poll", "rsi", "rdx");
    stub_stacked!("mote_task_spawn" => "mote_entry_task_spawn", "r9");
    stub!("mote_task_await" => "mote_entry_task_await", "rdx", "rcx");
    stub!("mote_task_cancel" => "mote_entry_task_cancel", "rdx", "rcx");
    stub!("mote_task_ready" => "mote_entry_task_ready", "rdx", "rcx");
    stub!("mote_task_message" => "mote_entry_task_message", "rdx", "rcx");
    stub!("mote_scope_begin" => "mote_entry_scope_begin", "rsi", "rdx");
    stub!("mote_scope_end" => "mote_entry_scope_end", "rdx", "rcx");
    stub!("mote_channel_new" => "mote_entry_channel_new", "rcx", "r8");
    stub!("mote_channel_end" => "mote_entry_channel_end", "rcx", "r8");
    stub!("mote_channel_send" => "mote_entry_channel_send", "r8", "r9");
    stub!("mote_channel_recv" => "mote_entry_channel_recv", "r8", "r9");
    stub!("mote_sender_close" => "mote_entry_sender_close", "rdx", "rcx");
    stub!("mote_sender_clone" => "mote_entry_sender_clone", "rdx", "rcx");
    stub!("mote_receiver_close" => "mote_entry_receiver_close", "rdx", "rcx");
    stub!("mote_receiver_clone" => "mote_entry_receiver_clone", "rdx", "rcx");
    stub!("mote_receiver_closed" => "mote_entry_receiver_closed", "rdx", "rcx");
    stub!("mote_mutex_new" => "mote_entry_mutex_new", "rdx", "rcx");
    stub!("mote_mutex_take" => "mote_entry_mutex_take", "rcx", "r8");
    stub!("mote_mutex_put" => "mote_entry_mutex_put", "rcx", "r8");
    stub!("mote_once_new" => "mote_entry_once_new", "rdx", "rcx");
    stub!("mote_once_begin" => "mote_entry_once_begin", "rdx", "rcx");
    stub!("mote_once_finish" => "mote_entry_once_finish", "rcx", "r8");
    stub!("mote_once_value" => "mote_entry_once_value", "rcx", "r8");
    stub!("mote_sleep" => "mote_entry_sleep", "rdx", "rcx");
    stub!("mote_task_await_timeout" => "mote_entry_task_await_timeout", "rcx", "r8");
    stub!("mote_channel_recv_timeout" => "mote_entry_channel_recv_timeout", "r8", "r9");
    stub!("mote_port_new" => "mote_entry_port_new", "r8", "r9");
    stub!("mote_port_ref" => "mote_entry_port_ref", "rdx", "rcx");
    stub!("mote_defer" => "mote_entry_defer", "rcx", "r8");
    stub!("mote_undefer" => "mote_entry_undefer", "rsi", "rdx");
    stub!("mote_scope_end_error" => "mote_entry_scope_end_error", "rdx", "rcx");
    stub!("mote_native_blocking" => "mote_entry_native_blocking", "r8", "r9");
}

#[cfg(target_os = "windows")]
macro_rules! stub_w64_1 {
    ($entry:literal => $inner:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            "mov rdx, rbp",
            "mov r8, [rsp]",
            concat!("jmp ", $inner),
        );
    };
}

#[cfg(target_os = "windows")]
macro_rules! stub_w64_2 {
    ($entry:literal => $inner:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            "mov r8, rbp",
            "mov r9, [rsp]",
            concat!("jmp ", $inner),
        );
    };
}

#[cfg(target_os = "windows")]
macro_rules! stub_w64_3 {
    ($entry:literal => $inner:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            "mov rax, [rsp]",
            "sub rsp, 40",
            "mov r9, rbp",
            "mov [rsp + 32], rax",
            concat!("call ", $inner),
            "add rsp, 40",
            "ret",
        );
    };
}

#[cfg(target_os = "windows")]
macro_rules! stub_w64_4 {
    ($entry:literal => $inner:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            "mov rax, [rsp]",
            "sub rsp, 56",
            "mov [rsp + 32], rbp",
            "mov [rsp + 40], rax",
            concat!("call ", $inner),
            "add rsp, 56",
            "ret",
        );
    };
}

#[cfg(target_os = "windows")]
macro_rules! stub_w64_5 {
    ($entry:literal => $inner:literal) => {
        global_asm!(
            ".text",
            ".p2align 4",
            concat!(".global ", $entry),
            concat!($entry, ":"),
            "sub rsp, 56",
            "mov rax, [rsp + 96]",
            "mov [rsp + 32], rax",
            "mov [rsp + 40], rbp",
            "mov rax, [rsp + 56]",
            "mov [rsp + 48], rax",
            concat!("call ", $inner),
            "add rsp, 56",
            "ret",
        );
    };
}

#[cfg(target_os = "windows")]
mod stubs {
    use super::*;
    stub_w64_1!("mote_poll" => "mote_entry_poll");
    stub_w64_1!("mote_scope_begin" => "mote_entry_scope_begin");
    stub_w64_1!("mote_undefer" => "mote_entry_undefer");

    stub_w64_2!("mote_fault" => "mote_entry_fault");
    stub_w64_2!("mote_alloc" => "mote_entry_alloc");
    stub_w64_2!("mote_fault_message" => "mote_entry_fault_message");
    stub_w64_2!("mote_task_await" => "mote_entry_task_await");
    stub_w64_2!("mote_task_cancel" => "mote_entry_task_cancel");
    stub_w64_2!("mote_task_ready" => "mote_entry_task_ready");
    stub_w64_2!("mote_task_message" => "mote_entry_task_message");
    stub_w64_2!("mote_scope_end" => "mote_entry_scope_end");
    stub_w64_2!("mote_sender_close" => "mote_entry_sender_close");
    stub_w64_2!("mote_sender_clone" => "mote_entry_sender_clone");
    stub_w64_2!("mote_receiver_close" => "mote_entry_receiver_close");
    stub_w64_2!("mote_receiver_clone" => "mote_entry_receiver_clone");
    stub_w64_2!("mote_receiver_closed" => "mote_entry_receiver_closed");
    stub_w64_2!("mote_mutex_new" => "mote_entry_mutex_new");
    stub_w64_2!("mote_once_new" => "mote_entry_once_new");
    stub_w64_2!("mote_once_begin" => "mote_entry_once_begin");
    stub_w64_2!("mote_sleep" => "mote_entry_sleep");
    stub_w64_2!("mote_port_ref" => "mote_entry_port_ref");
    stub_w64_2!("mote_scope_end_error" => "mote_entry_scope_end_error");

    stub_w64_3!("mote_alloc_array" => "mote_entry_alloc_array");
    stub_w64_3!("mote_channel_new" => "mote_entry_channel_new");
    stub_w64_3!("mote_channel_end" => "mote_entry_channel_end");
    stub_w64_3!("mote_mutex_take" => "mote_entry_mutex_take");
    stub_w64_3!("mote_mutex_put" => "mote_entry_mutex_put");
    stub_w64_3!("mote_once_finish" => "mote_entry_once_finish");
    stub_w64_3!("mote_once_value" => "mote_entry_once_value");
    stub_w64_3!("mote_task_await_timeout" => "mote_entry_task_await_timeout");
    stub_w64_3!("mote_defer" => "mote_entry_defer");

    stub_w64_4!("mote_channel_send" => "mote_entry_channel_send");
    stub_w64_4!("mote_channel_recv" => "mote_entry_channel_recv");
    stub_w64_4!("mote_channel_recv_timeout" => "mote_entry_channel_recv_timeout");
    stub_w64_4!("mote_port_new" => "mote_entry_port_new");
    stub_w64_4!("mote_native_blocking" => "mote_entry_native_blocking");

    stub_w64_5!("mote_task_spawn" => "mote_entry_task_spawn");
}

/// The symbols as compiled code sees them.
pub(crate) mod symbols {
    use super::*;

    unsafe extern "C" {
        pub fn mote_fault(context: *mut TaskContext, site: u32) -> !;
        pub fn mote_alloc(context: *mut TaskContext, ty: u32) -> usize;
        pub fn mote_alloc_array(context: *mut TaskContext, ty: u32, count: i64) -> usize;
        pub fn mote_fault_message(context: *mut TaskContext, message: usize) -> !;
        pub fn mote_poll(context: *mut TaskContext);
        pub fn mote_task_spawn(context: *mut TaskContext, ty: u32, thunk: usize, closure: usize, scope: i64) -> usize;
        pub fn mote_task_await(context: *mut TaskContext, handle: usize) -> u64;
        pub fn mote_task_cancel(context: *mut TaskContext, handle: usize);
        pub fn mote_task_ready(context: *mut TaskContext, handle: usize) -> u64;
        pub fn mote_task_message(context: *mut TaskContext, handle: usize) -> usize;
        pub fn mote_scope_begin(context: *mut TaskContext) -> u64;
        pub fn mote_scope_end(context: *mut TaskContext, token: u64);
        pub fn mote_channel_new(context: *mut TaskContext, ty: u32, capacity: i64) -> usize;
        pub fn mote_channel_end(context: *mut TaskContext, channel: usize, ty: u32) -> usize;
        pub fn mote_channel_send(context: *mut TaskContext, tx: usize, source: usize, wait: u64) -> u64;
        pub fn mote_channel_recv(context: *mut TaskContext, rx: usize, dest: usize, wait: u64) -> u64;
        pub fn mote_sender_close(context: *mut TaskContext, tx: usize);
        pub fn mote_sender_clone(context: *mut TaskContext, tx: usize) -> usize;
        pub fn mote_receiver_close(context: *mut TaskContext, rx: usize);
        pub fn mote_receiver_clone(context: *mut TaskContext, rx: usize) -> usize;
        pub fn mote_receiver_closed(context: *mut TaskContext, rx: usize) -> u64;
        pub fn mote_mutex_new(context: *mut TaskContext, ty: u32) -> usize;
        pub fn mote_mutex_take(context: *mut TaskContext, mutex: usize, dest: usize);
        pub fn mote_mutex_put(context: *mut TaskContext, mutex: usize, source: usize);
        pub fn mote_once_new(context: *mut TaskContext, ty: u32) -> usize;
        pub fn mote_once_begin(context: *mut TaskContext, once: usize) -> u64;
        pub fn mote_once_finish(context: *mut TaskContext, once: usize, source: usize);
        pub fn mote_once_value(context: *mut TaskContext, once: usize, dest: usize);
        pub fn mote_sleep(context: *mut TaskContext, ms: i64);
        pub fn mote_task_await_timeout(context: *mut TaskContext, handle: usize, ms: i64) -> u64;
        pub fn mote_channel_recv_timeout(context: *mut TaskContext, rx: usize, dest: usize, ms: i64) -> u64;
        pub fn mote_port_new(context: *mut TaskContext, channel: usize, ty: u32, queue: i64) -> usize;
        pub fn mote_port_ref(context: *mut TaskContext, port: usize) -> usize;
        pub fn mote_defer(context: *mut TaskContext, resource: usize, thunk: usize);
        pub fn mote_undefer(context: *mut TaskContext);
        pub fn mote_scope_end_error(context: *mut TaskContext, token: u64) -> usize;
        pub fn mote_native_blocking(context: *mut TaskContext, thunk: usize, buffer: usize, words: i64);
    }
}

/// The address of the symbol of an entry, for a JIT to bind.
pub fn address(entry: Entry) -> usize {
    use symbols::*;
    match entry {
        Entry::Fault => mote_fault as *const () as usize,
        Entry::Alloc => mote_alloc as *const () as usize,
        Entry::AllocArray => mote_alloc_array as *const () as usize,
        Entry::FaultMessage => mote_fault_message as *const () as usize,
        Entry::Poll => mote_poll as *const () as usize,
        Entry::TaskSpawn => mote_task_spawn as *const () as usize,
        Entry::TaskAwait => mote_task_await as *const () as usize,
        Entry::TaskCancel => mote_task_cancel as *const () as usize,
        Entry::TaskReady => mote_task_ready as *const () as usize,
        Entry::TaskMessage => mote_task_message as *const () as usize,
        Entry::ScopeBegin => mote_scope_begin as *const () as usize,
        Entry::ScopeEnd => mote_scope_end as *const () as usize,
        Entry::ChannelNew => mote_channel_new as *const () as usize,
        Entry::ChannelEnd => mote_channel_end as *const () as usize,
        Entry::ChannelSend => mote_channel_send as *const () as usize,
        Entry::ChannelRecv => mote_channel_recv as *const () as usize,
        Entry::SenderClose => mote_sender_close as *const () as usize,
        Entry::SenderClone => mote_sender_clone as *const () as usize,
        Entry::ReceiverClose => mote_receiver_close as *const () as usize,
        Entry::ReceiverClone => mote_receiver_clone as *const () as usize,
        Entry::ReceiverClosed => mote_receiver_closed as *const () as usize,
        Entry::MutexNew => mote_mutex_new as *const () as usize,
        Entry::MutexTake => mote_mutex_take as *const () as usize,
        Entry::MutexPut => mote_mutex_put as *const () as usize,
        Entry::OnceNew => mote_once_new as *const () as usize,
        Entry::OnceBegin => mote_once_begin as *const () as usize,
        Entry::OnceFinish => mote_once_finish as *const () as usize,
        Entry::OnceValue => mote_once_value as *const () as usize,
        Entry::Sleep => mote_sleep as *const () as usize,
        Entry::TaskAwaitTimeout => mote_task_await_timeout as *const () as usize,
        Entry::ChannelRecvTimeout => mote_channel_recv_timeout as *const () as usize,
        Entry::PortNew => mote_port_new as *const () as usize,
        Entry::PortRef => mote_port_ref as *const () as usize,
        Entry::Defer => mote_defer as *const () as usize,
        Entry::Undefer => mote_undefer as *const () as usize,
        Entry::ScopeEndError => mote_scope_end_error as *const () as usize,
        Entry::NativeBlocking => mote_native_blocking as *const () as usize,
    }
}

fn report(kind: FaultKind, message: String, site: Option<u32>, span: Option<(u32, u32)>, fp: usize, pc: usize) -> FaultReport {
    FaultReport { kind, message, site, span, backtrace: backtrace(fp, pc) }
}

/// Stops the program for a call the compiled code should not make.
fn misuse(runtime: &Runtime, context: *mut TaskContext, message: String, fp: usize, pc: usize) -> ! {
    // SAFETY: the context is live, as for every entry.
    runtime.stop(unsafe { &mut *context }, report(FaultKind::Raised, message, None, None, fp, pc))
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_fault(context: *mut TaskContext, site: u32, fp: usize, pc: usize) -> ! {
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    let runtime = unsafe { Runtime::of(context) };
    let info = runtime.faults().get_raw(site);
    // A cancelled task has its stack limit poisoned, so its next prologue arrives here.
    if info.is_some_and(|info| info.kind == FaultKind::StackDepth) && runtime.is_cancelled_task(context) {
        runtime.end_task(context, TASK_CANCELLED, None);
    }
    let report = match info {
        Some(info) => report(info.kind, info.kind.message().to_string(), Some(site), Some((info.span.start, info.span.end)), fp, pc),
        None => report(FaultKind::Raised, format!("fault site {site} is not in the table"), None, None, fp, pc),
    };
    // SAFETY: as above.
    runtime.stop(unsafe { &mut *context }, report)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_fault_message(context: *mut TaskContext, message: usize, fp: usize, pc: usize) -> ! {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let text = string_text(runtime, message);
    // SAFETY: as above.
    runtime.stop(unsafe { &mut *context }, report(FaultKind::Raised, text, None, None, fp, pc))
}

/// The text of a string object; a null reference is empty and a reference to anything else is named as such.
fn string_text(runtime: &Runtime, reference: usize) -> String {
    let Some(object) = Object::from_address(reference) else { return String::new() };
    let is_string = runtime.heap().table().get_raw(object.header().type_id().raw()).is_some_and(|info| info.kind == Kind::String);
    if !is_string {
        return "(the message is not a string)".to_string();
    }
    // SAFETY: a string object holds `length` bytes of UTF-8 from `ELEMENTS_OFFSET`.
    let bytes = unsafe { std::slice::from_raw_parts((object.address() + ELEMENTS_OFFSET as usize) as *const u8, object.length() as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_alloc(context: *mut TaskContext, ty: u32, fp: usize, pc: usize) -> usize {
    allocate(context, ty, 0, false, fp, pc)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_alloc_array(context: *mut TaskContext, ty: u32, count: i64, fp: usize, pc: usize) -> usize {
    allocate(context, ty, count as u64, true, fp, pc)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_poll(context: *mut TaskContext, fp: usize, pc: usize) {
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    runtime.poll(unsafe { &mut *context }, Top { fp, pc });
}

/// The `Task` object a reference stands for; the program stops when it is anything else.
fn task_object(runtime: &Runtime, context: *mut TaskContext, handle: usize, fp: usize, pc: usize) -> Object {
    let found = Object::from_address(handle).filter(|object| runtime.heap().table().get_raw(object.header().type_id().raw()).is_some_and(|info| matches!(info.kind, Kind::Handle(HandleKind::Task, _))));
    match found {
        Some(object) => object,
        None => misuse(runtime, context, "a task call on a value that is not a task".to_string(), fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_task_spawn(context: *mut TaskContext, ty: u32, thunk: usize, closure: usize, scope: i64, fp: usize, pc: usize) -> usize {
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    let runtime = unsafe { Runtime::of(context) };
    if !runtime.heap().table().get_raw(ty).is_some_and(|info| matches!(info.kind, Kind::Handle(HandleKind::Task, _))) {
        misuse(runtime, context, format!("a task of type {ty}, which is not a task type"), fp, pc);
    }
    // SAFETY: compiled code passes the address of the function it made for the task type, which has the signature of a `TaskBody`.
    let body: TaskBody = unsafe { std::mem::transmute::<usize, TaskBody>(thunk) };
    // SAFETY: as above.
    let context = unsafe { &mut *context };
    match runtime.spawn(context, TypeId::from_raw(ty), body, closure, u64::try_from(scope).ok()) {
        Ok(handle) => handle.address(),
        Err(Full) => runtime.stop(context, report(FaultKind::MemoryLimit, FaultKind::MemoryLimit.message().to_string(), None, None, fp, pc)),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_task_await(context: *mut TaskContext, handle: usize, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let task = task_object(runtime, context, handle, fp, pc);
    // SAFETY: as above.
    let state = runtime.await_task(unsafe { &mut *context }, Top { fp, pc }, task);
    if state == TASK_PENDING {
        runtime.end_task(context, TASK_CANCELLED, None);
    }
    state
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_task_cancel(context: *mut TaskContext, handle: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    runtime.cancel_task(task_object(runtime, context, handle, fp, pc));
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_task_ready(context: *mut TaskContext, handle: usize, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    u64::from(runtime.task_is_ready(task_object(runtime, context, handle, fp, pc)))
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_task_message(context: *mut TaskContext, handle: usize, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let text = runtime.task_message(task_object(runtime, context, handle, fp, pc));
    // SAFETY: as above.
    let context = unsafe { &mut *context };
    match runtime.alloc_string(context, &text) {
        Ok(string) => string.address(),
        Err(Full) => runtime.stop(context, report(FaultKind::MemoryLimit, FaultKind::MemoryLimit.message().to_string(), None, None, fp, pc)),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_scope_begin(context: *mut TaskContext, _fp: usize, _pc: usize) -> u64 {
    // SAFETY: as above.
    unsafe { Runtime::of(context) }.scope_begin(context)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_scope_end_error(context: *mut TaskContext, token: u64, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    match runtime.scope_end(unsafe { &mut *context }, Top { fp, pc }, token) {
        ScopeEnd::Clean => 0,
        ScopeEnd::Faulted(fault) => {
            // SAFETY: as above.
            let context = unsafe { &mut *context };
            match runtime.alloc_string(context, &fault.message) {
                Ok(string) => string.address(),
                Err(Full) => runtime.stop(context, report(FaultKind::MemoryLimit, FaultKind::MemoryLimit.message().to_string(), None, None, fp, pc)),
            }
        }
        ScopeEnd::Cancelled => runtime.end_task(context, TASK_CANCELLED, None),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_native_blocking(context: *mut TaskContext, thunk: usize, buffer: usize, words: i64, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    runtime.blocked(context, Top { fp, pc }, || crate::loader::call_blocking(thunk, buffer, words as usize));
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_defer(context: *mut TaskContext, resource: usize, thunk: usize, _fp: usize, _pc: usize) {
    // SAFETY: as above.
    unsafe { Runtime::of(context) }.defer(context, resource, thunk);
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_undefer(context: *mut TaskContext, _fp: usize, _pc: usize) {
    // SAFETY: as above.
    unsafe { Runtime::of(context) }.undefer(context);
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_scope_end(context: *mut TaskContext, token: u64, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    match runtime.scope_end(unsafe { &mut *context }, Top { fp, pc }, token) {
        ScopeEnd::Clean => {}
        // SAFETY: as above.
        ScopeEnd::Faulted(fault) => runtime.stop(unsafe { &mut *context }, fault),
        ScopeEnd::Cancelled => runtime.end_task(context, TASK_CANCELLED, None),
    }
}

/// The handle object a reference stands for when it is of the kind; the program stops when it is anything else.
fn handle_object(runtime: &Runtime, context: *mut TaskContext, reference: usize, wanted: HandleKind, fp: usize, pc: usize) -> Object {
    let found = Object::from_address(reference).filter(|object| runtime.heap().table().get_raw(object.header().type_id().raw()).is_some_and(|info| matches!(&info.kind, Kind::Handle(kind, _) if *kind == wanted)));
    match found {
        Some(object) => object,
        None => {
            let call = match wanted {
                HandleKind::Mutex => "a mutex",
                HandleKind::Once => "a once",
                HandleKind::Port => "a port",
                _ => "a channel",
            };
            misuse(runtime, context, format!("{call} call on a value that is not a {wanted:?}"), fp, pc)
        }
    }
}

/// Stops the program for a channel call that failed.
fn channel_failed(runtime: &Runtime, context: *mut TaskContext, failure: Failure, fp: usize, pc: usize) -> ! {
    let report = match failure {
        Failure::Raised(message) => report(FaultKind::Raised, message.to_string(), None, None, fp, pc),
        Failure::Full => report(FaultKind::MemoryLimit, FaultKind::MemoryLimit.message().to_string(), None, None, fp, pc),
    };
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    runtime.stop(unsafe { &mut *context }, report)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_channel_new(context: *mut TaskContext, ty: u32, capacity: i64, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    match runtime.channel_new(unsafe { &mut *context }, TypeId::from_raw(ty), capacity) {
        Ok(channel) => channel.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_channel_end(context: *mut TaskContext, channel: usize, ty: u32, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let channel = handle_object(runtime, context, channel, HandleKind::Channel, fp, pc);
    // SAFETY: as above.
    match runtime.channel_end(unsafe { &mut *context }, channel, TypeId::from_raw(ty)) {
        Ok(end) => end.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_channel_send(context: *mut TaskContext, tx: usize, source: usize, wait: u64, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let tx = handle_object(runtime, context, tx, HandleKind::Sender, fp, pc);
    // SAFETY: as above.
    match runtime.channel_send(unsafe { &mut *context }, Top { fp, pc }, tx, source, wait != 0) {
        Ok(sent) => u64::from(sent),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_channel_recv(context: *mut TaskContext, rx: usize, dest: usize, wait: u64, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let rx = handle_object(runtime, context, rx, HandleKind::Receiver, fp, pc);
    // SAFETY: as above.
    runtime.channel_recv(unsafe { &mut *context }, Top { fp, pc }, rx, dest, wait != 0)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_sender_close(context: *mut TaskContext, tx: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    runtime.sender_close(handle_object(runtime, context, tx, HandleKind::Sender, fp, pc));
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_sender_clone(context: *mut TaskContext, tx: usize, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let tx = handle_object(runtime, context, tx, HandleKind::Sender, fp, pc);
    // SAFETY: as above.
    match runtime.sender_clone(unsafe { &mut *context }, tx) {
        Ok(end) => end.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_receiver_close(context: *mut TaskContext, rx: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    runtime.receiver_close(handle_object(runtime, context, rx, HandleKind::Receiver, fp, pc));
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_receiver_clone(context: *mut TaskContext, rx: usize, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let rx = handle_object(runtime, context, rx, HandleKind::Receiver, fp, pc);
    // SAFETY: as above.
    match runtime.receiver_clone(unsafe { &mut *context }, rx) {
        Ok(end) => end.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_receiver_closed(context: *mut TaskContext, rx: usize, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    u64::from(runtime.receiver_closed(handle_object(runtime, context, rx, HandleKind::Receiver, fp, pc)))
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_mutex_new(context: *mut TaskContext, ty: u32, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    match runtime.mutex_new(unsafe { &mut *context }, TypeId::from_raw(ty)) {
        Ok(mutex) => mutex.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_mutex_take(context: *mut TaskContext, mutex: usize, dest: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let mutex = handle_object(runtime, context, mutex, HandleKind::Mutex, fp, pc);
    // SAFETY: as above.
    if let Err(failure) = runtime.mutex_take(unsafe { &mut *context }, Top { fp, pc }, mutex, dest) {
        channel_failed(runtime, context, failure, fp, pc);
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_mutex_put(context: *mut TaskContext, mutex: usize, source: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let mutex = handle_object(runtime, context, mutex, HandleKind::Mutex, fp, pc);
    // SAFETY: as above.
    if let Err(failure) = runtime.mutex_put(unsafe { &mut *context }, mutex, source) {
        channel_failed(runtime, context, failure, fp, pc);
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_once_new(context: *mut TaskContext, ty: u32, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    match runtime.once_new(unsafe { &mut *context }, TypeId::from_raw(ty)) {
        Ok(once) => once.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_once_begin(context: *mut TaskContext, once: usize, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let once = handle_object(runtime, context, once, HandleKind::Once, fp, pc);
    // SAFETY: as above.
    match runtime.once_begin(unsafe { &mut *context }, Top { fp, pc }, once) {
        Ok(run) => u64::from(run),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_once_finish(context: *mut TaskContext, once: usize, source: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    runtime.once_finish(handle_object(runtime, context, once, HandleKind::Once, fp, pc), source);
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_once_value(context: *mut TaskContext, once: usize, dest: usize, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let once = handle_object(runtime, context, once, HandleKind::Once, fp, pc);
    if let Err(failure) = runtime.once_value(once, dest) {
        channel_failed(runtime, context, failure, fp, pc);
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_sleep(context: *mut TaskContext, ms: i64, fp: usize, pc: usize) {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    // SAFETY: as above.
    runtime.sleep_ms(unsafe { &mut *context }, Top { fp, pc }, ms);
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_task_await_timeout(context: *mut TaskContext, handle: usize, ms: i64, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let task = task_object(runtime, context, handle, fp, pc);
    // SAFETY: as above.
    runtime.await_task_timeout(unsafe { &mut *context }, Top { fp, pc }, task, ms)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_channel_recv_timeout(context: *mut TaskContext, rx: usize, dest: usize, ms: i64, fp: usize, pc: usize) -> u64 {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let rx = handle_object(runtime, context, rx, HandleKind::Receiver, fp, pc);
    // SAFETY: as above.
    runtime.channel_recv_timeout(unsafe { &mut *context }, Top { fp, pc }, rx, dest, ms)
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_port_new(context: *mut TaskContext, channel: usize, ty: u32, queue: i64, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let channel = handle_object(runtime, context, channel, HandleKind::Channel, fp, pc);
    // SAFETY: as above.
    match runtime.port_new(unsafe { &mut *context }, channel, TypeId::from_raw(ty), queue) {
        Ok(port) => port.address(),
        Err(failure) => channel_failed(runtime, context, failure, fp, pc),
    }
}

#[unsafe(no_mangle)]
extern "C" fn mote_entry_port_ref(context: *mut TaskContext, port: usize, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    runtime.port_ref(handle_object(runtime, context, port, HandleKind::Port, fp, pc))
}

fn allocate(context: *mut TaskContext, ty: u32, count: u64, array: bool, fp: usize, pc: usize) -> usize {
    // SAFETY: as above.
    let runtime = unsafe { Runtime::of(context) };
    let shape = runtime.heap().table().get_raw(ty).and_then(|info| info.object.as_ref());
    match shape {
        None => misuse(runtime, context, format!("allocation of type {ty}, which is not an object type"), fp, pc),
        Some(shape) if shape.element.is_some() != array => {
            let wanted = if array { "mote_alloc" } else { "mote_alloc_array" };
            misuse(runtime, context, format!("allocation of type {ty} by the wrong entry; it is the job of {wanted}"), fp, pc)
        }
        Some(_) => {}
    }
    // SAFETY: as above.
    let context = unsafe { &mut *context };
    match runtime.alloc(context, TypeId::from_raw(ty), count) {
        Ok(object) => object.address(),
        Err(Full) => runtime.stop(context, report(FaultKind::MemoryLimit, FaultKind::MemoryLimit.message().to_string(), None, None, fp, pc)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mote_abi::{FaultTable, Header, Scalar, Span, TypeTable, LENGTH_OFFSET};
    use mote_heap::Config;

    use super::symbols::*;
    use super::*;

    struct Types {
        point: TypeId,
        text: TypeId,
        ints: TypeId,
        channel: TypeId,
        sender: TypeId,
        receiver: TypeId,
        mutex: TypeId,
        once: TypeId,
        port: TypeId,
    }

    fn types() -> (Arc<TypeTable>, Types) {
        let mut table = TypeTable::new();
        let int = table.add("Int", Kind::Scalar(Scalar::I64));
        let point = table.add("Point", Kind::Class(vec![int, int]));
        let text = table.add("String", Kind::String);
        let ints = table.add("[Int]", Kind::Array(int));
        let channel = table.add("Channel<Int>", Kind::Handle(HandleKind::Channel, vec![ints]));
        let sender = table.add("Sender<Int>", Kind::Handle(HandleKind::Sender, vec![]));
        let receiver = table.add("Receiver<Int>", Kind::Handle(HandleKind::Receiver, vec![]));
        let mutex = table.add("Mutex<Int>", Kind::Handle(HandleKind::Mutex, vec![int]));
        let once = table.add("Once<Int>", Kind::Handle(HandleKind::Once, vec![int]));
        let port = table.add("PortEnd", Kind::Handle(HandleKind::Port, vec![]));
        (Arc::new(table), Types { point, text, ints, channel, sender, receiver, mutex, once, port })
    }

    fn faults() -> FaultTable {
        let mut faults = FaultTable::new();
        faults.add(FaultKind::DivideByZero, Span::new(10, 15));
        faults.add(FaultKind::IndexOutOfRange, Span::new(20, 30));
        faults
    }

    /// A runtime and a context attached to it; the context is leaked so a stopped thread can keep it.
    fn start(limit: usize) -> (Arc<Runtime>, &'static mut TaskContext, Types) {
        let (table, ty) = types();
        let runtime = Runtime::new(table, faults(), Config { limit, ..Config::default() });
        let context = Box::leak(Box::new(TaskContext::default()));
        runtime.attach(context);
        (runtime, context, ty)
    }

    /// Runs `call` on a thread; the thread ends at the fault hook, which hands the report over and never returns.
    fn stopped_by(runtime: &Arc<Runtime>, context: &'static mut TaskContext, call: impl FnOnce(*mut TaskContext) + Send + 'static) -> FaultReport {
        let (send, receive) = channel();
        let send = Mutex::new(send);
        runtime.set_fault_hook(Arc::new(move |report| {
            send.lock().unwrap().send(report.clone()).unwrap();
            loop {
                std::thread::park();
            }
        }));
        let pointer = context as *mut TaskContext as usize;
        std::thread::spawn(move || {
            call(pointer as *mut TaskContext);
            panic!("the call came back");
        });
        match receive.recv_timeout(Duration::from_secs(10)) {
            Ok(report) => report,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => panic!("the program was not stopped"),
        }
    }

    #[test]
    fn entries_alloc_returns_a_zeroed_object_with_its_header_and_moves_the_region() {
        let (_runtime, context, ty) = start(1 << 30);
        // SAFETY: the context is attached to a live runtime.
        let first = unsafe { mote_alloc(context, ty.point.raw()) };
        // SAFETY: as above.
        let second = unsafe { mote_alloc(context, ty.point.raw()) };
        let object = Object::from_address(first).unwrap();
        assert_eq!((object.header(), object.word(8), object.word(16)), (Header::new(ty.point, 0), 0, 0));
        assert_eq!(second, first + 24);
        assert_eq!(context.region_cursor, second as u64 + 24);
    }

    #[test]
    fn entries_alloc_array_sets_the_length_word_of_a_string_or_an_array() {
        let (_runtime, context, ty) = start(1 << 30);
        // SAFETY: the context is attached to a live runtime.
        let (text, ints) = unsafe { (mote_alloc_array(context, ty.text.raw(), 5), mote_alloc_array(context, ty.ints.raw(), 3)) };
        let (text, ints) = (Object::from_address(text).unwrap(), Object::from_address(ints).unwrap());
        assert_eq!((text.header().type_id(), text.word(LENGTH_OFFSET)), (ty.text, 5));
        assert_eq!((ints.header().type_id(), ints.length(), ints.word(ELEMENTS_OFFSET)), (ty.ints, 3, 0));
    }

    #[test]
    fn entries_a_heap_without_room_stops_the_program_with_the_memory_limit() {
        let (runtime, context, ty) = start(1 << 20);
        let report = stopped_by(&runtime, context, move |context| {
            // SAFETY: the context is attached to a live runtime.
            unsafe { mote_alloc_array(context, ty.ints.raw(), 1 << 30) };
        });
        assert_eq!((report.kind, report.message.as_str(), report.site), (FaultKind::MemoryLimit, "memory limit reached", None));
    }

    #[test]
    fn entries_a_negative_count_is_too_large_to_hold_and_stops_the_program() {
        let (runtime, context, ty) = start(1 << 30);
        let report = stopped_by(&runtime, context, move |context| {
            // SAFETY: as above.
            unsafe { mote_alloc_array(context, ty.ints.raw(), -1) };
        });
        assert_eq!(report.kind, FaultKind::MemoryLimit);
    }

    #[test]
    fn entries_fault_reports_the_kind_message_site_and_span_and_marks_the_context() {
        let (runtime, context, _) = start(1 << 30);
        let seen = context as *mut TaskContext as usize;
        let report = stopped_by(&runtime, context, |context| {
            // SAFETY: as above.
            unsafe { mote_fault(context, 1) };
        });
        assert_eq!((report.kind, report.message.as_str(), report.site, report.span), (FaultKind::IndexOutOfRange, "index out of range", Some(1), Some((20, 30))));
        // SAFETY: the stopped thread is parked and the context was leaked.
        assert_eq!(unsafe { (*(seen as *const TaskContext)).fault_site }, 2);
    }

    #[test]
    fn entries_fault_message_reports_the_text_of_the_string_as_a_raised_fault() {
        let (runtime, context, ty) = start(1 << 30);
        // SAFETY: as above.
        let text = unsafe { mote_alloc_array(context, ty.text.raw(), 7) };
        // SAFETY: the object has 7 bytes of room after its length word.
        unsafe { std::ptr::copy_nonoverlapping(b"no room".as_ptr(), (text + ELEMENTS_OFFSET as usize) as *mut u8, 7) };
        let seen = context as *mut TaskContext as usize;
        let report = stopped_by(&runtime, context, move |context| {
            // SAFETY: as above.
            unsafe { mote_fault_message(context, text) };
        });
        assert_eq!((report.kind, report.message.as_str(), report.site), (FaultKind::Raised, "no room", None));
        // SAFETY: as above.
        assert_eq!(unsafe { (*(seen as *const TaskContext)).fault_site }, 0);
    }

    #[test]
    fn entries_a_message_that_is_null_is_empty_and_one_that_is_not_a_string_is_named() {
        let (runtime, context, _) = start(1 << 30);
        let report = stopped_by(&runtime, context, |context| {
            // SAFETY: as above.
            unsafe { mote_fault_message(context, 0) };
        });
        assert_eq!(report.message, "");
        let (runtime, context, ty) = start(1 << 30);
        // SAFETY: as above.
        let point = unsafe { mote_alloc(context, ty.point.raw()) };
        let report = stopped_by(&runtime, context, move |context| {
            // SAFETY: as above.
            unsafe { mote_fault_message(context, point) };
        });
        assert_eq!(report.message, "(the message is not a string)");
    }

    #[test]
    fn entries_a_site_that_is_not_in_the_table_and_a_wrong_type_stop_the_program_with_a_message() {
        let (runtime, context, ty) = start(1 << 30);
        let report = stopped_by(&runtime, context, |context| {
            // SAFETY: as above.
            unsafe { mote_fault(context, 99) };
        });
        assert_eq!((report.kind, report.message.as_str()), (FaultKind::Raised, "fault site 99 is not in the table"));
        for (call, text) in [
            (0u8, "allocation of type 77, which is not an object type".to_string()),
            (1, format!("allocation of type {} by the wrong entry; it is the job of mote_alloc_array", ty.text.raw())),
            (2, format!("allocation of type {} by the wrong entry; it is the job of mote_alloc", ty.point.raw())),
        ] {
            let (runtime, context, ty) = start(1 << 30);
            let report = stopped_by(&runtime, context, move |context| {
                // SAFETY: as above.
                unsafe {
                    match call {
                        0 => mote_alloc(context, 77),
                        1 => mote_alloc(context, ty.text.raw()),
                        _ => mote_alloc_array(context, ty.point.raw(), 1),
                    }
                };
            });
            assert_eq!((report.kind, report.message), (FaultKind::Raised, text));
        }
    }

    #[inline(never)]
    fn call_fault(context: *mut TaskContext) -> ! {
        // SAFETY: as above.
        unsafe { mote_fault(context, 0) }
    }

    #[test]
    fn entries_the_backtrace_starts_where_the_entry_was_called() {
        let (runtime, context, _) = start(1 << 30);
        let report = stopped_by(&runtime, context, |context| call_fault(context));
        let start = call_fault as *const () as usize;
        assert!((1..512).contains(&report.backtrace[0].wrapping_sub(start)), "{:#x} against {start:#x}", report.backtrace[0]);
    }

    #[test]
    fn entries_every_entry_of_the_abi_has_a_distinct_symbol() {
        let addresses: std::collections::HashSet<usize> = Entry::ALL.iter().map(|entry| address(*entry)).collect();
        assert_eq!(addresses.len(), Entry::ALL.len());
        assert!(!addresses.contains(&0));
    }

    #[test]
    fn entries_channels_make_ends_and_pass_a_message_through_their_symbols() {
        let (_runtime, context, ty) = start(1 << 30);
        let message = 11i64;
        let (mut got, mut none) = (0i64, 0i64);
        // SAFETY: the context is attached to a live runtime and the addresses are of live integers.
        let (sent, taken, closed_with_room, again, refused_try, empty_try, closed) = unsafe {
            let channel = mote_channel_new(context, ty.channel.raw(), 2);
            let tx = mote_channel_end(context, channel, ty.sender.raw());
            let rx = mote_channel_end(context, channel, ty.receiver.raw());
            let sent = mote_channel_send(context, tx, &message as *const i64 as usize, 1);
            let taken = mote_channel_recv(context, rx, &mut got as *mut i64 as usize, 1);
            let closed_with_room = mote_receiver_closed(context, rx);
            let other = mote_sender_clone(context, tx);
            mote_sender_close(context, tx);
            let again = mote_channel_send(context, other, &message as *const i64 as usize, 0);
            mote_sender_close(context, other);
            let refused_try = mote_channel_send(context, other, &message as *const i64 as usize, 0);
            let second = mote_channel_recv(context, rx, &mut none as *mut i64 as usize, 0);
            let empty_try = mote_channel_recv(context, rx, &mut none as *mut i64 as usize, 0);
            let more = mote_receiver_clone(context, rx);
            mote_receiver_close(context, more);
            (sent, (taken, got), closed_with_room, (again, second), refused_try, empty_try, mote_receiver_closed(context, rx))
        };
        assert_eq!((sent, taken, closed_with_room, again, refused_try, empty_try, closed), (1, (1, 11), 0, (1, 1), 0, 0, 1));
    }

    #[test]
    fn entries_locks_make_a_mutex_and_a_once_and_pass_their_values_through_their_symbols() {
        let (_runtime, context, ty) = start(1 << 30);
        let (mut value, mut held, mut again) = (3i64, 0i64, 0i64);
        // SAFETY: the context is attached to a live runtime and the addresses are of live integers.
        let (first, second, begun, begun_again, done) = unsafe {
            let mutex = mote_mutex_new(context, ty.mutex.raw());
            Object::from_address(mutex).unwrap().set_word(24, 3);
            mote_mutex_take(context, mutex, &mut held as *mut i64 as usize);
            value += 1;
            mote_mutex_put(context, mutex, &value as *const i64 as usize);
            mote_mutex_take(context, mutex, &mut again as *mut i64 as usize);
            let once = mote_once_new(context, ty.once.raw());
            let begun = mote_once_begin(context, once);
            mote_once_finish(context, once, &value as *const i64 as usize);
            let begun_again = mote_once_begin(context, once);
            let mut out = 0i64;
            mote_once_value(context, once, &mut out as *mut i64 as usize);
            (held, again, begun, begun_again, out)
        };
        assert_eq!((first, second, begun, begun_again, done), (3, 4, 1, 0, 4));
    }

    #[test]
    fn entries_the_timed_calls_give_up_through_their_symbols() {
        let (_runtime, context, ty) = start(1 << 30);
        let address = context as *mut TaskContext as usize;
        let (channel, receiver, sender) = (ty.channel.raw(), ty.receiver.raw(), ty.sender.raw());
        let got = mote_scheduler::Scheduler::new(mote_scheduler::Config::seeded(1)).expect("a scheduler").run(move || {
            let context = address as *mut TaskContext;
            let mut out = 0i64;
            // SAFETY: the context is attached to a live runtime and the address is of a live integer.
            let (status, empty) = unsafe {
                let channel = mote_channel_new(context, channel, 1);
                let rx = mote_channel_end(context, channel, receiver);
                let tx = mote_channel_end(context, channel, sender);
                let message = 5i64;
                mote_channel_send(context, tx, &message as *const i64 as usize, 1);
                mote_sleep(context, 0);
                let status = mote_channel_recv_timeout(context, rx, &mut out as *mut i64 as usize, 0);
                (status, mote_channel_recv_timeout(context, rx, &mut out as *mut i64 as usize, 0))
            };
            (status, out, empty)
        });
        assert_eq!(got, Ok((1, 5, 0)));
    }

    #[test]
    fn entries_a_port_opens_on_a_channel_and_hands_a_native_reference_through_its_symbols() {
        let (runtime, context, ty) = start(1 << 30);
        let address = context as *mut TaskContext as usize;
        let (channel, port) = (ty.channel.raw(), ty.port.raw());
        let got = mote_scheduler::Scheduler::new(mote_scheduler::Config::new()).expect("a scheduler").run(move || {
            let context = address as *mut TaskContext;
            // SAFETY: the context is attached to a live runtime.
            let (reference, other) = unsafe {
                let channel = mote_channel_new(context, channel, 1);
                let port = mote_port_new(context, channel, port, 1);
                (mote_port_ref(context, port), mote_port_ref(context, port))
            };
            crate::host_port_close(reference as *mut mote_abi::MotePort);
            crate::host_port_close(other as *mut mote_abi::MotePort);
            reference != 0 && reference == other
        });
        assert_eq!(got, Ok(true));
        assert_eq!(runtime.ports_opened(), 1);
    }

    #[test]
    fn entries_a_port_call_on_a_value_that_is_not_a_port_stops_the_program() {
        let (runtime, context, ty) = start(1 << 30);
        let report = stopped_by(&runtime, context, move |context| {
            // SAFETY: as above.
            unsafe {
                let mutex = mote_mutex_new(context, ty.mutex.raw());
                mote_port_ref(context, mutex);
            }
        });
        assert_eq!((report.kind, report.message.as_str()), (FaultKind::Raised, "a port call on a value that is not a Port"));
    }

    #[test]
    fn entries_a_lock_call_that_fails_stops_the_program_with_a_raised_fault() {
        for (call, text) in [
            (0u8, "Mutex is already locked by this task"),
            (1, "Mutex is not locked by this task"),
            (2, "Once is used inside its own initializer"),
            (3, "Once has no value"),
            (4, "a mutex call on a value that is not a Mutex"),
            (5, "a mutex of a type that is not a mutex"),
        ] {
            let (runtime, context, ty) = start(1 << 30);
            let report = stopped_by(&runtime, context, move |context| {
                let mut value = 1i64;
                let address = &mut value as *mut i64 as usize;
                // SAFETY: the context is attached to a live runtime and the address is of a live integer.
                unsafe {
                    let mutex = mote_mutex_new(context, ty.mutex.raw());
                    let once = mote_once_new(context, ty.once.raw());
                    match call {
                        0 => {
                            mote_mutex_take(context, mutex, address);
                            mote_mutex_take(context, mutex, address);
                        }
                        1 => mote_mutex_put(context, mutex, address),
                        2 => {
                            mote_once_begin(context, once);
                            mote_once_begin(context, once);
                        }
                        3 => mote_once_value(context, once, address),
                        4 => mote_mutex_take(context, once, address),
                        _ => {
                            mote_mutex_new(context, ty.once.raw());
                        }
                    }
                }
            });
            assert_eq!((report.kind, report.message.as_str(), report.site), (FaultKind::Raised, text, None));
        }
    }

    #[test]
    fn entries_a_channel_call_that_fails_stops_the_program_with_a_raised_fault() {
        for (call, text) in [
            (0u8, "a channel buffer is not negative"),
            (1, "send on a closed Sender"),
            (2, "clone of a closed Sender"),
            (3, "send on a closed Receiver"),
            (4, "a channel call on a value that is not a Sender"),
        ] {
            let (runtime, context, ty) = start(1 << 30);
            let report = stopped_by(&runtime, context, move |context| {
                let message = 1i64;
                // SAFETY: the context is attached to a live runtime and the address is of a live integer.
                unsafe {
                    if call == 0 {
                        mote_channel_new(context, ty.channel.raw(), -1);
                        return;
                    }
                    let channel = mote_channel_new(context, ty.channel.raw(), 1);
                    let tx = mote_channel_end(context, channel, ty.sender.raw());
                    let rx = mote_channel_end(context, channel, ty.receiver.raw());
                    match call {
                        1 => {
                            mote_sender_close(context, tx);
                            mote_channel_send(context, tx, &message as *const i64 as usize, 1);
                        }
                        2 => {
                            mote_sender_close(context, tx);
                            mote_sender_clone(context, tx);
                        }
                        3 => {
                            mote_receiver_close(context, rx);
                            mote_channel_send(context, tx, &message as *const i64 as usize, 1);
                        }
                        _ => {
                            mote_channel_send(context, rx, &message as *const i64 as usize, 1);
                        }
                    }
                }
            });
            assert_eq!((report.kind, report.message.as_str(), report.site), (FaultKind::Raised, text, None));
        }
    }
}
