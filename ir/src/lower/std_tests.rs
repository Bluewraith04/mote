//! The std functions real programs reach, lowered: what is left shows how far the slices have come.

use std::collections::BTreeMap;

use mote_semantics::compile_all;

use super::tests::lowered;
use crate::{lower, verify};

const COLLECTIONS: &str = "import { set_from, union } from std.collections\nfn f() {\n var xs: List<Int> = [3, 1, 2]\n xs.push(4)\n let n = xs.len()\n let h = xs.first()\n let t = xs[1]\n xs[0] = 9\n var m: Map<String, Int> = {\"a\": 1}\n m[\"b\"] = 2\n let a = set_from([1, 2, 3])\n let u = union(a, set_from([3, 4]))\n var grid: [Int] = [0; 100]\n grid[5] = 1\n let g = grid.len()\n let part = xs[1..3]\n let m2 = Map<String, Int>()\n let s = Set<Int>()\n let found = \"hello\".find(\"l\")\n let parts = \"a,b,c\".split(\",\")\n for i in 0..3 { xs.push(i) }\n for (k, v) in m.items() { let line = \"${k}=${v}\" }\n}";

const SYNC: &str = "import Channel from std.sync\nimport { merge, broadcast } from std.sync\nimport { await_all, sleep, with_timeout } from std.task\nfn f() {\n let m = Mutex([1])\n let size = m.lock(|xs| { return xs.len() })\n let o = Once<Int>()\n let value = o.get_or_init(|| 1)\n let count = AtomicInt(0)\n let before = count.fetch_add(1)\n let flag = AtomicBool(false)\n let was = flag.swap(true)\n let (tx, rx) = Channel<Int>(3)\n tx.send(10)\n tx.close()\n let v = rx.recv()\n let h = spawn { return 6 * 7 }\n let n = h.await().unwrap()\n sleep(1)\n let a = spawn { return 1 }\n let b = spawn { return 2 }\n let results = await_all([a, b])\n let t = with_timeout(100, || 5)\n let (tx2, rx2) = Channel<Int>(0)\n let all = merge([rx2])\n let (tx3, rx3) = Channel<Int>(0)\n let outs = broadcast(rx3, 2)\n}";

const ITER: &str = "import { contains, enumerate, filter, fold, map, sort, sum } from std.iter\nimport { join, lines, pad_start, parse_int } from std.string\nimport { PI, sqrt } from std.math\nimport { assert, assert_eq, assert_ne } from std.test\nfn f() {\n let doubled = map([1, 2, 3], |n| n * 2)\n let total = fold(doubled, 0, |a, n| a + n)\n let kept = filter(doubled, |n| n > 2)\n let sorted = sort([3, 1, 2])\n let s = sum(sorted)\n let pairs = enumerate([\"a\", \"b\"])\n let has = contains([1, 2], 2)\n let csv = join([\"a\", \"b\"], \",\")\n let n = parse_int(\"42\")!\n let ls = lines(\"a\\nb\")\n let p = pad_start(\"7\", 3, \"0\")\n let r = sqrt(2.0) * PI\n}\ntest \"adds\" {\n assert_eq(1 + 1, 2)\n assert_ne(\"a\", \"b\")\n assert(true)\n}";

/// How many times each construct that is not lowered yet appears.
fn left_over(source: &str) -> BTreeMap<&'static str, usize> {
    let (compiled, lowered) = lowered(source);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let mut kinds = BTreeMap::new();
    for item in &lowered.unsupported {
        *kinds.entry(item.what).or_insert(0) += 1;
    }
    kinds
}

#[test]
fn the_collections_program_lowers_completely() {
    assert_eq!(left_over(COLLECTIONS), BTreeMap::new());
}

#[test]
fn the_iter_program_lowers_completely() {
    assert_eq!(left_over(ITER), BTreeMap::new());
}

#[test]
fn the_sync_program_lowers_completely() {
    assert_eq!(left_over(SYNC), BTreeMap::new());
}

#[test]
fn every_function_of_std_lowers_and_verifies_with_int_for_each_type_parameter() {
    let mut compiled = compile_all("fn main() {}");
    assert_eq!(compiled.errors, Vec::<String>::new());
    let lowered = lower(&mut compiled.program, &mut compiled.interner);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    assert!(lowered.module.functions.iter().count() > 400);
}
