use mote_base::{Interner, Span};
use mote_semantics::{compile, BinaryOp, Conversion, Program, TyId};

use crate::*;

fn span() -> Span {
    Span::new(0, 1)
}

/// The messages `verify` finds in a module of one function built by `build`, which ends with `ret`.
fn problems(ret: TyId, build: impl FnOnce(&mut FunctionBuilder, &mut Program)) -> Vec<String> {
    let mut program = Program::new();
    let mut b = FunctionBuilder::new(FunctionKind::Start, ret);
    build(&mut b, &mut program);
    if !b.is_terminated() {
        b.terminate(Terminator::Return(None));
    }
    let mut module = Module::new();
    module.add(b.finish().expect("closed"));
    verify(&module, &program, &Interner::new()).into_iter().map(|error| error.message).collect()
}

fn reg(operand: Operand) -> RegId {
    match operand {
        Operand::Reg(reg) => reg,
        Operand::Const(_) => panic!("a register"),
    }
}

#[test]
fn a_branch_needs_a_bool() {
    let found = problems(TyId::UNIT, |b, _| {
        let (yes, no) = (b.block(), b.block());
        b.terminate(Terminator::Branch { condition: Operand::Const(Const::Int(1)), then: yes, otherwise: no });
        b.switch_to(yes);
        b.terminate(Terminator::Return(None));
        b.switch_to(no);
        b.terminate(Terminator::Return(None));
    });
    assert_eq!(found, ["branch condition Int(1) is not Bool"]);
}

#[test]
fn arithmetic_needs_one_number_type() {
    let found = problems(TyId::UNIT, |b, _| {
        let a = b.param(TyId::INT, None, false);
        let x = b.param(TyId::FLOAT, None, false);
        b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(a), right: Operand::Reg(x) }, span());
    });
    assert_eq!(found, ["`r2 = add r0, r1`: right r1: Float is not Int"]);
}

#[test]
fn a_comparison_yields_a_bool() {
    let found = problems(TyId::UNIT, |b, _| {
        let a = b.param(TyId::INT, None, false);
        b.value(TyId::INT, Op::Binary { op: BinaryOp::Lt, left: Operand::Reg(a), right: Operand::Const(Const::Int(1)) }, span());
    });
    assert_eq!(found, ["`r1 = lt r0, 1`: result is Int, not Bool"]);
}

#[test]
fn a_constant_must_fit_the_integer_type() {
    let found = problems(TyId::UNIT, |b, _| {
        b.value(TyId::UINT8, Op::Copy(Operand::Const(Const::Int(300))), span());
    });
    assert_eq!(found, ["`r0 = copy 300`: operand Int(300) is not UInt8"]);
}

#[test]
fn a_return_value_fits_the_result_type() {
    let found = problems(TyId::INT, |b, _| {
        let s = b.param(TyId::STRING, None, false);
        b.terminate(Terminator::Return(Some(Operand::Reg(s))));
    });
    assert_eq!(found, ["returned r0: String is not Int"]);
}

#[test]
fn a_function_returning_a_value_cannot_return_nothing() {
    assert_eq!(problems(TyId::INT, |_, _| {}), ["returns nothing from a function returning Int"]);
}

#[test]
fn a_direct_call_fits_the_parameters_and_the_result() {
    let mut module = Module::new();
    let mut callee = FunctionBuilder::new(FunctionKind::Start, TyId::INT);
    let n = callee.param(TyId::INT, None, false);
    callee.terminate(Terminator::Return(Some(Operand::Reg(n))));
    let callee = module.add(callee.finish().expect("closed"));
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    let text = b.param(TyId::STRING, None, false);
    b.value(TyId::BOOL, Op::Call { callee: Callee::Function(callee), args: vec![Operand::Reg(text)], outs: vec![] }, span());
    b.terminate(Terminator::Return(None));
    module.add(b.finish().expect("closed"));
    let found: Vec<String> = verify(&module, &Program::new(), &Interner::new()).into_iter().map(|error| error.message).collect();
    assert_eq!(found, ["`r1 = call f0(r0)`: argument r0: String is not Int", "`r1 = call f0(r0)`: result is Bool, not Int"]);
}

