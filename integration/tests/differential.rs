//! One program runs compiled, with and without the optimizer, and what it shows is compared with the file recorded beside it.

mod common;

use std::path::Path;

use common::expect::{block_body, check, main_text, output_lines, output_of, sections, tests_text};
use mote::{run_main_fed, run_main_with, run_tests, Ended, MainRun, Options, Pipeline, Settings, Target, Verdict};
use mote_packages::testing::Scratch;

use common::{corpus, feeds_of};

/// The source built as the package `corpus`.
fn built(source: &str, optimize: bool) -> Result<Pipeline, String> {
    let scratch = Scratch::new();
    let dir = scratch.package("corpus", "corpus", &[], &[("main.mote", source)]);
    Pipeline::build(&dir, Target::Run, &Options { optimize, ..Options::default() }).map_err(|failure| failure.to_string())
}

/// The source run compiled, with the pushes it asks for.
fn run_on(source: &str, optimize: bool) -> Result<MainRun, String> {
    run_main_fed(&mut built(source, optimize)?, &feeds_of(source)).map_err(|failure| failure.to_string())
}

/// The files of the directory with the extension `recorded` that have no file of the extension `kept` beside them.
fn orphans(dir: &Path, kept: &str, recorded: &str) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(dir).expect("a readable directory").map(|entry| entry.expect("a directory entry").path()).collect();
    files.sort();
    files.iter().filter(|path| path.extension().is_some_and(|extension| extension == recorded) && !path.with_extension(kept).exists()).map(|path| format!("{} has no .{kept} file beside it; delete it", common::relative(path))).collect()
}

/// The problems of the programs of the group run compiled, unoptimized and optimized, against the files beside them.
fn recorded(group: &str) -> Vec<String> {
    let dir = common::root().join("integration/corpus").join(group);
    let mut problems = Vec::new();
    for optimize in [false, true] {
        for (name, source) in corpus(group) {
            match run_on(&source, optimize) {
                Ok(run) => problems.extend(check(&dir.join(format!("{name}.expect")), &main_text(&run), !optimize)),
                Err(error) => problems.push(format!("{group}/{name}: {error}")),
            }
        }
    }
    problems.extend(orphans(&dir, "mote", "expect"));
    problems
}

fn run(output: &str, ended: Ended) -> MainRun {
    MainRun { output: output.into(), ended }
}

#[test]
fn differential_a_recorded_result_gives_the_ending_and_the_output_lines() {
    let fault = Ended::Fault { text: "fault: division by zero\n  at a.mote:1:1\n".into(), kind: "DivideByZero".into() };
    assert_eq!(main_text(&run("x\n", fault)), "ended Fault DivideByZero\n| x\n");
    assert_eq!(main_text(&run("", Ended::Err("boom".into()))), "ended Err \"boom\"\n");
    assert_eq!(main_text(&run("a\n\n b", Ended::Returned)), "ended Returned\n| a\n|\n|  b\n\\ no newline at end\n");
}

#[test]
fn differential_the_output_lines_of_a_recorded_result_give_back_the_output() {
    for output in ["", "a\n", "a", "\n", "a\n\n b\n", "1\n2", "| x\n", "\\ y\n"] {
        assert_eq!(output_of(&output_lines(output)), output);
    }
}

#[test]
fn differential_the_sections_of_a_tests_file_are_the_start_and_each_block() {
    let text = "start\n| s\ntest \"a\"\npassed\n| 1\ntest \"b\"\nfailed \"x\"\n";
    let found: Vec<(String, String)> = sections(text);
    assert_eq!(found, [("start".to_string(), "| s\n".to_string()), ("test \"a\"".to_string(), "passed\n| 1\n".to_string()), ("test \"b\"".to_string(), "failed \"x\"\n".to_string())]);
    assert_eq!(block_body(&Verdict::Skipped, ""), "skipped\n");
}

#[test]
fn differential_a_recorded_file_changes_only_when_recording_and_a_difference_is_shown() {
    let scratch = Scratch::new();
    let path = scratch.write("a.expect", "ended Returned\n| 1\n| 2\n");
    assert_eq!(check(&path, "ended Returned\n| 1\n| 2\n", false), None);
    let problem = check(&path, "ended Returned\n| 1\n| 3\n", false).expect("a difference");
    assert!(problem.ends_with("(MOTE_BLESS=1 records it):\n- | 2\n+ | 3"), "{problem}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ended Returned\n| 1\n| 2\n");
    if !common::expect::recording() {
        assert!(check(&path, "ended Returned\n", true).is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ended Returned\n| 1\n| 2\n");
    }
}

#[test]
fn differential_scalar_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("scalar"), Vec::<String>::new());
}

