//! One request script, run against every `Platform`; a new implementation is trusted because it passes.

use contracts::{
    FileKind, FileMode, FileStat, Platform, PlatformError, PlatformErrorKind, PlatformRequest, PlatformResponse, StdStream,
    Whence,
};

fn int(platform: &dyn Platform, request: PlatformRequest) -> i64 {
    match platform.execute(request).expect("request failed") {
        PlatformResponse::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    }
}

/// `expected_args` is what the platform under test was built with; `scratch` is a file path it may create and replace.
pub fn check(platform: &dyn Platform, expected_args: &[String], scratch: &str) {
    files(platform, scratch);
    file_handles(platform, scratch);
    directories(platform, scratch);
    sockets(platform);
    let offset = int(platform, PlatformRequest::LocalOffset { unix_seconds: 1_700_000_000 });
    assert!(offset > -86_400 && offset < 86_400, "local offset {offset} is not under a day");
    sleep(platform);
    let a = int(platform, PlatformRequest::MonotonicNanos);
    let b = int(platform, PlatformRequest::MonotonicNanos);
    assert!(b >= a, "monotonic clock went backwards: {a} then {b}");

    assert!(int(platform, PlatformRequest::WallClockMillis) > 1_600_000_000_000, "wall clock before 2020");

    let e1 = int(platform, PlatformRequest::Entropy);
    let e2 = int(platform, PlatformRequest::Entropy);
    assert_ne!(e1, e2, "two entropy draws were equal");

    for stream in [StdStream::Stdout, StdStream::Stderr] {
        let wrote = platform.execute(PlatformRequest::Write { stream, bytes: Vec::new() }).unwrap();
        assert_eq!(wrote, PlatformResponse::Unit);
        let flushed = platform.execute(PlatformRequest::Flush { stream }).unwrap();
        assert_eq!(flushed, PlatformResponse::Unit);
    }

    assert_eq!(
        platform.execute(PlatformRequest::Args).unwrap(),
        PlatformResponse::Strings(expected_args.to_vec())
    );

    assert_eq!(
        platform.execute(PlatformRequest::EnvVar { name: "MOTE_NO_SUCH_VARIABLE_X".to_string() }).unwrap(),
        PlatformResponse::OptionString(None)
    );
    let PlatformResponse::Pairs(vars) = platform.execute(PlatformRequest::EnvVars).unwrap() else {
        panic!("EnvVars did not return pairs");
    };
    for (name, value) in vars {
        let got = platform.execute(PlatformRequest::EnvVar { name: name.clone() }).unwrap();
        assert_eq!(got, PlatformResponse::OptionString(Some(value)), "EnvVar({name}) disagrees with EnvVars");
    }

    for request in [PlatformRequest::WallClockMillis, PlatformRequest::Entropy, PlatformRequest::Args] {
        assert!(!platform.blocking(&request), "{request:?} must not block");
    }
}

fn files(platform: &dyn Platform, scratch: &str) {
    let path = scratch.to_string();
    let wrote = platform.execute(PlatformRequest::WriteFile { path: path.clone(), bytes: b"a\nb".to_vec() });
    assert_eq!(wrote.unwrap(), PlatformResponse::Unit);
    assert_eq!(
        platform.execute(PlatformRequest::ReadFile { path: path.clone() }).unwrap(),
        PlatformResponse::Text("a\nb".to_string())
    );
    assert_eq!(
        platform.execute(PlatformRequest::ReadFileBytes { path: path.clone() }).unwrap(),
        PlatformResponse::Bytes(b"a\nb".to_vec())
    );
    let missing = format!("{scratch}.missing");
    let err = platform.execute(PlatformRequest::ReadFile { path: missing }).unwrap_err();
    assert_eq!(err.kind, PlatformErrorKind::NotFound);
}

fn sleep(platform: &dyn Platform) {
    let before = int(platform, PlatformRequest::MonotonicNanos);
    platform.execute(PlatformRequest::Sleep { nanos: 2_000_000 }).unwrap();
    let after = int(platform, PlatformRequest::MonotonicNanos);
    assert!(after - before >= 2_000_000, "sleep of 2ms advanced the clock by {}", after - before);
}

