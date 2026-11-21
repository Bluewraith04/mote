//! A program prints the same unoptimized and under every pass configuration of the optimizer.

mod common;

use common::blocks::{build, in_parallel};
use common::expect::{check, tests_text};
use mote::{run_main, run_tests, Ended, Target};
use mote_ir::opt::{configurations, optimize_with, Passes};
use mote_ir::verify;

const HEAD: &str = "";

/// What the source prints, with only `passes` applied after the build when given.
fn shown(source: &str, passes: Option<Passes>) -> String {
    let mut pipeline = build(source, Target::Run).unwrap_or_else(|errors| panic!("{}", errors.join("; ")));
    if let Some(passes) = passes {
        optimize_with(&mut pipeline.module, &mut pipeline.program, &mut pipeline.interner, passes);
        assert_eq!(verify(&pipeline.module, &pipeline.program, &pipeline.interner), []);
    }
    let run = run_main(&mut pipeline).unwrap_or_else(|failure| panic!("{failure}"));
    assert_eq!(run.ended, Ended::Returned, "{}", run.output);
    run.output
}

/// What the source prints, the same with every pass configuration.
fn printed(source: &str) -> String {
    let expected = shown(source, None);
    for (name, passes) in configurations() {
        assert_eq!(shown(source, Some(passes)), expected, "with {name}");
    }
    expected
}

/// What `main` returns, where `main` is `fn main() -> Int`.
fn same(source: &str, expected: i128) {
    let wrapped = format!("{}\nfn main() {{\n    println(value())\n}}\n", source.replacen("fn main() -> Int", "fn value() -> Int", 1));
    assert_eq!(printed(&wrapped), format!("{expected}\n"));
}

/// What `f` prints, where `f` is `fn f()`.
fn called(source: &str) -> String {
    printed(&format!("{HEAD}{source}\nfn main() {{\n    f()\n}}\n"))
}

#[test]
fn passes_every_pass_configuration_gives_the_recorded_result_of_every_block_of_std_tests() {
    let dir = common::root().join("mote_std/std/tests");
    let mut files: Vec<_> = std::fs::read_dir(&dir).expect("the std/tests directory").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.extension().is_some_and(|ext| ext == "mote")).collect();
    files.sort();
    let jobs: Vec<(&std::path::PathBuf, &str, Passes)> = files.iter().flat_map(|path| configurations().into_iter().map(move |(name, passes)| (path, name, passes))).collect();
    let found = in_parallel(&jobs, |_, (path, name, passes)| {
        let source = std::fs::read_to_string(path).expect("a readable file");
        let mut pipeline = build(&source, Target::Test).unwrap_or_else(|errors| panic!("{}: {}", common::relative(path), errors.join("; ")));
        optimize_with(&mut pipeline.module, &mut pipeline.program, &mut pipeline.interner, *passes);
        let problems = verify(&pipeline.module, &pipeline.program, &pipeline.interner);
        if !problems.is_empty() {
            return Some(format!("{} with {name}: verify {problems:?}", common::relative(path)));
        }
        match run_tests(&mut pipeline) {
            Ok(run) => check(&path.with_extension("expect"), &tests_text(&run), false).map(|problem| format!("with {name}: {problem}")),
            Err(failure) => Some(format!("{} with {name}: {failure}", common::relative(path))),
        }
    });
    assert_eq!(found.into_iter().flatten().collect::<Vec<_>>(), Vec::<String>::new());
}

const BIG: &str = "fn big(a: Int) -> Int {\n let b = a * 2\n let c = b + a\n let d = c * c\n let e = d - b\n let g = e + c\n let h = g * 3\n let i = h - a\n let j = i * i\n let k = j + b\n return k - 1\n}\n";

/// What `BIG` computes.
fn big(a: i128) -> i128 {
    let b = a * 2;
    let c = b + a;
    let d = c * c;
    let e = d - b;
    let g = e + c;
    let h = g * 3;
    let i = h - a;
    let j = i * i;
    j + b - 1
}

const COUNT: &str = "fn count(n: Int) -> Stream<Int> {\n var i = 0\n while i < n {\n yield i\n i += 1\n }\n}\n";

