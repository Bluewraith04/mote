//! The real `std/` source through resolve, check, instantiate and analyze; each test is one stage's status.

use crate::defs::LangItem;
use crate::harness::{compile, std_sources};
use crate::resolve::resolve;
use crate::testing::parse_sources;

/// The std source and `main` checked: `file:line message` per error.
fn run(main: &str) -> Vec<String> {
    compile(main).errors
}

/// The lang items the std source does not declare.
fn missing_lang_items() -> Vec<&'static str> {
    let sources = std_sources();
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect();
    let mut parsed = parse_sources(&borrowed);
    let program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    LangItem::ALL.into_iter().filter(|&item| program.lang_item(item).is_none()).map(LangItem::name).collect()
}

#[test]
fn every_lang_item_is_declared() {
    assert_eq!(missing_lang_items(), Vec::<&str>::new());
}

/// The names of the fields of the class a lang item is declared as, in order.
fn field_names(item: LangItem) -> Vec<String> {
    let sources = std_sources();
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect();
    let mut parsed = parse_sources(&borrowed);
    let program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    let def = program.lang_item(item).expect("the lang item is declared");
    match &program.defs[def].kind {
        crate::defs::DefKind::Record(record) => {
            assert!(record.is_class);
            record.fields.iter().map(|field| parsed.interner.resolve(field.name).to_string()).collect()
        }
        _ => panic!("the lang item is not a record"),
    }
}

#[test]
fn abi_pointer_is_a_struct_with_one_uint64_address() {
    let sources = std_sources();
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect();
    let mut parsed = parse_sources(&borrowed);
    let program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    let def = program.lang_item(LangItem::Pointer).expect("the lang item is declared");
    let crate::defs::DefKind::Record(record) = &program.defs[def].kind else { panic!("the lang item is not a record") };
    assert!(!record.is_class);
    let fields: Vec<(String, crate::types::TyId)> = record.fields.iter().map(|field| (parsed.interner.resolve(field.name).to_string(), field.ty)).collect();
    assert_eq!(fields, [("address".to_string(), crate::types::TyId::UINT64)]);
}

#[test]
fn abi_map_and_set_are_classes_with_the_fields_in_the_order_the_abi_names() {
    assert_eq!(field_names(LangItem::Map), ["held_keys", "held_values", "hashes", "slots", "count"]);
    assert_eq!(field_names(LangItem::Set), ["members"]);
}

#[test]
fn the_std_source_alone_is_clean() {
    assert_eq!(run(""), Vec::<String>::new());
}

#[test]
fn collections_have_their_methods() {
    let main = "import { Set, set_from, union } from std.collections\nfn f() {\n var xs: List<Int> = [3, 1, 2]\n xs.push(4)\n let n = xs.len()\n let h = xs.first()\n let t = xs[1]\n xs[0] = 9\n var m: Map<String, Int> = {\"a\": 1}\n m[\"b\"] = 2\n let a = set_from([1, 2, 3])\n let u = union(a, set_from([3, 4]))\n var grid: [Int] = [0; 100]\n grid[5] = 1\n let g = grid.len()\n let part = xs[1..3]\n let m2 = Map<String, Int>()\n let s = Set<Int>()\n let found = \"hello\".find(\"l\")\n let parts = \"a,b,c\".split(\",\")\n for i in 0..3 { xs.push(i) }\n for (k, v) in m.items() { let line = \"${k}=${v}\" }\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn a_key_holds_no_float_and_no_collection_that_can_change() {
    let ok = "import { Set } from std.collections\nimport { Key } from std.traits\nstruct P {\n x: Int\n y: String\n}\nenum E {\n A(Int)\n B(P)\n}\nclass C {\n var f: Float\n}\nfn by<K: Key>(k: K) -> Map<K, Int> { return Map<K, Int>() }\nfn f() {\n let a = Map<(Int, String), Int>()\n let b = Map<P, Int>()\n let c = Map<E, Int>()\n let d = Map<C, Int>()\n let e = Set<Int?>()\n let g = Map<[Int; 3], Int>()\n let h = Map<Int | String, Int>()\n let i = by(P { x: 1, y: \"a\" })\n}";
    assert_eq!(run(ok), Vec::<String>::new());
    let bad = "import { Set } from std.collections\nstruct Q {\n f: Float\n}\nenum E {\n A(Q)\n}\nfn f<K>(k: K) -> Map<K, Int> { return Map<K, Int>() }\nfn g() {\n let a = Map<Float, Int>()\n let b = Map<Q, Int>()\n let c = Map<List<Int>, Int>()\n let d = Set<[Int]>()\n let e = Map<(Int, Map<Int, Int>), Int>()\n let h = Set<E>()\n}";
    let errors = run(bad);
    assert_eq!(
        errors,
        [
            "main:8 K does not satisfy `Key`: it has no such bound",
            "main:8 K does not satisfy `Key`: it has no such bound",
            "main:10 Float does not satisfy `Key`: it contains Float",
            "main:11 Q does not satisfy `Key`: it contains Float",
            "main:12 List<Int> does not satisfy `Key`: it contains List<Int>",
            "main:13 [Int] does not satisfy `Key`: it contains [Int]",
            "main:14 (Int, Map<Int, Int>) does not satisfy `Key`: it contains Map<Int, Int>",
            "main:15 E does not satisfy `Key`: it contains Float",
        ]
    );
}

