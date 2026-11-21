//! Runs the program of a pipeline on compiled code.

use mote_base::{render, Diagnostic, Span};
pub use mote_codegen::Feed;
use mote_codegen::LinkError;
use mote_ir::EntryKind;
use mote_packages::{library_file, Location};
use mote_runtime::HeapConfig;

use crate::pipeline::{Failure, Pipeline};

/// How `main` ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    Returned,
    /// `main` returned an integer other than 0, or the program called `exit` with one.
    Exit(i32),
    /// `main` returned an `Err`: the message of an `Error`, else the debug text of the value.
    Err(String),
    /// A fault: the text with the line it happened at, and the name of its kind.
    Fault { text: String, kind: String },
}

/// What `main` wrote and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MainRun {
    pub output: String,
    pub ended: Ended,
}

impl MainRun {
    /// 0 when `main` returned, the code of an exit, else 1.
    pub fn code(&self) -> i32 {
        match self.ended {
            Ended::Returned => 0,
            Ended::Exit(code) => code,
            _ => 1,
        }
    }

    /// The line for standard error, if `main` did not return.
    pub fn failure(&self) -> Option<String> {
        match &self.ended {
            Ended::Returned | Ended::Exit(_) => None,
            Ended::Err(text) => Some(format!("error: {text}\n")),
            Ended::Fault { text, .. } => Some(text.clone()),
        }
    }
}

/// `file:line:column` of the span, or nothing when it is outside every file.
fn place(pipeline: &Pipeline, span: Span) -> Option<String> {
    let at = pipeline.map.lookup(span.start)?;
    Some(format!("{}:{}:{}", pipeline.map.file(at.file).name, at.line, at.column))
}

/// `file:line` of the span with the file below the root package's directory.
fn block_place(pipeline: &Pipeline, span: Span) -> String {
    let Some(at) = pipeline.map.lookup(span.start) else { return String::new() };
    let name = &pipeline.map.file(at.file).name;
    let root = mote_packages::shown(&pipeline.packages.root().dir.join(""));
    let short = name.strip_prefix(&root).unwrap_or(name);
    format!("{short}:{}", at.line)
}

/// How a run is set up; the default is the scheduler and heap of the machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Worker threads of the scheduler.
    pub workers: Option<usize>,
    /// Runs on one worker with a virtual clock in the order the seed gives.
    pub seed: Option<u64>,
    /// The most bytes the heap holds.
    pub max_heap: Option<usize>,
    /// Keeps the heap report of the run.
    pub mem_stats: bool,
    /// Writes the output of `main` to standard output as it is printed, so `MainRun::output` is empty.
    pub live: bool,
    /// The arguments `std.io.args` gives.
    pub args: Vec<String>,
}

impl Settings {
    fn scheduler(&self) -> mote_scheduler::Config {
        let mut config = self.seed.map_or_else(mote_scheduler::Config::new, mote_scheduler::Config::seeded);
        if let Some(workers) = self.workers {
            config.workers = workers;
        }
        config
    }

    fn heap(&self) -> HeapConfig {
        let mut config = HeapConfig::default();
        if let Some(limit) = self.max_heap {
            config.limit = limit;
            config.trigger = config.trigger.min(limit / 2);
        }
        config
    }
}

/// A finished run, and the heap report when `Settings::mem_stats` asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub run: MainRun,
    pub mem_stats: Option<String>,
}

/// Runs start-up and `main`.
pub fn run_main(pipeline: &mut Pipeline) -> Result<MainRun, Failure> {
    run_main_fed(pipeline, &[])
}

/// Like `run_main`, with the feeds pushed to the ports `main` opens.
pub fn run_main_fed(pipeline: &mut Pipeline, feeds: &[Feed]) -> Result<MainRun, Failure> {
    Ok(compiled_main(pipeline, feeds, &Settings::default())?.run)
}

/// Like `run_main`, on the settings.
pub fn run_main_with(pipeline: &mut Pipeline, settings: &Settings) -> Result<Finished, Failure> {
    compiled_main(pipeline, &[], settings)
}

/// The file the loader opens for each library of the program, in the order of its link table.
pub(crate) fn library_files(pipeline: &Pipeline) -> Vec<String> {
    let name = |library: &mote_packages::Library| match &library.location {
        Location::File(path) => path.display().to_string(),
        Location::System => library_file(&library.name),
    };
    pipeline.natives.libraries.iter().map(name).collect()
}