#[test]
fn passes_a_write_to_either_side_of_a_copy_ends_the_sharing() {
    same("struct P {\n var x: Int\n}\nfn main() -> Int {\n var a = P { x: 1 }\n let b = a\n a.x = 5\n return b.x * 10 + a.x\n}", 15);
    same("struct P {\n var x: Int\n}\nfn main() -> Int {\n var a = P { x: 1 }\n var b = a\n b.x = 7\n return a.x * 10 + b.x\n}", 17);
    same("fn main() -> Int {\n var a = [1, 2, 3]\n let b = a\n a.push(4)\n return b.len() * 10 + a.len()\n}", 44);
    same("fn main() -> Int {\n var a: [Int; 3] = [1, 2, 3]\n let b = a\n a[0] = 9\n return b[0] * 10 + a[0]\n}", 19);
}

#[test]
fn passes_loops_and_var_parameters_keep_their_values() {
    same("fn bump(var n: Int) {\n n += 1\n}\nfn main() -> Int {\n var c = 0\n for i in 0..5 {\n bump(c)\n }\n return c\n}", 5);
    same("fn main() -> Int {\n var s = 0\n var i = 0\n while i < 10 {\n let k = i\n s += k\n i += 1\n }\n return s\n}", 45);
}

#[test]
fn passes_closures_and_cells_still_share_a_variable() {
    same("fn main() -> Int {\n var c = 0\n let inc = || { c += 1 }\n inc()\n inc()\n return c\n}", 2);
}

#[test]
fn passes_a_literal_match_still_picks_its_arm() {
    for (n, expected) in [(1, 10), (2, 20), (3, 30), (4, 0), (-1, 0)] {
        same(&format!("fn pick(n: Int) -> Int {{\n return match n {{\n 1 => 10\n 2 => 20\n 3 => 30\n _ => 0\n }}\n}}\nfn main() -> Int {{ return pick({n}) }}"), expected);
    }
}

#[test]
fn passes_a_task_still_sees_the_array_as_it_was_when_it_started() {
    same("fn main() -> Int {\n var xs: [Int] = [1, 2, 3]\n let ys = xs\n let h = spawn { return ys[0] }\n xs[0] = 9\n return h.await().unwrap() * 10 + xs[0]\n}", 19);
    same("fn main() -> Int {\n let xs: [Int] = [1, 2, 3]\n let h = spawn { return xs[0] + xs[2] }\n return h.await().unwrap()\n}", 4);
}

#[test]
fn passes_a_recursive_function_gives_its_result() {
    same("fn fact(n: Int) -> Int {\n if n <= 1 {\n return 1\n }\n return n * fact(n - 1)\n}\nfn f() -> Int {\n return fact(5)\n}\nfn main() -> Int { return f() }", 120);
}

#[test]
fn passes_an_inlined_call_gives_the_same_results() {
    same(&format!("{BIG}fn main() -> Int {{\n return big(3)\n}}"), big(3));
    same(&format!("{BIG}fn main() -> Int {{\n return big(3) + big(4)\n}}"), big(3) + big(4));
    same("fn sum(n: Int) -> Int {\n var s = 0\n var i = 0\n while i < n {\n s += i\n i += 1\n }\n return s\n}\nfn main() -> Int {\n var t = 0\n for k in 0..5 {\n t += sum(k)\n }\n return t\n}", 10);
    same("fn pick(n: Int) -> Int {\n if n < 0 {\n return 0 - n\n }\n if n == 0 {\n return 100\n }\n return n\n}\nfn main() -> Int {\n return pick(0 - 7) + pick(0) + pick(5)\n}", 112);
    same("fn make(n: Int) -> Int {\n let f = |x: Int| -> Int { return x + n }\n return f(1)\n}\nfn main() -> Int {\n return make(4) * 10 + make(1)\n}", 52);
}

#[test]
fn passes_an_inlined_call_copies_its_arguments_and_returns_the_var_parameters() {
    same("struct P {\n var x: Int\n}\nfn change(p: P) -> Int {\n var q = p\n q.x = 9\n return q.x\n}\nfn main() -> Int {\n var a = P { x: 1 }\n let r = change(a)\n return a.x * 10 + r\n}", 19);
    same("fn step(var n: Int, k: Int) -> Int {\n n += k\n return n * 2\n}\nfn main() -> Int {\n var c = 1\n let r = step(c, 4)\n return c * 100 + r\n}", 510);
}