#[test]
fn a_cell_is_made_by_new_cell_and_read_by_load_cell() {
    let found = problems(TyId::UNIT, |b, _| {
        let plain = b.param(TyId::INT, None, false);
        let cell = b.cell(TyId::INT, None);
        b.emit(Some(cell), Op::Copy(Operand::Const(Const::Int(1))), span());
        b.value(TyId::INT, Op::LoadCell(Operand::Reg(plain)), span());
        b.value(TyId::INT, Op::Copy(Operand::Reg(cell)), span());
    });
    assert_eq!(
        found,
        [
            "`r1 = copy 1`: only `NewCell` and the load of a cell slot write a cell",
            "`r2 = load_cell r0`: r0: Int is not a cell",
            "`r3 = copy r1`: operand r1: cell Int is not Int",
        ]
    );
}

#[test]
fn a_cell_holds_the_type_it_was_made_for() {
    let found = problems(TyId::UNIT, |b, _| {
        let cell = b.cell(TyId::INT, None);
        b.emit(Some(cell), Op::NewCell(Operand::Const(Const::Int(0))), span());
        b.emit(None, Op::StoreCell { cell: Operand::Reg(cell), value: Operand::Const(Const::Bool(true)) }, span());
        b.value(TyId::STRING, Op::LoadCell(Operand::Reg(cell)), span());
    });
    assert_eq!(found, ["`store_cell r0, true`: value Bool(true) is not Int", "`r1 = load_cell r0`: loads r0: cell Int into String"]);
}

#[test]
fn a_conversion_keeps_the_type_it_names() {
    let found = problems(TyId::UNIT, |b, program| {
        let optional = program.types.optional(TyId::STRING);
        let n = b.param(TyId::INT, None, false);
        b.value(optional, Op::Convert { conversion: Conversion::ToOptional, operand: Operand::Reg(n) }, span());
        b.value(TyId::INT, Op::Convert { conversion: Conversion::ToUnion, operand: Operand::Reg(n) }, span());
    });
    assert_eq!(found, ["`r1 = to_optional r0`: to_optional does not turn r0: Int into String?","`r2 = to_union r0`: to_union does not turn r0: Int into Int"]);
}

#[test]
fn a_tuple_field_is_inside_the_tuple() {
    let found = problems(TyId::UNIT, |b, program| {
        let pair = program.types.tuple(vec![TyId::INT, TyId::STRING]);
        let p = b.param(pair, None, false);
        b.value(TyId::STRING, Op::TupleField { object: Operand::Reg(p), index: 1 }, span());
        b.value(TyId::INT, Op::TupleField { object: Operand::Reg(p), index: 1 }, span());
        b.value(TyId::INT, Op::TupleField { object: Operand::Reg(p), index: 2 }, span());
    });
    assert_eq!(found, ["`r2 = tuple_field r0.1`: reads element 1 of (Int, String) into Int", "`r3 = tuple_field r0.2`: reads element 2 of (Int, String) into Int"]);
}

#[test]
fn a_switch_value_is_an_integer_and_its_cases_fit() {
    let found = problems(TyId::UNIT, |b, _| {
        let small = b.param(TyId::UINT8, None, false);
        let text = b.param(TyId::STRING, None, false);
        let other = b.block();
        b.terminate(Terminator::Switch { value: Operand::Reg(small), cases: vec![(1, other), (256, other)], default: other });
        b.switch_to(other);
        let end = b.block();
        b.terminate(Terminator::Switch { value: Operand::Reg(text), cases: vec![], default: end });
        b.switch_to(end);
        b.terminate(Terminator::Return(None));
    });
    assert_eq!(found, ["switch case 256 does not fit UInt8", "switch value r1: String is not an integer"]);
}

