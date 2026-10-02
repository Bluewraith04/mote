//! Waiting for time or a socket holds no offload thread, so waits past the pool size still work.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Running {
    child: Child,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn program(source: &str, tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_reactor_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.mote");
    std::fs::write(&path, source).unwrap();
    path
}

fn start(source: &str, tag: &str) -> (Running, String) {
    let path = program(source, tag);
    let mut child = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&path).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
    (Running { child }, line.trim().to_string())
}

fn raise_fd_limit() {
    // SAFETY: `limit` is valid for both calls; the limit is inherited by the servers started afterwards.
    unsafe {
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) == 0 {
            limit.rlim_cur = limit.rlim_max.min(16384);
            libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
        }
    }
}

#[test]
fn three_thousand_sleeping_tasks_wake_together() {
    let source = "import std.time as time\n\
        fn main() {\n\
            let started = time.unix_millis()\n\
            scope {\n\
                var i = 0\n\
                while i < 3000 {\n\
                    spawn { time.sleep(time.Duration.from_millis(1500)) }\n\
                    i = i + 1\n\
                }\n\
            }\n\
            println(time.unix_millis() - started)\n\
        }\n";
    let path = program(source, "sleep");
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&path).output().unwrap();
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let millis: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    assert!((1500..3500).contains(&millis), "took {millis} ms");
}

#[test]
fn idle_connections_past_the_pool_size_do_not_stall_a_new_request() {
    raise_fd_limit();
    let source = "import std.sys.http_server as server\n\
        fn handle(conn: server.Connection) {\n\
            var requests = conn.requests()\n\
            while true {\n\
                match requests.next() {\n\
                    Ok(next) => {\n\
                        match next {\n\
                            Some(req) => { let sent = req.respond(server.Response.text(200, \"hello\")) }\n\
                            None => { break }\n\
                        }\n\
                    }\n\
                    Err(e) => { break }\n\
                }\n\
            }\n\
        }\n\
        fn main() {\n\
            let s = server.listen(\"127.0.0.1\", 0).unwrap()\n\
            println(s.local_port().unwrap())\n\
            scope {\n\
                while true {\n\
                    let conn = s.accept().unwrap()\n\
                    spawn { handle(conn) }\n\
                }\n\
            }\n\
        }\n";
    let (server, line) = start(source, "idle");
    let port: u16 = line.parse().unwrap_or_else(|_| panic!("the server printed {line:?}"));
    let idle: Vec<TcpStream> = (0..1200).map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
    let started = Instant::now();
    let mut probe = TcpStream::connect(("127.0.0.1", port)).unwrap();
    probe.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    probe.write_all(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap();
    let mut reply = Vec::new();
    probe.read_to_end(&mut reply).unwrap();
    let reply = String::from_utf8_lossy(&reply);
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(started.elapsed() < Duration::from_secs(3), "took {:?}", started.elapsed());
    drop(idle);
    drop(server);
}
