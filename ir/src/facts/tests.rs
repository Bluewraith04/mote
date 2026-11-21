use mote_semantics::{compile, Compiled};

use super::{analyze, Facts};
use crate::{lower, verify, FuncId, FunctionKind, Lowered};

fn analyzed(source: &str) -> (Compiled, Lowered, Facts) {
    let mut compiled = compile(source);
    assert_eq!(compiled.errors, Vec::<String>::new());
    let lowered = lower(&mut compiled.program, &mut compiled.interner);
    assert_eq!(lowered.unsupported, []);
    assert_eq!(verify(&lowered.module, &compiled.program, &compiled.interner), []);
    let facts = analyze(&lowered.module, &compiled.program, &compiled.interner);
    (compiled, lowered, facts)
}

/// The instance named `name`, or the top-level code of module `main` for `<top>`.
fn find(compiled: &Compiled, lowered: &Lowered, name: &str) -> FuncId {
    lowered
        .module
        .functions
        .iter()
        .find(|(_, function)| match &function.kind {
            FunctionKind::Instance(instance) => compiled.interner.resolve(compiled.program.defs[instance.def].name) == name,
            FunctionKind::TopLevel(module) => name == "<top>" && compiled.interner.resolve(compiled.program.modules[*module].path[0]) == "main",
            _ => false,
        })
        .unwrap_or_else(|| panic!("no function {name}"))
        .0
}

/// Each last use as `b0.2: r1, r3`, with `.t` for the terminator.
fn last_uses(source: &str, name: &str) -> Vec<String> {
    let (compiled, lowered, facts) = analyzed(source);
    let id = find(&compiled, &lowered, name);
    let function = &lowered.module.functions[id];
    let mut out = Vec::new();
    for (block, data) in function.blocks.iter() {
        for at in 0..=data.insts.len() {
            let regs = facts.function(id).last_uses(block, at);
            if !regs.is_empty() {
                let place = if at == data.insts.len() { "t".to_string() } else { at.to_string() };
                let regs: Vec<String> = regs.iter().map(|reg| format!("r{}", reg.index())).collect();
                out.push(format!("b{}.{place}: {}", block.index(), regs.join(", ")));
            }
        }
    }
    out
}

/// Each tracked allocation as `op: flags`, in program order.
fn allocations(source: &str, name: &str) -> Vec<String> {
    let (compiled, lowered, facts) = analyzed(source);
    let id = find(&compiled, &lowered, name);
    let function = &lowered.module.functions[id];
    let mut out = Vec::new();
    for (block, data) in function.blocks.iter() {
        for (at, inst) in data.insts.iter().enumerate() {
            if let Some(found) = facts.function(id).allocation(block, at) {
                let op = format!("{:?}", inst.op);
                let op = op.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("").to_lowercase();
                let flags: Vec<&str> = [(found.escapes, "escapes"), (found.written, "written"), (found.frozen, "frozen")].into_iter().filter(|(on, _)| *on).map(|(_, flag)| flag).collect();
                out.push(format!("{op}: {}", if flags.is_empty() { "local".to_string() } else { flags.join(" ") }));
            }
        }
    }
    out
}

/// The inputs of the function as `escapes`/`written` flags per position.
fn inputs(source: &str, name: &str) -> Vec<String> {
    let (compiled, lowered, facts) = analyzed(source);
    let id = find(&compiled, &lowered, name);
    let function = &lowered.module.functions[id];
    (0..function.params.len() + function.captures.len())
        .map(|position| {
            let facts = facts.function(id);
            let flags: Vec<&str> = [(facts.input_escapes(position), "escapes"), (facts.input_written(position), "written")].into_iter().filter(|(on, _)| *on).map(|(_, flag)| flag).collect();
            if flags.is_empty() { "untouched".to_string() } else { flags.join(" ") }
        })
        .collect()
}

#[test]
fn a_register_is_last_used_where_it_is_read_for_the_final_time() {
    let got = last_uses("fn f(a: Int, b: Int) -> Int {\n let c = a + b\n return c * 2\n}", "f");
    assert_eq!(got, ["b0.0: r0, r1", "b0.1: r2", "b0.2: r3", "b0.t: r4"]);
}

#[test]
fn a_register_read_by_a_loop_is_not_a_last_use_inside_it() {
    let got = last_uses("fn f(n: Int) -> Int {\n var s = 0\n var i = 0\n while i < n {\n s += i\n i += 1\n }\n return s\n}", "f");
    assert!(got.iter().all(|line| !line.contains("r0")), "{got:?}");
}

#[test]
fn a_register_is_last_used_in_each_branch_that_ends_it() {
    let got = last_uses("fn f(a: Int, c: Bool) -> Int {\n if c {\n return a\n }\n return 0\n}", "f");
    assert_eq!(got, ["b0.t: r1", "b1.t: r0"]);
}