#[test]
fn a_register_is_written_before_it_is_read() {
    let found = problems(TyId::UNIT, |b, _| {
        let unset = b.reg(TyId::INT, None);
        b.value(TyId::INT, Op::Copy(Operand::Reg(unset)), span());
    });
    assert_eq!(found, ["`r1 = copy r0`: r0 is read before it is written"]);
}

#[test]
fn a_register_written_on_one_path_is_not_written_at_the_join() {
    let found = problems(TyId::UNIT, |b, _| {
        let flag = b.param(TyId::BOOL, None, false);
        let late = b.reg(TyId::INT, None);
        let (set, join) = (b.block(), b.block());
        b.terminate(Terminator::Branch { condition: Operand::Reg(flag), then: set, otherwise: join });
        b.switch_to(set);
        b.emit(Some(late), Op::Copy(Operand::Const(Const::Int(1))), span());
        b.terminate(Terminator::Jump(join));
        b.switch_to(join);
        b.value(TyId::INT, Op::Copy(Operand::Reg(late)), span());
    });
    assert_eq!(found, ["`r2 = copy r1`: r1 is read before it is written"]);
}

#[test]
fn a_loop_keeps_what_was_written_before_it() {
    let found = problems(TyId::UNIT, |b, _| {
        let n = b.reg(TyId::INT, None);
        b.emit(Some(n), Op::Copy(Operand::Const(Const::Int(0))), span());
        let body = b.block();
        b.terminate(Terminator::Jump(body));
        b.switch_to(body);
        let next = b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(n), right: Operand::Const(Const::Int(1)) }, span());
        b.emit(Some(n), Op::Copy(next), span());
        b.terminate(Terminator::Jump(body));
    });
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn an_intrinsic_call_fits_a_declaration_in_std() {
    let mut compiled = compile("");
    let name = compiled.interner.intern("string_len");
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    b.value(TyId::INT, Op::Call { callee: Callee::Intrinsic(name), args: vec![Operand::Const(Const::String(name))], outs: vec![] }, span());
    b.value(TyId::INT, Op::Call { callee: Callee::Intrinsic(name), args: vec![Operand::Const(Const::Int(3))], outs: vec![] }, span());
    b.terminate(Terminator::Return(None));
    let mut module = Module::new();
    module.add(b.finish().expect("closed"));
    let found: Vec<String> = verify(&module, &compiled.program, &compiled.interner).into_iter().map(|error| error.message).collect();
    assert_eq!(found, ["`r1 = call intrinsic string_len(3)`: the arguments and result do not fit a declaration of `string_len`"]);
}

#[test]
fn a_yield_belongs_to_a_generator() {
    let found = problems(TyId::UNIT, |b, _| {
        b.emit(None, Op::Yield(Operand::Const(Const::Int(1))), span());
    });
    assert_eq!(found, ["`yield 1`: yields outside a generator"]);
}

#[test]
fn a_register_holds_a_resolved_type() {
    let found = problems(TyId::UNIT, |b, _| {
        b.reg(TyId::ERROR, None);
    });
    assert_eq!(found, ["r0 has the unresolved type <error>"]);
}

#[test]
fn an_intrinsic_takes_the_arguments_of_its_row() {
    let mut interner = Interner::new();
    let name = interner.intern("string_len");
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    let result = b.value(TyId::INT, Op::Call { callee: Callee::Intrinsic(name), args: vec![], outs: vec![] }, span());
    assert_eq!(reg(result).index(), 0);
    b.terminate(Terminator::Return(None));
    let mut module = Module::new();
    module.add(b.finish().expect("closed"));
    let found: Vec<String> = verify(&module, &Program::new(), &interner).into_iter().map(|error| error.message).collect();
    assert_eq!(found, ["`r0 = call intrinsic string_len()`: `string_len` takes 1 arguments, found 0"]);
}

