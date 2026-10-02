
use runtime::Runtime;

const MAIN: &str = "
fn main() -> Int {
    let a = spawn { work() }
    let b = spawn { work() }
    let c = spawn { work() }
    return a.join().unwrap() + b.join().unwrap() + c.join().unwrap() + work()
}
";

struct Program {
    name: &'static str,
    source: &'static str,
    per_task: i64,
}

const PROGRAMS: &[Program] = &[
    Program {
        name: "list churn",
        source: "
fn work() -> Int {
    var total = 0
    var round = 0
    while round < 300 {
        var xs: List<Int> = []
        var i = 0
        while i < 200 {
            xs.push(i + round)
            i = i + 1
        }
        total = total + xs.len()
        round = round + 1
    }
    return total
}
",
        per_task: 300 * 200,
    },
    Program {
        name: "string churn",
        source: "
fn work() -> Int {
    var total = 0
    var i = 0
    while i < 1500 {
        var s = \"k\"
        var j = 0
        while j < 20 {
            s = s + \"x\"
            j = j + 1
        }
        total = total + s.len()
        i = i + 1
    }
    return total
}
",
        per_task: 1500 * 21,
    },
    Program {
        name: "map churn",
        source: "
fn fill(n: Int) -> Int {
    var m: Map<String, Int> = {}
    var i = 0
    while i < n {
        m.set(\"k${i % 50}\", i)
        i = i + 1
    }
    return m.len()
}
fn work() -> Int {
    var total = 0
    var r = 0
    while r < 150 {
        total = total + fill(120)
        r = r + 1
    }
    return total
}
",
        per_task: 150 * 50,
    },
    Program {
        name: "struct churn",
        source: "
struct Pt {
    x: Int
    y: Int
}
fn norm(p: Pt) -> Int {
    return p.x + p.y
}
fn work() -> Int {
    var total = 0
    var kept: List<Pt> = []
    var i = 0
    while i < 4000 {
        let p = Pt { x: i, y: 1 }
        kept.push(p)
        if kept.len() == 50 {
            kept.clear()
        }
        total = total + norm(p)
        i = i + 1
    }
    return total
}
",
        per_task: 4000 * 3999 / 2 + 4000,
    },
    Program {
        name: "closure churn",
        source: "
fn make(n: Int) -> (Int) -> Int {
    return |x: Int| x + n
}
fn work() -> Int {
    var total = 0
    var i = 0
    while i < 3000 {
        let f = make(i)
        total = total + f(1)
        i = i + 1
    }
    return total
}
",
        per_task: 3000 * 2999 / 2 + 3000,
    },
    Program {
        name: "wide frame with retained lists",
        source: "
fn work() -> Int {
    var a: List<Int> = [1, 2, 3, 4, 5]
    var b: List<Int> = [1, 2, 3, 4, 5]
    var c: List<Int> = [1, 2, 3, 4, 5]
    var d: List<Int> = [1, 2, 3, 4, 5]
    var e: List<Int> = [1, 2, 3, 4, 5]
    var f: List<Int> = [1, 2, 3, 4, 5]
    var g: List<Int> = [1, 2, 3, 4, 5]
    var h: List<Int> = [1, 2, 3, 4, 5]
    var k: List<Int> = [1, 2, 3, 4, 5]
    var m: List<Int> = [1, 2, 3, 4, 5]
    var r = 0
    while r < 1500 {
        var xs: List<Int> = []
        var q = 0
        while q < 100 {
            xs.push(q)
            q = q + 1
        }
        r = r + 1
    }
    return a.len() + b.len() + c.len() + d.len() + e.len() + f.len() + g.len() + h.len() + k.len() + m.len()
}
",
        per_task: 50,
    },
    Program {
        name: "retained data across churn",
        source: "
fn work() -> Int {
    var keep: List<String> = []
    var i = 0
    while i < 200 {
        keep.push(\"item\" + i.to_string())
        i = i + 1
    }
    var r = 0
    while r < 1500 {
        var xs: List<Int> = []
        var k = 0
        while k < 100 {
            xs.push(k)
            k = k + 1
        }
        r = r + 1
    }
    var total = 0
    for s in keep {
        total = total + s.len()
    }
    return total
}
",
        per_task: 10 * 5 + 90 * 6 + 100 * 7,
    },
];

fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(180))
        .expect("churn program hung or crashed")
}

fn run(program: &'static Program, gc: gc::GCConfig, workers: usize) -> (i64, usize) {
    with_timeout(move || {
        let source = format!("{}{}", program.source, MAIN);
        let compiled = compiler::Compiler::compile(&source, program.name)
            .unwrap_or_else(|e| panic!("{}: does not compile: {e:?}", program.name));
        let global_count = compiled.global_count as usize;
        let mut rt = Runtime::with_types(compiled.code_objects, compiled.type_descriptors);
        rt.set_global_count(global_count);
        ffi::builtins::install(&mut rt);
        rt.set_native_table(&compiled.native_table).unwrap();
        rt.set_heap(gc::build_gc_engine(&gc));
        let task = rt.run_entry_on(workers)
            .unwrap_or_else(|e| panic!("{}: run failed: {e}", program.name));
        let collections = rt.gc_stats().map_or(0, |s| s.collections);
        (task.registers[0].as_int().expect("main returns an Int"), collections)
    })
}

fn check(gc: fn() -> gc::GCConfig, must_collect: bool) {
    for program in PROGRAMS {
        for workers in [1, 4] {
            let (total, collections) = run(program, gc(), workers);
            assert_eq!(
                total,
                4 * program.per_task,
                "{} on {workers} worker(s)",
                program.name
            );
            if must_collect {
                assert!(collections > 0, "{}: expected collections to run", program.name);
            }
        }
    }
}

#[test]
fn test_churn_programs_under_the_collector() {
    check(|| gc::GCConfig::mark_sweep().with_threshold(64 * 1024), true);
}

#[test]
fn test_churn_programs_with_no_collector() {
    check(gc::GCConfig::nogc, false);
}
