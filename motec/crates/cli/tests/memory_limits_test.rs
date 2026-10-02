//! The task stack limit, the heap limit, and the flags and variables that set them.

use std::process::Command;

fn mote(source: &str, tag: &str, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_limits_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mote"));
    cmd.arg("run").arg(&file).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const DEPTH: &str = "\
fn depth(n: Int) -> Int {
    if n == 0 {
        return 0
    }
    return depth(n - 1) + 1
}

fn main() {
    println(depth(20000))
}
";

const RUNAWAY: &str = "\
fn f(n: Int) -> Int {
    return f(n + 1) + 1
}

fn main() {
    println(f(0))
}
";

const HOARD: &str = "\
fn main() {
    var keep: List<String> = []
    var i = 0
    while true {
        keep.push(\"item ${i}\")
        i += 1
    }
}
";

const CHURN: &str = "\
fn main() {
    var total = 0
    var i = 0
    while i < 30000 {
        var xs: List<Int> = []
        var j = 0
        while j < 100 {
            xs.push(j)
            j += 1
        }
        total += xs.len()
        i += 1
    }
    println(total)
}
";

const EMPTIED_LISTS: &str = "\
fn main() {
    var lists: List<List<Int>> = []
    var k = 0
    while k < 12 {
        var xs: List<Int> = []
        var i = 0
        while i < 40000 {
            xs.push(i)
            i += 1
        }
        while xs.len() > 0 {
            xs.pop()
        }
        lists.push(xs)
        k += 1
    }
    println(lists.len())
}
";

#[test]
fn emptied_lists_give_their_memory_back() {
    let (ok, text) = mote(EMPTIED_LISTS, "emptied", &["--max-heap", "8MiB"], &[]);
    assert!(ok, "{text}");
    assert!(text.contains("12"), "{text}");
}

#[test]
fn mem_stats_prints_what_the_run_used() {
    let (ok, text) = mote(CHURN, "stats", &["--mem-stats"], &[]);
    assert!(ok, "{text}");
    for key in ["mem.heap.peak_live", "mem.heap.chunks", "mem.gc.collections", "mem.gc.pause_max_ms", "mem.heap.alloc.units_", "mem.tasks.spawned = 1", "mem.regions.scopes"] {
        assert!(text.contains(key), "missing {key}:\n{text}");
    }
}

const DEEP_REGIONS: &str = "\
struct Point {
    var x: Int
    var y: Int
}

fn tag(p: Point) -> Int {
    let q = p
    return q.x
}

fn f(n: Int) -> Int {
    let p = Point { x: n, y: 1 }
    if n == 0 {
        return p.y
    }
    return f(n - 1) + tag(p)
}

fn main() {
    println(f(1000))
}
";

#[test]
fn regions_nested_past_250_levels_use_the_heap() {
    let (ok, text) = mote(DEEP_REGIONS, "regions_deep", &["--mem-stats"], &[]);
    assert!(ok, "{text}");
    assert!(text.contains("500501"), "{text}");
    assert!(text.contains("mem.regions.too_deep = 751"), "1,001 frames, the first 250 in regions:\n{text}");
}

#[test]
fn deep_recursion_within_the_limit_runs() {
    let (ok, text) = mote(DEPTH, "deep", &[], &[]);
    assert!(ok, "{text}");
    assert!(text.contains("20000"), "{text}");
}

#[test]
fn a_small_stack_limit_faults_deep_recursion() {
    let (ok, text) = mote(DEPTH, "small", &[], &[("MOTE_MAX_STACK", "64KiB")]);
    assert!(!ok, "{text}");
    assert!(text.contains("stack overflow"), "{text}");
}

#[test]
fn runaway_recursion_faults_instead_of_exhausting_memory() {
    let (ok, text) = mote(RUNAWAY, "runaway", &[], &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("stack overflow"), "{text}");
}

#[test]
fn a_bad_stack_limit_is_reported() {
    let (ok, text) = mote(DEPTH, "bad", &[], &[("MOTE_MAX_STACK", "lots")]);
    assert!(!ok, "{text}");
    assert!(text.contains("MOTE_MAX_STACK"), "{text}");
}

#[test]
fn a_growing_heap_faults_at_the_limit() {
    let (ok, text) = mote(HOARD, "hoard", &["--max-heap", "32MiB"], &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("out of memory: heap limit 32.0 MiB reached"), "{text}");
}

#[test]
fn one_allocation_past_the_limit_faults_at_once() {
    let (ok, text) = mote("fn main() {\n    let b = Bytes(1000000000)\n    println(b.len())\n}\n", "huge", &["--max-heap", "64MiB"], &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("allocation passes the heap limit of 64.0 MiB"), "{text}");
}

#[test]
fn the_heap_limit_comes_from_the_environment_too() {
    let (ok, text) = mote(HOARD, "hoard_env", &[], &[("MOTE_MAX_HEAP", "32MiB")]);
    assert!(!ok, "{text}");
    assert!(text.contains("out of memory"), "{text}");
}

#[test]
fn garbage_under_the_limit_is_collected_not_faulted() {
    let (ok, text) = mote(CHURN, "churn", &["--max-heap", "32MiB"], &[]);
    assert!(ok, "{text}");
    assert!(text.contains("3000000"), "{text}");
}

#[test]
fn an_unlimited_heap_runs_normally() {
    let (ok, text) = mote(CHURN, "unlimited", &["--max-heap", "unlimited"], &[]);
    assert!(ok, "{text}");
}

#[test]
fn a_bad_heap_limit_is_reported() {
    let (ok, text) = mote(CHURN, "bad_heap", &["--max-heap", "lots"], &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("--max-heap"), "{text}");
}
