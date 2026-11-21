use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{ElseExpr, FloatLiteral, FloatSuffix, IntLiteral, IntSuffix};

use super::FnCx;
use crate::comptime;
use crate::cx::Cx;
use crate::defs::{DefId, DefKind, GenericParam, Receiver, Resolved};
use crate::tree::{Conversion, Expr, ExprKind, FnRef};
use crate::types::{ConstArg, ConstVal, FnParam, GenericArg, TyId, TyKind};

fn is_int(kind: &TyKind) -> bool {
    matches!(kind, TyKind::Int(_))
}

fn is_float(kind: &TyKind) -> bool {
    matches!(kind, TyKind::Float(_))
}

impl Cx<'_> {
    /// Checks an expression; `expected` only steers literals and branches, `coerce` enforces it.
    pub(crate) fn expr(&mut self, f: &mut FnCx, id: Id<ast::Expr>, expected: Option<TyId>) -> Id<Expr> {
        let ast = self.ast(f.module);
        let node = &ast[id];
        let span = node.span;
        match &node.kind {
            ast::ExprKind::Int(literal) => self.int_literal(f, literal, span, expected, false),
            ast::ExprKind::Float(literal) => self.float_literal(f, literal, span, expected, false),
            ast::ExprKind::String(text) => f.body.expr(ExprKind::String(*text), TyId::STRING, span),
            ast::ExprKind::Interpolated { head, parts } => {
                let mut pieces = Vec::new();
                if !self.text(*head).is_empty() {
                    pieces.push(f.body.expr(ExprKind::String(*head), TyId::STRING, span));
                }
                for part in parts {
                    let value = self.expr(f, part.value, None);
                    pieces.push(self.show(f, value));
                    if !self.text(part.text).is_empty() {
                        pieces.push(f.body.expr(ExprKind::String(part.text), TyId::STRING, span));
                    }
                }
                f.body.expr(ExprKind::Concat(pieces), TyId::STRING, span)
            }
            ast::ExprKind::Bool(value) => f.body.expr(ExprKind::Bool(*value), TyId::BOOL, span),
            ast::ExprKind::Null => f.body.expr(ExprKind::Null, TyId::UNIT, span),
            ast::ExprKind::Ident(name) => self.ident(f, *name, expected, span),
            ast::ExprKind::SelfValue => {
                let name = self.interner.intern("self");
                match f.lookup(name) {
                    Some(local) => f.body.expr(ExprKind::Local(local), f.body.locals[local].ty, span),
                    None => {
                        self.error(span, "`self` is only inside a method");
                        self.error_expr(f, span)
                    }
                }
            }
            ast::ExprKind::Paren(inner) => self.expr(f, *inner, expected),
            ast::ExprKind::Tuple(items) => {
                let hints: Option<Vec<TyId>> = match expected.map(|ty| self.program.types.kind(ty)) {
                    Some(TyKind::Tuple(elements)) if elements.len() == items.len() => Some(elements.clone()),
                    _ => None,
                };
                let elements: Vec<Id<Expr>> = items.iter().enumerate().map(|(index, &item)| self.expr(f, item, hints.as_ref().map(|hints| hints[index]))).collect();
                let types = elements.iter().map(|&element| f.body.exprs[element].ty).collect();
                let ty = self.program.types.tuple(types);
                f.body.expr(ExprKind::Tuple(elements), ty, span)
            }
            ast::ExprKind::TupleIndex { object, index } => self.tuple_index(f, *object, index.value, span),
            ast::ExprKind::Unary { op, operand } => self.unary(f, *op, *operand, expected, span),
            ast::ExprKind::Binary { op, left, right } => self.binary(f, *op, *left, *right, expected, span),
            ast::ExprKind::Cast { expr, ty } => self.cast(f, *expr, *ty, span),
            ast::ExprKind::Call { callee, args } => self.call(f, *callee, args, expected, span),
            ast::ExprKind::Builtin { name, generic_args, args } => self.check_builtin(f, *name, generic_args, args, expected, span),
            ast::ExprKind::Member { object, name, .. } => self.member(f, *object, *name, expected, span),
            ast::ExprKind::StructInit { path, args, fields } => self.struct_init(f, path, args, fields, expected, span),
            ast::ExprKind::Comptime(block) => self.comptime_expr(f, block, expected, span),
            ast::ExprKind::ComptimeIf { condition, then, otherwise } => self.comptime_value_if(f, condition, *then, otherwise, expected, span),
            ast::ExprKind::If { condition, then, otherwise } => self.value_if(f, condition, *then, otherwise, expected, span),
            ast::ExprKind::Match { subject, arms } => self.value_match(f, *subject, arms, expected, span),
            ast::ExprKind::OptionalMember { object, name, .. } => self.optional_member(f, *object, *name, span),
            ast::ExprKind::Propagate(operand) => self.propagate(f, *operand, span),
            ast::ExprKind::Unwrap(operand) => self.unwrap(f, *operand, span),
            ast::ExprKind::TypeTest { expr, ty } => self.type_test(f, *expr, *ty, span),
            ast::ExprKind::Lambda { params, ret, body } => self.lambda(f, params, *ret, body, expected, span),
            ast::ExprKind::List(items) => self.list_literal(f, items, expected, span),
            ast::ExprKind::ArrayRepeat { value, len } => self.array_repeat(f, *value, *len, expected, span),
            ast::ExprKind::Map(entries) => self.map_literal(f, entries, expected, span),
            ast::ExprKind::Range { start, end, inclusive } => self.range_expr(f, *start, *end, *inclusive, expected, span),
            ast::ExprKind::Index { object, index } => self.index_expr(f, *object, *index, expected, span),
            ast::ExprKind::Slice { object, start, end, inclusive, step } => self.slice_expr(f, *object, *start, *end, *inclusive, *step, expected, span),
            ast::ExprKind::Spawn(body) => self.spawn_expr(f, body, span),
            ast::ExprKind::TypeApply { .. } => {
                self.error(span, "a type is not a value");
                self.error_expr(f, span)
            }
            ast::ExprKind::Error => self.error_expr(f, span),
        }
    }

    /// Checks an expression and makes it a value of `ty`.
    pub(crate) fn expr_to(&mut self, f: &mut FnCx, id: Id<ast::Expr>, ty: TyId) -> Id<Expr> {
        let mut ty = self.solved(f, ty);
        let open = self.program.types.has_infer(ty);
        let node = self.expr(f, id, (!open).then_some(ty));
        if open {
            let found = f.body.exprs[node].ty;
            self.unify(f, ty, found);
            ty = self.solved(f, ty);
            if self.program.types.has_infer(ty) {
                return node;
            }
        }
        let span = self.ast(f.module)[id].span;
        self.coerce(f, node, ty, span)
    }

    pub(crate) fn error_expr(&mut self, f: &mut FnCx, span: Span) -> Id<Expr> {
        f.body.expr(ExprKind::Error, TyId::ERROR, span)
    }

    /// The numeric type an expected type asks a literal to take.
    fn literal_hint(&self, expected: Option<TyId>, accept: fn(&TyKind) -> bool) -> Option<TyId> {
        let mut ty = expected?;
        loop {
            match self.program.types.kind(ty) {
                TyKind::Optional(inner) => ty = *inner,
                TyKind::Union(members) => {
                    let mut found = members.iter().copied().filter(|&member| accept(self.program.types.kind(member)));
                    let first = found.next()?;
                    return found.next().is_none().then_some(first);
                }
                kind if accept(kind) => return Some(ty),
                _ => return None,
            }
        }
    }

    pub(crate) fn int_literal(&mut self, f: &mut FnCx, literal: &IntLiteral, span: Span, expected: Option<TyId>, negative: bool) -> Id<Expr> {
        let Ok(magnitude) = i128::try_from(literal.value) else {
            self.error(span, "integer literal is too large");
            return self.error_expr(f, span);
        };
        let value = if negative { -magnitude } else { magnitude };
        let ty = match literal.suffix {
            Some(IntSuffix::I8) => TyId::INT8,
            Some(IntSuffix::I16) => TyId::INT16,
            Some(IntSuffix::I32) => TyId::INT32,
            Some(IntSuffix::I64) => TyId::INT64,
            Some(IntSuffix::U8) => TyId::UINT8,
            Some(IntSuffix::U16) => TyId::UINT16,
            Some(IntSuffix::U32) => TyId::UINT32,
            Some(IntSuffix::U64) => TyId::UINT64,
            None => self.literal_hint(expected, is_int).unwrap_or(TyId::INT),
        };
        if let TyKind::Int(int) = self.program.types.kind(ty)
            && !int.fits(value)
        {
            let message = format!("{value} does not fit {}", int.name());
            self.error(span, message);
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Int(value), ty, span)
    }

    pub(crate) fn float_literal(&mut self, f: &mut FnCx, literal: &FloatLiteral, span: Span, expected: Option<TyId>, negative: bool) -> Id<Expr> {
        let ty = match literal.suffix {
            Some(FloatSuffix::F32) => TyId::FLOAT32,
            Some(FloatSuffix::F64) => TyId::FLOAT64,
            None => self.literal_hint(expected, is_float).unwrap_or(TyId::FLOAT),
        };
        let value = if negative { -literal.value } else { literal.value };
        f.body.expr(ExprKind::Float(value), ty, span)
    }

    /// Whether `to_string` is built in for the type: a tuple, union, array, fixed array or function whose parts can be written.
    pub(crate) fn builtin_show(&mut self, f: &FnCx, ty: TyId) -> bool {
        match self.program.types.kind(ty).clone() {
            TyKind::Tuple(members) | TyKind::Union(members) => members.into_iter().all(|member| self.can_show(f, member)),
            TyKind::Array(element) | TyKind::FixedArray(element, _) => self.can_show(f, element),
            TyKind::Function(_) | TyKind::Any => true,
            _ => false,
        }
    }

    fn can_show(&mut self, f: &FnCx, ty: TyId) -> bool {
        match self.program.types.kind(ty) {
            TyKind::String | TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool | TyKind::Unit => true,
            TyKind::Param { index, .. } => f.bounds.get(*index as usize).is_some_and(|bounds| bounds.iter().any(|bound| self.program.std.display == Some(bound.def))),
            TyKind::Tuple(_) | TyKind::Union(_) | TyKind::Any | TyKind::Array(_) | TyKind::FixedArray(..) | TyKind::Function(_) => self.builtin_show(f, ty),
            _ => {
                let Some(name) = self.interner.get("to_string") else { return false };
                self.methods_for_type(ty, name)
                    .into_iter()
                    .any(|(method, _)| matches!(self.program.defs[method].function(), Some(function) if function.receiver == Receiver::Read && function.params.is_empty()))
            }
        }
    }

    /// A value as the text an interpolation writes.
    fn show(&mut self, f: &mut FnCx, value: Id<Expr>) -> Id<Expr> {
        let ty = self.solved(f, f.body.exprs[value].ty);
        let span = f.body.exprs[value].span;
        let to_string = self.interner.intern("to_string");
        let has_to_string = !matches!(self.program.types.kind(ty), TyKind::Param { .. }) && !self.methods_for_type(ty, to_string).is_empty();
        let is_builtin_show = self.builtin_show(f, ty);
        match self.program.types.kind(ty).clone() {
            TyKind::String | TyKind::Error => value,
            TyKind::Int(_) | TyKind::Float(_) | TyKind::Bool => f.body.expr(ExprKind::Show(value), TyId::STRING, span),
            _ if has_to_string => self.method_call_on(f, value, mote_syntax::Ident { name: to_string, span }, &[], &[], Some(TyId::STRING), span),
            _ if is_builtin_show => f.body.expr(ExprKind::Show(value), TyId::STRING, span),
            TyKind::Tuple(_) | TyKind::Union(_) | TyKind::Array(_) | TyKind::FixedArray(..) =>
            {
                let message = format!("{} cannot be written in a string", self.show_ty(ty));
                self.error(span, message);
                self.error_expr(f, span)
            }
            _ => self.method_call_on(f, value, mote_syntax::Ident { name: to_string, span }, &[], &[], Some(TyId::STRING), span),
        }
    }

    pub(crate) fn ident(&mut self, f: &mut FnCx, name: Symbol, expected: Option<TyId>, span: Span) -> Id<Expr> {
        if let Some((val, mut ty)) = f.lookup_const(name) {
            if let (ConstVal::Int(n, _), Some(exp)) = (&val, expected)
                && let TyKind::Int(int) = self.program.types.kind(exp)
                && int.fits(*n)
            {
                ty = exp;
            }
            return comptime::const_val_to_expr(self.program, &mut f.body, &val, ty, span);
        }
        if let Some(local) = f.lookup(name) {
            let ty = f.body.locals[local].ty;
            let node = f.body.expr(ExprKind::Local(local), ty, span);
            return match f.narrowed_ty(local) {
                Some(narrow) if narrow != ty => f.body.expr(ExprKind::Convert { conversion: Conversion::Narrow, expr: node }, narrow, span),
                _ => node,
            };
        }
        if let Some(pos) = f.scope.params.iter().position(|&p| p == name)
            && f.scope.is_const_param(pos)
        {
            return f.body.expr(ExprKind::ConstArg(ConstArg::Param { index: pos as u32, name }), TyId::INT, span);
        }
        match self.lookup(f.module, name) {
            Some(Resolved::Def(def)) => self.def_value(f, def, expected, span),
            Some(Resolved::Module(_)) => {
                let message = format!("`{}` is a module, not a value", self.text(name));
                self.error(span, message);
                self.error_expr(f, span)
            }
            None => {
                if let Some((ty, index)) = self.variant_hint(expected, name) {
                    return self.unit_variant(f, ty, index, span);
                }
                if self.text(name) == "None" {
                    return f.body.expr(ExprKind::Null, TyId::UNIT, span);
                }
                let message = format!("unknown name `{}`", self.text(name));
                self.error(span, message);
                self.error_expr(f, span)
            }
        }
    }

    /// A name's value; `expected` fixes the type arguments of a generic function.
    pub(crate) fn def_value(&mut self, f: &mut FnCx, def: DefId, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let name = self.text(self.program.defs[def].name);
        match &self.program.defs[def].kind {
            DefKind::Global(_) => {
                let ty = self.global_type(def);
                f.body.expr(ExprKind::Global(def), ty, span)
            }
            DefKind::Const(_) => {
                let val = self.ensure_const_def(def);
                let ty = match &self.program.defs[def].kind {
                    DefKind::Const(c) => c.ty,
                    _ => TyId::ERROR,
                };
                comptime::const_val_to_expr(self.program, &mut f.body, &val, ty, span)
            }
            DefKind::Function(function) if function.generics.is_empty() => {
                let ty = self.fn_type(def);
                let ty = self.sendable_function(f, ty, expected);
                f.body.expr(ExprKind::Function(FnRef { def, args: Vec::new() }), ty, span)
            }
            DefKind::Function(function) => {
                let params_spec = function.generics.clone();
                let Some(args) = self.own_generic_args(f, def, &[], span) else { return self.error_expr(f, span) };
                let declared = self.fn_type(def);
                let ty = self.program.types.subst_generic(declared, &args);
                if let Some(expected) = expected {
                    self.unify(f, ty, expected);
                }
                let mut found = Vec::new();
                for (index, argument) in args.iter().enumerate() {
                    let argument = self.solved_generic(f, argument);
                    if self.program.types.generic_has_infer(&argument) {
                        let missing = self.text(self.generic_names(def)[index]);
                        self.error(span, format!("cannot infer the generic argument `{missing}`"));
                        return self.error_expr(f, span);
                    }
                    found.push(argument);
                }
                for (index, param) in params_spec.iter().enumerate() {
                    if let (GenericParam::Type { bounds, .. }, GenericArg::Type(ty_arg)) = (param, &found[index]) {
                        self.check_bounds(f, *ty_arg, bounds, &found, span);
                    }
                }
                let ty = self.solved(f, ty);
                let ty = self.sendable_function(f, ty, expected);
                f.body.expr(ExprKind::Function(FnRef { def, args: found }), ty, span)
            }
            _ => {
                self.error(span, format!("`{name}` is not a value"));
                self.error_expr(f, span)
            }
        }
    }

    /// A named function has no captures, so it is Sendable where a `Send` function is expected.
    fn sendable_function(&mut self, f: &FnCx, ty: TyId, expected: Option<TyId>) -> TyId {
        if !self.function_hint(f, expected).is_some_and(|hint| hint.send) {
            return ty;
        }
        match self.program.types.kind(ty).clone() {
            TyKind::Function(function) => self.program.types.function(function.params, function.ret, true),
            _ => ty,
        }
    }

    /// The type of a free function used as a value; a variadic parameter is the list it arrives as.
    pub(crate) fn fn_type(&mut self, def: DefId) -> TyId {
        let (params, ret, span) = {
            let item = &self.program.defs[def];
            let function = item.function().expect("a function");
            (function.params.clone(), function.ret, item.span)
        };
        let params = params
            .iter()
            .map(|param| FnParam { ty: if param.variadic { self.list_of(param.ty, span) } else { param.ty }, is_var: param.is_var })
            .collect();
        self.program.types.function(params, ret, false)
    }

    fn tuple_index(&mut self, f: &mut FnCx, object: Id<ast::Expr>, index: u128, span: Span) -> Id<Expr> {
        let object = self.expr(f, object, None);
        let ty = f.body.exprs[object].ty;
        match self.program.types.kind(ty) {
            TyKind::Error => self.error_expr(f, span),
            TyKind::Tuple(elements) => match elements.get(index as usize).copied() {
                Some(element) => f.body.expr(ExprKind::TupleField { object, index: index as u32 }, element, span),
                None => {
                    let message = format!("{} has no element {index}", self.show_ty(ty));
                    self.error(span, message);
                    self.error_expr(f, span)
                }
            },
            _ => {
                let message = format!("{} is not a tuple", self.show_ty(ty));
                self.error(span, message);
                self.error_expr(f, span)
            }
        }
    }

    fn value_if(&mut self, f: &mut FnCx, condition: &ast::Condition, then: Id<ast::Expr>, otherwise: &ElseExpr, expected: Option<TyId>, span: Span) -> Id<Expr> {
        f.push_scope();
        let condition = self.condition(f, condition);
        self.assume(f, &condition, true);
        let then = self.expr(f, then, expected);
        f.pop_scope();
        let then_ty = f.body.exprs[then].ty;
        let hint = expected.or(Some(then_ty));
        f.push_scope();
        self.assume(f, &condition, false);
        let otherwise = match otherwise {
            ElseExpr::If(inner) | ElseExpr::Value(inner) => self.expr(f, *inner, hint),
        };
        f.pop_scope();
        let (then, otherwise, ty) = self.join(f, then, otherwise, span);
        f.body.expr(ExprKind::If { condition, then, otherwise }, ty, span)
    }

    /// Checks `comptime { ... }` expression by evaluating it at compile time.
    fn comptime_expr(&mut self, f: &mut FnCx, block: &ast::Block, expected: Option<TyId>, span: Span) -> Id<Expr> {
        match comptime::eval_comptime_block(self, f, block) {
            Ok(Some((val, mut ty))) => {
                if let (ConstVal::Int(n, _), Some(exp)) = (&val, expected)
                    && let TyKind::Int(int) = self.program.types.kind(exp)
                    && int.fits(*n)
                {
                    ty = exp;
                }
                comptime::const_val_to_expr(self.program, &mut f.body, &val, ty, span)
            }
            Ok(None) => f.body.expr(ExprKind::Null, TyId::UNIT, span),
            Err(msg) => {
                self.error(span, msg);
                self.error_expr(f, span)
            }
        }
    }

    /// Checks `comptime if cond then a else b` as an expression, evaluating `cond` and checking only the taken branch.
    fn comptime_value_if(&mut self, f: &mut FnCx, condition: &ast::Condition, then: Id<ast::Expr>, otherwise: &ElseExpr, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ast::Condition::Expr(cond_e) = condition else {
            self.error(span, "`comptime if` does not support pattern conditions");
            return self.error_expr(f, span);
        };
        let take = match comptime::eval_ast_const_in_fn(self, f, *cond_e) {
            Ok(ConstVal::Bool(b)) => b,
            Ok(_) => {
                self.error(span, "`comptime if` condition must be a Bool");
                return self.error_expr(f, span);
            }
            Err(msg) => {
                self.error(span, msg);
                return self.error_expr(f, span);
            }
        };
        if take {
            self.expr(f, then, expected)
        } else {
            match otherwise {
                ElseExpr::If(inner) | ElseExpr::Value(inner) => self.expr(f, *inner, expected),
            }
        }
    }

    /// The type two branches share: one fits the other, or a `null` branch makes the other optional.
    fn join(&mut self, f: &mut FnCx, a: Id<Expr>, b: Id<Expr>, span: Span) -> (Id<Expr>, Id<Expr>, TyId) {
        let (joined, ty) = self.join_all(f, &[a, b], span, "branches");
        (joined[0], joined[1], ty)
    }
}