/// A 1 ms timer delivers `Int(1)`, `Int(2)`,... in order and stops when its handle is closed.
/// `tick` lets time pass: the fake advances its clock, the system sleeps.
pub fn check_timer(platform: &dyn Platform, tick: &dyn Fn()) {
    use contracts::{event_queue, EventPayload, Overflow, SourceRequest};

    let (sink, queue) = event_queue(16, Overflow::Pause);
    let handle = platform
        .open_source(SourceRequest::Timer { period_nanos: 1_000_000 }, sink)
        .expect("timers are supported");

    let mut got = Vec::new();
    for _ in 0..2000 {
        tick();
        while let Some(event) = queue.try_pop() {
            got.push(event);
        }
        if got.len() >= 3 {
            break;
        }
    }
    let first_three: Vec<_> = got.iter().take(3).cloned().collect();
    assert_eq!(first_three, vec![EventPayload::Int(1), EventPayload::Int(2), EventPayload::Int(3)]);

    handle.close();
    tick();
    while queue.try_pop().is_some() {}
    for _ in 0..3 {
        tick();
    }
    assert!(queue.is_empty(), "a closed source kept producing");
}

fn open(platform: &dyn Platform, path: &str, mode: FileMode) -> i64 {
    int(platform, PlatformRequest::OpenFile { path: path.to_string(), mode })
}

fn read_line(platform: &dyn Platform, id: i64) -> Option<String> {
    match platform.execute(PlatformRequest::FileReadLine { id }).expect("read_line failed") {
        PlatformResponse::Line(line) => line,
        other => panic!("expected Line, got {other:?}"),
    }
}

fn read(platform: &dyn Platform, id: i64, max: usize) -> Vec<u8> {
    match platform.execute(PlatformRequest::FileRead { id, max }).expect("read failed") {
        PlatformResponse::Bytes(bytes) => bytes,
        other => panic!("expected Bytes, got {other:?}"),
    }
}

fn file_handles(platform: &dyn Platform, scratch: &str) {
    let path = format!("{scratch}.handle");
    let w = open(platform, &path, FileMode::Write);
    for chunk in [&b"one\r\n"[..], b"two\n", b"three"] {
        let wrote = platform.execute(PlatformRequest::FileWrite { id: w, bytes: chunk.to_vec() });
        assert_eq!(wrote.unwrap(), PlatformResponse::Unit);
    }
    let seek = |id, offset, whence| int(platform, PlatformRequest::FileSeek { id, offset, whence });
    assert_eq!(seek(w, 0, Whence::Current), 14);
    assert_eq!(seek(w, 0, Whence::Start), 0);
    platform.execute(PlatformRequest::FileWrite { id: w, bytes: b"ONE".to_vec() }).unwrap();
    platform.execute(PlatformRequest::FileClose { id: w }).unwrap();

    let a = open(platform, &path, FileMode::Append);
    assert_ne!(a, w, "handle ids must not be reused");
    platform.execute(PlatformRequest::FileWrite { id: a, bytes: b"\nfour\n".to_vec() }).unwrap();
    platform.execute(PlatformRequest::FileClose { id: a }).unwrap();

    let r = open(platform, &path, FileMode::Read);
    assert_eq!(read_line(platform, r).as_deref(), Some("ONE"));
    assert_eq!(read(platform, r, 2), b"tw");
    assert_eq!(read_line(platform, r).as_deref(), Some("o"));
    assert_eq!(seek(r, -5, Whence::End), 15);
    assert_eq!(read_line(platform, r).as_deref(), Some("four"));
    assert_eq!(read_line(platform, r), None);
    assert!(read(platform, r, 8).is_empty(), "read at end of file is empty");
    assert_eq!(seek(r, 5, Whence::Start), 5);
    assert_eq!(read(platform, r, 3), b"two");
    assert_eq!(seek(r, -2, Whence::Current), 6);
    assert_eq!(read(platform, r, 1), b"w");

    let err = platform.execute(PlatformRequest::FileSeek { id: r, offset: -1, whence: Whence::Start }).unwrap_err();
    assert_eq!(err.kind, PlatformErrorKind::InvalidData);
    let err = platform.execute(PlatformRequest::FileWrite { id: r, bytes: b"x".to_vec() }).unwrap_err();
    assert!(!err.message.is_empty(), "writing a read handle must fail");

    platform.execute(PlatformRequest::FileClose { id: r }).unwrap();
    for request in [
        PlatformRequest::FileRead { id: r, max: 1 },
        PlatformRequest::FileReadLine { id: r },
        PlatformRequest::FileClose { id: r },
        PlatformRequest::FileRead { id: 9_999_999, max: 1 },
    ] {
        let err = platform.execute(request).unwrap_err();
        assert_eq!(err.kind, PlatformErrorKind::Other);
        assert_eq!(err.message, "file is closed");
    }

    let missing = platform.execute(PlatformRequest::OpenFile { path: format!("{scratch}.absent"), mode: FileMode::Read });
    assert_eq!(missing.unwrap_err().kind, PlatformErrorKind::NotFound);
}

