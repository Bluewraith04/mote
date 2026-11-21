use mote_base::{Id, Span};
use mote_syntax as ast;

use super::coerce::plan;
use super::call::Args;
use super::FnCx;
use crate::cx::Cx;
use crate::defs::{Bound, DefKind, Receiver};
use crate::tree::{BinaryOp, Condition, Conversion, Expr, ExprKind, UnaryOp};
use crate::types::{GenericArg, TyId, TyKind};

/// The tree operator for a source one; `??` has none because it is rewritten.
pub(crate) fn binary_op(op: ast::BinaryOp) -> Option<BinaryOp> {
    Some(match op {
        ast::BinaryOp::Add => BinaryOp::Add,
        ast::BinaryOp::Sub => BinaryOp::Sub,
        ast::BinaryOp::Mul => BinaryOp::Mul,
        ast::BinaryOp::Div => BinaryOp::Div,
        ast::BinaryOp::Rem => BinaryOp::Rem,
        ast::BinaryOp::Eq => BinaryOp::Eq,
        ast::BinaryOp::NotEq => BinaryOp::NotEq,
        ast::BinaryOp::Lt => BinaryOp::Lt,
        ast::BinaryOp::LtEq => BinaryOp::LtEq,
        ast::BinaryOp::Gt => BinaryOp::Gt,
        ast::BinaryOp::GtEq => BinaryOp::GtEq,
        ast::BinaryOp::And => BinaryOp::And,
        ast::BinaryOp::Or => BinaryOp::Or,
        ast::BinaryOp::BitAnd => BinaryOp::BitAnd,
        ast::BinaryOp::BitOr => BinaryOp::BitOr,
        ast::BinaryOp::BitXor => BinaryOp::BitXor,
        ast::BinaryOp::Shl => BinaryOp::Shl,
        ast::BinaryOp::Shr => BinaryOp::Shr,
        ast::BinaryOp::Coalesce => return None,
    })
}

fn op_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Rem => "%",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::LtEq => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::GtEq => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::Coalesce => "??",
    }
}

fn binary_dunders(op: BinaryOp) -> Option<(&'static str, &'static str)> {
    Some(match op {
        BinaryOp::Add => ("__add__", "__radd__"),
        BinaryOp::Sub => ("__sub__", "__rsub__"),
        BinaryOp::Mul => ("__mul__", "__rmul__"),
        BinaryOp::Div => ("__div__", "__rdiv__"),
        BinaryOp::Rem => ("__rem__", "__rrem__"),
        BinaryOp::BitAnd => ("__bit_and__", "__rbit_and__"),
        BinaryOp::BitOr => ("__bit_or__", "__rbit_or__"),
        BinaryOp::BitXor => ("__bit_xor__", "__rbit_xor__"),
        BinaryOp::Shl => ("__shl__", "__rshl__"),
        BinaryOp::Shr => ("__shr__", "__rshr__"),
        BinaryOp::Eq => ("__eq__", "__eq__"),
        BinaryOp::NotEq => ("__ne__", "__ne__"),
        BinaryOp::Lt => ("__lt__", "__gt__"),
        BinaryOp::LtEq => ("__le__", "__ge__"),
        BinaryOp::Gt => ("__gt__", "__lt__"),
        BinaryOp::GtEq => ("__ge__", "__le__"),
        BinaryOp::And | BinaryOp::Or | BinaryOp::Coalesce => return None,
    })
}

