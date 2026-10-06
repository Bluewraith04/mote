
use isa::value::TypeRegistry;
use modules::MultiFileCompiler;

fn run(name: &str, source: &str, workers: usize) -> Option<i64> {
    let dir = std::env::temp_dir().join(format!("mote_chan_iter_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(&main_file, source).unwrap();

    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    let task = rt.run_entry_on(workers).unwrap();
    let result = task.registers.first().and_then(|v| v.as_int());
    std::fs::remove_dir_all(dir).ok();
    result
}

const DRAIN: &str = "\
fn main() -> Int {
    let (tx, rx) = Channel<Int>(4)
    var sum = 0
    scope {
        spawn {
            var i = 1
            while i <= 100 {
                tx.send(i)
                i = i + 1
            }
            tx.close()
        }
        for x in rx { sum = sum + x }
    }
    return sum
}
return main()
";

#[test]
fn test_for_in_drains_a_channel_until_it_is_closed() {
    for workers in [1, 4] {
        assert_eq!(run("drain", DRAIN, workers), Some(5050), "workers={workers}");
    }
}

#[test]
fn test_for_in_channel_supports_break_and_continue() {
    let source = "\
fn main() -> Int {
    let (tx, rx) = Channel<Int>(8)
    var i = 1
    while i <= 6 {
        tx.send(i)
        i = i + 1
    }
    var sum = 0
    for x in rx {
        if x == 2 { continue }
        if x == 5 { break }
        sum = sum + x
    }
    return sum
}
return main()
";
    assert_eq!(run("break_continue", source, 1), Some(8));
}

#[test]
fn test_for_in_still_iterates_lists_and_ranges_beside_channels() {
    let source = "\
fn main() -> Int {
    var sum = 0
    let xs = [10, 20, 30]
    for x in xs { sum = sum + x }
    for i in 0..4 { sum = sum + i }
    let (tx, rx) = Channel<Int>(2)
    tx.send(1)
    tx.send(2)
    tx.close()
    for x in rx { sum = sum + x }
    return sum
}
return main()
";
    assert_eq!(run("mixed", source, 1), Some(69));
}
