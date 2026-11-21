use mote_semantics::{Compiled, DefId};

use super::tests::lowered;
use crate::{print_function, print_module, verify, Callee, FuncId, FunctionKind, Lowered, Op};

fn in_main(compiled: &Compiled, def: DefId) -> bool {
    compiled.interner.resolve(compiled.program.modules[compiled.program.defs[def].module].path[0]) == "main"
}

/// The entries, the functions of module `main` and its start-up steps, and what `start` runs of them.
fn text(compiled: &Compiled, lowered: &Lowered) -> String {
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let mut out: Vec<String> = print_module(&lowered.module, &compiled.program, &compiled.interner).lines().filter(|line| line.starts_with("entry")).map(str::to_string).collect();
    let mut steps: Vec<FuncId> = Vec::new();
    for (id, function) in lowered.module.functions.iter() {
        let keep = match &function.kind {
            FunctionKind::Global(def) => in_main(compiled, *def),
            FunctionKind::TopLevel(module) => compiled.interner.resolve(compiled.program.modules[*module].path[0]) == "main",
            FunctionKind::Instance(instance) => in_main(compiled, instance.def),
            _ => false,
        };
        if keep {
            if !matches!(function.kind, FunctionKind::Instance(_)) {
                steps.push(id);
            }
            out.push(format!("f{}:", id.index()));
            out.extend(print_function(function, &compiled.program, &compiled.interner).lines().filter(|line| !line.starts_with("  r")).map(str::to_string));
        }
    }
    let start = &lowered.module.functions[lowered.module.start.expect("a start function")];
    let mut runs = Vec::new();
    let insts = &start.blocks[start.entry].insts;
    for (position, inst) in insts.iter().enumerate() {
        if let Op::Call { callee: Callee::Function(func), .. } = &inst.op
            && steps.contains(func)
        {
            let stored = match insts.get(position + 1).map(|next| &next.op) {
                Some(Op::StoreGlobal { def, .. }) => format!(" -> {}", compiled.interner.resolve(compiled.program.defs[*def].name)),
                _ => String::new(),
            };
            runs.push(format!("f{}{stored}", func.index()));
        }
    }
    out.push(format!("start runs {}", runs.join(", ")));
    out.join("\n")
}

/// The text of `main` must be exactly `expected`, one line each, with nothing unsupported.
fn expect(source: &str, expected: &[&str]) {
    let (compiled, lowered) = lowered(source);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(text(&compiled, &lowered), expected.join("\n"));
}

#[test]
fn globals_are_initialized_after_the_globals_they_read() {
    expect(
        "let total = base * 2\nlet base = 10\nvar count = 0\nfn main() -> Int {\n return total\n}",
        &[
            "entry main f0",
            "f0:",
            "fn instance main() -> Int",
            "  b0:",
            "    r0 = load_global total",
            "    return r0",
            "f3:",
            "fn global base() -> Int",
            "  b0:",
            "    return 10",
            "f4:",
            "fn global total() -> Int",
            "  b0:",
            "    r0 = load_global base",
            "    r1 = mul r0, 2",
            "    return r1",
            "f5:",
            "fn global count() -> Int",
            "  b0:",
            "    return 0",
            "start runs f3 -> base, f4 -> total, f5 -> count",
        ],
    );
}

#[test]
fn a_destructured_global_is_one_step_that_stores_each_name() {
    expect(
        "let (a, b) = (1, 2)\nfn main() -> Int { return a + b }",
        &[
            "entry main f0",
            "f0:",
            "fn instance main() -> Int",
            "  b0:",
            "    r0 = load_global a",
            "    r1 = load_global b",
            "    r2 = add r0, r1",
            "    return r2",
            "f3:",
            "fn global a() -> Null",
            "  b0:",
            "    r0 = tuple 1, 2",
            "    r1 = tuple_field r0.0",
            "    r2 = copy r1",
            "    r3 = tuple_field r0.1",
            "    r4 = copy r3",
            "    store_global a, r2",
            "    store_global b, r4",
            "    return _",
            "start runs f3",
        ],
    );
}

#[test]
fn top_level_statements_run_after_the_globals_of_their_module() {
    expect(
        "var hits = 0\nhits += 1\nfn main() -> Int { return hits }",
        &[
            "entry main f0",
            "f0:",
            "fn instance main() -> Int",
            "  b0:",
            "    r0 = load_global hits",
            "    return r0",
            "f3:",
            "fn global hits() -> Int",
            "  b0:",
            "    return 0",
            "f4:",
            "fn top-level m26() -> Null",
            "  b0:",
            "    r0 = load_global hits",
            "    r1 = add r0, 1",
            "    store_global hits, r1",
            "    return _",
            "start runs f3 -> hits, f4",
        ],
    );
}

#[test]
fn main_and_each_test_block_are_entries_in_source_order() {
    expect(
        "fn main() {}\ntest \"adds\" {\n let n = 1 + 1\n}\ntest \"other\" {}",
        &[
            "entry main f0",
            "entry test \"adds\" f1",
            "entry test \"other\" f2",
            "f0:",
            "fn instance main() -> Null",
            "  b0:",
            "    return _",
            "f1:",
            "fn instance adds() -> Null",
            "  b0:",
            "    r0 = add 1, 1",
            "    r1 = copy r0",
            "    return _",
            "f2:",
            "fn instance other() -> Null",
            "  b0:",
            "    return _",
            "start runs ",
        ],
    );
}

#[test]
fn std_has_its_own_start_up_steps_before_the_program() {
    let (compiled, lowered) = lowered("fn main() {}");
    let start = &lowered.module.functions[lowered.module.start.expect("a start function")];
    let stores: Vec<&str> = start.blocks[start.entry].insts.iter().filter_map(|inst| if let Op::StoreGlobal { def, .. } = &inst.op { Some(compiled.interner.resolve(compiled.program.defs[*def].name)) } else { None }).collect();
    assert_eq!(stores, ["PI", "E"]);
}
