//! One program uses every form, and each misuse is a compile error.

use std::process::Command;

fn mote(verb: &str, files: &[(&str, &str)], tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_sharing_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, source) in files {
        std::fs::write(dir.join(name), source).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const EVERY_FORM: &str = "\
struct Point {
    var x: Int
    y: Int
}

class Counter {
    var n: Int
    pub fn bump(var self) { self.n = self.n + 1 }
    pub fn get_n(self) -> Int { return self.n }
}

fn total(p: Point) -> Int { return p.x + p.y }
fn shift(var p: Point) { p.x += 10 }
fn bump(var n: Int) { n += 1 }

fn main() {
    var p = Point { x: 1, y: 2 }
    shift(p)
    println(total(p))
    var n = 1
    bump(n)
    println(n)
    let c = Counter { n: 0 }
    c.bump()
    println(c.get_n())

    var hits = Shared(0)
    let (tx, rx) = Channel<Int>(2)
    scope {
        let a = tx.clone()
        let b = tx.clone()
        spawn {
            hits.update(|h| { h += 1 })
            a.send(10)
        }
        spawn {
            hits.update(|h| { h += 1 })
            b.send(20)
        }
        tx.close()
        let waiter = spawn { return hits.wait_until(|h| h == 2) }
        println(waiter.join().unwrap())
        println(waiter.is_ready())
        var sum = 0
        for x in rx { sum = sum + x }
        println(sum)
    }
    println(hits.get())
}
";

#[test]
fn a_program_uses_every_form() {
    let (ok, text) = mote("run", &[("main.mote", EVERY_FORM)], "all");
    assert!(ok && text == "13\n2\n1\n2\ntrue\n30\n2\n", "{text}");
}

const POINT: &str = "struct Point {\n    var x: Int\n    y: Int\n}\n";

fn misuses() -> Vec<(&'static str, String, &'static str)> {
    vec![
        ("plain_param_write", format!("{POINT}fn f(p: Point) {{ p.x = 1 }}\nfn main() {{ }}\n"), "read-only"),
        ("let_struct_write", format!("{POINT}fn main() {{\n    let p = Point {{ x: 1, y: 2 }}\n    p.x = 5\n}}\n"), "let"),
        ("let_field_write", format!("{POINT}fn main() {{\n    var p = Point {{ x: 1, y: 2 }}\n    p.y = 5\n}}\n"), "y"),
        ("cell_read", format!("{POINT}fn main() {{\n    let s = Shared(Point {{ x: 1, y: 2 }})\n    println(s.x)\n}}\n"), "get()"),
        ("snapshot_write", "class Box { var n: Int }\nfn main() {\n    let s = Shared(Box { n: 1 })\n    let v = s.get()\n    v.n = 2\n}\n".into(), "is a snapshot"),
        ("update_on_let", "fn main() {\n    let s = Shared(0)\n    s.update(|n| { n += 1 })\n}\n".into(), "only a `var` can write a `Shared`"),
        ("spawn_captures_list", "fn main() {\n    var xs = [1]\n    scope {\n        spawn { xs.push(2) }\n    }\n}\n".into(), "Sendable"),
        ("send_a_list", "fn main() {\n    let (tx, rx) = Channel<List<Int>>(1)\n}\n".into(), "must be Sendable"),
        ("use_after_spawn", "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    scope {\n        spawn { tx.send(1) }\n        tx.send(2)\n    }\n}\n".into(), "moved into a `spawn`"),
        ("receiver_close", "fn main() {\n    let (tx, rx) = Channel<Int>(1)\n    rx.close()\n}\n".into(), "can't close"),
        ("channel_type", "fn main() {\n    let c: Channel<Int> = Channel(1)\n}\n".into(), "`Channel` is not a type"),
        ("mutex", "fn main() {\n    let m: Mutex<Int> = Mutex(0)\n}\n".into(), "there is no `Mutex`"),
        ("mut_param", "fn f(mut x: Int) { }\nfn main() { }\n".into(), "write `var x`"),
        ("ref_self", "struct P {\n    var x: Int\n    fn get(&self) -> Int { return 1 }\n}\nfn main() { }\n".into(), "write `self`"),
        ("shared_method", "fn main() {\n    let s = Shared(1)\n    let t = s.shared()\n}\n".into(), "no method `.shared()`"),
    ]
}

#[test]
fn every_misuse_is_a_compile_error() {
    for (tag, source, message) in misuses() {
        let (ok, text) = mote("check", &[("main.mote", &source)], tag);
        assert!(!ok && text.contains(message), "{tag}: expected `{message}`, got: {text}");
    }
}

#[test]
fn another_module_reaches_only_what_is_pub() {
    let lib = "pub struct Vault {\n    pub id: Int\n    secret: Int\n    pub var open: Bool\n\n    pub fn new(id: Int) -> Vault { return Vault { id: id, secret: 7, open: false } }\n    fn hidden(self) -> Int { return self.secret }\n}\n";
    let cases = [
        ("read_private", "let v = Vault.new(1)\n    println(v.secret)", "secret"),
        ("write_pub_field", "let v = Vault.new(1)\n    v.id = 2", "id"),
        ("call_private", "let v = Vault.new(1)\n    println(v.hidden())", "hidden"),
        ("literal", "let v = Vault { id: 1, secret: 2, open: true }", "Vault"),
    ];
    for (tag, body, message) in cases {
        let main = format!("import {{ Vault }} from .lib\n\nfn main() {{\n    {body}\n}}\n");
        let (ok, text) = mote("check", &[("main.mote", &main), ("lib.mote", lib)], tag);
        assert!(!ok && text.contains(message), "{tag}: expected `{message}`, got: {text}");
    }
    let fine = "import { Vault } from .lib\n\nfn main() {\n    let v = Vault.new(4)\n    println(v.id)\n}\n";
    let (ok, text) = mote("run", &[("main.mote", fine), ("lib.mote", lib)], "fine");
    assert!(ok && text == "4\n", "{text}");
}
