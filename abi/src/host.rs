/// The runtime's end of a port, opaque to a library.
#[repr(C)]
pub struct MotePort {
    _private: [u8; 0],
}

/// `port_push` result: the message was queued.
pub const PUSH_QUEUED: i32 = 0;
/// `port_push` result: the port is full and the message was not taken.
pub const PUSH_FULL: i32 = 1;
/// `port_push` result: the receiver was closed or dropped.
pub const PUSH_CLOSED: i32 = 2;

/// `fd_wait` interest: the descriptor can be read.
pub const WAIT_READABLE: i32 = 0;
/// `fd_wait` interest: the descriptor can be written.
pub const WAIT_WRITABLE: i32 = 1;

/// `fd_wait` result: the descriptor is ready.
pub const WAIT_READY: i32 = 0;
/// `fd_wait` result: the time ran out first.
pub const WAIT_TIMED_OUT: i32 = 1;
/// `fd_wait` result: the descriptor is invalid or in error.
pub const WAIT_FAILED: i32 = 2;

/// The table a library's `mote_init` receives; fields are only added at the end.
#[repr(C)]
pub struct MoteHost {
    /// The size of this table in bytes; a library reads only the fields inside it.
    pub size: u64,
    /// Copies a message into the port; callable from any thread, never waits.
    pub port_push: extern "C" fn(*mut MotePort, *const u8, i64) -> i32,
    /// Ends the sender and releases the reference the library was given.
    pub port_close: extern "C" fn(*mut MotePort),
    /// Blocks the calling thread until the descriptor is ready or the time in milliseconds is out (negative waits without limit).
    pub fd_wait: extern "C" fn(i32, i32, i64) -> i32,
}

impl MoteHost {
    pub const fn new(port_push: extern "C" fn(*mut MotePort, *const u8, i64) -> i32, port_close: extern "C" fn(*mut MotePort), fd_wait: extern "C" fn(i32, i32, i64) -> i32) -> Self {
        Self { size: size_of::<MoteHost>() as u64, port_push, port_close, fd_wait }
    }
}

/// A `#define` of `mote.h`: its name, value and what it means.
const CONSTANTS: [(&str, i32, &str); 8] = [
    ("MOTE_PUSH_QUEUED", PUSH_QUEUED, "port_push: the message was queued"),
    ("MOTE_PUSH_FULL", PUSH_FULL, "port_push: the port is full and the message was not taken"),
    ("MOTE_PUSH_CLOSED", PUSH_CLOSED, "port_push: the receiver was closed or dropped"),
    ("MOTE_WAIT_READABLE", WAIT_READABLE, "fd_wait interest: the descriptor can be read"),
    ("MOTE_WAIT_WRITABLE", WAIT_WRITABLE, "fd_wait interest: the descriptor can be written"),
    ("MOTE_WAIT_READY", WAIT_READY, "fd_wait: the descriptor is ready"),
    ("MOTE_WAIT_TIMED_OUT", WAIT_TIMED_OUT, "fd_wait: the time ran out first"),
    ("MOTE_WAIT_FAILED", WAIT_FAILED, "fd_wait: the descriptor is invalid or in error"),
];

/// A field of `MoteHost` after `size`: the C result type, name, parameters and what it does.
const FIELDS: [(&str, &str, &str, &str); 3] = [
    ("int32_t", "port_push", "MotePort *port, const uint8_t *bytes, int64_t length", "Copies a message into the port; callable from any thread, never waits."),
    ("void", "port_close", "MotePort *port", "Ends the sender and releases the reference the library was given."),
    ("int32_t", "fd_wait", "int32_t fd, int32_t interest, int64_t timeout_ms", "Blocks the calling thread until the descriptor is ready or the time is out (negative waits without limit)."),
];

/// The text of `mote.h`, made from the table and its constants.
pub fn header() -> String {
    let mut text = String::from("/* The host table of a Mote external library. Generated from abi/src/host.rs; do not edit. */\n#ifndef MOTE_H\n#define MOTE_H\n\n#include <stddef.h>\n#include <stdint.h>\n\n");
    text.push_str("/* The runtime's end of a port. */\ntypedef struct MotePort MotePort;\n\n");
    for (name, value, doc) in CONSTANTS {
        text.push_str(&format!("#define {name} {value} /* {doc} */\n"));
    }
    text.push_str("\n/* The table `mote_init` receives; a library reads only the fields inside `size`. */\ntypedef struct MoteHost {\n    uint64_t size;\n");
    for (result, name, params, doc) in FIELDS {
        text.push_str(&format!("    /* {doc} */\n    {result} (*{name})({params});\n"));
    }
    text.push_str("} MoteHost;\n\n/* Called once when the library is loaded. */\nvoid mote_init(const MoteHost *host);\n\n#endif\n");
    text
}

/// A C file that includes the header and asserts the layout `MoteHost` has.
#[cfg(test)]
fn layout_check() -> String {
    let mut text = String::from("#include \"mote.h\"\n\n");
    text.push_str(&format!("_Static_assert(sizeof(MoteHost) == {}, \"the table size\");\n_Static_assert(offsetof(MoteHost, size) == 0, \"size\");\n", size_of::<MoteHost>()));
    for (at, (_, name, _, _)) in FIELDS.iter().enumerate() {
        text.push_str(&format!("_Static_assert(offsetof(MoteHost, {name}) == {}, \"{name}\");\n", 8 * (at + 1)));
    }
    text
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;

    extern "C" fn push(_: *mut MotePort, _: *const u8, _: i64) -> i32 {
        PUSH_QUEUED
    }

    extern "C" fn close(_: *mut MotePort) {}

    extern "C" fn wait(_: i32, _: i32, _: i64) -> i32 {
        WAIT_READY
    }

    fn header_file() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("mote.h")
    }

    #[test]
    fn the_table_is_four_words_with_its_size_first() {
        assert_eq!(offset_of!(MoteHost, size), 0);
        assert_eq!(offset_of!(MoteHost, port_push), 8);
        assert_eq!(offset_of!(MoteHost, port_close), 16);
        assert_eq!(offset_of!(MoteHost, fd_wait), 24);
        assert_eq!(MoteHost::new(push, close, wait).size, 32);
    }

    #[test]
    fn push_results_are_stable() {
        assert_eq!([PUSH_QUEUED, PUSH_FULL, PUSH_CLOSED], [0, 1, 2]);
    }

    #[test]
    fn wait_values_are_stable() {
        assert_eq!([WAIT_READABLE, WAIT_WRITABLE], [0, 1]);
        assert_eq!([WAIT_READY, WAIT_TIMED_OUT, WAIT_FAILED], [0, 1, 2]);
    }

    #[test]
    fn host_header_file_is_the_generated_text() {
        if std::env::var_os("MOTE_BLESS").is_some() {
            std::fs::write(header_file(), header()).unwrap();
        }
        assert_eq!(std::fs::read_to_string(header_file()).unwrap(), header(), "run with MOTE_BLESS=1 to regenerate abi/mote.h");
    }

    #[test]
    fn host_header_compiles_and_matches_the_layout_of_the_table() {
        let dir = std::env::temp_dir().join(format!("mote-header-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("check.c"), layout_check()).unwrap();
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let result = Command::new(cc).args(["-std=c11", "-Wall", "-Werror", "-fsyntax-only", "-I"]).arg(header_file().parent().unwrap()).arg(dir.join("check.c")).output();
        std::fs::remove_dir_all(&dir).ok();
        match result {
            Ok(output) => assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr)),
            Err(_) => eprintln!("skipped: no C compiler"),
        }
    }
}
