use std::collections::HashSet;

use mote_abi::{CParam, CType, LinkTable, NativeSig};
use mote_base::Interner;

use crate::defs::{DefId, Function, Receiver};
use crate::program::Program;
use crate::tree::{Body, Callee, ExprKind};
use crate::types::{BuiltinTy, FloatTy, IntTy, TyId, TyKind};

/// Why an extern signature cannot be a C signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReason {
    MissingLink,
    Generic,
    Receiver,
    Variadic,
    /// A `var` parameter of a type that has no pointer form, such as a string or an array.
    VarParameter,
    /// `Null` is a result only.
    VoidParameter,
    /// An array is a parameter only.
    ArrayResult,
    /// A `PortEnd` is a parameter only.
    PortResult,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeError {
    /// The parameter at fault; `None` for the result or the function.
    pub param: Option<usize>,
    pub ty: TyId,
    pub reason: NativeReason,
}

/// An extern function the program calls whose signature cannot cross to C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeFailure {
    pub def: DefId,
    pub error: NativeError,
}

fn is_pointer(ty: TyId, program: &Program) -> bool {
    matches!(program.types.kind(ty), TyKind::Builtin { kind: BuiltinTy::Pointer, .. })
}

fn is_port_end(ty: TyId, program: &Program) -> bool {
    matches!(program.types.kind(ty), TyKind::Builtin { kind: BuiltinTy::PortEnd, .. })
}

/// The C type a Mote type becomes as a parameter or as a result.
pub fn c_type(program: &Program, ty: TyId, result: bool) -> Result<CType, NativeReason> {
    let types = &program.types;
    match types.kind(ty) {
        TyKind::Int(int) => Ok(match int {
            IntTy::I8 => CType::I8,
            IntTy::I16 => CType::I16,
            IntTy::I32 => CType::I32,
            IntTy::I64 => CType::I64,
            IntTy::U8 => CType::U8,
            IntTy::U16 => CType::U16,
            IntTy::U32 => CType::U32,
            IntTy::U64 => CType::U64,
        }),
        TyKind::Float(FloatTy::F32) => Ok(CType::F32),
        TyKind::Float(FloatTy::F64) => Ok(CType::F64),
        TyKind::Bool => Ok(CType::U8),
        TyKind::Unit if result => Ok(CType::Void),
        TyKind::Unit => Err(NativeReason::VoidParameter),
        TyKind::String => Ok(if result { CType::StringOut } else { CType::StringIn }),
        TyKind::Array(_) | TyKind::FixedArray(..) if result => Err(NativeReason::ArrayResult),
        TyKind::Array(element) if matches!(types.kind(*element), TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool) => Ok(CType::Array),
        TyKind::FixedArray(element, _) if matches!(types.kind(*element), TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool) => Ok(CType::FixedArray),
        TyKind::Builtin { kind: BuiltinTy::Pointer, .. } => Ok(CType::Pointer),
        TyKind::Builtin { kind: BuiltinTy::PortEnd, .. } => if result { Err(NativeReason::PortResult) } else { Ok(CType::PortEnd) },
        TyKind::Optional(inner) if is_pointer(*inner, program) => Ok(CType::NullablePointer),
        _ => {
            let _ = is_port_end;
            Err(NativeReason::Unsupported)
        }
    }
}

/// The C signature of an `extern` fn, or the first reason it has none.
pub fn signature(function: &Function, program: &Program) -> Result<NativeSig, NativeError> {
    let fail = |param, ty, reason| NativeError { param, ty, reason };
    let Some(link) = function.link else {
        return Err(fail(None, TyId::UNIT, NativeReason::MissingLink));
    };
    if !function.generics.is_empty() {
        return Err(fail(None, TyId::UNIT, NativeReason::Generic));
    }
    if function.receiver != Receiver::None {
        return Err(fail(None, TyId::UNIT, NativeReason::Receiver));
    }
    let mut params = Vec::new();
    for (index, param) in function.params.iter().enumerate() {
        if param.variadic {
            return Err(fail(Some(index), param.ty, NativeReason::Variadic));
        }
        let ty = c_type(program, param.ty, false).map_err(|reason| fail(Some(index), param.ty, reason))?;
        if param.is_var {
            if !ty.can_be_by_ref() {
                return Err(fail(Some(index), param.ty, NativeReason::VarParameter));
            }
            params.push(CParam::by_ref(ty));
        } else {
            params.push(CParam::value(ty));
        }
    }
    let ret = c_type(program, function.ret, true).map_err(|reason| fail(None, function.ret, reason))?;
    Ok(NativeSig { params, ret, blocking: link.blocking })
}