#[test]
fn a_global_annotation_and_a_map_literal_check_the_key() {
    let bad = "let a: Map<Float, Int> = {}\nlet b = {1.5: 1}\nfn f() {\n let c = {[1]: 2}\n}";
    assert_eq!(
        run(bad),
        ["main:1 Float does not satisfy `Key`: it contains Float", "main:2 Float does not satisfy `Key`: it contains Float", "main:4 List<Int> does not satisfy `Key`: it contains List<Int>"]
    );
}

#[test]
fn derive_listing_display_debug_and_clone_needs_no_method() {
    let main = "import { Display, Debug, Clone } from std.traits\nstruct P: (Display, Debug, Clone) {\n x: Int\n}\nenum E: (Display, Debug, Clone) {\n A\n B(Int)\n}\nclass C: (Display, Debug, Clone) {\n var n: Int\n}\nfn f() -> String {\n let c = C { n: 1 }.clone()\n return P { x: 1 }.to_string() + E.A.debug() + c.to_string()\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn derive_a_trait_without_a_default_still_needs_its_method() {
    let main = "import { Eq } from std.traits\nstruct P: (Eq) {\n x: Int\n}";
    assert_eq!(run(main), ["main:2 `P` has no `eq`, which `Eq` requires"]);
}

/// The names of the methods of the types `main` writes that have a body in the program.
fn built_methods(main: &str) -> Vec<String> {
    let compiled = compile(main);
    assert_eq!(compiled.errors, Vec::<String>::new());
    let mut names: Vec<String> = compiled
        .program
        .bodies
        .keys()
        .filter_map(|instance| {
            let def = &compiled.program.defs[instance.def];
            let owner = &compiled.program.defs[def.owner?];
            Some(format!("{}.{}", compiled.interner.resolve(owner.name), compiled.interner.resolve(def.name)))
        })
        .filter(|name| name.starts_with("E.") || name.starts_with("G."))
        .collect();
    names.sort();
    names
}

#[test]
fn require_in_a_default_method_builds_it_for_each_type_without_parameters_that_lists_the_trait() {
    let head = "import { require } from std.meta\ntrait Needs {\n fn probe(self) -> Int {\n  require(true, \"fine\")\n  return 1\n }\n fn plain(self) -> Int { return 2 }\n}\n";
    assert_eq!(built_methods(&format!("{head}enum E: (Needs) {{\n A\n}}")), ["E.probe"]);
    assert_eq!(built_methods(&format!("{head}enum G<T>: (Needs) {{\n A(T)\n}}")), Vec::<String>::new());
}

