
use runtime::Runtime;

const PROGRAM: &str = "\
fn churn(n: Int) -> Int {
    var total = 0
    var round = 0
    while round < n {
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

fn main() -> Int {
    let a = spawn { churn(600) }
    let b = spawn { churn(600) }
    let c = spawn { churn(600) }
    return a.join().unwrap() + b.join().unwrap() + c.join().unwrap() + churn(600)
}
";

fn run(workers: usize) -> (i64, usize) {
    let compiled = compiler::Compiler::compile(PROGRAM, "<gc-roots>").expect("compiles");
    let global_count = compiled.global_count as usize;
    let mut rt = Runtime::with_types(compiled.code_objects, compiled.type_descriptors);
    rt.set_global_count(global_count);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::mark_sweep().with_threshold(64 * 1024))));
    let task = rt.run_entry_on(workers).expect("run succeeds");
    let collections = rt.gc_stats().map_or(0, |s| s.collections);
    (task.registers[0].as_int().expect("main returns an Int"), collections)
}

#[test]
fn test_collection_keeps_live_objects_in_a_wide_running_frame() {
    for workers in [1, 4] {
        let (total, collections) = run(workers);
        assert_eq!(total, 4 * 600 * 200, "workers={workers}");
        assert!(collections > 0, "workers={workers}: the program was meant to collect");
    }
}
