//! The result of a scalar operation on constants, when it cannot fault.

use std::cmp::Ordering;

use mote_semantics::{BinaryOp, FloatTy, IntTy, TyKind, UnaryOp};

use crate::ir::Const;

/// `value` as a number of the type: the low bits, read as signed or unsigned.
fn wrap(value: i128, int: IntTy) -> i128 {
    let bits = int.bits();
    if bits >= 128 {
        return value;
    }
    let low = value & ((1i128 << bits) - 1);
    if int.signed() && low >= 1i128 << (bits - 1) { low - (1i128 << bits) } else { low }
}

fn round(x: f64, float: FloatTy) -> f64 {
    match float {
        FloatTy::F32 => f64::from(x as f32),
        FloatTy::F64 => x,
    }
}

/// A float cut to an integer type: toward zero, saturating, `NaN` as 0.
fn float_to_int(x: f64, int: IntTy) -> i128 {
    if x.is_nan() {
        return 0;
    }
    let cut = x.trunc();
    if cut <= int.min() as f64 {
        int.min()
    } else if cut >= int.max() as f64 {
        int.max()
    } else {
        cut as i128
    }
}

fn int_result(int: IntTy, value: Option<i128>) -> Option<Const> {
    value.filter(|found| int.fits(*found)).map(Const::Int)
}

fn compare(left: &Const, right: &Const) -> Option<Ordering> {
    match (left, right) {
        (Const::Int(a), Const::Int(b)) => Some(a.cmp(b)),
        (Const::Float(a), Const::Float(b)) => a.partial_cmp(b),
        _ => None,
    }
}

fn equal(left: &Const, right: &Const) -> bool {
    match (left, right) {
        (Const::Null, Const::Null) => true,
        (Const::Bool(a), Const::Bool(b)) => a == b,
        (Const::Int(a), Const::Int(b)) => a == b,
        (Const::Float(a), Const::Float(b)) => a == b,
        _ => false,
    }
}

pub(super) fn unary(op: UnaryOp, operand: &Const, ty: &TyKind) -> Option<Const> {
    match (op, operand, ty) {
        (UnaryOp::Not, Const::Bool(b), _) => Some(Const::Bool(!b)),
        (UnaryOp::Neg, Const::Int(n), TyKind::Int(int)) => int_result(*int, n.checked_neg()),
        (UnaryOp::Neg, Const::Float(x), _) => Some(Const::Float(-x)),
        (UnaryOp::BitNot, Const::Int(n), TyKind::Int(int)) => Some(Const::Int(wrap(!n, *int))),
        _ => None,
    }
}

pub(super) fn binary(op: BinaryOp, left: &Const, right: &Const, ty: &TyKind) -> Option<Const> {
    use BinaryOp::*;
    if matches!(left, Const::String(_)) || matches!(right, Const::String(_)) {
        return None;
    }
    match op {
        Eq => return Some(Const::Bool(equal(left, right))),
        NotEq => return Some(Const::Bool(!equal(left, right))),
        Lt => return Some(Const::Bool(compare(left, right) == Some(Ordering::Less))),
        LtEq => return Some(Const::Bool(matches!(compare(left, right), Some(Ordering::Less | Ordering::Equal)))),
        Gt => return Some(Const::Bool(compare(left, right) == Some(Ordering::Greater))),
        GtEq => return Some(Const::Bool(matches!(compare(left, right), Some(Ordering::Greater | Ordering::Equal)))),
        _ => {}
    }
    match (left, right) {
        (Const::Bool(a), Const::Bool(b)) => match op {
            And | BitAnd => Some(Const::Bool(*a && *b)),
            Or | BitOr => Some(Const::Bool(*a || *b)),
            BitXor => Some(Const::Bool(a != b)),
            _ => None,
        },
        (Const::Int(a), Const::Int(b)) => {
            let TyKind::Int(int) = ty else { return None };
            int_binary(op, *a, *b, *int)
        }
        (Const::Float(a), Const::Float(b)) => {
            let float = if let TyKind::Float(float) = ty { *float } else { FloatTy::F64 };
            let result = match op {
                Add => a + b,
                Sub => a - b,
                Mul => a * b,
                Div => a / b,
                Rem => a % b,
                _ => return None,
            };
            Some(Const::Float(round(result, float)))
        }
        _ => None,
    }
}

fn int_binary(op: BinaryOp, a: i128, b: i128, int: IntTy) -> Option<Const> {
    use BinaryOp::*;
    match op {
        Add => int_result(int, a.checked_add(b)),
        Sub => int_result(int, a.checked_sub(b)),
        Mul => int_result(int, a.checked_mul(b)),
        Div | Rem if b == 0 => None,
        Div => int_result(int, a.checked_div(b)),
        Rem => int_result(int, a.checked_rem(b)),
        BitAnd => Some(Const::Int(a & b)),
        BitOr => Some(Const::Int(a | b)),
        BitXor => Some(Const::Int(a ^ b)),
        Shl | Shr => {
            if b < 0 || b >= i128::from(int.bits()) {
                return None;
            }
            let amount = b as u32;
            Some(Const::Int(if op == Shl { wrap(a.wrapping_shl(amount), int) } else { a >> amount }))
        }
        _ => None,
    }
}

/// `value as T` for a number or `Bool`, where `to` is the type of the receiving register.
pub(super) fn cast(value: &Const, to: &TyKind) -> Option<Const> {
    match (value, to) {
        (Const::Int(n), TyKind::Int(int)) => Some(Const::Int(wrap(*n, *int))),
        (Const::Int(n), TyKind::Float(float)) => Some(Const::Float(round(*n as f64, *float))),
        (Const::Float(x), TyKind::Int(int)) => Some(Const::Int(float_to_int(*x, *int))),
        (Const::Float(x), TyKind::Float(float)) => Some(Const::Float(round(*x, *float))),
        (Const::Bool(b), TyKind::Int(_)) => Some(Const::Int(i128::from(*b))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use mote_base::Interner;

    use super::*;

    #[test]
    fn constants_text_is_never_folded_and_numbers_fold_unless_they_fault() {
        let text = Const::String(Interner::new().intern("a"));
        assert_eq!(binary(BinaryOp::Eq, &text, &text, &TyKind::Bool), None);
        let int = TyKind::Int(IntTy::I8);
        assert_eq!(binary(BinaryOp::Add, &Const::Int(2), &Const::Int(3), &int), Some(Const::Int(5)));
        assert_eq!(binary(BinaryOp::Add, &Const::Int(127), &Const::Int(1), &int), None);
        assert_eq!(binary(BinaryOp::Div, &Const::Int(1), &Const::Int(0), &int), None);
        assert_eq!(binary(BinaryOp::Lt, &Const::Int(1), &Const::Int(2), &TyKind::Bool), Some(Const::Bool(true)));
    }
}