fn unit(platform: &dyn Platform, request: PlatformRequest) {
    assert_eq!(platform.execute(request).expect("request failed"), PlatformResponse::Unit);
}

fn names(platform: &dyn Platform, path: &str) -> Vec<String> {
    match platform.execute(PlatformRequest::ListDir { path: path.to_string() }).expect("list failed") {
        PlatformResponse::Strings(names) => names,
        other => panic!("expected Strings, got {other:?}"),
    }
}

fn stat(platform: &dyn Platform, path: &str) -> Result<FileStat, PlatformError> {
    match platform.execute(PlatformRequest::Stat { path: path.to_string() })? {
        PlatformResponse::Stat(stat) => Ok(stat),
        other => panic!("expected Stat, got {other:?}"),
    }
}

fn kind_of(platform: &dyn Platform, request: PlatformRequest) -> PlatformErrorKind {
    platform.execute(request).expect_err("request should fail").kind
}

fn directories(platform: &dyn Platform, scratch: &str) {
    let d = format!("{scratch}.dir");
    let at = |name: &str| format!("{d}/{name}");
    let create = |path: String, recursive| PlatformRequest::CreateDir { path, recursive };

    unit(platform, create(d.clone(), false));
    assert_eq!(kind_of(platform, create(d.clone(), false)), PlatformErrorKind::AlreadyExists);
    assert_eq!(kind_of(platform, create(at("x/y"), false)), PlatformErrorKind::NotFound);
    unit(platform, create(at("a/b"), true));
    unit(platform, create(d.clone(), true));

    for (name, text) in [("b.txt", "hello"), ("a.txt", "")] {
        unit(platform, PlatformRequest::WriteFile { path: at(name), bytes: text.as_bytes().to_vec() });
    }
    assert_eq!(names(platform, &d), ["a", "a.txt", "b.txt"]);
    assert_eq!(names(platform, &at("a")), ["b"]);
    assert_eq!(kind_of(platform, PlatformRequest::ListDir { path: at("absent") }), PlatformErrorKind::NotFound);
    assert!(platform.execute(PlatformRequest::ListDir { path: at("b.txt") }).is_err(), "listing a file must fail");

    let file = stat(platform, &at("b.txt")).unwrap();
    assert_eq!((file.kind, file.size), (FileKind::File, 5));
    assert!(file.modified_millis > 1_600_000_000_000, "modified time before 2020");
    assert_eq!(stat(platform, &d).unwrap().kind, FileKind::Dir);
    assert_eq!(stat(platform, &at("absent")).unwrap_err().kind, PlatformErrorKind::NotFound);

    unit(platform, PlatformRequest::Rename { from: at("b.txt"), to: at("c.txt") });
    assert_eq!(kind_of(platform, PlatformRequest::ReadFile { path: at("b.txt") }), PlatformErrorKind::NotFound);
    assert_eq!(stat(platform, &at("c.txt")).unwrap().size, 5);
    unit(platform, PlatformRequest::Rename { from: at("a"), to: at("z") });
    assert_eq!(names(platform, &d), ["a.txt", "c.txt", "z"]);
    assert_eq!(names(platform, &at("z")), ["b"]);
    assert_eq!(kind_of(platform, PlatformRequest::Rename { from: at("absent"), to: at("q") }), PlatformErrorKind::NotFound);

    unit(platform, PlatformRequest::RemoveFile { path: at("c.txt") });
    assert_eq!(kind_of(platform, PlatformRequest::RemoveFile { path: at("c.txt") }), PlatformErrorKind::NotFound);
    assert!(platform.execute(PlatformRequest::RemoveDir { path: d.clone(), recursive: false }).is_err());
    unit(platform, PlatformRequest::RemoveDir { path: at("z/b"), recursive: false });
    unit(platform, PlatformRequest::RemoveDir { path: d.clone(), recursive: true });
    assert_eq!(stat(platform, &d).unwrap_err().kind, PlatformErrorKind::NotFound);
}

