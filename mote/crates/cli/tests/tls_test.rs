//! TLS sockets and `std.sys.tls` identities, against rustls peers written in Rust.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned};

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

struct Running {
    child: Child,
    port: u16,
    cert: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(source: &str, tag: &str) -> Running {
    let dir = std::env::temp_dir().join(format!("mote_tls_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let cert_path = dir.join("cert.pem");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(dir.join("main.mote"))
        .env("CERT_OUT", &cert_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    let port = line.trim().parse().unwrap_or_else(|_| panic!("the server printed {line:?}"));
    let cert = std::fs::read_to_string(&cert_path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    Running { child, port, cert }
}

fn client(port: u16, cert_pem: &str, name: &str) -> StreamOwned<ClientConnection, TcpStream> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(cert_pem.as_bytes()).unwrap()).unwrap();
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let conn = ClientConnection::new(Arc::new(config), ServerName::try_from(name.to_string()).unwrap()).unwrap();
    let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    StreamOwned::new(conn, tcp)
}

const ECHO: &str = r#"import std.sys.net as net
import std.sys.tls as tls
import std.sys.io as io
import std.sys.env as env
import std.sys.fs as fs

fn echo(conn: net.TcpStream) {
    while true {
        match conn.read(1024) {
            Ok(b) => {
                if b.len() == 0 { break }
                let sent = conn.write(b)
            }
            Err(e) => { break }
        }
    }
    let closed = conn.close()
}

fn main() {
    let id = tls.self_signed(["localhost"]).unwrap()
    let out = env.get_var("CERT_OUT").unwrap()
    let saved = fs.write_text(out, id.cert_pem())
    let l = net.listen_tls("127.0.0.1", 0, id).unwrap()
    println(l.local_port().unwrap())
    io.stdout().flush()
    scope {
        while true {
            let conn = l.accept().unwrap()
            spawn { echo(conn) }
        }
    }
}
"#;

#[test]
fn a_tls_listener_echoes_to_a_client_that_trusts_its_certificate() {
    let s = start(ECHO, "echo");
    let mut tls = client(s.port, &s.cert, "localhost");
    tls.write_all(b"ping").unwrap();
    let mut buf = [0u8; 4];
    tls.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ping");
    let big = vec![b'x'; 200_000];
    tls.write_all(&big).unwrap();
    let mut got = vec![0u8; big.len()];
    tls.read_exact(&mut got).unwrap();
    assert_eq!(got, big);
    tls.conn.send_close_notify();
    tls.flush().unwrap();
    let mut rest = Vec::new();
    tls.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
}

#[test]
fn a_client_that_does_not_trust_the_certificate_cannot_connect() {
    let s = start(ECHO, "untrusted");
    let other = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let mut tls = client(s.port, &other.cert.pem(), "localhost");
    let err = tls.write_all(b"ping").and_then(|_| tls.flush()).and_then(|_| tls.read(&mut [0u8; 4]).map(|_| ())).unwrap_err();
    assert!(["UnknownIssuer", "BadSignature", "alert"].iter().any(|w| err.to_string().contains(w)), "{err}");
    let mut wrong = client(s.port, &s.cert, "example.com");
    let err = wrong.write_all(b"ping").and_then(|_| wrong.flush()).and_then(|_| wrong.read(&mut [0u8; 4]).map(|_| ())).unwrap_err();
    assert!(err.to_string().contains("not valid for name") || err.to_string().contains("alert"), "{err}");
}

#[test]
fn several_tls_connections_are_served_at_once() {
    let s = start(ECHO, "many");
    let mut idle = client(s.port, &s.cert, "localhost");
    idle.write_all(b"a").unwrap();
    let mut half = client(s.port, &s.cert, "localhost");
    half.write_all(b"b").unwrap();
    let mut buf = [0u8; 1];
    half.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"b");
    idle.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"a");
}

const HTTPS: &str = r#"import std.sys.http_server as server
import std.sys.tls as tls
import std.sys.io as io
import std.sys.env as env
import std.sys.fs as fs

fn handle(conn: server.Connection) {
    var requests = conn.requests()
    while true {
        match requests.next() {
            Ok(next) => {
                match next {
                    Some(req) => {
                        let sent = req.respond(server.Response.text(200, "secure ${req.path} ${req.body.len()}"))
                    }
                    None => { break }
                }
            }
            Err(e) => { break }
        }
    }
}

fn main() {
    let id = tls.self_signed(["localhost"]).unwrap()
    let saved = fs.write_text(env.get_var("CERT_OUT").unwrap(), id.cert_pem())
    let s = server.listen_tls("127.0.0.1", 0, id).unwrap()
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

fn read_reply(tls: &mut StreamOwned<ClientConnection, TcpStream>) -> (String, String) {
    let mut reader = BufReader::new(tls);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    let mut length = 0;
    loop {
        let mut line = String::new();
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
    (status.trim_end().to_string(), String::from_utf8(body).unwrap())
}

#[test]
fn an_https_server_answers_requests_on_one_connection() {
    let s = start(HTTPS, "https");
    let mut tls = client(s.port, &s.cert, "localhost");
    tls.write_all(b"GET /one HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    assert_eq!(read_reply(&mut tls), ("HTTP/1.1 200 OK".to_string(), "secure /one 0".to_string()));
    tls.write_all(b"POST /two HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nhello").unwrap();
    assert_eq!(read_reply(&mut tls), ("HTTP/1.1 200 OK".to_string(), "secure /two 5".to_string()));
}

#[test]
fn plain_http_to_an_https_port_is_dropped() {
    let s = start(HTTPS, "plain");
    let mut tcp = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let mut reply = Vec::new();
    let _ = tcp.read_to_end(&mut reply);
    assert!(!String::from_utf8_lossy(&reply).contains("200 OK"));
}

fn mote(body: &str, tag: &str) -> Vec<String> {
    let helpers = "fn why(r: Result<tls.Identity, Error>) -> String {
    match r {
        Ok(i) => { return \"ok\" }
        Err(e) => { return e.message }
    }
}

fn kind(r: Result<tls.Identity, Error>) -> ErrorKind {
    match r {
        Ok(i) => { return ErrorKind.Other }
        Err(e) => { return e.kind }
    }
}
";
    let source = format!("import std.sys.net as net\nimport std.sys.tls as tls\nimport std.sys.fs as fs\n\n{helpers}\nfn main() {{\n{body}}}\n");
    let dir = std::env::temp_dir().join(format!("mote_tlsc_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().map(String::from).collect()
}

fn rust_server() -> u16 {
    let made = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(made.signing_key.serialize_pem().as_bytes()).unwrap();
    let cert = CertificateDer::from_pem_slice(made.cert.pem().as_bytes()).unwrap();
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let mut tls = StreamOwned::new(ServerConnection::new(config.clone()).unwrap(), tcp);
            let _ = tls.read(&mut [0u8; 1]);
        }
    });
    port
}

#[test]
fn the_client_refuses_a_certificate_it_does_not_trust() {
    let port = rust_server();
    let got = mote(
        &format!(
            "    match net.connect_tls(\"localhost\", {port}) {{
        Ok(c) => {{ println(\"connected\") }}
        Err(e) => {{
            println(e.kind == ErrorKind.InvalidData)
            println(e.message.contains(\"UnknownIssuer\"))
        }}
    }}
    match net.connect_tls(\"127.0.0.1\", 1) {{
        Ok(c) => {{ println(\"connected\") }}
        Err(e) => {{ println(\"refused\") }}
    }}
"
        ),
        "untrusted",
    );
    assert_eq!(got, ["true", "true", "refused"]);
}

#[test]
fn identities_load_or_say_why_not() {
    let a = rcgen::generate_simple_self_signed(vec!["a.test".to_string()]).unwrap();
    let b = rcgen::generate_simple_self_signed(vec!["b.test".to_string()]).unwrap();
    let dir = std::env::temp_dir().join(format!("mote_tlsid_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.crt"), a.cert.pem()).unwrap();
    std::fs::write(dir.join("a.key"), a.signing_key.serialize_pem()).unwrap();
    std::fs::write(dir.join("b.key"), b.signing_key.serialize_pem()).unwrap();
    let d = dir.display();
    let got = mote(
        &format!(
            "    let ok = tls.identity_from_files(\"{d}/a.crt\", \"{d}/a.key\")
    println(ok.is_ok())
    println(ok.unwrap().cert_pem().starts_with(\"-----BEGIN CERTIFICATE-----\"))
    let missing = tls.identity_from_files(\"{d}/none.crt\", \"{d}/a.key\")
    println(kind(missing) == ErrorKind.NotFound)
    let crt = fs.read_text(\"{d}/a.crt\").unwrap()
    let bkey = fs.read_text(\"{d}/b.key\").unwrap()
    println(why(tls.identity_from_pem(crt, bkey)).contains(\"certificate and key\"))
    println(why(tls.identity_from_pem(\"\", bkey)).contains(\"no certificate\"))
    println(why(tls.identity_from_pem(crt, \"\")).contains(\"private key\"))
    let none: List<String> = []
    println(tls.self_signed(none).is_err())
    let made = tls.self_signed([\"x.test\", \"127.0.0.1\"]).unwrap()
    println(made.key_pem().contains(\"PRIVATE KEY\"))
    println(tls.identity_from_pem(made.cert_pem(), made.key_pem()).is_ok())
    println(net.listen_tls(\"127.0.0.1\", 0, made).is_ok())
"
        ),
        "identity",
    );
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(got, ["true", "true", "true", "true", "true", "true", "true", "true", "true", "true"]);
}
