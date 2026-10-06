//! Invariant mutable containers, no silent `Any`, recorded type arguments, checks out of `Any`.

use std::process::Command;

fn mote(verb: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_trusted_{}_{}_{}", verb, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn rejected(src: &str, tag: &str, message: &str) {
    let (ok, text) = mote("check", src, tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn a_list_of_int_is_not_a_list_of_any() {
    rejected(
        "import { Any } from std.experimental.types\nfn add_text(xs: List<Any>) { xs.push(\"oops\") }\n\npub fn main() {\n    let nums: List<Int> = [1, 2]\n    add_text(nums)\n}\n",
        "list",
        "`add_text` argument 1 expects 'List<Any>', found 'List<Int>'",
    );
    for (ty, value) in [("Map<String, Any>", "m"), ("Set<Any>", "s"), ("Receiver<Any>", "c"), ("Sender<Any>", "t"), ("Shared<Any>", "x")] {
        let src = format!(
            "import {{ Any }} from std.experimental.types\npub fn main() {{\n    let m: Map<String, Int> = Map()\n    let s: Set<Int> = Set()\n    let (t, c) = Channel<Int>(1)\n    let x = Shared(0)\n    let w: {ty} = {value}\n}}\n"
        );
        rejected(&src, &format!("inv_{value}"), &format!("of type '{ty}'"));
    }
}

#[test]
fn read_only_handles_stay_covariant() {
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    scope {\n        let t: Task<Int> = spawn { 41 + 1 }\n        let u: Task<Any> = t\n        println(u.join().unwrap())\n    }\n}\n";
    let (ok, text) = mote("run", src, "task");
    assert!(ok && text == "42\n", "{text}");
}

#[test]
fn a_generic_function_takes_any_list() {
    let src = "fn count<T>(xs: List<T>) -> Int { return xs.len() }\n\npub fn main() {\n    println(count([1, 2, 3]) + count([\"a\"]))\n}\n";
    let (ok, text) = mote("run", src, "generic");
    assert!(ok && text == "4\n", "{text}");
}

#[test]
fn empty_constructors_take_their_type_from_context() {
    let src = r#"fn find(k: Int) -> Result<Int, Error> {
    return Ok(k)
}

fn first(xs: List<Int>) -> Option<Int> {
    return xs.first()
}

fn wrap(n: Int) -> Result<Option<Int>, String> {
    return Ok(Some(n))
}

pub fn main() {
    let xs: List<Int> = []
    let m: Map<String, Int> = {}
    let (tx, rx) = Channel<String>(1)
    tx.send("hi")
    println(find(3).unwrap() + xs.len() + m.len())
    println(first([]).is_none())
    println(rx.recv().unwrap())
    println(wrap(5).unwrap().unwrap())
    let nested = [[], [1]]
    println(nested)
}
"#;
    let (ok, text) = mote("run", src, "context");
    assert!(ok && text == "3\ntrue\nhi\n5\n[[], [1]]\n", "{text}");
}

#[test]
fn a_type_nothing_fixes_is_an_error() {
    for (body, name, shown) in [
        ("let m = Map()", "m", "Map<_, _>"),
        ("var xs = []", "xs", "List<_>"),
        ("let (tx, rx) = Channel(2)", "tx", "Sender<_>"),
        ("let r = Ok(1)", "r", "Result<Int, _>"),
        ("let n = None", "n", "_?"),
        ("var n = null", "n", "_?"),
    ] {
        rejected(
            &format!("pub fn main() {{\n    {body}\n}}\n"),
            name,
            &format!("the type of `{name}` is not fully known (`{shown}`); annotate it"),
        );
    }
}

#[test]
fn a_generic_result_type_comes_from_the_binding() {
    let src = "fn empty<T>() -> List<T> {\n    let out: List<T> = []\n    return out\n}\n\npub fn main() {\n    let xs: List<String> = empty()\n    xs.push(\"a\")\n    println(xs)\n}\n";
    let (ok, text) = mote("run", src, "ret");
    assert!(ok && text == "[\"a\"]\n", "{text}");
    rejected(
        "fn empty<T>() -> List<T> {\n    let out: List<T> = []\n    return out\n}\n\npub fn main() {\n    let xs = empty()\n}\n",
        "unbound",
        "the type of `xs` is not fully known (`List<_>`)",
    );
}

#[test]
fn a_bare_generic_name_is_an_error() {
    rejected(
        "fn f() -> Result {\n    return Ok(1)\n}\n\npub fn main() {\n    println(f())\n}\n",
        "bare",
        "`Result` needs its type arguments, as in `Result<T, E>`",
    );
}

