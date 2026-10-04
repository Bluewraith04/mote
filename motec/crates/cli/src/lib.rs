//! The `mote` command-line tool, plus a textual bytecode assembler (`.masm`) and the runtime limit and memory-report helpers.
/// Tokens of the `.masm` assembler.
pub mod token;
/// Syntax tree of the `.masm` assembler.
pub mod ast;
/// Parser of the `.masm` assembler.
pub mod parser;
/// Assembles `.masm` text into a compiled program.
pub mod assembler;
pub mod builtins;
pub mod limits;
pub mod memreport;
pub mod memstats;
pub mod standalone;

pub use assembler::{Assembler, AssembledProgram};
pub use parser::Parser;
pub use token::Tokenizer;

/// Assembles `.masm` source.
pub fn assemble(source: &str) -> Result<AssembledProgram, String> {
    let mut tokenizer = Tokenizer::new(source);
    let tokens = tokenizer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program()?;
    Assembler::assemble(program)
}

/// Compiles `.mote` source and runs it, returning the finished runtime and main task.
pub fn run_source(source: &str) -> Result<(runtime::Runtime, runtime::TaskContext), String> {
    run_source_with_gc(source, gc::GCConfig::default())
}

/// Like [`run_source`], with an explicit collector configuration.
pub fn run_source_with_gc(source: &str, gc: gc::GCConfig) -> Result<(runtime::Runtime, runtime::TaskContext), String> {
    let assembled = assemble(source)?;
    let mut rt = runtime::Runtime::with_types(assembled.code_objects, assembled.type_descriptors);
    builtins::install(&mut rt);
    rt.set_heap(gc::build_gc_engine(&gc));
    let task = rt.run_entry()?;
    Ok((rt, task))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_simple_arithmetic() {
        let src = r#"
        .func @main regs=8 params=0
            LOADI r0, 15
            LOADI r1, 30
            ADD r2, r0, r1
            HALT
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[2].as_int(), Some(45));
    }

    #[test]
    fn test_assemble_loop_with_labels() {
        let src = r#"
        .func @main regs=8 params=0
            LOADI r0, 5     ; count
            LOADI r1, 0     ; sum
            LOADI r2, 1     ; step
            LOADI r4, 0     ; zero comparator

        loop_start:
            EQ r3, r0, r4
            JMPIF r3, loop_end
            ADD r1, r1, r0
            SUB r0, r0, r2
            JMP loop_start

        loop_end:
            HALT
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[1].as_int(), Some(15));
        assert_eq!(task.registers[0].as_int(), Some(0));
    }

    #[test]
    fn test_assemble_function_call_and_recursion() {
        let src = r#"
        .func @main regs=8 params=0
            LOADI r2, 5
            CALL r1, @fact
            HALT
        .end

        .func @fact regs=8 params=1
            LOADI r1, 1
            LE r2, r0, r1
            JMPIFNOT r2, recursive_step
            RET r1

        recursive_step:
            SUB r5, r0, r1
            CALL r4, @fact
            MUL r5, r0, r4
            RET r5
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[1].as_int(), Some(120));
    }

    #[test]
    fn test_assemble_objects_and_fields() {
        let src = r#"
        .type @Point id=100 fields=x,y

        .func @main regs=8 params=0
            NEWOBJ r0, @Point
            LOADI r1, 25
            LOADI r2, 75
            SETFIELD r0, 0, r1
            SETFIELD r0, 1, r2
            GETFIELD r3, r0, 0
            GETFIELD r4, r0, 1
            ADD r5, r3, r4
            TYPEOF r6, r0
            HALT
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[5].as_int(), Some(100));
        assert_eq!(task.registers[6].as_int(), Some(100));
    }

    #[test]
    fn test_assemble_large_integer_auto_spill() {
        let src = r#"
        .func @main regs=8 params=0
            LOADI r0, 1000000000     ; 1 Billion (exceeds i16 max 32767)
            LOADI r1, -500000000     ; -500 Million (exceeds i16 min -32768)
            ADD r2, r0, r1
            HALT
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[0].as_int(), Some(1000000000));
        assert_eq!(task.registers[1].as_int(), Some(-500000000));
        assert_eq!(task.registers[2].as_int(), Some(500000000));
    }

    #[test]
    fn test_callnative_dispatches_to_cli_builtins() {
        let src = r#"
        .func @main regs=8 params=0
            LOADI r0, 7
            LOADI r1, 40
            CALLNATIVE r2, 5, r0
            LOADI r3, 12
            CALLNATIVE r4, 3, r3     ; int_abs(12) -> 12
            ADD r5, r2, r4           ; 52
            HALT
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[2].as_int(), Some(40));
        assert_eq!(task.registers[5].as_int(), Some(52));
    }

    #[test]
    fn test_callnative_assert_builtin_propagates_error() {
        let src = r#"
        .func @main regs=4 params=0
            LOADI r0, 0
            CALLNATIVE r1, 2, r0
            HALT
        .end
        "#;
        let err = run_source(src).err().expect("expected a runtime error");
        assert!(err.contains("assertion failed"), "got: {err}");
    }

    #[test]
    fn test_assemble_arena_allocation_and_scope() {
        let src = r#"
        .type @Node id=50 fields=val,next

        .func @main regs=8 params=0
            ENTERARENA
            ARENAALLOC r0, @Node
            LOADI r1, 42
            SETFIELD r0, 0, r1
            GETFIELD r2, r0, 0
            TYPEOF r3, r0
            EXITARENA
            HALT
        .end
        "#;
        let (_rt, task) = run_source(src).unwrap();
        assert_eq!(task.registers[2].as_int(), Some(42));
        assert_eq!(task.registers[3].as_int(), Some(50));
    }
}

/// The `mote` release, from the `cli` crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Environment variable overriding the worker count.
pub const WORKERS_ENV: &str = "MOTE_WORKERS";

/// How many scheduler workers `mote run` and a bundled executable use: `--workers N` (`flag`), else `MOTE_WORKERS` (`env`), else the number of cores.
pub fn resolve_workers(flag: Option<&str>, env: Option<&str>) -> Result<usize, String> {
    for (source, raw) in [("--workers", flag), (WORKERS_ENV, env)] {
        if let Some(raw) = raw {
            return match raw.trim().parse::<usize>() {
                Ok(n) if n >= 1 => Ok(n),
                _ => Err(format!("{source} must be a whole number of at least 1, got '{raw}'")),
            };
        }
    }
    Ok(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1))
}

#[cfg(test)]
mod worker_tests {
    use super::resolve_workers;

    #[test]
    fn flag_beats_env_beats_discovery() {
        assert_eq!(resolve_workers(Some("3"), Some("5")), Ok(3));
        assert_eq!(resolve_workers(None, Some("5")), Ok(5));
        assert!(resolve_workers(None, None).unwrap() >= 1);
    }

    #[test]
    fn rejects_zero_and_junk() {
        assert!(resolve_workers(Some("0"), None).is_err());
        assert!(resolve_workers(None, Some("many")).is_err());
    }
}
