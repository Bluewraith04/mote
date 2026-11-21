//! A program or `test` block is linked into an executable, and what the executable prints and how it ends is compared with the file recorded beside it, under no seed and under every seed.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::blocks::in_parallel;
use common::expect::{block_body, main_text, output_of, sections};
use common::{corpus, feeds_of};
use mote::executable::{link, link_entry};
use mote::{Ended, MainRun, Options, Pipeline, Target, Verdict};
use mote_ir::EntryKind;
use mote_packages::testing::Scratch;

/// The corpus groups whose programs run as executables.
const GROUPS: [&str; 16] = ["cancel", "channels", "collections", "deadlock", "functions", "meta", "numeric", "objects", "ports", "scalar", "sequences", "streams", "strings", "sync", "tasks", "timed"];

/// How many seeds each task program runs under.
const SEEDS: u64 = 1000;

/// How many seeds the gate runs again with the optimizer on.
const OPTIMIZED_SEEDS: u64 = 100;

/// The corpus groups whose programs run under every seed; a seeded run feeds the ports on its virtual clock.
const SEEDED_GROUPS: [&str; 8] = ["cancel", "channels", "deadlock", "ports", "streams", "sync", "tasks", "timed"];

/// The `std/tests` files whose blocks run under every seed.
const SEEDED_TESTS: [&str; 2] = ["sync", "tasks"];

/// A program or `test` block that runs as an executable, with the text each run must show.
struct Job {
    place: String,
    source: String,
    /// The index of the `test` block, or none for `main`.
    block: Option<usize>,
    expected: String,
    /// What start-up prints before a block, which the block's output does not hold.
    start_output: String,
}

impl Job {
    fn program(place: String, source: String, expected: String) -> Job {
        Job { place, source, block: None, expected, start_output: String::new() }
    }
}