#[test]
fn differential_values_a_type_test_on_an_optional_union() {
    let source = corpus("values").into_iter().find(|(name, _)| name == "optional_union").unwrap().1;
    let compiled = run_on(&source, true).unwrap();
    assert_eq!(compiled.output, "null\nint 7\nstring seven\nnull\nint 3\nis int\nfalse\nfalse\nfalse\ntrue\nwide string\nsome\n");
    assert_eq!(compiled.ended, Ended::Returned);
}

#[test]
fn differential_values_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("values"), Vec::<String>::new());
}

#[test]
fn differential_objects_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("objects"), Vec::<String>::new());
}

#[test]
fn differential_sequences_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("sequences"), Vec::<String>::new());
}

#[test]
fn differential_functions_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("functions"), Vec::<String>::new());
}

#[test]
fn differential_numeric_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("numeric"), Vec::<String>::new());
}

#[test]
fn differential_strings_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("strings"), Vec::<String>::new());
}

#[test]
fn differential_collections_a_map_inside() {
    let source = corpus("collections").into_iter().find(|(name, _)| name == "map_inside").unwrap().1;
    let compiled = run_on(&source, true).unwrap();
    assert_eq!(compiled.output, "true\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\n1\nfalse\ntrue\nfalse\n1\n2\n");
    assert_eq!(compiled.ended, Ended::Returned);
}

#[test]
fn differential_collections_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("collections"), Vec::<String>::new());
}

const BLOCKS: &str = "var seen = 1\nseen += 1\nprintln(\"up\")\n\nfn halve(n: Int) -> Int {\n    return 10 / n\n}\n\ntest \"adds\" {\n    println(seen + 1)\n}\n\ntest \"divides by zero\" {\n    var zero = 0\n    zero += 0\n    println(halve(5))\n    println(halve(zero))\n}\n\n@ignore\ntest \"skipped\" {\n    println(halve(0))\n}\n\ntest \"closes over\" {\n    let add = |n: Int| n + seen\n    println(add(40))\n}\n";

#[test]
fn differential_functions_a_test_block_compiled_gives_its_verdict_and_output() {
    let scratch = Scratch::new();
    let dir = scratch.package("corpus", "corpus", &[], &[("main.mote", BLOCKS)]);
    let mut pipeline = Pipeline::build(&dir, Target::Test, &Options::default()).unwrap_or_else(|failure| panic!("{failure}"));
    let run = run_tests(&mut pipeline).unwrap_or_else(|failure| panic!("{failure}"));
    let want = "start\n| up\ntest \"adds\"\npassed\n| 3\ntest \"divides by zero\"\nfailed \"division by zero\"\n| 2\ntest \"skipped\"\nskipped\ntest \"closes over\"\npassed\n| 42\n";
    assert_eq!(tests_text(&run), want);
}

#[test]
fn differential_meta_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("meta"), Vec::<String>::new());
}

#[test]
fn differential_tasks_a_task_widened() {
    let source = corpus("tasks").into_iter().find(|(name, _)| name == "widened").unwrap().1;
    let compiled = run_on(&source, true).unwrap();
    assert_eq!(compiled.output, "7\n7\nseven\n7\n7\ntrue\ntrue\nfalse\n1\ntrue\n7\n");
    assert_eq!(compiled.ended, Ended::Returned);
}

#[test]
fn differential_tasks_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("tasks"), Vec::<String>::new());
}

#[test]
fn differential_channels_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("channels"), Vec::<String>::new());
}

#[test]
fn differential_sync_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("sync"), Vec::<String>::new());
}

#[test]
fn differential_timed_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("timed"), Vec::<String>::new());
}

#[test]
fn differential_ports_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("ports"), Vec::<String>::new());
}

