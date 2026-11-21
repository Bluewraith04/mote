//! Compiled code and the collector: the safepoints, the stack maps, the allocation path and the gate.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mote::{Options, Pipeline, Target};
use mote_codegen::{compile, Feed, Ran};
use mote_ir::EntryKind;
use mote_packages::testing::Scratch;
use mote_runtime::{HeapConfig, Runtime};

/// A program run with the settings of a heap, and the runtime it ran in.
struct Collected {
    ran: Ran,
    runtime: Arc<Runtime>,
}

/// Runs the source in a heap with these settings.
fn run_in(source: &str, heap: HeapConfig) -> Collected {
    run_entry(&mut built(source), EntryKind::Main, heap, &[]).unwrap_or_else(|error| panic!("{error}"))
}

/// Runs the entry of the pipeline in a heap with these settings and the pushes to its ports, or says what the compiler refused.
fn run_entry(pipeline: &mut Pipeline, kind: EntryKind, heap: HeapConfig, feeds: &[Feed]) -> Result<Collected, mote_codegen::CompileError> {
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, kind)?.with_heap(heap).with_feeds(feeds.to_vec());
    let kept = Arc::new(Mutex::new(None));
    let slot = kept.clone();
    let ran = compiled.run_with(move |runtime: &Arc<Runtime>| *slot.lock().expect("the runtime") = Some(runtime.clone()));
    let runtime = kept.lock().expect("the runtime").take().expect("the runtime was handed over");
    Ok(Collected { ran, runtime })
}

/// A heap that collects at the next safepoint after every allocation and checks each collection.
fn tortured() -> HeapConfig {
    HeapConfig { torture: true, verify: true, ..HeapConfig::default() }
}

/// The source built without the optimizer, so its loops and calls stay.
fn built(source: &str) -> Pipeline {
    built_with(source, false)
}

/// The source built with the optimizer on or off.
fn built_with(source: &str, optimize: bool) -> Pipeline {
    let scratch = Scratch::new();
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    Pipeline::build(&dir, Target::Run, &Options { optimize, ..Options::default() }).unwrap_or_else(|failure| panic!("{failure}"))
}

/// What the program printed when a stop caught it, and how the run ended.
struct Caught {
    seen: String,
    ran: Ran,
}

/// Runs the program, stops the world once it has printed a line, and keeps what it had printed by then.
fn stopped_while_running(source: &str) -> Caught {
    let mut pipeline = built(source);
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).expect("a program that compiles");
    let seen = Arc::new(Mutex::new(String::new()));
    let kept = seen.clone();
    let ran = compiled.run_with(move |runtime: &Arc<Runtime>| {
        let runtime = runtime.clone();
        std::thread::spawn(move || {
            let mut printed = String::new();
            while !printed.contains("started\n") {
                printed.push_str(&String::from_utf8_lossy(&runtime.take_output()));
                std::thread::sleep(Duration::from_millis(1));
            }
            // SAFETY: the context of the run is alive until the run leaves, which removes it from the runtime.
            unsafe {
                runtime.stop_world(|| {
                    printed.push_str(&String::from_utf8_lossy(&runtime.take_output()));
                    *kept.lock().expect("the text") = printed;
                });
            }
        });
    });
    let seen = seen.lock().expect("the text").clone();
    Caught { seen, ran }
}

const LOOP: &str = "fn main() {\n    println(\"started\")\n    var n = 0\n    while n < 1500000000 {\n        n += 1\n    }\n    println(\"finished\")\n}\n";

#[test]
fn gc_polls_a_loop_with_no_call_is_stopped_before_it_ends() {
    let caught = stopped_while_running(LOOP);
    assert_eq!(caught.seen, "started\n");
    assert_eq!(caught.ran.fault, None);
}

#[test]
fn gc_polls_a_stopped_loop_goes_on_and_ends_with_its_result() {
    let caught = stopped_while_running(LOOP);
    assert!(caught.ran.output.ends_with("finished\n"), "{:?}", caught.ran.output);
}

const RECURSION: &str = "fn fib(n: Int) -> Int {\n    if n < 2 {\n        return n\n    }\n    return fib(n - 1) + fib(n - 2)\n}\n\nfn main() {\n    println(\"started\")\n    println(fib(38))\n    println(\"finished\")\n}\n";