impl Program {
    /// The external library functions the instances, the globals and the top-level code call or name, in the order first seen.
    pub fn linked_libraries(&self) -> Vec<DefId> {
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        let mut bodies: Vec<&Body> = self.order.iter().map(|instance| &self.bodies[instance]).collect();
        bodies.extend(self.defs.iter().filter_map(|(def, _)| self.global_inits.get(&def)));
        bodies.extend(self.modules.iter().filter_map(|(module, _)| self.top_level.get(&module)));
        for body in bodies {
            for (_, expr) in body.exprs.iter() {
                let def = match &expr.kind {
                    ExprKind::Call { callee: Callee::Function(function), .. } | ExprKind::Function(function) => function.def,
                    _ => continue,
                };
                let extern_fn = self.defs[def].function().is_some_and(|function| function.link.is_some());
                if extern_fn && seen.insert(def) {
                    found.push(def);
                }
            }
        }
        found
    }

    /// What the program loads at start-up, and the external calls that cannot be loaded.
    pub fn link_table(&self, interner: &Interner) -> (LinkTable, Vec<NativeFailure>) {
        let mut table = LinkTable::new();
        let mut failures = Vec::new();
        for def in self.linked_libraries() {
            let item = &self.defs[def];
            let function = item.function().expect("a linked function is a function");
            match signature(function, self) {
                Ok(sig) => {
                    let link = function.link.expect("a valid signature has a link");
                    table.add(interner.resolve(link.library), interner.resolve(link.symbol), sig, item.span);
                }
                Err(error) => failures.push(NativeFailure { def, error }),
            }
        }
        (table, failures)
    }
}

#[cfg(test)]
mod tests {
    use mote_abi::Scalar;

    use super::*;
    use crate::defs::test_support::Fixture;
    use crate::defs::{DefKind, GenericParam, Link, Param};
    use crate::program::Instance;
    use crate::tree::{Body, FnRef};

    fn native(fixture: &mut Fixture, library: &str, params: Vec<(TyId, bool)>, ret: TyId, blocking: bool) -> Function {
        let params = params
            .into_iter()
            .enumerate()
            .map(|(index, (ty, is_var))| Param { name: fixture.interner.intern(&format!("p{index}")), ty, is_var, default: None, variadic: false })
            .collect();
        let link = Link { library: fixture.interner.intern(library), symbol: fixture.interner.intern("symbol"), blocking };
        Function { generics: Vec::new(), params, receiver: Receiver::None, ret, has_body: false, link: Some(link) }
    }

    fn main_function() -> Function {
        Function { generics: vec![], params: vec![], receiver: Receiver::None, ret: TyId::UNIT, has_body: true, link: None }
    }

    /// A program with `Pointer<Db>`.
    fn with_pointer() -> (Fixture, TyId) {
        let mut fixture = Fixture::new();
        let db = fixture.def("Db");
        let db_ty = fixture.program.types.adt_types(db, vec![]);
        let ty = fixture.program.types.builtin(BuiltinTy::Pointer, vec![db_ty]);
        (fixture, ty)
    }

    #[test]
    fn numbers_bool_and_unit_map_to_their_c_types() {
        let program = Program::new();
        assert_eq!(c_type(&program, TyId::INT, false), Ok(CType::I64));
        assert_eq!(c_type(&program, TyId::UINT8, true), Ok(CType::U8));
        assert_eq!(c_type(&program, TyId::INT16, false), Ok(CType::I16));
        assert_eq!(c_type(&program, TyId::FLOAT32, false), Ok(CType::F32));
        assert_eq!(c_type(&program, TyId::FLOAT, true), Ok(CType::F64));
        assert_eq!(c_type(&program, TyId::BOOL, false), Ok(CType::U8));
        assert_eq!(c_type(&program, TyId::UNIT, true), Ok(CType::Void));
        assert_eq!(c_type(&program, TyId::UNIT, false), Err(NativeReason::VoidParameter));
    }

    #[test]
    fn strings_go_in_by_copy_and_come_out_by_copy() {
        let program = Program::new();
        assert_eq!(c_type(&program, TyId::STRING, false), Ok(CType::StringIn));
        assert_eq!(c_type(&program, TyId::STRING, true), Ok(CType::StringOut));
    }

    #[test]
    fn arrays_of_scalars_are_parameters_only() {
        let mut program = Program::new();
        let bytes = program.types.array(TyId::UINT8);
        let strings = program.types.array(TyId::STRING);
        assert_eq!(c_type(&program, bytes, false), Ok(CType::Array));
        assert_eq!(c_type(&program, bytes, true), Err(NativeReason::ArrayResult));
        assert_eq!(c_type(&program, strings, false), Err(NativeReason::Unsupported));
    }

