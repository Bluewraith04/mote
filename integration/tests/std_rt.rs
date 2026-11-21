//! The parts of `std` that reach the runtime: the prelude, `std.io` and `std.time`, run as a program would use them.

use std::path::{Path, PathBuf};

use mote::{execute_at, Outcome};
use mote_packages::testing::Scratch;

fn run(line: &str, dir: &Path, home: &Path) -> Outcome {
    let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    execute_at(&words, dir, home)
}

/// A package `app` whose `src/main.mote` is the source.
fn app(scratch: &Scratch, source: &str) -> PathBuf {
    scratch.package("app", "app", &[], &[("main.mote", source)])
}

/// What the program prints, with the exit code 0 asserted.
fn printed(source: &str) -> String {
    let scratch = Scratch::new();
    let dir = app(&scratch, source);
    let outcome = run("run", &dir, &scratch.path("home"));
    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""), "{}", outcome.stdout);
    outcome.stdout
}

#[test]
fn std_rt_prelude_names_are_in_scope_with_no_import() {
    let source = r#"
fn pick(a: Int64, b: UInt8, c: Float32, d: Bool, e: Float) -> Option<String> {
    return Some("${a} ${b} ${c} ${d} ${e}")
}

fn halve(n: Int) -> Result<Int, Error> {
    if n % 2 != 0 {
        return Err(Error.new(ErrorKind.InvalidData, "odd"))
    }
    return Ok(n / 2)
}

fn main() {
    var names: List<String> = []
    names.push(pick(1, 2, 3.5, true, 0.5)!)
    var by: Map<String, Int> = Map.new()
    by.set("a", 1)
    println(names.get(0))
    println(by.len())
    println(halve(4)!)
    println(halve(3).is_err())
    println(typeof(names))
}
"#;
    assert_eq!(printed(source), "1 2 3.5 true 0.5\n1\n2\ntrue\nList<String>\n");
}

#[test]
fn std_rt_prelude_arrays_and_lists_convert_with_no_import() {
    let source = "fn main() {\n    let xs = [5, 6, 7]\n    let array = xs.to_array()\n    println(array.len())\n    println(array.to_list().get(2))\n}\n";
    assert_eq!(printed(source), "3\n7\n");
}

#[test]
fn std_rt_prelude_the_byte_buffer_needs_an_import() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let dir = app(&scratch, "fn main() {\n    var b = Bytes.new()\n    b.push(65)\n    println(b.to_string()!)\n}\n");
    let outcome = run("check", &dir, &home);
    assert_eq!(outcome.code, 1);
    let dir = app(&scratch, "import { Bytes } from std.bytes\n\nfn main() {\n    var b = Bytes.new()\n    b.push(65)\n    println(b.to_string()!)\n}\n");
    assert_eq!(run("run", &dir, &home), Outcome::ok("A\n"));
}

/// The built executable of the source, run with the arguments and the standard input.
fn exe_run(source: &str, args: &[&str], stdin: &str) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let scratch = Scratch::new();
    let dir = app(&scratch, source);
    let built = run("build", &dir, &scratch.path("home"));
    assert_eq!(built.code, 0, "{}{}", built.stdout, built.stderr);
    let mut child = Command::new(dir.join("target/app")).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn std_rt_io_eprint_writes_to_standard_error() {
    let source = "fn main() {\n    print(\"a\")\n    eprint(\"b\")\n    eprintln(\"c\")\n    println(\"d\")\n}\n";
    let out = exe_run(source, &[], "");
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).as_ref(), String::from_utf8_lossy(&out.stderr).as_ref()), (Some(0), "ad\n", "bc\n"));
}

#[test]
fn std_rt_io_read_line_reads_lines_then_null_at_the_end() {
    let source = "import { read_line } from std.io\n\nfn main() {\n    while true {\n        let line = read_line()\n        if line == null {\n            break\n        }\n        println(\"got ${line}\")\n    }\n    println(\"end\")\n}\n";
    let out = exe_run(source, &[], "one\ntwo\r\nlast");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "got one\ngot two\ngot last\nend\n");
}

#[test]
fn std_rt_io_read_line_in_run_sees_the_end_of_input() {
    let source = "import { read_line } from std.io\n\nfn main() {\n    println(read_line() == null)\n}\n";
    assert_eq!(printed(source), "true\n");
}

#[test]
fn std_rt_io_args_are_the_words_after_the_double_dash() {
    let source = "import { args } from std.io\n\nfn main() {\n    let all = args()\n    println(all.len())\n    for a in all {\n        println(a)\n    }\n}\n";
    let out = exe_run(source, &["--", "x", "-y", "z z"], "");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "3\nx\n-y\nz z\n");
    let none = exe_run(source, &[], "");
    assert_eq!(String::from_utf8_lossy(&none.stdout), "0\n");
}

#[test]
fn std_rt_io_args_reach_a_program_started_by_run() {
    let scratch = Scratch::new();
    let dir = app(&scratch, "import { args } from std.io\n\nfn main() {\n    println(args().len())\n}\n");
    assert_eq!(run("run -- a b", &dir, &scratch.path("home")), Outcome::ok("2\n"));
}

#[test]
fn std_rt_io_exit_ends_the_program_with_its_code() {
    let source = "import { exit } from std.io\n\nfn main() {\n    println(\"before\")\n    exit(3)\n    println(\"after\")\n}\n";
    let out = exe_run(source, &[], "");
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).as_ref()), (Some(3), "before\n"));
    let scratch = Scratch::new();
    let dir = app(&scratch, source);
    let outcome = run("run", &dir, &scratch.path("home"));
    assert_eq!((outcome.code, outcome.stdout.as_str()), (3, "before\n"));
}

#[test]
fn std_rt_io_main_that_returns_an_int_exits_with_it() {
    let source = "fn main() -> Int {\n    println(\"x\")\n    return 4\n}\n";
    let out = exe_run(source, &[], "");
    assert_eq!(out.status.code(), Some(4));
    let scratch = Scratch::new();
    let dir = app(&scratch, source);
    assert_eq!(run("run", &dir, &scratch.path("home")).code, 4);
}

const TIMED: &str = "import { Duration, Instant, sleep } from std.time\n\nfn main() {\n    let start = Instant.now()\n    sleep(Duration.from_millis(250))\n    println(start.elapsed())\n}\n";

#[test]
fn std_rt_time_the_clock_is_virtual_under_a_seed_and_a_sleep_is_exact() {
    let out = exe_run(TIMED, &["--seed", "7"], "");
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).as_ref()), (Some(0), "250ms\n"));
}

#[test]
fn std_rt_time_a_sleep_takes_at_least_its_duration_on_the_real_clock() {
    let source = "import { Duration, Instant, sleep } from std.time\n\nfn main() {\n    let start = Instant.now()\n    sleep(Duration.from_millis(60))\n    println(start.elapsed().as_millis() >= 60)\n}\n";
    assert_eq!(printed(source), "true\n");
}

#[test]
fn std_rt_time_a_sleep_in_a_task_and_the_main_task_share_one_clock() {
    let source = "import { Duration, Instant, sleep } from std.time\n\nfn main() {\n    let start = Instant.now()\n    let task = spawn {\n        sleep(Duration.from_millis(100))\n        return start.elapsed().as_millis()\n    }\n    sleep(Duration.from_millis(300))\n    println(task.await()!)\n    println(start.elapsed())\n}\n";
    let out = exe_run(source, &["--seed", "3"], "");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "100\n300ms\n");
}