/// The error for a library or symbol that could not be loaded, at its `native fn`.
fn link_failure(pipeline: &Pipeline, error: &LinkError) -> Failure {
    Failure { messages: vec![render(&pipeline.map, &Diagnostic::error(error.message.clone(), error.span))] }
}

/// Compiles the program and runs start-up and `main` on the machine code.
fn compiled_main(pipeline: &mut Pipeline, feeds: &[Feed], settings: &Settings) -> Result<Finished, Failure> {
    let error_text = pipeline.error_text.as_ref().and_then(|instance| pipeline.module.function(instance));
    let compiled = mote_codegen::compile_with(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main, error_text).map_err(compile_failure)?;
    let mut compiled = compiled.with_libraries(library_files(pipeline)).with_feeds(feeds.to_vec()).with_heap(settings.heap()).with_scheduler(settings.scheduler()).with_args(settings.args.clone());
    if settings.live {
        compiled = compiled.with_sink(Box::new(std::io::stdout()));
    }
    let ran = compiled.run();
    if let Some(error) = &ran.link {
        return Err(link_failure(pipeline, error));
    }
    let ended = match ran.fault {
        None => ran.err.map_or(if ran.exit == 0 { Ended::Returned } else { Ended::Exit(ran.exit) }, Ended::Err),
        Some(fault) => compiled_fault(pipeline, &fault),
    };
    let mem_stats = settings.mem_stats.then(|| ran.stats.to_string());
    Ok(Finished { run: MainRun { output: ran.output, ended }, mem_stats })
}

pub(crate) fn compile_failure(error: mote_codegen::CompileError) -> Failure {
    Failure { messages: vec![format!("error: {error}\n")] }
}

fn compiled_fault(pipeline: &Pipeline, fault: &mote_codegen::Fault) -> Ended {
    let mut text = match fault.span.and_then(|(start, end)| place(pipeline, Span::new(start, end))) {
        Some(at) => format!("fault: {}\n  at {at}\n", fault.message),
        None => format!("fault: {}\n", fault.message),
    };
    for frame in &fault.frames {
        match block_place(pipeline, Span::new(frame.position, frame.position)) {
            at if at.is_empty() || frame.position == mote_codegen::Located::NONE => text.push_str(&format!("  in {}\n", frame.function)),
            at => text.push_str(&format!("  in {} ({at})\n", frame.function)),
        }
    }
    Ended::Fault { text, kind: format!("{:?}", fault.kind) }
}

/// How one `test` block ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    Skipped,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestLine {
    pub name: String,
    /// `file:line` of the block.
    pub at: String,
    pub verdict: Verdict,
    /// What the block printed.
    pub output: String,
}

/// The `test` blocks of a pipeline, run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRun {
    /// What start-up printed.
    pub start_output: String,
    /// Set when start-up faulted; no block ran.
    pub start_fault: Option<String>,
    pub tests: Vec<TestLine>,
}

impl TestRun {
    pub fn passed(&self) -> usize {
        self.tests.iter().filter(|test| test.verdict == Verdict::Passed).count()
    }

    pub fn skipped(&self) -> usize {
        self.tests.iter().filter(|test| test.verdict == Verdict::Skipped).count()
    }

    pub fn failed(&self) -> usize {
        self.tests.iter().filter(|test| matches!(test.verdict, Verdict::Failed(_))).count()
    }

    /// 0 when every block that ran passed, else 1.
    pub fn code(&self) -> i32 {
        i32::from(self.failed() > 0 || self.start_fault.is_some())
    }

    /// One line per block, then each failure with what it printed, then the count.
    pub fn report(&self) -> String {
        let mut out = self.start_output.clone();
        if let Some(fault) = &self.start_fault {
            out.push_str(fault);
        }
        for test in &self.tests {
            let word = match test.verdict {
                Verdict::Passed => "ok",
                Verdict::Skipped => "skipped",
                Verdict::Failed(_) => "FAILED",
            };
            out.push_str(&format!("test {} ({}) ... {word}\n", test.name, test.at));
        }
        if self.failed() > 0 {
            out.push_str("\nfailures:\n");
            for test in &self.tests {
                if let Verdict::Failed(message) = &test.verdict {
                    out.push_str(&format!("\n{} ({}): {message}\n{}", test.name, test.at, test.output));
                }
            }
        }
        let word = if self.code() == 0 { "ok" } else { "FAILED" };
        out.push_str(&format!("\ntest result: {word}. {} passed; {} failed; {} skipped\n", self.passed(), self.failed(), self.skipped()));
        out
    }
}

