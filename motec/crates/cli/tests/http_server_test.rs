//! `std.sys.http_server` driven by raw TCP clients against a Mote server process.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const APP: &str = r#"import std.sys.http_server as server
import std.sys.io as io
import std.data.json as json

fn route(req: server.Request) -> server.Response {
    if req.path == "/hello" { return server.Response.text(200, "hello") }
    if req.path == "/echo" {
        let t = req.text().unwrap_or("(binary)")
        return server.Response.text(200, "${req.method} len=${req.body.len()} ${t}")
    }
    if req.path == "/query" {
        let q = req.query()
        var out = req.path + " " + req.target
        for k in q.keys() {
            out = out + " " + k + "=" + q[k]
        }
        return server.Response.text(200, out)
    }
    if req.path == "/json" {
        match req.json() {
            Ok(j) => { return server.Response.json(201, j) }
            Err(e) => { return server.Response.text(400, e.message) }
        }
    }
    if req.path == "/form" {
        match req.form() {
            Ok(f) => {
                let a = f.get_or("a", "-")
                let b = f.get_or("b", "-")
                return server.Response.text(200, "a=${a} b=${b}")
            }
            Err(e) => { return server.Response.text(400, e.message) }
        }
    }
    if req.path == "/headers" {
        let token = req.header("X-Token").unwrap_or("none")
        let accept = req.header_map().get_or("accept", "none")
        return server.Response.text(200, "token=${token} accept=${accept}")
    }
    if req.path == "/binary" {
        let b = Bytes()
        var i = 0
        while i < 256 {
            b.push(i)
            i = i + 1
        }
        return server.Response.new(200, b)
    }
    if req.path == "/created" { return server.Response.empty(201).header("Location", "/x").header("X-A", "1").header("x-a", "2") }
    if req.path == "/nocontent" { return server.Response.empty(204) }
    if req.path == "/bad-header" { return server.Response.text(200, "x").header("x-bad", "a\r\nb: c") }
    if req.path == "/bad-status" { return server.Response.empty(42) }
    if req.path == "/nan" { return server.Response.json(200, json.Json.Float(0.0 / 0.0)) }
    return server.Response.text(404, "no ${req.path}")
}

fn handle(conn: server.Connection) {
    var requests = conn.requests()
    while true {
        match requests.next() {
            Ok(next) => {
                match next {
                    Some(req) => {
                        match req.respond(route(req)) {
                            Ok(sent) => {}
                            Err(e) => { let r = req.respond(server.Response.text(500, e.message)) }
                        }
                    }
                    None => { break }
                }
            }
            Err(e) => { break }
        }
    }
}

fn main() {
    var s = server.listen("127.0.0.1", 0).unwrap()
    s.set_max_body(1000)
    s.set_max_header(2000)
    s.set_idle_timeout(1)
    println(s.local_port().unwrap())
    io.stdout().flush()
    scope {
        while true {
            let conn = s.accept().unwrap()
            spawn { handle(conn) }
        }
    }
}
"#;

