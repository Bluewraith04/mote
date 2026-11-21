use std::collections::HashMap;

use mote_base::Span;
use mote_semantics::{plan, Closure, Conversion, FnRef, LangItem, Local, TyId, TyKind};

use super::{placeholder, Cleanup, Lowering};
use crate::builder::FunctionBuilder;
use crate::ir::{BlockId, Callee, Const, FuncId, FunctionKind, Op, Operand, RegId, Terminator};

/// The function being lowered, set aside while a closure inside it is.
pub(super) struct Outer {
    b: FunctionBuilder,
    regs: HashMap<mote_base::Id<Local>, RegId>,
    cell_params: Vec<(RegId, RegId)>,
    loops: Vec<(BlockId, BlockId, usize)>,
    cleanups: Vec<Cleanup>,
    scopes: Vec<Operand>,
    detach_result: bool,
    ret: TyId,
    func: FuncId,
}

impl Lowering<'_> {
    /// Starts lowering another function; `restore` goes back to the one set aside.
    pub(super) fn begin(&mut self, func: FuncId, kind: FunctionKind, ret: TyId) -> Outer {
        Outer {
            b: std::mem::replace(&mut self.b, FunctionBuilder::new(kind, ret)),
            regs: std::mem::take(&mut self.regs),
            cell_params: std::mem::take(&mut self.cell_params),
            loops: std::mem::take(&mut self.loops),
            cleanups: std::mem::take(&mut self.cleanups),
            scopes: std::mem::take(&mut self.scopes),
            detach_result: std::mem::replace(&mut self.detach_result, false),
            ret: std::mem::replace(&mut self.ret, ret),
            func: std::mem::replace(&mut self.func, func),
        }
    }

    pub(super) fn restore(&mut self, outer: Outer) {
        self.b = outer.b;
        self.regs = outer.regs;
        self.cell_params = outer.cell_params;
        self.loops = outer.loops;
        self.cleanups = outer.cleanups;
        self.scopes = outer.scopes;
        self.detach_result = outer.detach_result;
        self.ret = outer.ret;
        self.func = outer.func;
    }

    /// A lambda or nested `fn`: its body becomes a function of its own and the value carries the captures.
    pub(super) fn closure(&mut self, closure: &Closure, ty: TyId, span: Span) -> Operand {
        let ret = match self.program.types.kind(ty) {
            TyKind::Function(function) => function.ret,
            _ => TyId::UNIT,
        };
        let mut captures = Vec::new();
        for local in &closure.captures {
            match self.regs.get(local) {
                Some(&reg) => captures.push(Operand::Reg(reg)),
                None => return self.unsupported(span, "capture of an unsupported local"),
            }
        }
        let func = self.closure_function(closure, ret, false);
        self.b.value(ty, Op::Closure { func, captures }, span)
    }

    /// Lowers the body of a closure; its captured locals are the function's captures. A task's `return` detaches the arrays of its result.
    pub(super) fn closure_function(&mut self, closure: &Closure, ret: TyId, detach_result: bool) -> FuncId {
        let kind = FunctionKind::Closure { parent: self.func };
        let id = self.functions.alloc(placeholder(kind.clone(), ret));
        let outer = self.begin(id, kind, ret);
        self.detach_result = detach_result;
        for &local in &closure.captures {
            let info = &self.body.locals[local];
            let reg = self.b.capture(info.ty, Some(info.name), self.cells.contains(&local));
            self.regs.insert(local, reg);
        }
        self.enter(&closure.params);
        self.block(&closure.body);
        let function = self.end_body(false);
        self.restore(outer);
        self.functions[id] = function;
        id
    }

    /// A named function used as a value; a native or an intrinsic goes through a function that calls it.
    pub(super) fn function_value(&mut self, function: &FnRef, ty: TyId, span: Span) -> Operand {
        let found = match self.instances.get(function) {
            Some(&func) => Some(func),
            None => self.native_wrapper(function, ty),
        };
        match found {
            Some(func) => self.b.value(ty, Op::Closure { func, captures: Vec::new() }, span),
            None => self.unsupported(span, "function value without a body"),
        }
    }

    /// The function that calls a native or an intrinsic with the parameters of its function type.
    fn native_wrapper(&mut self, function: &FnRef, ty: TyId) -> Option<FuncId> {
        if let Some(&id) = self.wrappers.get(function) {
            return Some(id);
        }
        let callee = self.function_callee(function)?;
        let TyKind::Function(sig) = self.program.types.kind(ty).clone() else { return None };
        let kind = FunctionKind::Native(function.clone());
        let id = self.functions.alloc(placeholder(kind.clone(), sig.ret));
        self.wrappers.insert(function.clone(), id);
        let outer = self.begin(id, kind, sig.ret);
        let span = Span::default();
        let params: Vec<RegId> = sig.params.iter().map(|param| self.b.param(param.ty, None, param.is_var)).collect();
        let vars: Vec<(RegId, RegId)> = sig.params.iter().zip(&params).filter(|(param, _)| param.is_var).map(|(param, &reg)| (reg, self.b.reg(param.ty, None))).collect();
        let call = Op::Call { callee, args: params.iter().map(|&reg| Operand::Reg(reg)).collect(), outs: vars.iter().map(|&(_, out)| out).collect() };
        let result = if sig.ret == TyId::UNIT {
            self.b.emit(None, call, span);
            None
        } else {
            Some(self.b.value(sig.ret, call, span))
        };
        for (reg, out) in vars {
            self.b.emit(Some(reg), Op::Copy(Operand::Reg(out)), span);
        }
        self.b.terminate(Terminator::Return(result));
        let built = self.end_body(false);
        self.restore(outer);
        self.functions[id] = built;
        Some(id)
    }

    /// A value of one type as another it fits, one conversion at a time.
    pub(super) fn fit_value(&mut self, mut operand: Operand, from: TyId, to: TyId, span: Span) -> Operand {
        let Some(steps) = plan(self.program, from, to) else { return operand };
        let mut current = from;
        for (conversion, ty) in steps {
            operand = match conversion {
                Conversion::Adapt => self.adapt(operand, current, ty, span),
                Conversion::ToOptional if current == TyId::UNIT => self.b.value(ty, Op::Copy(Operand::Const(Const::Null)), span),
                _ => self.b.value(ty, Op::Convert { conversion, operand }, span),
            };
            current = ty;
        }
        operand
    }

    /// The element types of a built-in type of the item (`Stream` or `Task`) read as the same type of another element.
    fn handle_elements(&self, item: LangItem, from: TyId, to: TyId) -> Option<(TyId, TyId)> {
        let handle = *self.program.lang.get(&item)?;
        match (self.program.types.kind(from), self.program.types.kind(to)) {
            (TyKind::Adt { def: was, args: from }, TyKind::Adt { def: now, args: to }) if *was == handle && *now == handle => Some((*from.first()?, *to.first()?)),
            _ => None,
        }
    }

    /// A `Task` as a `Task` of a wider result: a view that shares the task and converts its result through a closure over the task.
    fn adapt_task(&mut self, operand: Operand, from: TyId, to: TyId, was: TyId, now: TyId, span: Span) -> Operand {
        let kind = FunctionKind::Closure { parent: self.func };
        let id = self.functions.alloc(placeholder(kind.clone(), now));
        let outer = self.begin(id, kind, now);
        let held = self.b.capture(from, None, false);
        let task_value = self.interner.intern("task_value");
        let value = self.b.value(was, Op::Call { callee: Callee::Intrinsic(task_value), args: vec![Operand::Reg(held)], outs: Vec::new() }, span);
        let fitted = self.fit_value(value, was, now, span);
        self.b.terminate(Terminator::Return(Some(fitted)));
        let function = self.end_body(false);
        self.restore(outer);
        self.functions[id] = function;
        let convert_ty = self.program.types.function(Vec::new(), now, false);
        let convert = self.b.value(convert_ty, Op::Closure { func: id, captures: vec![operand] }, span);
        let task_view = self.interner.intern("task_view");
        self.b.value(to, Op::Call { callee: Callee::Intrinsic(task_view), args: vec![operand, convert], outs: Vec::new() }, span)
    }

    /// A `Stream` as a `Stream` of a wider element: a resume function over the stream converts each element it yields.
    fn adapt_stream(&mut self, operand: Operand, from: TyId, to: TyId, was: TyId, now: TyId, span: Span) -> Operand {
        let next = self.program.types.optional(now);
        let kind = FunctionKind::Resume { generator: self.func };
        let id = self.functions.alloc(placeholder(kind.clone(), next));
        let outer = self.begin(id, kind, next);
        self.b.slot(TyId::INT, false);
        let held = self.b.slot(from, false);
        let (more, done) = (self.b.block(), self.b.block());
        let inner = self.b.value(from, Op::FrameLoad(held), span);
        let stream_next = self.interner.intern("stream_next");
        let out = self.b.reg(from, None);
        let got = self.b.value(self.program.types.optional(was), Op::Call { callee: Callee::Intrinsic(stream_next), args: vec![inner], outs: vec![out] }, span);
        let has = self.b.value(TyId::BOOL, Op::IsSome(got), span);
        self.b.terminate(Terminator::Branch { condition: has, then: more, otherwise: done });
        self.b.switch_to(more);
        let element = self.b.value(was, Op::SomePayload(got), span);
        let fitted = self.fit_value(element, was, now, span);
        let wrapped = self.b.value(next, Op::Convert { conversion: Conversion::ToOptional, operand: fitted }, span);
        self.b.terminate(Terminator::Return(Some(wrapped)));
        self.b.switch_to(done);
        self.b.terminate(Terminator::Return(Some(Operand::Const(Const::Null))));
        let function = self.end_body(false);
        self.restore(outer);
        self.functions[id] = function;
        self.b.value(to, Op::Stream { resume: id, init: vec![operand] }, span)
    }

    /// A function as a function type it fits: a wrapper converts the arguments and the result. A `Stream` gets a resume function that converts its elements; a `Task` gets a view that converts its result.
    pub(super) fn adapt(&mut self, operand: Operand, from: TyId, to: TyId, span: Span) -> Operand {
        if let Some((was, now)) = self.handle_elements(LangItem::Stream, from, to) {
            return if was == now { operand } else { self.adapt_stream(operand, from, to, was, now, span) };
        }
        if let Some((was, now)) = self.handle_elements(LangItem::Task, from, to) {
            return if was == now { operand } else { self.adapt_task(operand, from, to, was, now, span) };
        }
        let (TyKind::Function(source), TyKind::Function(target)) = (self.program.types.kind(from).clone(), self.program.types.kind(to).clone()) else {
            return self.b.value(to, Op::Convert { conversion: Conversion::Adapt, operand }, span);
        };
        if source.params == target.params && source.ret == target.ret {
            return operand;
        }
        let kind = FunctionKind::Closure { parent: self.func };
        let id = self.functions.alloc(placeholder(kind.clone(), target.ret));
        let outer = self.begin(id, kind, target.ret);
        let held = self.b.capture(from, None, false);
        let params: Vec<RegId> = target.params.iter().map(|param| self.b.param(param.ty, None, param.is_var)).collect();
        let mut args = Vec::new();
        let mut outs = Vec::new();
        for (position, (want, have)) in target.params.iter().zip(&source.params).enumerate() {
            args.push(self.fit_value(Operand::Reg(params[position]), want.ty, have.ty, span));
            if have.is_var {
                outs.push((position, self.b.reg(have.ty, None)));
            }
        }
        let call = Op::Call { callee: Callee::Value(Operand::Reg(held)), args, outs: outs.iter().map(|&(_, out)| out).collect() };
        let result = if source.ret == TyId::UNIT {
            self.b.emit(None, call, span);
            Operand::Const(Const::Null)
        } else {
            self.b.value(source.ret, call, span)
        };
        for (position, out) in outs {
            self.b.emit(Some(params[position]), Op::Copy(Operand::Reg(out)), span);
        }
        let value = (target.ret != TyId::UNIT).then(|| self.fit_value(result, source.ret, target.ret, span));
        self.b.terminate(Terminator::Return(value));
        let function = self.end_body(false);
        self.restore(outer);
        self.functions[id] = function;
        self.b.value(to, Op::Closure { func: id, captures: vec![operand] }, span)
    }
}