/// `file:line` of the `test` block the function was lowered from.
fn test_place(pipeline: &Pipeline, func: mote_ir::FuncId) -> String {
    let span = match &pipeline.module.functions[func].kind {
        mote_ir::FunctionKind::Instance(instance) => pipeline.program.defs[instance.def].span,
        _ => Span::default(),
    };
    block_place(pipeline, span)
}

/// Whether the `test` block has the `@ignore` attribute.
fn ignored(pipeline: &Pipeline, func: mote_ir::FuncId) -> bool {
    let mote_ir::FunctionKind::Instance(instance) = &pipeline.module.functions[func].kind else { return false };
    let def = &pipeline.program.defs[instance.def];
    let Some(origin) = def.origin else { return false };
    pipeline.program.modules[def.module].ast.items[origin.item].attrs.iter().any(|attr| pipeline.interner.resolve(attr.name.name) == "ignore")
}

/// Runs start-up, then each `test` block on its own.
pub fn run_tests(pipeline: &mut Pipeline) -> Result<TestRun, Failure> {
    run_tests_with(pipeline, &Settings::default())
}

/// Like `run_tests`, on the settings.
pub fn run_tests_with(pipeline: &mut Pipeline, settings: &Settings) -> Result<TestRun, Failure> {
    let files = library_files(pipeline);
    let start = mote_codegen::compile_start(&pipeline.module, &mut pipeline.program, &pipeline.interner).map_err(compile_failure)?;
    let started = start.with_libraries(files.clone()).with_heap(settings.heap()).with_scheduler(settings.scheduler()).run();
    if let Some(error) = &started.link {
        return Err(link_failure(pipeline, error));
    }
    let start_output = started.output;
    if let Some(fault) = &started.fault {
        let Ended::Fault { text, .. } = compiled_fault(pipeline, fault) else { unreachable!("a fault ends as a fault") };
        return Ok(TestRun { start_output, start_fault: Some(text), tests: Vec::new() });
    }
    let blocks: Vec<(mote_ir::FuncId, EntryKind)> = pipeline.module.entries.iter().filter(|entry| matches!(entry.kind, EntryKind::Test(_))).map(|entry| (entry.func, entry.kind)).collect();
    let mut tests = Vec::new();
    for (func, kind) in blocks {
        let EntryKind::Test(name) = kind else { continue };
        let (name, at) = (pipeline.interner.resolve(name).to_string(), test_place(pipeline, func));
        if ignored(pipeline, func) {
            tests.push(TestLine { name, at, verdict: Verdict::Skipped, output: String::new() });
            continue;
        }
        let compiled = mote_codegen::compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, kind).map_err(compile_failure)?;
        let ran = compiled.with_libraries(files.clone()).with_heap(settings.heap()).with_scheduler(settings.scheduler()).run();
        if let Some(error) = &ran.link {
            return Err(link_failure(pipeline, error));
        }
        let output = ran.output.strip_prefix(&start_output).unwrap_or(&ran.output).to_string();
        let verdict = match ran.fault {
            None => Verdict::Passed,
            Some(fault) => Verdict::Failed(fault.message),
        };
        tests.push(TestLine { name, at, verdict, output });
    }
    Ok(TestRun { start_output, start_fault: None, tests })
}

#[cfg(test)]
mod tests {
    use mote_packages::testing::Scratch;

    use super::*;
    use crate::pipeline::{Options, Target};

    fn build(source: &str, target: Target) -> Pipeline {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
        Pipeline::build(&dir, target, &Options::default()).unwrap_or_else(|failure| panic!("{failure}"))
    }

    fn ran(source: &str) -> MainRun {
        run_main(&mut build(source, Target::Run)).unwrap()
    }

    #[test]
    fn run_tests_the_compiled_backend_prints_and_returns() {
        let run = run_main(&mut build("fn main() {\n    println(1 + 2)\n}\n", Target::Run)).unwrap();
        assert_eq!(run, MainRun { output: "3\n".into(), ended: Ended::Returned });
    }

