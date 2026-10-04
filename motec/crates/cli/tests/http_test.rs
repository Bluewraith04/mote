//! The package `http` against a small local server.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::Command;

fn respond(path: &str, method: &str, headers: &[(String, String)], body: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).unwrap_or_default();
    match path {
        "/hello" => (200, vec![("Content-Type".into(), "text/plain".into()), ("X-Greeting".into(), "hi".into())], b"hello".to_vec()),
        "/missing" => (404, vec![], b"nope".to_vec()),
        "/redirect" => (302, vec![("Location".into(), "/hello".into())], vec![]),
        "/loop" => (302, vec![("Location".into(), "/loop".into())], vec![]),
        "/echo" => {
            let text = format!("{method} ct={} token={} len={}\n{}", header("content-type"), header("x-token"), body.len(), String::from_utf8_lossy(body));
            (200, vec![], text.into_bytes())
        }
        "/json" => (200, vec![("Content-Type".into(), "application/json".into())], br#"{"a":[1,2],"ok":true}"#.to_vec()),
        "/slow" => {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            (200, vec![], b"late".to_vec())
        }
        "/big" => (200, vec![], vec![b'x'; 10_000]),
        "/binary" => (200, vec![], (0..=255u8).collect()),
        "/multi" => (200, vec![("Set-Cookie".into(), "a=1".into()), ("Set-Cookie".into(), "b=2".into())], vec![]),
        "/head" if method == "HEAD" => (200, vec![("X-Method".into(), "head".into())], vec![]),
        _ => (500, vec![], b"unrouted".to_vec()),
    }
}

fn serve_connection(stream: std::net::TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let len: usize = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).unwrap();
    let (status, extra, payload) = respond(&path, &method, &headers, &body);
    let mut out = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n", payload.len());
    for (k, v) in extra {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let mut stream = stream;
    let _ = stream.write_all(out.as_bytes());
    if method != "HEAD" {
        let _ = stream.write_all(&payload);
    }
}

fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve_connection(stream));
        }
    });
    port
}