#[test]
fn an_intrinsic_call_ties_one_type_parameter_across_the_parameters_that_name_it() {
    let mut compiled = compile("");
    let equal = compiled.interner.intern("equal_of");
    let get = compiled.interner.intern("array_get");
    let ints = compiled.program.types.array(TyId::INT);
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    let (n, text, array) = (b.param(TyId::INT, None, false), b.param(TyId::STRING, None, false), b.param(ints, None, false));
    let call = |name, args: Vec<Operand>| Op::Call { callee: Callee::Intrinsic(name), args, outs: vec![] };
    b.value(TyId::BOOL, call(equal, vec![Operand::Reg(n), Operand::Reg(n)]), span());
    b.value(TyId::BOOL, call(equal, vec![Operand::Reg(n), Operand::Reg(text)]), span());
    b.value(TyId::BOOL, call(equal, vec![Operand::Reg(text), Operand::Const(Const::Int(1))]), span());
    b.value(TyId::INT, call(get, vec![Operand::Reg(array), Operand::Const(Const::Int(0))]), span());
    b.value(TyId::STRING, call(get, vec![Operand::Reg(array), Operand::Const(Const::Int(0))]), span());
    b.terminate(Terminator::Return(None));
    let mut module = Module::new();
    module.add(b.finish().expect("closed"));
    let found: Vec<String> = verify(&module, &compiled.program, &compiled.interner).into_iter().map(|error| error.message).collect();
    assert_eq!(
        found,
        [
            "`r4 = call intrinsic equal_of(r0, r1)`: the arguments and result do not fit a declaration of `equal_of`",
            "`r5 = call intrinsic equal_of(r1, 1)`: the arguments and result do not fit a declaration of `equal_of`",
            "`r7 = call intrinsic array_get(r2, 0)`: the arguments and result do not fit a declaration of `array_get`",
        ]
    );
}

/// The messages `verify` finds in a module of a plain function and a resume function with the slots Int and String, built by `build`.
fn frame_problems(build: impl FnOnce(&mut FunctionBuilder)) -> Vec<String> {
    let mut module = Module::new();
    let mut plain = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    plain.terminate(Terminator::Return(None));
    let generator = module.add(plain.finish().expect("closed"));
    let mut b = FunctionBuilder::new(FunctionKind::Resume { generator }, TyId::UNIT);
    b.slot(TyId::INT, false);
    b.slot(TyId::STRING, false);
    build(&mut b);
    b.terminate(Terminator::Return(None));
    module.add(b.finish().expect("closed"));
    verify(&module, &Program::new(), &Interner::new()).into_iter().map(|error| error.message).collect()
}

#[test]
fn a_frame_slot_exists_and_holds_its_type() {
    let found = frame_problems(|b| {
        b.emit(None, Op::FrameStore { slot: 1, value: Operand::Const(Const::Int(1)) }, span());
        b.value(TyId::INT, Op::FrameLoad(1), span());
        b.value(TyId::INT, Op::FrameLoad(5), span());
    });
    assert_eq!(found, ["`frame_store 1, 1`: value Int(1) is not String", "`r0 = frame_load 1`: result is Int, not String", "frame slot 5 does not exist"]);
}

#[test]
fn a_frame_slot_is_for_a_resume_function() {
    let found = problems(TyId::UNIT, |b, _| {
        b.value(TyId::INT, Op::FrameLoad(0), span());
    });
    assert_eq!(found, ["frame slot 0 does not exist"]);
}

#[test]
fn a_generator_stream_is_built_over_a_resume_function() {
    let mut compiled = compile("import { Stream } from std.collections\n");
    let def = compiled.program.lang[&mote_semantics::LangItem::Stream];
    let stream = compiled.program.types.adt(def, vec![TyId::INT]);
    let mut module = Module::new();
    let mut plain = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    plain.terminate(Terminator::Return(None));
    let target = module.add(plain.finish().expect("closed"));
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    b.value(stream, Op::Stream { resume: target, init: vec![] }, span());
    b.terminate(Terminator::Return(None));
    module.add(b.finish().expect("closed"));
    let found: Vec<String> = verify(&module, &compiled.program, &compiled.interner).into_iter().map(|error| error.message).collect();
    assert_eq!(found, ["`r0 = stream f0 []`: f0 is not a resume function"]);
}
