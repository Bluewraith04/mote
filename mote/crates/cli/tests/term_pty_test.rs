//! `std.term` on a real pseudo-terminal: size, keys, and the terminal restored when the program ends.
#![cfg(unix)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PROGRAM: &str = "import std.term as term\n\
    fn main() {\n\
        let t = term.open(false).unwrap()\n\
        let size = t.size().unwrap()\n\
        println(\"size ${size.cols}x${size.rows}\")\n\
        var going = true\n\
        for e in t.events() {\n\
            if going {\n\
                match e {\n\
                    Key(k, m) => {\n\
                        match k {\n\
                            Char(c) => {\n\
                                println(\"char ${c}\")\n\
                                if c == \"q\" { going = false }\n\
                            }\n\
                            Up => { println(\"up\") }\n\
                            _ => {}\n\
                        }\n\
                    }\n\
                    _ => {}\n\
                }\n\
            }\n\
            if !going { break }\n\
        }\n\
    }\n";

fn openpty() -> (File, OwnedFd) {
    let (mut master, mut slave) = (0, 0);
    let size = libc::winsize { ws_row: 30, ws_col: 100, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: the out-pointers are valid; the descriptors are owned right after.
    unsafe {
        assert_eq!(libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size), 0);
        (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
    }
}

fn read_until(master: &mut File, needle: &str, seen: &mut String) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut buf = [0u8; 4096];
    while !seen.contains(needle) {
        assert!(Instant::now() < deadline, "timed out waiting for {needle:?}; saw {seen:?}");
        // SAFETY: polls one valid descriptor.
        let ready = unsafe {
            let mut fd = libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(master), events: libc::POLLIN, revents: 0 };
            libc::poll(&mut fd, 1, 100)
        };
        if ready > 0 {
            let n = master.read(&mut buf).unwrap_or(0);
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
}

fn icanon(slave: &OwnedFd) -> bool {
    // SAFETY: `t` is a valid out-pointer for a descriptor that is a terminal.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(std::os::fd::AsRawFd::as_raw_fd(slave), &mut t), 0);
        t.c_lflag & libc::ICANON != 0
    }
}

#[test]
fn keys_arrive_and_the_terminal_is_restored_at_exit() {
    let dir = std::env::temp_dir().join(format!("mote_term_pty_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.mote");
    std::fs::write(&path, PROGRAM).unwrap();

    let (mut master, slave) = openpty();
    let stdio = |fd: &OwnedFd| Stdio::from(fd.try_clone().unwrap());
    let mut child = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(&path)
        .stdin(stdio(&slave))
        .stdout(stdio(&slave))
        .stderr(stdio(&slave))
        .spawn()
        .unwrap();

    let mut seen = String::new();
    read_until(&mut master, "size 100x30", &mut seen);
    assert!(!icanon(&slave), "the terminal should be raw while the program runs");
    master.write_all(b"x").unwrap();
    read_until(&mut master, "char x", &mut seen);
    master.write_all(b"\x1b[A").unwrap();
    read_until(&mut master, "up", &mut seen);
    master.write_all(b"q").unwrap();
    read_until(&mut master, "char q", &mut seen);

    assert!(child.wait().unwrap().success());
    assert!(icanon(&slave), "the terminal should be restored when the program ends");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_fault_and_an_exit_also_restore_the_terminal() {
    for (tag, body) in [("fault", "let xs: List<Int> = []\n    println(xs.get(3))"), ("exit", "process.exit(3)")] {
        let dir = std::env::temp_dir().join(format!("mote_term_pty_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.mote");
        let source = format!("import std.term as term\nimport std.process as process\nfn main() {{\n    let t = term.open(false).unwrap()\n    {body}\n}}\n");
        std::fs::write(&path, source).unwrap();
        let (_master, slave) = openpty();
        let stdio = |fd: &OwnedFd| Stdio::from(fd.try_clone().unwrap());
        let mut child = Command::new(env!("CARGO_BIN_EXE_mote"))
            .arg("run")
            .arg(&path)
            .stdin(stdio(&slave))
            .stdout(stdio(&slave))
            .stderr(stdio(&slave))
            .spawn()
            .unwrap();
        assert!(!child.wait().unwrap().success(), "{tag} should end the program with a failure");
        assert!(icanon(&slave), "the terminal should be restored after a {tag}");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn a_pipe_is_not_a_terminal() {
    let dir = std::env::temp_dir().join(format!("mote_term_pipe_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.mote");
    std::fs::write(&path, "import std.term as term\nfn main() {\n    match term.open(false) {\n        Ok(t) => { println(\"opened\") }\n        Err(e) => { println(\"no terminal\") }\n    }\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&path).stdin(Stdio::null()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "no terminal\n");
    std::fs::remove_dir_all(&dir).ok();
}