fn read_exact(platform: &dyn Platform, id: i64, n: usize) -> Vec<u8> {
    let mut got = Vec::new();
    while got.len() < n {
        let piece = match platform.execute(PlatformRequest::SocketRead { id, max: n - got.len() }).expect("socket read failed") {
            PlatformResponse::Bytes(bytes) => bytes,
            other => panic!("expected Bytes, got {other:?}"),
        };
        assert!(!piece.is_empty(), "stream ended after {} of {n} bytes", got.len());
        got.extend(piece);
    }
    got
}

fn sockets(platform: &dyn Platform) {
    let host = || "127.0.0.1".to_string();
    let listener = int(platform, PlatformRequest::TcpListen { host: host(), port: 0 });
    let port = int(platform, PlatformRequest::LocalPort { id: listener });
    assert!(port > 0, "a listener on port 0 got no port");
    let client = int(platform, PlatformRequest::TcpConnect { host: host(), port: port as u16 });
    let server = int(platform, PlatformRequest::TcpAccept { id: listener });
    match platform.execute(PlatformRequest::PeerAddr { id: client }).expect("peer_addr failed") {
        PlatformResponse::Text(addr) => assert!(addr.ends_with(&format!(":{port}")), "peer of the client is {addr}"),
        other => panic!("expected Text, got {other:?}"),
    }

    unit(platform, PlatformRequest::SocketWrite { id: client, bytes: b"hello".to_vec() });
    assert_eq!(read_exact(platform, server, 5), b"hello");
    unit(platform, PlatformRequest::SocketWrite { id: server, bytes: b"world!".to_vec() });
    assert_eq!(read_exact(platform, client, 6), b"world!");

    unit(platform, PlatformRequest::SocketShutdownWrite { id: client });
    match platform.execute(PlatformRequest::SocketRead { id: server, max: 8 }).expect("read after shutdown failed") {
        PlatformResponse::Bytes(bytes) => assert!(bytes.is_empty(), "read after the peer shut down its write side gave {bytes:?}"),
        other => panic!("expected Bytes, got {other:?}"),
    }
    unit(platform, PlatformRequest::SocketWrite { id: server, bytes: b"still".to_vec() });
    assert_eq!(read_exact(platform, client, 5), b"still");

    unit(platform, PlatformRequest::SocketClose { id: client });
    unit(platform, PlatformRequest::SocketClose { id: server });
    assert_eq!(kind_of(platform, PlatformRequest::SocketRead { id: client, max: 1 }), PlatformErrorKind::Other);
    assert_eq!(kind_of(platform, PlatformRequest::SocketClose { id: client }), PlatformErrorKind::Other);

    unit(platform, PlatformRequest::SocketClose { id: listener });
    let mut refused = None;
    for _ in 0..200 {
        match platform.execute(PlatformRequest::TcpConnect { host: host(), port: port as u16 }) {
            Err(e) => {
                refused = Some(e.kind);
                break;
            }
            Ok(PlatformResponse::Int(stray)) => {
                unit(platform, PlatformRequest::SocketClose { id: stray });
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(other) => panic!("expected Int, got {other:?}"),
        }
    }
    assert_eq!(refused, Some(PlatformErrorKind::Other), "connecting to a closed listener was never refused");

    let a = int(platform, PlatformRequest::UdpBind { host: host(), port: 0 });
    let b = int(platform, PlatformRequest::UdpBind { host: host(), port: 0 });
    let (pa, pb) = (int(platform, PlatformRequest::LocalPort { id: a }), int(platform, PlatformRequest::LocalPort { id: b }));
    assert_ne!(pa, pb);
    let sent = int(platform, PlatformRequest::UdpSendTo { id: a, host: host(), port: pb as u16, bytes: b"ping".to_vec() });
    assert_eq!(sent, 4);
    match platform.execute(PlatformRequest::UdpRecvFrom { id: b, max: 64 }).expect("recv failed") {
        PlatformResponse::Datagram { bytes, from } => {
            assert_eq!(bytes, b"ping");
            assert!(from.ends_with(&format!(":{pa}")), "datagram came from {from}");
        }
        other => panic!("expected Datagram, got {other:?}"),
    }
    unit(platform, PlatformRequest::SocketClose { id: a });
    unit(platform, PlatformRequest::SocketClose { id: b });
}