#[test]
fn gc_polls_a_recursion_with_no_loop_is_stopped_before_it_ends() {
    let caught = stopped_while_running(RECURSION);
    assert_eq!(caught.seen, "started\n");
    assert_eq!(caught.ran.fault, None);
}

#[test]
fn gc_polls_a_stopped_recursion_goes_on_and_ends_with_its_result() {
    let caught = stopped_while_running(RECURSION);
    assert!(caught.ran.output.ends_with("39088169\nfinished\n"), "{:?}", caught.ran.output);
}

#[test]
fn gc_polls_a_stop_with_no_running_program_does_not_wait() {
    let mut pipeline = built("fn main() {\n    println(1)\n}\n");
    let compiled = compile(&pipeline.module, &mut pipeline.program, &pipeline.interner, EntryKind::Main).expect("a program that compiles");
    let late = Arc::new(Mutex::new(None));
    let kept = late.clone();
    let ran = compiled.run_with(move |runtime: &Arc<Runtime>| *kept.lock().expect("the runtime") = Some(runtime.clone()));
    assert_eq!(ran.output, "1\n");
    let runtime = late.lock().expect("the runtime").take().expect("the runtime was handed over");
    let reached = AtomicBool::new(false);
    // SAFETY: no context is joined after the run.
    unsafe { runtime.stop_world(|| reached.store(true, Ordering::SeqCst)) };
    assert!(reached.load(Ordering::SeqCst));
}

/// Strings, structs, lists and a recursive enum, held in variables, in frames and across calls.
const HOLDERS: &str = "struct Tag {\n    name: String\n    weight: Int\n}\n\nenum Tree {\n    Leaf\n    Branch(Tree, Int, Tree)\n}\n\nfn insert(t: Tree, v: Int) -> Tree {\n    return match t {\n        Leaf => Tree.Branch(Tree.Leaf, v, Tree.Leaf)\n        Branch(l, x, r) => if v < x { Tree.Branch(insert(l, v), x, r) } else { Tree.Branch(l, x, insert(r, v)) }\n    }\n}\n\nfn sum(t: Tree) -> Int {\n    return match t {\n        Leaf => 0\n        Branch(l, x, r) => sum(l) + x + sum(r)\n    }\n}\n\nfn label(n: Int) -> String {\n    return \"item-${n}\"\n}\n\nfn main() {\n    var names: List<String> = List.new()\n    var tags: List<Tag> = List.new()\n    var tree = Tree.Leaf\n    var i = 0\n    while i < 200 {\n        names.push(label(i))\n        tree = insert(tree, (i * 37) % 101)\n        tags.push(Tag { name: label(i * 2), weight: i })\n        i += 1\n    }\n    println(names.len())\n    println(names[199])\n    println(sum(tree))\n    println(tags[150].name)\n    var total = 0\n    var at = 0\n    while at < tags.len() {\n        total += tags[at].weight\n        at += 1\n    }\n    println(total)\n}\n";

#[test]
fn gc_maps_a_program_that_holds_references_in_every_kind_of_frame_prints_what_it_prints_without_collection() {
    let plain = run_in(HOLDERS, HeapConfig::default());
    let tortured = run_in(HOLDERS, tortured());
    assert_eq!(plain.ran.fault, None);
    assert_eq!(tortured.ran.fault, None);
    assert_eq!(tortured.ran.output, plain.ran.output);
    assert_eq!(plain.runtime.collections(), 0);
    assert!(tortured.runtime.collections() > 100, "{}", tortured.runtime.collections());
    assert_eq!(tortured.runtime.take_problems(), Vec::<String>::new());
}

/// A loop that allocates 20000 small objects and keeps only the last.
const CHURN: &str = "class Node {\n    var value: Int\n    var next: Node?\n}\n\nfn main() {\n    var last = Node { value: 0, next: null }\n    var total = 0\n    var i = 0\n    while i < 20000 {\n        last = Node { value: i, next: last }\n        last.next = null\n        total += last.value\n        i += 1\n    }\n    println(total)\n}\n";

