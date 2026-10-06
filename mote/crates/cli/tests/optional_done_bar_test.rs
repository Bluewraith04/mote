//! Optional and operator forms done bar: a stock tracker using every form, and each misuse as a compile error.

use std::process::Command;

fn mote(cmd: &str, source: &str, tag: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_optional_bar_{}_{}_{}", cmd, tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(cmd).arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const PROGRAM: &str = r#"
import { Any } from std.experimental.types
class Item { name: String  count: Int  next: Item? }

fn find(stock: Map<String, Int>, name: String) -> Int? {
    if stock.contains_key(name) { return stock[name] }
    return None
}

fn restock(stock: Map<String, Int>, name: String) -> Int? {
    let have = find(stock, name)?
    return have + 10
}

fn chain(head: Item?) -> String {
    if head == null { return "empty" }
    let second = head.next?.name ?? "-"
    let third = head.next?.next?.name ?? "-"
    return "${head.name},${second},${third}"
}

fn deep(x: Int??) -> String {
    match x {
        Some(inner) => { return inner == null ? "some(none)" : "some(${inner})" }
        None => { return "none" }
    }
    return "unreachable"
}

fn check(v: Any) -> String {
    if v is List<Int> { return "ints" }
    if v is Int? { return "maybe int" }
    return "other"
}

pub fn main() {
    let stock: Map<String, Int> = {"bolt": 3, "nut": 0, "gear": 7,}
    stock["nut"] += 5
    stock["washer"] = 2
    println(stock)

    let xs = [10, 20, 30,]
    xs[1] = xs[0] + xs[2]
    xs[2] *= 2
    println(xs)

    let b = "hi".bytes()
    b[0] = 72
    println(b.decode() ?? "?")

    println(find(stock, "gear") ?? -1)
    println(find(stock, "cog") ?? -1)
    println(restock(stock, "bolt"))
    println(restock(stock, "cog"))

    var cache: Int? = None
    cache ??= 4
    cache ??= 9
    println(cache)

    let c = Item { name: "c", count: 1, next: None }
    let a = Item { name: "a", count: 1, next: Item { name: "b", count: 2, next: c } }
    println(chain(a))
    println(chain(a.next))
    println(chain(None))

    let n: Int? = find(stock, "bolt")
    let verdict = n != null && n > 2 ? "plenty" : "few"
    println(verdict)
    println(n == null || n < 2 ? "low" : "ok")

    let one: Int?? = Some(None)
    let two: Int?? = 5
    let three: Option<Option<Int>> = None
    println("${deep(one)} ${deep(two)} ${deep(three)}")

    let word: Any = "x"
    println("${check([1, 2])} ${check(null)} ${check(word)}")
    println(Some([1]) == Some([1]))
    println(find(stock, "bolt")!)
}
"#;

#[test]
fn a_stock_tracker_uses_every_form() {
    let (ok, text) = mote("run", PROGRAM, "run");
    let want = "{\"bolt\": 3, \"nut\": 5, \"gear\": 7, \"washer\": 2}\n[10, 40, 60]\nHi\n7\n-1\n13\nnull\n4\na,b,c\nb,c,-\nempty\nplenty\nok\nsome(none) some(5) none\nints maybe int other\ntrue\n3\n";
    assert!(ok && text == want, "{text}");
}

fn misuse(from: &str, to: &str, tag: &str, message: &str) {
    assert!(PROGRAM.contains(from), "`{from}` is not in the program");
    let (ok, text) = mote("check", &PROGRAM.replace(from, to), tag);
    assert!(!ok && text.contains(message), "expected `{message}`, got: {text}");
}

#[test]
fn each_misuse_is_a_compile_error() {
    misuse("    b[0] = 72\n", "    let s = \"hi\"\n    println(s[0])\n", "string_index", "a `String` has no integer indexing");
    misuse("n != null && n > 2 ?", "n ?", "ternary_cond", "the condition of `?:` must be `Bool`, found `Int?`");
    misuse("? \"plenty\" : \"few\"", "? \"plenty\" : 3", "ternary_join", "the branches of `?:` have different types, `String` and `Int`");
    misuse("    if head == null { return \"empty\" }\n", "", "unchecked", "`Item?` may be `None`, so `.next` needs");
    misuse("println(find(stock, \"gear\") ?? -1)", "println(find(stock, \"gear\")! ?? -1)", "coalesce_plain", "`??` needs an optional on its left, found `Int`");
    misuse("fn restock(stock: Map<String, Int>, name: String) -> Int? {", "fn restock(stock: Map<String, Int>, name: String) -> Result<Int, String> {", "try_kind", "`?` on an optional returns `None`, but this function returns a `Result`");
    misuse("let xs = [10, 20, 30,]", "let xs = [10, \"20\", 30,]", "mixed", "list elements have different types, `Int` and `String`");
    misuse("{\"bolt\": 3, \"nut\": 0,", "{\"bolt\": 3, \"bolt\": 0,", "dup_key", "key \"bolt\" appears twice");
    misuse("println(cache)", "println(cache as Int)", "as", "`as` only renames an import");
    misuse("var cache: Int? = None", "var cache = None", "none_type", "the type of `cache` is not fully known (`_?`)");
    misuse("if v is List<Int>", "if v is Option.Some", "variant", "`Option.Some` is an enum variant, not a type");
    misuse("let n: Int? = find(stock, \"bolt\")", "var n: Int? = find(stock, \"bolt\")", "var_narrow", "`Int?` may be `None`, so `>` needs");
}