fn run(source: &str, tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("mote_http_{}_{}", tag, std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    common::install_native_packages(&dir, &["http"]);
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).current_dir(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().map(String::from).collect()
}

fn program(port: u16, body: &str) -> String {
    format!("import http as http\nimport std.data.json as json\n\nlet BASE = \"http://127.0.0.1:{port}\"\n\nfn main() {{\n{body}}}\n")
}

#[test]
fn a_get_answers_status_headers_and_text() {
    let port = serve();
    let src = program(
        port,
        "    let r = http.get(BASE + \"/hello\").unwrap()
    println(r.status)
    println(r.ok())
    println(r.text().unwrap())
    println(r.header(\"X-Greeting\").unwrap())
    println(r.header(\"x-nothing\").is_none())
    println(r.header_map()[\"content-type\"])
",
    );
    assert_eq!(run(&src, "get"), ["200", "true", "hello", "hi", "true", "text/plain"]);
}

#[test]
fn a_bad_status_is_an_answer_not_an_error() {
    let port = serve();
    let src = program(
        port,
        "    let r = http.get(BASE + \"/missing\").unwrap()
    println(r.status)
    println(r.ok())
    println(r.text().unwrap())
    match r.error_for_status() {
        Ok(x) => { println(\"ok\") }
        Err(e) => { println(e.message.ends_with(\"/missing\")) }
    }
    println(http.get(BASE + \"/missing\").unwrap().error_for_status().is_err())
    println(http.get(BASE + \"/hello\").unwrap().error_for_status().is_ok())
",
    );
    assert_eq!(run(&src, "status"), ["404", "false", "nope", "true", "true", "true"]);
}

#[test]
fn redirects_are_followed_up_to_a_limit() {
    let port = serve();
    let src = program(
        port,
        "    println(http.get(BASE + \"/redirect\").unwrap().text().unwrap())
    println(http.request(\"GET\", BASE + \"/loop\").send().is_err())
    let first = http.request(\"GET\", BASE + \"/redirect\").max_redirects(0).send().unwrap()
    let target = first.header(\"location\").unwrap()
    println(\"${first.status} ${target}\")
",
    );
    assert_eq!(run(&src, "redirect"), ["hello", "true", "302 /hello"]);
}

#[test]
fn bodies_headers_json_and_forms_are_sent() {
    let port = serve();
    let src = program(
        port,
        "    println(http.post(BASE + \"/echo\", \"raw\".bytes()).unwrap().text().unwrap())
    println(http.request(\"PUT\", BASE + \"/echo\").header(\"X-Token\", \"abc\").text(\"note\").send().unwrap().text().unwrap())
    println(http.post_json(BASE + \"/echo\", json.parse(\"{\\\"k\\\": [1, 2]}\").unwrap()).unwrap().text().unwrap())
    println(http.post_form(BASE + \"/echo\", {\"a\": \"1 2\", \"b\": \"x&y\"}).unwrap().text().unwrap())
    println(http.delete(BASE + \"/echo\").unwrap().text().unwrap())
    println(http.request(\"patch\", BASE + \"/echo\").header(\"x-token\", \"one\").header(\"X-TOKEN\", \"two\").send().unwrap().text().unwrap())
",
    );
    assert_eq!(
        run(&src, "send"),
        [
            "POST ct= token= len=3",
            "raw",
            "PUT ct=text/plain; charset=utf-8 token=abc len=4",
            "note",
            "POST ct=application/json token= len=11",
            "{\"k\":[1,2]}",
            "POST ct=application/x-www-form-urlencoded token= len=13",
            "a=1+2&b=x%26y",
            "DELETE ct= token= len=0",
            "",
            "PATCH ct= token=two len=0",
            "",
        ]
    );
}

#[test]
fn a_json_answer_parses() {
    let port = serve();
    let src = program(
        port,
        "    let j = http.get(BASE + \"/json\").unwrap().json().unwrap()
    println(json.to_text(j).unwrap())
    println(http.get(BASE + \"/hello\").unwrap().json().is_err())
",
    );
    assert_eq!(run(&src, "json"), ["{\"a\":[1,2],\"ok\":true}", "true"]);
}

#[test]
fn binary_bodies_head_and_repeated_headers() {
    let port = serve();
    let src = program(
        port,
        "    let b = http.get(BASE + \"/binary\").unwrap()
    println(b.body.len())
    println(b.body.get(255))
    println(b.text().is_err())
    println(http.head(BASE + \"/head\").unwrap().header(\"x-method\").unwrap())
    println(http.get(BASE + \"/multi\").unwrap().header_map()[\"set-cookie\"])
",
    );
    assert_eq!(run(&src, "binary"), ["256", "255", "true", "head", "a=1, b=2"]);
}

#[test]
fn limits_and_failures_are_errors() {
    let port = serve();
    let src = program(
        port,
        "    match http.request(\"GET\", BASE + \"/slow\").timeout(1).send() {
        Ok(r) => { println(\"answered\") }
        Err(e) => { println(e.kind == ErrorKind.TimedOut) }
    }
    println(http.request(\"GET\", BASE + \"/big\").max_body(100).send().is_err())
    println(http.request(\"GET\", BASE + \"/big\").max_body(10000).send().unwrap().body.len())
    println(http.get(\"http://127.0.0.1:1/\").is_err())
    println(http.get(\"not a url\").is_err())
    println(http.get(\"ftp://127.0.0.1/\").is_err())
",
    );
    assert_eq!(run(&src, "limits"), ["true", "true", "10000", "true", "true", "true"]);
}

#[test]
fn query_strings_percent_encode_in_order() {
    let src = "import http as http

fn main() {
    println(http.query({\"q\": \"a b\", \"x\": \"1&2=3\", \"é\": \"ü\"}))
    println(http.query(Map<String, String>()))
}
";
    assert_eq!(run(src, "query"), ["q=a+b&x=1%262%3D3&%C3%A9=%C3%BC", ""]);
}