struct Running {
    child: Child,
    port: u16,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(source: &str, tag: &str) -> Running {
    let dir = std::env::temp_dir().join(format!("mote_httpd_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(dir.join("main.mote"))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    let port = line.trim().parse().unwrap_or_else(|_| panic!("the server printed {line:?}"));
    std::fs::remove_dir_all(&dir).ok();
    Running { child, port }
}

fn app(tag: &str) -> Running {
    start(APP, tag)
}

struct Reply {
    status: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

type Reader = BufReader<TcpStream>;

fn connect(port: u16) -> (TcpStream, Reader) {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    (stream, reader)
}

fn read_reply(reader: &mut Reader, head: bool) -> Option<Reply> {
    let mut status = String::new();
    if reader.read_line(&mut status).ok()? == 0 {
        return None;
    }
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (k, v) = line.split_once(':').unwrap();
        headers.push((k.to_string(), v.trim().to_string()));
    }
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; if head { 0 } else { len }];
    reader.read_exact(&mut body).ok()?;
    Some(Reply { status: status.trim_end().to_string(), headers, body: String::from_utf8_lossy(&body).into_owned() })
}

fn once(port: u16, raw: &str) -> Reply {
    let (mut stream, mut reader) = connect(port);
    stream.write_all(raw.as_bytes()).unwrap();
    read_reply(&mut reader, raw.starts_with("HEAD")).expect("no reply")
}

fn closed(reader: &mut Reader) -> bool {
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).is_ok() && rest.is_empty()
}

#[test]
fn a_get_is_answered() {
    let s = app("get");
    let r = once(s.port, "GET /hello HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.body, "hello");
    assert_eq!(r.header("content-length"), Some("5"));
    assert_eq!(r.header("content-type"), Some("text/plain; charset=utf-8"));
    assert_eq!(r.header("connection"), None);
    let date = r.header("date").unwrap();
    assert!(date.ends_with(" GMT") && date.len() == 29, "{date}");
    let missing = once(s.port, "GET /nothing HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!((missing.status.as_str(), missing.body.as_str()), ("HTTP/1.1 404 Not Found", "no /nothing"));
}

#[test]
fn a_connection_serves_requests_in_order_and_closes_on_request() {
    let s = app("keepalive");
    let (mut stream, mut reader) = connect(s.port);
    stream.write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut reader, false).unwrap().body, "hello");
    stream.write_all(b"GET /nothing HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut reader, false).unwrap().body, "no /nothing");
    stream.write_all(b"GET /hello HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let last = read_reply(&mut reader, false).unwrap();
    assert_eq!(last.header("connection"), Some("close"));
    assert!(closed(&mut reader));

    let (mut old, mut old_reader) = connect(s.port);
    old.write_all(b"GET /hello HTTP/1.0\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut old_reader, false).unwrap().header("connection"), Some("close"));
    assert!(closed(&mut old_reader));

    let (mut kept, mut kept_reader) = connect(s.port);
    kept.write_all(b"GET /hello HTTP/1.0\r\nConnection: keep-alive\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut kept_reader, false).unwrap().header("connection"), None);
    kept.write_all(b"GET /hello HTTP/1.0\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut kept_reader, false).unwrap().body, "hello");
}

#[test]
fn pipelined_requests_are_answered_in_order() {
    let s = app("pipeline");
    let (mut stream, mut reader) = connect(s.port);
    stream
        .write_all(b"POST /echo HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /hello HTTP/1.1\r\n\r\nGET /nothing HTTP/1.1\r\n\r\n")
        .unwrap();
    let bodies: Vec<String> = (0..3).map(|_| read_reply(&mut reader, false).unwrap().body).collect();
    assert_eq!(bodies, ["POST len=3 abc", "hello", "no /nothing"]);
}

#[test]
fn bodies_arrive_whole_by_length_or_in_chunks() {
    let s = app("bodies");
    let sized = once(s.port, "POST /echo HTTP/1.1\r\nContent-Length: 11\r\n\r\nhello world");
    assert_eq!(sized.body, "POST len=11 hello world");
    let chunked = once(s.port, "POST /echo HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\nX-Trailer: 1\r\n\r\n");
    assert_eq!(chunked.body, "POST len=11 hello world");
    let empty = once(s.port, "POST /echo HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n");
    assert_eq!(empty.body, "POST len=0 ");

    let (mut stream, mut reader) = connect(s.port);
    stream.write_all(b"POST /echo HTTP/1.1\r\nContent-Length: 10\r\n\r\nabcde").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    stream.write_all(b"fghij").unwrap();
    assert_eq!(read_reply(&mut reader, false).unwrap().body, "POST len=10 abcdefghij");

    let (mut slow, mut slow_reader) = connect(s.port);
    slow.write_all(b"POST /echo HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nab").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    slow.write_all(b"cd\r\n0\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut slow_reader, false).unwrap().body, "POST len=4 abcd");
}

#[test]
fn expect_continue_gets_an_interim_reply() {
    let s = app("expect");
    let (mut stream, mut reader) = connect(s.port);
    stream.write_all(b"POST /echo HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 2\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut reader, false).unwrap().status, "HTTP/1.1 100 Continue");
    stream.write_all(b"ok").unwrap();
    assert_eq!(read_reply(&mut reader, false).unwrap().body, "POST len=2 ok");
}

#[test]
fn paths_queries_and_headers_are_decoded() {
    let s = app("decode");
    let q = once(s.port, "GET /query?a=1&b=x%26y+z&a=2&%C3%A9=%C3%BC HTTP/1.1\r\n\r\n");
    assert_eq!(q.body, "/query /query?a=1&b=x%26y+z&a=2&%C3%A9=%C3%BC a=2 b=x&y z é=ü");
    let p = once(s.port, "GET /que%72y?x=1 HTTP/1.1\r\n\r\n");
    assert_eq!(p.body, "/query /que%72y?x=1 x=1");
    let h = once(s.port, "GET /headers HTTP/1.1\r\nx-TOKEN: abc\r\nAccept: a\r\nAccept: b\r\n\r\n");
    assert_eq!(h.body, "token=abc accept=a, b");
    let none = once(s.port, "GET /headers HTTP/1.1\r\n\r\n");
    assert_eq!(none.body, "token=none accept=none");
    let absolute = once(s.port, "GET http://example.com:80/que%72y?x=1 HTTP/1.1\r\n\r\n");
    assert_eq!(absolute.body, "/query http://example.com:80/que%72y?x=1 x=1");
}

#[test]
fn json_and_form_bodies_parse() {
    let s = app("jsonform");
    let j = once(s.port, "POST /json HTTP/1.1\r\nContent-Length: 11\r\n\r\n{\"k\":[1,2]}");
    assert_eq!(j.status, "HTTP/1.1 201 Created");
    assert_eq!(j.header("content-type"), Some("application/json"));
    assert_eq!(j.body, "{\"k\":[1,2]}");
    let bad = once(s.port, "POST /json HTTP/1.1\r\nContent-Length: 3\r\n\r\n{k:");
    assert_eq!(bad.status, "HTTP/1.1 400 Bad Request");
    let f = once(s.port, "POST /form HTTP/1.1\r\nContent-Length: 13\r\n\r\na=1+2&b=x%26y");
    assert_eq!(f.body, "a=1 2 b=x&y");
}

#[test]
fn replies_carry_status_headers_and_bodies() {
    let s = app("replies");
    let created = once(s.port, "GET /created HTTP/1.1\r\n\r\n");
    assert_eq!(created.status, "HTTP/1.1 201 Created");
    assert_eq!(created.header("location"), Some("/x"));
    assert_eq!(created.header("x-a"), Some("2"));
    assert_eq!(created.headers.iter().filter(|(k, _)| k == "x-a").count(), 1);
    assert_eq!(created.header("content-length"), Some("0"));
    let none = once(s.port, "GET /nocontent HTTP/1.1\r\n\r\n");
    assert_eq!(none.status, "HTTP/1.1 204 No Content");
    assert_eq!(none.header("content-length"), None);
    let head = once(s.port, "HEAD /hello HTTP/1.1\r\n\r\n");
    assert_eq!((head.header("content-length"), head.body.as_str()), (Some("5"), ""));

    let (mut stream, mut reader) = connect(s.port);
    stream.write_all(b"GET /binary HTTP/1.1\r\n\r\n").unwrap();
    let mut line = String::new();
    let mut length = 0;
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if let Some(v) = line.strip_prefix("content-length: ") {
            length = v.trim().parse().unwrap();
        }
        if line == "\r\n" {
            break;
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).unwrap();
    assert_eq!(body, (0..=255u8).collect::<Vec<_>>());
}

#[test]
fn a_bad_reply_is_an_error_not_a_broken_response() {
    let s = app("badreply");
    let (mut stream, mut reader) = connect(s.port);
    for (path, message) in [("/bad-header", "bad header x-bad"), ("/bad-status", "bad status 42"), ("/nan", "NaN")] {
        stream.write_all(format!("GET {path} HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
        let r = read_reply(&mut reader, false).unwrap();
        assert_eq!(r.status, "HTTP/1.1 500 Internal Server Error", "{path}");
        assert!(r.body.contains(message), "{path}: {}", r.body);
    }
}

#[test]
fn bad_requests_are_answered_and_closed() {
    let s = app("refuse");
    let cases: [(String, &str); 7] = [
        ("GET /hello HTTP/1.1\r\nbad header line\r\n\r\n".to_string(), "HTTP/1.1 400 Bad Request"),
        ("NOT A REQUEST\r\n\r\n".to_string(), "HTTP/1.1 400 Bad Request"),
        (format!("GET /hello HTTP/1.1\r\nX-Big: {}\r\n\r\n", "a".repeat(3000)), "HTTP/1.1 431 Request Header Fields Too Large"),
        ("POST /echo HTTP/1.1\r\nContent-Length: 5000\r\n\r\n".to_string(), "HTTP/1.1 413 Payload Too Large"),
        ("POST /echo HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n".to_string(), "HTTP/1.1 501 Not Implemented"),
        ("POST /echo HTTP/1.1\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n".to_string(), "HTTP/1.1 400 Bad Request"),
        ("POST /echo HTTP/1.1\r\nContent-Length: 3x\r\n\r\n".to_string(), "HTTP/1.1 400 Bad Request"),
    ];
    for (raw, status) in cases {
        let (mut stream, mut reader) = connect(s.port);
        stream.write_all(raw.as_bytes()).unwrap();
        let r = read_reply(&mut reader, false).expect("no reply");
        assert_eq!(r.status, status, "{raw:?}");
        assert_eq!(r.header("connection"), Some("close"));
        assert!(closed(&mut reader), "{raw:?}");
    }
    let (mut stream, mut reader) = connect(s.port);
    stream.write_all(b"POST /echo HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut reader, false).unwrap().status, "HTTP/1.1 400 Bad Request");
    let (mut big, mut big_reader) = connect(s.port);
    big.write_all(b"POST /echo HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
    big.write_all(format!("{:x}\r\n", 5000).as_bytes()).unwrap();
    assert_eq!(read_reply(&mut big_reader, false).unwrap().status, "HTTP/1.1 413 Payload Too Large");
}

#[test]
fn idle_connections_close_and_half_requests_time_out() {
    let s = app("idle");
    let (_quiet, mut quiet_reader) = connect(s.port);
    assert!(closed(&mut quiet_reader));
    let (mut half, mut half_reader) = connect(s.port);
    half.write_all(b"GET /hello HTTP/1.1\r\nHost:").unwrap();
    let r = read_reply(&mut half_reader, false).unwrap();
    assert_eq!(r.status, "HTTP/1.1 408 Request Timeout");
    assert!(closed(&mut half_reader));
    let (mut body, mut body_reader) = connect(s.port);
    body.write_all(b"POST /echo HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc").unwrap();
    assert_eq!(read_reply(&mut body_reader, false).unwrap().status, "HTTP/1.1 408 Request Timeout");
    let (mut gone, mut gone_reader) = connect(s.port);
    gone.write_all(b"GET /hel").unwrap();
    gone.shutdown(std::net::Shutdown::Write).unwrap();
    assert!(closed(&mut gone_reader));
}

#[test]
fn connections_are_served_at_the_same_time() {
    let s = app("concurrent");
    let (_idle, _idle_reader) = connect(s.port);
    let (mut half, _half_reader) = connect(s.port);
    half.write_all(b"GET /hel").unwrap();
    let r = once(s.port, "GET /hello HTTP/1.1\r\n\r\n");
    assert_eq!(r.body, "hello");
}

#[test]
fn the_client_and_the_server_meet() {
    let source = r#"import std.sys.http_server as server
import http as http

fn handle(conn: server.Connection) {
    var requests = conn.requests()
    while true {
        match requests.next() {
            Ok(next) => {
                match next {
                    Some(req) => {
                        let who = req.query().get_or("name", "world")
                        let sent = req.respond(server.Response.text(200, "${req.method} ${who} ${req.body.len()}"))
                    }
                    None => { break }
                }
            }
            Err(e) => { break }
        }
    }
}

fn main() {
    var s = server.listen("127.0.0.1", 0).unwrap()
    let port = s.local_port().unwrap()
    scope {
        spawn {
            let a = http.get("http://127.0.0.1:${port}/x?name=zed").unwrap()
            println(a.text().unwrap())
            let b = http.post("http://127.0.0.1:${port}/x", "abc".bytes()).unwrap()
            println(b.text().unwrap())
        }
        var served = 0
        while served < 2 {
            let conn = s.accept().unwrap()
            handle(conn)
            served = served + 1
        }
    }
}
"#;
    let dir = std::env::temp_dir().join(format!("mote_httpd_meet_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    common::install_native_packages(&dir, &["http"]);
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).current_dir(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    assert_eq!(text.lines().collect::<Vec<_>>(), ["GET zed 0", "POST world 3"]);
}
