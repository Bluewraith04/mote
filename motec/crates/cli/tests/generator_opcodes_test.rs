
use isa::encoding::*;
use isa::opcode::Opcode::*;
use runtime::{CodeObject, Runtime};

fn strings() -> Vec<String> {
    vec!["kept".into(), "junk".into()]
}

#[test]
fn test_a_suspended_generator_window_survives_collections() {
    let gen_code = vec![
        encode_none(MKGEN),
        encode_newstr(1, 0),
        encode_ri(LOADI, 2, 0),
        encode_r3(LT, 3, 2, 0),
        encode_jc(JMPIFNOT, 3, 6),
        encode_newstr(4, 1),
        encode_r2(YIELD, 2, 0),
        encode_ri(LOADI, 5, 1),
        encode_r3(ADD, 2, 2, 5),
        encode_ju(JMP, -6),
        encode_r2(YIELD, 1, 0),
        encode_ret(0),
    ];
    let main = vec![
        encode_ri(LOADI, 2, 400),
        encode_call(1, 1),
        encode_ri(LOADI, 6, 0),
        encode_r3(RESUME, 2, 1, 3),
        encode_jc(JMPIFNOT, 3, 4),
        encode_newstr(7, 1),
        encode_r2(MOVE, 6, 2),
        encode_ju(JMP, -4),
        encode_ret(6),
    ];
    let mut rt = Runtime::new(vec![
        CodeObject::new(main, vec![], 10, 0).with_string_table(strings()),
        CodeObject::new(gen_code, vec![], 8, 1).with_string_table(strings()),
    ]);
    rt.set_heap(Box::new(gc::GCController::new(gc::GCConfig::mark_sweep().with_threshold(2048))));
    let task = rt.run_entry().unwrap();
    assert!(rt.gc_stats().map_or(0, |s| s.collections) > 0, "the program was meant to collect");
    assert!(ffi::builtins::render(&task.registers[0]).contains("kept"));
}

#[test]
fn test_clone_refuses_a_generator() {
    let gen_code = vec![encode_none(MKGEN)];
    let main = vec![encode_call(1, 1), encode_callnative(2, 48, 1), encode_ret(2)];
    let mut rt = Runtime::new(vec![
        CodeObject::new(main, vec![], 6, 0),
        CodeObject::new(gen_code, vec![], 4, 0),
    ]);
    ffi::builtins::install(&mut rt);
    let err = rt.run_entry().unwrap_err();
    assert!(err.contains("cannot clone a generator"), "got: {err}");
}