#[test]
fn a_port_opens_with_an_end_and_a_receiver_of_bytes() {
    let main = "import { Port, PortMode, PortEnd } from std.port\n@link(\"pane\")\nnative fn start(events: PortEnd)\nfn f() {\n let (end, events) = Port.open(PortMode.Queue(64))\n start(end)\n for event in events { let n = event.len() }\n let (latest, frames) = Port.open(PortMode.Latest)\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn tasks_and_sync_have_their_methods() {
    let main = "import { Channel, broadcast, merge } from std.sync\nimport { await_all, sleep, with_timeout } from std.task\nfn f() {\n let (tx, rx) = Channel<Int>(3)\n tx.send(10)\n tx.close()\n let v = rx.recv()\n let late = rx.recv_timeout(500)\n let h = spawn { return 6 * 7 }\n let n = h.await().unwrap()\n let a = spawn { return 1 }\n let b = spawn { return 2 }\n let results = await_all([a, b])\n let waited = a.await_timeout(10)\n sleep(1)\n let t = with_timeout(100, || 5)\n let (tx2, rx2) = Channel<Int>(0)\n let all = merge([rx2])\n let (tx3, rx3) = Channel<Int>(0)\n let outs = broadcast(rx3, 2)\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn a_spawn_block_captures_numbers_text_and_handles_and_no_collection_that_can_change() {
    let head = "import { Set } from std.collections\nimport { Sender, Receiver, Mutex, Once } from std.sync\nimport { Task } from std.task\nimport { PortEnd } from std.port\nimport { Pointer } from std.types\nimport std.atomic as atomic\nclass Frozen { n: Int }\nclass Cell { var n: Int }\nstruct P { x: Int }\nstruct Q { c: Frozen }\nenum Fine {\n A(Int)\n B(String)\n}\nenum Mixed {\n A(Int)\n B(List<Int>)\n}\n";
    let ok = "fn f(a: Int, b: Float, c: Bool, d: String, e: [Int], g: [String; 2], h: [[Int]], i: Frozen, k: P, l: Q, m: (Int, String), n: Int | String, o: Int?, p: Result<Int, Error>, q: Fine, r: Sender<Int>, s: Receiver<Int>, t: Task<Int>, u: Mutex<List<Int>>, v: Once<Int>, w: atomic.Int64, x: atomic.Bool, y: PortEnd) {\n spawn { let z = (a, b, c, d, e, g, h, i, k, l, m) }\n spawn { let z = (n, o, p, q, r, s, t, u, v, w, x, y) }\n}";
    assert_eq!(run(&format!("{head}{ok}")), Vec::<String>::new());
    let bad = "fn f<T>(a: Cell, b: [Frozen], c: Mixed, d: List<Int>, e: Map<String, Int>, f: Set<Int>, g: Stream<Int>, h: Pointer<Int>, i: T, j: () -> Int) {\n spawn { let z = a }\n spawn { let z = b }\n spawn { let z = c }\n spawn { let z = d }\n spawn { let z = e }\n spawn { let z = f }\n spawn { let z = g }\n spawn { let z = h }\n spawn { let z = i }\n spawn { let z = j }\n}";
    let errors = run(&format!("import {{ Stream }} from std.collections\n{head}{bad}"));
    let names = ["Cell", "[Frozen]", "Mixed", "List<Int>", "Map<String, Int>", "Set<Int>", "Stream<Int>", "Pointer<Int>", "T", "() -> Int"];
    let letters = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let expected: Vec<String> = letters.iter().zip(names).enumerate().map(|(index, (letter, name))| format!("main:{} `{letter}` is not Sendable: {name}", 21 + index)).collect();
    assert_eq!(errors, expected);
}

