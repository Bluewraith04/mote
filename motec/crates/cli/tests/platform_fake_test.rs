
use std::sync::Arc;

use isa::value::TypeRegistry;
use modules::MultiFileCompiler;
use platform::FakePlatform;
use runtime::VmStatus;

struct Ran {
    status: VmStatus,
    result: Option<i64>,
    fake: Arc<FakePlatform>,
}

fn run(name: &str, source: &str, fake: FakePlatform, workers: usize) -> Ran {
    let dir = std::env::temp_dir().join(format!("mote_fake_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(&main_file, source).unwrap();

    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let fake = Arc::new(fake);
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_platform(fake.clone());
    let task = rt.run_entry_on(workers).unwrap();
    let status = rt.status();
    let result = task.registers.first().and_then(|v| v.as_int());
    std::fs::remove_dir_all(dir).ok();
    Ran { status, result, fake }
}

#[test]
fn test_program_runs_on_the_fake_platform_with_virtual_time_and_files() {
    let source = "\
import { Duration, now, sleep } from std.time
import std.sys.io as io

fn main() -> Int {
    let a = now()
    sleep(Duration.from_millis(5))
    sleep(Duration.from_millis(3))
    let b = now()
    println(\"slept\")
    match io.read_file(\"/virtual/in.txt\") {
        Ok(text) => { println(text) }
        Err(e) => { println(\"missing\") }
    }
    match io.read_file(\"/virtual/none.txt\") {
        Ok(text) => { println(text) }
        Err(e) => { println(\"missing\") }
    }
    return b.duration_since(a).as_nanos()
}
return main()
";
    let fake = FakePlatform::new(1).with_file("/virtual/in.txt", b"from the fake");
    let ran = run("time_files", source, fake, 1);
    assert_eq!(ran.result, Some(8_000_000));
    assert_eq!(ran.fake.stdout(), "slept\nfrom the fake\nmissing\n");
}

#[test]
fn test_exit_stops_the_run_and_reports_the_code() {
    let source = "\
import std.sys.env as env

fn main() {
    println(\"before\")
    env.exit(3)
    println(\"after\")
}
main()
";
    let ran = run("exit_main", source, FakePlatform::new(1), 1);
    assert_eq!(ran.status, VmStatus::Exited(3));
    assert_eq!(ran.fake.stdout(), "before\n");
}

#[test]
fn test_exit_from_a_task_stops_a_spinning_sibling_on_every_worker() {
    let source = "\
import std.sys.env as env

fn main() {
    scope {
        spawn {
            var i = 0
            while true { i = i + 1 }
        }
        spawn {
            println(\"exiting\")
            env.exit(4)
        }
    }
    println(\"unreachable\")
}
main()
";
    for workers in [1, 4] {
        let ran = run(&format!("exit_task_{workers}"), source, FakePlatform::new(1), workers);
        assert_eq!(ran.status, VmStatus::Exited(4), "workers={workers}");
        assert_eq!(ran.fake.stdout(), "exiting\n", "workers={workers}");
    }
}

#[test]
fn test_file_ops_round_trip_and_not_found() {
    let source = "\
import std.sys.io as io

fn kind_of(e: Error) -> ErrorKind {
    return e.kind
}

fn main() -> Int {
    let w = io.write_file(\"/virtual/out.txt\", \"hello file\")
    if w.is_err() { return 1 }

    let r = io.read_file(\"/virtual/out.txt\")
    match r {
        Ok(contents) => {
            if contents != \"hello file\" { return 2 }
        }
        Err(e) => { return 3 }
    }

    let wb = io.write_file_bytes(\"/virtual/out.txt\", \"bytes content\".bytes())
    if wb.is_err() { return 4 }
    let rb = io.read_file_bytes(\"/virtual/out.txt\")
    match rb {
        Ok(data) => {
            if data.len() != 13 { return 5 }
        }
        Err(e) => { return 6 }
    }

    let missing = io.read_file(\"/virtual/does_not_exist.txt\")
    match missing {
        Ok(v) => { return 7 }
        Err(e) => {
            match kind_of(e) {
                ErrorKind.NotFound => { return 42 }
                _ => { return 8 }
            }
        }
    }
}
return main()
";
    assert_eq!(run("files", source, FakePlatform::new(1), 1).result, Some(42));
}

#[test]
fn test_time_durations_fake_clock_and_a_virtual_sleep() {
    let source = "\
import { Duration, Clock, sleep, unix_millis } from std.time

fn main() -> Int {
    let d = Duration.from_millis(1500)
    if d.as_secs() != 1 { return 1 }
    if d.as_millis() != 1500 { return 2 }
    let sum = d.plus(Duration.from_secs(2))
    if sum.as_millis() != 3500 { return 3 }
    if d.compare(sum) != -1 { return 4 }
    if sum.minus(d).as_secs() != 2 { return 5 }

    var fake = Clock.fake(1000)
    let start = fake.now()
    fake.advance(Duration.from_millis(250))
    if fake.since(start).as_millis() != 250 { return 6 }
    if start.compare(fake.now()) != -1 { return 7 }

    let sys = Clock.system()
    let t0 = sys.now()
    sleep(Duration.from_millis(20))
    if sys.since(t0).as_millis() != 20 { return 8 }
    if unix_millis() < 1600000000000 { return 9 }
    return 42
}
return main()
";
    assert_eq!(run("time", source, FakePlatform::new(1), 1).result, Some(42));
}

#[test]
fn test_stdout_and_stderr_write_line_are_captured_per_stream() {
    let source = "\
import std.sys.io as io

fn main() -> Int {
    let out = io.stdout()
    out.write_line(\"line one\")
    out.write(\"no newline here\")
    out.flush()
    let err = io.stderr()
    err.write_line(\"an error line\")
    return 0
}
return main()
";
    let ran = run("stdout_stderr", source, FakePlatform::new(1), 1);
    assert_eq!(ran.fake.stdout(), "line one\nno newline here");
    assert_eq!(ran.fake.stderr(), "an error line\n");
}

const READ_LINE_PROGRAM: &str = "\
import std.sys.io as io

fn main() -> Int {
    let sin = io.stdin()
    let r = sin.read_line()
    match r {
        Ok(v) => {
            match v {
                Some(line) => {
                    io.stdout().write_line(\"got:\" + line)
                    return 0
                }
                None => {
                    io.stdout().write_line(\"eof\")
                    return 1
                }
            }
        }
        Err(e) => { return 2 }
    }
}
return main()
";

#[test]
fn test_stdin_read_line_returns_a_scripted_line() {
    let fake = FakePlatform::new(1).with_stdin_line("hello there").with_stdin_line("more");
    let ran = run("stdin_line", READ_LINE_PROGRAM, fake, 1);
    assert_eq!(ran.result, Some(0));
    assert_eq!(ran.fake.stdout(), "got:hello there\n");
}

#[test]
fn test_stdin_read_line_at_end_of_input_is_none() {
    let ran = run("stdin_eof", READ_LINE_PROGRAM, FakePlatform::new(1), 1);
    assert_eq!(ran.result, Some(1));
    assert_eq!(ran.fake.stdout(), "eof\n");
}

const ENTROPY_PROGRAM: &str = "\
import { Rng } from std.random

fn main() {
    var rng = Rng.from_entropy()
    println(\"${rng.next_int(0, 1000000000)}\")
}
main()
";

#[test]
fn test_entropy_seeded_rng_is_reproducible_under_the_fake() {
    let draw = |seed: u64, tag: &str| run(tag, ENTROPY_PROGRAM, FakePlatform::new(seed), 1).fake.stdout();
    assert_eq!(draw(5, "entropy_a"), draw(5, "entropy_b"));
    assert_ne!(draw(5, "entropy_c"), draw(6, "entropy_d"));
}

#[test]
fn test_stdin_lines_stream_reads_scripted_input_until_the_end() {
    let source = "\
import { stdin_lines } from std.stream

fn main() -> Int {
    var count = 0
    for item in stdin_lines() {
        match item {
            Ok(line) => {
                println(\"got:\" + line)
                count = count + 1
            }
            Err(e) => { return 99 }
        }
    }
    return count
}
return main()
";
    let fake = FakePlatform::new(1).with_stdin_line("alpha").with_stdin_line("beta");
    let ran = run("stdin_lines", source, fake, 1);
    assert_eq!(ran.result, Some(2));
    assert_eq!(ran.fake.stdout(), "got:alpha\ngot:beta\n");
}

fn run_with_clock(name: &str, source: &str, workers: usize) -> Ran {
    let dir = std::env::temp_dir().join(format!("mote_fake_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    std::fs::write(&main_file, source).unwrap();

    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let fake = Arc::new(FakePlatform::new(1));
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_platform(fake.clone());

    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let clock = {
        let (fake, done) = (fake.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                fake.advance(1_000_000);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
    };
    let task = rt.run_entry_on(workers).unwrap();
    let status = rt.status();
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    clock.join().unwrap();
    let result = task.registers.first().and_then(|v| v.as_int());
    std::fs::remove_dir_all(dir).ok();
    Ran { status, result, fake }
}

#[test]
fn test_a_ticker_delivers_ticks_on_the_virtual_clock() {
    let source = "\
import { Duration, ticker } from std.time

fn main() -> Int {
    let t = ticker(Duration.from_millis(1))
    var sum = 0
    var seen = 0
    for n in t {
        sum = sum + n
        seen = seen + 1
        if seen == 4 { break }
    }
    return sum
}
return main()
";
    for workers in [1, 2] {
        let ran = run_with_clock(&format!("ticker{workers}"), source, workers);
        assert_eq!(ran.result, Some(1 + 2 + 3 + 4));
        assert!(ran.fake.source_closed(0), "the timer must stop when the program ends");
    }
}

#[test]
fn test_a_ticker_task_and_a_sleeping_task_share_the_run() {
    let source = "\
import { Duration, ticker, sleep } from std.time

fn main() -> Int {
    let t = ticker(Duration.from_millis(2))
    var total = 0
    scope {
        let waiter = spawn {
            sleep(Duration.from_millis(5))
            return 100
        }
        let first = t.recv()
        total = first.unwrap() + waiter.join().unwrap()
    }
    return total
}
return main()
";
    let ran = run_with_clock("ticker_mix", source, 2);
    assert_eq!(ran.result, Some(101));
}

#[test]
fn test_program_runs_scripted_processes_and_the_fake_logs_them() {
    let source = "import std.sys.process as process\nimport { Output } from std.sys.process\n\nfn main() {\n    let o: Output = process.run(\"git\", [\"status\"]).unwrap()\n    println(o.status)\n    println(process.run(\"hg\", []).is_err())\n}\n";
    let fake = FakePlatform::new(1).with_process("git", 0, b"clean\n", b"");
    let ran = run("process", source, fake, 1);
    assert_eq!(ran.fake.stdout(), "0\ntrue\n");
    let programs: Vec<_> = ran
        .fake
        .process_log()
        .into_iter()
        .map(|r| match r {
            contracts::PlatformRequest::RunProcess { program, args, .. } => (program, args),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(programs, [("git".to_string(), vec!["status".to_string()]), ("hg".to_string(), vec![])]);
}
