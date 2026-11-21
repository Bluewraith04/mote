//! Loads the external libraries of a program before `main` and serves the helpers of C FFI calls.

use std::ffi::{c_char, c_void, CString};

use mote_abi::{LinkTable, Meta, MoteHost, TaskContext};
use mote_base::Span;

use crate::helpers::new_string;
use crate::Runtime;

/// Why a program's libraries could not be loaded, with the `extern` fn it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError {
    pub message: String,
    pub span: Span,
}

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(pointer: *mut c_void);
}

#[cfg(target_os = "linux")]
mod sys_load {
    use std::ffi::{c_char, c_int, c_void, CString};

    unsafe extern "C" {
        fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn poll(fds: *mut PollFd, count: u64, timeout: c_int) -> c_int;
    }

    const RTLD_NOW: c_int = 2;
    const POLLIN: i16 = 1;
    const POLLOUT: i16 = 4;
    const POLLHUP: i16 = 0x10;
    const EINTR: i32 = 4;

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    pub fn default_file(library: &str) -> String {
        format!("lib{library}.so")
    }

    pub unsafe fn open(file: &str) -> Option<*mut c_void> {
        let name = CString::new(file).ok()?;
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        (!handle.is_null()).then_some(handle)
    }

    pub unsafe fn symbol(handle: *mut c_void, name: &std::ffi::CStr) -> Option<*mut c_void> {
        let addr = unsafe { dlsym(handle, name.as_ptr()) };
        (!addr.is_null()).then_some(addr)
    }