#[test]
fn a_handle_is_sendable_and_does_not_change_inside_so_a_once_may_hold_one() {
    let main = "import { Mutex, Once, Sender } from std.sync\nimport std.atomic as atomic\nfn f(a: Once<Mutex<List<Int>>>, b: Once<Sender<Int>>, c: Once<atomic.Int64>) {\n spawn { let x = (a, b, c) }\n}\nfn g(d: Once<List<Int>>) {}";
    assert_eq!(run(main), ["main:6 a `Once` cannot hold List<Int>"]);
}

#[test]
fn mutex_once_and_atomics_have_their_methods() {
    let main = "import { Mutex, Once } from std.sync\nimport std.atomic as atomic\nfn f() {\n let m = Mutex(0)\n m.lock(|n| { n += 1 })\n let seen = m.lock(|n| { return n })\n let whole = m.get()\n m.set(whole + 1)\n let o = Once<Int>()\n let v = o.get_or_init(|| 5)\n let a = atomic.Int64(1)\n a.store(a.load() + a.fetch_add(2) + a.swap(0) + a.fetch_sub(1))\n let swapped = a.compare_exchange(0, 1)\n let flag = atomic.Bool(false)\n flag.store(flag.swap(true) || flag.compare_exchange(true, false) || flag.load())\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn a_lock_body_that_parks_is_refused() {
    let main = "import { Channel, Mutex } from std.sync\nfn f() {\n let (tx, rx) = Channel<Int>(1)\n let m = Mutex(0)\n m.lock(|n| { rx.recv() })\n}";
    assert_eq!(run(main), ["main:5 `lock` must not park; its body reaches `Receiver.recv`"]);
}

#[test]
fn iter_string_math_and_test_modules_work_together() {
    let main = "import { contains, enumerate, filter, fold, map, sort, sum } from std.iter\nimport { join, lines, pad_start, parse_int } from std.string\nimport { PI, sqrt } from std.math\nimport { assert, assert_eq, assert_ne } from std.test\nfn f() {\n let doubled = map([1, 2, 3], |n| n * 2)\n let total = fold(doubled, 0, |a, n| a + n)\n let kept = filter(doubled, |n| n > 2)\n let sorted = sort([3, 1, 2])\n let s = sum(sorted)\n let pairs = enumerate([\"a\", \"b\"])\n let has = contains([1, 2], 2)\n let csv = join([\"a\", \"b\"], \",\")\n let n = parse_int(\"42\")!\n let ls = lines(\"a\\nb\")\n let p = pad_start(\"7\", 3, \"0\")\n let r = sqrt(2.0) * PI\n}\ntest \"adds\" {\n assert_eq(1 + 1, 2)\n assert_ne(\"a\", \"b\")\n assert(true)\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn numbers_have_their_methods() {
    let main = "fn f() {\n let a = 255u8.wrapping_add(1)\n let b = (-3).abs()\n let c = 7.min(2)\n let d = 5.to_string()\n let e = Int.from(2.7)\n let (v, o) = 200u8.overflowing_add(100)\n let g = 2.5.max(1.0)\n let h = 1.checked_add(2)\n let s = \"${a}\"\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn a_destructured_global_assigns_only_its_own_names() {
    let compiled = compile("let (a, b) = (1, 2)\nfn main() {}");
    assert_eq!(compiled.errors, Vec::<String>::new());
    let first = compiled.program.global_inits.iter().find(|(def, _)| compiled.interner.resolve(compiled.program.defs[**def].name) == "a").expect("a body under `a`").1;
    assert_eq!(first.block.stmts.len(), 3);
}

#[test]
fn a_with_in_a_generic_function_reaches_the_close_of_the_resource() {
    let compiled = compile("import { Closeable } from std.traits\nclass File {\n pub fn close(self) -> Result<Null, Error> { return Ok(null) }\n}\nfn hold<T: Closeable>(x: T) {\n with r = x { }\n}\nfn main() {\n hold(File {})\n}");
    assert_eq!(compiled.errors, Vec::<String>::new());
    let names: Vec<&str> = compiled.program.order.iter().map(|instance| compiled.interner.resolve(compiled.program.defs[instance.def].name)).collect();
    assert!(names.contains(&"close"), "{names:?}");
}