/// What an executable printed and how it ended.
struct Exit {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

/// The executable of the job in the scratch directory.
fn built(job: &Job, optimize: bool, scratch: &Scratch) -> Result<PathBuf, String> {
    let dir = scratch.package("corpus", "corpus", &[], &[("main.mote", job.source.as_str())]);
    let target = if job.block.is_some() { Target::Test } else { Target::Run };
    let mut pipeline = Pipeline::build(&dir, target, &Options { optimize, ..Options::default() }).map_err(|failure| failure.to_string())?;
    let out = scratch.dir.join("corpus-exe");
    match job.block {
        None => link(&mut pipeline, &out),
        Some(index) => {
            let kinds: Vec<EntryKind> = pipeline.module.entries.iter().map(|entry| entry.kind).filter(|kind| matches!(kind, EntryKind::Test(_))).collect();
            link_entry(&mut pipeline, kinds[index], &out)
        }
    }
    .map_err(|failure| failure.to_string())?;
    Ok(out)
}

/// The executable run under the seed, with the pushes the source asks for.
fn run_exe(exe: &Path, seed: Option<u64>, source: &str) -> Result<Exit, String> {
    let mut command = Command::new(exe);
    if let Some(seed) = seed {
        command.arg("--seed").arg(seed.to_string());
    }
    for feed in feeds_of(source) {
        match feed.bytes {
            Some(bytes) => command.arg("--feed").arg(feed.port.to_string()).arg(feed.at_ms.to_string()).arg(bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(",")),
            None => command.arg("--close").arg(feed.port.to_string()).arg(feed.at_ms.to_string()),
        };
    }
    let ran = command.output().map_err(|error| format!("cannot run the executable: {error}"))?;
    Ok(Exit { stdout: String::from_utf8_lossy(&ran.stdout).into_owned(), stderr: String::from_utf8_lossy(&ran.stderr).into_owned(), code: ran.status.code() })
}

/// How the executable ended, then its output, as a recorded file would show them; a fault takes its kind from the recorded text, which the executable does not print.
fn main_shown(exit: Exit, recorded: &str) -> Result<String, String> {
    let ended = match (exit.code, exit.stderr.as_str()) {
        (Some(0), "") => Ended::Returned,
        (Some(1), text) if text.starts_with("fault: ") => {
            let kind = recorded.lines().next().and_then(|line| line.strip_prefix("ended Fault ")).unwrap_or("?").to_string();
            Ended::Fault { text: text.to_string(), kind }
        }
        (Some(1), text) if text.starts_with("error: ") => Ended::Err(text.strip_prefix("error: ").unwrap_or(text).trim_end_matches('\n').to_string()),
        (Some(code), "") if code != 0 => Ended::Exit(code),
        (code, text) => return Err(format!("the executable ended with {code:?} and printed {text:?} to standard error")),
    };
    Ok(main_text(&MainRun { output: exit.stdout, ended }))
}

/// A block's verdict and output as a recorded file would show them.
fn block_shown(exit: Exit, start_output: &str) -> Result<String, String> {
    let verdict = match (exit.code, exit.stderr.as_str()) {
        (Some(0), "") => Verdict::Passed,
        (Some(1), text) if text.starts_with("fault: ") => Verdict::Failed(text.strip_prefix("fault: ").unwrap_or(text).trim_end_matches('\n').to_string()),
        (code, text) => return Err(format!("the executable ended with {code:?} and printed {text:?} to standard error")),
    };
    Ok(block_body(&verdict, exit.stdout.strip_prefix(start_output).unwrap_or(&exit.stdout)))
}

/// What the executable of the job shows under the seed.
fn shown(job: &Job, exe: &Path, seed: Option<u64>) -> Result<String, String> {
    let exit = run_exe(exe, seed, &job.source)?;
    match job.block {
        None => main_shown(exit, &job.expected),
        Some(_) => block_shown(exit, &job.start_output),
    }
}

/// What each seed shows for the job, with how many seeds show it; the executable is built once.
fn seeded_runs(job: &Job, seeds: u64, optimize: bool) -> Result<BTreeMap<String, u64>, String> {
    let scratch = Scratch::new();
    let exe = built(job, optimize, &scratch)?;
    let mut seen = BTreeMap::new();
    for seed in 0..seeds {
        *seen.entry(shown(job, &exe, Some(seed))?).or_insert(0) += 1;
    }
    Ok(seen)
}

/// The problems of the runs: a place that shows more than one result, and one result that is not the recorded one.
fn seeded_problems(jobs: &[Job], seeds: u64, optimize: bool) -> Vec<String> {
    let mut problems = Vec::new();
    for (job, found) in jobs.iter().zip(in_parallel(jobs, |_, job| seeded_runs(job, seeds, optimize))) {
        let place = job.place.as_str();
        match found {
            Err(error) => problems.push(format!("{place}: {error}")),
            Ok(seen) if seen.len() != 1 => problems.push(format!("{place} shows {} results under {seeds} seeds: {seen:?}", seen.len())),
            Ok(seen) if !seen.contains_key(&job.expected) => problems.push(format!("{place}: the recorded result is {:?}, every seed shows {:?}", job.expected, seen.keys().next())),
            Ok(_) => {}
        }
    }
    problems
}

/// The programs of the group with their recorded results.
fn programs(group: &str) -> Vec<Job> {
    let dir = common::root().join("integration/corpus").join(group);
    corpus(group).into_iter().map(|(name, source)| Job::program(format!("{group}/{name}"), source, std::fs::read_to_string(dir.join(format!("{name}.expect"))).unwrap_or_default())).collect()
}

/// The task programs of the corpus with their recorded results.
fn program_jobs() -> Vec<Job> {
    SEEDED_GROUPS.iter().flat_map(|group| programs(group)).collect()
}

/// The `test` blocks of `std/tests` that are about tasks, with their recorded results.
fn block_jobs() -> Vec<Job> {
    let mut jobs = Vec::new();
    for file in SEEDED_TESTS {
        let source = std::fs::read_to_string(common::root().join("std/tests").join(format!("{file}.mote"))).expect("a readable file");
        let recorded = std::fs::read_to_string(common::root().join("std/tests").join(format!("{file}.expect"))).unwrap_or_default();
        let found = sections(&recorded);
        let start_output = found.first().map(|(_, body)| output_of(body)).unwrap_or_default();
        for (index, (header, body)) in found.iter().skip(1).enumerate().filter(|(_, (_, body))| !body.starts_with("skipped")) {
            jobs.push(Job { place: format!("std/tests/{file}/{header}"), source: source.clone(), block: Some(index), expected: body.clone(), start_output: start_output.clone() });
        }
    }
    jobs
}

/// The problems of the programs of the group run as executables, against the files beside them.
fn problems(group: &str) -> Vec<String> {
    let jobs = programs(group);
    let found = in_parallel(&jobs, |_, job| {
        let scratch = Scratch::new();
        match built(job, true, &scratch).and_then(|exe| shown(job, &exe, None)) {
            Ok(text) if text == job.expected => None,
            Ok(text) => Some(format!("{}: the executable shows\n{text}but the file records\n{}", job.place, job.expected)),
            Err(error) => Some(format!("{}: {error}", job.place)),
        }
    });
    found.into_iter().flatten().collect()
}

#[test]
fn aot_a_recorded_output_is_what_the_executable_prints() {
    let recorded = "ended Returned\n| 1\n| 2\n";
    let job = Job::program("a".into(), "fn main() {\n    println(1)\n    println(2)\n}\n".into(), recorded.into());
    let scratch = Scratch::new();
    assert_eq!(shown(&job, &built(&job, true, &scratch).unwrap(), None).unwrap(), recorded);
}

#[test]
fn aot_a_fault_and_an_error_end_the_executable_with_code_1() {
    let recorded = "ended Fault DivideByZero\n| a\n";
    let job = Job::program("a".into(), "fn main() {\n    println(\"a\")\n    var zero = 0\n    zero += 0\n    println(1 / zero)\n}\n".into(), recorded.into());
    let scratch = Scratch::new();
    assert_eq!(shown(&job, &built(&job, true, &scratch).unwrap(), None).unwrap(), recorded);
}

/// Runs `cc` with the arguments.
fn cc(args: &[&std::ffi::OsStr]) {
    let status = Command::new("cc").args(args).status().expect("cc runs");
    assert!(status.success(), "cc {args:?}");
}

/// The package `app` with `libdemo.so` and, when `archive` is set, `libdemo.a` built from `integration/natives/demo.c`; the archive's init is `mote_init_demo`.
fn demo_package(scratch: &Scratch, source: &str, archive: bool) -> PathBuf {
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    let natives = dir.join("native").join(mote_packages::host_triple());
    std::fs::create_dir_all(&natives).expect("a directory for the library");
    let demo = common::root().join("integration/natives/demo.c");
    cc(&["-shared".as_ref(), "-fPIC".as_ref(), "-O1".as_ref(), "-o".as_ref(), natives.join("libdemo.so").as_os_str(), demo.as_os_str(), "-lpthread".as_ref()]);
    if archive {
        let object = natives.join("demo.o");
        cc(&["-c".as_ref(), "-fPIC".as_ref(), "-O1".as_ref(), "-Dmote_init=mote_init_demo".as_ref(), "-o".as_ref(), object.as_os_str(), demo.as_os_str()]);
        let made = Command::new("ar").arg("rcs").arg(natives.join("libdemo.a")).arg(&object).status().expect("ar runs");
        assert!(made.success(), "ar builds the archive");
    }
    dir
}

/// The executable of the package, run after the shared library and the copy beside the executable are removed.
fn run_without_the_shared_library(dir: &Path, scratch: &Scratch) -> Exit {
    let mut pipeline = Pipeline::build(dir, Target::Run, &Options::default()).unwrap_or_else(|failure| panic!("{failure}"));
    let out = scratch.dir.join("demo-exe");
    link(&mut pipeline, &out).unwrap_or_else(|failure| panic!("{failure}"));
    std::fs::remove_file(dir.join("native").join(mote_packages::host_triple()).join("libdemo.so")).expect("the shared library is there");
    let _ = std::fs::remove_dir_all(scratch.dir.join("native"));
    run_exe(&out, None, "").unwrap()
}

#[test]
fn aot_static_a_program_whose_natives_ship_an_archive_runs_without_the_shared_library() {
    let scratch = Scratch::new();
    let source = "@link(\"demo\")\nnative fn add_i64(a: Int, b: Int) -> Int\n\n@link(\"demo\")\nnative fn greeting() -> String\n\nfn main() {\n    println(add_i64(1, 2))\n    println(greeting())\n}\n";
    let dir = demo_package(&scratch, source, true);
    let exit = run_without_the_shared_library(&dir, &scratch);
    assert_eq!((exit.stdout.as_str(), exit.stderr.as_str(), exit.code), ("3\nhello from c\n", "", Some(0)));
}

#[test]
fn aot_static_a_linked_library_gets_its_init_before_main() {
    let scratch = Scratch::new();
    let source = "@link(\"demo\")\nnative fn host_size() -> UInt64\n\nfn main() {\n    println(host_size())\n}\n";
    let dir = demo_package(&scratch, source, true);
    let exit = run_without_the_shared_library(&dir, &scratch);
    assert_eq!((exit.stdout.as_str(), exit.code), ("32\n", Some(0)));
}

#[test]
fn aot_static_a_library_with_no_archive_is_opened_when_the_executable_starts() {
    let scratch = Scratch::new();
    let source = "@link(\"demo\")\nnative fn add_i64(a: Int, b: Int) -> Int\n\nfn main() {\n    println(add_i64(1, 2))\n}\n";
    let dir = demo_package(&scratch, source, false);
    let exit = run_without_the_shared_library(&dir, &scratch);
    assert_eq!((exit.stdout.as_str(), exit.stderr.as_str(), exit.code), ("", "error: library `demo` not found\n", Some(1)));
}

/// The pipeline of the source built as a package in a directory of its own; the scratch directory goes with it.
fn pipeline_of(source: &str, optimize: bool) -> (Scratch, Pipeline) {
    let scratch = Scratch::new();
    let dir = scratch.package("corpus", "corpus", &[], &[("main.mote", source)]);
    let pipeline = Pipeline::build(&dir, Target::Run, &Options { optimize, ..Options::default() }).unwrap_or_else(|failure| panic!("{failure}"));
    (scratch, pipeline)
}

#[test]
fn aot_reproducible_the_same_source_gives_the_same_object_from_two_directories() {
    let jobs: Vec<Job> = GROUPS.iter().flat_map(|group| programs(group)).collect();
    let found = in_parallel(&jobs, |_, job| {
        let mut differing = Vec::new();
        for optimize in [false, true] {
            let objects: Vec<Vec<u8>> = (0..2).map(|_| mote::executable::object(&mut pipeline_of(&job.source, optimize).1).unwrap_or_else(|failure| panic!("{failure}"))).collect();
            if objects[0] != objects[1] {
                differing.push(format!("{} (optimize {optimize}): the object files differ", job.place));
            }
        }
        differing
    });
    assert_eq!(found.concat(), Vec::<String>::new());
}

#[test]
fn aot_reproducible_the_same_source_links_the_same_executable_from_two_directories() {
    let jobs: Vec<Job> = ["scalar", "tasks", "cancel"].iter().flat_map(|group| programs(group)).take(12).collect();
    let found = in_parallel(&jobs, |_, job| {
        let executables: Vec<Vec<u8>> = (0..2)
            .map(|_| {
                let (scratch, mut pipeline) = pipeline_of(&job.source, true);
                let out = scratch.dir.join("corpus-exe");
                link(&mut pipeline, &out).unwrap_or_else(|failure| panic!("{failure}"));
                std::fs::read(out).expect("the executable")
            })
            .collect();
        (executables[0] != executables[1]).then(|| format!("{}: the executables differ", job.place))
    });
    assert_eq!(found.into_iter().flatten().collect::<Vec<_>>(), Vec::<String>::new());
}

const DEEP_FAULT: &str = "fn halve(n: Int) -> Int {\n    return 10 / n\n}\n\nfn main() {\n    var zero = 0\n    zero += 0\n    println(halve(zero))\n}\n";

#[test]
fn aot_backtrace_a_fault_names_the_check_and_each_function_with_its_line() {
    let job = Job::program("deep".into(), DEEP_FAULT.into(), String::new());
    let scratch = Scratch::new();
    let exit = run_exe(&built(&job, false, &scratch).unwrap(), None, DEEP_FAULT).unwrap();
    assert_eq!((exit.stdout.as_str(), exit.code), ("", Some(1)));
    assert_eq!(exit.stderr, "fault: division by zero\n  at src/main.mote:2:12\n  in halve (src/main.mote:2)\n  in main (src/main.mote:8)\n");
}

#[test]
fn aot_backtrace_an_inlined_function_is_named_by_the_function_it_was_inlined_into() {
    let job = Job::program("deep".into(), DEEP_FAULT.into(), String::new());
    let scratch = Scratch::new();
    let exit = run_exe(&built(&job, true, &scratch).unwrap(), None, DEEP_FAULT).unwrap();
    assert_eq!(exit.stderr, "fault: division by zero\n  at src/main.mote:2:12\n  in main (src/main.mote:2)\n");
}

#[test]
fn aot_every_corpus_group_shows_the_recorded_results() {
    let found: Vec<String> = GROUPS.iter().flat_map(|group| problems(group)).collect();
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn aot_seeds_every_task_program_shows_one_result_under_every_seed() {
    assert_eq!(seeded_problems(&program_jobs(), SEEDS, false), Vec::<String>::new());
}

#[test]
fn aot_seeds_optimized_every_task_program_shows_the_recorded_result_under_every_seed() {
    assert_eq!(seeded_problems(&program_jobs(), OPTIMIZED_SEEDS, true), Vec::<String>::new());
}

#[test]
fn aot_seeds_every_task_block_of_std_tests_shows_one_result_under_every_seed() {
    assert_eq!(seeded_problems(&block_jobs(), SEEDS, false), Vec::<String>::new());
}

#[test]
fn aot_seeds_optimized_every_task_block_of_std_tests_shows_the_recorded_result_under_every_seed() {
    assert_eq!(seeded_problems(&block_jobs(), OPTIMIZED_SEEDS, true), Vec::<String>::new());
}

#[test]
fn aot_seeds_a_racing_program_shows_both_orders_and_a_seed_repeats() {
    let source = "fn main() {\n    let a = spawn { println(\"a\") }\n    let b = spawn { println(\"b\") }\n    a.await()\n    b.await()\n}\n".to_string();
    let job = Job::program("race".into(), source, String::new());
    let first = seeded_runs(&job, 50, false).unwrap();
    assert_eq!(first.keys().cloned().collect::<Vec<_>>(), ["ended Returned\n| a\n| b\n", "ended Returned\n| b\n| a\n"]);
    assert_eq!(seeded_runs(&job, 50, false).unwrap(), first);
}