    pub fn fd_wait(fd: i32, interest: i32, timeout_ms: i64) -> i32 {
        if fd < 0 {
            return mote_abi::WAIT_FAILED;
        }
        let events = if interest == mote_abi::WAIT_WRITABLE { POLLOUT } else { POLLIN };
        let mut entry = PollFd { fd, events, revents: 0 };
        let started = std::time::Instant::now();
        loop {
            let left = if timeout_ms < 0 { -1 } else { (timeout_ms - started.elapsed().as_millis().min(i64::MAX as u128) as i64).clamp(0, c_int::MAX as i64) as c_int };
            // SAFETY: `entry` is one valid `pollfd`.
            let ready = unsafe { poll(&mut entry, 1, left) };
            if ready > 0 {
                return if entry.revents & (events | POLLHUP) != 0 { mote_abi::WAIT_READY } else { mote_abi::WAIT_FAILED };
            }
            if ready == 0 {
                return mote_abi::WAIT_TIMED_OUT;
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(EINTR) {
                return mote_abi::WAIT_FAILED;
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod sys_load {
    use std::ffi::{c_char, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::sync::Once;

    const POLLRDNORM: i16 = 0x0100;
    const POLLWRNORM: i16 = 0x0010;
    const POLLERR: i16 = 0x0001;
    const POLLHUP: i16 = 0x0002;
    const POLLNVAL: i16 = 0x0004;

    #[repr(C)]
    struct WsaPollFd {
        fd: usize,
        events: i16,
        revents: i16,
    }

    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAStartup(version: u16, data: *mut [u8; 512]) -> i32;
        fn WSAPoll(fds: *mut WsaPollFd, count: u32, timeout_ms: i32) -> i32;
    }

    static WSA_INIT: Once = Once::new();

    fn ensure_wsa() {
        WSA_INIT.call_once(|| {
            let mut data = [0u8; 512];
            unsafe { WSAStartup(0x0202, &mut data) };
        });
    }

    pub fn default_file(library: &str) -> String {
        format!("{library}.dll")
    }

    pub unsafe fn open(file: &str) -> Option<*mut c_void> {
        if file.contains('\0') {
            return None;
        }
        let wide: Vec<u16> = std::ffi::OsStr::new(file).encode_wide().chain(std::iter::once(0)).collect();
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        (!handle.is_null()).then_some(handle)
    }

    pub unsafe fn symbol(handle: *mut c_void, name: &std::ffi::CStr) -> Option<*mut c_void> {
        let addr = unsafe { GetProcAddress(handle, name.as_ptr()) };
        (!addr.is_null()).then_some(addr)
    }

    pub fn fd_wait(fd: i32, interest: i32, timeout_ms: i64) -> i32 {
        if fd < 0 {
            return mote_abi::WAIT_FAILED;
        }
        ensure_wsa();
        let events = if interest == mote_abi::WAIT_WRITABLE { POLLWRNORM } else { POLLRDNORM };
        let mut entry = WsaPollFd { fd: fd as usize, events, revents: 0 };
        let left = if timeout_ms < 0 { -1 } else { timeout_ms.clamp(0, i32::MAX as i64) as i32 };
        let ready = unsafe { WSAPoll(&mut entry, 1, left) };
        if ready > 0 {
            if entry.revents & (POLLERR | POLLNVAL) != 0 && (entry.revents & (events | POLLHUP) == 0) {
                return mote_abi::WAIT_FAILED;
            }
            return if entry.revents & (events | POLLHUP) != 0 { mote_abi::WAIT_READY } else { mote_abi::WAIT_FAILED };
        }
        if ready == 0 { mote_abi::WAIT_TIMED_OUT } else { mote_abi::WAIT_FAILED }
    }
}

/// The symbol of the helpers of native calls.
pub const NATIVE: &str = "mote_native";

/// The codes `mote_native` is given.
pub mod native {
    /// A NUL-terminated copy of the string in `a`.
    pub const IN: u32 = 0;
    /// Frees a copy made by `IN`.
    pub const FREE: u32 = 1;
    /// A new string of the C string at `a`.
    pub const OUT: u32 = 2;
}

/// Runs the thunk of a `@blocking` native over a copy of its `words`-word argument block on a helper thread, parks the task until it returns and copies the block back; outside a task it waits on the thread.
pub(crate) fn call_blocking(thunk: usize, buffer: usize, words: usize) {
    // `thunk` is a function `extern "C" fn(*mut u64)` that compiled code made.
    let run = move |block: *mut u64| {
        // SAFETY: as above, and `block` has the `words` words the thunk reads and writes.
        let thunk: extern "C" fn(*mut u64) = unsafe { std::mem::transmute(thunk) };
        thunk(block);
    };
    // SAFETY: `buffer` is the argument block of the call, `words` long, and the task's frame holds it until this returns.
    let mut block: Vec<u64> = unsafe { std::slice::from_raw_parts(buffer as *const u64, words) }.to_vec();
    if mote_scheduler::in_a_task() {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = mote_scheduler::current();
        let source = mote_scheduler::foreign_source();
        let signal = done.clone();
        let helper = std::thread::spawn(move || {
            run(block.as_mut_ptr());
            signal.store(true, std::sync::atomic::Ordering::Release);
            task.unpark();
            drop(source);
            block
        });
        while !done.load(std::sync::atomic::Ordering::Acquire) {
            mote_scheduler::park_on("a native call");
        }
        block = helper.join().expect("a native call does not unwind");
    } else {
        std::thread::scope(|scope| scope.spawn(|| run(block.as_mut_ptr())).join().expect("a native call does not unwind"));
    }
    // SAFETY: as above; the thunk wrote its result in the copy.
    unsafe { std::ptr::copy_nonoverlapping(block.as_ptr(), buffer as *mut u64, words) };
}

static HOST: MoteHost = MoteHost::new(crate::host_port_push, crate::host_port_close, host_fd_wait);

/// `MoteHost::fd_wait`: polls the descriptor on the calling thread.
extern "C" fn host_fd_wait(fd: i32, interest: i32, timeout_ms: i64) -> i32 {
    sys_load::fd_wait(fd, interest, timeout_ms)
}

/// Opens each library that has a function whose slot is empty, fills the empty slots, and calls each `mote_init`.
///
/// # Safety
/// `slots` addresses a word for each function of the table.
pub unsafe fn load(table: &LinkTable, files: &[String], slots: *mut usize) -> Result<(), LinkError> {
    let first_span = |library: usize| table.functions.iter().find(|function| function.library as usize == library).map_or_else(Span::default, |function| function.span);
    // SAFETY: the caller's promise.
    let empty = |index: usize| unsafe { *slots.add(index) == 0 };
    let mut handles = Vec::new();
    for (index, library) in table.libraries.iter().enumerate() {
        if !table.functions.iter().enumerate().any(|(at, function)| function.library as usize == index && empty(at)) {
            handles.push(std::ptr::null_mut());
            continue;
        }
        let file = files.get(index).cloned().unwrap_or_else(|| sys_load::default_file(library));
        let missing = || LinkError { message: format!("library `{library}` not found"), span: first_span(index) };
        // SAFETY: opens the dynamic library by path or system name.
        let handle = unsafe { sys_load::open(&file) }.ok_or_else(missing)?;
        handles.push(handle);
    }
    for (index, function) in table.functions.iter().enumerate().filter(|(index, _)| empty(*index)) {
        let library = &table.libraries[function.library as usize];
        let missing = || LinkError { message: format!("symbol `{}` not found in `{library}`", function.symbol), span: function.span };
        let symbol = CString::new(function.symbol.as_str()).map_err(|_| missing())?;
        // SAFETY: the handle came from `sys_load::open` and `symbol` is a NUL-terminated string.
        let address = unsafe { sys_load::symbol(handles[function.library as usize], &symbol) }.ok_or_else(missing)?;
        // SAFETY: the caller's promise.
        unsafe { *slots.add(index) = address as usize };
    }
    for handle in handles.into_iter().filter(|handle| !handle.is_null()) {
        // SAFETY: the handle came from `sys_load::open`.
        if let Some(init) = unsafe { sys_load::symbol(handle, c"mote_init") } {
            // SAFETY: `mote_init` is `void mote_init(const MoteHost *)` by the contract of the library.
            let init: extern "C" fn(*const MoteHost) = unsafe { std::mem::transmute(init) };
            init(&HOST);
        }
    }
    Ok(())
}

/// The files with each path replaced by `<dir>/lib/<its file name>` where that file exists; a bare name is left to the system.
pub fn prefer_beside(files: &[String], dir: &std::path::Path) -> Vec<String> {
    let beside = |file: &String| {
        let path = std::path::Path::new(file);
        let near = dir.join("lib").join(path.file_name()?);
        ((file.contains('/') || file.contains('\\')) && near.is_file()).then(|| near.display().to_string())
    };
    files.iter().map(|file| beside(file).unwrap_or_else(|| file.clone())).collect()
}

/// Calls the `mote_init_<name>` that the linker resolved for a library linked in; a library without one has 0.
pub fn init_linked(inits: &[usize]) {
    for &address in inits.iter().filter(|&&address| address != 0) {
        // SAFETY: the compiler wrote the address of a function `void mote_init_<name>(const MoteHost *)` by the contract of the library.
        let init: extern "C" fn(*const MoteHost) = unsafe { std::mem::transmute(address) };
        init(&HOST);
    }
}

/// No-op init used on Windows when a statically linked native archive defines no `mote_init_<name>`.
#[unsafe(no_mangle)]
pub extern "C" fn mote_init_noop(_host: *const MoteHost) {}

/// The helpers of native calls, by their code in `native`; gives a pointer, a reference, or 0 when it cannot.
#[unsafe(no_mangle)]
pub extern "C" fn mote_native(context: *mut TaskContext, code: u32, a: u64, _b: u64) -> u64 {
    // SAFETY: compiled code passes a string it holds, a pointer `IN` made, a C string a native returned, or a thunk with its buffer.
    unsafe {
        match code {
            native::IN => {
                let length = *((a as usize + mote_abi::LENGTH_OFFSET as usize) as *const u64) as usize;
                let copy = malloc(length + 1) as *mut u8;
                if copy.is_null() {
                    return 0;
                }
                std::ptr::copy_nonoverlapping((a as usize + mote_abi::ELEMENTS_OFFSET as usize) as *const u8, copy, length);
                *copy.add(length) = 0;
                copy as u64
            }
            native::FREE => {
                free(a as *mut c_void);
                0
            }
            native::OUT => {
                let meta = Runtime::of(context).data::<Meta>().expect("a compiled program keeps its meta");
                let bytes = if a == 0 { &[][..] } else { std::ffi::CStr::from_ptr(a as *const c_char).to_bytes() };
                match std::str::from_utf8(bytes) {
                    Ok(_) => new_string(context, meta.string.raw(), bytes),
                    Err(_) => new_string(context, meta.string.raw(), String::from_utf8_lossy(bytes).as_bytes()),
                }
            }
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use mote_abi::{WAIT_FAILED, WAIT_READABLE, WAIT_READY, WAIT_TIMED_OUT, WAIT_WRITABLE};

    use super::{host_fd_wait, prefer_beside, sys_load};

    #[cfg(target_os = "linux")]
    fn stream_pair() -> (std::os::unix::net::UnixStream, std::os::unix::net::UnixStream, i32) {
        use std::os::fd::AsRawFd;
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let fb = b.as_raw_fd();
        (a, b, fb)
    }

    #[cfg(target_os = "windows")]
    fn stream_pair() -> (std::net::TcpStream, std::net::TcpStream, i32) {
        use std::net::{TcpListener, TcpStream};
        use std::os::windows::io::AsRawSocket;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let a = TcpStream::connect(addr).unwrap();
        let (b, _) = listener.accept().unwrap();
        let fb = b.as_raw_socket() as i32;
        (a, b, fb)
    }

    #[test]
    fn beside_a_library_next_to_the_executable_replaces_the_built_in_path() {
        let dir = std::env::temp_dir().join(format!("mote-beside-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        let here = sys_load::default_file("here");
        let gone = sys_load::default_file("gone");
        let syslib = sys_load::default_file("m");
        std::fs::write(dir.join("lib").join(&here), "x").unwrap();
        let files = [format!("/build/lib/{here}"), format!("/build/lib/{gone}"), syslib];
        let found = prefer_beside(&files, &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found[0], dir.join("lib").join(&here).display().to_string());
        assert_eq!(found[1..], files[1..]);
    }

    #[test]
    fn host_fd_wait_times_out_while_empty_and_is_ready_after_a_write() {
        let (mut near, _far, far_fd) = stream_pair();
        assert_eq!(host_fd_wait(far_fd, WAIT_READABLE, 10), WAIT_TIMED_OUT);
        near.write_all(b"x").unwrap();
        assert_eq!(host_fd_wait(far_fd, WAIT_READABLE, 10), WAIT_READY);
        assert_eq!(host_fd_wait(far_fd, WAIT_WRITABLE, -1), WAIT_READY);
    }

    #[test]
    fn host_fd_wait_a_closed_peer_is_ready_and_a_bad_descriptor_fails() {
        let (near, _far, far_fd) = stream_pair();
        drop(near);
        assert_eq!(host_fd_wait(far_fd, WAIT_READABLE, 1000), WAIT_READY);
        assert_eq!(host_fd_wait(-1, WAIT_READABLE, 10), WAIT_FAILED);
        assert_eq!(host_fd_wait(9999, WAIT_READABLE, 10), WAIT_FAILED);
    }
}
