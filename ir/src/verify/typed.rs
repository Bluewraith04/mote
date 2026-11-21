//! The typed half of `verify`: operands, results, calls and terminators against the types of the registers.

use mote_semantics::{BinaryOp, BuiltinTy, ConstArg, Conversion, DefId, DefKind, Enum, Function as Declared, GenericArg, Payload, Record, TyId, TyKind, Types, UnaryOp};

use super::Check;
use crate::intrinsics;
use crate::ir::{Callee, Const, FrameSlot, FuncId, Function, FunctionKind, Inst, Op, Operand, RegId, Terminator};
use crate::print::{conversion_name, print_inst};

impl<'a> Check<'a> {
    fn types(&self) -> &'a Types {
        let program = self.program;
        &program.types
    }

    fn kind(&self, ty: TyId) -> &'a TyKind {
        self.types().kind(ty)
    }

    fn name(&self, ty: TyId) -> String {
        self.program.render(ty, self.interner)
    }

    /// Reports a type error, naming the instruction being checked.
    pub(super) fn problem(&mut self, message: String) {
        let message = match self.inst {
            Some(inst) => format!("`{}`: {message}", print_inst(inst, self.program, self.interner)),
            None => message,
        };
        self.fail(message);
    }

    fn same(&self, a: TyId, b: TyId) -> bool {
        a == b || a == TyId::ERROR || b == TyId::ERROR
    }

    fn is_error(&self, ty: TyId) -> bool {
        ty == TyId::ERROR
    }

    fn is_int(&self, ty: TyId) -> bool {
        matches!(self.kind(ty), TyKind::Int(_) | TyKind::Error)
    }

    fn is_number(&self, ty: TyId) -> bool {
        matches!(self.kind(ty), TyKind::Int(_) | TyKind::Float(_) | TyKind::Error)
    }

    fn is_bool(&self, ty: TyId) -> bool {
        matches!(self.kind(ty), TyKind::Bool | TyKind::Error)
    }

    /// The type a register holds; `None` for a constant or a register the function lacks.
    fn operand_ty(&self, operand: &Operand) -> Option<TyId> {
        match operand {
            Operand::Reg(reg) => self.function.regs.get(*reg).map(|found| found.ty),
            Operand::Const(_) => None,
        }
    }

    fn show(&self, operand: &Operand) -> String {
        match operand {
            Operand::Reg(reg) => match self.function.regs.get(*reg) {
                Some(found) => format!("r{}: {}{}", reg.index(), if found.cell { "cell " } else { "" }, self.name(found.ty)),
                None => format!("r{}", reg.index()),
            },
            Operand::Const(constant) => format!("{constant:?}"),
        }
    }

    fn const_fits(&self, constant: &Const, ty: TyId) -> bool {
        match (constant, self.kind(ty)) {
            (_, TyKind::Error) => true,
            (Const::Int(n), TyKind::Int(int)) => int.fits(*n),
            (Const::Float(_), TyKind::Float(_)) => true,
            (Const::Bool(_), TyKind::Bool) => true,
            (Const::String(_), TyKind::String) => true,
            (Const::Null, TyKind::Unit | TyKind::Optional(_)) => true,
            (_, TyKind::Union(members)) => members.iter().any(|&member| self.const_fits(constant, member)),
            _ => false,
        }
    }

    /// Whether the operand is a plain value of the type.
    fn fits(&self, operand: &Operand, ty: TyId) -> bool {
        match operand {
            Operand::Reg(reg) => self.function.regs.get(*reg).is_none_or(|found| !found.cell && self.same(found.ty, ty)),
            Operand::Const(constant) => self.const_fits(constant, ty),
        }
    }

    fn want(&mut self, operand: &Operand, ty: TyId, what: &str) {
        if !self.fits(operand, ty) {
            let message = format!("{what} {} is not {}", self.show(operand), self.name(ty));
            self.problem(message);
        }
    }

    fn want_int(&mut self, operand: &Operand, what: &str) {
        let ok = match operand {
            Operand::Reg(reg) => self.function.regs.get(*reg).is_none_or(|found| !found.cell && self.is_int(found.ty)),
            Operand::Const(constant) => matches!(constant, Const::Int(_)),
        };
        if !ok {
            let message = format!("{what} {} is not an integer", self.show(operand));
            self.problem(message);
        }
    }

    /// Whether `actual` is `declared`, each generic parameter being the one arg `bound` holds for it, or the first arg it meets.
    fn bind(&self, declared: TyId, actual: TyId, bound: &mut Vec<Option<GenericArg>>) -> bool {
        if self.is_error(declared) || self.is_error(actual) {
            return true;
        }
        match (self.kind(declared), self.kind(actual)) {
            (TyKind::Param { index, .. }, _) => {
                let index = *index as usize;
                if bound.len() <= index {
                    bound.resize(index + 1, None);
                }
                match &bound[index] {
                    Some(GenericArg::Type(held)) => self.same(*held, actual),
                    Some(GenericArg::Const(_)) => false,
                    None => {
                        bound[index] = Some(GenericArg::Type(actual));
                        true
                    }
                }
            }
            (TyKind::Builtin { kind: a, args: x }, TyKind::Builtin { kind: b, args: y }) => {
                a == b && x.len() == y.len() && x.iter().zip(y).all(|(&d, &t)| self.bind(d, t, bound))
            }
            (TyKind::Adt { def: a, args: x }, TyKind::Adt { def: b, args: y }) => {
                a == b && x.len() == y.len() && x.iter().zip(y).all(|(d, t)| self.bind_generic(d, t, bound))
            }
            (TyKind::Tuple(x), TyKind::Tuple(y)) => x.len() == y.len() && x.iter().zip(y).all(|(&d, &t)| self.bind(d, t, bound)),
            (TyKind::Array(x), TyKind::Array(y)) | (TyKind::Optional(x), TyKind::Optional(y)) => self.bind(*x, *y, bound),
            (TyKind::FixedArray(x, n), TyKind::FixedArray(y, m)) => self.bind_const(n, m, bound) && self.bind(*x, *y, bound),
            (TyKind::Union(x), TyKind::Union(y)) => x.len() == y.len() && y.iter().all(|&t| self.bind_one_of(x, t, bound)),
            (TyKind::Function(f), TyKind::Function(g)) => {
                f.params.len() == g.params.len()
                    && f.params.iter().zip(&g.params).all(|(d, t)| d.is_var == t.is_var && self.bind(d.ty, t.ty, bound))
                    && self.bind(f.ret, g.ret, bound)
            }
            _ => declared == actual,
        }
    }

    fn bind_const(&self, declared: &ConstArg, actual: &ConstArg, bound: &mut Vec<Option<GenericArg>>) -> bool {
        match declared {
            ConstArg::Param { index, .. } => {
                let index = *index as usize;
                if bound.len() <= index {
                    bound.resize(index + 1, None);
                }
                match &bound[index] {
                    Some(GenericArg::Const(held)) => held == actual,
                    Some(GenericArg::Type(_)) => false,
                    None => {
                        bound[index] = Some(GenericArg::Const(actual.clone()));
                        true
                    }
                }
            }
            _ => declared == actual,
        }
    }

    fn bind_generic(&self, declared: &GenericArg, actual: &GenericArg, bound: &mut Vec<Option<GenericArg>>) -> bool {
        match (declared, actual) {
            (GenericArg::Type(d), GenericArg::Type(t)) => self.bind(*d, *t, bound),
            (GenericArg::Const(d), GenericArg::Const(t)) => self.bind_const(d, t, bound),
            _ => false,
        }
    }

    /// Whether `actual` is one of the `declared` types; a failed try leaves `bound` as it was.
    fn bind_one_of(&self, declared: &[TyId], actual: TyId, bound: &mut Vec<Option<GenericArg>>) -> bool {
        declared.iter().any(|&each| {
            let mut trial = bound.clone();
            let fits = self.bind(each, actual, &mut trial);
            if fits {
                *bound = trial;
            }
            fits
        })
    }

    /// Whether `actual` is `declared` with the generic parameters read from `args`.
    fn shape(&self, declared: TyId, args: &[GenericArg], actual: TyId) -> bool {
        self.bind(declared, actual, &mut args.iter().cloned().map(Some).collect())
    }

    /// Whether the operand is a plain value of the declared type, under the parameters bound so far.
    fn declared_binds(&self, operand: &Operand, declared: TyId, bound: &mut Vec<Option<GenericArg>>) -> bool {
        match operand {
            Operand::Reg(reg) => self.function.regs.get(*reg).is_none_or(|found| !found.cell && self.bind(declared, found.ty, bound)),
            Operand::Const(constant) => match self.kind(declared) {
                TyKind::Param { index, .. } => bound.get(*index as usize).and_then(|slot| slot.as_ref()).and_then(GenericArg::as_type).is_none_or(|held| self.const_fits(constant, held)),
                _ => self.types().has_param(declared) || self.const_fits(constant, declared),
            },
        }
    }

    /// Whether the operand is a plain value of the declared type.
    fn declared_fits(&self, operand: &Operand, declared: TyId, args: &[GenericArg]) -> bool {
        self.declared_binds(operand, declared, &mut args.iter().cloned().map(Some).collect())
    }

    fn want_declared(&mut self, operand: &Operand, declared: TyId, args: &[GenericArg], what: &str) {
        if !self.declared_fits(operand, declared, args) {
            let message = format!("{what} {} is not {}", self.show(operand), self.name(declared));
            self.problem(message);
        }
    }

    fn builtin_ty(&self, ty: TyId, item: BuiltinTy) -> Option<&'a [TyId]> {
        match self.kind(ty) {
            TyKind::Builtin { kind, args } if *kind == item => Some(args),
            _ => None,
        }
    }

    fn std_adt(&self, ty: TyId, def: Option<DefId>) -> Option<&'a [GenericArg]> {
        let target = def?;
        match self.kind(ty) {
            TyKind::Adt { def: found, args } if *found == target => Some(args),
            _ => None,
        }
    }

    /// The element type of a list or an array.
    fn elements(&self, ty: TyId) -> Option<TyId> {
        match self.kind(ty) {
            TyKind::Array(element) | TyKind::FixedArray(element, _) => Some(*element),
            _ => self.builtin_ty(ty, BuiltinTy::List).and_then(|args| args.first().copied()),
        }
    }

    fn record(&self, ty: TyId) -> Option<(&'a Record, &'a [GenericArg])> {
        let program = self.program;
        let TyKind::Adt { def, args } = program.types.kind(ty) else { return None };
        Some((program.defs[*def].record()?, args))
    }

    fn enum_of(&self, ty: TyId) -> Option<(&'a Enum, &'a [GenericArg])> {
        let program = self.program;
        let TyKind::Adt { def, args } = program.types.kind(ty) else { return None };
        Some((program.defs[*def].enum_def()?, args))
    }

    fn global_ty(&self, def: DefId) -> Option<TyId> {
        match &self.program.defs[def].kind {
            DefKind::Global(global) => Some(global.ty),
            _ => None,
        }
    }

    /// Whether `whole` is `part`, or an optional or union that holds it.
    fn contains(&self, whole: TyId, part: TyId) -> bool {
        if self.same(whole, part) {
            return true;
        }
        match self.kind(whole) {
            TyKind::Optional(inner) => self.contains(*inner, part),
            TyKind::Union(members) => match self.kind(part) {
                TyKind::Union(parts) => parts.iter().all(|&each| self.contains(whole, each)),
                _ => members.iter().any(|&member| self.contains(member, part)),
            },
            TyKind::Any => true,
            _ => false,
        }
    }

    pub(super) fn typed_function(&mut self) {
        let function: &'a Function = self.function;
        for (id, reg) in function.regs.iter() {
            let types = self.types();
            if types.has_error(reg.ty) || types.has_infer(reg.ty) || types.has_param(reg.ty) {
                let message = format!("r{} has the unresolved type {}", id.index(), self.name(reg.ty));
                self.fail(message);
            }
        }
        let types = self.types();
        if types.has_error(function.ret) || types.has_infer(function.ret) || types.has_param(function.ret) {
            let message = format!("the result has the unresolved type {}", self.name(function.ret));
            self.fail(message);
        }
        if function.is_generator && self.builtin_ty(function.ret, BuiltinTy::Stream).is_none() && !self.is_error(function.ret) {
            let message = format!("a generator returns a Stream, not {}", self.name(function.ret));
            self.fail(message);
        }
    }

    pub(super) fn typed_inst(&mut self, inst: &'a Inst) {
        let function: &'a Function = self.function;
        let dest = inst.dest.and_then(|dest| function.regs.get(dest));
        let value = !matches!(inst.op, Op::SetField { .. } | Op::SetIndex { .. } | Op::SetSlice { .. } | Op::StoreCell { .. } | Op::StoreGlobal { .. } | Op::ScopeEnd(_) | Op::Defer { .. } | Op::Undefer | Op::Yield(_) | Op::FrameStore { .. } | Op::Call { .. });
        let makes_cell = match inst.op {
            Op::NewCell(_) => true,
            Op::FrameLoad(slot) => function.frame.get(slot as usize).is_some_and(|slot| slot.cell),
            _ => false,
        };
        if let Some(reg) = dest
            && reg.cell != makes_cell
        {
            self.problem("only `NewCell` and the load of a cell slot write a cell".to_string());
        }
        if value && dest.is_none() {
            self.problem("has no result register".to_string());
            return;
        }
        let ty = dest.map_or(TyId::UNIT, |reg| reg.ty);
        match &inst.op {
            Op::Copy(a) => self.want(a, ty, "operand"),
            Op::Unary { op, operand } => self.unary(*op, operand, ty),
            Op::Binary { op, left, right } => self.binary(*op, left, right, ty),
            Op::Cast(a) => self.cast(a, ty),
            Op::Show(a) => {
                let ok = match a {
                    Operand::Reg(_) => self.operand_ty(a).is_none_or(|found| matches!(self.kind(found), TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool | TyKind::String | TyKind::Tuple(_) | TyKind::Union(_) | TyKind::Any | TyKind::Array(_) | TyKind::FixedArray(..) | TyKind::Function(_) | TyKind::Error)),
                    Operand::Const(_) => true,
                };
                if !ok || !self.same(ty, TyId::STRING) {
                    self.problem(format!("shows {} as {}", self.show(a), self.name(ty)));
                }
            }
            Op::Concat(items) => {
                self.want_type(ty, TyId::STRING, "result");
                for item in items {
                    self.want(item, TyId::STRING, "part");
                }
            }
            Op::Convert { conversion, operand } => self.convert(*conversion, operand, ty),
            Op::Tuple(items) => match self.kind(ty) {
                TyKind::Tuple(elements) if elements.len() == items.len() => {
                    for (item, &element) in items.iter().zip(elements) {
                        self.want(item, element, "element");
                    }
                }
                TyKind::Error => {}
                _ => self.problem(format!("builds {} from {} elements", self.name(ty), items.len())),
            },
            Op::List(items) => match self.builtin_ty(ty, BuiltinTy::List) {
                Some([element]) => {
                    for item in items {
                        self.want(item, *element, "element");
                    }
                }
                _ if self.is_error(ty) => {}
                _ => self.problem(format!("builds {} as a list", self.name(ty))),
            },
            Op::Array(items) => match self.kind(ty) {
                TyKind::Array(element) => {
                    for item in items {
                        self.want(item, *element, "element");
                    }
                }
                TyKind::FixedArray(element, len) if len.as_u32().is_some_and(|n| n as usize == items.len()) => {
                    for item in items {
                        self.want(item, *element, "element");
                    }
                }
                TyKind::Error => {}
                _ => self.problem(format!("builds {} from {} elements", self.name(ty), items.len())),
            },
            Op::ArrayRepeat { value, len } => match self.kind(ty) {
                TyKind::Array(element) | TyKind::FixedArray(element, _) => {
                    self.want(value, *element, "value");
                    self.want_int(len, "length");
                }
                TyKind::Error => {}
                _ => self.problem(format!("repeats into {}", self.name(ty))),
            },
            Op::Record(items) => match self.record(ty) {
                Some((record, args)) if record.fields.len() == items.len() => {
                    for (item, field) in items.iter().zip(&record.fields) {
                        self.want_declared(item, field.ty, args, "field");
                    }
                }
                _ if self.is_error(ty) => {}
                _ => self.problem(format!("builds {} with {} fields", self.name(ty), items.len())),
            },
            Op::Variant { variant, fields } => self.variant(*variant, fields, ty),
            Op::Range { start, end, .. } => match self.std_adt(ty, self.program.std.range) {
                Some([GenericArg::Type(bound)]) => {
                    self.want(start, *bound, "start");
                    self.want(end, *bound, "end");
                }
                _ if self.is_error(ty) => {}
                _ => self.problem(format!("builds {} as a range", self.name(ty))),
            },
            Op::Field { object, index } => {
                if let Some((declared, args)) = self.field_of(object, *index)
                    && !self.shape(declared, args, ty)
                {
                    self.problem(format!("reads a {} into {}", self.name(declared), self.name(ty)));
                }
            }
            Op::SetField { object, index, value } => {
                if let Some((declared, args)) = self.field_of(object, *index) {
                    self.want_declared(value, declared, args, "value");
                }
            }
            Op::TupleField { object, index } => {
                if let Some(found) = self.operand_ty(object) {
                    match self.kind(found) {
                        TyKind::Tuple(elements) => match elements.get(*index as usize) {
                            Some(&element) if self.same(element, ty) => {}
                            _ => self.problem(format!("reads element {index} of {} into {}", self.name(found), self.name(ty))),
                        },
                        TyKind::Error => {}
                        _ => self.problem(format!("{} is not a tuple", self.show(object))),
                    }
                }
            }
            Op::Len(a) => {
                self.want_type(ty, TyId::INT, "result");
                if let Some(found) = self.operand_ty(a) {
                    let ok = self.elements(found).is_some() || self.is_error(found) || matches!(self.kind(found), TyKind::String);
                    if !ok {
                        self.problem(format!("{} has no length", self.show(a)));
                    }
                }
            }
            Op::Index { object, index } => {
                if let Some(found) = self.operand_ty(object) {
                    match self.elements(found) {
                        Some(element) => {
                            self.want_int(index, "position");
                            self.want_type(ty, element, "result");
                        }
                        None if self.is_error(found) => {}
                        None => self.problem(format!("{} cannot be indexed", self.show(object))),
                    }
                }
            }
            Op::SetIndex { object, index, value } => {
                if let Some(found) = self.operand_ty(object) {
                    match self.elements(found) {
                        Some(element) => {
                            self.want_int(index, "position");
                            self.want(value, element, "value");
                        }
                        None if self.is_error(found) => {}
                        None => self.problem(format!("{} cannot be indexed", self.show(object))),
                    }
                }
            }
            Op::Slice { object, start, end, step, .. } => {
                for bound in [start, end, step].into_iter().flatten() {
                    self.want_int(bound, "bound");
                }
                if let Some(found) = self.operand_ty(object) {
                    let ok = match (self.elements(found), self.elements(ty)) {
                        (Some(a), Some(b)) => self.same(a, b),
                        (None, None) => matches!(self.kind(found), TyKind::String | TyKind::Error) && self.same(ty, found),
                        _ => self.is_error(found) || self.is_error(ty),
                    };
                    if !ok {
                        self.problem(format!("slices {} into {}", self.show(object), self.name(ty)));
                    }
                }
            }
            Op::SetSlice { object, start, end, value, .. } => {
                for bound in [start, end].into_iter().flatten() {
                    self.want_int(bound, "bound");
                }
                if let Some(found) = self.operand_ty(object) {
                    let whole = self.operand_ty(value).is_some_and(|held| self.same(held, found));
                    match self.elements(found) {
                        Some(element) if !whole => self.want(value, element, "value"),
                        _ => self.want(value, found, "value"),
                    }
                }
            }
            Op::NewCell(Operand::Const(Const::Null)) => {}
            Op::NewCell(a) => self.want(a, ty, "operand"),
            Op::LoadCell(a) => match a {
                Operand::Reg(reg) => match function.regs.get(*reg) {
                    Some(found) if !found.cell => self.problem(format!("{} is not a cell", self.show(a))),
                    Some(found) if !self.same(found.ty, ty) => self.problem(format!("loads {} into {}", self.show(a), self.name(ty))),
                    _ => {}
                },
                Operand::Const(_) => self.problem("loads a constant".to_string()),
            },
            Op::StoreCell { cell, value } => match cell {
                Operand::Reg(reg) => match function.regs.get(*reg) {
                    Some(found) if !found.cell => self.problem(format!("{} is not a cell", self.show(cell))),
                    Some(found) => self.want(value, found.ty, "value"),
                    None => {}
                },
                Operand::Const(_) => self.problem("stores into a constant".to_string()),
            },
            Op::LoadGlobal(def) => match self.global_ty(*def) {
                Some(global) => self.want_type(ty, global, "result"),
                None => self.problem("loads a name that is not a global".to_string()),
            },
            Op::StoreGlobal { def, value } => match self.global_ty(*def) {
                Some(global) => self.want(value, global, "value"),
                None => self.problem("stores a name that is not a global".to_string()),
            },
            Op::IsSome(a) => {
                self.want_type(ty, TyId::BOOL, "result");
                if let Some(found) = self.operand_ty(a)
                    && !matches!(self.kind(found), TyKind::Optional(_) | TyKind::Error)
                {
                    self.problem(format!("{} is not optional", self.show(a)));
                }
            }
            Op::SomePayload(a) | Op::Unwrap(a) => {
                if let Some(found) = self.operand_ty(a) {
                    let inner = match self.kind(found) {
                        TyKind::Optional(inner) => Some(*inner),
                        _ if matches!(inst.op, Op::Unwrap(_)) => self.std_adt(found, self.program.std.result).and_then(|args| args.first().and_then(GenericArg::as_type)),
                        _ => None,
                    };
                    match inner {
                        Some(inner) => self.want_type(ty, inner, "result"),
                        None if self.is_error(found) => {}
                        None => self.problem(format!("{} holds no value", self.show(a))),
                    }
                }
            }
            Op::VariantTag(a) => {
                if !self.is_int(ty) {
                    self.problem(format!("the tag is not {}", self.name(ty)));
                }
                if let Some(found) = self.operand_ty(a)
                    && self.enum_of(found).is_none()
                    && !self.is_error(found)
                {
                    self.problem(format!("{} is not an enum", self.show(a)));
                }
            }
            Op::IsVariant { object, variant } => {
                self.want_type(ty, TyId::BOOL, "result");
                if let Some(found) = self.operand_ty(object) {
                    match self.enum_of(found) {
                        Some((def, _)) if (*variant as usize) < def.variants.len() => {}
                        Some(_) => self.problem(format!("{} has no variant {variant}", self.name(found))),
                        None if self.is_error(found) => {}
                        None => self.problem(format!("{} is not an enum", self.show(object))),
                    }
                }
            }
            Op::VariantField { object, variant, index } => {
                if let Some(found) = self.operand_ty(object) {
                    match self.enum_of(found) {
                        Some((def, args)) => {
                            let declared = def.variants.get(*variant as usize).and_then(|each| match &each.payload {
                                Payload::Unit => None,
                                Payload::Tuple(types) => types.get(*index as usize).copied(),
                                Payload::Struct(fields) => fields.get(*index as usize).map(|field| field.ty),
                            });
                            match declared {
                                Some(declared) if self.shape(declared, args, ty) => {}
                                Some(declared) => self.problem(format!("reads a {} into {}", self.name(declared), self.name(ty))),
                                None => self.problem(format!("{} has no field {index} in variant {variant}", self.name(found))),
                            }
                        }
                        None if self.is_error(found) => {}
                        None => self.problem(format!("{} is not an enum", self.show(object))),
                    }
                }
            }
            Op::IsType { operand, ty: target } => {
                self.want_type(ty, TyId::BOOL, "result");
                self.typed_subject(operand, *target);
            }
            Op::UnionPayload { operand, ty: target } => {
                self.want_type(ty, *target, "result");
                self.typed_subject(operand, *target);
            }
            Op::Call { callee, args, outs } => self.call(callee, args, outs, dest.map(|reg| reg.ty)),
            Op::Closure { func, captures } => self.closure(*func, captures, ty),
            Op::Spawn { func, captures, scope } => {
                self.carrier_types(*func, captures);
                if let Some(target) = self.module.functions.get(*func) {
                    match self.builtin_ty(ty, BuiltinTy::Task) {
                        Some([result]) if self.same(*result, target.ret) => {}
                        _ if self.is_error(ty) => {}
                        _ => self.problem(format!("starts a body returning {} as {}", self.name(target.ret), self.name(ty))),
                    }
                    if !target.params.is_empty() {
                        self.problem(format!("f{} takes arguments", func.index()));
                    }
                }
                if let Some(scope) = scope {
                    self.want(scope, TyId::INT, "scope");
                }
            }
            Op::ScopeBegin => self.want_type(ty, TyId::INT, "result"),
            Op::Defer { .. } | Op::Undefer => {}
            Op::ScopeEnd(a) => {
                self.want(a, TyId::INT, "scope");
                if dest.is_some() {
                    match self.kind(ty) {
                        TyKind::Optional(inner) => self.want_type(*inner, function.ret, "result"),
                        _ => self.problem(format!("the result of a scope end is {}, not an optional", self.name(ty))),
                    }
                }
            }
            Op::Yield(a) => {
                if !function.is_generator {
                    self.problem("yields outside a generator".to_string());
                } else if let Some([element]) = self.builtin_ty(function.ret, BuiltinTy::Stream) {
                    self.want(a, *element, "value");
                }
            }
            Op::Detach(a) => self.want(a, ty, "operand"),
            Op::FrameLoad(slot) => {
                if let Some(held) = function.frame.get(*slot as usize) {
                    self.want_type(ty, held.ty, "result");
                }
            }
            Op::FrameStore { slot, value } => {
                if let Some(held) = function.frame.get(*slot as usize) {
                    self.slot_value(value, held, "value");
                }
            }
            Op::Stream { resume, init } => self.stream(*resume, init, ty),
        }
    }

    /// A value for a frame slot: a cell reg for a cell slot, a plain value of the type for any other.
    fn slot_value(&mut self, operand: &Operand, slot: &FrameSlot, what: &str) {
        match operand {
            Operand::Reg(reg) => {
                if let Some(found) = self.function.regs.get(*reg)
                    && (found.cell != slot.cell || !self.same(found.ty, slot.ty))
                {
                    let message = format!("{what} {} does not fit {}{}", self.show(operand), if slot.cell { "cell " } else { "" }, self.name(slot.ty));
                    self.problem(message);
                }
            }
            Operand::Const(_) if slot.cell => self.problem(format!("{what} {} is not a cell", self.show(operand))),
            Operand::Const(_) => self.want(operand, slot.ty, what),
        }
    }

    /// A `Stream` over the frame of a `Resume` function that returns the next element, or `null`.
    fn stream(&mut self, resume: FuncId, init: &[Operand], ty: TyId) {
        let Some(target) = self.module.functions.get(resume) else { return };
        if !matches!(target.kind, FunctionKind::Resume { .. }) || !target.params.is_empty() {
            self.problem(format!("f{} is not a resume function", resume.index()));
            return;
        }
        match (self.builtin_ty(ty, BuiltinTy::Stream), self.kind(target.ret)) {
            (Some([element]), TyKind::Optional(next)) if self.same(*element, *next) => {}
            _ if self.is_error(ty) => {}
            _ => self.problem(format!("f{} returns {}, not the next element of {}", resume.index(), self.name(target.ret), self.name(ty))),
        }
        if target.frame.first().is_none_or(|state| state.cell || !self.same(state.ty, TyId::INT)) {
            self.problem(format!("slot 0 of f{} is not the state", resume.index()));
        }
        for (value, slot) in init.iter().zip(target.frame.iter().skip(1)) {
            self.slot_value(value, slot, "initial value");
        }
    }

    fn want_type(&mut self, found: TyId, expected: TyId, what: &str) {
        if !self.same(found, expected) {
            let message = format!("{what} is {}, not {}", self.name(found), self.name(expected));
            self.problem(message);
        }
    }

    fn unary(&mut self, op: UnaryOp, operand: &Operand, ty: TyId) {
        let ok = match op {
            UnaryOp::Not => self.is_bool(ty),
            UnaryOp::Neg => self.is_number(ty),
            UnaryOp::BitNot => self.is_int(ty),
        };
        if !ok {
            self.problem(format!("the operator does not apply to {}", self.name(ty)));
        }
        self.want(operand, ty, "operand");
    }

    fn binary(&mut self, op: BinaryOp, left: &Operand, right: &Operand, ty: TyId) {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if !self.is_number(ty) && !(op == BinaryOp::Add && ty == TyId::STRING) {
                    self.problem(format!("arithmetic on {}", self.name(ty)));
                }
                self.want(left, ty, "left");
                self.want(right, ty, "right");
            }
            BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                self.want_type(ty, TyId::BOOL, "result");
                if let Some(side) = self.operand_ty(left).or_else(|| self.operand_ty(right)) {
                    let ordered = matches!(op, BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq);
                    if ordered && !self.is_number(side) && !matches!(self.kind(side), TyKind::String) {
                        self.problem(format!("{} cannot be ordered", self.name(side)));
                    }
                    self.want(left, side, "left");
                    self.want(right, side, "right");
                }
            }
            BinaryOp::And | BinaryOp::Or => {
                self.want_type(ty, TyId::BOOL, "result");
                self.want(left, TyId::BOOL, "left");
                self.want(right, TyId::BOOL, "right");
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                if !self.is_int(ty) && !self.is_bool(ty) {
                    self.problem(format!("bit operation on {}", self.name(ty)));
                }
                self.want(left, ty, "left");
                self.want(right, ty, "right");
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                if !self.is_int(ty) {
                    self.problem(format!("shift of {}", self.name(ty)));
                }
                self.want(left, ty, "left");
                self.want_int(right, "shift");
            }
            BinaryOp::Coalesce => self.problem("`??` is lowered before the IR".to_string()),
        }
    }

    fn cast(&mut self, operand: &Operand, ty: TyId) {
        if !self.is_number(ty) {
            self.problem(format!("casts to {}", self.name(ty)));
        }
        let ok = match operand {
            Operand::Reg(_) => self.operand_ty(operand).is_none_or(|found| matches!(self.kind(found), TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool | TyKind::Error) || self.enum_of(found).is_some()),
            Operand::Const(constant) => matches!(constant, Const::Int(_) | Const::Float(_) | Const::Bool(_)),
        };
        if !ok {
            self.problem(format!("casts {}", self.show(operand)));
        }
    }

    fn convert(&mut self, conversion: Conversion, operand: &Operand, ty: TyId) {
        let from = self.operand_ty(operand);
        let ok = match conversion {
            Conversion::ToOptional => match self.kind(ty) {
                TyKind::Optional(inner) => self.fits(operand, *inner),
                kind => matches!(kind, TyKind::Error),
            },
            Conversion::ToUnion => match (self.kind(ty), from) {
                (TyKind::Union(members), Some(found)) => members.iter().any(|&member| self.same(member, found)),
                (TyKind::Union(members), None) => members.iter().any(|&member| self.fits(operand, member)),
                (TyKind::Any, _) => true,
                (kind, _) => matches!(kind, TyKind::Error),
            },
            Conversion::WidenUnion => matches!(self.kind(ty), TyKind::Union(_) | TyKind::Any | TyKind::Error) && from.is_none_or(|found| matches!(self.kind(found), TyKind::Union(_) | TyKind::Any) && self.contains(ty, found)),
            Conversion::Narrow => from.is_none_or(|found| self.contains(found, ty)),
            Conversion::Adapt => from.is_none_or(|found| match (self.kind(found), self.kind(ty)) {
                (TyKind::Function(_), TyKind::Function(_)) => true,
                (TyKind::Adt { def: a, .. }, TyKind::Adt { def: b, .. }) => a == b,
                (TyKind::Error, _) | (_, TyKind::Error) => true,
                _ => false,
            }),
            Conversion::ToView => from.is_none_or(|found| mote_semantics::plan(self.program, found, ty).is_some()),
        };
        if !ok {
            self.problem(format!("{} does not turn {} into {}", conversion_name(conversion), self.show(operand), self.name(ty)));
        }
    }

    fn variant(&mut self, variant: u32, fields: &[Operand], ty: TyId) {
        let Some((def, args)) = self.enum_of(ty) else {
            if !self.is_error(ty) {
                self.problem(format!("builds {} as an enum", self.name(ty)));
            }
            return;
        };
        let Some(found) = def.variants.get(variant as usize) else {
            self.problem(format!("{} has no variant {variant}", self.name(ty)));
            return;
        };
        let declared: Vec<TyId> = match &found.payload {
            Payload::Unit => Vec::new(),
            Payload::Tuple(types) => types.clone(),
            Payload::Struct(each) => each.iter().map(|field| field.ty).collect(),
        };
        if declared.len() != fields.len() {
            self.problem(format!("variant {variant} of {} takes {} fields, found {}", self.name(ty), declared.len(), fields.len()));
            return;
        }
        for (field, &want) in fields.iter().zip(&declared) {
            self.want_declared(field, want, args, "field");
        }
    }

    /// The declared type of a field of the record or tuple the operand holds, with the record's type arguments.
    fn field_of(&mut self, object: &Operand, index: u32) -> Option<(TyId, &'a [GenericArg])> {
        let found = self.operand_ty(object)?;
        if self.is_error(found) {
            return None;
        }
        if let TyKind::Tuple(elements) = self.kind(found) {
            return elements.get(index as usize).map(|&element| (element, &[][..]));
        }
        if matches!(self.kind(found), TyKind::Builtin { kind: BuiltinTy::Pointer, .. }) && index == 0 {
            return Some((TyId::UINT64, &[][..]));
        }
        let Some((record, args)) = self.record(found) else {
            self.problem(format!("{} is not a record", self.show(object)));
            return None;
        };
        match record.fields.get(index as usize) {
            Some(field) => Some((field.ty, args)),
            None => {
                self.problem(format!("{} has no field {index}", self.name(found)));
                None
            }
        }
    }

    /// An operand tested or unwrapped by type must be a union, and the type must be concrete.
    fn typed_subject(&mut self, operand: &Operand, target: TyId) {
        let types = self.types();
        if types.has_error(target) || types.has_infer(target) || types.has_param(target) {
            self.problem(format!("tests for the unresolved type {}", self.name(target)));
        }
        if let Some(found) = self.operand_ty(operand)
            && !matches!(self.kind(found), TyKind::Union(_) | TyKind::Any | TyKind::Error)
        {
            self.problem(format!("{} is not a union", self.show(operand)));
        }
    }

    fn call(&mut self, callee: &Callee, args: &[Operand], outs: &[RegId], dest: Option<TyId>) {
        let function: &'a Function = self.function;
        match callee {
            Callee::Function(func) => {
                let Some(target) = self.module.functions.get(*func) else { return };
                for (arg, &param) in args.iter().zip(&target.params) {
                    if let Some(reg) = target.regs.get(param) {
                        self.want(arg, reg.ty, "argument");
                    }
                }
                if let Some(found) = dest {
                    self.want_type(found, target.ret, "result");
                }
                for (&out, &position) in outs.iter().zip(&target.out_params) {
                    if let (Some(reg), Some(param)) = (function.regs.get(out), target.params.get(position).and_then(|&param| target.regs.get(param))) {
                        self.want_type(reg.ty, param.ty, "copied-back value");
                    }
                }
            }
            Callee::Value(operand) => {
                let Some(found) = self.operand_ty(operand) else {
                    self.problem("calls a constant".to_string());
                    return;
                };
                let TyKind::Function(signature) = self.kind(found) else {
                    if !self.is_error(found) {
                        self.problem(format!("{} is not a function", self.show(operand)));
                    }
                    return;
                };
                if signature.params.len() != args.len() {
                    self.problem(format!("{} takes {} arguments, found {}", self.name(found), signature.params.len(), args.len()));
                    return;
                }
                for (arg, param) in args.iter().zip(&signature.params) {
                    self.want(arg, param.ty, "argument");
                }
                if let Some(result) = dest {
                    self.want_type(result, signature.ret, "result");
                }
                let writes: Vec<TyId> = signature.params.iter().filter(|param| param.is_var).map(|param| param.ty).collect();
                if writes.len() != outs.len() {
                    self.problem(format!("{} copies back {} values, found {}", self.name(found), writes.len(), outs.len()));
                }
            }
            Callee::Intrinsic(name) => {
                let text = self.interner.resolve(*name);
                match intrinsics::lookup(text) {
                    Some(row) if row.params != args.len() => self.problem(format!("`{text}` takes {} arguments, found {}", row.params, args.len())),
                    Some(_) => {}
                    None => self.problem(format!("`{text}` is not an intrinsic")),
                }
            }
            Callee::Native(def) => {
                let program = self.program;
                let Some(function_def) = program.defs[*def].function() else {
                    self.problem("calls a name that is not a function".to_string());
                    return;
                };
                if function_def.link.is_none() {
                    self.problem("calls a function that is not native".to_string());
                }
                if function_def.params.len() != args.len() {
                    self.problem(format!("takes {} arguments, found {}", function_def.params.len(), args.len()));
                    return;
                }
                let mut bound = Vec::new();
                for (arg, param) in args.iter().zip(&function_def.params) {
                    if !param.variadic && !self.declared_binds(arg, param.ty, &mut bound) {
                        let message = format!("argument {} is not {}", self.show(arg), self.name(param.ty));
                        self.problem(message);
                    }
                }
                if let Some(result) = dest
                    && !self.bind(function_def.ret, result, &mut bound)
                {
                    self.problem(format!("result {} is not {}", self.name(result), self.name(function_def.ret)));
                }
                if !self.outs_fit(function_def, outs, &mut bound) {
                    self.problem(format!("copies back {} values that do not fit the `var` parameters", outs.len()));
                }
            }
        }
    }

    /// Whether the registers that take the final values fit the `var` parameters of the declaration.
    fn outs_fit(&self, declared: &Declared, outs: &[RegId], bound: &mut Vec<Option<GenericArg>>) -> bool {
        let writes: Vec<TyId> = declared.params.iter().filter(|param| param.is_var).map(|param| param.ty).collect();
        writes.len() == outs.len() && writes.iter().zip(outs).all(|(&ty, &out)| self.function.regs.get(out).is_none_or(|reg| self.bind(ty, reg.ty, bound)))
    }

    fn closure(&mut self, func: FuncId, captures: &[Operand], ty: TyId) {
        self.carrier_types(func, captures);
        let Some(target) = self.module.functions.get(func) else { return };
        let TyKind::Function(signature) = self.kind(ty) else {
            if !self.is_error(ty) {
                self.problem(format!("a function value is {}", self.name(ty)));
            }
            return;
        };
        if signature.params.len() != target.params.len() {
            self.problem(format!("f{} takes {} arguments, {} wants {}", func.index(), target.params.len(), self.name(ty), signature.params.len()));
            return;
        }
        for (position, (param, &reg)) in signature.params.iter().zip(&target.params).enumerate() {
            let Some(found) = target.regs.get(reg) else { continue };
            if !self.same(found.ty, param.ty) {
                self.problem(format!("parameter {position} of f{} is {}, not {}", func.index(), self.name(found.ty), self.name(param.ty)));
            }
            if param.is_var != target.out_params.contains(&position) {
                self.problem(format!("parameter {position} of f{} disagrees with {} about `var`", func.index(), self.name(ty)));
            }
        }
        if !self.same(signature.ret, target.ret) {
            self.problem(format!("f{} returns {}, not {}", func.index(), self.name(target.ret), self.name(signature.ret)));
        }
    }

    /// Each capture is a value or cell like the register of the closure that holds it.
    fn carrier_types(&mut self, func: FuncId, captures: &[Operand]) {
        let Some(target) = self.module.functions.get(func) else { return };
        for (capture, &reg) in captures.iter().zip(&target.captures) {
            let Some(held) = target.regs.get(reg) else { continue };
            match capture {
                Operand::Reg(found) => {
                    if let Some(found) = self.function.regs.get(*found)
                        && (found.cell != held.cell || !self.same(found.ty, held.ty))
                    {
                        self.problem(format!("capture {} does not fit {}{} of f{}", self.show(capture), if held.cell { "cell " } else { "" }, self.name(held.ty), func.index()));
                    }
                }
                Operand::Const(constant) => {
                    if held.cell || !self.const_fits(constant, held.ty) {
                        self.problem(format!("capture {} does not fit {} of f{}", self.show(capture), self.name(held.ty), func.index()));
                    }
                }
            }
        }
    }

    pub(super) fn typed_term(&mut self, term: &Terminator) {
        let function: &'a Function = self.function;
        match term {
            Terminator::Branch { condition, .. } => self.want(condition, TyId::BOOL, "branch condition"),
            Terminator::Switch { value, cases, .. } => {
                match value {
                    Operand::Reg(_) => {
                        if let Some(found) = self.operand_ty(value) {
                            match self.kind(found) {
                                TyKind::Int(int) => {
                                    for &(case, _) in cases {
                                        if !int.fits(case) {
                                            self.fail(format!("switch case {case} does not fit {}", self.name(found)));
                                        }
                                    }
                                }
                                TyKind::Error => {}
                                _ => self.fail(format!("switch value {} is not an integer", self.show(value))),
                            }
                        }
                    }
                    Operand::Const(constant) => {
                        if !matches!(constant, Const::Int(_)) {
                            self.fail(format!("switch value {} is not an integer", self.show(value)));
                        }
                    }
                }
            }
            Terminator::Return(Some(value)) => {
                if function.is_generator {
                    self.fail("a generator ends without a value".to_string());
                } else {
                    self.want(value, function.ret, "returned");
                }
            }
            Terminator::Return(None) => {
                if !function.is_generator && !self.same(function.ret, TyId::UNIT) {
                    self.fail(format!("returns nothing from a function returning {}", self.name(function.ret)));
                }
            }
            Terminator::Jump(_) | Terminator::Unreachable => {}
        }
    }
}