#[test]
fn a_var_parameter_is_read_by_the_return_that_hands_it_back() {
    let got = last_uses("fn bump(var n: Int) {\n n += 1\n}", "bump");
    assert_eq!(got, ["b0.0: r0", "b0.1: r1", "b0.2: r2", "b0.t: r0"]);
}

#[test]
fn an_object_that_stays_inside_does_not_escape() {
    let got = allocations("fn f() -> Int {\n let t = (1, 2)\n return t.0\n}", "f");
    assert_eq!(got, ["tuple: frozen"]);
}

#[test]
fn an_object_that_is_returned_escapes() {
    let got = allocations("fn f() -> (Int, Int) {\n return (1, 2)\n}", "f");
    assert_eq!(got, ["tuple: escapes"]);
}

#[test]
fn an_object_stored_in_a_global_escapes() {
    let got = allocations("var g = (0, 0)\ng = (1, 2)", "<top>");
    assert_eq!(got, ["tuple: escapes"]);
}

#[test]
fn an_argument_escapes_only_when_the_callee_lets_it() {
    let source = "fn keeps(t: (Int, Int)) -> Int {\n let h = spawn { return t.0 }\n return h.await().unwrap()\n}\nfn reads(t: (Int, Int)) -> Int {\n return t.0\n}\nfn a() -> Int {\n let t = (1, 2)\n return keeps(t)\n}\nfn b() -> Int {\n let t = (1, 2)\n return reads(t)\n}";
    assert_eq!(allocations(source, "a"), ["tuple: escapes"]);
    assert_eq!(allocations(source, "b"), ["tuple: frozen"]);
    assert_eq!(inputs(source, "keeps"), ["escapes"]);
    assert_eq!(inputs(source, "reads"), ["untouched"]);
}

#[test]
fn an_argument_escapes_through_the_chain_of_callees() {
    let source = "fn keeps(t: (Int, Int)) -> Int {\n let h = spawn { return t.0 }\n return h.await().unwrap()\n}\nfn passes(t: (Int, Int)) -> Int {\n return keeps(t)\n}\nfn a() -> Int {\n return passes((1, 2))\n}";
    assert_eq!(inputs(source, "passes"), ["escapes"]);
    assert_eq!(allocations(source, "a"), ["tuple: escapes"]);
}

#[test]
fn a_value_returned_from_a_callee_makes_its_argument_escape() {
    let source = "fn same(t: (Int, Int)) -> (Int, Int) {\n return t\n}\nfn a() -> Int {\n let t = same((1, 2))\n return t.0\n}";
    assert_eq!(inputs(source, "same"), ["escapes"]);
    assert_eq!(allocations(source, "a"), ["tuple: escapes"]);
}

#[test]
fn an_object_held_by_an_object_that_escapes_escapes() {
    let got = allocations("fn f() -> List<(Int, Int)> {\n return [(1, 2)]\n}", "f");
    assert_eq!(got, ["tuple: escapes", "list: escapes"]);
}

#[test]
fn a_list_that_is_read_and_never_written_is_frozen() {
    let got = allocations("fn f() -> Int {\n let xs = [1, 2, 3]\n return xs.len()\n}", "f");
    assert_eq!(got, ["list: frozen"]);
}

#[test]
fn a_list_that_is_pushed_to_is_written() {
    let got = allocations("fn f() -> Int {\n var xs = [1]\n xs.push(2)\n return xs.len()\n}", "f");
    assert_eq!(got, ["list: written"]);
}

#[test]
fn an_element_write_marks_the_list_written() {
    let got = allocations("fn f() -> Int {\n var xs = [1, 2]\n xs[0] = 5\n return xs[0]\n}", "f");
    assert_eq!(got, ["list: written"]);
}

#[test]
fn a_list_in_a_list_that_is_written_makes_the_outer_unfrozen() {
    let got = allocations("fn f() -> Int {\n var inner = [1]\n let outer = [inner]\n inner.push(2)\n return outer.len()\n}", "f");
    assert_eq!(got, ["list: written", "list: local"]);
}

#[test]
fn a_var_parameter_is_written_when_it_or_its_contents_change() {
    let source = "fn bump(var n: Int) {\n n += 1\n}\nfn peek(var n: Int) -> Int {\n return n\n}\nfn grow(var xs: List<Int>) {\n xs.push(1)\n}\nfn look(var xs: List<Int>) -> Int {\n return xs.len()\n}\nfn outer(var xs: List<Int>) {\n grow(xs)\n}\nfn outer_look(var xs: List<Int>) -> Int {\n return look(xs)\n}";
    assert_eq!(inputs(source, "bump"), ["written"]);
    assert_eq!(inputs(source, "peek"), ["untouched"]);
    assert_eq!(inputs(source, "grow"), ["written"]);
    assert_eq!(inputs(source, "look"), ["untouched"]);
    assert_eq!(inputs(source, "outer"), ["written"]);
    assert_eq!(inputs(source, "outer_look"), ["untouched"]);
}

