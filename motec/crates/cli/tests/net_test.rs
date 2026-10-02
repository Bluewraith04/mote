//! `std.sys.net` TCP through `mote run` on real loopback and on the fake network.

use std::process::Command;
use std::sync::Arc;

use isa::value::TypeRegistry;
use modules::MultiFileCompiler;
use platform::FakePlatform;

const HEADER: &str = "import std.sys.net as net\nimport { TcpStream, TcpListener } from std.sys.net\n\n";

fn run_real(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_net_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("{HEADER}{body}")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_fake(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_net_fake_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(&main_file, format!("{HEADER}{body}")).unwrap();
    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let fake = Arc::new(FakePlatform::new(1));
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_platform(fake.clone());
    rt.run_entry_on(1).unwrap();
    std::fs::remove_dir_all(dir).ok();
    fake.stdout().trim().to_string()
}

const ROUND_TRIP: &str = r#"
fn main() {
    let l: TcpListener = net.listen("127.0.0.1", 0).unwrap()
    let port = l.local_port().unwrap()
    let c: TcpStream = net.connect("127.0.0.1", port).unwrap()
    let s: TcpStream = l.accept().unwrap()
    c.write_text("ping").unwrap()
    c.shutdown_write().unwrap()
    let got: Bytes = s.read_to_end().unwrap()
    println(got.decode().unwrap())
    s.write_text("pong").unwrap()
    s.close().unwrap()
    let back: Bytes = c.read_to_end().unwrap()
    println(back.decode().unwrap())
    println(c.close().is_ok())
    println(c.close().is_err())
    l.close().unwrap()
    println(net.connect("127.0.0.1", port).is_err())
}
"#;

#[test]
fn a_loopback_exchange_on_the_real_network() {
    assert_eq!(run_real("round_trip", ROUND_TRIP), "ping\npong\ntrue\ntrue\ntrue");
}

#[test]
fn the_same_program_on_the_fake_network() {
    assert_eq!(run_fake("round_trip", ROUND_TRIP), "ping\npong\ntrue\ntrue\ntrue");
}

const UDP: &str = r#"
import { UdpSocket, Datagram } from std.sys.net

fn main() {
    let a: UdpSocket = net.bind("127.0.0.1", 0).unwrap()
    let b: UdpSocket = net.bind("127.0.0.1", 0).unwrap()
    let pa = a.local_port().unwrap()
    let pb = b.local_port().unwrap()
    let msg = Bytes()
    msg.push(104)
    msg.push(105)
    println(a.send_to("127.0.0.1", pb, msg).unwrap())
    let d: Datagram = b.recv_from(64).unwrap()
    println(d.bytes.decode().unwrap())
    println(d.sender == "127.0.0.1:" + pa.to_string())
    a.close().unwrap()
    b.close().unwrap()
}
"#;

#[test]
fn a_udp_datagram_on_the_real_network() {
    assert_eq!(run_real("udp", UDP), "2\nhi\ntrue");
}

#[test]
fn a_udp_datagram_on_the_fake_network() {
    assert_eq!(run_fake("udp", UDP), "2\nhi\ntrue");
}

#[test]
fn a_port_out_of_range_is_a_runtime_error_not_a_wrapped_port() {
    let dir = std::env::temp_dir().join(format!("mote_net_port_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let body = "fn main() {\n    println(net.listen(\"127.0.0.1\", 70000).is_err())\n}\n";
    std::fs::write(dir.join("main.mote"), format!("{HEADER}{body}")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!out.status.success(), "an out-of-range port ran");
    assert!(String::from_utf8_lossy(&out.stderr).contains("port must be 0 to 65535"));
}

const ECHO_SERVER: &str = r#"
fn handle(conn: TcpStream) {
    let got: Bytes = conn.read_to_end().unwrap()
    let text: String = got.decode().unwrap()
    conn.write_text("echo:" + text).unwrap()
    conn.close().unwrap()
}

fn client(port: Int, n: Int) -> Int {
    let c: TcpStream = net.connect("127.0.0.1", port).unwrap()
    c.write_text("msg" + n.to_string()).unwrap()
    c.shutdown_write().unwrap()
    let back: Bytes = c.read_to_end().unwrap()
    c.close().unwrap()
    if back.decode().unwrap() == "echo:msg" + n.to_string() { return 1 }
    return 0
}

fn serve(l: TcpListener, count: Int) {
    var i = 0
    while i < count {
        let conn: TcpStream = l.accept().unwrap()
        spawn { handle(conn) }
        i = i + 1
    }
}

fn main() {
    let l: TcpListener = net.listen("127.0.0.1", 0).unwrap()
    let port = l.local_port().unwrap()
    let (tx, rx) = Channel<Int>(16)
    var total = 0
    scope {
        spawn { serve(l, 8) }
        var n = 0
        while n < 8 {
            let k = n
            let out = tx.clone()
            spawn { out.send(client(port, k)) }
            n = n + 1
        }
        var seen = 0
        while seen < 8 {
            total = total + rx.recv().unwrap()
            seen = seen + 1
        }
    }
    l.close().unwrap()
    println(total)
}
"#;

#[test]
fn a_task_per_connection_echo_server_serves_more_clients_than_the_pool_has_threads() {
    assert_eq!(run_real("echo_server", ECHO_SERVER), "8");
}