#[test]
fn gc_fastpath_small_objects_are_bumped_in_the_region_and_the_runtime_is_called_only_for_a_new_one() {
    let run = run_in(CHURN, HeapConfig::default());
    assert_eq!(run.ran.fault, None);
    assert_eq!(run.ran.output, "199990000\n");
    let stats = run.runtime.heap().stats();
    assert!(stats.allocations > 0 && stats.allocations < 1000, "{}", stats.allocations);
}

#[test]
fn gc_fastpath_under_torture_every_object_calls_the_runtime() {
    let run = run_in(CHURN, tortured());
    assert_eq!(run.ran.output, "199990000\n");
    assert!(run.runtime.heap().stats().allocations >= 20000);
    assert_eq!(run.runtime.take_problems(), Vec::<String>::new());
}

/// Where compiled runs of each entry differ between a plain heap and a tortured one, or fail a check; an entry the compiler refuses is left to the differential gate.
fn tortured_differences(place: &str, pipeline: &mut Pipeline, feeds: &[Feed]) -> Vec<String> {
    let kinds: Vec<EntryKind> = pipeline.module.entries.iter().map(|entry| entry.kind).collect();
    let mut found = Vec::new();
    for kind in kinds {
        let Ok(plain) = run_entry(pipeline, kind, HeapConfig::default(), feeds) else { continue };
        let Ok(hard) = run_entry(pipeline, kind, tortured(), feeds) else { continue };
        let place = format!("{place} {kind:?}");
        if hard.ran.output != plain.ran.output || hard.ran.fault != plain.ran.fault {
            found.push(format!("{place}: {:?} {:?} != {:?} {:?}", hard.ran.output, hard.ran.fault, plain.ran.output, plain.ran.fault));
        }
        found.extend(hard.runtime.take_problems().into_iter().map(|problem| format!("{place}: {problem}")));
    }
    found
}

/// The programs of the group that end differently, or fail a check, when every allocation is followed by a collection.
fn tortured_disagreements(group: &str) -> Vec<String> {
    tortured_disagreements_with(group, false)
}

/// Like `tortured_disagreements`, with the optimizer on or off.
fn tortured_disagreements_with(group: &str, optimize: bool) -> Vec<String> {
    common::corpus(group).into_iter().flat_map(|(name, source)| tortured_differences(&format!("{group}/{name}"), &mut built_with(&source, optimize), &common::feeds_of(&source))).collect()
}

#[test]
fn gc_maps_pairs_enums_closures_and_cells_live_across_collections_keep_what_they_hold() {
    let mut found = tortured_disagreements("values");
    found.extend(tortured_disagreements("functions"));
    assert_eq!(found, Vec::<String>::new());
}

/// Every corpus group, with the optimizer on or off.
fn corpus_gate(optimize: bool) {
    let groups = ["scalar", "values", "objects", "sequences", "functions", "numeric", "strings", "collections", "meta", "tasks", "channels", "sync", "timed", "ports", "cancel", "deadlock", "streams"];
    let found = common::blocks::in_parallel(&groups, |_, group| tortured_disagreements_with(group, optimize));
    assert_eq!(found.into_iter().flatten().collect::<Vec<_>>(), Vec::<String>::new());
}

#[test]
fn gc_gate_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection() {
    corpus_gate(false);
}

#[test]
fn gc_optimized_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection() {
    corpus_gate(true);
}

#[test]
fn gc_gate_every_block_of_std_tests_ends_as_it_does_when_every_allocation_is_followed_by_a_collection() {
    std_tests_gate(false);
}

#[test]
fn gc_optimized_every_block_of_std_tests_ends_as_it_does_when_every_allocation_is_followed_by_a_collection() {
    std_tests_gate(true);
}

/// Every `test` block of `std/tests`, with the optimizer on or off.
fn std_tests_gate(optimize: bool) {
    let dir = common::root().join("mote_std/std/tests");
    let mut files: Vec<_> = std::fs::read_dir(&dir).expect("the std/tests directory").map(|entry| entry.expect("a directory entry").path()).filter(|path| path.extension().is_some_and(|ext| ext == "mote")).collect();
    files.sort();
    let found = common::blocks::in_parallel(&files, |_, path| {
        let name = path.file_name().and_then(|name| name.to_str()).expect("a file name").to_string();
        let source = std::fs::read_to_string(path).expect("a readable file");
        let scratch = Scratch::new();
        let dir = scratch.package("corpus", "corpus", &[], &[("main.mote", source.as_str())]);
        let mut pipeline = Pipeline::build(&dir, Target::Test, &Options { optimize, ..Options::default() }).unwrap_or_else(|failure| panic!("{name}: {failure}"));
        tortured_differences(&format!("std/tests/{name}"), &mut pipeline, &[])
    });
    assert_eq!(found.into_iter().flatten().collect::<Vec<_>>(), Vec::<String>::new());
}