impl Cx<'_> {
    pub(crate) fn is_number(&self, ty: TyId) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Int(_) | TyKind::Float(_))
    }

    pub(crate) fn is_integer(&self, ty: TyId) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Int(_))
    }

    /// Whether the expression is a number literal with no suffix, which takes its type from the other side.
    pub(crate) fn is_untyped_literal(ast: &ast::Ast, id: Id<ast::Expr>) -> bool {
        match &ast[id].kind {
            ast::ExprKind::Int(literal) => literal.suffix.is_none(),
            ast::ExprKind::Float(literal) => literal.suffix.is_none(),
            ast::ExprKind::Paren(inner) => Self::is_untyped_literal(ast, *inner),
            ast::ExprKind::Unary { op: ast::UnaryOp::Neg, operand } => Self::is_untyped_literal(ast, *operand),
            _ => false,
        }
    }

    pub(crate) fn operand_hint(&self, ty: TyId) -> TyId {
        match self.program.types.kind(ty) {
            TyKind::FixedArray(element, _) => *element,
            _ => ty,
        }
    }

    /// Checks both operands, letting a bare literal take the type of the side that has one.
    pub(crate) fn operands(&mut self, f: &mut FnCx, left: Id<ast::Expr>, right: Id<ast::Expr>, hint: Option<TyId>) -> (Id<Expr>, Id<Expr>) {
        let ast = self.ast(f.module);
        let (left_untyped, right_untyped) = (Self::is_untyped_literal(ast, left), Self::is_untyped_literal(ast, right));
        if left_untyped && !right_untyped {
            let right = self.expr(f, right, hint);
            let other = self.operand_hint(f.body.exprs[right].ty);
            let left = self.expr(f, left, Some(other));
            (left, right)
        } else {
            let left = self.expr(f, left, hint);
            let other = if right_untyped { self.operand_hint(f.body.exprs[left].ty) } else { f.body.exprs[left].ty };
            let right = self.expr(f, right, Some(other));
            (left, right)
        }
    }

    pub(crate) fn unary(&mut self, f: &mut FnCx, op: ast::UnaryOp, operand: Id<ast::Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let ast = self.ast(f.module);
        if op == ast::UnaryOp::Neg {
            match &ast[operand].kind {
                ast::ExprKind::Int(literal) => return self.int_literal(f, literal, span, expected, true),
                ast::ExprKind::Float(literal) => return self.float_literal(f, literal, span, expected, true),
                _ => {}
            }
        }
        let operand = self.expr(f, operand, expected);
        let ty = f.body.exprs[operand].ty;
        if ty == TyId::ERROR {
            return self.error_expr(f, span);
        }
        let target = match self.program.types.kind(ty) {
            TyKind::FixedArray(element, _) => *element,
            _ => ty,
        };
        let (tree_op, allowed) = match op {
            ast::UnaryOp::Neg => {
                let signed = match self.program.types.kind(target) {
                    TyKind::Int(int) => int.signed(),
                    TyKind::Float(_) => true,
                    _ => false,
                };
                (UnaryOp::Neg, signed)
            }
            ast::UnaryOp::Not => (UnaryOp::Not, target == TyId::BOOL),
            ast::UnaryOp::BitNot => (UnaryOp::BitNot, self.is_integer(target)),
        };
        if !allowed {
            let dunder = match op {
                ast::UnaryOp::Neg => "__neg__",
                ast::UnaryOp::Not => "__not__",
                ast::UnaryOp::BitNot => "__bit_not__",
            };
            if let Some(name) = self.interner.get(dunder) 
                && let Some((method, owner_args)) = self.quiet_method(f, ty, name)
                && self.receiver_of(method) != Receiver::None
            {
                return self.call_with(f, method, &owner_args, &[], Some(operand), Args::Checked(&[]), expected, span)
            }
             
            let symbol = match op {
                ast::UnaryOp::Neg => "-",
                ast::UnaryOp::Not => "!",
                ast::UnaryOp::BitNot => "~",
            };
            let message = format!("`{symbol}` does not work on {}", self.show_ty(ty));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Unary { op: tree_op, operand }, ty, span)
    }

    pub(crate) fn binary(&mut self, f: &mut FnCx, op: ast::BinaryOp, left: Id<ast::Expr>, right: Id<ast::Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let Some(tree_op) = binary_op(op) else { return self.coalesce(f, left, right, span) };
        if matches!(tree_op, BinaryOp::And | BinaryOp::Or) {
            let left = self.expr_to(f, left, TyId::BOOL);
            f.push_scope();
            let known = Condition::Expr(left);
            self.assume(f, &known, tree_op == BinaryOp::And);
            let right = self.expr_to(f, right, TyId::BOOL);
            f.pop_scope();
            return f.body.expr(ExprKind::Binary { op: tree_op, left, right }, TyId::BOOL, span);
        }
        let comparing = matches!(tree_op, BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq);
        let (left, right) = self.operands(f, left, right, if comparing { None } else { expected });
        self.binary_checked(f, tree_op, left, right, expected, span)
    }

    /// Type-checks a binary operation whose operands have already been evaluated.
    pub(crate) fn binary_checked(&mut self, f: &mut FnCx, tree_op: BinaryOp, left: Id<Expr>, right: Id<Expr>, expected: Option<TyId>, span: Span) -> Id<Expr> {
        let (lt, rt) = (f.body.exprs[left].ty, f.body.exprs[right].ty);
        if lt == TyId::ERROR || rt == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if matches!(tree_op, BinaryOp::Eq | BinaryOp::NotEq) {
            if let Some(ty) = self.fixed_array_binary_type(tree_op, lt, rt) {
                return f.body.expr(ExprKind::Binary { op: tree_op, left, right }, ty, span);
            }
            if let Some(node) = self.try_equality_dunder(f, tree_op, left, right, expected, span) {
                return node;
            }
            return match self.equality_operands(f, left, right, span) {
                Some((left, right)) => f.body.expr(ExprKind::Binary { op: tree_op, left, right }, TyId::BOOL, span),
                None => self.error_expr(f, span),
            };
        }
        if let Some(ty) = self.primitive_binary_type(tree_op, lt, rt).or_else(|| self.fixed_array_binary_type(tree_op, lt, rt)) {
            return f.body.expr(ExprKind::Binary { op: tree_op, left, right }, ty, span);
        }
        if let Some((fwd, rev)) = binary_dunders(tree_op)
            && let Some(node) = self.try_dunder_pair(f, fwd, rev, left, right, expected, span)
        {
            return node;
        }
        if matches!(tree_op, BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq)
            && lt == rt
            && let Some(node) = self.try_ord_comparison(f, tree_op, left, right, span)
        {
            return node;
        }
        match self.binary_type(tree_op, lt, rt, span) {
            Some(ty) => f.body.expr(ExprKind::Binary { op: tree_op, left, right }, ty, span),
            None => self.error_expr(f, span),
        }
    }

    pub(crate) fn fixed_array_binary_type(&mut self, op: BinaryOp, lt: TyId, rt: TyId) -> Option<TyId> {
        let (element, len) = match (self.program.types.kind(lt), self.program.types.kind(rt)) {
            (TyKind::FixedArray(le, ln), TyKind::FixedArray(re, rn)) if le == re && ln == rn => (*le, ln.clone()),
            (TyKind::FixedArray(le, ln), _) if *le == rt => (*le, ln.clone()),
            (_, TyKind::FixedArray(re, rn)) if lt == *re => (*re, rn.clone()),
            _ => return None,
        };
        let mask = match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                if !self.is_number(element) {
                    return None;
                }
                false
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                if !self.is_integer(element) && element != TyId::BOOL {
                    return None;
                }
                false
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                if !self.is_integer(element) {
                    return None;
                }
                false
            }
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                if !self.is_number(element) {
                    return None;
                }
                true
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                if !self.is_number(element) && element != TyId::BOOL {
                    return None;
                }
                true
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Coalesce => return None,
        };
        Some(if mask { self.program.types.fixed_array_arg(TyId::BOOL, len) } else { self.program.types.fixed_array_arg(element, len) })
    }

    pub(crate) fn primitive_binary_type(&self, op: BinaryOp, lt: TyId, rt: TyId) -> Option<TyId> {
        if lt != rt {
            return None;
        }
        let ok = match op {
            BinaryOp::Add => self.is_number(lt) || lt == TyId::STRING,
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => self.is_number(lt),
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::Shl | BinaryOp::Shr => self.is_integer(lt),
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => self.is_number(lt) || lt == TyId::STRING,
            BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::And | BinaryOp::Or => lt == TyId::BOOL,
            BinaryOp::Coalesce => false,
        };
        if !ok {
            return None;
        }
        Some(match op {
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => TyId::BOOL,
            _ => lt,
        })
    }

    /// The result type of an arithmetic, bitwise or ordering operator, or `None` after reporting.
    pub(crate) fn binary_type(&mut self, op: BinaryOp, lt: TyId, rt: TyId, span: Span) -> Option<TyId> {
        if let Some(ty) = self.primitive_binary_type(op, lt, rt) {
            return Some(ty);
        }
        let message = if lt != rt {
            format!("`{}` needs one type on both sides, found {} and {}", op_text(op), self.show_ty(lt), self.show_ty(rt))
        } else {
            format!("`{}` does not work on {}", op_text(op), self.show_ty(lt))
        };
        self.error(span, message);
        None
    }

    /// Whether `receiver_ty` defines a non-generic method `dunder` taking one argument that `arg_ty` fits.
    fn dunder_accepts(&mut self, f: &FnCx, receiver_ty: TyId, dunder: &str, arg_ty: TyId) -> bool {
        let Some(name) = self.interner.get(dunder) else { return false };
        let Some((method, owner_args)) = self.quiet_method(f, receiver_ty, name) else { return false };
        if self.receiver_of(method) == Receiver::None {
            return false;
        }
        let Some(function) = self.program.defs[method].function() else { return false };
        if !function.generics.is_empty() || function.params.len() != 1 {
            return false;
        }
        let expected = self.program.types.subst_generic(function.params[0].ty, &owner_args);
        arg_ty == expected || plan(self.program, arg_ty, expected).is_some()
    }

    /// Dispatches `left <op> right` via `fwd` on `left` or `rev` on `right`, preferring `rev` when `fwd` refuses `right` and `rev` matches.
    #[allow(clippy::too_many_arguments)]
    fn try_dunder_pair(&mut self, f: &mut FnCx, fwd: &str, rev: &str, left: Id<Expr>, right: Id<Expr>, expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        let (lt, rt) = (f.body.exprs[left].ty, f.body.exprs[right].ty);
        if lt != rt && !self.dunder_accepts(f, lt, fwd, rt) && self.dunder_accepts(f, rt, rev, lt) {
            return self.try_binary_dunder(f, right, left, rev, expected, true, span);
        }
        if let Some(node) = self.try_binary_dunder(f, left, right, fwd, expected, false, span) {
            return Some(node);
        }
        (lt != rt).then(|| self.try_binary_dunder(f, right, left, rev, expected, true, span)).flatten()
    }

    /// Dispatches a binary operator through a dunder method on `receiver`, preserving left-to-right order on reverse calls.
    #[allow(clippy::too_many_arguments)]
    fn try_binary_dunder(&mut self, f: &mut FnCx, receiver: Id<Expr>, arg: Id<Expr>, dunder: &str, expected: Option<TyId>, reverse: bool, span: Span) -> Option<Id<Expr>> {
        let ty = f.body.exprs[receiver].ty;
        let name = self.interner.get(dunder)?;
        let (method, owner_args) = self.quiet_method(f, ty, name)?;
        if self.receiver_of(method) == Receiver::None {
            return None;
        }
        let call = self.call_with(f, method, &owner_args, &[], Some(receiver), Args::Checked(&[arg]), expected, span);
        if reverse && matches!(f.body.exprs[call].kind, ExprKind::Call { .. }) {
            f.body.arg_order.insert(call, vec![1, 0]);
        }
        Some(call)
    }

    /// Synthesizes `<`, `<=`, `>`, `>=` from `Ord.compare(left, right) <op> 0` when no explicit dunder matched.
    fn try_ord_comparison(&mut self, f: &mut FnCx, op: BinaryOp, left: Id<Expr>, right: Id<Expr>, span: Span) -> Option<Id<Expr>> {
        let ty = f.body.exprs[left].ty;
        let ord = self.program.std.ord?;
        if self.unmet(f, ty, &Bound { def: ord, args: Vec::new() }).is_some() {
            return None;
        }
        let compare_name = self.interner.get("compare")?;
        let cmp = if matches!(self.program.types.kind(ty), TyKind::Param { .. }) {
            let DefKind::Trait(item) = &self.program.defs[ord].kind else { return None };
            let method = item.methods.iter().copied().find(|&m| self.program.defs[m].name == compare_name)?;
            self.call_with(f, method, &[GenericArg::Type(ty)], &[], Some(left), Args::Checked(&[right]), Some(TyId::INT), span)
        } else {
            let candidates = self.methods_for_type(ty, compare_name);
            let (method, owner_args) = candidates.into_iter().find(|(c, _)| self.visible(f, *c, self.program.defs[*c].is_pub))?;
            if self.receiver_of(method) == Receiver::None {
                return None;
            }
            self.call_with(f, method, &owner_args, &[], Some(left), Args::Checked(&[right]), Some(TyId::INT), span)
        };
        if f.body.exprs[cmp].ty == TyId::ERROR {
            return Some(cmp);
        }
        let zero = f.body.expr(ExprKind::Int(0), TyId::INT, span);
        Some(f.body.expr(ExprKind::Binary { op, left: cmp, right: zero }, TyId::BOOL, span))
    }

    /// Whether `==`/`!=` widens a value to match an optional, union, or `null` rather than invoking `__eq__`.
    fn is_widening_equality(&self, f: &FnCx, left: Id<Expr>, right: Id<Expr>) -> bool {
        let (lt, rt) = (f.body.exprs[left].ty, f.body.exprs[right].ty);
        if lt == rt {
            return false;
        }
        let is_null = |node: Id<Expr>| f.body.exprs[node].kind == ExprKind::Null;
        let widening = |steps: &Vec<(Conversion, TyId)>| steps.iter().all(|(conversion, _)| matches!(conversion, Conversion::ToOptional | Conversion::ToUnion));
        is_null(left)
            || is_null(right)
            || plan(self.program, rt, lt).filter(widening).is_some()
            || plan(self.program, lt, rt).filter(widening).is_some()
    }

    /// Dispatches `==`/`!=` via `__eq__` or `__ne__` before built-in structural equality.
    fn try_equality_dunder(&mut self, f: &mut FnCx, op: BinaryOp, left: Id<Expr>, right: Id<Expr>, expected: Option<TyId>, span: Span) -> Option<Id<Expr>> {
        if self.is_widening_equality(f, left, right) {
            return None;
        }
        let (lt, rt) = (f.body.exprs[left].ty, f.body.exprs[right].ty);
        let (fwd, rev) = binary_dunders(op)?;
        if let Some(node) = self.try_dunder_pair(f, fwd, rev, left, right, expected, span) {
            return Some(node);
        }
        if op == BinaryOp::NotEq {
            let eq = self
                .try_binary_dunder(f, left, right, "__eq__", expected, false, span)
                .or_else(|| (lt != rt).then(|| self.try_binary_dunder(f, right, left, "__eq__", expected, true, span)).flatten());
            if let Some(eq) = eq {
                let eq_ty = f.body.exprs[eq].ty;
                if eq_ty == TyId::ERROR {
                    return Some(eq);
                }
                if eq_ty == TyId::BOOL {
                    return Some(f.body.expr(ExprKind::Unary { op: UnaryOp::Not, operand: eq }, TyId::BOOL, span));
                }
                let message = format!("`!` does not work on {}", self.show_ty(eq_ty));
                self.error(span, message);
                return Some(self.error_expr(f, span));
            }
        }
        None
    }

    /// Brings the sides of `==` to one type: the same, or one side an optional or union holding the other.
    fn equality_operands(&mut self, f: &mut FnCx, left: Id<Expr>, right: Id<Expr>, span: Span) -> Option<(Id<Expr>, Id<Expr>)> {
        let (lt, rt) = (f.body.exprs[left].ty, f.body.exprs[right].ty);
        let is_null = |node: Id<Expr>| f.body.exprs[node].kind == ExprKind::Null;
        let narrowed_optional = |node: Id<Expr>| match &f.body.exprs[node].kind {
            ExprKind::Convert { conversion: Conversion::Narrow, expr } => matches!(self.program.types.kind(f.body.exprs[*expr].ty), TyKind::Optional(_)),
            _ => false,
        };
        let is_param = |ty: TyId| matches!(self.program.types.kind(ty), TyKind::Param { .. });
        if (is_null(left) && (narrowed_optional(right) || is_param(rt))) || (is_null(right) && (narrowed_optional(left) || is_param(lt))) {
            return Some((left, right));
        }
        let widening = |steps: &Vec<(Conversion, TyId)>| steps.iter().all(|(conversion, _)| matches!(conversion, Conversion::ToOptional | Conversion::ToUnion));
        let (left, right, ty) = if let Some(steps) = plan(self.program, rt, lt).filter(widening) {
            (left, self.apply(f, right, steps), lt)
        } else if let Some(steps) = plan(self.program, lt, rt).filter(widening) {
            (self.apply(f, left, steps), right, rt)
        } else {
            let message = format!("`==` cannot compare {} with {}", self.show_ty(lt), self.show_ty(rt));
            self.error(span, message);
            return None;
        };
        let comparable = match self.program.types.kind(ty) {
            TyKind::Function(_) | TyKind::Any => false,
            TyKind::Param { index, .. } => {
                let eq = self.program.std.eq;
                eq.is_some_and(|eq| f.bounds.get(*index as usize).is_some_and(|bounds| bounds.iter().any(|bound| bound.def == eq)))
            }
            _ => true,
        };
        if !comparable {
            let message = format!("`==` does not work on {}", self.show_ty(ty));
            self.error(span, message);
            return None;
        }
        Some((left, right))
    }

    pub(crate) fn cast(&mut self, f: &mut FnCx, operand: Id<ast::Expr>, ty: Id<ast::Type>, span: Span) -> Id<Expr> {
        let operand = self.expr(f, operand, None);
        let target = self.written_type(f, ty);
        let from = f.body.exprs[operand].ty;
        if from == TyId::ERROR || target == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if self.is_enum(from)
            && let TyKind::Int(int) = *self.program.types.kind(target)
        {
            let message = match self.enum_constants(from) {
                None => Some(format!("cannot cast {} to {}: a variant has no constant", self.show_ty(from), self.show_ty(target))),
                Some(constants) => constants.into_iter().find(|&value| !int.fits(value)).map(|value| format!("constant {value} does not fit {}", self.show_ty(target))),
            };
            if let Some(message) = message {
                self.error(span, message);
                return self.error_expr(f, span);
            }
            return f.body.expr(ExprKind::Cast(operand), target, span);
        }
        let types = &self.program.types;
        let scalar = |ty| matches!(types.kind(ty), TyKind::Int(_) | TyKind::Float(_));
        let ok = (scalar(from) && scalar(target)) || (from == TyId::BOOL && matches!(types.kind(target), TyKind::Int(_)));
        if !ok {
            let message = format!("cannot cast {} to {}", self.show_ty(from), self.show_ty(target));
            self.error(span, message);
            return self.error_expr(f, span);
        }
        f.body.expr(ExprKind::Cast(operand), target, span)
    }
}
