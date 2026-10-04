//! The per-program native table: builtins are called by name, resolved at load.

use std::collections::HashSet;

use compiler::CompiledProgram;
use runtime::Runtime;

const SOURCE: &str = "let xs = [3, 1, 2]\nxs.push(4)\nprint(xs.len())\nreturn xs.len()\n";

fn compile() -> CompiledProgram {
    compiler::Compiler::compile(SOURCE, "<native-table>").expect("compiles")
}

#[test]
fn compiled_program_lists_the_builtins_it_calls() {
    let table = compile().native_table;
    assert!(table.contains(&"print".to_string()), "{table:?}");
    assert!(table.len() < 20, "only the natives the program uses: {table:?}");
}

#[test]
fn table_round_trips_through_bytes() {
    let program = compile();
    let restored = CompiledProgram::from_bytes(&program.to_bytes()).unwrap();
    assert_eq!(restored.native_table, program.native_table);
}

#[test]
fn builtin_names_are_unique() {
    let reg = ffi::builtins::registry();
    let mut seen = HashSet::new();
    for idx in 0.. {
        let Some(entry) = reg.get(idx) else { break };
        assert!(seen.insert(entry.name.clone()), "duplicate builtin name {}", entry.name);
    }
}

#[test]
fn unknown_native_fails_at_load_naming_it() {
    let mut rt = Runtime::new(vec![]);
    ffi::builtins::install(&mut rt);
    let err = rt.set_native_table(&["print".into(), "no_such_native".into()]).unwrap_err();
    assert!(err.contains("no_such_native") && !err.contains("print"), "{err}");
}

#[test]
fn compiled_program_runs_through_the_table() {
    let program = compile();
    let mut rt = Runtime::with_types(program.code_objects, program.type_descriptors);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&program.native_table).unwrap();
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(4));
}

#[test]
fn every_builtin_interface_declaration_names_a_registry_native() {
    let reg = ffi::builtins::registry();
    let decls = compiler::builtin_natives::decls();
    assert!(decls.len() > 25, "the interface file parsed: {}", decls.len());
    for decl in decls {
        if matches!(decl.name.as_str(), "__task_any" | "__task_pin" | "assert" | "assert_eq") {
            continue;
        }
        let name = compiler::builtin_natives::registry_name(&decl.name);
        assert!(reg.get_by_name(name).is_some(), "`{}` declares `{name}`, which the registry lacks", decl.name);
    }
}

#[test]
fn intrinsic_names_are_unique_and_round_trip() {
    use runtime::handlers::sched_intrinsics::{intrinsic_id, INTRINSIC_NAMES};
    for (i, name) in INTRINSIC_NAMES.iter().enumerate() {
        assert_eq!(intrinsic_id(name), Some(i as u8));
    }
    assert_eq!(intrinsic_id("no.such"), None);
}