#[test]
fn gc_gate_every_runnable_block_of_the_reference_ends_as_it_does_when_every_allocation_is_followed_by_a_collection() {
    reference_gate(false);
}

#[test]
fn gc_optimized_every_runnable_block_of_the_reference_ends_as_it_does_when_every_allocation_is_followed_by_a_collection() {
    reference_gate(true);
}

/// Every runnable block of the reference, with the optimizer on or off.
fn reference_gate(optimize: bool) {
    let (all, verdicts) = common::blocks::judged();
    let runnable: Vec<(&common::blocks::Block, &str)> = all.iter().zip(verdicts).filter_map(|(block, verdict)| if let common::blocks::Verdict::Runs(source) = verdict { Some((block, source.as_str())) } else { None }).collect();
    let found = common::blocks::in_parallel(&runnable, |_, (block, source)| {
        let mut pipeline = common::blocks::build_with(source, common::blocks::target_of(source), optimize).unwrap_or_else(|errors| panic!("{}: {}", block.name, errors.join("; ")));
        let mut found = tortured_differences(&format!("docs/reference/{}", block.key()), &mut pipeline, &[]);
        if common::blocks::has_tests(source) {
            let mut pipeline = common::blocks::build_with(source, Target::Check, optimize).unwrap_or_else(|errors| panic!("{}: {}", block.name, errors.join("; ")));
            found.extend(tortured_differences(&format!("docs/reference/{} tests", block.key()), &mut pipeline, &[]));
        }
        found
    });
    assert_eq!(found.into_iter().flatten().collect::<Vec<_>>(), Vec::<String>::new());
}

/// A live list of 500 entries and 160000 rounds of garbage strings, together more than 100 times a heap of 4 MiB.
const PRESSURE: &str = "class Entry {\n    var key: Int\n    var label: String\n}\n\nfn label(n: Int) -> String {\n    let base = \"0123456789012345678901234567890123456789\"\n    return base + base + \"-${n}\"\n}\n\nfn main() {\n    var kept: List<Entry> = List.new()\n    var i = 0\n    while i < 500 {\n        kept.push(Entry { key: i, label: label(i) })\n        i += 1\n    }\n    var wasted = 0\n    var round = 0\n    while round < 160000 {\n        var j = 0\n        while j < 32 {\n            wasted += label(round + j).len()\n            j += 1\n        }\n        if round % 50 == 0 {\n            kept[round % 500] = Entry { key: round % 500, label: label(round) }\n        }\n        round += 1\n    }\n    var keys = 0\n    var width = 0\n    for entry in kept {\n        keys += entry.key\n        width += entry.label.len()\n    }\n    println(wasted)\n    println(keys)\n    println(width)\n}\n";

#[test]
fn gc_gate_a_program_that_allocates_a_hundred_times_the_heap_limit_keeps_its_live_structure() {
    let limit = 4 << 20;
    let small = HeapConfig { limit, trigger: 512 << 10, verify: true, ..HeapConfig::default() };
    let roomy = run_in(PRESSURE, HeapConfig::default());
    let tight = run_in(PRESSURE, small);
    assert_eq!(roomy.ran.fault, None);
    assert_eq!(tight.ran.fault, None);
    assert_eq!(tight.ran.output, roomy.ran.output);
    let (wasted, _) = roomy.ran.output.split_once('\n').expect("the first line");
    // The garbage strings alone: their bytes and a header and a length each.
    let allocated = wasted.parse::<usize>().expect("a number") + 16 * 160000 * 32;
    assert!(allocated > 100 * limit, "{allocated}");
    let stats = tight.runtime.heap().stats();
    assert!(stats.collections >= 100, "{}", stats.collections);
    assert!(stats.peak_used <= limit, "{}", stats.peak_used);
    assert_eq!(tight.runtime.take_problems(), Vec::<String>::new());
}