#[test]
fn differential_ports_a_fed_program_prints_what_the_port_delivered() {
    let queue = run_on(&corpus("ports").into_iter().find(|(name, _)| name == "queue").unwrap().1, true).unwrap();
    assert_eq!(queue.output, "1\n4\n6\n3\n6\ntrue\n");
    let full = run_on(&corpus("ports").into_iter().find(|(name, _)| name == "full").unwrap().1, true).unwrap();
    assert_eq!(full.output, "1\n2\n");
}

#[test]
fn differential_cancel_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("cancel"), Vec::<String>::new());
}

#[test]
fn differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none() {
    let output = |name: &str| run_on(&corpus("cancel").into_iter().find(|(n, _)| n == name).unwrap().1, true).unwrap().output;
    assert_eq!(output("close"), "closed 1\ntrue\nend\n");
    assert_eq!(output("innermost"), "closed 3\nclosed 2\nclosed 1\ntrue\nend\n");
    assert_eq!(output("finished"), "closed 1\ntrue\nend\n");
    assert_eq!(output("fault"), "true\nend\n");
    assert_eq!(output("receiving"), "closed 7\ntrue\n");
}

#[test]
fn differential_cancel_a_cancelled_task_cancels_and_waits_for_the_tasks_of_its_scope() {
    let output = |name: &str| run_on(&corpus("cancel").into_iter().find(|(n, _)| n == name).unwrap().1, true).unwrap().output;
    assert_eq!(output("scope"), "closed child\nclosed outer\ntrue\nend\n");
}

#[test]
fn differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result() {
    let output = |name: &str| run_on(&corpus("cancel").into_iter().find(|(n, _)| n == name).unwrap().1, true).unwrap().output;
    assert_eq!(output("scope_error"), "closed 1\ndivision by zero\n");
    assert_eq!(output("scope_inner"), "slow done\ndivision by zero\n");
    assert_eq!(output("scope_ok"), "21\ndivision by zero\n");
}

#[test]
fn differential_cancel_the_error_of_a_scope_has_the_message_of_the_fault() {
    let source = corpus("cancel").into_iter().find(|(n, _)| n == "scope_message").unwrap().1;
    let expect = "division by zero\nindex out of range\ncustom message\nunwrapped a missing value\n";
    assert_eq!(run_on(&source, false).unwrap().output, expect);
    assert_eq!(run_on(&source, true).unwrap().output, expect);
}

#[test]
fn differential_deadlock_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("deadlock"), Vec::<String>::new());
}

#[test]
fn differential_streams_the_compiled_code_shows_the_recorded_results() {
    assert_eq!(recorded("streams"), Vec::<String>::new());
}

#[test]
fn differential_streams_compiled_generators_yield_in_order_widen_and_catch_a_read_from_inside() {
    let run = |name: &str| run_on(&corpus("streams").into_iter().find(|(n, _)| n == name).unwrap().1, true).unwrap();
    assert_eq!(run("values").output, "170\nitem 0\nitem 1\nitem 2\n0 p0\n1 p1\n2 p2\n4.0\n4\n");
    assert_eq!(run("captured").output, "10\n15\n20\n25\nn1\nn2\nn3\n1\n100\n2\n200\n");
    assert_eq!(run("adapted").output, "0\n1\n2\n0\n1\na\nb\n7\nseven\n");
    assert_eq!(run("nested").output, "[0, 4, 16, 36, 64]\n[]\n[0]\n[0, 1]\n[0, 1, 2]\n0\n1\n2\n[1, 2]\n0\nnull\nnull\n");
    assert_eq!(run("tasks").output, "45\n0\n1\n3\n10\n");
    let reentered = run("reentered");
    assert_eq!(reentered.output, "1\n");
    assert!(matches!(&reentered.ended, Ended::Fault { text, .. } if text.starts_with("fault: a stream was read while it runs")), "{:?}", reentered.ended);
}

#[test]
fn differential_channels_a_channel_of_optional_messages() {
    let scratch = Scratch::new();
    let source = "import { Channel } from std.sync\n\nfn main() {\n    let (tx, rx) = Channel<Int?>(1)\n    tx.send(1)\n    println(rx.recv() == null)\n}\n";
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    let failure = Pipeline::build(&dir, Target::Run, &Options::default()).err().expect("the checker refuses it").to_string();
    assert!(failure.contains("a channel cannot carry optional messages"), "{failure}");
}

