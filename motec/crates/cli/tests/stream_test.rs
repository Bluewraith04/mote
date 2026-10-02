
use isa::value::TypeRegistry;
use modules::MultiFileCompiler;

fn compile(name: &str, source: &str) -> Result<runtime::Runtime, String> {
    let dir = std::env::temp_dir().join(format!("mote_stream_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(&main_file, source).unwrap();
    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file);
    std::fs::remove_dir_all(dir).ok();
    let compiled = compiled?;
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    Ok(rt)
}

fn run(name: &str, source: &str, workers: usize) -> Option<i64> {
    let mut rt = compile(name, source).unwrap();
    let task = rt.run_entry_on(workers).unwrap();
    task.registers.first().and_then(|v| v.as_int())
}

#[test]
fn test_a_generator_function_streams_into_for() {
    let source = "\
fn count(n: Int) -> Stream<Int> {
    var i = 0
    while i < n {
        yield i * i
        i = i + 1
    }
}

fn main() -> Int {
    var sum = 0
    for x in count(5) { sum = sum + x }
    return sum
}
return main()
";
    for workers in [1, 4] {
        assert_eq!(run("count", source, workers), Some(30), "workers={workers}");
    }
}

#[test]
fn test_next_pulls_one_item_at_a_time_and_ends_with_none() {
    let source = "\
fn two() -> Stream<Int> {
    yield 10
    yield 20
}

fn main() -> Int {
    let s = two()
    var total = 0
    let a = s.next()
    if a.is_some() { total = total + a.unwrap() }
    let b = s.next()
    if b.is_some() { total = total + b.unwrap() }
    let c = s.next()
    if c.is_none() { total = total + 1000 }
    return total
}
return main()
";
    assert_eq!(run("next", source, 1), Some(1030));
}

#[test]
fn test_streams_nest_and_break_stops_early() {
    let source = "\
fn evens(n: Int) -> Stream<Int> {
    for i in 0..n {
        if i % 2 == 0 { yield i }
    }
}

fn doubled(n: Int) -> Stream<Int> {
    for x in evens(n) { yield x * 2 }
}

fn main() -> Int {
    var sum = 0
    for x in doubled(100) {
        if x > 20 { break }
        sum = sum + x
    }
    return sum
}
return main()
";
    assert_eq!(run("nest", source, 1), Some(60));
}

#[test]
fn test_stream_views_over_a_list_and_a_channel() {
    let source = "\
fn total(s: Stream<Int>) -> Int {
    var sum = 0
    for x in s { sum = sum + x }
    return sum
}

fn main() -> Int {
    let xs = [1, 2, 3, 4]
    let (tx, rx) = Channel<Int>(4)
    tx.send(10)
    tx.send(20)
    tx.close()
    return total(xs.stream()) + total(rx.stream())
}
return main()
";
    assert_eq!(run("views", source, 1), Some(40));
}

#[test]
fn test_a_generator_can_wait_on_a_channel_between_yields() {
    let source = "\
fn relay(rx: Receiver<Int>) -> Stream<Int> {
    for x in rx { yield x + 1 }
}

fn main() -> Int {
    let (tx, rx) = Channel<Int>(2)
    var sum = 0
    scope {
        spawn {
            var i = 0
            while i < 50 {
                tx.send(i)
                i = i + 1
            }
            tx.close()
        }
        for y in relay(rx) { sum = sum + y }
    }
    return sum
}
return main()
";
    for workers in [1, 4] {
        assert_eq!(run("relay", source, workers), Some(1275), "workers={workers}");
    }
}

#[test]
fn test_yield_type_and_placement_errors() {
    let wrong = "fn f() -> Stream<Int> { yield \"no\" }\nreturn 0\n";
    assert!(compile("wrong", wrong).err().unwrap().contains("`yield` expects"));

    let undeclared = "fn f() -> Int { yield 1 }\nreturn 0\n";
    assert!(compile("undeclared", undeclared).err().unwrap().contains("E0704"));

    let in_lambda = "fn f() -> Stream<Int> {\n    let g = || { yield 1 }\n    yield 2\n}\nreturn 0\n";
    assert!(compile("in_lambda", in_lambda).err().unwrap().contains("E0704"));

    let in_spawn = "fn f() -> Stream<Int> {\n    scope { spawn { yield 1 } }\n    yield 2\n}\nreturn 0\n";
    assert!(compile("in_spawn", in_spawn).err().unwrap().contains("E0704"));

    let toplevel = "yield 1\nreturn 0\n";
    assert!(compile("toplevel", toplevel).err().unwrap().contains("E0704"));
}

#[test]
fn test_a_stream_is_never_send() {
    let source = "\
fn nums() -> Stream<Int> { yield 1 }

fn main() -> Int {
    let s = nums()
    scope { spawn { for x in s { println(x) } } }
    return 0
}
return main()
";
    assert!(compile("send", source).err().unwrap().contains("Sendable"));
}

#[test]
fn test_std_stream_chars_and_lines() {
    let source = "\
import { chars, lines } from std.stream

fn main() -> Int {
    var n = 0
    var wide = 0
    for c in chars(\"héllo €\") {
        n = n + 1
        if c.len() > 1 { wide = wide + 1 }
    }
    var count = 0
    var blank = 0
    for line in lines(\"a\\nb\\n\\nc\\n\") {
        count = count + 1
        if line.is_empty() { blank = blank + 1 }
    }
    return n * 1000 + wide * 100 + count * 10 + blank
}
return main()
";
    assert_eq!(run("std_stream", source, 1), Some(7 * 1000 + 2 * 100 + 4 * 10 + 1));
}

#[test]
fn test_std_stream_file_lines_yield_errors_as_items() {
    let dir = std::env::temp_dir().join(format!("mote_stream_file_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("data.txt");
    std::fs::write(&data, "one\ntwo\nthree\n").unwrap();
    let source = format!(
        "\
import {{ file_lines }} from std.stream

fn main() -> Int {{
    var lines = 0
    var errs = 0
    for item in file_lines(\"{}\") {{
        match item {{
            Ok(line) => {{ lines = lines + 1 }}
            Err(e) => {{ errs = errs + 1 }}
        }}
    }}
    for item in file_lines(\"{}/missing.txt\") {{
        match item {{
            Ok(line) => {{ lines = lines + 100 }}
            Err(e) => {{ errs = errs + 1 }}
        }}
    }}
    return lines * 10 + errs
}}
return main()
",
        data.display(),
        dir.display()
    );
    assert_eq!(run("file_lines", &source, 1), Some(31));
    std::fs::remove_dir_all(dir).ok();
}
