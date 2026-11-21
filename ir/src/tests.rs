use mote_base::{Interner, Span};
use mote_semantics::{BinaryOp, Program, TyId};

use crate::*;

fn span() -> Span {
    Span::new(0, 1)
}

/// `fn(a, b) -> Int { if a < b { return a + b } return 0 }`
fn sum_if_less() -> Function {
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::INT);
    let a = b.param(TyId::INT, None, false);
    let c = b.param(TyId::INT, None, false);
    let less = b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: Operand::Reg(a), right: Operand::Reg(c) }, span());
    let yes = b.block();
    let no = b.block();
    b.terminate(Terminator::Branch { condition: less, then: yes, otherwise: no });
    b.switch_to(yes);
    let sum = b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(a), right: Operand::Reg(c) }, span());
    b.terminate(Terminator::Return(Some(sum)));
    b.switch_to(no);
    b.terminate(Terminator::Return(Some(Operand::Const(Const::Int(0)))));
    b.finish().expect("every block ends")
}

#[test]
fn a_function_prints_its_registers_and_blocks() {
    let program = Program::new();
    let interner = Interner::new();
    let text = print_function(&sum_if_less(), &program, &interner);
    let expected = "fn start(r0, r1) -> Int\n  r0: Int\n  r1: Int\n  r2: Bool\n  r3: Int\n  b0:\n    r2 = lt r0, r1\n    branch r2, b1, b2\n  b1:\n    r3 = add r0, r1\n    return r3\n  b2:\n    return 0\n";
    assert_eq!(text, expected);
}

#[test]
fn a_builder_reports_the_first_block_without_a_terminator() {
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    let open = b.block();
    b.terminate(Terminator::Jump(open));
    let Err(found) = b.finish() else { panic!("a block is open") };
    assert_eq!(found, open);
}

#[test]
#[should_panic(expected = "emit into a terminated block")]
fn a_terminated_block_takes_no_more_instructions() {
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    b.terminate(Terminator::Return(None));
    b.emit(None, Op::ScopeBegin, span());
}

#[test]
fn a_var_parameter_is_recorded_for_copy_out() {
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    b.param(TyId::INT, None, false);
    b.param(TyId::INT, None, true);
    b.terminate(Terminator::Return(None));
    let function = b.finish().expect("closed");
    assert_eq!(function.out_params, [1]);
}

#[test]
fn a_well_formed_module_verifies() {
    let mut module = Module::new();
    let callee = module.add(sum_if_less());
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::INT);
    let result = b.value(TyId::INT, Op::Call { callee: Callee::Function(callee), args: vec![Operand::Const(Const::Int(1)), Operand::Const(Const::Int(2))], outs: vec![] }, span());
    b.terminate(Terminator::Return(Some(result)));
    module.add(b.finish().expect("closed"));
    assert_eq!(verify(&module, &Program::new(), &Interner::new()), []);
}

#[test]
fn verify_finds_a_wrong_argument_count_and_a_missing_out() {
    let mut module = Module::new();
    let mut callee = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    callee.param(TyId::INT, None, true);
    callee.terminate(Terminator::Return(None));
    let callee = module.add(callee.finish().expect("closed"));
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    b.emit(None, Op::Call { callee: Callee::Function(callee), args: vec![], outs: vec![] }, span());
    b.terminate(Terminator::Return(None));
    let caller = module.add(b.finish().expect("closed"));
    let messages: Vec<_> = verify(&module, &Program::new(), &Interner::new()).into_iter().map(|error| (error.function, error.message)).collect();
    assert_eq!(messages, [(caller, "f0 takes 1 arguments, found 0".to_string()), (caller, "f0 copies back 1 values, found 0".to_string())]);
}

#[test]
fn verify_finds_a_register_from_another_function() {
    let mut module = Module::new();
    let mut other = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    other.reg(TyId::INT, None);
    let foreign = other.reg(TyId::INT, None);
    other.terminate(Terminator::Return(None));
    module.add(other.finish().expect("closed"));
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    b.terminate(Terminator::Return(Some(Operand::Reg(foreign))));
    let broken = module.add(b.finish().expect("closed"));
    assert_eq!(verify(&module, &Program::new(), &Interner::new()), [VerifyError { function: broken, message: "r1 does not exist".to_string() }]);
}

#[test]
fn a_closure_carries_exactly_the_captures_it_declares() {
    let mut module = Module::new();
    let mut inner = FunctionBuilder::new(FunctionKind::Start, TyId::INT);
    let captured = inner.capture(TyId::INT, None, false);
    inner.terminate(Terminator::Return(Some(Operand::Reg(captured))));
    let inner = module.add(inner.finish().expect("closed"));
    let mut outer = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    let mut program = Program::new();
    let function = program.types.function(vec![], TyId::INT, false);
    outer.value(function, Op::Closure { func: inner, captures: vec![] }, span());
    outer.terminate(Terminator::Return(None));
    let outer = module.add(outer.finish().expect("closed"));
    let errors = verify(&module, &program, &Interner::new());
    assert_eq!(errors, [VerifyError { function: outer, message: "f0 carries 1 captures, found 0".to_string() }]);
}

#[test]
fn a_switch_names_every_target() {
    let mut b = FunctionBuilder::new(FunctionKind::Start, TyId::UNIT);
    let (one, two, other) = (b.block(), b.block(), b.block());
    let term = Terminator::Switch { value: Operand::Const(Const::Int(1)), cases: vec![(0, one), (1, two)], default: other };
    assert_eq!(term.targets(), [one, two, other]);
}
