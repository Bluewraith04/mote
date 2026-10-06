//! Union types done bar: a settings loader using every union form, and each misuse as a compile error.

use std::process::Command;

fn mote(cmd: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_union_bar_{}_{}_{}", cmd, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(cmd).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const PROGRAM: &str = r#"
import { Any } from std.experimental.types
class Flag { name: String  on: Bool
    pub fn label(self) -> String { return "flag ${self.name}" }
}
class Limit { name: String  max: Int
    pub fn label(self) -> String { return "limit ${self.name}" }
}
type Setting = Flag | Limit
type Raw = Int | String | Bool

fn parse(name: String, raw: Raw) -> Setting | String {
    match raw {
        b: Bool => { return Flag { name: name, on: b } }
        n: Int if n >= 0 => { return Limit { name: name, max: n } }
        _: Int => { return "${name}: a limit can't be negative" }
        s: String => { return "${name}: can't read `${s}`" }
    }
    return "unreachable"
}

fn describe(s: Setting) -> String {
    let base = s.label()
    if s is Flag { return base + (s.on ? " on" : " off") }
    return "${base} <= ${s.max}"
}

fn find(xs: List<Setting>, name: String) -> Setting? {
    for x in xs {
        if x.name == name { return x }
    }
    return None
}

fn kind<T>(x: T | Int) -> String {
    if x is Int { return "a number" }
    return "something else"
}

pub fn main() {
    let raws: List<(String, Raw)> = [("debug", true), ("workers", 8), ("retries", -1), ("mode", "fast")]
    var settings: List<Setting> = []
    var problems: List<String> = []
    for pair in raws {
        let (name, raw) = pair
        let r = parse(name, raw)
        if r is String { problems.push(r) } else { settings.push(r) }
    }
    for s in settings { println(describe(s)) }
    for p in problems { println(p) }

    let w = find(settings, "workers")
    if w != null && w is Limit { println("workers max ${w.max}") }
    println(find(settings, "missing") == null)

    let loaded: Any = "slow"
    let raw: Raw = loaded
    let show = |v: (Int | String)| "<${v}>"
    let pick: Int | String = settings.len() > 1 ? settings.len() : "few"
    println([show(pick), raw.to_string(), kind("x"), kind(3)])
}
"#;

#[test]
fn a_settings_loader_uses_every_union_form() {
    let (ok, text) = mote("run", PROGRAM, "run");
    let want = "flag debug on\nlimit workers <= 8\nretries: a limit can't be negative\nmode: can't read `fast`\nworkers max 8\ntrue\n[\"<2>\", \"slow\", \"something else\", \"a number\"]\n";
    assert!(ok && text == want, "{text}");
}

fn misuse(from: &str, to: &str, tag: &str, message: &str) {
    assert!(PROGRAM.contains(from), "`{from}` is not in the program");
    let (ok, text) = mote("check", &PROGRAM.replace(from, to), tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn each_misuse_is_a_compile_error() {
    misuse("type Raw = Int | String | Bool", "type Raw = Int | String? | Bool", "optional", "write `(Int | String | Bool)?`");
    misuse("        _: Int => { return \"${name}: a limit can't be negative\" }\n", "", "uncovered", "`Int` is not covered");
    misuse("_: Int =>", "_: Float =>", "never", "is never a `Float`");
    misuse("\"${base} <= ${s.max}\"", "\"${base} <= ${s.on}\"", "rest", "Field 'on' not found on type 'Limit'");
    misuse("let base = s.label()", "let base = s.on", "member", "`.on` is not on every member of `Flag | Limit`: `Limit` has none");
    misuse("settings.push(r)", "problems.push(r)", "else", "expects 'String'");
    misuse("let show = |v: (Int | String)| \"<${v}>\"", "let show = |v: (Int | String)| v + 1", "op", "`+` needs a narrowed operand");
    misuse("let pick: Int | String = ", "let pick = ", "inferred", "for example `Int | String`");
}
