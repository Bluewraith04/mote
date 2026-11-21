//! Runs the `mote` blocks of `docs/reference` compiled on `mote::Pipeline`, unoptimized and under every pass configuration.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::blocks::{build, classify, has_tests, in_parallel, judged, parse_page, read, target_of, Block, Verdict};
use mote::{run_main, run_tests, Ended, MainRun, Pipeline, Target, TestRun, Verdict as TestVerdict};
use mote_ir::opt::{configurations, optimize_with, Passes};
use mote_ir::verify;

/// What running a source shows: how `main` ended and what it wrote, and the `test` blocks.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    main: MainRun,
    tests: Option<TestRun>,
}

/// Builds and runs the source, with only `passes` applied after the build when given.
fn observe(source: &str, passes: Option<Passes>) -> Result<Seen, String> {
    let ready = |target| -> Result<Pipeline, String> {
        let mut pipeline = build(source, target).map_err(|errors| errors.join("; "))?;
        if let Some(passes) = passes {
            optimize_with(&mut pipeline.module, &mut pipeline.program, &mut pipeline.interner, passes);
            let problems = verify(&pipeline.module, &pipeline.program, &pipeline.interner);
            if !problems.is_empty() {
                return Err(format!("verify: {}", problems.iter().map(|problem| problem.message.as_str()).collect::<Vec<_>>().join("; ")));
            }
        }
        Ok(pipeline)
    };
    let mut pipeline = ready(target_of(source))?;
    let prefix = format!("{}/", pipeline.packages.root().dir.display());
    let mut main = run_main(&mut pipeline).map_err(|failure| failure.to_string())?;
    if let Ended::Fault { text, .. } = &mut main.ended {
        let kept: Vec<&str> = text.lines().filter(|line| !line.starts_with("  in ")).collect();
        *text = format!("{}\n", kept.join("\n")).replace(&prefix, "");
    }
    let tests = if has_tests(source) { Some(run_tests(&mut ready(Target::Check)?).map_err(|failure| failure.to_string())?) } else { None };
    Ok(Seen { main, tests })
}

/// The first line of the refusal of a library the machine does not have, which a block needs and cannot run without.
fn deferred(problem: &str) -> Option<String> {
    let first = problem.lines().next()?.trim_start_matches("error: ");
    (first.starts_with("library `") && first.contains("not found")).then(|| first.to_string())
}

/// The kind a block is marked `// faults` with: none when unmarked, an error when the mark names no kind.
fn fault_mark(text: &str) -> Result<Option<String>, String> {
    let Some(at) = text.find("// faults") else { return Ok(None) };
    let rest = text[at + "// faults".len()..].lines().next().unwrap_or_default();
    match rest.split_whitespace().next() {
        Some(kind) => Ok(Some(kind.to_string())),
        None => Err("`// faults` names no fault kind".to_string()),
    }
}

/// How a run that was built from the block disagrees with its page, if it does.
fn run_problems(block: &Block, seen: &Seen) -> Vec<String> {
    let mut problems = Vec::new();
    match (fault_mark(&block.text), &seen.main.ended) {
        (Err(problem), _) => problems.push(problem),
        (Ok(None), Ended::Returned | Ended::Exit(_)) => {}
        (Ok(None), Ended::Err(text)) => problems.push(format!("main returned the error {text}")),
        (Ok(None), Ended::Fault { text, .. }) => problems.push(format!("faulted: {}", text.lines().next().unwrap_or_default())),
        (Ok(Some(kind)), Ended::Fault { kind: found, .. }) if *found == kind => {}
        (Ok(Some(kind)), ended) => problems.push(format!("marked `// faults {kind}` and ended {ended:?}")),
    }
    if let Some(expected) = &block.output
        && *expected != seen.main.output
    {
        problems.push(format!("printed {:?}, the `output` block is {expected:?}", seen.main.output));
    }
    if let Some(tests) = &seen.tests {
        if let Some(fault) = &tests.start_fault {
            problems.push(format!("start-up faulted in the test run: {fault}"));
        }
        for line in tests.tests.iter().filter(|line| matches!(line.verdict, TestVerdict::Failed(_))) {
            problems.push(format!("test {} failed", line.name));
        }
    }
    problems
}