/// The program as the package `app` with the library `demo` built from `integration/natives/demo.c`, run compiled.
fn native_run(source: &str) -> Result<MainRun, String> {
    native_run_with(source, &Settings::default())
}

/// Like `native_run`, on the settings.
fn native_run_with(source: &str, settings: &Settings) -> Result<MainRun, String> {
    let scratch = Scratch::new();
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    let natives = dir.join("native").join(mote_packages::host_triple());
    std::fs::create_dir_all(&natives).expect("a directory for the library");
    let built = std::process::Command::new("cc").args(["-shared", "-fPIC", "-O1", "-o"]).arg(natives.join("libdemo.so")).arg(common::root().join("integration/natives/demo.c")).arg("-lpthread").status().expect("cc runs");
    assert!(built.success(), "cc builds the library");
    let mut pipeline = Pipeline::build(&dir, Target::Run, &Options::default()).map_err(|failure| failure.to_string())?;
    run_main_with(&mut pipeline, settings).map(|finished| finished.run).map_err(|failure| failure.to_string())
}

/// The output of a program that returns.
fn native_output(source: &str) -> String {
    let run = native_run(source).unwrap_or_else(|failure| panic!("{failure}"));
    assert_eq!(run.ended, Ended::Returned, "{}", run.output);
    run.output
}

const NUMBERS: &str = r#"
@link("demo")
native fn add_i64(a: Int, b: Int) -> Int
@link("demo")
native fn add_i32(a: Int32, b: Int32) -> Int32
@link("demo")
native fn neg_i16(a: Int16) -> Int16
@link("demo")
native fn neg_i8(a: Int8) -> Int8
@link("demo")
native fn inc_u8(a: UInt8) -> UInt8
@link("demo")
native fn inc_u16(a: UInt16) -> UInt16
@link("demo")
native fn inc_u32(a: UInt32) -> UInt32
@link("demo")
native fn half_u64(a: UInt64) -> UInt64
@link("demo")
native fn widen_i8(a: Int8) -> Int
@link("demo")
native fn widen_u8(a: UInt8) -> Int
@link("demo")
native fn widen_i16(a: Int16) -> Int
@link("demo")
native fn widen_u16(a: UInt16) -> Int
@link("demo")
native fn widen_i32(a: Int32) -> Int
@link("demo")
native fn widen_u32(a: UInt32) -> Int
@link("demo")
native fn squares(a: Float, b: Float) -> Float
@link("demo")
native fn halve(a: Float32) -> Float32
@link("demo")
native fn is_even(n: Int) -> Bool

fn main() {
    println(add_i64(40, 2))
    println(add_i32(-5, 3))
    println(neg_i16(-300))
    println(neg_i8(-100))
    println(inc_u8(255))
    println(inc_u16(65535))
    println(inc_u32(4294967295))
    println(half_u64(18446744073709551615))
    println(widen_i8(-1))
    println(widen_u8(255))
    println(widen_i16(-2))
    println(widen_u16(65535))
    println(widen_i32(-3))
    println(widen_u32(4294967295))
    println(squares(3.0, 4.0))
    let five: Float32 = 5.0
    println(halve(five))
    println(is_even(10))
    println(is_even(7))
}
"#;

#[test]
fn differential_natives_numbers_cross_with_their_width_and_sign() {
    let want = "42\n-2\n300\n100\n0\n0\n0\n9223372036854775807\n-1\n255\n-2\n65535\n-3\n4294967295\n25.0\n2.5\ntrue\nfalse\n";
    assert_eq!(native_output(NUMBERS), want);
}

#[test]
fn differential_natives_numbers_cross_the_same_on_a_helper_thread() {
    let blocking = NUMBERS.replace("@link(\"demo\")\n", "@link(\"demo\")\n@blocking\n");
    assert_eq!(native_output(&blocking), native_output(NUMBERS));
}

const TEXT: &str = r#"
@link("demo")
native fn length_of(text: String) -> Int
@link("demo")
native fn first_byte(text: String) -> Int
@link("demo")
native fn greeting() -> String
@link("demo")
native fn nothing() -> String
@link("demo")
native fn sum_ints(items: [Int]) -> Int
@link("demo")
native fn sum_floats(items: [Float]) -> Float
@link("demo")
native fn count_set(items: [Bool]) -> Int

