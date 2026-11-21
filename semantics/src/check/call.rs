use mote_base::{Id, Span};
use mote_syntax as ast;
use mote_syntax::Ident;

use super::FnCx;
use crate::cx::Cx;
use crate::defs::{DefId, DefKind, GenericParam, ModuleId, Param, Receiver};
use crate::tree::{Callee, Conversion, Expr, ExprKind, FnRef};
use crate::types::{ConstArg, ConstVal, GenericArg, TyId, TyKind};

/// The arguments of a call: as written, or one already checked.
pub(crate) enum Args<'a> {
    Written(&'a [ast::Arg]),
    Checked(&'a [Id<Expr>]),
}

#[derive(Clone, Copy)]
enum Source {
    Written(Id<ast::Expr>),
    Checked(Id<Expr>),
}

/// The `args` positions in evaluation order, when it is not their own order.
fn evaluation_order(written: &[usize], total: usize, offset: u32) -> Option<Vec<u32>> {
    let mut order: Vec<usize> = written.to_vec();
    order.extend((0..total).filter(|index| !written.contains(index)));
    if order.iter().enumerate().all(|(position, &index)| position == index) {
        return None;
    }
    Some((0..offset).chain(order.into_iter().map(|index| index as u32 + offset)).collect())
}

/// Arguments paired with their parameters; `pending` are the values still to be made to fit.
struct Matched {
    values: Vec<Id<Expr>>,
    pending: Vec<usize>,
    rest: Vec<Id<Expr>>,
    /// Parameter positions in the order the arguments were written.
    written: Vec<usize>,
}

impl Cx<'_> {
    pub(crate) fn call(&mut self, f: &mut FnCx, callee: Id<ast::Expr>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        if let Some(node) = self.named_call(f, callee, args, expected, span) {
            return node;
        }
        let callee_node = self.expr(f, callee, None);
        let ty = f.body.exprs[callee_node].ty;
        match self.program.types.kind(ty).clone() {
            TyKind::Error => self.error_expr(f, span),
            TyKind::Function(function) => self.call_value(f, callee_node, &function.params, function.ret, args, span),
            _ => {
                let message = format!("{} is not callable", self.show_ty(ty));
                self.error(span, message);
                self.error_expr(f, span)
            }
        }
    }

    /// Calls a function or method; `type_args` are its owner's, `object` is the method's `self`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_def(&mut self, f: &mut FnCx, def: DefId, type_args: &[GenericArg], object: Option<Id<Expr>>, args: &[ast::Arg], expected: Option<TyId>, span: Span) -> Id<Expr> {
        self.call_with(f, def, type_args, &[], object, Args::Written(args), expected, span)
    }

    /// A call whose own generic arguments may be written; every one left out is found from the arguments or the expected type.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_with(&mut self, f: &mut FnCx, def: DefId, owner_args: &[GenericArg], own_args: &[GenericArg], object: Option<Id<Expr>>, args: Args, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let (params, ret, module, receiver, own_generics) = {
            let item = &self.program.defs[def];
            let function = item.function().expect("a function");
            (function.params.clone(), function.ret, item.module, function.receiver, function.generics.clone())
        };
        if object.is_none()
            && owner_args.is_empty()
            && let Args::Written(written_args) = args
        {
            let mod_path = &self.program.modules[module].path;
            let is_std_array = mod_path.len() == 2
                && self.interner.resolve(mod_path[0]) == "std"
                && self.interner.resolve(mod_path[1]) == "array";
            if is_std_array
                && let Some(expr) = self.check_std_array_call(f, def, own_args, written_args, expected, span)
            {
                return expr;
            }
        }
        let owner_params = self.owner_generic_params(def);
        let owner_count = owner_params.len();
        let mut type_args = owner_args.to_vec();
        for param in owner_params.iter().skip(type_args.len()) {
            match param {
                GenericParam::Type { .. } => type_args.push(GenericArg::Type(self.fresh(f))),
                GenericParam::Const { .. } => type_args.push(GenericArg::Const(self.fresh_const(f))),
            }
        }
        let Some(own) = self.own_generic_args(f, def, own_args, span) else { return self.error_expr(f, span) };
        type_args.extend(own);
        let params: Vec<Param> = params.into_iter().map(|param| Param { ty: self.program.types.subst_generic(param.ty, &type_args), ..param }).collect();
        let ret = self.program.types.subst_generic(ret, &type_args);
        if let Some(object) = object
            && receiver == Receiver::Write
            && Self::is_place(f, object)
            && !self.writable(f, object)
        {
            let object_span = f.body.exprs[object].span;
            self.error(object_span, "this place has no `var` path to write through");
        }
        let Some(Matched { mut values, pending, rest, written }) = self.match_args(f, &params, args, module, span) else { return self.error_expr(f, span) };
        let ret_open = self.solved(f, ret);
        if let Some(expected) = expected
            && self.program.types.has_infer(ret_open)
        {
            self.unify(f, ret_open, expected);
        }
        let failed = values.iter().chain(&rest).any(|&value| f.body.exprs[value].ty == TyId::ERROR);
        let mut found = Vec::new();
        for (index, argument) in type_args.iter().enumerate() {
            let mut resolved = self.solved_generic(f, argument);
            if let GenericArg::Const(c) = &resolved {
                resolved = GenericArg::Const(crate::comptime::normalize_const_arg(self, c));
            }
            if self.program.types.generic_has_infer(&resolved) {
                if !failed {
                    let name = self.text(self.generic_names(def)[index]);
                    self.error(span, format!("cannot infer the generic argument `{name}`"));
                }
                return self.error_expr(f, span);
            }
            found.push(resolved);
        }
        for index in pending {
            let ty = self.solved(f, params[index].ty);
            let value_span = f.body.exprs[values[index]].span;
            values[index] = self.coerce_arg(f, values[index], ty, value_span, !params[index].is_var);
        }
        if params.last().is_some_and(|param| param.variadic) {
            let element = self.solved(f, params[params.len() - 1].ty);
            let items: Vec<Id<Expr>> = rest
                .into_iter()
                .map(|item| {
                    let item_span = f.body.exprs[item].span;
                    self.coerce(f, item, element, item_span)
                })
                .collect();
            let list = self.list_of(element, span);
            values.push(f.body.expr(ExprKind::List(items), list, span));
        }
        for (index, param) in owner_params.iter().enumerate() {
            match (param, &found[index]) {
                (GenericParam::Type { bounds, .. }, GenericArg::Type(ty_arg)) => {
                    self.check_bounds(f, *ty_arg, bounds, &found, span);
                }
                (GenericParam::Const { ty, .. }, GenericArg::Const(c_arg)) => {
                    self.check_const_arg_type(c_arg, *ty, &found, span);
                }
                _ => {}
            }
        }
        for (index, param) in own_generics.iter().enumerate() {
            match (param, &found[owner_count + index]) {
                (GenericParam::Type { bounds, .. }, GenericArg::Type(ty_arg)) => {
                    self.check_bounds(f, *ty_arg, bounds, &found, span);
                }
                (GenericParam::Const { ty, .. }, GenericArg::Const(c_arg)) => {
                    self.check_const_arg_type(c_arg, *ty, &found, span);
                }
                _ => {}
            }
        }
        let solved_ret = self.solved(f, ret);
        let ret = crate::comptime::normalize_ty_consts(self, solved_ret);
        let callee = if owner_count > 0 && self.program.defs[def].owner.is_some_and(|o| matches!(self.program.defs[o].kind, DefKind::Trait(_))) {
            let self_ty = found[0].as_type().unwrap_or(TyId::ERROR);
            Callee::TraitMethod { method: def, self_ty, args: found[1..].to_vec() }
        } else {
            Callee::Function(FnRef { def, args: found })
        };
        let offset = u32::from(object.is_some());
        let order = evaluation_order(&written, values.len(), offset);
        let call = f.body.expr(ExprKind::Call { callee, args: object.into_iter().chain(values).collect() }, ret, span);
        if let Some(order) = order {
            f.body.arg_order.insert(call, order);
        }
        call
    }

    fn owner_generic_params(&self, def: DefId) -> Vec<GenericParam> {
        if let Some(ext) = self.program.defs[def].extend {
            return self.program.extends[ext].generics.clone();
        }
        let Some(owner) = self.program.defs[def].owner else { return Vec::new() };
        match &self.program.defs[owner].kind {
            DefKind::Trait(item) => {
                let mut params = vec![GenericParam::Type { name: self.self_name, bounds: Vec::new() }];
                params.extend(item.generics.clone());
                params
            }
            DefKind::Record(record) => record.generics.clone(),
            DefKind::Enum(en) => en.generics.clone(),
            _ => Vec::new(),
        }
    }

    fn check_const_arg_type(&mut self, arg: &ConstArg, param_ty: TyId, subst_args: &[GenericArg], span: Span) {
        let expected_ty = self.program.types.subst_generic(param_ty, subst_args);
        if expected_ty == TyId::ERROR {
            return;
        }
        let ConstArg::Val(val) = arg else { return };
        let ok = match (val, self.program.types.kind(expected_ty)) {
            (ConstVal::Int(n, _), TyKind::Int(int_ty)) => int_ty.fits(*n),
            (ConstVal::Bool(_), TyKind::Bool) => true,
            (ConstVal::String(_), TyKind::String) => true,
            _ => false,
        };
        if !ok {
            self.error(span, format!("const argument does not match {}", self.show_ty(expected_ty)));
        }
    }

    /// A default argument or field value, checked in the module that declares it.
    pub(crate) fn default_value(&mut self, f: &mut FnCx, module: ModuleId, default: Id<ast::Expr>, ty: TyId) -> Id<Expr> {
        let saved = f.enter(module);
        let value = self.expr_to(f, default, ty);
        f.leave(saved);
        value
    }

    /// Pairs arguments with parameters: positional first, then named, then defaults; a variadic parameter takes the rest.
    fn match_args(&mut self, f: &mut FnCx, params: &[Param], args: Args, module: crate::defs::ModuleId, span: Span) -> Option<Matched> {
        let items: Vec<(Option<Ident>, Source)> = match args {
            Args::Written(args) => args.iter().map(|arg| (arg.name, Source::Written(arg.value))).collect(),
            Args::Checked(nodes) => nodes.iter().map(|&node| (None, Source::Checked(node))).collect(),
        };
        let variadic = params.last().filter(|param| param.variadic);
        let fixed = if variadic.is_some() { &params[..params.len() - 1] } else { params };
        let mut slots: Vec<Option<(Id<Expr>, bool)>> = vec![None; fixed.len()];
        let mut rest = Vec::new();
        let mut positional = 0;
        let mut named = false;
        let mut ok = true;
        let mut written = Vec::new();
        for (name, source) in items {
            let arg_span = self.source_span(f, source);
            let index = match name {
                None => {
                    if named {
                        self.error(arg_span, "a positional argument comes before the named ones");
                        ok = false;
                        continue;
                    }
                    if positional >= fixed.len() {
                        let Some(variadic) = variadic else {
                            self.error(arg_span, "too many arguments");
                            ok = false;
                            break;
                        };
                        rest.push(self.argument(f, source, variadic.ty, false).0);
                        continue;
                    }
                    positional += 1;
                    positional - 1
                }
                Some(name) => {
                    named = true;
                    if variadic.is_some() {
                        self.error(name.span, "a variadic function takes no named arguments");
                        ok = false;
                        continue;
                    }
                    match fixed.iter().position(|param| param.name == name.name) {
                        Some(index) if slots[index].is_none() => index,
                        Some(_) => {
                            let message = format!("`{}` is given twice", self.text(name.name));
                            self.error(name.span, message);
                            ok = false;
                            continue;
                        }
                        None => {
                            let message = format!("no parameter `{}`", self.text(name.name));
                            self.error(name.span, message);
                            ok = false;
                            continue;
                        }
                    }
                }
            };
            slots[index] = Some(self.argument(f, source, fixed[index].ty, fixed[index].is_var));
            written.push(index);
        }
        let mut values = Vec::new();
        let mut pending = Vec::new();
        for (param, slot) in fixed.iter().zip(slots) {
            let value = match slot {
                Some((value, coerced)) => {
                    if !coerced {
                        pending.push(values.len());
                    }
                    value
                }
                None => match param.default {
                    Some(default) => {
                        let ty = self.solved(f, param.ty);
                        if self.program.types.has_infer(ty) {
                            let message = format!("cannot infer the type of `{}` for its default", self.text(param.name));
                            self.error(span, message);
                            ok = false;
                            continue;
                        }
                        self.default_value(f, module, default, ty)
                    }
                    None => {
                        if ok {
                            let message = format!("missing argument `{}`", self.text(param.name));
                            self.error(span, message);
                        }
                        ok = false;
                        continue;
                    }
                },
            };
            values.push(value);
        }
        ok.then_some(Matched { values, pending, rest, written })
    }

    fn source_span(&self, f: &FnCx, source: Source) -> Span {
        match source {
            Source::Written(value) => self.ast(f.module)[value].span,
            Source::Checked(node) => f.body.exprs[node].span,
        }
    }

    fn call_value(&mut self, f: &mut FnCx, callee: Id<Expr>, params: &[crate::types::FnParam], ret: TyId, args: &[ast::Arg], span: Span) -> Id<Expr> {
        let mut ok = true;
        if let Some(named) = args.iter().find_map(|arg| arg.name) {
            self.error(named.span, "a function value takes no named arguments");
            ok = false;
        }
        if args.len() != params.len() {
            let message = format!("expected {} arguments, found {}", params.len(), args.len());
            self.error(span, message);
            ok = false;
        }
        let values: Vec<Id<Expr>> = args.iter().zip(params).map(|(arg, param)| self.argument(f, Source::Written(arg.value), param.ty, param.is_var).0).collect();
        if !ok {
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Call { callee: Callee::Value(callee), args: values }, ret, span)
    }

    /// One argument, with whether it was already made to fit; a type still holding variables waits for the call's others.
    fn argument(&mut self, f: &mut FnCx, source: Source, ty: TyId, is_var: bool) -> (Id<Expr>, bool) {
        let span = self.source_span(f, source);
        let known = self.solved(f, ty);
        let node = match source {
            Source::Written(value) => {
                let lambda = matches!(self.ast(f.module)[value].kind, ast::ExprKind::Lambda { .. });
                self.expr(f, value, (lambda || !self.program.types.has_infer(known)).then_some(known))
            }
            Source::Checked(node) => node,
        };
        if is_var && Self::is_place(f, node) && !self.writable(f, node) {
            self.error(span, "this place has no `var` path to write through");
        }
        if self.program.types.has_infer(known) {
            let found = f.body.exprs[node].ty;
            self.unify(f, known, found);
            let known = self.solved(f, known);
            if self.program.types.has_infer(known) {
                return (node, false);
            }
            return (self.coerce_arg(f, node, known, span, !is_var), true);
        }
        (self.coerce_arg(f, node, known, span, !is_var), true)
    }

    /// Whether the expression names a place rather than a value.
    pub(crate) fn is_place(f: &FnCx, node: Id<Expr>) -> bool {
        match &f.body.exprs[node].kind {
            ExprKind::Convert { conversion: Conversion::Narrow, expr } => Self::is_place(f, *expr),
            kind => matches!(kind, ExprKind::Local(_) | ExprKind::Global(_) | ExprKind::Field { .. } | ExprKind::TupleField { .. } | ExprKind::Index { .. }),
        }
    }

    /// Whether a write through the place is allowed: a `var` path all the way down, and fields only inside their module.
    pub(crate) fn writable(&self, f: &FnCx, node: Id<Expr>) -> bool {
        match &f.body.exprs[node].kind {
            ExprKind::Local(local) => f.body.locals[*local].is_var,
            ExprKind::Global(def) => {
                let item = &self.program.defs[*def];
                f.top_level && item.module == f.module && matches!(&item.kind, DefKind::Global(global) if global.is_var)
            }
            ExprKind::Convert { conversion: Conversion::Narrow, expr } => self.writable(f, *expr),
            ExprKind::TupleField { object, .. } | ExprKind::Index { object, .. } => self.writable(f, *object),
            ExprKind::Field { object, index } => {
                let TyKind::Adt { def, .. } = self.program.types.kind(f.body.exprs[*object].ty) else { return false };
                let item = &self.program.defs[*def];
                let Some(record) = item.record() else { return false };
                record.fields[*index as usize].is_var && item.module == f.module && self.writable(f, *object)
            }
            _ => false,
        }
    }
}