/// The lines of a list file in this crate's directory: block key, then the first line of what is wrong with it.
fn listed(file: &str) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file)).unwrap_or_else(|_| panic!("{file} is missing"));
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (key, what) = line.split_once(" | ").unwrap_or_else(|| panic!("{file}: `{line}` is not `key | text`"));
            (key.to_string(), what.to_string())
        })
        .collect()
}

/// What differs between the blocks found, by key with their name and what is wrong, and the blocks the file lists.
fn reconcile(file: &str, found: &BTreeMap<String, (String, String)>) -> Vec<String> {
    let listed = listed(file);
    let mut problems = Vec::new();
    for (key, (name, what)) in found {
        match listed.get(key) {
            None => problems.push(format!("{name} is not in {file}; add the line: {key} | {what}")),
            Some(known) if known != what => problems.push(format!("{name} ({key}) is `{what}`; {file} has `{known}`")),
            Some(_) => {}
        }
    }
    for key in listed.keys().filter(|key| !found.contains_key(*key)) {
        problems.push(format!("{key} is no longer like that or is gone; take it off {file}"));
    }
    problems
}

#[test]
fn docs_every_output_block_follows_a_mote_block() {
    assert_eq!(read().1, Vec::<String>::new());
}

#[test]
fn docs_every_block_is_run_rejected_as_marked_or_listed_as_a_fragment() {
    let (all, verdicts) = judged();
    let mut problems = Vec::new();
    let mut fragments = BTreeMap::new();
    let mut runs = Vec::new();
    for (index, (block, verdict)) in all.iter().zip(verdicts).enumerate() {
        match verdict {
            Verdict::Runs(source) => runs.push((index, source)),
            Verdict::Rejected => {
                if block.output.is_some() {
                    problems.push(format!("{} is rejected and has an `output` block", block.name));
                }
            }
            Verdict::Fragment(error) => {
                fragments.insert(block.key(), (block.name.clone(), error.clone()));
            }
            Verdict::Wrong(problem) => problems.push(problem.clone()),
        }
    }
    problems.extend(reconcile("fragments.txt", &fragments));
    let found = in_parallel(&runs, |_, (index, source)| match observe(source, None) {
        Ok(seen) => run_problems(&all[*index], &seen),
        Err(problem) => vec![problem],
    });
    let (mut broken, mut refused) = (BTreeMap::new(), BTreeMap::new());
    for ((index, _), found) in runs.iter().zip(found) {
        let Some(first) = found.into_iter().next() else { continue };
        let block = &all[*index];
        match deferred(&first) {
            Some(what) => refused.insert(block.key(), (block.name.clone(), what)),
            None => broken.insert(block.key(), (block.name.clone(), first)),
        };
    }
    problems.extend(reconcile("broken.txt", &broken));
    problems.extend(reconcile("gate_refused.txt", &refused));
    assert_eq!(problems, Vec::<String>::new());
    assert!(runs.len() >= 90, "only {} blocks run", runs.len());
}

#[test]
fn docs_every_pass_of_optimize_shows_what_the_unoptimized_run_of_a_block_shows() {
    let (all, verdicts) = judged();
    let runnable: Vec<(&str, &str)> = all.iter().zip(verdicts).filter_map(|(block, verdict)| if let Verdict::Runs(source) = verdict { Some((block.name.as_str(), source.as_str())) } else { None }).collect();
    let results = in_parallel(&runnable, |_, (name, source)| {
        let base = match observe(source, None) {
            Ok(base) => base,
            Err(problem) if deferred(&problem).is_some() => return Vec::new(),
            Err(problem) => return vec![format!("{name}: {problem}")],
        };
        let mut differences = Vec::new();
        for (config, passes) in configurations() {
            match observe(source, Some(passes)) {
                Ok(seen) if seen == base => {}
                Ok(seen) => differences.push(format!("{name} with {config}: {base:?} became {seen:?}")),
                Err(problem) => differences.push(format!("{name} with {config}: {problem}")),
            }
        }
        differences
    });
    assert_eq!(results.into_iter().flatten().collect::<Vec<_>>(), Vec::<String>::new());
}

fn block(text: &str, output: Option<&str>) -> Block {
    Block { name: "p.md#1".into(), page: "p.md".into(), text: text.into(), output: output.map(str::to_string) }
}