    #[test]
    fn a_pointer_is_a_pointer_and_an_optional_pointer_may_be_null() {
        let (mut fixture, pointer) = with_pointer();
        let maybe = fixture.program.types.optional(pointer);
        assert_eq!(c_type(&fixture.program, pointer, false), Ok(CType::Pointer));
        assert_eq!(c_type(&fixture.program, pointer, true), Ok(CType::Pointer));
        assert_eq!(c_type(&fixture.program, maybe, true), Ok(CType::NullablePointer));
        assert_eq!(c_type(&fixture.program, maybe, false), Ok(CType::NullablePointer));
    }

    #[test]
    fn a_port_end_crosses_as_a_parameter_only() {
        let mut fixture = Fixture::new();
        let ty = fixture.program.types.builtin(BuiltinTy::PortEnd, vec![]);
        assert_eq!(c_type(&fixture.program, ty, false), Ok(CType::PortEnd));
        assert_eq!(c_type(&fixture.program, ty, true), Err(NativeReason::PortResult));
        let function = native(&mut fixture, "pane", vec![(ty, false)], TyId::UNIT, false);
        assert_eq!(signature(&function, &fixture.program).unwrap().params, [CParam::value(CType::PortEnd)]);
        let by_ref = native(&mut fixture, "pane", vec![(ty, true)], TyId::UNIT, false);
        assert_eq!(signature(&by_ref, &fixture.program).unwrap_err().reason, NativeReason::VarParameter);
    }

    #[test]
    fn a_class_that_is_not_the_pointer_lang_item_does_not_cross() {
        let mut fixture = Fixture::new();
        let class = fixture.def("Db");
        let ty = fixture.program.types.adt_types(class, vec![]);
        let maybe = fixture.program.types.optional(ty);
        assert_eq!(c_type(&fixture.program, ty, false), Err(NativeReason::Unsupported));
        assert_eq!(c_type(&fixture.program, maybe, false), Err(NativeReason::Unsupported));
    }

    #[test]
    fn other_types_do_not_cross() {
        let mut program = Program::new();
        let optional = program.types.optional(TyId::INT);
        let union = program.types.union(&[TyId::INT, TyId::STRING]);
        let tuple = program.types.tuple(vec![TyId::INT]);
        for ty in [optional, union, tuple, TyId::ERROR] {
            assert_eq!(c_type(&program, ty, false), Err(NativeReason::Unsupported));
        }
    }

    #[test]
    fn fixed_arrays_of_scalars_cross_including_by_ref() {
        let mut fixture = Fixture::new();
        let fixed = fixture.program.types.fixed_array(TyId::UINT8, 32);
        let bad_fixed = fixture.program.types.fixed_array(TyId::STRING, 4);
        assert_eq!(c_type(&fixture.program, fixed, false), Ok(CType::FixedArray));
        assert_eq!(c_type(&fixture.program, fixed, true), Err(NativeReason::ArrayResult));
        assert_eq!(c_type(&fixture.program, bad_fixed, false), Err(NativeReason::Unsupported));
        let by_ref = native(&mut fixture, "crypto", vec![(fixed, true)], TyId::INT32, false);
        let sig = signature(&by_ref, &fixture.program).unwrap();
        assert_eq!(sig.params, [CParam::by_ref(CType::FixedArray)]);
        assert!(sig.is_valid());
    }

    #[test]
    fn a_signature_collects_params_blocking_and_result() {
        let mut fixture = Fixture::new();
        let bytes = fixture.program.types.array(TyId::UINT8);
        let function = native(&mut fixture, "z", vec![(TyId::STRING, false), (bytes, false)], TyId::INT32, true);
        let sig = signature(&function, &fixture.program).unwrap();
        assert_eq!(sig.params, [CParam::value(CType::StringIn), CParam::value(CType::Array)]);
        assert_eq!(sig.ret, CType::I32);
        assert!(sig.blocking);
        assert_eq!(sig.param_scalars(), [Scalar::I64, Scalar::I64, Scalar::I64]);
    }

    #[test]
    fn a_var_parameter_is_passed_by_pointer() {
        let (mut fixture, pointer) = with_pointer();
        let maybe = fixture.program.types.optional(pointer);
        let function = native(&mut fixture, "sqlite3", vec![(TyId::STRING, false), (maybe, true), (TyId::INT32, true)], TyId::INT32, false);
        let sig = signature(&function, &fixture.program).unwrap();
        assert_eq!(sig.params, [CParam::value(CType::StringIn), CParam::by_ref(CType::NullablePointer), CParam::by_ref(CType::I32)]);
        assert!(sig.is_valid());
    }

    #[test]
    fn a_var_string_or_array_has_no_pointer_form() {
        let mut fixture = Fixture::new();
        let bytes = fixture.program.types.array(TyId::UINT8);
        for ty in [TyId::STRING, bytes] {
            let function = native(&mut fixture, "z", vec![(ty, true)], TyId::UNIT, false);
            let error = signature(&function, &fixture.program).unwrap_err();
            assert_eq!((error.param, error.reason), (Some(0), NativeReason::VarParameter));
        }
    }