#[test]
fn passes_split_records_keep_their_values() {
    same("struct P {\n var x: Int\n var y: Int\n}\nfn main() -> Int {\n var p = P { x: 0, y: 0 }\n for i in 0..4 {\n p.x += i\n p.y += 1\n }\n return p.x * 10 + p.y\n}", 64);
    same("class C {\n var n: Int\n}\nfn main() -> Int {\n var a = C { n: 1 }\n var b = a\n b.n = 5\n return a.n\n}", 5);
    same("fn main() -> Int {\n var t = (1, 2)\n let u = t\n t.0 = 7\n return u.0 * 10 + t.0\n}", 17);
}

#[test]
fn passes_an_unrolled_loop_computes_what_the_loop_computed() {
    same("fn main() -> Int {\n var total = 0\n for i in 0..5 {\n total = total * 3 + i\n }\n return total\n}", 58);
}

#[test]
fn passes_a_loop_with_a_break_or_a_nested_loop_computes_what_it_computed() {
    same("fn main() -> Int {\n var total = 0\n for i in 0..6 {\n if i == 4 {\n break\n }\n total += i\n }\n return total\n}", 6);
    same("fn main() -> Int {\n var total = 0\n for i in 0..3 {\n for j in 0..3 {\n total += i * 10 + j\n }\n }\n return total\n}", 99);
}

#[test]
fn passes_a_generator_keeps_its_locals_between_reads() {
    assert_eq!(called(&format!("{COUNT}fn f() {{\n var total = 0\n for v in count(5) {{\n total += v\n println(total)\n }}\n}}")), "0\n1\n3\n6\n10\n");
}

#[test]
fn passes_a_generator_runs_only_as_far_as_it_is_read() {
    let source = "fn noisy() -> Stream<Int> {\n println(\"a\")\n yield 1\n println(\"b\")\n yield 2\n println(\"c\")\n}\nfn f() {\n var s = noisy()\n println(\"start\")\n println(s.next()!)\n println(s.next()!)\n println(\"end\")\n}";
    assert_eq!(called(source), "start\na\n1\nb\n2\nend\n");
}

#[test]
fn passes_generators_of_one_function_have_separate_frames() {
    let source = format!("{COUNT}fn f() {{\n var a = count(3)\n var b = count(3)\n println(a.next()!)\n println(a.next()!)\n println(b.next()!)\n println(a.next()!)\n println(b.next()!)\n}}");
    assert_eq!(called(&source), "0\n1\n0\n2\n1\n");
}

#[test]
fn passes_a_generator_that_has_ended_stays_ended() {
    let source = "fn one() -> Stream<Int> {\n yield 7\n}\nfn f() {\n var s = one()\n println(s.next()!)\n println(s.next() == null)\n println(s.next() == null)\n}";
    assert_eq!(called(source), "7\ntrue\ntrue\n");
}

#[test]
fn passes_a_generator_resumes_in_the_branch_it_left() {
    let source = "fn pick(n: Int) -> Stream<String> {\n for i in 0..n {\n if i % 2 == 0 {\n yield \"even\"\n } else {\n yield \"odd\"\n }\n println(i)\n }\n}\nfn f() {\n for word in pick(3) {\n println(word)\n }\n}";
    assert_eq!(called(source), "even\n0\nodd\n1\neven\n2\n");
}

#[test]
fn passes_a_generator_keeps_its_parameters_and_strings_and_tuples_across_yields() {
    let source = "fn tag(name: String, n: Int) -> Stream<(String, Int)> {\n var pair = (name, 0)\n for i in 0..n {\n pair = (name, i)\n yield pair\n }\n yield (name + \"!\", n)\n}\nfn f() {\n for (name, i) in tag(\"x\", 2) {\n println(name)\n println(i)\n }\n}";
    assert_eq!(called(source), "x\n0\nx\n1\nx!\n2\n");
}

#[test]
fn passes_a_generator_cell_survives_a_yield() {
    let source = "fn counter() -> Stream<Int> {\n var n = 0\n let inc = || n += 1\n inc()\n yield n\n inc()\n yield n\n inc()\n yield n\n}\nfn f() {\n for v in counter() {\n println(v)\n }\n}";
    assert_eq!(called(source), "1\n2\n3\n");
}