/// The problems of running the block alone.
fn problems_of(block: &Block) -> Vec<String> {
    run_problems(block, &observe(&block.text, None).unwrap())
}

#[test]
fn docs_harness_pairs_an_output_block_with_the_mote_block_before_it() {
    let page = "# T\n\n```mote\nprintln(1)\n```\n\n```output\n1\n```\n\ntext\n\n```mote\nprintln(2)\n```\n\n```text\nnot a mote block\n```\n";
    let (blocks, orphans) = parse_page("p.md", page);
    let seen: Vec<(&str, &str, Option<&str>)> = blocks.iter().map(|block| (block.name.as_str(), block.text.as_str(), block.output.as_deref())).collect();
    assert_eq!(seen, [("p.md#1", "println(1)\n", Some("1\n")), ("p.md#2", "println(2)\n", None)]);
    assert_eq!(orphans, Vec::<String>::new());
}

#[test]
fn docs_harness_reports_an_output_block_that_follows_no_mote_block() {
    let (blocks, orphans) = parse_page("p.md", "text\n\n```output\n1\n```\n\n```mote\nprintln(1)\n```\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(orphans, ["p.md:3: an `output` block after no `mote` block"]);
}

#[test]
fn docs_harness_compares_what_a_block_prints_with_its_output_block() {
    assert_eq!(problems_of(&block("println(1)\n", Some("1\n"))), Vec::<String>::new());
    assert_eq!(problems_of(&block("println(1)\n", None)), Vec::<String>::new());
    assert_eq!(problems_of(&block("println(1)\n", Some("2\n"))), ["printed \"1\\n\", the `output` block is \"2\\n\""]);
}

#[test]
fn docs_harness_wants_the_kind_a_fault_mark_names() {
    let fault = "let xs: List<Int> = []\nlet a = xs[0]\n";
    assert_eq!(problems_of(&block(&format!("{fault}// faults IndexOutOfRange\n"), None)), Vec::<String>::new());
    assert_eq!(problems_of(&block(&format!("{fault}// faults DivideByZero\n"), None)).len(), 1);
    assert_eq!(problems_of(&block(&format!("{fault}// faults\n"), None)), ["`// faults` names no fault kind"]);
    assert_eq!(problems_of(&block(fault, None)), ["faulted: fault: index out of range"]);
    assert_eq!(problems_of(&block("// faults IntegerOverflow\nprintln(1)\n", None)), ["marked `// faults IntegerOverflow` and ended Returned"]);
}

#[test]
fn docs_harness_runs_the_test_blocks_of_a_block() {
    let passing = "import { assert } from std.test\n\ntest \"t\" {\n    assert(true)\n}\n";
    assert_eq!(problems_of(&block(passing, None)), Vec::<String>::new());
    assert_eq!(problems_of(&block(&passing.replace("true", "false"), None)), ["test t failed"]);
}

#[test]
fn docs_harness_wants_an_error_mark_to_be_rejected_with_the_message_it_names() {
    let verdict = |text: &str| classify(&[block(text, None)], 0);
    assert!(matches!(verdict("let x: Int = \"s\" // error\n"), Verdict::Rejected));
    assert!(matches!(verdict("let x: Int = \"s\" // error: String\n"), Verdict::Rejected));
    assert!(matches!(verdict("let x: Int = \"s\" // error: no such thing\n"), Verdict::Wrong(text) if text.starts_with("p.md#1 is marked `// error: no such thing` and the error is ")));
    assert!(matches!(verdict("let x = 1 // error\n"), Verdict::Wrong(text) if text == "p.md#1 is marked `// error` and builds"));
}

#[test]
fn docs_harness_runs_a_block_after_the_declarations_before_it_on_its_page() {
    let declares = Block { name: "p.md#1".into(), page: "p.md".into(), text: "struct P {\n    x: Int\n}\n".into(), output: None };
    let uses = Block { name: "p.md#2".into(), page: "p.md".into(), text: "let p = P { x: 1 }\nprintln(p.x)\n".into(), output: None };
    assert!(matches!(classify(&[declares, uses], 1), Verdict::Runs(source) if source.starts_with("struct P")));
    let alone = block("println(nothing)\n", None);
    assert!(matches!(classify(&[alone], 0), Verdict::Fragment(text) if text == "unknown name `nothing`"));
}