#[test]
fn values_record_their_type_arguments() {
    let src = r#"import { Any } from std.experimental.types
struct Box<T> {
    v: T
}

pub fn main() {
    let r: Result<Int, String> = Ok(1)
    let a: Any = []
    let (tx, rx) = Channel<Bool>(1)
    for v in [typeof([1]), typeof({"a": 1.5}), typeof(Box { v: "s" }), typeof(r), typeof((1, 'c')), typeof(a), typeof(rx), typeof(tx)] {
        println(v)
    }
    scope {
        println(typeof(spawn { 42 }))
    }
}
"#;
    let (ok, text) = mote("run", src, "record");
    let want = "List<Int>\nMap<String, Float>\nBox<String>\nResult<Int, String>\n(Int, Char)\nList<_>\nReceiver<Bool>\nSender<Bool>\nTask<Int>\n";
    assert!(ok && text == want, "{text}");
}

#[test]
fn a_boundary_fault_names_the_full_type() {
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    let x: Any = [\"a\"]\n    let m: Map<String, Int> = x\n}\n";
    let (ok, text) = mote("run", src, "fault");
    assert!(!ok && text.contains("expected `Map<String, Int>`, found `List<String>`"), "{text}");
}

#[test]
fn a_recorded_value_equals_an_unrecorded_one() {
    let src = "fn wrap<T>(x: T) -> List<T> {\n    return [x]\n}\n\npub fn main() {\n    println(wrap(1) == [1])\n    println(Some(wrap(2)) == Some([2]))\n}\n";
    let (ok, text) = mote("run", src, "eq");
    assert!(ok && text == "true\ntrue\n", "{text}");
}

#[test]
fn generic_code_records_the_caller_s_type_arguments() {
    let src = r#"import std.iter as iter

struct Box<T> {
    v: T
}

class Stack<T> {
    var items: List<T>

    fn top_box(self) -> Box<List<T>> {
        return Box { v: [self.items.get(0)] }
    }

    fn pair<U>(self, u: U) -> (T, U) {
        return (self.items.get(0), u)
    }
}

fn wrap<T>(x: T) -> List<T> {
    return [x]
}

fn nest<T>(x: T) -> List<List<T>> {
    return [wrap(x)]
}

fn later<T>(x: T) -> () -> List<T> {
    return || [x]
}

fn in_task<T>(x: T) -> List<T> {
    var out: List<T> = []
    scope {
        let t = spawn {
            let e: List<T> = []
            e
        }
        out = t.join().unwrap()
    }
    return out
}

fn count(...xs: Int) -> String {
    return typeof(xs)
}

fn gen<T>(x: T) -> Stream<T> {
    yield x
}

fn ok<T>(x: T) -> Result<T, String> {
    return Ok(x)
}

pub fn main() {
    let s = Stack { items: [1, 2] }
    let f: (Int) -> List<Int> = wrap
    let r: Result<List<Int>, String> = ok([])
    let seen = [
        typeof(wrap("s")), typeof(nest(2.5)), typeof(later('c')()), typeof(in_task(1)),
        typeof(s.top_box()), typeof(s.pair("x")), count(1, 2), typeof(gen(1)),
        typeof("a,b".split(",")), typeof(f(1)), typeof(iter.map([1], |x| wrap(x))), typeof(r),
    ]
    for t in seen {
        println(t)
    }
}
"#;
    let (ok, text) = mote("run", src, "v3");
    let want = [
        "List<String>", "List<List<Float>>", "List<Char>", "List<Int>", "Box<List<Int>>", "(Int, String)", "List<Int>",
        "Stream<Int>", "List<String>", "List<Int>", "List<List<Int>>", "Result<List<Int>, String>",
    ];
    assert!(ok && text == want.join("\n") + "\n", "{text}");
}

#[test]
fn a_generic_function_value_takes_its_type_from_context() {
    rejected(
        "fn wrap<T>(x: T) -> List<T> {\n    return [x]\n}\n\npub fn main() {\n    let f = wrap\n}\n",
        "fnvalue",
        "the type of `f` is not fully known (`Send (_) -> List<_>`)",
    );
    rejected(
        "fn wrap<T>(x: T) -> List<T> {\n    return [x]\n}\n\npub fn main() {\n    let f: (Int) -> List<Int> = wrap\n    let n: Int = f\n}\n",
        "fntyped",
        "cannot initialize variable 'n'",
    );
}

#[test]
fn an_any_value_faults_at_the_boundary() {
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    let x: Any = [\"a\", \"b\"]\n    let ys: List<Int> = x\n    let n: Int = ys.get(0)\n    println(n)\n}\n";
    let (ok, text) = mote("run", src, "boundary");
    assert!(!ok && text.contains("expected `List<Int>`, found `List<String>`") && text.contains("main.mote:4:25"), "{text}");
    assert!(!text.contains("\na\n"), "{text}");
}

#[test]
fn every_exit_from_any_is_checked() {
    let exits = [
        "var n: Int = 0\n    n = a",
        "let xs: List<Int> = [1]\n    xs.push(a)",
        "let xs: List<Int> = [1]\n    xs[0] = a",
        "let p = P { x: a }",
        "let p = P { x: 1 }\n    p.x = a",
        "let m: Map<String, Int> = Map()\n    m.set(\"k\", a)",
        "let f: (Int) -> Int = |v| a\n    f(1)",
        "let o: Option<Int> = a",
    ];
    for (i, exit) in exits.iter().enumerate() {
        let src = format!("import {{ Any }} from std.experimental.types\nclass P {{\n    var x: Int\n}}\n\npub fn main() {{\n    let a: Any = \"s\"\n    {exit}\n    println(\"passed\")\n}}\n");
        let (ok, text) = mote("run", &src, &format!("exit{i}"));
        assert!(!ok && text.contains("found `String`") && !text.contains("passed"), "{exit}: {text}");
    }
}

#[test]
fn a_check_compares_the_whole_type() {
    let faults = [
        ("[1]", "List<Any>"),
        ("[[1]]", "List<List<String>>"),
        ("(1, \"s\")", "(Int, Int)"),
        ("|n: Int| n", "(String) -> Int"),
        ("\"s\"", "Int?"),
        ("Some(1)", "Option<String>"),
    ];
    for (i, (value, ty)) in faults.iter().enumerate() {
        let src = format!("import {{ Any }} from std.experimental.types\npub fn main() {{\n    let a: Any = {value}\n    let x: {ty} = a\n}}\n");
        let (ok, text) = mote("run", &src, &format!("whole{i}"));
        let shown = ty.replace("Option<String>", "String?");
        assert!(!ok && text.contains(&format!("expected `{shown}`")), "{ty}: {text}");
    }
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    var a: Any = (1, \"s\")\n    let t: (Int, Any) = a\n    a = |n: Int| n\n    let f: (Int) -> Null = a\n    a = null\n    let n: Int? = a\n    var h: Any = 0\n    scope {\n        h = spawn { 1 }\n    }\n    let k: Task<Any> = h\n    println(\"passed\")\n}\n";
    let (ok, text) = mote("run", src, "whole_ok");
    assert!(ok && text == "passed\n", "{text}");
}

#[test]
fn an_open_type_argument_is_fixed_by_its_first_use() {
    let src = "import { Any } from std.experimental.types\npub fn main() {\n    let r: Any = Ok(1)\n    println(typeof(r))\n    let x: Result<Int, String> = r\n    println(typeof(x))\n    let a: Any = []\n    let xs: List<Int> = a\n    xs.push(1)\n    let ys: List<String> = a\n}\n";
    let (ok, text) = mote("run", src, "open");
    assert!(!ok && text.starts_with("Result<Int, _>\nResult<Int, String>\n") && text.contains("expected `List<String>`, found `List<Int>`"), "{text}");
}

#[test]
fn an_any_inside_a_type_is_not_checked_out_silently() {
    rejected(
        "import { Any } from std.experimental.types\npub fn main() {\n    let a: Any = 1\n    let t: (Any, Int) = (a, 2)\n    let u: (Int, Int) = t\n}\n",
        "nested",
        "cannot initialize variable 'u'",
    );
    rejected(
        "import { Any } from std.experimental.types\npub fn main() {\n    let f: () -> Any = || 1\n    let g: () -> Int = f\n}\n",
        "nested_ret",
        "cannot initialize variable 'g'",
    );
}

#[test]
fn a_field_assignment_is_type_checked() {
    rejected(
        "class P {\n    var x: Int\n}\n\npub fn main() {\n    let p = P { x: 1 }\n    p.x = \"s\"\n}\n",
        "field",
        "field 'x' is 'Int', found 'String'",
    );
}

#[test]
fn unwrap_needs_an_option_or_a_result() {
    rejected("pub fn main() {\n    let n = [1].get(0)!\n}\n", "bang", "`!` needs an optional or a `Result`, found 'Int'");
}
