//! The real `std/` source through resolve, check, instantiate and analyze; each test is one stage's status.

use crate::defs::DefId;
use crate::harness::{compile, std_sources};
use crate::resolve::resolve;
use crate::testing::parse_sources;

/// The std source and `main` checked: `file:line message` per error.
fn run(main: &str) -> Vec<String> {
    compile(main).errors
}

#[test]
fn every_std_item_is_declared() {
    let sources = std_sources();
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect();
    let mut parsed = parse_sources(&borrowed);
    let program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    assert!(program.std.map.is_some());
    assert!(program.std.set.is_some());
    assert!(program.std.result.is_some());
    assert!(program.std.error.is_some());
    assert!(program.std.display.is_some());
    assert!(program.std.eq.is_some());
    assert!(program.std.ord.is_some());
    assert!(program.std.clone.is_some());
    assert!(program.std.closeable.is_some());
    assert!(program.std.key.is_some());
    assert!(program.std.span.is_some());
    assert!(program.std.mut_span.is_some());
    assert!(program.std.str_view.is_some());
    assert!(program.std.buffer.is_some());
    assert!(program.std.bytes.is_some());
}

/// The names of the fields of a std class declaration, in order.
fn field_names(pick: fn(&crate::program::StdItems) -> Option<DefId>) -> Vec<String> {
    let sources = std_sources();
    let borrowed: Vec<(&str, &str)> = sources.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect();
    let mut parsed = parse_sources(&borrowed);
    let program = resolve(parsed.input, &mut parsed.interner, &mut parsed.sink);
    let def = pick(&program.std).expect("the std item is declared");
    match &program.defs[def].kind {
        crate::defs::DefKind::Record(record) => {
            assert!(record.is_class);
            record.fields.iter().map(|field| parsed.interner.resolve(field.name).to_string()).collect()
        }
        _ => panic!("the std item is not a record"),
    }
}

