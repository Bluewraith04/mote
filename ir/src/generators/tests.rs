use mote_semantics::{compile, Compiled};

use crate::lower::lower_bodies;
use crate::{lower, print_function, verify, Function, FunctionKind, Lowered, Op, Terminator};

const HEAD: &str = "";

fn lowered(source: &str) -> (Compiled, Lowered) {
    let mut compiled = compile(&format!("{HEAD}{source}"));
    assert_eq!(compiled.errors, Vec::<String>::new());
    let lowered = lower(&mut compiled.program, &mut compiled.interner);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    (compiled, lowered)
}

fn named(compiled: &Compiled, function: &Function, name: &str) -> bool {
    matches!(&function.kind, FunctionKind::Instance(instance) if compiled.interner.resolve(compiled.program.defs[instance.def].name) == name)
}

/// The function named `name` and its resume function, as text without the register types.
fn dump(source: &str, name: &str) -> String {
    let (compiled, lowered) = lowered(source);
    let mut out = Vec::new();
    for (id, function) in lowered.module.functions.iter() {
        let resume = matches!(&function.kind, FunctionKind::Resume { generator } if named(&compiled, &lowered.module.functions[*generator], name));
        if named(&compiled, function, name) || resume {
            out.push(format!("f{}:", id.index()));
            out.extend(print_function(function, &compiled.program, &compiled.interner).lines().filter(|line| !line.starts_with("  r")).map(str::to_string));
        }
    }
    out.join("\n")
}

/// The resume function of the generator named `name`.
fn resume_of(source: &str, name: &str) -> (Compiled, Function) {
    let (compiled, lowered) = lowered(source);
    let found = lowered
        .module
        .functions
        .iter()
        .find(|(_, function)| matches!(&function.kind, FunctionKind::Resume { generator } if named(&compiled, &lowered.module.functions[*generator], name)))
        .map(|(_, function)| function.clone())
        .unwrap_or_else(|| panic!("no resume function of {name}"));
    (compiled, found)
}

fn ops(function: &Function) -> Vec<Op> {
    function.blocks.iter().flat_map(|(_, block)| block.insts.iter().map(|inst| inst.op.clone())).collect()
}

const COUNT: &str = "fn count(n: Int) -> Stream<Int> {\n var i = 0\n while i < n {\n yield i\n i += 1\n }\n}\n";

#[test]
fn a_generator_body_holds_its_yields_until_it_is_split() {
    let mut compiled = compile(&format!("{HEAD}fn f() -> Stream<Int> {{\n yield 1\n for i in 0..3 {{ yield i }}\n}}"));
    let lowered = lower_bodies(&mut compiled.program, &mut compiled.interner);
    let (_, function) = lowered.module.functions.iter().find(|(_, function)| named(&compiled, function, "f")).expect("a function f");
    let text = print_function(function, &compiled.program, &compiled.interner);
    let blocks: Vec<&str> = text.lines().filter(|line| !line.starts_with("  r")).collect();
    assert_eq!(
        blocks,
        [
            "generator fn instance f() -> Stream<Int>",
            "  b0:",
            "    yield 1",
            "    r0 = copy 0",
            "    r1 = copy 3",
            "    jump b1",
            "  b1:",
            "    r2 = lt r0, r1",
            "    branch r2, b2, b4",
            "  b2:",
            "    r3 = copy r0",
            "    yield r3",
            "    jump b3",
            "  b3:",
            "    r0 = add r0, 1",
            "    jump b1",
            "  b4:",
            "    return _",
        ]
    );
}

#[test]
fn a_generator_call_allocates_the_frame_and_returns_the_stream() {
    let text = dump(COUNT, "count");
    let expected = [
        "f0:",
        "fn instance count(r0) -> Stream<Int>",
        "  b0:",
        "    r1 = stream f4 [r0]",
        "    return r1",
        "f4:",
        "fn resume of f0() -> Int?",
        "  slot 0: Int",
        "  slot 1: Int",
        "  slot 2: Int",
        "  b0:",
        "    r1 = copy 0",
        "    jump b1",
        "  b1:",
        "    r2 = copy r1",
        "    r3 = lt r2, r0",
        "    branch r3, b2, b3",
        "  b2:",
        "    r4 = copy r1",
        "    frame_store 1, r0",
        "    frame_store 2, r1",
        "    frame_store 0, 1",
        "    r8 = to_optional r4",
        "    return r8",
        "  b3:",
        "    frame_store 0, -2",
        "    return null",
        "  b4:",
        "    r0 = frame_load 1",
        "    r1 = frame_load 2",
        "    r5 = copy r1",
        "    r6 = add r5, 1",
        "    r1 = copy r6",
        "    jump b1",
        "  b5:",
        "    r0 = frame_load 1",
        "    jump b0",
        "  b6:",
        "    frame_store 0, -2",
        "    return null",
        "  b7:",
        "    call intrinsic fault(\"a stream was read while it runs\")",
        "    unreachable",
        "  b8:",
        "    r7 = frame_load 0",
        "    frame_store 0, -1",
        "    switch r7 [0 => b5, -2 => b6, 1 => b4] else b7",
    ];
    assert_eq!(text, expected.join("\n"));
}

#[test]
fn a_generator_is_no_longer_a_generator_after_lowering() {
    let (_, lowered) = lowered(COUNT);
    assert!(lowered.module.functions.iter().all(|(_, function)| !function.is_generator));
    assert!(lowered.module.functions.iter().all(|(_, function)| !function.blocks.iter().any(|(_, block)| block.insts.iter().any(|inst| matches!(inst.op, Op::Yield(_))))));
}

#[test]
fn a_generator_frame_keeps_only_the_registers_live_across_a_yield() {
    let (_, resume) = resume_of("fn two() -> Stream<Int> {\n var x = 1\n var y = 2\n yield x\n yield y\n}", "two");
    assert_eq!(resume.frame.len(), 2);
    let stores = ops(&resume).into_iter().filter(|op| matches!(op, Op::FrameStore { slot, .. } if *slot != 0)).count();
    assert_eq!(stores, 1);
}

#[test]
fn a_generator_gets_a_resume_point_for_each_yield() {
    let (_, resume) = resume_of("fn three() -> Stream<Int> {\n yield 1\n yield 2\n yield 3\n}", "three");
    let switches: Vec<usize> = resume.blocks.iter().filter_map(|(_, block)| if let Terminator::Switch { cases, .. } = &block.term { Some(cases.len()) } else { None }).collect();
    assert_eq!(switches, [5]);
}

#[test]
fn a_generator_cell_live_across_a_yield_is_a_cell_slot() {
    let source = "fn counter() -> Stream<Int> {\n var n = 0\n let inc = || n += 1\n inc()\n yield n\n inc()\n yield n\n}";
    let (_, resume) = resume_of(source, "counter");
    assert!(resume.frame.iter().any(|slot| slot.cell));
}