#[test]
fn a_cell_shared_with_a_closure_that_stays_inside_does_not_escape() {
    let got = allocations("fn f() -> Int {\n var c = 0\n let inc = || { c += 1 }\n inc()\n inc()\n return c\n}", "f");
    assert_eq!(got, ["newcell: written", "closure: local"]);
}

#[test]
fn a_closure_that_is_returned_takes_its_cell_with_it() {
    let got = allocations("fn f() -> () -> Int {\n var c = 0\n return || {\n c += 1\n return c\n }\n}", "f");
    assert_eq!(got, ["newcell: escapes", "closure: escapes"]);
}

#[test]
fn a_spawned_task_takes_what_it_captures_out_of_the_function() {
    let got = allocations("fn f() -> Int {\n let t = (1, 2)\n let h = spawn { return t.0 }\n return h.await().unwrap()\n}", "f");
    assert_eq!(got[0], "tuple: escapes");
}

#[test]
fn a_type_without_mutable_parts_is_immutable_and_a_list_is_not() {
    let (compiled, lowered, facts) = analyzed("struct P {\n x: Int\n s: String\n}\nclass C {\n var n: Int\n}\nfn f(a: Int, b: String, c: (Int, String), d: P, e: List<Int>, g: C, h: [Int]) {}");
    let id = find(&compiled, &lowered, "f");
    let function = &lowered.module.functions[id];
    let got: Vec<bool> = function.params.iter().map(|&reg| facts.function(id).immutable(reg)).collect();
    assert_eq!(got, [true, true, true, true, false, false, false]);
}

#[test]
fn a_list_nobody_writes_or_keeps_is_immutable_where_it_lives() {
    let (compiled, lowered, facts) = analyzed("fn f() -> Int {\n let xs = [1, 2]\n let ys = xs\n return ys.len()\n}");
    let id = find(&compiled, &lowered, "f");
    let function = &lowered.module.functions[id];
    let held: Vec<bool> = function.regs.iter().filter(|(_, reg)| compiled.program.render(reg.ty, &compiled.interner) == "List<Int>").map(|(id_, _)| facts.function(id).immutable(id_)).collect();
    assert!(!held.is_empty() && held.iter().all(|&frozen| frozen), "{held:?}");
}

#[test]
fn a_recursive_function_settles() {
    let source = "fn go(t: (Int, Int), n: Int) -> Int {\n if n == 0 {\n let h = spawn { return t.0 }\n return h.await().unwrap()\n }\n return go(t, n - 1)\n}\nfn a() -> Int {\n return go((1, 2), 3)\n}";
    assert_eq!(inputs(source, "go"), ["escapes", "untouched"]);
    assert_eq!(allocations(source, "a"), ["tuple: escapes"]);
}

/// Every `.mote` file of `std/tests` as (file name, text), in name order.
fn std_files() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../std/tests");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the std/tests directory").map(|entry| entry.expect("a directory entry").path()).collect();
    paths.sort();
    paths
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "mote"))
        .map(|path| (path.file_name().and_then(|name| name.to_str()).expect("a file name").to_string(), std::fs::read_to_string(&path).expect("a readable file")))
        .collect()
}

#[test]
fn every_function_of_every_std_program_has_consistent_facts() {
    let mut checked = 0;
    for (file, source) in std_files() {
        let mut compiled = compile(&source);
        assert_eq!(compiled.errors, Vec::<String>::new(), "{file}");
        let lowered = lower(&mut compiled.program, &mut compiled.interner);
        let facts = analyze(&lowered.module, &compiled.program, &compiled.interner);
        for (id, function) in lowered.module.functions.iter() {
            let mine = facts.function(id);
            for (block_id, block) in function.blocks.iter() {
                for at in 0..=block.insts.len() {
                    for &reg in mine.last_uses(block_id, at) {
                        let read = if at == block.insts.len() {
                            crate::uses::terminator_reads(&block.term).contains(&reg) || (matches!(block.term, crate::Terminator::Return(_)) && function.out_params.iter().any(|&position| function.params[position] == reg))
                        } else {
                            crate::uses::reads(&block.insts[at].op).contains(&reg)
                        };
                        assert!(read, "{file}: r{} is not read where it is last used", reg.index());
                    }
                }
            }
            checked += 1;
        }
    }
    println!("{checked} functions");
    assert!(checked > 0);
}