#[test]
fn abi_map_and_set_are_classes_with_the_fields_in_the_order_the_abi_names() {
    assert_eq!(field_names(|s| s.map), ["held_keys", "held_values", "hashes", "slots", "count"]);
    assert_eq!(field_names(|s| s.set), ["members"]);
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
    let ok = "import Key from std.traits\nstruct P {\n x: Int\n y: String\n}\nenum E {\n A(Int)\n B(P)\n}\nclass C {\n var f: Float\n}\nfn by<K: Key>(k: K) -> Map<K, Int> { return Map<K, Int>() }\nfn f() {\n let a = Map<(Int, String), Int>()\n let b = Map<P, Int>()\n let c = Map<E, Int>()\n let d = Map<C, Int>()\n let e = Set<Int?>()\n let g = Map<[Int; 3], Int>()\n let h = Map<Int | String, Int>()\n let i = by(P { x: 1, y: \"a\" })\n}";
    assert_eq!(run(ok), Vec::<String>::new());
    let bad = "struct Q {\n f: Float\n}\nenum E {\n A(Q)\n}\nfn f<K>(k: K) -> Map<K, Int> { return Map<K, Int>() }\nfn g() {\n let a = Map<Float, Int>()\n let b = Map<Q, Int>()\n let c = Map<List<Int>, Int>()\n let d = Set<[Int]>()\n let e = Map<(Int, Map<Int, Int>), Int>()\n let h = Set<E>()\n}";
    let errors = run(bad);
    assert_eq!(
        errors,
        [
            "main:7 K does not satisfy `Key`: it has no such bound",
            "main:7 K does not satisfy `Key`: it has no such bound",
            "main:9 Float does not satisfy `Key`: it contains Float",
            "main:10 Q does not satisfy `Key`: it contains Float",
            "main:11 List<Int> does not satisfy `Key`: it contains List<Int>",
            "main:12 [Int] does not satisfy `Key`: it contains [Int]",
            "main:13 (Int, Map<Int, Int>) does not satisfy `Key`: it contains Map<Int, Int>",
            "main:14 E does not satisfy `Key`: it contains Float",
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
    let head = "trait Needs {\n fn probe(self) -> Int {\n  @compile_assert(true, \"fine\")\n  return 1\n }\n fn plain(self) -> Int { return 2 }\n}\n";
    assert_eq!(built_methods(&format!("{head}enum E: (Needs) {{\n A\n}}")), ["E.probe"]);
    assert_eq!(built_methods(&format!("{head}enum G<T>: (Needs) {{\n A(T)\n}}")), Vec::<String>::new());
}

#[test]
fn a_port_opens_with_an_end_and_a_receiver_of_bytes() {
    let main = "import { Port, PortMode } from std.port\nextern \"pane\" {\n fn start(events: PortEnd)\n}\nfn f() {\n let (end, events) = Port.open(PortMode.Queue(64))\n start(end)\n for event in events { let n = event.len() }\n let (latest, frames) = Port.open(PortMode.Latest)\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn tasks_and_sync_have_their_methods() {
    let main = "import { Channel, broadcast, merge } from std.sync\nimport { await_all, sleep, with_timeout } from std.task\nfn f() {\n let (tx, rx) = Channel<Int>(3)\n tx.send(10)\n tx.close()\n let v = rx.recv()\n let late = rx.recv_timeout(500)\n let h = spawn { return 6 * 7 }\n let n = h.await().unwrap()\n let a = spawn { return 1 }\n let b = spawn { return 2 }\n let results = await_all([a, b])\n let waited = a.await_timeout(10)\n sleep(1)\n let t = with_timeout(100, || 5)\n let (tx2, rx2) = Channel<Int>(0)\n let all = merge([rx2])\n let (tx3, rx3) = Channel<Int>(0)\n let outs = broadcast(rx3, 2)\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn a_spawn_block_captures_numbers_text_and_handles_and_no_collection_that_can_change() {
    let head = "class Frozen { n: Int }\nclass Cell { var n: Int }\nstruct P { x: Int }\nstruct Q { c: Frozen }\nenum Fine {\n A(Int)\n B(String)\n}\nenum Mixed {\n A(Int)\n B(List<Int>)\n}\n";
    let ok = "fn f(a: Int, b: Float, c: Bool, d: String, e: [Int], g: [String; 2], h: [[Int]], i: Frozen, k: P, l: Q, m: (Int, String), n: Int | String, o: Int?, p: Result<Int, Error>, q: Fine, r: Sender<Int>, s: Receiver<Int>, t: Task<Int>, u: Mutex<List<Int>>, v: Once<Int>, w: AtomicInt, x: AtomicBool, y: PortEnd) {\n spawn { let z = (a, b, c, d, e, g, h, i, k, l, m) }\n spawn { let z = (n, o, p, q, r, s, t, u, v, w, x, y) }\n}";
    assert_eq!(run(&format!("{head}{ok}")), Vec::<String>::new());
    let bad = "fn f<T>(a: Cell, b: [Frozen], c: Mixed, d: List<Int>, e: Map<String, Int>, f: Set<Int>, g: Stream<Int>, h: Pointer<Int>, i: T, j: () -> Int) {\n spawn { let z = a }\n spawn { let z = b }\n spawn { let z = c }\n spawn { let z = d }\n spawn { let z = e }\n spawn { let z = f }\n spawn { let z = g }\n spawn { let z = h }\n spawn { let z = i }\n spawn { let z = j }\n}";
    let errors = run(&format!("{head}{bad}"));
    let names = ["Cell", "[Frozen]", "Mixed", "List<Int>", "Map<String, Int>", "Set<Int>", "Stream<Int>", "Pointer<Int>", "T", "() -> Int"];
    let letters = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let expected: Vec<String> = letters.iter().zip(names).enumerate().map(|(index, (letter, name))| format!("main:{} `{letter}` is not Sendable: {name}", 14 + index)).collect();
    assert_eq!(errors, expected);
}

#[test]
fn a_handle_is_sendable_and_does_not_change_inside_so_a_once_may_hold_one() {
    let main = "fn f(a: Once<Mutex<List<Int>>>, b: Once<Sender<Int>>, c: Once<AtomicInt>) {\n spawn { let x = (a, b, c) }\n}\nfn g(d: Once<List<Int>>) {}";
    assert_eq!(run(main), ["main:4 a `Once` cannot hold List<Int>"]);
}

#[test]
fn mutex_once_and_atomics_have_their_methods() {
    let main = "fn f() {\n let m = Mutex(0)\n m.lock(|n| { n += 1 })\n let seen = m.lock(|n| { return n })\n let whole = m.get()\n m.set(whole + 1)\n let o = Once<Int>()\n let v = o.get_or_init(|| 5)\n let a = AtomicInt(1)\n a.store(a.load() + a.fetch_add(2) + a.swap(0) + a.fetch_sub(1))\n let swapped = a.compare_exchange(0, 1)\n let flag = AtomicBool(false)\n flag.store(flag.swap(true) || flag.compare_exchange(true, false) || flag.load())\n}";
    assert_eq!(run(main), Vec::<String>::new());
}

#[test]
fn a_lock_body_that_parks_is_refused() {
    let main = "import Channel from std.sync\nfn f() {\n let (tx, rx) = Channel<Int>(1)\n let m = Mutex(0)\n m.lock(|n| { rx.recv() })\n}";
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

#[test]
fn std_array_mask_reductions_require_import_and_bool_fixed_array() {
    assert!(!run("fn f(m: [Bool; 2]) -> Bool {\n return all(m)\n}").is_empty());
    let ok = "import { all, any } from std.array\nfn f(a: [Int; 3], b: [Int; 3]) -> Bool {\n return all(a == b) && any(a < 2)\n}";
    assert_eq!(run(ok), Vec::<String>::new());
    let bad = "import { all, any } from std.array\nfn f(a: [Int; 3], xs: List<Bool>) -> Bool {\n return all(a) && any(xs)\n}";
    assert_eq!(
        run(bad),
        [
            "main:3 `all` expects a `[Bool]` or `[Bool; N]` mask, found [Int; 3]",
            "main:3 `any` expects an array `[T]` or `[T; N]`, found List<Bool>",
        ],
    );
}

#[test]
fn std_array_category_a_and_b_typecheck_both_dyn_and_fixed() {
    let ok = "import { all, any, none, all_by, any_by, none_by, count, count_by, sum, product, mean, min, max, argmin, argmax, dot, norm_sq, fold, reduce, scan } from std.array
fn f(dyn_xs: [Int], fixed_xs: [Int; 3], dyn_mask: [Bool], fixed_f: [Float; 3]) {
    let b1 = all(dyn_mask) && any(dyn_mask) && none(dyn_mask)
    let b2 = all_by(dyn_xs, |x| x > 0) && any_by(fixed_xs, |x| x == 2) && none_by(dyn_xs, |x| x < 0)
    let c1 = count(fixed_xs, 2) + count_by(dyn_xs, |x| x > 1)
    let s1 = sum(dyn_xs) + sum(fixed_xs)
    let p1 = product(dyn_xs) + product(fixed_xs)
    let m1 = mean(dyn_xs) + mean(fixed_xs) + mean(fixed_f)
    let mn = min(dyn_xs) ?? min(fixed_xs) ?? 0
    let mx = max(dyn_xs) ?? max(fixed_xs) ?? 0
    let imn = argmin(dyn_xs) ?? argmin(fixed_xs) ?? 0
    let imx = argmax(dyn_xs) ?? argmax(fixed_xs) ?? 0
    let d1 = dot(dyn_xs, dyn_xs) + dot(fixed_xs, fixed_xs)
    let n1 = norm_sq(dyn_xs) + norm_sq(fixed_xs)
    let fl1 = fold(dyn_xs, 0, |acc, x| acc + x) + fold(fixed_xs, 0, |acc, x| acc + x)
    let r1 = (reduce(dyn_xs, |a, b| a + b) ?? 0) + (reduce(fixed_xs, |a, b| a + b) ?? 0)
    let sc_dyn: [Int] = scan(dyn_xs, 0, |acc, x| acc + x)
    let sc_fix: [Float; 3] = scan(fixed_xs, 0.0, |acc, x| acc + Float.from(x))
}";
    assert_eq!(run(ok), Vec::<String>::new());

    let bad = "import { sum, dot } from std.array
fn f(strs: [String], a: [Int; 2], b: [Int; 3]) {
    let s = sum(strs)
    let d = dot(a, b)
}";
    assert_eq!(run(bad).len(), 2);
}

#[test]
fn std_array_category_c_d_e_typecheck_both_dyn_and_fixed() {
    let ok = "import { find, find_index, index_of, select, map, zip_with, reverse, clamp, filter, zip, enumerate, concat, take, drop, flatten, sort, sort_by } from std.array
fn f(dyn_xs: [Int], fixed_xs: [Int; 3], dyn_mask: [Bool], fixed_mask: [Bool; 3], dyn_strs: [String], fixed_strs: [String; 3], nested: [[Int; 2]; 2]) {
    let f1 = find(dyn_xs, |x| x > 1) ?? find(fixed_xs, |x| x > 1) ?? 0
    let f2 = find_index(dyn_xs, |x| x == 2) ?? find_index(fixed_xs, |x| x == 2) ?? 0
    let f3 = index_of(dyn_xs, 2) ?? index_of(fixed_xs, 2) ?? 0
    let s1: [Int] = select(dyn_mask, dyn_xs, dyn_xs)
    let s2: [Int; 3] = select(fixed_mask, fixed_xs, fixed_xs)
    let m1: [Float] = map(dyn_xs, |x| Float.from(x))
    let m2: [Bool; 3] = map(fixed_xs, |x| x > 0)
    let z1: [String] = zip_with(dyn_xs, dyn_strs, |n, s| s)
    let z2: [Int; 3] = zip_with(fixed_xs, fixed_xs, |a, b| a + b)
    let r1: [Int] = reverse(dyn_xs)
    let r2: [Int; 3] = reverse(fixed_xs)
    let c1: [Int] = clamp(dyn_xs, 0, 10)
    let c2: [Int; 3] = clamp(fixed_xs, 0, 10)
    let fl1: [Int] = filter(dyn_xs, |x| x > 0)
    let fl2: [Int] = filter(fixed_xs, |x| x > 0)
    let zp1: [(Int, String)] = zip(dyn_xs, dyn_strs)
    let zp2: [(Int, String); 3] = zip(fixed_xs, fixed_strs)
    let en1: [(Int, Int)] = enumerate(dyn_xs)
    let en2: [(Int, Int); 3] = enumerate(fixed_xs)
    let cc1: [Int] = concat(dyn_xs, fixed_xs)
    let tk1: [Int] = take(fixed_xs, 2)
    let dr1: [Int] = drop(fixed_xs, 1)
    let ft1: [Int] = flatten(nested)
    let so1: [Int] = sort(dyn_xs)
    let so2: [Int; 3] = sort(fixed_xs)
    let sb1: [String] = sort_by(dyn_strs, |a, b| a.compare(b))
    let sb2: [String; 3] = sort_by(fixed_strs, |a, b| b.compare(a))
}";
    assert_eq!(run(ok), Vec::<String>::new());

    let bad = "import { select, zip, zip_with, concat, flatten, sort } from std.array
fn f(m: [Bool; 2], a: [Int; 3], b: [Int], strs: [String; 2], fns: [() -> Int]) {
    let x = select(m, a, a)
    let y = zip(a, b)
    let z = zip_with(a, b, |p, q| p)
    let c = concat(a, strs)
    let fl = flatten(a)
    let so = sort(fns)
}";
    assert_eq!(run(bad).len(), 6);
}

#[test]
fn ord_synthesizes_comparison_operators() {
    let ok = "import { Ord } from std.traits
struct Span: (Ord) {
    ns: Int
    pub fn compare(self, other: Span) -> Int { return self.ns - other.ns }
}
struct Custom: (Ord) {
    ns: Int
    pub fn compare(self, other: Custom) -> Int { return self.ns - other.ns }
    pub fn __lt__(self, other: Custom) -> Bool { return self.ns > other.ns }
}
fn ok(a: Span, b: Span, c: Custom, d: Custom) -> Bool {
    return a < b && a <= b && a > b && a >= b && c < d && c <= d
}
fn generic<T: Ord>(x: T, y: T) -> Bool {
    return x < y && x <= y && x > y && x >= y
}";
    assert_eq!(run(ok), Vec::<String>::new());
}

#[test]
fn array_methods_and_mut_span_enforce_domain_and_mutability() {
    let bad = "fn main() {\n let xs: [String] = [\"a\", \"b\"]\n let s = xs.sum()\n let fx: [String; 2] = [\"a\", \"b\"]\n let m = fx.mean()\n let ints: [Int] = [1, 2]\n let b = ints.all()\n let mv = ints.mut_span()\n}";
    let errs = run(bad);
    assert!(errs.iter().any(|e| e.contains("numeric array elements")), "{errs:?}");
    assert!(errs.iter().any(|e| e.contains("no method `all`")), "{errs:?}");
    assert!(errs.iter().any(|e| e.contains("no `var` path")), "{errs:?}");
}

#[test]
fn is_flat_and_off_heap_types_reject_gc_references() {
    let ok = "import std.mem.{Buffer}
struct FlatVec {
    x: Float
    y: Float
}
fn main() {
    @compile_assert(@is_flat(Int), \"Int\")
    @compile_assert(@is_flat(FlatVec), \"FlatVec\")
    @compile_assert(@is_flat(Pointer<Int>), \"Pointer<Int>\")
    @compile_assert(!@is_flat(String), \"String\")
    @compile_assert(!@is_flat([Int]), \"array\")
    let p = Pointer<FlatVec>.alloc(2)
    p.free()
    var b = Buffer<Int>.alloc(2)
    let _ = b.close()
}";
    assert_eq!(run(ok), Vec::<String>::new());

    for snippet in [
        "fn main() { let p = Pointer<String>.alloc(4) }",
        "import std.mem.{Buffer}\nfn main() { let b = Buffer<String>.alloc(4) }",
        "import std.mem.{Buffer}\nfn main() { let b = Buffer<List<Int>>.alloc(4) }",
        "import std.mem.{Buffer}\nclass Boxed { var n: Int }\nfn main() { let b = Buffer<Boxed>.alloc(4) }",
        "import std.mem.{Buffer}\nenum Tree { Leaf, Node(Tree, Tree) }\nfn main() { let b = Buffer<Tree>.alloc(4) }",
    ] {
        let errs = run(snippet);
        assert!(errs.iter().any(|e| e.contains("flat")), "expected flat error for {snippet:?}, got {errs:?}");
    }
}

#[test]
fn slice_view_types_and_owner_to_view_coercion_errors() {
    let ok = "import { MutSpan, Span } from std.collections
import StrView from std.string
fn f(s: String, a: [Int], xs: List<Int>, fx: [Int; 3]) -> (StrView, Span<Int>, Span<Int>, [Int]) {
    let v: StrView = s[1..3]
    let sa: Span<Int> = a[1..3]
    let sl: Span<Int> = xs[1..3]
    let sf: [Int] = fx[1..3]
    return (v, sa, sl, sf)
}";
    assert_eq!(run(ok), Vec::<String>::new());

    let no_var = "class Hold { var m: MutSpan<Int> }
fn f_var(var m: MutSpan<Int>) {}
fn f_val(m: MutSpan<Int>) {}
fn bad() {
    let a: [Int] = [1, 2]
    f_var(a)
    f_val(a)
    let m: MutSpan<Int> = a
    let h = Hold { m: a }
}";
    assert_eq!(
        run(no_var),
        [
            "main:6 this place has no `var` path to write through",
            "main:7 this place has no `var` path to write through",
            "main:8 this place has no `var` path to write through",
            "main:9 this place has no `var` path to write through",
        ]
    );

    let bad_fn_adapt = "fn takes_var(var m: MutSpan<Int>) {}
fn takes_val(m: MutSpan<Int>) {}
fn bad() {
    let f1: ([Int]) -> Null = takes_var
    let f2: ([Int]) -> Null = takes_val
}";
    assert_eq!(run(bad_fn_adapt).len(), 2);

    let no_view_to_array = "fn need_arr(xs: [Int]) {}
fn bad(s: Span<Int>) {
    need_arr(s)
}";
    assert_eq!(run(no_view_to_array), ["main:3 expected [Int], found Span<Int>"]);
}

#[test]
fn generic_eq_bound_dispatches_dunders_post_monomorphization_only() {
    use crate::tree::{BinaryOp, Callee, ExprKind, UnaryOp};

    let src = "import Eq from std.traits
struct OnlyEq: (Eq) {
    x: Int
    pub fn eq(self, other: OnlyEq) -> Bool { return true }
}
struct WithDunder: (Eq) {
    x: Int
    pub fn eq(self, other: WithDunder) -> Bool { return false }
    pub fn __eq__(self, other: WithDunder) -> Bool { return true }
}
struct WithNe: (Eq) {
    x: Int
    pub fn eq(self, other: WithNe) -> Bool { return true }
    pub fn __ne__(self, other: WithNe) -> Bool { return false }
}
fn same<T: Eq>(a: T, b: T) -> Bool { return a == b }
fn diff<T: Eq>(a: T, b: T) -> Bool { return a != b }
fn main() {
    let a = same(OnlyEq { x: 1 }, OnlyEq { x: 2 })
    let b = diff(OnlyEq { x: 1 }, OnlyEq { x: 2 })
    let c = same(WithDunder { x: 1 }, WithDunder { x: 2 })
    let d = diff(WithDunder { x: 1 }, WithDunder { x: 2 })
    let e = diff(WithNe { x: 1 }, WithNe { x: 2 })
}";
    let compiled = compile(src);
    assert_eq!(compiled.errors, Vec::<String>::new());

    let find_body = |fn_name: &str, ty_name: &str| {
        compiled
            .program
            .bodies
            .iter()
            .find(|(inst, _)| {
                compiled.interner.resolve(compiled.program.defs[inst.def].name) == fn_name
                    && inst
                        .args
                        .iter()
                        .any(|arg| matches!(arg, crate::types::GenericArg::Type(ty) if compiled.program.render(*ty, &compiled.interner) == ty_name))
            })
            .expect("monomorphized instance")
            .1
    };

    let only_eq_same = find_body("same", "OnlyEq");
    assert!(only_eq_same.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Binary { op: BinaryOp::Eq, .. })));
    assert!(!only_eq_same.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Call { .. })));

    let only_eq_diff = find_body("diff", "OnlyEq");
    assert!(only_eq_diff.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Binary { op: BinaryOp::NotEq, .. })));
    assert!(!only_eq_diff.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Call { .. })));

    let dunder_same = find_body("same", "WithDunder");
    assert!(!dunder_same.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Binary { .. })));
    assert!(dunder_same.exprs.iter().any(|(_, e)| matches!(&e.kind, ExprKind::Call { callee: Callee::Function(f), .. } if compiled.interner.resolve(compiled.program.defs[f.def].name) == "__eq__")));

    let dunder_diff = find_body("diff", "WithDunder");
    assert!(!dunder_diff.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Binary { .. })));
    assert!(dunder_diff.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Unary { op: UnaryOp::Not, .. })));
    assert!(dunder_diff.exprs.iter().any(|(_, e)| matches!(&e.kind, ExprKind::Call { callee: Callee::Function(f), .. } if compiled.interner.resolve(compiled.program.defs[f.def].name) == "__eq__")));

    let ne_diff = find_body("diff", "WithNe");
    assert!(!ne_diff.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Binary { .. })));
    assert!(!ne_diff.exprs.iter().any(|(_, e)| matches!(e.kind, ExprKind::Unary { op: UnaryOp::Not, .. })));
    assert!(ne_diff.exprs.iter().any(|(_, e)| matches!(&e.kind, ExprKind::Call { callee: Callee::Function(f), .. } if compiled.interner.resolve(compiled.program.defs[f.def].name) == "__ne__")));
}
