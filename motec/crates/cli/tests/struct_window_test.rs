//! A struct local used only through its fields lives in registers, with no object; behaviour is unchanged.

use std::process::Command;

use isa::opcode::Opcode;
use modules::MultiFileCompiler;

const TYPES: &str = "struct V {\n    var x: Int\n    var y: Int\n}\n\nstruct Row {\n    var name: String\n    var score: Float\n    var ok: Bool\n    var n: Int\n}\n\nfn len2(v: V) -> Int {\n    return v.x * v.x + v.y * v.y\n}\n\nfn keep2(v: V) -> Int {\n    let w = v\n    return w.x * w.x + w.y * w.y\n}\n\n";

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_window_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(body: &str, tag: &str) -> Vec<String> {
    let dir = dir(tag);
    std::fs::write(dir.join("main.mote"), format!("{TYPES}fn main() {{\n{body}}}\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().filter(|l| !l.starts_with("GC:")).map(String::from).collect()
}

fn allocations(source: &str, tag: &str) -> usize {
    let dir = dir(tag);
    let main = dir.join("main.mote");
    std::fs::write(&main, source).unwrap();
    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let ops = [Opcode::NEWOBJ as u8, Opcode::ARENAALLOC as u8];
    compiled.code_objects.iter().flat_map(|c| c.instructions.iter()).filter(|i| ops.contains(&((**i & 0xFF) as u8))).count()
}

fn objects(body: &str, tag: &str) -> usize {
    allocations(&format!("{TYPES}fn main() {{\n{body}}}\n"), tag) - allocations(&format!("{TYPES}fn main() {{\n}}\n"), &format!("{tag}_base"))
}

fn run_with(defs: &str, body: &str, tag: &str) -> Vec<String> {
    let dir = dir(tag);
    std::fs::write(dir.join("main.mote"), format!("{TYPES}{defs}\nfn main() {{\n{body}}}\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().filter(|l| !l.starts_with("GC:")).map(String::from).collect()
}

fn objects_with(defs: &str, body: &str, tag: &str) -> usize {
    allocations(&format!("{TYPES}{defs}\nfn main() {{\n{body}}}\n"), tag) - allocations(&format!("{TYPES}fn main() {{\n}}\n"), &format!("{tag}_base"))
}

const MK: &str = "fn mk(i: Int) -> V {\n    return V { x: i, y: i * 2 }\n}\n";

#[test]
fn a_returned_struct_bound_by_let_takes_no_object() {
    let body = "    var v = mk(3)\n    v.x += 10\n    println(v.x + v.y)\n";
    assert_eq!(objects_with(MK, body, "ret_plain"), 0);
    assert_eq!(run_with(MK, body, "ret_plain"), ["19"]);
}

#[test]
fn a_returned_struct_in_a_loop_is_fresh_each_time() {
    let body = "    var total = 0\n    var i = 0\n    while i < 5 {\n        var v = mk(i)\n        v.y += 1\n        total += v.x + v.y\n        i += 1\n    }\n    println(total)\n";
    assert_eq!(objects_with(MK, body, "ret_loop"), 0);
    assert_eq!(run_with(MK, body, "ret_loop"), ["35"]);
}

#[test]
fn a_returned_local_hands_over_its_registers() {
    let defs = "fn build(i: Int) -> V {\n    var p = V { x: i, y: 1 }\n    p.x += 100\n    p.y = p.x + 1\n    return p\n}\n";
    let body = "    let v = build(5)\n    println(v.x)\n    println(v.y)\n";
    assert_eq!(objects_with(defs, body, "ret_local"), 0);
    assert_eq!(run_with(defs, body, "ret_local"), ["105", "106"]);
}

#[test]
fn a_returned_local_that_is_also_an_object_is_unpacked() {
    let defs = "fn build(i: Int) -> V {\n    var p = V { x: i, y: 1 }\n    println(keep2(p))\n    return p\n}\n";
    let body = "    let v = build(3)\n    println(v.x + v.y)\n";
    assert_eq!(run_with(defs, body, "ret_unpack"), ["10", "4"]);
}

#[test]
fn a_function_with_a_call_that_is_not_a_let_returns_an_object() {
    let body = "    let v = mk(3)\n    println(mk(4).y + v.x)\n";
    assert_eq!(objects_with(MK, body, "ret_mixed"), 1);
    assert_eq!(run_with(MK, body, "ret_mixed"), ["11"]);
}

#[test]
fn a_function_used_as_a_value_returns_an_object() {
    let body = "    let f = mk\n    let v = f(3)\n    let w = mk(4)\n    println(v.x + w.y)\n";
    assert_eq!(run_with(MK, body, "ret_value"), ["11"]);
}

#[test]
fn a_returned_struct_bound_but_passed_on_is_still_correct() {
    let body = "    let v = mk(3)\n    println(len2(v))\n";
    assert_eq!(run_with(MK, body, "ret_passed"), ["45"]);
}

#[test]
fn a_recursive_function_returns_in_registers() {
    let defs = "fn fib(n: Int) -> V {\n    if n == 0 {\n        return V { x: 0, y: 1 }\n    }\n    let p = fib(n - 1)\n    return V { x: p.y, y: p.x + p.y }\n}\n";
    let body = "    let r = fib(10)\n    println(r.x)\n";
    assert_eq!(run_with(defs, body, "ret_fib"), ["55"]);
}

#[test]
fn a_returned_struct_of_every_field_type_keeps_its_fields() {
    let defs = "fn row(i: Int) -> Row {\n    return Row { name: \"n${i}\", score: 1.5, ok: i > 1, n: i }\n}\n";
    let body = "    let r = row(2)\n    println(r.name)\n    println(r.score)\n    println(r.ok)\n    println(r.n)\n";
    assert_eq!(objects_with(defs, body, "ret_types"), 0);
    assert_eq!(run_with(defs, body, "ret_types"), ["n2", "1.5", "true", "2"]);
}

#[test]
fn two_returned_structs_in_one_scope_do_not_overlap() {
    let body = "    let a = mk(1)\n    let b = mk(10)\n    println(a.x + a.y + b.x + b.y)\n";
    assert_eq!(objects_with(MK, body, "ret_two"), 0);
    assert_eq!(run_with(MK, body, "ret_two"), ["33"]);
}

#[test]
fn a_call_with_more_arguments_than_fields_returns_in_registers() {
    let defs = "fn mix(a: Int, b: Int, c: Int, d: Int, e: Int) -> V {\n    return V { x: a + b, y: c + d + e }\n}\n";
    let body = "    let v = mix(1, 2, 3, 4, 5)\n    println(v.x)\n    println(v.y)\n";
    assert_eq!(objects_with(defs, body, "ret_wide"), 0);
    assert_eq!(run_with(defs, body, "ret_wide"), ["3", "12"]);
}

#[test]
fn a_struct_used_only_through_its_fields_has_no_object() {
    let body = "    var v = V { x: 3, y: 4 }\n    v.x += 1\n    v.y = v.x * 2\n    println(v.x + v.y)\n";
    assert_eq!(objects(body, "plain"), 0);
    assert_eq!(run(body, "plain"), ["12"]);
}

#[test]
fn a_struct_passed_whole_still_takes_an_object() {
    let body = "    let v = V { x: 3, y: 4 }\n    println(keep2(v))\n";
    assert_eq!(objects(body, "whole"), 1);
    assert_eq!(run(body, "whole"), ["25"]);
}

#[test]
fn fields_of_every_type_read_and_write() {
    let out = run(
        "    var r = Row { name: \"ann\", score: 1.5, ok: true, n: 2 }\n    r.name = r.name + \"!\"\n    r.score *= 2.0\n    r.ok = !r.ok\n    r.n -= 5\n    println(r.name)\n    println(r.score)\n    println(r.ok)\n    println(r.n)\n",
        "types",
    );
    assert_eq!(out, ["ann!", "3.0", "false", "-3"]);
}

#[test]
fn a_struct_declared_in_a_loop_starts_fresh_each_time() {
    let out = run(
        "    var total = 0\n    var i = 0\n    while i < 5 {\n        var v = V { x: i, y: 10 }\n        v.x += 100\n        total += v.x + v.y\n        i += 1\n    }\n    println(total)\n",
        "loop",
    );
    assert_eq!(out, ["560"]);
}

#[test]
fn two_structs_in_nested_scopes_keep_their_own_registers() {
    let out = run(
        "    var a = V { x: 1, y: 2 }\n    if a.x == 1 {\n        var b = V { x: 10, y: 20 }\n        b.x += a.y\n        a.y = b.x + b.y\n    }\n    var c = V { x: 7, y: 8 }\n    println(a.x)\n    println(a.y)\n    println(c.x + c.y)\n",
        "nested",
    );
    assert_eq!(out, ["1", "32", "15"]);
}

#[test]
fn a_field_expression_may_use_the_other_fields_of_an_earlier_struct() {
    let out = run("    var a = V { x: 2, y: 3 }\n    var b = V { x: a.y, y: a.x + a.y }\n    a.x = b.y * b.x\n    println(a.x)\n    println(b.y)\n", "chain");
    assert_eq!(out, ["15", "5"]);
}

#[test]
fn a_struct_that_a_lambda_uses_stays_an_object() {
    let body = "    var v = V { x: 1, y: 2 }\n    let f = || v.x + v.y\n    v.x = 10\n    println(f())\n";
    assert_eq!(objects(body, "lambda"), 2, "the closure and the struct");
    assert_eq!(run(body, "lambda"), ["12"]);
}

#[test]
fn a_struct_that_is_copied_returned_or_matched_stays_an_object() {
    for (tag, body, want) in [
        ("copied", "    let a = V { x: 1, y: 2 }\n    var b = a\n    b.x = 9\n    println(a.x + b.x)\n", "10"),
        ("printed", "    let a = V { x: 1, y: 2 }\n    println(a)\n", "V { x: 1, y: 2 }"),
        ("compared", "    let a = V { x: 1, y: 2 }\n    println(a == V { x: 1, y: 2 })\n", "true"),
        ("method", "    let a = V { x: 1, y: 2 }\n    println(a.x + keep2(a))\n", "6"),
    ] {
        assert!(objects(body, tag) >= 1, "{tag}");
        assert_eq!(run(body, tag), [want], "{tag}");
    }
}

#[test]
fn a_name_bound_twice_in_a_body_is_not_held_in_registers() {
    let body = "    let p = V { x: 1, y: 2 }\n    println(p.x)\n    if p.x == 1 {\n        let p = V { x: 5, y: 6 }\n        println(p.y)\n    }\n    println(p.y)\n";
    assert_eq!(objects(body, "shadow"), 2);
    assert_eq!(run(body, "shadow"), ["1", "6", "2"]);
}

#[test]
fn a_struct_across_a_yield_keeps_its_fields() {
    let out = run(
        "    let g = counter()\n    var sum = 0\n    for x in g {\n        sum += x\n    }\n    println(sum)\n}\n\nfn counter() -> Stream<Int> {\n    var v = V { x: 1, y: 10 }\n    yield v.x\n    v.x += 1\n    yield v.x + v.y\n    v.y = 100\n    yield v.y\n",
        "yield",
    );
    assert_eq!(out, ["113"]);
}

#[test]
fn a_struct_with_too_many_fields_stays_an_object() {
    let fields = (0..9).map(|i| format!("    var f{i}: Int\n")).collect::<String>();
    let init = (0..9).map(|i| format!("f{i}: {i}, ")).collect::<String>();
    let src = format!("struct Wide {{\n{fields}}}\n\nfn main() {{\n    var w = Wide {{ {init}}}\n    w.f0 += w.f8\n    println(w.f0)\n}}\n");
    let with = allocations(&src, "wide9");
    let without = allocations("fn main() {\n}\n", "wide9_base");
    assert_eq!(with - without, 1);
}

fn run_src(source: &str, tag: &str) -> Vec<String> {
    let dir = dir(tag);
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().filter(|l| !l.starts_with("GC:")).map(String::from).collect()
}

fn user_objects(source: &str, tag: &str) -> usize {
    allocations(source, tag) - allocations("fn main() {\n}\n", &format!("{tag}_base"))
}

#[test]
fn a_reader_function_takes_the_struct_in_registers() {
    let body = "    let v = V { x: 3, y: 4 }\n    println(len2(v))\n";
    assert_eq!(objects(body, "reader"), 0);
    assert_eq!(run(body, "reader"), ["25"]);
}

#[test]
fn a_literal_argument_is_built_in_registers() {
    let body = "    println(len2(V { x: 1, y: 2 }))\n    println(len2(V { x: len2(V { x: 1, y: 1 }), y: 1 }))\n";
    assert_eq!(objects(body, "literal_arg"), 0);
    assert_eq!(run(body, "literal_arg"), ["5", "5"]);
}

#[test]
fn a_struct_in_registers_passes_through_another_reader() {
    let defs = "fn outer(v: V) -> Int {\n    return len2(v) + v.x\n}\n";
    let body = "    let v = V { x: 3, y: 4 }\n    println(outer(v))\n";
    assert_eq!(objects_with(defs, body, "pass_on"), 0);
    assert_eq!(run_with(defs, body, "pass_on"), ["28"]);
}

#[test]
fn a_function_called_with_an_object_keeps_taking_objects() {
    let body = "    let xs = [V { x: 1, y: 2 }]\n    println(len2(xs[0]))\n    let v = V { x: 3, y: 4 }\n    println(len2(v))\n";
    assert_eq!(run(body, "mixed_args"), ["5", "25"]);
}

const VEC: &str = "struct Vec2 {\n    var x: Int\n    var y: Int\n    pub fn dot(self, o: Vec2) -> Int { return self.x * o.x + self.y * o.y }\n    pub fn scaled(self, k: Int) -> Vec2 { return Vec2 { x: self.x * k, y: self.y * k } }\n    pub fn sum(self) -> Int { return self.x + self.y }\n    pub fn bump(var self) { self.x += 1 }\n    pub fn same(self) -> Vec2 { return self }\n}\n\n";

#[test]
fn methods_take_self_and_return_in_registers() {
    let src = format!("{VEC}fn main() {{\n    let a = Vec2 {{ x: 1, y: 2 }}\n    let b = Vec2 {{ x: 3, y: 4 }}\n    let c = a.scaled(2)\n    println(a.dot(b))\n    println(c.sum())\n    println(c.x + b.sum())\n}}\n");
    assert_eq!(run_src(&src, "methods"), ["11", "6", "9"]);
    assert_eq!(user_objects(&src, "methods"), 0);
}

#[test]
fn a_method_on_an_object_receiver_still_works() {
    let src = format!("{VEC}fn main() {{\n    let xs = [Vec2 {{ x: 1, y: 2 }}]\n    let b = Vec2 {{ x: 3, y: 4 }}\n    println(xs[0].dot(b))\n    println(b.dot(b))\n    println(xs[0].scaled(3).sum())\n}}\n");
    assert_eq!(run_src(&src, "obj_recv"), ["11", "25", "9"]);
}

#[test]
fn var_self_and_returning_self_stay_objects() {
    let src = format!("{VEC}fn main() {{\n    var a = Vec2 {{ x: 1, y: 2 }}\n    a.bump()\n    let b = a.same()\n    println(a.x)\n    println(b.x + b.y)\n}}\n");
    assert_eq!(run_src(&src, "var_self"), ["2", "4"]);
}

#[test]
fn a_method_calls_another_with_self_in_registers() {
    let src = "struct P {\n    var x: Int\n    var y: Int\n    pub fn sum(self) -> Int { return self.x + self.y }\n    pub fn twice(self) -> Int { return self.sum() * 2 }\n    pub fn both(self, o: P) -> Int { return self.sum() + o.sum() }\n}\n\nfn main() {\n    let a = P { x: 1, y: 2 }\n    let b = P { x: 10, y: 20 }\n    println(a.twice())\n    println(a.both(b))\n}\n";
    assert_eq!(run_src(src, "chain_methods"), ["6", "33"]);
    assert_eq!(user_objects(src, "chain_methods"), 0);
}

#[test]
fn registers_and_objects_agree_across_a_loop() {
    let src = format!("{VEC}fn main() {{\n    var total = 0\n    var i = 0\n    while i < 1000 {{\n        let a = Vec2 {{ x: i, y: 1 }}\n        let b = a.scaled(2)\n        total += a.dot(b) + b.sum()\n        i += 1\n    }}\n    println(total)\n}}\n");
    assert_eq!(run_src(&src, "loop_methods"), ["666670000"]);
}
