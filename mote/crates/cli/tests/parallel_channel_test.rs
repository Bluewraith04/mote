
use runtime::Runtime;

const PROGRAM: &str = "\
fn main() -> Int {
    let (tx, rx) = Channel<Int>(8)
    let rx_b = rx.clone()
    let rx_c = rx.clone()
    var total = 0
    scope {
        let a = spawn {
            var s = 0
            var run = true
            while run {
                let v = rx.recv()
                if v.is_some() { s = s + v.unwrap() } else { run = false }
            }
            return s
        }
        let b = spawn {
            var s = 0
            var run = true
            while run {
                let v = rx_b.recv()
                if v.is_some() { s = s + v.unwrap() } else { run = false }
            }
            return s
        }
        let c = spawn {
            var s = 0
            var run = true
            while run {
                let v = rx_c.recv()
                if v.is_some() { s = s + v.unwrap() } else { run = false }
            }
            return s
        }
        var i = 1
        while i <= 3000 {
            tx.send(i)
            i = i + 1
        }
        tx.close()
        total = a.join().unwrap() + b.join().unwrap() + c.join().unwrap()
    }
    return total
}
";

fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(60))
        .expect("parallel run hung (lost wakeup?)")
}

fn run_parallel(workers: usize) -> i64 {
    run_program(PROGRAM, workers)
}

fn run_program(source: &'static str, workers: usize) -> i64 {
    with_timeout(move || {
        let compiled = compiler::Compiler::compile(source, "<parallel-channel>").expect("compiles");
        let global_count = compiled.global_count as usize;
        let mut rt = Runtime::with_types(compiled.code_objects, compiled.type_descriptors);
        rt.set_global_count(global_count);
        ffi::builtins::install(&mut rt);
        rt.set_native_table(&compiled.native_table).unwrap();
        rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
        let main = runtime::TaskContext::entry(&rt);
        let finished = rt.run_main_parallel(main, workers).expect("run succeeds");
        finished.registers[0].as_int().expect("main returns an Int")
    })
}

#[test]
fn test_parallel_receivers_each_get_distinct_items() {
    let expected = 3000 * 3001 / 2;
    for round in 0..15 {
        assert_eq!(run_parallel(4), expected, "round {round}");
    }
}

#[test]
fn test_env_args_is_visible_on_every_worker() {
    use isa::value::TypeRegistry;
    use modules::MultiFileCompiler;

    ffi::builtins::set_script_args(vec!["prog".to_string(), "x".to_string()]);
    let dir = std::env::temp_dir().join(format!("mote_par_args_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(
        &main_file,
        "\
import std.sys.env as env

fn main() -> Int {
    var total = 0
    scope {
        let a = spawn { return env.args().len() }
        let b = spawn { return env.args().len() }
        let c = spawn { return env.args().len() }
        total = env.args().len() + a.join().unwrap() + b.join().unwrap() + c.join().unwrap()
    }
    return total
}
return main()",
    )
    .unwrap();

    let mut compiler = MultiFileCompiler::new(dir.clone());
    let compiled = compiler.compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = Runtime::with_type_registry(compiled.code_objects, &registry);
    rt.set_global_count(compiled.global_count as usize);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::default())));
    let main = runtime::TaskContext::entry(&rt);
    let finished = rt.run_main_parallel(main, 4).expect("run succeeds");
    assert_eq!(finished.registers[0].as_int(), Some(8));

    std::fs::remove_dir_all(dir).ok();
}

const CANCEL_PROGRAM: &str = "\
fn one_round() -> Int {
    var out = 0
    scope {
        let (tx, rx) = Channel<Int>(1)
        let helper = spawn { return 1 }
        let stuck = spawn {
            rx.recv()
            return 0
        }
        helper.join()
        stuck.cancel()
        let r = stuck.join()
        if r.is_err() { out = 1 }
    }
    return out
}

fn main() -> Int {
    var total = 0
    var i = 0
    while i < 200 {
        total = total + one_round()
        i = i + 1
    }
    return total
}
";

#[test]
fn test_cancelling_a_task_racing_to_park_on_a_channel_never_loses_the_wake() {
    assert_eq!(run_program(CANCEL_PROGRAM, 4), 200);
}

const JOIN_PROGRAM: &str = "\
fn main() -> Int {
    var total = 0
    var i = 0
    while i < 3000 {
        scope {
            let a = spawn { return 1 }
            let b = spawn { return 2 }
            total = total + a.join().unwrap() + b.join().unwrap()
        }
        i = i + 1
    }
    return total
}
";

#[test]
fn test_joining_a_task_that_finishes_on_another_worker_never_loses_the_wake() {
    assert_eq!(run_program(JOIN_PROGRAM, 4), 3000 * 3);
}
