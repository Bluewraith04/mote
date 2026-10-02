//! Conversions through `T.from`, built in and declared.

use std::process::Command;

fn mote(cmd: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_conversions_{}_{}_{}", cmd, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(cmd).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(src: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", src, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn built_in_and_declared_from_convert() {
    let src = r#"class Kelvin { degrees: Float }
class Fahrenheit { degrees: Float }
class Celsius {
    degrees: Float
    pub fn from(t: Fahrenheit | Kelvin) -> Self {
        match t {
            f: Fahrenheit => { return Celsius { degrees: (f.degrees - 32.0) * 5.0 / 9.0 } }
            k: Kelvin => { return Celsius { degrees: k.degrees - 200.0 } }
        }
    }
}
pub fn main() {
    println([Int.from(3.9), Int.from(-2.5), Int.from(true), Int.from('A')])
    println([Float.from(7), Float.from(-1)])
    println([String.from(42), String.from([1, 2]), String.from(None)])
    let u: Float | Bool = false
    println(Int.from(u))
    println([Celsius.from(Kelvin { degrees: 300.0 }).degrees, Celsius.from(Fahrenheit { degrees: 212.0 }).degrees])
}
"#;
    let (ok, text) = mote("run", src, "run");
    assert!(ok && text == "[3, -2, 1, 65]\n[7.0, -1.0]\n[\"42\", \"[1, 2]\", \"null\"]\n0\n[100.0, 100.0]\n", "{text}");
}

#[test]
fn a_non_finite_float_faults() {
    let (ok, text) = mote("run", "pub fn main() {\n    println(Int.from(1.0 / 0.0))\n}\n", "inf");
    assert!(!ok && text.contains("is not finite"), "{text}");
}

#[test]
fn conversion_misuse_is_a_compile_error() {
    rejected(
        "pub fn main() {\n    println(Int.from(\"3\"))\n}\n",
        "string",
        "`Int.from` takes `Float | Bool | Char`, found `String`; to read a number from text, use `std.string.parse_int`",
    );
    rejected("pub fn main() {\n    println(Float.from(1.5))\n}\n", "float", "`Float.from` takes `Int`, found `Float`");
    rejected("pub fn main() {\n    println(Int.from(1, 2))\n}\n", "arity", "`Int.from` takes 1 argument, but 2 were given");
    let class = |from: &str| format!("class P {{\n    x: Int\n    {from}\n}}\npub fn main() {{}}\n");
    rejected(&class("pub fn from(self) -> Self { return self }"), "self", "`P.from` is static: it takes no `self`");
    rejected(&class("pub fn from(a: Int, b: Int) -> Self { return P { x: a } }"), "two", "`P.from` takes exactly one parameter");
    rejected(&class("pub fn from(a: String) -> P? { return None }"), "ret", "`P.from` returns `Self`; a conversion that can fail needs another name");
    rejected(
        "class P {\n    x: Int\n    pub fn from(a: Int | String) -> P { return P { x: 1 } }\n}\npub fn main() {\n    println(P.from(true).x)\n}\n",
        "arg",
        "expects 'Int | String', found 'Bool'",
    );
}