    #[test]
    fn run_tests_the_compiled_backend_stops_at_a_fault_with_its_line() {
        let source = "fn main() {\n    var zero = 0\n    zero += 0\n    println(5)\n    println(1 / zero)\n}\n";
        let run = run_main(&mut build(source, Target::Run)).unwrap();
        assert_eq!(run.output, "5\n");
        let Ended::Fault { text, kind } = &run.ended else { panic!("{run:?}") };
        assert_eq!(kind, "DivideByZero");
        assert!(text.starts_with("fault: division by zero\n  at "), "{text}");
        assert!(text.contains("src/main.mote:5:"), "{text}");
    }

    #[test]
    fn run_tests_the_compiled_backend_lists_each_function_of_the_stack_with_its_line() {
        let source = "fn halve(n: Int) -> Int {\n    return 10 / n\n}\n\nfn main() {\n    var zero = 0\n    zero += 0\n    println(halve(zero))\n}\n";
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
        let mut pipeline = Pipeline::build(&dir, Target::Run, &Options { optimize: false, ..Options::default() }).unwrap_or_else(|failure| panic!("{failure}"));
        let run = run_main(&mut pipeline).unwrap();
        let Ended::Fault { text, .. } = &run.ended else { panic!("{run:?}") };
        assert!(text.ends_with("\n  in halve (src/main.mote:2)\n  in main (src/main.mote:8)\n"), "{text}");
    }

    #[test]
    fn run_tests_the_compiled_backend_ends_with_the_error_main_returned() {
        let source = "enum Bad {\n    Missing(String)\n    Full\n}\n\nfn main() -> Result<Int, Bad> {\n    println(\"a\")\n    return Err(Missing(\"disk\"))\n}\n";
        let run = run_main(&mut build(source, Target::Run)).unwrap();
        assert_eq!(run, MainRun { output: "a\n".into(), ended: Ended::Err("Missing(\"disk\")".into()) });
        let source = "fn main() -> Result<(), Error> {\n    return Err(Error.other(\"boom\"))\n}\n";
        let run = run_main(&mut build(source, Target::Run)).unwrap();
        assert_eq!(run.ended, Ended::Err("boom".into()));
    }

    #[test]
    fn run_tests_main_that_returns_ends_with_code_0_and_its_output() {
        let run = ran("fn main() {\n    println(\"hello\")\n}\n");
        assert_eq!(run, MainRun { output: "hello\n".into(), ended: Ended::Returned });
        assert_eq!((run.code(), run.failure()), (0, None));
    }

    #[test]
    fn run_tests_main_that_returns_ok_ends_with_code_0() {
        let run = ran("fn main() -> Result<(), Error> {\n    println(\"x\")\n    return Ok(())\n}\n");
        assert_eq!(run, MainRun { output: "x\n".into(), ended: Ended::Returned });
    }

    #[test]
    fn run_tests_main_that_returns_an_error_prints_its_message_and_ends_with_code_1() {
        let run = ran("fn main() -> Result<(), Error> {\n    println(\"before\")\n    return Err(Error.other(\"boom\"))\n}\n");
        assert_eq!(run, MainRun { output: "before\n".into(), ended: Ended::Err("boom".into()) });
        assert_eq!((run.code(), run.failure().as_deref()), (1, Some("error: boom\n")));
    }

    #[test]
    fn run_tests_main_that_returns_another_error_prints_its_debug_text() {
        let source = "enum Bad {\n    Missing(String)\n    Full\n}\n\nfn main() -> Result<Int, Bad> {\n    return Err(Missing(\"disk\"))\n}\n";
        let run = ran(source);
        assert_eq!(run.ended, Ended::Err("Missing(\"disk\")".into()));
        assert_eq!(run.code(), 1);
    }

    #[test]
    fn run_tests_main_that_returns_an_int_ends_with_that_code() {
        let run = ran("fn main() -> Int {\n    return 7\n}\n");
        assert_eq!(run.ended, Ended::Exit(7));
        assert_eq!(run.code(), 7);
    }

    #[test]
    fn run_tests_main_that_returns_0_returns() {
        assert_eq!(ran("fn main() -> Int {\n    return 0\n}\n").ended, Ended::Returned);
    }

