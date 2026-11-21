use mote_base::{Id, Span};
use mote_semantics::{BinaryOp, BuiltinTy, Conversion, Expr, TyId, TyKind, UnaryOp};

use super::Lowering;
use crate::ir::{Callee, Const, Op, Operand, RegId, Terminator};

const FIXED_UNROLL: u32 = 32;

#[derive(Clone, Copy)]
enum MaskKind {
    All,
    Any,
    None,
}

impl Lowering<'_> {
    pub(super) fn lower_array_builtin(&mut self, name: &str, args: &[Id<Expr>], ret_ty: TyId, span: Span) -> Option<Operand> {
        let out = match name {
            "mask_all" => self.lower_mask_op(args[0], None, MaskKind::All, span),
            "mask_any" => self.lower_mask_op(args[0], None, MaskKind::Any, span),
            "mask_none" => self.lower_mask_op(args[0], None, MaskKind::None, span),
            "array_all_by" => self.lower_mask_op(args[0], Some(args[1]), MaskKind::All, span),
            "array_any_by" => self.lower_mask_op(args[0], Some(args[1]), MaskKind::Any, span),
            "array_none_by" => self.lower_mask_op(args[0], Some(args[1]), MaskKind::None, span),
            "array_count" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let target = self.expr(args[1]);
                self.array_count_like(arr, arr_ty, target, false, span)
            }
            "array_count_by" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let pred = self.expr(args[1]);
                self.array_count_like(arr, arr_ty, pred, true, span)
            }
            "array_sum" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_accumulate(arr, arr_ty, BinaryOp::Add, false, span)
            }
            "array_product" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_accumulate(arr, arr_ty, BinaryOp::Mul, false, span)
            }
            "array_norm_sq" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_accumulate(arr, arr_ty, BinaryOp::Add, true, span)
            }
            "array_mean" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_mean(arr, arr_ty, span)
            }
            "array_min" | "array_max" | "array_argmin" | "array_argmax" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let want_min = matches!(name, "array_min" | "array_argmin");
                let return_index = matches!(name, "array_argmin" | "array_argmax");
                self.array_extremum(arr, arr_ty, want_min, return_index, ret_ty, span)
            }
            "array_dot" => {
                let (a, arr_ty) = self.eval_array(args[0]);
                let b = self.expr(args[1]);
                self.array_dot(a, b, arr_ty, span)
            }
            "array_fold" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let init = self.expr(args[1]);
                let f = self.expr(args[2]);
                self.array_fold(arr, arr_ty, init, f, ret_ty, span)
            }
            "array_reduce" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let f = self.expr(args[1]);
                self.array_reduce(arr, arr_ty, f, ret_ty, span)
            }
            "array_scan" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let init = self.expr(args[1]);
                let f = self.expr(args[2]);
                self.array_scan(arr, arr_ty, init, f, ret_ty, span)
            }
            "array_find" | "array_find_index" | "array_index_of" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let needle = self.expr(args[1]);
                let is_pred = !matches!(name, "array_index_of");
                let return_index = !matches!(name, "array_find");
                self.array_find_like(arr, arr_ty, needle, is_pred, return_index, ret_ty, span)
            }
            "array_select" => {
                let (mask, mask_ty) = self.eval_array(args[0]);
                let (on_true, val_ty) = self.eval_array(args[1]);
                let on_false = self.expr(args[2]);
                self.array_select(mask, mask_ty, on_true, on_false, val_ty, span)
            }
            "array_map" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let f = self.expr(args[1]);
                self.array_map(arr, arr_ty, f, ret_ty, span)
            }
            "array_zip_with" => {
                let (a, a_ty) = self.eval_array(args[0]);
                let (b, b_ty) = self.eval_array(args[1]);
                let f = self.expr(args[2]);
                self.array_zip_with(a, a_ty, b, b_ty, Some(f), ret_ty, span)
            }
            "array_zip" => {
                let (a, a_ty) = self.eval_array(args[0]);
                let (b, b_ty) = self.eval_array(args[1]);
                self.array_zip_with(a, a_ty, b, b_ty, None, ret_ty, span)
            }
            "array_reverse" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_reverse(arr, arr_ty, ret_ty, span)
            }
            "array_clamp" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let lo = self.expr(args[1]);
                let hi = self.expr(args[2]);
                self.array_clamp(arr, arr_ty, lo, hi, ret_ty, span)
            }
            "array_filter" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let pred = self.expr(args[1]);
                self.array_filter(arr, arr_ty, pred, ret_ty, span)
            }
            "array_enumerate" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_enumerate(arr, arr_ty, ret_ty, span)
            }
            "array_concat" => {
                let (a, a_ty) = self.eval_array(args[0]);
                let (b, b_ty) = self.eval_array(args[1]);
                self.array_concat(a, a_ty, b, b_ty, ret_ty, span)
            }
            "array_take" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let n = self.expr(args[1]);
                self.array_take_or_drop(arr, arr_ty, n, true, ret_ty, span)
            }
            "array_drop" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let n = self.expr(args[1]);
                self.array_take_or_drop(arr, arr_ty, n, false, ret_ty, span)
            }
            "array_flatten" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_flatten(arr, arr_ty, ret_ty, span)
            }
            "array_sort" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                self.array_sort(arr, arr_ty, None, ret_ty, span)
            }
            "array_sort_by" => {
                let (arr, arr_ty) = self.eval_array(args[0]);
                let cmp = self.expr(args[1]);
                self.array_sort(arr, arr_ty, Some(cmp), ret_ty, span)
            }
            _ => return None,
        };
        Some(out)
    }

    fn eval_array(&mut self, expr_id: Id<Expr>) -> (Operand, TyId) {
        let ty = self.body.exprs[expr_id].ty;
        (self.expr(expr_id), ty)
    }

    fn array_info(&mut self, arr: Operand, arr_ty: TyId, span: Span) -> (TyId, Operand, Option<u32>) {
        match self.program.types.kind(arr_ty) {
            TyKind::FixedArray(elem, n) => {
                let len = n.as_u32().unwrap_or(0);
                (*elem, Operand::Const(Const::Int(i128::from(len))), Some(len))
            }
            TyKind::Array(elem) => {
                let elem = *elem;
                let len = self.b.value(TyId::INT, Op::Len(arr), span);
                (elem, len, None)
            }
            _ => (TyId::ERROR, Operand::Const(Const::Int(0)), Some(0)),
        }
    }

    fn scalar_zero(&self, ty: TyId) -> Const {
        match self.program.types.kind(ty) {
            TyKind::Float(_) => Const::Float(0.0),
            _ => Const::Int(0),
        }
    }

    fn scalar_one(&self, ty: TyId) -> Const {
        match self.program.types.kind(ty) {
            TyKind::Float(_) => Const::Float(1.0),
            _ => Const::Int(1),
        }
    }

    fn call_fn(&mut self, callee: Operand, args: Vec<Operand>, ret_ty: TyId, span: Span) -> Operand {
        self.b.value(ret_ty, Op::Call { callee: Callee::Value(callee), args, outs: Vec::new() }, span)
    }

    fn index_elem(&mut self, arr: Operand, elem_ty: TyId, index: Operand, span: Span) -> Operand {
        self.b.value(elem_ty, Op::Index { object: arr, index }, span)
    }

    fn emit_fault(&mut self, msg: &str, span: Span) {
        let fault = self.interner.intern("fault");
        let text = self.interner.intern(msg);
        self.b.emit(None, Op::Call { callee: Callee::Intrinsic(fault), args: vec![Operand::Const(Const::String(text))], outs: Vec::new() }, span);
        self.b.terminate(Terminator::Unreachable);
    }

    fn emit_for_range(&mut self, start: Operand, end: Operand, span: Span, body_fn: impl FnOnce(&mut Self, Operand)) {
        let idx = self.b.reg(TyId::INT, None);
        self.b.emit(Some(idx), Op::Copy(start), span);
        let head = self.b.block();
        let body = self.b.block();
        let exit = self.b.block();
        self.b.terminate(Terminator::Jump(head));

        self.b.switch_to(head);
        let cond = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: Operand::Reg(idx), right: end }, span);
        self.b.terminate(Terminator::Branch { condition: cond, then: body, otherwise: exit });

        self.b.switch_to(body);
        body_fn(self, Operand::Reg(idx));
        if !self.b.is_terminated() {
            let next = self.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(idx), right: Operand::Const(Const::Int(1)) }, span);
            self.b.emit(Some(idx), Op::Copy(next), span);
            self.b.terminate(Terminator::Jump(head));
        }

        self.b.switch_to(exit);
    }

    /// Runs `nonempty_fn` only when the array has >= 1 element; otherwise sets `res` via `empty_fn`.
    fn with_nonempty_check(
        &mut self,
        len: Operand,
        fixed_len: Option<u32>,
        ret_ty: TyId,
        span: Span,
        empty_fn: impl FnOnce(&mut Self) -> Operand,
        nonempty_fn: impl FnOnce(&mut Self) -> Operand,
    ) -> Operand {
        match fixed_len {
            Some(0) => empty_fn(self),
            Some(_) => nonempty_fn(self),
            None => {
                let res = self.b.reg(ret_ty, None);
                let is_empty = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Eq, left: len, right: Operand::Const(Const::Int(0)) }, span);
                let empty_bb = self.b.block();
                let non_bb = self.b.block();
                let join_bb = self.b.block();
                self.b.terminate(Terminator::Branch { condition: is_empty, then: empty_bb, otherwise: non_bb });

                self.b.switch_to(empty_bb);
                let ev = empty_fn(self);
                self.b.emit(Some(res), Op::Copy(ev), span);
                self.b.terminate(Terminator::Jump(join_bb));

                self.b.switch_to(non_bb);
                let nv = nonempty_fn(self);
                self.b.emit(Some(res), Op::Copy(nv), span);
                self.b.terminate(Terminator::Jump(join_bb));

                self.b.switch_to(join_bb);
                Operand::Reg(res)
            }
        }
    }

    fn lower_mask_op(&mut self, arr_expr: Id<Expr>, pred_expr: Option<Id<Expr>>, kind: MaskKind, span: Span) -> Operand {
        let (arr, arr_ty) = self.eval_array(arr_expr);
        let pred = pred_expr.map(|id| self.expr(id));
        if pred.is_none()
            && let TyKind::FixedArray(_, ref len) = *self.program.types.kind(arr_ty)
            && let Some(len) = len.as_u32()
            && len <= FIXED_UNROLL
        {
            return match kind {
                MaskKind::All => self.mask_reduction(true, arr, arr_ty, span),
                MaskKind::Any => self.mask_reduction(false, arr, arr_ty, span),
                MaskKind::None if len == 0 => Operand::Const(Const::Bool(true)),
                MaskKind::None => {
                    let any = self.mask_reduction(false, arr, arr_ty, span);
                    self.b.value(TyId::BOOL, Op::Unary { op: UnaryOp::Not, operand: any }, span)
                }
            };
        }
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        let (initial, stop_on, found_result) = match kind {
            MaskKind::All => (true, false, false),
            MaskKind::Any => (false, true, true),
            MaskKind::None => (true, true, false),
        };
        let res = self.b.reg(TyId::BOOL, None);
        self.b.emit(Some(res), Op::Copy(Operand::Const(Const::Bool(initial))), span);
        let idx = self.b.reg(TyId::INT, None);
        self.b.emit(Some(idx), Op::Copy(Operand::Const(Const::Int(0))), span);

        let head = self.b.block();
        let body = self.b.block();
        let step = self.b.block();
        let hit = self.b.block();
        let exit = self.b.block();
        self.b.terminate(Terminator::Jump(head));

        self.b.switch_to(head);
        let has_more = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: Operand::Reg(idx), right: len }, span);
        self.b.terminate(Terminator::Branch { condition: has_more, then: body, otherwise: exit });

        self.b.switch_to(body);
        let item = self.index_elem(arr, elem_ty, Operand::Reg(idx), span);
        let flag = match pred {
            Some(pred_fn) => self.call_fn(pred_fn, vec![item], TyId::BOOL, span),
            None => item,
        };
        let cond = if stop_on {
            flag
        } else {
            self.b.value(TyId::BOOL, Op::Unary { op: UnaryOp::Not, operand: flag }, span)
        };
        self.b.terminate(Terminator::Branch { condition: cond, then: hit, otherwise: step });

        self.b.switch_to(hit);
        self.b.emit(Some(res), Op::Copy(Operand::Const(Const::Bool(found_result))), span);
        self.b.terminate(Terminator::Jump(exit));

        self.b.switch_to(step);
        let next = self.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(idx), right: Operand::Const(Const::Int(1)) }, span);
        self.b.emit(Some(idx), Op::Copy(next), span);
        self.b.terminate(Terminator::Jump(head));

        self.b.switch_to(exit);
        Operand::Reg(res)
    }

    fn array_count_like(&mut self, arr: Operand, arr_ty: TyId, target_or_pred: Operand, is_pred: bool, span: Span) -> Operand {
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        let acc = self.b.reg(TyId::INT, None);
        self.b.emit(Some(acc), Op::Copy(Operand::Const(Const::Int(0))), span);
        self.emit_for_range(Operand::Const(Const::Int(0)), len, span, |this, idx| {
            let item = this.index_elem(arr, elem_ty, idx, span);
            let cond = if is_pred {
                this.call_fn(target_or_pred, vec![item], TyId::BOOL, span)
            } else {
                this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Eq, left: item, right: target_or_pred }, span)
            };
            let then_bb = this.b.block();
            let join_bb = this.b.block();
            this.b.terminate(Terminator::Branch { condition: cond, then: then_bb, otherwise: join_bb });

            this.b.switch_to(then_bb);
            let inc = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(acc), right: Operand::Const(Const::Int(1)) }, span);
            this.b.emit(Some(acc), Op::Copy(inc), span);
            this.b.terminate(Terminator::Jump(join_bb));

            this.b.switch_to(join_bb);
        });
        Operand::Reg(acc)
    }

    fn array_accumulate(&mut self, arr: Operand, arr_ty: TyId, op: BinaryOp, norm_sq: bool, span: Span) -> Operand {
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        let init = if matches!(op, BinaryOp::Mul) { self.scalar_one(elem_ty) } else { self.scalar_zero(elem_ty) };
        let acc = self.b.reg(elem_ty, None);
        self.b.emit(Some(acc), Op::Copy(Operand::Const(init)), span);
        self.emit_for_range(Operand::Const(Const::Int(0)), len, span, |this, idx| {
            let mut val = this.index_elem(arr, elem_ty, idx, span);
            if norm_sq {
                val = this.b.value(elem_ty, Op::Binary { op: BinaryOp::Mul, left: val, right: val }, span);
            }
            let next = this.b.value(elem_ty, Op::Binary { op, left: Operand::Reg(acc), right: val }, span);
            this.b.emit(Some(acc), Op::Copy(next), span);
        });
        Operand::Reg(acc)
    }

    fn array_mean(&mut self, arr: Operand, arr_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        self.with_nonempty_check(
            len,
            fixed_len,
            TyId::FLOAT,
            span,
            |_| Operand::Const(Const::Float(0.0)),
            |this| {
                let acc = this.b.reg(TyId::FLOAT, None);
                this.b.emit(Some(acc), Op::Copy(Operand::Const(Const::Float(0.0))), span);
                this.emit_for_range(Operand::Const(Const::Int(0)), len, span, |this, idx| {
                    let val = this.index_elem(arr, elem_ty, idx, span);
                    let fval = if elem_ty == TyId::FLOAT { val } else { this.b.value(TyId::FLOAT, Op::Cast(val), span) };
                    let next = this.b.value(TyId::FLOAT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(acc), right: fval }, span);
                    this.b.emit(Some(acc), Op::Copy(next), span);
                });
                let denom = this.b.value(TyId::FLOAT, Op::Cast(len), span);
                this.b.value(TyId::FLOAT, Op::Binary { op: BinaryOp::Div, left: Operand::Reg(acc), right: denom }, span)
            },
        )
    }

    fn array_extremum(&mut self, arr: Operand, arr_ty: TyId, want_min: bool, return_index: bool, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        let cmp_op = if want_min { BinaryOp::Lt } else { BinaryOp::Gt };
        self.with_nonempty_check(
            len,
            fixed_len,
            ret_ty,
            span,
            |_| Operand::Const(Const::Null),
            |this| {
                let first = this.index_elem(arr, elem_ty, Operand::Const(Const::Int(0)), span);
                let best_val = this.b.reg(elem_ty, None);
                this.b.emit(Some(best_val), Op::Copy(first), span);
                let best_idx = this.b.reg(TyId::INT, None);
                this.b.emit(Some(best_idx), Op::Copy(Operand::Const(Const::Int(0))), span);

                this.emit_for_range(Operand::Const(Const::Int(1)), len, span, |this, idx| {
                    let item = this.index_elem(arr, elem_ty, idx, span);
                    let better = this.b.value(TyId::BOOL, Op::Binary { op: cmp_op, left: item, right: Operand::Reg(best_val) }, span);
                    let update = this.b.block();
                    let join = this.b.block();
                    this.b.terminate(Terminator::Branch { condition: better, then: update, otherwise: join });

                    this.b.switch_to(update);
                    this.b.emit(Some(best_val), Op::Copy(item), span);
                    this.b.emit(Some(best_idx), Op::Copy(idx), span);
                    this.b.terminate(Terminator::Jump(join));

                    this.b.switch_to(join);
                });
                let inner = if return_index { Operand::Reg(best_idx) } else { Operand::Reg(best_val) };
                this.b.value(ret_ty, Op::Convert { conversion: Conversion::ToOptional, operand: inner }, span)
            },
        )
    }

    fn array_dot(&mut self, a: Operand, b: Operand, arr_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len_a, fixed_len) = self.array_info(a, arr_ty, span);
        if fixed_len.is_none() {
            let len_b = self.b.value(TyId::INT, Op::Len(b), span);
            let mismatch = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::NotEq, left: len_a, right: len_b }, span);
            let err_bb = self.b.block();
            let ok_bb = self.b.block();
            self.b.terminate(Terminator::Branch { condition: mismatch, then: err_bb, otherwise: ok_bb });

            self.b.switch_to(err_bb);
            self.emit_fault("dot: arrays must have equal length", span);

            self.b.switch_to(ok_bb);
        }
        let acc = self.b.reg(elem_ty, None);
        self.b.emit(Some(acc), Op::Copy(Operand::Const(self.scalar_zero(elem_ty))), span);
        self.emit_for_range(Operand::Const(Const::Int(0)), len_a, span, |this, idx| {
            let va = this.index_elem(a, elem_ty, idx, span);
            let vb = this.index_elem(b, elem_ty, idx, span);
            let prod = this.b.value(elem_ty, Op::Binary { op: BinaryOp::Mul, left: va, right: vb }, span);
            let next = this.b.value(elem_ty, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(acc), right: prod }, span);
            this.b.emit(Some(acc), Op::Copy(next), span);
        });
        Operand::Reg(acc)
    }

    fn array_fold(&mut self, arr: Operand, arr_ty: TyId, init: Operand, f: Operand, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        let acc = self.b.reg(ret_ty, None);
        self.b.emit(Some(acc), Op::Copy(init), span);
        self.emit_for_range(Operand::Const(Const::Int(0)), len, span, |this, idx| {
            let item = this.index_elem(arr, elem_ty, idx, span);
            let next = this.call_fn(f, vec![Operand::Reg(acc), item], ret_ty, span);
            this.b.emit(Some(acc), Op::Copy(next), span);
        });
        Operand::Reg(acc)
    }

    fn array_reduce(&mut self, arr: Operand, arr_ty: TyId, f: Operand, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        self.with_nonempty_check(
            len,
            fixed_len,
            ret_ty,
            span,
            |_| Operand::Const(Const::Null),
            |this| {
                let first = this.index_elem(arr, elem_ty, Operand::Const(Const::Int(0)), span);
                let acc = this.b.reg(elem_ty, None);
                this.b.emit(Some(acc), Op::Copy(first), span);
                this.emit_for_range(Operand::Const(Const::Int(1)), len, span, |this, idx| {
                    let item = this.index_elem(arr, elem_ty, idx, span);
                    let next = this.call_fn(f, vec![Operand::Reg(acc), item], elem_ty, span);
                    this.b.emit(Some(acc), Op::Copy(next), span);
                });
                this.b.value(ret_ty, Op::Convert { conversion: Conversion::ToOptional, operand: Operand::Reg(acc) }, span)
            },
        )
    }

    fn array_scan(&mut self, arr: Operand, arr_ty: TyId, init: Operand, f: Operand, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        let out_elem = match self.program.types.kind(ret_ty) {
            TyKind::Array(el) | TyKind::FixedArray(el, _) => *el,
            _ => TyId::ERROR,
        };
        self.with_nonempty_check(
            len,
            fixed_len,
            ret_ty,
            span,
            |this| this.b.value(ret_ty, Op::Array(Vec::new()), span),
            |this| {
                let item0 = this.index_elem(arr, elem_ty, Operand::Const(Const::Int(0)), span);
                let first = this.call_fn(f, vec![init, item0], out_elem, span);
                let acc = this.b.reg(out_elem, None);
                this.b.emit(Some(acc), Op::Copy(first), span);
                let out = this.b.reg(ret_ty, None);
                this.b.emit(Some(out), Op::ArrayRepeat { value: first, len }, span);
                this.emit_for_range(Operand::Const(Const::Int(1)), len, span, |this, idx| {
                    let item = this.index_elem(arr, elem_ty, idx, span);
                    let next = this.call_fn(f, vec![Operand::Reg(acc), item], out_elem, span);
                    this.b.emit(Some(acc), Op::Copy(next), span);
                    this.b.emit(None, Op::SetIndex { object: Operand::Reg(out), index: idx, value: next }, span);
                });
                Operand::Reg(out)
            },
        )
    }

    fn output_element(&self, arr_ty: TyId) -> TyId {
        match self.program.types.kind(arr_ty) {
            TyKind::Array(el) | TyKind::FixedArray(el, _) => *el,
            _ => TyId::ERROR,
        }
    }

    fn min_op(&mut self, a: Operand, b: Operand, span: Span) -> Operand {
        let res = self.b.reg(TyId::INT, None);
        let a_lt_b = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: a, right: b }, span);
        let then_bb = self.b.block();
        let else_bb = self.b.block();
        let join_bb = self.b.block();
        self.b.terminate(Terminator::Branch { condition: a_lt_b, then: then_bb, otherwise: else_bb });
        self.b.switch_to(then_bb);
        self.b.emit(Some(res), Op::Copy(a), span);
        self.b.terminate(Terminator::Jump(join_bb));
        self.b.switch_to(else_bb);
        self.b.emit(Some(res), Op::Copy(b), span);
        self.b.terminate(Terminator::Jump(join_bb));
        self.b.switch_to(join_bb);
        Operand::Reg(res)
    }

    fn clamp_count(&mut self, n: Operand, max_len: Operand, span: Span) -> Operand {
        let res = self.b.reg(TyId::INT, None);
        let neg = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: n, right: Operand::Const(Const::Int(0)) }, span);
        let neg_bb = self.b.block();
        let nonneg_bb = self.b.block();
        let over_bb = self.b.block();
        let ok_bb = self.b.block();
        let join_bb = self.b.block();
        self.b.terminate(Terminator::Branch { condition: neg, then: neg_bb, otherwise: nonneg_bb });
        self.b.switch_to(neg_bb);
        self.b.emit(Some(res), Op::Copy(Operand::Const(Const::Int(0))), span);
        self.b.terminate(Terminator::Jump(join_bb));
        self.b.switch_to(nonneg_bb);
        let over = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Gt, left: n, right: max_len }, span);
        self.b.terminate(Terminator::Branch { condition: over, then: over_bb, otherwise: ok_bb });
        self.b.switch_to(over_bb);
        self.b.emit(Some(res), Op::Copy(max_len), span);
        self.b.terminate(Terminator::Jump(join_bb));
        self.b.switch_to(ok_bb);
        self.b.emit(Some(res), Op::Copy(n), span);
        self.b.terminate(Terminator::Jump(join_bb));
        self.b.switch_to(join_bb);
        Operand::Reg(res)
    }

    fn build_array_from_indices(&mut self, len: Operand, fixed_len: Option<u32>, ret_ty: TyId, span: Span, elem_at: impl Fn(&mut Self, Operand) -> Operand) -> Operand {
        self.with_nonempty_check(
            len,
            fixed_len,
            ret_ty,
            span,
            |this| this.b.value(ret_ty, Op::Array(Vec::new()), span),
            |this| {
                let first = elem_at(this, Operand::Const(Const::Int(0)));
                let out = this.b.reg(ret_ty, None);
                this.b.emit(Some(out), Op::ArrayRepeat { value: first, len }, span);
                this.emit_for_range(Operand::Const(Const::Int(1)), len, span, |this, idx| {
                    let val = elem_at(this, idx);
                    this.b.emit(None, Op::SetIndex { object: Operand::Reg(out), index: idx, value: val }, span);
                });
                Operand::Reg(out)
            },
        )
    }

    fn collect_list_to_array(&mut self, elem_ty: TyId, ret_ty: TyId, span: Span, fill: impl FnOnce(&mut Self, RegId)) -> Operand {
        let list_ty = self.program.types.builtin(BuiltinTy::List, vec![elem_ty]);
        let buf = self.b.reg(list_ty, None);
        self.b.emit(Some(buf), Op::List(Vec::new()), span);
        fill(self, buf);
        let count = self.b.value(TyId::INT, Op::Len(Operand::Reg(buf)), span);
        self.build_array_from_indices(count, None, ret_ty, span, |this, idx| {
            this.index_elem(Operand::Reg(buf), elem_ty, idx, span)
        })
    }

    fn push_list(&mut self, list: RegId, item: Operand, span: Span) {
        let push = self.interner.intern("list_push");
        self.b.emit(None, Op::Call { callee: Callee::Intrinsic(push), args: vec![Operand::Reg(list), item], outs: vec![list] }, span);
    }

    #[allow(clippy::too_many_arguments)]
    fn array_find_like(&mut self, arr: Operand, arr_ty: TyId, needle: Operand, is_pred: bool, return_index: bool, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        let res = self.b.reg(ret_ty, None);
        self.b.emit(Some(res), Op::Copy(Operand::Const(Const::Null)), span);
        let idx = self.b.reg(TyId::INT, None);
        self.b.emit(Some(idx), Op::Copy(Operand::Const(Const::Int(0))), span);

        let head = self.b.block();
        let body = self.b.block();
        let hit = self.b.block();
        let step = self.b.block();
        let exit = self.b.block();
        self.b.terminate(Terminator::Jump(head));

        self.b.switch_to(head);
        let more = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: Operand::Reg(idx), right: len }, span);
        self.b.terminate(Terminator::Branch { condition: more, then: body, otherwise: exit });

        self.b.switch_to(body);
        let item = self.index_elem(arr, elem_ty, Operand::Reg(idx), span);
        let found = if is_pred {
            self.call_fn(needle, vec![item], TyId::BOOL, span)
        } else {
            self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Eq, left: item, right: needle }, span)
        };
        self.b.terminate(Terminator::Branch { condition: found, then: hit, otherwise: step });

        self.b.switch_to(hit);
        let payload = if return_index { Operand::Reg(idx) } else { item };
        let some = self.b.value(ret_ty, Op::Convert { conversion: Conversion::ToOptional, operand: payload }, span);
        self.b.emit(Some(res), Op::Copy(some), span);
        self.b.terminate(Terminator::Jump(exit));

        self.b.switch_to(step);
        let next = self.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(idx), right: Operand::Const(Const::Int(1)) }, span);
        self.b.emit(Some(idx), Op::Copy(next), span);
        self.b.terminate(Terminator::Jump(head));

        self.b.switch_to(exit);
        Operand::Reg(res)
    }

    fn array_select(&mut self, mask: Operand, mask_ty: TyId, on_true: Operand, on_false: Operand, val_ty: TyId, span: Span) -> Operand {
        let (_, len, fixed_len) = self.array_info(mask, mask_ty, span);
        let elem_ty = self.output_element(val_ty);
        if fixed_len.is_none() {
            let len_t = self.b.value(TyId::INT, Op::Len(on_true), span);
            let len_f = self.b.value(TyId::INT, Op::Len(on_false), span);
            let bad_t = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::NotEq, left: len, right: len_t }, span);
            let bad_f = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::NotEq, left: len, right: len_f }, span);
            let mismatch = self.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Or, left: bad_t, right: bad_f }, span);
            let err_bb = self.b.block();
            let ok_bb = self.b.block();
            self.b.terminate(Terminator::Branch { condition: mismatch, then: err_bb, otherwise: ok_bb });
            self.b.switch_to(err_bb);
            self.emit_fault("select: arrays must have equal length", span);
            self.b.switch_to(ok_bb);
        }
        self.build_array_from_indices(len, fixed_len, val_ty, span, |this, idx| {
            let cond = this.index_elem(mask, TyId::BOOL, idx, span);
            let picked = this.b.reg(elem_ty, None);
            let t_bb = this.b.block();
            let f_bb = this.b.block();
            let join_bb = this.b.block();
            this.b.terminate(Terminator::Branch { condition: cond, then: t_bb, otherwise: f_bb });
            this.b.switch_to(t_bb);
            let tv = this.index_elem(on_true, elem_ty, idx, span);
            this.b.emit(Some(picked), Op::Copy(tv), span);
            this.b.terminate(Terminator::Jump(join_bb));
            this.b.switch_to(f_bb);
            let fv = this.index_elem(on_false, elem_ty, idx, span);
            this.b.emit(Some(picked), Op::Copy(fv), span);
            this.b.terminate(Terminator::Jump(join_bb));
            this.b.switch_to(join_bb);
            Operand::Reg(picked)
        })
    }

    fn array_map(&mut self, arr: Operand, arr_ty: TyId, f: Operand, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        let out_elem = self.output_element(ret_ty);
        self.build_array_from_indices(len, fixed_len, ret_ty, span, |this, idx| {
            let item = this.index_elem(arr, elem_ty, idx, span);
            this.call_fn(f, vec![item], out_elem, span)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn array_zip_with(&mut self, a: Operand, a_ty: TyId, b: Operand, b_ty: TyId, f: Option<Operand>, ret_ty: TyId, span: Span) -> Operand {
        let (e1, len_a, fixed_a) = self.array_info(a, a_ty, span);
        let (e2, len_b, _) = self.array_info(b, b_ty, span);
        let out_elem = self.output_element(ret_ty);
        let len = if fixed_a.is_some() { len_a } else { self.min_op(len_a, len_b, span) };
        self.build_array_from_indices(len, fixed_a, ret_ty, span, |this, idx| {
            let va = this.index_elem(a, e1, idx, span);
            let vb = this.index_elem(b, e2, idx, span);
            match f {
                Some(func) => this.call_fn(func, vec![va, vb], out_elem, span),
                None => this.b.value(out_elem, Op::Tuple(vec![va, vb]), span),
            }
        })
    }

    fn array_reverse(&mut self, arr: Operand, arr_ty: TyId, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        self.build_array_from_indices(len, fixed_len, ret_ty, span, |this, idx| {
            let last = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Sub, left: len, right: Operand::Const(Const::Int(1)) }, span);
            let src = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Sub, left: last, right: idx }, span);
            this.index_elem(arr, elem_ty, src, span)
        })
    }

    fn array_clamp(&mut self, arr: Operand, arr_ty: TyId, lo: Operand, hi: Operand, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        self.build_array_from_indices(len, fixed_len, ret_ty, span, |this, idx| {
            let val = this.index_elem(arr, elem_ty, idx, span);
            let out = this.b.reg(elem_ty, None);
            let below = this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: val, right: lo }, span);
            let lo_bb = this.b.block();
            let check_hi = this.b.block();
            let hi_bb = this.b.block();
            let keep_bb = this.b.block();
            let join_bb = this.b.block();
            this.b.terminate(Terminator::Branch { condition: below, then: lo_bb, otherwise: check_hi });
            this.b.switch_to(lo_bb);
            this.b.emit(Some(out), Op::Copy(lo), span);
            this.b.terminate(Terminator::Jump(join_bb));
            this.b.switch_to(check_hi);
            let above = this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Gt, left: val, right: hi }, span);
            this.b.terminate(Terminator::Branch { condition: above, then: hi_bb, otherwise: keep_bb });
            this.b.switch_to(hi_bb);
            this.b.emit(Some(out), Op::Copy(hi), span);
            this.b.terminate(Terminator::Jump(join_bb));
            this.b.switch_to(keep_bb);
            this.b.emit(Some(out), Op::Copy(val), span);
            this.b.terminate(Terminator::Jump(join_bb));
            this.b.switch_to(join_bb);
            Operand::Reg(out)
        })
    }

    fn array_enumerate(&mut self, arr: Operand, arr_ty: TyId, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        let out_elem = self.output_element(ret_ty);
        self.build_array_from_indices(len, fixed_len, ret_ty, span, |this, idx| {
            let item = this.index_elem(arr, elem_ty, idx, span);
            this.b.value(out_elem, Op::Tuple(vec![idx, item]), span)
        })
    }

    fn array_filter(&mut self, arr: Operand, arr_ty: TyId, pred: Operand, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        self.collect_list_to_array(elem_ty, ret_ty, span, |this, buf| {
            this.emit_for_range(Operand::Const(Const::Int(0)), len, span, |this, idx| {
                let item = this.index_elem(arr, elem_ty, idx, span);
                let keep = this.call_fn(pred, vec![item], TyId::BOOL, span);
                let then_bb = this.b.block();
                let join_bb = this.b.block();
                this.b.terminate(Terminator::Branch { condition: keep, then: then_bb, otherwise: join_bb });
                this.b.switch_to(then_bb);
                this.push_list(buf, item, span);
                this.b.terminate(Terminator::Jump(join_bb));
                this.b.switch_to(join_bb);
            });
        })
    }

    fn array_flatten(&mut self, arr: Operand, arr_ty: TyId, ret_ty: TyId, span: Span) -> Operand {
        let (inner_ty, outer_len, _) = self.array_info(arr, arr_ty, span);
        let elem_ty = self.output_element(ret_ty);
        self.collect_list_to_array(elem_ty, ret_ty, span, |this, buf| {
            this.emit_for_range(Operand::Const(Const::Int(0)), outer_len, span, |this, i| {
                let sub = this.index_elem(arr, inner_ty, i, span);
                let (_, sub_len, _) = this.array_info(sub, inner_ty, span);
                this.emit_for_range(Operand::Const(Const::Int(0)), sub_len, span, |this, j| {
                    let item = this.index_elem(sub, elem_ty, j, span);
                    this.push_list(buf, item, span);
                });
            });
        })
    }

    fn array_take_or_drop(&mut self, arr: Operand, arr_ty: TyId, n: Operand, is_take: bool, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, _) = self.array_info(arr, arr_ty, span);
        let bound = self.clamp_count(n, len, span);
        let (start, count) = if is_take {
            (Operand::Const(Const::Int(0)), bound)
        } else {
            let count = self.b.value(TyId::INT, Op::Binary { op: BinaryOp::Sub, left: len, right: bound }, span);
            (bound, count)
        };
        self.build_array_from_indices(count, None, ret_ty, span, |this, idx| {
            let src = if is_take { idx } else { this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: start, right: idx }, span) };
            this.index_elem(arr, elem_ty, src, span)
        })
    }

    fn array_concat(&mut self, a: Operand, a_ty: TyId, b: Operand, b_ty: TyId, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len_a, _) = self.array_info(a, a_ty, span);
        let (_, len_b, _) = self.array_info(b, b_ty, span);
        let total = self.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: len_a, right: len_b }, span);
        self.with_nonempty_check(
            total,
            None,
            ret_ty,
            span,
            |this| this.b.value(ret_ty, Op::Array(Vec::new()), span),
            |this| {
                let first = this.b.reg(elem_ty, None);
                let has_a = this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Gt, left: len_a, right: Operand::Const(Const::Int(0)) }, span);
                let from_a = this.b.block();
                let from_b = this.b.block();
                let alloc_bb = this.b.block();
                this.b.terminate(Terminator::Branch { condition: has_a, then: from_a, otherwise: from_b });
                this.b.switch_to(from_a);
                let a0 = this.index_elem(a, elem_ty, Operand::Const(Const::Int(0)), span);
                this.b.emit(Some(first), Op::Copy(a0), span);
                this.b.terminate(Terminator::Jump(alloc_bb));
                this.b.switch_to(from_b);
                let b0 = this.index_elem(b, elem_ty, Operand::Const(Const::Int(0)), span);
                this.b.emit(Some(first), Op::Copy(b0), span);
                this.b.terminate(Terminator::Jump(alloc_bb));

                this.b.switch_to(alloc_bb);
                let out = this.b.reg(ret_ty, None);
                this.b.emit(Some(out), Op::ArrayRepeat { value: Operand::Reg(first), len: total }, span);
                this.emit_for_range(Operand::Const(Const::Int(0)), len_a, span, |this, i| {
                    let va = this.index_elem(a, elem_ty, i, span);
                    this.b.emit(None, Op::SetIndex { object: Operand::Reg(out), index: i, value: va }, span);
                });
                this.emit_for_range(Operand::Const(Const::Int(0)), len_b, span, |this, i| {
                    let vb = this.index_elem(b, elem_ty, i, span);
                    let dst = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: len_a, right: i }, span);
                    this.b.emit(None, Op::SetIndex { object: Operand::Reg(out), index: dst, value: vb }, span);
                });
                Operand::Reg(out)
            },
        )
    }

    fn array_sort(&mut self, arr: Operand, arr_ty: TyId, cmp: Option<Operand>, ret_ty: TyId, span: Span) -> Operand {
        let (elem_ty, len, fixed_len) = self.array_info(arr, arr_ty, span);
        let out = self.build_array_from_indices(len, fixed_len, ret_ty, span, |this, idx| {
            this.index_elem(arr, elem_ty, idx, span)
        });
        let Operand::Reg(out_reg) = out else { return out };
        if matches!(fixed_len, Some(0) | Some(1)) {
            return Operand::Reg(out_reg);
        }
        self.emit_for_range(Operand::Const(Const::Int(1)), len, span, |this, i| {
            let key = this.index_elem(Operand::Reg(out_reg), elem_ty, i, span);
            let j = this.b.reg(TyId::INT, None);
            let j0 = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Sub, left: i, right: Operand::Const(Const::Int(1)) }, span);
            this.b.emit(Some(j), Op::Copy(j0), span);

            let w_cond = this.b.block();
            let w_cmp = this.b.block();
            let w_shift = this.b.block();
            let w_done = this.b.block();
            this.b.terminate(Terminator::Jump(w_cond));

            this.b.switch_to(w_cond);
            let ge0 = this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::GtEq, left: Operand::Reg(j), right: Operand::Const(Const::Int(0)) }, span);
            this.b.terminate(Terminator::Branch { condition: ge0, then: w_cmp, otherwise: w_done });

            this.b.switch_to(w_cmp);
            let cur = this.index_elem(Operand::Reg(out_reg), elem_ty, Operand::Reg(j), span);
            let less = match cmp {
                Some(cmp_fn) => {
                    let ord = this.call_fn(cmp_fn, vec![key, cur], TyId::INT, span);
                    this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: ord, right: Operand::Const(Const::Int(0)) }, span)
                }
                None => this.b.value(TyId::BOOL, Op::Binary { op: BinaryOp::Lt, left: key, right: cur }, span),
            };
            this.b.terminate(Terminator::Branch { condition: less, then: w_shift, otherwise: w_done });

            this.b.switch_to(w_shift);
            let j_plus_1 = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(j), right: Operand::Const(Const::Int(1)) }, span);
            this.b.emit(None, Op::SetIndex { object: Operand::Reg(out_reg), index: j_plus_1, value: cur }, span);
            let j_minus_1 = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Sub, left: Operand::Reg(j), right: Operand::Const(Const::Int(1)) }, span);
            this.b.emit(Some(j), Op::Copy(j_minus_1), span);
            this.b.terminate(Terminator::Jump(w_cond));

            this.b.switch_to(w_done);
            let insert_at = this.b.value(TyId::INT, Op::Binary { op: BinaryOp::Add, left: Operand::Reg(j), right: Operand::Const(Const::Int(1)) }, span);
            this.b.emit(None, Op::SetIndex { object: Operand::Reg(out_reg), index: insert_at, value: key }, span);
        });
        Operand::Reg(out_reg)
    }
}