#[test]
fn passes_a_generator_can_read_another() {
    let source = format!("{COUNT}fn doubled(n: Int) -> Stream<Int> {{\n for v in count(n) {{\n yield v * 2\n }}\n}}\nfn f() {{\n for v in doubled(4) {{\n println(v)\n }}\n}}");
    assert_eq!(called(&source), "0\n2\n4\n6\n");
}

#[test]
fn passes_a_generator_stops_at_a_return_and_at_a_break_of_its_reader() {
    let source = "fn upto(n: Int) -> Stream<Int> {\n var i = 0\n while true {\n if i == n { return }\n yield i\n i += 1\n }\n}\nfn f() {\n for v in upto(2) {\n println(v)\n }\n for v in upto(100) {\n if v == 1 { break }\n println(v)\n }\n}";
    assert_eq!(called(source), "0\n1\n0\n");
}

#[test]
fn passes_sized_array_elementwise_broadcast_and_mask_reductions() {
    let source = "import { all, any } from std.array\nfn f() {\n var a: [Int; 3] = [1, 2, 3]\n let b: [Int; 3] = [4, 5, 6]\n let c = (a + b) * 2 - 1\n println(c[0])\n println(c[2])\n println(all(c == [9, 13, 17]))\n let neg = -a\n println(neg[1])\n a += 10\n a *= [1, 2, 3]\n println(a[0])\n println(a[1])\n println(a[2])\n println(all(a > 10))\n println(any(a == 24))\n let m1 = a >= 24\n let m2 = !(a == 39)\n println(all(m1 & m2))\n println(any(m1 & m2))\n println(all(m1 | (a == 11)))\n println(any(m1 ^ [false, true, false]))\n let fl: [Float; 2] = [1.5, 2.5]\n let fr = (fl * 2.0) + 1.0\n println(all(fr == [4.0, 6.0]))\n var big: [Int; 40] = [1; 40]\n big += 2\n big = 10 - big\n println(big[0])\n println(big[39])\n println(all(big == 7))\n big[5] = 0\n println(all(big == 7))\n println(any(big == 0))\n}";
    assert_eq!(
        printed(&format!("{source}\nfn main() {{\n    f()\n}}\n")),
        "9\n17\ntrue\n-2\n11\n24\n39\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue\ntrue\n7\n7\ntrue\nfalse\ntrue\n",
    );
}

#[test]
fn passes_std_array_category_a_and_b_dyn_and_fixed() {
    let source = "import { all, any, none, all_by, any_by, none_by, count, count_by, sum, product, mean, min, max, argmin, argmax, dot, norm_sq, fold, reduce, scan } from std.array
fn f() {
    let fx: [Int; 4] = [3, 1, 4, 2]
    let fy: [Int; 4] = [2, 0, 1, 5]
    let dx: [Int] = [10, 20, 30, 20]
    let emp: [Int] = []
    let dm: [Bool] = [true, false, true]
    println(all(dm))
    println(any(dm))
    println(none(dm))
    let fm: [Bool; 2] = [false, false]
    println(none(fm))
    println(all_by(fx, |x| x > 0))
    println(any_by(dx, |x| x == 30))
    println(none_by(fx, |x| x < 0))
    println(count(dx, 20))
    println(count_by(fx, |x| x >= 3))
    println(sum(fx))
    println(product(fx))
    println(mean(fx))
    println(mean(emp))
    println(min(fx)!)
    println(max(fx)!)
    println(argmin(fx)!)
    println(argmax(fx)!)
    println(min(emp) ?? -1)
    println(dot(fx, fy))
    println(norm_sq(fx))
    println(fold(dx, 1, |acc, x| acc + x))
    println(reduce(fx, |a, b| a + b)!)
    println(reduce(emp, |a, b| a + b) ?? -1)
    let scf = scan(fx, 0, |acc, x| acc + x)
    println(scf[0])
    println(scf[3])
    let scd = scan(dx, 0, |acc, x| acc + x)
    println(scd[1])
    println(scd[3])
    let u8s: [UInt8; 2] = [200u8, 200u8]
    println(mean(u8s))
}";
    assert_eq!(
        printed(&format!("{source}\nfn main() {{\n    f()\n}}\n")),
        "false\ntrue\nfalse\ntrue\ntrue\ntrue\ntrue\n2\n2\n10\n24\n2.5\n0.0\n1\n4\n1\n2\n-1\n20\n30\n81\n10\n-1\n3\n10\n30\n80\n200.0\n",
    );
}