    #[test]
    fn run_tests_a_fault_in_main_ends_with_code_1_and_the_line() {
        let run = ran("fn main() {\n    println(\"a\")\n    let xs: List<Int> = []\n    println(xs[0])\n}\n");
        assert_eq!(run.output, "a\n");
        let Ended::Fault { text, kind } = &run.ended else { panic!("{run:?}") };
        assert_eq!(kind, "IndexOutOfRange");
        assert!(text.starts_with("fault: index out of range\n  at "), "{text}");
        assert!(text.contains("src/main.mote:4:"), "{text}");
        assert_eq!(run.code(), 1);
    }

    #[test]
    fn run_tests_a_fault_while_starting_up_ends_with_code_1() {
        let run = ran("let xs: List<Int> = []\nlet first = xs[0]\n\nfn main() {\n    println(first)\n}\n");
        assert!(matches!(run.ended, Ended::Fault { .. }), "{run:?}");
        assert_eq!(run.output, "");
    }

    const SUITE: &str = "import { assert } from std.test\n\ntest \"adds\" {\n    println(\"quiet\")\n    assert(1 + 1 == 2)\n}\n\ntest \"breaks\" {\n    println(\"loud\")\n    assert(1 + 1 == 3)\n}\n\n@ignore\ntest \"later\" {\n    assert(false)\n}\n";

    #[test]
    fn run_tests_lists_one_line_per_block_with_ignored_ones_skipped_and_a_count() {
        let run = run_tests(&mut build(SUITE, Target::Test)).unwrap();
        let report = run.report();
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines[..3], ["test adds (src/main.mote:3) ... ok", "test breaks (src/main.mote:8) ... FAILED", "test later (src/main.mote:13) ... skipped"]);
        assert_eq!(lines.last(), Some(&"test result: FAILED. 1 passed; 1 failed; 1 skipped"));
        assert_eq!((run.passed(), run.failed(), run.skipped(), run.code()), (1, 1, 1, 1));
    }

    #[test]
    fn run_tests_prints_what_a_failing_block_printed_and_not_what_a_passing_one_did() {
        let report = run_tests(&mut build(SUITE, Target::Test)).unwrap().report();
        assert!(report.contains("\nfailures:\n\nbreaks (src/main.mote:8): "), "{report}");
        assert!(report.contains("loud\n"), "{report}");
        assert!(!report.contains("quiet"), "{report}");
    }

    #[test]
    fn run_tests_a_run_where_every_block_passes_has_code_0() {
        let source = "import { assert } from std.test\n\ntest \"one\" {\n    assert(true)\n}\n";
        let run = run_tests(&mut build(source, Target::Test)).unwrap();
        assert_eq!(run.code(), 0);
        assert_eq!(run.report(), "test one (src/main.mote:3) ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 skipped\n");
    }

    #[test]
    fn run_tests_with_no_blocks_is_a_run_of_zero_with_code_0() {
        let run = run_tests(&mut build("fn main() {\n}\n", Target::Test)).unwrap();
        assert_eq!(run.report(), "\ntest result: ok. 0 passed; 0 failed; 0 skipped\n");
        assert_eq!(run.code(), 0);
    }

    #[test]
    fn run_tests_a_block_that_faults_fails_only_itself() {
        let source = "test \"first\" {\n    let xs: List<Int> = []\n    println(xs[0])\n}\n\ntest \"second\" {\n    println(\"fine\")\n}\n";
        let run = run_tests(&mut build(source, Target::Test)).unwrap();
        assert_eq!(run.tests.iter().map(|test| test.verdict.clone()).collect::<Vec<_>>(), [Verdict::Failed("index out of range".into()), Verdict::Passed]);
    }

    #[test]
    fn run_tests_a_fault_while_starting_up_runs_no_block_and_has_code_1() {
        let source = "let xs: List<Int> = []\nlet first = xs[0]\n\ntest \"never\" {\n    println(first)\n}\n";
        let run = run_tests(&mut build(source, Target::Test)).unwrap();
        assert_eq!(run.tests, Vec::new());
        assert_eq!(run.code(), 1);
        assert!(run.report().contains("fault: index out of range"), "{}", run.report());
    }

    #[test]
    fn run_tests_start_up_output_comes_first() {
        let source = "println(\"setup\")\n\ntest \"one\" {\n}\n";
        let report = run_tests(&mut build(source, Target::Test)).unwrap().report();
        assert!(report.starts_with("setup\ntest one"), "{report}");
    }
}
