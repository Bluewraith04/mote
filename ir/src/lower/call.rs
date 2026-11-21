use mote_base::{Id, Span};
use mote_semantics::{Callee as TreeCallee, Conversion, DefId, Expr, FnRef, GenericArg, Receiver, TyId, TyKind};

use super::place::Place;
use super::Lowering;
use crate::intrinsics;
use crate::ir::{Callee, Const, Op, Operand, Terminator};

impl Lowering<'_> {
    pub(super) fn call(&mut self, id: Id<Expr>, callee: &TreeCallee, args: &[Id<Expr>], ty: TyId, span: Span) -> Operand {
        let (target, vars) = match callee {
            TreeCallee::Function(function) => {
                let Some(target) = self.function_callee(function) else { return self.unsupported(span, "call without a body") };
                (target, self.var_positions(function.def))
            }
            TreeCallee::Builtin { name, generic_args } => {
                let raw = self.interner.resolve(*name).to_string();
                let rt_name = raw.strip_prefix("__").unwrap_or(&raw).to_string();
                if rt_name == "any_to" {
                    return self.lower_any_to(args, generic_args, ty, span);
                }
                if let Some(out) = self.lower_array_builtin(&rt_name, args, ty, span) {
                    return out;
                }
                let Some(_) = intrinsics::lookup(&rt_name) else {
                    return self.unsupported(span, "compiler builtin without IR lowering");
                };
                let sym = self.interner.intern(&rt_name);
                let vars = if mote_semantics::builtins::first_arg_writes(&raw) {
                    vec![true]
                } else {
                    Vec::new()
                };
                (Callee::Intrinsic(sym), vars)
            }
            TreeCallee::Value(value) => {
                let vars = match self.program.types.kind(self.body.exprs[*value].ty) {
                    TyKind::Function(function) => function.params.iter().map(|param| param.is_var).collect(),
                    _ => Vec::new(),
                };
                (Callee::Value(self.expr(*value)), vars)
            }
            TreeCallee::TraitMethod { .. } => return self.unsupported(span, "trait method call"),
        };
        let order: Vec<usize> = match self.body.arg_order.get(&id) {
            Some(order) => order.iter().map(|&position| position as usize).collect(),
            None => (0..args.len()).collect(),
        };
        let mut operands = vec![Operand::Const(Const::Null); args.len()];
        let mut places: Vec<Option<Place>> = args.iter().map(|_| None).collect();
        for position in order {
            let arg = args[position];
            if vars.get(position).copied().unwrap_or(false)
                && self.is_place(arg)
                && let Some(place) = self.place(arg)
            {
                operands[position] = self.read(&place, span);
                places[position] = Some(place);
                continue;
            }
            operands[position] = self.expr(arg);
        }
        if let Callee::Intrinsic(name) = &target
            && matches!(self.interner.resolve(*name), "sender_send" | "sender_try_send")
            && operands.len() == 2
        {
            operands[1] = self.detach(operands[1], self.body.exprs[args[1]].ty, span);
        }
        let var_args: Vec<usize> = (0..args.len()).filter(|&position| vars.get(position).copied().unwrap_or(false)).collect();
        let outs: Vec<_> = var_args.iter().map(|&position| self.b.reg(self.body.exprs[args[position]].ty, None)).collect();
        let op = Op::Call { callee: target, args: operands, outs: outs.clone() };
        let result = if ty == TyId::UNIT {
            self.b.emit(None, op, span);
            Operand::Const(Const::Null)
        } else {
            self.b.value(ty, op, span)
        };
        for (position, out) in var_args.into_iter().zip(outs) {
            if let Some(place) = &places[position] {
                self.write(place, Operand::Reg(out), span);
            }
        }
        result
    }

    /// A call to the std method `name` of the `Map` or `Set` type `ty` that the language itself makes; `None` when `ty` has no such method.
    pub(super) fn collection_call(&mut self, ty: TyId, name: &str, args: Vec<Operand>, ret: TyId, span: Span) -> Option<Operand> {
        let method = self.interner.get(name).and_then(|name| self.program.collection_method(ty, name))?;
        let Some(target) = self.function_callee(&method) else { return Some(self.unsupported(span, "a Map operation")) };
        let writes = self.var_positions(method.def).into_iter().filter(|&var| var).count();
        let outs = (0..writes).map(|_| self.b.reg(ty, None)).collect();
        let op = Op::Call { callee: target, args, outs };
        if ret == TyId::UNIT {
            self.b.emit(None, op, span);
            return Some(Operand::Const(Const::Null));
        }
        Some(self.b.value(ret, op, span))
    }

    /// The IR callee of a named function: its lowered body or a linked extern library function.
    pub(super) fn function_callee(&self, function: &FnRef) -> Option<Callee> {
        if let Some(&func) = self.instances.get(function) {
            return Some(Callee::Function(func));
        }
        let item = self.program.defs[function.def].function()?;
        item.link.is_some().then_some(Callee::Native(function.def))
    }

    /// Which argument positions are `var`: the receiver of a `var self` method, then each `var` parameter.
    pub(super) fn var_positions(&self, def: DefId) -> Vec<bool> {
        let Some(function) = self.program.defs[def].function() else { return Vec::new() };
        let mut positions = Vec::new();
        if function.receiver != Receiver::None {
            positions.push(function.receiver == Receiver::Write);
        }
        positions.extend(function.params.iter().map(|param| param.is_var));
        positions
    }

    fn lower_any_to(&mut self, args: &[Id<Expr>], generic_args: &[GenericArg], ty: TyId, span: Span) -> Operand {
        let Some(target) = generic_args.first().and_then(GenericArg::as_type) else {
            return self.unsupported(span, "any.to requires a type argument");
        };
        let operand = self.expr(args[0]);
        if target == TyId::ANY {
            return self.b.value(ty, Op::Convert { conversion: Conversion::ToOptional, operand }, span);
        }
        let result = self.b.reg(ty, None);
        let (hit, miss, join) = (self.b.block(), self.b.block(), self.b.block());
        let is = self.b.value(TyId::BOOL, Op::IsType { operand, ty: target }, span);
        self.b.terminate(Terminator::Branch { condition: is, then: hit, otherwise: miss });
        self.b.switch_to(hit);
        let payload = self.b.value(target, Op::UnionPayload { operand, ty: target }, span);
        let some = self.b.value(ty, Op::Convert { conversion: Conversion::ToOptional, operand: payload }, span);
        self.b.emit(Some(result), Op::Copy(some), span);
        self.b.terminate(Terminator::Jump(join));
        self.b.switch_to(miss);
        self.b.emit(Some(result), Op::Copy(Operand::Const(Const::Null)), span);
        self.b.terminate(Terminator::Jump(join));
        self.b.switch_to(join);
        Operand::Reg(result)
    }
}
