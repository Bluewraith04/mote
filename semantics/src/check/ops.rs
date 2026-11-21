use mote_base::{Id, Span};
use mote_syntax as ast;

use super::coerce::plan;
use super::FnCx;
use crate::cx::Cx;
use crate::defs::LangItem;
use crate::tree::{BinaryOp, Condition, Conversion, Expr, ExprKind, UnaryOp};
use crate::types::{TyId, TyKind};

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

impl Cx<'_> {
    pub(crate) fn is_number(&self, ty: TyId) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Int(_) | TyKind::Float(_))
    }

    pub(crate) fn is_integer(&self, ty: TyId) -> bool {
        matches!(self.program.types.kind(ty), TyKind::Int(_))
    }

    /// Whether the expression is a number literal with no suffix, which takes its type from the other side.
    fn is_untyped_literal(ast: &ast::Ast, id: Id<ast::Expr>) -> bool {
        match &ast[id].kind {
            ast::ExprKind::Int(literal) => literal.suffix.is_none(),
            ast::ExprKind::Float(literal) => literal.suffix.is_none(),
            ast::ExprKind::Paren(inner) => Self::is_untyped_literal(ast, *inner),
            ast::ExprKind::Unary { op: ast::UnaryOp::Neg, operand } => Self::is_untyped_literal(ast, *operand),
            _ => false,
        }
    }

    /// Checks both operands, letting a bare literal take the type of the side that has one.
    pub(crate) fn operands(&mut self, f: &mut FnCx, left: Id<ast::Expr>, right: Id<ast::Expr>, hint: Option<TyId>) -> (Id<Expr>, Id<Expr>) {
        let ast = self.ast(f.module);
        if Self::is_untyped_literal(ast, left) && !Self::is_untyped_literal(ast, right) {
            let right = self.expr(f, right, hint);
            let left = self.expr(f, left, Some(f.body.exprs[right].ty));
            (left, right)
        } else {
            let left = self.expr(f, left, hint);
            let right = self.expr(f, right, Some(f.body.exprs[left].ty));
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
        let (tree_op, allowed) = match op {
            ast::UnaryOp::Neg => {
                let signed = match self.program.types.kind(ty) {
                    TyKind::Int(int) => int.signed(),
                    TyKind::Float(_) => true,
                    _ => false,
                };
                (UnaryOp::Neg, signed)
            }
            ast::UnaryOp::Not => (UnaryOp::Not, ty == TyId::BOOL),
            ast::UnaryOp::BitNot => (UnaryOp::BitNot, self.is_integer(ty)),
        };
        if !allowed {
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
        let (lt, rt) = (f.body.exprs[left].ty, f.body.exprs[right].ty);
        if lt == TyId::ERROR || rt == TyId::ERROR {
            return self.error_expr(f, span);
        }
        if matches!(tree_op, BinaryOp::Eq | BinaryOp::NotEq) {
            return match self.equality_operands(f, left, right, span) {
                Some((left, right)) => f.body.expr(ExprKind::Binary { op: tree_op, left, right }, TyId::BOOL, span),
                None => self.error_expr(f, span),
            };
        }
        match self.binary_type(tree_op, lt, rt, span) {
            Some(ty) => f.body.expr(ExprKind::Binary { op: tree_op, left, right }, ty, span),
            None => self.error_expr(f, span),
        }
    }

    /// The result type of an arithmetic, bitwise or ordering operator, or `None` after reporting.
    pub(crate) fn binary_type(&mut self, op: BinaryOp, lt: TyId, rt: TyId, span: Span) -> Option<TyId> {
        if lt != rt {
            let message = format!("`{}` needs one type on both sides, found {} and {}", op_text(op), self.show_ty(lt), self.show_ty(rt));
            self.error(span, message);
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
            let message = format!("`{}` does not work on {}", op_text(op), self.show_ty(lt));
            self.error(span, message);
            return None;
        }
        Some(match op {
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => TyId::BOOL,
            _ => lt,
        })
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
            TyKind::Function(_) => false,
            TyKind::Param { index, .. } => {
                let eq = self.program.lang_item(LangItem::Eq);
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