fn main() {
    println(length_of("héllo"))
    println(length_of(""))
    println(first_byte("A"))
    println(greeting())
    println(greeting().len())
    println("[${nothing()}]")
    println(sum_ints([1, 2, 3, 4]))
    println(sum_ints([]))
    println(sum_floats([0.5, 1.5]))
    println(count_set([true, false, true]))
}
"#;

#[test]
fn differential_natives_strings_and_arrays_cross_by_copy_and_by_pointer() {
    assert_eq!(native_output(TEXT), "6\n0\n65\nhello from c\n12\n[]\n10\n0\n2.0\n2\n");
}

const POINTERS: &str = r#"
import { Pointer } from std.types

struct Cell {}

@link("demo")
native fn cell_new(value: Int) -> Pointer<Cell>
@link("demo")
native fn cell_get(cell: Pointer<Cell>) -> Int
@link("demo")
native fn cell_none() -> Pointer<Cell>
@link("demo")
native fn cell_find(key: Int) -> Pointer<Cell>?
@link("demo")
native fn cell_or(cell: Pointer<Cell>?, otherwise: Int) -> Int
@link("demo")
native fn fresh_cell(var cell: Pointer<Cell>?)
@link("demo")
native fn clear_cell(var cell: Pointer<Cell>?)
"#;

#[test]
fn differential_natives_a_pointer_is_an_address_and_a_nullable_one_may_be_null() {
    let source = format!(
        "{POINTERS}\nfn main() {{\n    let cell = cell_new(41)\n    println(cell_get(cell) + 1)\n    println(cell.address != 0)\n    println(cell_find(7) != null)\n    println(cell_find(8) == null)\n    println(cell_or(cell_find(7), 0))\n    println(cell_or(cell_find(8), -1))\n    println(cell_or(cell, 0))\n    println(cell_or(null, 9))\n}}\n"
    );
    assert_eq!(native_output(&source), "42\ntrue\ntrue\ntrue\n70\n-1\n41\n9\n");
}

#[test]
fn differential_natives_a_null_pointer_result_is_a_fault() {
    let source = format!("{POINTERS}\nfn main() {{\n    println(\"before\")\n    let cell = cell_none()\n    println(cell.address)\n}}\n");
    let run = native_run(&source).unwrap_or_else(|failure| panic!("{failure}"));
    assert_eq!(run.output, "before\n");
    let Ended::Fault { text, kind } = run.ended else { panic!("{:?}", run.ended) };
    assert_eq!(kind, "NullPointer");
    assert!(text.starts_with("fault: native returned a null pointer\n  at "), "{text}");
}

#[test]
fn differential_natives_a_var_parameter_is_passed_by_pointer_and_copied_back() {
    let source = format!(
        r#"{POINTERS}
@link("demo")
native fn bump_i32(var x: Int32)
@link("demo")
native fn swap_f64(var a: Float, var b: Float)
@link("demo")
native fn set_flag(var flag: Bool, on: Int)

fn main() {{
    var n: Int32 = 41
    bump_i32(n)
    println(n)
    var a = 1.5
    var b = 2.5
    swap_f64(a, b)
    println("${{a}} ${{b}}")
    var flag = false
    set_flag(flag, 1)
    println(flag)
    set_flag(flag, 0)
    println(flag)
    var cell: Pointer<Cell>? = null
    fresh_cell(cell)
    println(cell_or(cell, -1))
    clear_cell(cell)
    println(cell == null)
}}
"#
    );
    assert_eq!(native_output(&source), "42\n2.5 1.5\ntrue\nfalse\n5\ntrue\n");
}

#[test]
fn differential_natives_a_blocking_call_runs_on_another_thread() {
    let source = r#"
@link("demo", symbol = "thread_tag")
native fn here() -> Int
@link("demo", symbol = "thread_tag")
@blocking
native fn elsewhere() -> Int

fn main() {
    println(here() == here())
    println(here() == elsewhere())
}
"#;
    assert_eq!(native_output(source), "true\nfalse\n");
}

#[test]
fn differential_natives_a_native_is_a_function_value() {
    let source = "@link(\"demo\")\nnative fn add_i64(a: Int, b: Int) -> Int\n\nfn main() {\n    let add = add_i64\n    println(add(40, 2))\n}\n";
    assert_eq!(native_output(source), "42\n");
}