    #[test]
    fn a_signature_names_the_first_thing_wrong() {
        let mut fixture = Fixture::new();
        let mut no_link = native(&mut fixture, "m", vec![], TyId::UNIT, false);
        no_link.link = None;
        assert_eq!(signature(&no_link, &fixture.program).unwrap_err().reason, NativeReason::MissingLink);

        let mut generic = native(&mut fixture, "m", vec![], TyId::UNIT, false);
        generic.generics.push(GenericParam::Type { name: fixture.interner.intern("T"), bounds: Vec::new() });
        assert_eq!(signature(&generic, &fixture.program).unwrap_err().reason, NativeReason::Generic);

        let mut method = native(&mut fixture, "m", vec![], TyId::UNIT, false);
        method.receiver = Receiver::Read;
        assert_eq!(signature(&method, &fixture.program).unwrap_err().reason, NativeReason::Receiver);

        let mut variadic = native(&mut fixture, "m", vec![(TyId::INT, false)], TyId::UNIT, false);
        variadic.params[0].variadic = true;
        assert_eq!(signature(&variadic, &fixture.program).unwrap_err().reason, NativeReason::Variadic);

        let union = fixture.program.types.union(&[TyId::INT, TyId::STRING]);
        let bad_result = native(&mut fixture, "m", vec![], union, false);
        let error = signature(&bad_result, &fixture.program).unwrap_err();
        assert_eq!((error.param, error.ty, error.reason), (None, union, NativeReason::Unsupported));
    }

    fn call(body: &mut Body, def: DefId) {
        let span = mote_base::Span::new(0, 1);
        let call = body.expr(ExprKind::Call { callee: Callee::Function(FnRef { def, args: vec![] }), args: vec![] }, TyId::UNIT, span);
        let stmt = body.stmt(crate::tree::StmtKind::Expr(call), span);
        body.block.stmts.push(stmt);
    }

    #[test]
    fn only_natives_the_program_calls_are_linked() {
        let mut fixture = Fixture::new();
        let sin = native(&mut fixture, "m", vec![(TyId::FLOAT, false)], TyId::FLOAT, false);
        let puts = native(&mut fixture, "c", vec![(TyId::STRING, false)], TyId::INT32, false);
        let unused = native(&mut fixture, "z", vec![], TyId::UNIT, false);
        let sin = fixture.add("sin", DefKind::Function(sin));
        let puts = fixture.add("puts", DefKind::Function(puts));
        let _unused = fixture.add("unused", DefKind::Function(unused));
        let main = fixture.add("main", DefKind::Function(main_function()));
        let mut body = Body::new();
        call(&mut body, puts);
        call(&mut body, sin);
        call(&mut body, puts);
        call(&mut body, main);
        fixture.program.add_body(Instance { def: main, args: vec![] }, body);
        assert_eq!(fixture.program.linked_libraries(), [puts, sin]);
        let (table, failures) = fixture.program.link_table(&fixture.interner);
        assert!(failures.is_empty());
        assert_eq!(table.libraries, ["c", "m"]);
        assert_eq!(table.functions.len(), 2);
        assert_eq!(table.functions[0].sig.params, [CParam::value(CType::StringIn)]);
    }

    #[test]
    fn a_native_a_global_or_top_level_code_calls_is_linked() {
        let mut fixture = Fixture::new();
        let first = native(&mut fixture, "m", vec![(TyId::INT, false)], TyId::INT, false);
        let first = fixture.add("first", DefKind::Function(first));
        let second = native(&mut fixture, "n", vec![(TyId::INT, false)], TyId::INT, false);
        let second = fixture.add("second", DefKind::Function(second));
        let global = fixture.def("g");
        let mut body = Body::new();
        call(&mut body, first);
        fixture.program.global_inits.insert(global, body);
        let module = fixture.program.modules.iter().next().map(|(id, _)| id).expect("a module");
        let mut body = Body::new();
        call(&mut body, second);
        fixture.program.top_level.insert(module, body);
        assert_eq!(fixture.program.linked_libraries(), [first, second]);
    }

    #[test]
    fn a_called_native_with_a_bad_signature_is_reported() {
        let mut fixture = Fixture::new();
        let union = fixture.program.types.union(&[TyId::INT, TyId::STRING]);
        let bad = native(&mut fixture, "m", vec![(union, false)], TyId::UNIT, false);
        let bad = fixture.add("bad", DefKind::Function(bad));
        let main = fixture.add("main", DefKind::Function(main_function()));
        let mut body = Body::new();
        call(&mut body, bad);
        fixture.program.add_body(Instance { def: main, args: vec![] }, body);
        let (table, failures) = fixture.program.link_table(&fixture.interner);
        assert!(table.functions.is_empty());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].def, bad);
        assert_eq!(failures[0].error.reason, NativeReason::Unsupported);
    }
}