#[test]
fn passes_std_array_category_c_d_e_dyn_and_fixed() {
    let source = "import { all, any, find, find_index, index_of, select, map, zip_with, reverse, clamp, filter, zip, enumerate, concat, take, drop, flatten, sort, sort_by } from std.array
fn f() {
    let fx: [Int; 4] = [30, 10, 40, 20]
    let fy: [Int; 4] = [1, 2, 3, 4]
    let dx: [Int] = [5, 15, 25, 15]
    let emp: [Int] = []
    println(find(fx, |x| x > 25)!)
    println(find(dx, |x| x > 100) ?? -1)
    println(find_index(fx, |x| x == 40)!)
    println(index_of(dx, 15)!)
    println(index_of(emp, 15) ?? -1)
    let fm: [Bool; 4] = [true, false, true, false]
    let sel = select(fm, fx, fy)
    println(all(sel == [30, 2, 40, 4]))
    let dm: [Bool] = [false, true]
    let da: [Int] = [1, 2]
    let db: [Int] = [10, 20]
    let ds = select(dm, da, db)
    println(ds[0])
    println(ds[1])
    let mf = map(fx, |x| x + 1)
    println(all(mf == [31, 11, 41, 21]))
    let md = map(dx, |x| x * 2)
    println(md[0])
    println(md[3])
    let zf = zip_with(fx, fy, |a, b| a + b)
    println(all(zf == [31, 12, 43, 24]))
    let rf = reverse(fx)
    println(all(rf == [20, 40, 10, 30]))
    let rd = reverse(da)
    println(rd[0])
    let cf = clamp(fx, 15, 35)
    println(all(cf == [30, 15, 35, 20]))
    let fl = filter(fx, |x| x >= 20)
    println(fl.len())
    println(fl[0])
    println(fl[2])
    let zp = zip(fx, fy)
    println(zp[2].0)
    println(zp[2].1)
    let en = enumerate(da)
    println(en[1].0)
    println(en[1].1)
    let cc = concat(fx, da)
    println(cc.len())
    println(cc[4])
    let tk = take(fx, 2)
    println(tk.len())
    println(tk[1])
    let dr = drop(fx, 2)
    println(dr.len())
    println(dr[0])
    let nest: [[Int; 2]; 2] = [[1, 2], [3, 4]]
    let ft = flatten(nest)
    println(ft.len())
    println(ft[3])
    let sf = sort(fx)
    println(all(sf == [10, 20, 30, 40]))
    let sb = sort_by(dx, |a, b| b - a)
    println(sb[0])
    println(sb[3])
}";
    assert_eq!(
        printed(&format!("{source}\nfn main() {{\n    f()\n}}\n")),
        "30\n-1\n2\n1\n-1\ntrue\n10\n2\ntrue\n10\n30\ntrue\ntrue\n2\ntrue\n3\n30\n20\n40\n3\n1\n2\n6\n1\n2\n10\n2\n40\n4\n4\ntrue\n25\n5\n",
    );
}

#[test]
fn passes_a_struct_holding_a_fixed_array() {
    let source = "import { sum } from std.array
struct Vec3 {
    var data: [Float; 3]
    pub fn dot(self, other: Vec3) -> Float {
        return sum(self.data * other.data)
    }
    pub fn to_string(self) -> String {
        return \"Vec3<${self.data}>\"
    }
}
fn main() {
    var a = Vec3 { data: [1.0, 2.0, 3.0] }
    let b = a
    a.data[0] = 10.0
    let dyn_arr: [Int] = [4, 5]
    println(b.data[0])
    println(a.data[0])
    println(a.dot(b))
    println(a.to_string())
    println(\"${dyn_arr} ${dyn_arr.to_string()} ${b.data.to_string()}\")
}";
    assert_eq!(printed(source), "1.0\n10.0\n23.0\nVec3<[10.0, 2.0, 3.0]>\n[4, 5] [4, 5] [1.0, 2.0, 3.0]\n");
}