#[test]
fn differential_natives_the_library_is_given_the_host_table_before_main() {
    let source = "@link(\"demo\")\nnative fn host_size() -> UInt64\n\nfn main() {\n    println(host_size())\n}\n";
    assert_eq!(native_output(source), "32\n");
}

#[test]
fn differential_natives_a_missing_library_is_an_error_at_its_native_fn() {
    let source = "@link(\"absent\")\nnative fn gone() -> Int\n\nfn main() {\n    println(gone())\n}\n";
    let text = native_run(source).unwrap_err();
    assert!(text.starts_with("error: library `absent` not found\n"), "{text}");
    assert!(text.contains("2 | native fn gone() -> Int"), "{text}");
}

#[test]
fn differential_natives_a_missing_symbol_is_an_error_at_its_native_fn() {
    let source = "@link(\"demo\")\nnative fn nope() -> Int\n\nfn main() {\n    println(nope())\n}\n";
    let text = native_run(source).unwrap_err();
    assert!(text.starts_with("error: symbol `nope` not found in `demo`\n"), "{text}");
    assert!(text.contains("2 | native fn nope() -> Int"), "{text}");
}

#[test]
fn differential_natives_a_native_the_program_never_calls_is_not_loaded() {
    let source = "@link(\"absent\")\nnative fn gone() -> Int\n\nfn main() {\n    println(1)\n}\n";
    assert_eq!(native_output(source), "1\n");
}

#[test]
fn differential_ports_a_library_thread_pushes_to_the_port_it_was_given() {
    let source = "import { Port, PortMode, PortEnd } from std.port\n\n@link(\"demo\")\nnative fn feed_port(port: PortEnd, count: Int)\n\nfn main() {\n    let (end, rx) = Port.open(PortMode.Queue(8))\n    feed_port(end, 5)\n    var total = 0\n    for message in rx {\n        println(message.len())\n        total += message[0] as Int\n    }\n    println(total)\n}\n";
    assert_eq!(native_output(source), "1\n2\n3\n4\n5\n10\n");
}

#[test]
fn differential_ports_a_seeded_run_without_feeds_waits_for_a_library_thread() {
    let source = "import { Port, PortMode, PortEnd } from std.port\n\n@link(\"demo\")\nnative fn feed_port(port: PortEnd, count: Int)\n\nfn main() {\n    let (end, rx) = Port.open(PortMode.Queue(8))\n    feed_port(end, 3)\n    var total = 0\n    for message in rx {\n        total += message[0] as Int\n    }\n    println(total)\n}\n";
    let run = native_run_with(source, &Settings { seed: Some(3), ..Settings::default() }).unwrap_or_else(|failure| panic!("{failure}"));
    assert_eq!((run.ended, run.output), (Ended::Returned, "3\n".to_string()));
}

#[test]
fn differential_ports_a_push_is_queued_or_refused_when_full_and_a_released_port_closes() {
    let source = "import { Port, PortMode, PortEnd } from std.port\n\n@link(\"demo\")\nnative fn push_status(port: PortEnd, length: Int) -> Int\n\n@link(\"demo\")\nnative fn release_port(port: PortEnd)\n\nfn main() {\n    let (end, rx) = Port.open(PortMode.Queue(1))\n    println(push_status(end, 1))\n    println(push_status(end, 2))\n    println(rx.recv()!.len())\n    release_port(end)\n    println(rx.recv() == null)\n    println(rx.is_closed())\n}\n";
    assert_eq!(native_output(source), "0\n1\n1\ntrue\ntrue\n");
}

#[test]
fn differential_ports_a_latest_port_keeps_the_newest_push() {
    let source = "import { Port, PortMode, PortEnd } from std.port\n\n@link(\"demo\")\nnative fn push_status(port: PortEnd, length: Int) -> Int\n\nfn main() {\n    let (end, rx) = Port.open(PortMode.Latest)\n    println(push_status(end, 1))\n    println(push_status(end, 2))\n    println(rx.recv()!.len())\n    println(rx.try_recv() == null)\n}\n";
    assert_eq!(native_output(source), "0\n0\n2\ntrue\n");
}

/// The problems of every `test` block of `std/tests` run compiled, optimized or not, against the file beside its source.
fn std_tests(optimize: bool) -> Vec<String> {
    let dir = common::root().join("std/tests");
    let mut files: Vec<_> = std::fs::read_dir(&dir).expect("the std/tests directory").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.extension().is_some_and(|ext| ext == "mote")).collect();
    files.sort();
    let found = common::blocks::in_parallel(&files, |_, path| {
        let name = path.file_name().and_then(|name| name.to_str()).expect("a file name").to_string();
        let source = std::fs::read_to_string(path).expect("a readable file");
        let scratch = Scratch::new();
        let package = scratch.package("corpus", "corpus", &[], &[("main.mote", source.as_str())]);
        let mut pipeline = Pipeline::build(&package, Target::Test, &Options { optimize, ..Options::default() }).unwrap_or_else(|failure| panic!("{name}: {failure}"));
        match run_tests(&mut pipeline) {
            Ok(run) if run.start_fault.is_some() => Some(format!("{name}: start-up faulted: {}", run.start_fault.unwrap_or_default())),
            Ok(run) => check(&path.with_extension("expect"), &tests_text(&run), !optimize),
            Err(failure) => Some(format!("{name}: {failure}")),
        }
    });
    let mut problems: Vec<String> = found.into_iter().flatten().collect();
    problems.extend(orphans(&dir, "mote", "expect"));
    problems
}

#[test]
fn differential_gate_every_block_of_std_tests_gives_the_recorded_verdict_and_output() {
    assert_eq!(std_tests(false), Vec::<String>::new());
}

#[test]
fn differential_optimized_every_block_of_std_tests_gives_the_recorded_verdict_and_output() {
    assert_eq!(std_tests(true), Vec::<String>::new());
}

#[test]
fn differential_every_block_of_std_tests_passes() {
    let dir = common::root().join("std/tests");
    let mut problems = Vec::new();
    for path in std::fs::read_dir(&dir).expect("the std/tests directory").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.extension().is_some_and(|ext| ext == "expect")) {
        for (header, body) in sections(&std::fs::read_to_string(&path).expect("a readable file")).iter().skip(1).filter(|(_, body)| !body.starts_with("passed")) {
            problems.push(format!("{} {header}: {}", common::relative(&path), body.lines().next().unwrap_or_default()));
        }
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn differential_gate_a_main_that_returns_an_err_ends_with_the_text_of_the_error() {
    let error = run_on("fn main() -> Result<(), Error> {\n    println(\"a\")\n    return Err(Error.other(\"boom\"))\n}\n", false).unwrap();
    assert_eq!(error, run("a\n", Ended::Err("boom".into())));
    let enum_error = run_on("enum Bad {\n    Missing(String)\n    Full\n}\n\nfn main() -> Result<Int, Bad> {\n    return Err(Full)\n}\n", false).unwrap();
    assert_eq!(enum_error, run("", Ended::Err("Full".into())));
    let record_error = run_on("struct Bad {\n    code: Int\n    name: String\n}\n\nfn main() -> Result<Int, Bad> {\n    return Err(Bad { code: 7, name: \"x\" })\n}\n", false).unwrap();
    assert!(matches!(record_error.ended, Ended::Err(text) if text.contains('7')));
}

#[test]
fn differential_gate_a_main_that_returns_an_ok_returns() {
    assert_eq!(run_on("fn main() -> Result<Int, Error> {\n    println(\"b\")\n    return Ok(3)\n}\n", false).unwrap(), run("b\n", Ended::Returned));
}

#[test]
fn differential_gate_unit_compares_equal_to_null_and_an_enum_with_constants_casts_to_an_integer() {
    let source = "enum Level {\n    Low = 1\n    High = 200\n}\n\nfn nothing() {\n}\n\nfn main() {\n    println(nothing() == null)\n    println(nothing() != null)\n    println(Level.High as Int)\n    println(Level.High as UInt8)\n    println(Level.Low as Int16)\n}\n";
    assert_eq!(run_on(source, false).unwrap(), run("true\nfalse\n200\n200\n1\n", Ended::Returned));
}

#[test]
fn differential_gate_typeof_a_union_names_the_type_it_holds() {
    let source = "fn name(x: Int | String | Bool) -> String {\n    return typeof(x)\n}\n\nfn main() {\n    println(name(1))\n    println(name(\"s\"))\n    println(name(true))\n}\n";
    assert_eq!(run_on(source, false).unwrap(), run("Int\nString\nBool\n", Ended::Returned));
}

