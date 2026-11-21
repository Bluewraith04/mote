//! Compile-time (`comptime`) evaluation, type layout computation, and lowering.

use std::collections::{HashMap, HashSet};

use mote_base::{Diagnostic, DiagnosticSink, Id, Interner, Span, Symbol};
use mote_syntax as ast;

use crate::check::FnCx;
use crate::cx::Cx;
use crate::defs::{DefKind, ModuleId, Payload, Resolved};
use crate::program::Program;
use crate::tree::{
    ArmBody, BinaryOp, Block, Body, Callee, Condition, Conversion, Else, Expr, ExprKind, FieldValue, LocalKind, Match, PatternKind,
    Stmt, StmtKind, UnaryOp,
};
use crate::types::{BuiltinTy, ConstArg, ConstVal, GenericArg, IntTy, TyId, TyKind};

const MAX_COMPTIME_STEPS: usize = 100_000;

/// Byte size of `ty` using Mote's target layout rules.
pub fn layout_size(program: &Program, ty: TyId) -> u64 {
    let mut seen = HashSet::new();
    size_inner(program, ty, &[], &mut seen)
}

/// Byte alignment of `ty`.
pub fn layout_align(program: &Program, ty: TyId) -> u64 {
    let mut seen = HashSet::new();
    align_inner(program, ty, &[], &mut seen)
}

/// Whether `ty` is laid out inline without GC-managed reference fields or boxes.
pub fn is_flat(program: &Program, ty: TyId) -> bool {
    let mut seen = HashSet::new();
    is_flat_inner(program, ty, &[], &mut seen)
}

fn is_flat_inner(program: &Program, ty: TyId, env: &[GenericArg], seen: &mut HashSet<TyId>) -> bool {
    let ty = resolve_readonly_ty(ty, program, env);
    if !seen.insert(ty) {
        return false;
    }
    let out = match program.types.kind(ty) {
        TyKind::Error | TyKind::Unit | TyKind::Bool | TyKind::Int(_) | TyKind::Float(_) => true,
        TyKind::Builtin { kind: BuiltinTy::Pointer, .. } => true,
        TyKind::String
        | TyKind::Array(_)
        | TyKind::Function(_)
        | TyKind::Any
        | TyKind::Union(_)
        | TyKind::Builtin { .. }
        | TyKind::Param { .. }
        | TyKind::Infer(_) => false,
        TyKind::Optional(inner) | TyKind::FixedArray(inner, _) => {
            let inner = *inner;
            is_flat_inner(program, inner, env, seen)
        }
        TyKind::Tuple(members) => {
            let members = members.clone();
            members.into_iter().all(|m| is_flat_inner(program, m, env, seen))
        }
        TyKind::Adt { def, args } => {
            let def = *def;
            let resolved_args: Vec<GenericArg> = args
                .iter()
                .map(|a| match a {
                    GenericArg::Type(t) => GenericArg::Type(resolve_readonly_ty(*t, program, env)),
                    GenericArg::Const(c) => GenericArg::Const(c.clone()),
                })
                .collect();
            match &program.defs[def].kind {
                DefKind::Record(r) => {
                    if r.is_class {
                        false
                    } else {
                        r.fields.iter().all(|f| is_flat_inner(program, f.ty, &resolved_args, seen))
                    }
                }
                DefKind::Enum(en) => en.variants.iter().all(|v| match &v.payload {
                    Payload::Unit => true,
                    Payload::Tuple(ts) => ts.iter().all(|&t| is_flat_inner(program, t, &resolved_args, seen)),
                    Payload::Struct(fs) => fs.iter().all(|f| is_flat_inner(program, f.ty, &resolved_args, seen)),
                }),
                _ => false,
            }
        }
    };
    seen.remove(&ty);
    out
}

fn resolve_readonly_ty(ty: TyId, program: &Program, env: &[GenericArg]) -> TyId {
    if let TyKind::Param { index, .. } = program.types.kind(ty)
        && let Some(GenericArg::Type(t)) = env.get(*index as usize)
    {
        *t
    } else {
        ty
    }
}

fn size_inner(program: &Program, ty: TyId, env: &[GenericArg], seen: &mut HashSet<TyId>) -> u64 {
    let ty = resolve_readonly_ty(ty, program, env);
    if !seen.insert(ty) {
        return 8;
    }
    let out = match program.types.kind(ty) {
        TyKind::Error | TyKind::Unit | TyKind::Param { .. } | TyKind::Infer(_) => 0,
        TyKind::Bool => 1,
        TyKind::Int(int) => u64::from(int.bits() / 8),
        TyKind::Float(float) => match float {
            crate::types::FloatTy::F32 => 4,
            crate::types::FloatTy::F64 => 8,
        },
        TyKind::String
        | TyKind::Array(_)
        | TyKind::Function(_)
        | TyKind::Builtin { .. } => 8,
        TyKind::Optional(inner) => {
            let inner = *inner;
            if is_pointer_repr(program, inner) {
                8
            } else {
                let inner_align = align_inner(program, inner, env, seen).max(1);
                let inner_size = size_inner(program, inner, env, seen);
                align_up(inner_align + inner_size, inner_align)
            }
        }
        TyKind::FixedArray(inner, len) => {
            let count = len.as_u32().unwrap_or(0) as u64;
            let elem_align = align_inner(program, *inner, env, seen).max(1);
            let stride = align_up(size_inner(program, *inner, env, seen), elem_align);
            stride.saturating_mul(count)
        }
        TyKind::Tuple(members) => {
            let members = members.clone();
            struct_layout(&members.into_iter().map(|m| (m, env.to_vec(), seen.clone())).collect::<Vec<_>>(), program)
        }
        TyKind::Union(_) | TyKind::Any => 16,
        TyKind::Adt { def, args } => {
            let def = *def;
            let resolved_args: Vec<GenericArg> = args
                .iter()
                .map(|a| match a {
                    GenericArg::Type(t) => GenericArg::Type(resolve_readonly_ty(*t, program, env)),
                    GenericArg::Const(c) => GenericArg::Const(c.clone()),
                })
                .collect();
            match &program.defs[def].kind {
                DefKind::Record(record) => {
                    if record.is_class {
                        8
                    } else {
                        let mut offset = 0u64;
                        let mut max_align = 1u64;
                        for f in &record.fields {
                            let a = align_inner(program, f.ty, &resolved_args, seen).max(1);
                            let s = size_inner(program, f.ty, &resolved_args, seen);
                            offset = align_up(offset, a);
                            offset += s;
                            max_align = max_align.max(a);
                        }
                        align_up(offset, max_align)
                    }
                }
                DefKind::Enum(en) => {
                    if en.variants.is_empty() {
                        0
                    } else if en.variants.iter().all(|v| matches!(v.payload, Payload::Unit)) {
                        if en.variants.len() <= 256 { 1 } else { 4 }
                    } else {
                        let mut max_payload = 0u64;
                        let mut max_align = 8u64;
                        for v in &en.variants {
                            let mut offset = 0u64;
                            let mut v_align = 1u64;
                            let field_tys: Vec<TyId> = match &v.payload {
                                Payload::Unit => Vec::new(),
                                Payload::Tuple(ts) => ts.clone(),
                                Payload::Struct(fs) => fs.iter().map(|f| f.ty).collect(),
                            };
                            for ft in field_tys {
                                let a = align_inner(program, ft, &resolved_args, seen).max(1);
                                let s = size_inner(program, ft, &resolved_args, seen);
                                offset = align_up(offset, a);
                                offset += s;
                                v_align = v_align.max(a);
                            }
                            max_payload = max_payload.max(align_up(offset, v_align));
                            max_align = max_align.max(v_align);
                        }
                        align_up(align_up(8, max_align) + max_payload, max_align)
                    }
                }
                _ => 8,
            }
        }
    };
    seen.remove(&ty);
    out
}

fn struct_layout(members: &[(TyId, Vec<GenericArg>, HashSet<TyId>)], program: &Program) -> u64 {
    let mut offset = 0u64;
    let mut max_align = 1u64;
    for (m, env, seen) in members {
        let mut s_clone = seen.clone();
        let a = align_inner(program, *m, env, &mut s_clone).max(1);
        let s = size_inner(program, *m, env, &mut s_clone);
        offset = align_up(offset, a);
        offset += s;
        max_align = max_align.max(a);
    }
    align_up(offset, max_align)
}

fn align_inner(program: &Program, ty: TyId, env: &[GenericArg], seen: &mut HashSet<TyId>) -> u64 {
    let ty = resolve_readonly_ty(ty, program, env);
    if !seen.insert(ty) {
        return 8;
    }
    let out = match program.types.kind(ty) {
        TyKind::Error | TyKind::Unit | TyKind::Param { .. } | TyKind::Infer(_) => 1,
        TyKind::Bool => 1,
        TyKind::Int(int) => u64::from((int.bits() / 8).clamp(1, 8)),
        TyKind::Float(float) => match float {
            crate::types::FloatTy::F32 => 4,
            crate::types::FloatTy::F64 => 8,
        },
        TyKind::String | TyKind::Array(_) | TyKind::Function(_) | TyKind::Builtin { .. } | TyKind::Union(_) | TyKind::Any => 8,
        TyKind::Optional(inner) => align_inner(program, *inner, env, seen).max(1),
        TyKind::FixedArray(inner, _) => align_inner(program, *inner, env, seen).max(1),
        TyKind::Tuple(members) => {
            let members = members.clone();
            members.into_iter().map(|m| align_inner(program, m, env, seen)).max().unwrap_or(1).max(1)
        }
        TyKind::Adt { def, args } => {
            let def = *def;
            let resolved_args: Vec<GenericArg> = args
                .iter()
                .map(|a| match a {
                    GenericArg::Type(t) => GenericArg::Type(resolve_readonly_ty(*t, program, env)),
                    GenericArg::Const(c) => GenericArg::Const(c.clone()),
                })
                .collect();
            match &program.defs[def].kind {
                DefKind::Record(record) => {
                    if record.is_class {
                        8
                    } else {
                        record
                            .fields
                            .iter()
                            .map(|f| align_inner(program, f.ty, &resolved_args, seen))
                            .max()
                            .unwrap_or(1)
                            .max(1)
                    }
                }
                DefKind::Enum(en) if en.variants.iter().all(|v| matches!(v.payload, Payload::Unit)) => {
                    if en.variants.len() <= 256 { 1 } else { 4 }
                }
                _ => 8,
            }
        }
    };
    seen.remove(&ty);
    out
}

fn is_pointer_repr(program: &Program, ty: TyId) -> bool {
    match program.types.kind(ty) {
        TyKind::String | TyKind::Array(_) | TyKind::Function(_) | TyKind::Builtin { .. } => true,
        TyKind::Adt { def, .. } => matches!(&program.defs[*def].kind, DefKind::Record(r) if r.is_class),
        _ => false,
    }
}

fn align_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        value
    } else {
        (value + align - 1) & !(align - 1)
    }
}

/// Whether `ty` is a struct or class with a field named `name`.
pub fn type_has_field(program: &mut Program, ty: TyId, name: Symbol) -> bool {
    lookup_record_field(program, ty, name).is_some()
}

/// Finds `(field_index, concrete_field_ty)` on a struct/class type.
pub fn lookup_record_field(program: &mut Program, ty: TyId, name: Symbol) -> Option<(u32, TyId)> {
    let (def, args) = match program.types.kind(ty) {
        TyKind::Adt { def, args } => (*def, args.clone()),
        _ => return None,
    };
    let record = program.defs[def].record()?;
    let (index, field) = record.fields.iter().enumerate().find(|(_, f)| f.name == name)?;
    let field_ty = program.types.subst_generic(field.ty, &args);
    Some((index as u32, field_ty))
}

/// Evaluates an AST expression at compile time without a function context (top-level `const` / generic args).
pub(crate) fn eval_ast_const(cx: &mut Cx, module: ModuleId, expr: Id<ast::Expr>) -> Result<ConstVal, String> {
    eval_ast_const_with_env(cx, None, None, module, expr, &HashMap::new())
}

/// Evaluates an AST expression at compile time within an active function checking context.
pub(crate) fn eval_ast_const_in_fn(cx: &mut Cx, f: &mut FnCx, expr: Id<ast::Expr>) -> Result<ConstVal, String> {
    let module = f.module;
    eval_ast_const_with_env(cx, Some(f), None, module, expr, &HashMap::new())
}

fn eval_ast_const_with_env(
    cx: &mut Cx,
    mut f: Option<&mut FnCx>,
    generic_args: Option<&[GenericArg]>,
    module: ModuleId,
    expr: Id<ast::Expr>,
    env: &HashMap<Symbol, (ConstVal, TyId)>,
) -> Result<ConstVal, String> {
    let node = cx.ast(module)[expr].clone();
    match &node.kind {
        ast::ExprKind::Paren(inner) => eval_ast_const_with_env(cx, f, generic_args, module, *inner, env),
        ast::ExprKind::Int(lit) => i128::try_from(lit.value)
            .map(|n| ConstVal::Int(n, IntTy::I64))
            .map_err(|_| "integer constant out of range".to_string()),
        ast::ExprKind::Bool(b) => Ok(ConstVal::Bool(*b)),
        ast::ExprKind::String(s) => Ok(ConstVal::String(*s)),
        ast::ExprKind::Null => Ok(ConstVal::Null),
        ast::ExprKind::Tuple(els) => {
            let mut out = Vec::with_capacity(els.len());
            for &e in els {
                out.push(eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, e, env)?);
            }
            Ok(ConstVal::Tuple(out))
        }
        ast::ExprKind::List(els) => {
            let mut out = Vec::with_capacity(els.len());
            for &e in els {
                out.push(eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, e, env)?);
            }
            Ok(ConstVal::Array(out))
        }
        ast::ExprKind::Ident(sym) => {
            if let Some((val, _)) = env.get(sym) {
                return Ok(val.clone());
            }
            if let Some(fcx) = f.as_deref()
                && let Some((val, _)) = fcx.lookup_const(*sym)
            {
                return Ok(val);
            }
            if let Some(args) = generic_args
                && let Some(idx) = f
                    .as_deref()
                    .and_then(|fcx| fcx.scope.params.iter().position(|p| p == sym))
                && let Some(arg) = args.get(idx)
            {
                return match arg {
                    GenericArg::Type(t) => Ok(ConstVal::Type(*t)),
                    GenericArg::Const(ConstArg::Val(v)) => Ok(v.clone()),
                    GenericArg::Const(ConstArg::Expr(ce)) => {
                        eval_ast_const_with_env(cx, f, Some(&ce.args), ce.module, ce.expr, env)
                    }
                    GenericArg::Const(_) => Err("unresolved const generic parameter".to_string()),
                };
            }
            if let Some(fcx) = f.as_deref()
                && let Some(pos) = fcx.scope.params.iter().position(|p| p == sym)
                && fcx.scope.is_const_param(pos)
            {
                return Err(format!("const parameter `{}` has no concrete value yet", cx.text(*sym)));
            }
            match cx.lookup(module, *sym) {
                Some(Resolved::Def(def)) if matches!(cx.program.defs[def].kind, DefKind::Const(_)) => Ok(cx.ensure_const_def(def)),
                _ => Err(format!("`{}` is not a compile-time constant", cx.text(*sym))),
            }
        }
        ast::ExprKind::Member { object, name, args } if args.is_empty() => {
            // Enum variant literal (e.g. `Kind.Struct`) or field access on a ConstVal::Struct
            if let ast::ExprKind::Ident(base) = cx.ast(module)[*object].kind
                && !env.contains_key(&base)
                && f.as_deref().and_then(|fcx| fcx.lookup_const(base)).is_none()
                && let Some(Resolved::Def(def)) = cx.lookup(module, base)
                && let Some(en) = cx.program.defs[def].enum_def()
                && en.generics.is_empty()
                && let Some((v_idx, v)) = en.variants.iter().enumerate().find(|(_, v)| v.name == name.name)
                && matches!(v.payload, Payload::Unit)
            {
                let ty = cx.program.types.adt(def, Vec::new());
                return Ok(ConstVal::Enum { ty, variant: v_idx as u32 });
            }
            let obj = eval_ast_const_with_env(cx, f, generic_args, module, *object, env)?;
            if let ConstVal::Struct { fields, .. } = obj {
                if let Some((_, val)) = fields.iter().find(|(n, _)| *n == name.name) {
                    return Ok(val.clone());
                }
                return Err(format!("no field `{}` in compile-time struct", cx.text(name.name)));
            }
            Err("cannot access field on this compile-time value".to_string())
        }
        ast::ExprKind::TupleIndex { object, index } => {
            let obj = eval_ast_const_with_env(cx, f, generic_args, module, *object, env)?;
            match obj {
                ConstVal::Tuple(els) => els
                    .get(index.value as usize)
                    .cloned()
                    .ok_or_else(|| "tuple index out of bounds".to_string()),
                _ => Err("expected tuple value".to_string()),
            }
        }
        ast::ExprKind::Index { object, index } => {
            let obj = eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, *object, env)?;
            let idx = eval_ast_const_with_env(cx, f, generic_args, module, *index, env)?;
            let (ConstVal::Array(els), ConstVal::Int(i, _)) = (obj, idx) else {
                return Err("expected array and integer index".to_string());
            };
            let Ok(ui) = usize::try_from(i) else {
                return Err("negative index in compile-time indexing".to_string());
            };
            els.get(ui).cloned().ok_or_else(|| "compile-time index out of bounds".to_string())
        }
        ast::ExprKind::Unary { op, operand } => {
            let v = eval_ast_const_with_env(cx, f, generic_args, module, *operand, env)?;
            eval_unary(*op, v)
        }
        ast::ExprKind::Binary { op, left, right } => {
            // Short-circuit `&&` / `||`
            match op {
                ast::BinaryOp::And => {
                    let l = eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, *left, env)?;
                    if let ConstVal::Bool(false) = l {
                        return Ok(ConstVal::Bool(false));
                    }
                    let r = eval_ast_const_with_env(cx, f, generic_args, module, *right, env)?;
                    eval_binary(*op, l, r, cx.interner)
                }
                ast::BinaryOp::Or => {
                    let l = eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, *left, env)?;
                    if let ConstVal::Bool(true) = l {
                        return Ok(ConstVal::Bool(true));
                    }
                    let r = eval_ast_const_with_env(cx, f, generic_args, module, *right, env)?;
                    eval_binary(*op, l, r, cx.interner)
                }
                _ => {
                    let l = eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, *left, env)?;
                    let r = eval_ast_const_with_env(cx, f, generic_args, module, *right, env)?;
                    eval_binary(*op, l, r, cx.interner)
                }
            }
        }
        ast::ExprKind::ComptimeIf { condition, then, otherwise } | ast::ExprKind::If { condition, then, otherwise } => {
            let ast::Condition::Expr(cond_e) = condition else {
                return Err("pattern condition is not supported in constant expression".to_string());
            };
            let cond = eval_ast_const_with_env(cx, f.as_deref_mut(), generic_args, module, *cond_e, env)?;
            let ConstVal::Bool(take) = cond else {
                return Err("compile-time condition must be a Bool".to_string());
            };
            if take {
                eval_ast_const_with_env(cx, f, generic_args, module, *then, env)
            } else {
                match otherwise {
                    ast::ElseExpr::If(e) | ast::ElseExpr::Value(e) => eval_ast_const_with_env(cx, f, generic_args, module, *e, env),
                }
            }
        }
        ast::ExprKind::Comptime(block) => {
            if let Some(fcx) = f {
                match eval_comptime_block_with_env(cx, fcx, block, env)? {
                    Some((v, _)) => Ok(v),
                    None => Ok(ConstVal::Null),
                }
            } else {
                let scope = crate::resolve::Scope::empty(module);
                let mut temp = FnCx::new(module, scope, TyId::UNIT);
                match eval_comptime_block_with_env(cx, &mut temp, block, env)? {
                    Some((v, _)) => Ok(v),
                    None => Ok(ConstVal::Null),
                }
            }
        }
        ast::ExprKind::Builtin { name, generic_args: ga, args } => {
            let raw = cx.text(name.name);
            eval_ast_builtin(cx, f, generic_args, module, &raw, ga, args, node.span, env)
        }
        _ => Err("expression is not a compile-time constant".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn eval_ast_builtin(
    cx: &mut Cx,
    mut f: Option<&mut FnCx>,
    _outer_generic_args: Option<&[GenericArg]>,
    module: ModuleId,
    raw: &str,
    written_generics: &[ast::GenericArg],
    args: &[ast::Arg],
    span: Span,
    env: &HashMap<Symbol, (ConstVal, TyId)>,
) -> Result<ConstVal, String> {
    match raw {
        "compile_error" => {
            if args.len() != 1 {
                return Err("`@compile_error` takes 1 String argument".to_string());
            }
            let msg = eval_ast_const_with_env(cx, f, None, module, args[0].value, env)?;
            let ConstVal::String(sym) = msg else {
                return Err("`@compile_error` expects a String message".to_string());
            };
            let text = cx.text(sym);
            cx.error(span, text);
            Ok(ConstVal::Null)
        }
        "compile_assert" => {
            if args.len() != 2 {
                return Err("`@compile_assert` takes `(Bool, String)`".to_string());
            }
            let cond = eval_ast_const_with_env(cx, f.as_deref_mut(), None, module, args[0].value, env)?;
            let ConstVal::Bool(ok) = cond else {
                return Err("`@compile_assert` condition must be Bool".to_string());
            };
            if !ok {
                let msg = match eval_ast_const_with_env(cx, f, None, module, args[1].value, env) {
                    Ok(ConstVal::String(s)) => cx.text(s),
                    _ => "compile-time assertion failed".to_string(),
                };
                cx.error(span, msg);
            }
            Ok(ConstVal::Null)
        }
        "typeof" => {
            if !written_generics.is_empty() || args.len() != 1 || args[0].name.is_some() {
                return Err("`@typeof` takes 1 expression argument".to_string());
            }
            let Some(fcx) = f.as_deref_mut() else {
                return Err("`@typeof` needs a value in scope".to_string());
            };
            let inner = cx.expr(fcx, args[0].value, None);
            let ty = cx.solved(fcx, fcx.body.exprs[inner].ty);
            if ty == TyId::ERROR || cx.program.types.has_param(ty) {
                return Err("`@typeof` requires a concrete type".to_string());
            }
            Ok(ConstVal::Type(ty))
        }
        "size_of" | "align_of" | "is_flat" | "type_name" | "has_field" | "type_info" | "fields" | "variants" => {
            // Determine target type from written `<T>` or first value arg `T` or env/const
            let (ty, extra_start) = if let [ast::GenericArg::Type(t)] = written_generics {
                let scope = f.as_deref().map(|fcx| fcx.scope.clone()).unwrap_or_else(|| crate::resolve::Scope::empty(module));
                (cx.lower_type(&scope, *t), 0)
            } else if !args.is_empty() {
                if let ast::ExprKind::Ident(sym) = cx.ast(module)[args[0].value].kind {
                    let env_hit = env.get(&sym).cloned();
                    let const_hit = env_hit.or_else(|| f.as_deref().and_then(|fcx| fcx.lookup_const(sym)));
                    if let Some((ConstVal::Type(t), _)) = const_hit {
                        (t, 1)
                    } else if let Some(fcx) = f.as_deref() {
                        if cx.text(sym) == "Self"
                            && let Some(st) = fcx.scope.self_ty
                        {
                            (st, 1)
                        } else {
                            let scope = &fcx.scope;
                            let ident = ast::Ident { name: sym, span: cx.ast(module)[args[0].value].span };
                            let path = ast::Path { segments: vec![ident], span: ident.span };
                            (cx.lower_path(scope, &path, None, ident.span), 1)
                        }
                    } else {
                        (eval_ast_type_only(cx, None, module, args[0].value, env)?, 1)
                    }
                } else if let Some(fcx) = f.as_deref_mut() {
                    let scope = fcx.scope.clone();
                    let ast_e = cx.ast(module)[args[0].value].clone();
                    match &ast_e.kind {
                        ast::ExprKind::TypeApply { path, args: ga } => {
                            let ga = ga.iter().map(|g| cx.lower_generic_arg(&scope, g)).collect();
                            (cx.lower_path(&scope, path, Some(ga), ast_e.span), 1)
                        }
                        _ => (eval_ast_type_only(cx, Some(fcx), module, args[0].value, env)?, 1),
                    }
                } else {
                    (eval_ast_type_only(cx, None, module, args[0].value, env)?, 1)
                }
            } else {
                return Err(format!("`@{raw}` requires a type argument"));
            };
            if cx.program.types.has_param(ty) {
                return Err(format!("`@{raw}` requires a concrete type"));
            }
            match raw {
                "size_of" => Ok(ConstVal::Int(i128::from(layout_size(cx.program, ty)), IntTy::I64)),
                "align_of" => Ok(ConstVal::Int(i128::from(layout_align(cx.program, ty)), IntTy::I64)),
                "is_flat" => Ok(ConstVal::Bool(is_flat(cx.program, ty))),
                "type_name" => {
                    let rendered = cx.program.render_source(ty, cx.interner);
                    Ok(ConstVal::String(cx.interner.intern(&rendered)))
                }
                "has_field" => {
                    let Some(field_arg) = args.get(extra_start) else {
                        return Err("`@has_field` requires a field name String".to_string());
                    };
                    let val = eval_ast_const_with_env(cx, f, None, module, field_arg.value, env)?;
                    let ConstVal::String(name) = val else {
                        return Err("`@has_field` expects a String field name".to_string());
                    };
                    Ok(ConstVal::Bool(type_has_field(cx.program, ty, name)))
                }
                "type_info" => Ok(type_info_val(cx.program, cx.interner, ty)),
                "fields" => {
                    let items: Vec<ConstVal> = fields_vals(cx.program, cx.interner, ty).into_iter().map(|(v, _)| v).collect();
                    Ok(ConstVal::Array(items))
                }
                "variants" => {
                    let items: Vec<ConstVal> = variants_vals(cx.program, cx.interner, ty).into_iter().map(|(v, _)| v).collect();
                    Ok(ConstVal::Array(items))
                }
                _ => unreachable!(),
            }
        }
        _ => Err(format!("`@{raw}` cannot be evaluated at compile time")),
    }
}

fn eval_ast_type_only(
    cx: &mut Cx,
    f: Option<&mut FnCx>,
    module: ModuleId,
    expr: Id<ast::Expr>,
    env: &HashMap<Symbol, (ConstVal, TyId)>,
) -> Result<TyId, String> {
    if let Ok(ConstVal::Type(ty)) = eval_ast_const_with_env(cx, None, None, module, expr, env) {
        return Ok(ty);
    }
    let is_type_ast = matches!(
        &cx.ast(module)[expr].kind,
        ast::ExprKind::Ident(_)
            | ast::ExprKind::Member { .. }
            | ast::ExprKind::TypeApply { .. }
            | ast::ExprKind::Tuple(_)
            | ast::ExprKind::List(_)
            | ast::ExprKind::ArrayRepeat { .. }
    ) || matches!(
        &cx.ast(module)[expr].kind,
        ast::ExprKind::Builtin { name, .. }
            if matches!(
                cx.text(name.name).as_str(),
                "typeof" | "TypeArg" | "FieldType" | "VariantFieldType"
            )
    );
    if is_type_ast {
        let mut temp;
        let fcx = match f {
            Some(existing) => existing,
            None => {
                temp = FnCx::new(module, crate::resolve::Scope::empty(module), TyId::UNIT);
                &mut temp
            }
        };
        let pushed = !env.is_empty();
        if pushed {
            fcx.consts.push(env.clone());
        }
        let ty = cx.expr_as_type_arg(fcx, expr);
        if pushed {
            fcx.consts.pop();
        }
        if ty != TyId::ERROR {
            return Ok(ty);
        }
    }
    Err("expected a type argument".to_string())
}

/// Evaluates an already type-checked HIR `Expr` at compile time.
pub(crate) fn eval_const_expr(cx: &mut Cx, f: &FnCx, expr: Id<Expr>) -> Result<ConstVal, String> {
    eval_const_tree_expr(cx.program, cx.interner, Some(f), &f.body, expr)
}

fn eval_const_tree_expr(
    program: &Program,
    interner: &Interner,
    f: Option<&FnCx>,
    body: &Body,
    expr: Id<Expr>,
) -> Result<ConstVal, String> {
    match &body.exprs[expr].kind {
        ExprKind::Int(n) => {
            let int_ty = match program.types.kind(body.exprs[expr].ty) {
                TyKind::Int(i) => *i,
                _ => IntTy::I64,
            };
            Ok(ConstVal::Int(*n, int_ty))
        }
        ExprKind::Bool(b) => Ok(ConstVal::Bool(*b)),
        ExprKind::String(s) => Ok(ConstVal::String(*s)),
        ExprKind::Null => Ok(ConstVal::Null),
        ExprKind::Const(def) => program
            .const_vals
            .get(def)
            .cloned()
            .ok_or_else(|| "unresolved const item".to_string()),
        ExprKind::ConstArg(ConstArg::Val(v)) => Ok(v.clone()),
        ExprKind::ConstArg(_) => Err("const parameter has no concrete value yet".to_string()),
        ExprKind::Local(local) => {
            if let Some(fcx) = f {
                let sym = body.locals[*local].name;
                if let Some((val, _)) = fcx.lookup_const(sym) {
                    return Ok(val);
                }
            }
            Err("local variable is not a compile-time constant".to_string())
        }
        ExprKind::Tuple(els) => {
            let mut out = Vec::with_capacity(els.len());
            for &e in els {
                out.push(eval_const_tree_expr(program, interner, f, body, e)?);
            }
            Ok(ConstVal::Tuple(out))
        }
        ExprKind::Array(els) | ExprKind::List(els) => {
            let mut out = Vec::with_capacity(els.len());
            for &e in els {
                out.push(eval_const_tree_expr(program, interner, f, body, e)?);
            }
            Ok(ConstVal::Array(out))
        }
        ExprKind::Unary { op, operand } => {
            let v = eval_const_tree_expr(program, interner, f, body, *operand)?;
            eval_tree_unary(*op, v)
        }
        ExprKind::Binary { op, left, right } => {
            let l = eval_const_tree_expr(program, interner, f, body, *left)?;
            let r = eval_const_tree_expr(program, interner, f, body, *right)?;
            eval_tree_binary(*op, l, r, interner)
        }
        ExprKind::Field { object, index } => {
            let obj = eval_const_tree_expr(program, interner, f, body, *object)?;
            match obj {
                ConstVal::Struct { fields, .. } => fields
                    .get(*index as usize)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| "field index out of bounds".to_string()),
                _ => Err("cannot read field at compile time".to_string()),
            }
        }
        ExprKind::TupleField { object, index } => {
            let obj = eval_const_tree_expr(program, interner, f, body, *object)?;
            match obj {
                ConstVal::Tuple(els) => els
                    .get(*index as usize)
                    .cloned()
                    .ok_or_else(|| "tuple index out of bounds".to_string()),
                _ => Err("expected tuple value".to_string()),
            }
        }
        ExprKind::Record { fields, base: None } => {
            let ty = body.exprs[expr].ty;
            let field_names: Vec<Symbol> = match program.types.kind(ty) {
                TyKind::Adt { def, .. } => program.defs[*def]
                    .record()
                    .map(|r| r.fields.iter().map(|f| f.name).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let mut out = Vec::with_capacity(fields.len());
            for fv in fields {
                let Some(name) = field_names.get(fv.index as usize).copied() else {
                    return Err("record field index out of bounds".to_string());
                };
                let val = eval_const_tree_expr(program, interner, f, body, fv.value)?;
                out.push((name, val));
            }
            Ok(ConstVal::Struct { ty, fields: out })
        }
        ExprKind::Convert { expr: inner, .. } => eval_const_tree_expr(program, interner, f, body, *inner),
        ExprKind::Variant { variant, fields } if fields.is_empty() => Ok(ConstVal::Enum {
            ty: body.exprs[expr].ty,
            variant: *variant,
        }),
        ExprKind::If { condition: Condition::Expr(c), then, otherwise }
        | ExprKind::ComptimeIf { condition: c, then, otherwise } => {
            let cond = eval_const_tree_expr(program, interner, f, body, *c)?;
            match cond {
                ConstVal::Bool(true) => eval_const_tree_expr(program, interner, f, body, *then),
                ConstVal::Bool(false) => eval_const_tree_expr(program, interner, f, body, *otherwise),
                _ => Err("if condition must be Bool".to_string()),
            }
        }
        _ => Err("expression is not a compile-time constant".to_string()),
    }
}

/// Evaluates a `comptime { ... }` block with an empty initial local environment.
pub(crate) fn eval_comptime_block(cx: &mut Cx, f: &mut FnCx, block: &ast::Block) -> Result<Option<(ConstVal, TyId)>, String> {
    let env = HashMap::new();
    eval_comptime_block_with_env(cx, f, block, &env)
}

enum Flow {
    Next,
    Break,
    Continue,
    Return(Option<(ConstVal, TyId)>),
}

fn eval_comptime_block_with_env(
    cx: &mut Cx,
    f: &mut FnCx,
    block: &ast::Block,
    outer_env: &HashMap<Symbol, (ConstVal, TyId)>,
) -> Result<Option<(ConstVal, TyId)>, String> {
    let mut env = outer_env.clone();
    let mut steps = 0usize;
    let stmts = block.stmts.clone();
    let module = f.module;
    let len = stmts.len();
    for (idx, stmt_id) in stmts.into_iter().enumerate() {
        let is_last = idx + 1 == len;
        let stmt = cx.ast(module)[stmt_id].clone();
        if is_last && let ast::StmtKind::Expr(e) = &stmt.kind {
            let val = eval_ast_const_with_env(cx, Some(f), None, module, *e, &env)?;
            let ty = infer_const_ty(&mut cx.program.types, &val);
            return Ok(Some((val, ty)));
        }
        match exec_comptime_stmt(cx, f, stmt_id, &mut env, &mut steps)? {
            Flow::Next => {}
            Flow::Return(val) => return Ok(val),
            Flow::Break | Flow::Continue => return Err("`break`/`continue` outside loop in `comptime` block".to_string()),
        }
    }
    Ok(None)
}

fn exec_comptime_stmt(
    cx: &mut Cx,
    f: &mut FnCx,
    stmt_id: Id<ast::Stmt>,
    env: &mut HashMap<Symbol, (ConstVal, TyId)>,
    steps: &mut usize,
) -> Result<Flow, String> {
    *steps += 1;
    if *steps > MAX_COMPTIME_STEPS {
        return Err("`comptime` evaluation exceeded step limit".to_string());
    }
    let module = f.module;
    let stmt = cx.ast(module)[stmt_id].clone();
    match &stmt.kind {
        ast::StmtKind::Binding(b) => {
            let val = eval_ast_const_with_env(cx, Some(f), None, module, b.init, env)?;
            let ty = infer_const_ty(&mut cx.program.types, &val);
            bind_ast_pattern(cx, module, b.pattern, val, ty, env)?;
            Ok(Flow::Next)
        }
        ast::StmtKind::Const(c) => {
            let val = eval_ast_const_with_env(cx, Some(f), None, module, c.value, env)?;
            let ty = infer_const_ty(&mut cx.program.types, &val);
            env.insert(c.name.name, (val, ty));
            Ok(Flow::Next)
        }
        ast::StmtKind::Assign { target, op, value } => {
            let rhs = eval_ast_const_with_env(cx, Some(f), None, module, *value, env)?;
            let ast::ExprKind::Ident(name) = cx.ast(module)[*target].kind else {
                return Err("only variable assignment is supported in `comptime` blocks".to_string());
            };
            let Some((slot, slot_ty)) = env.get(&name).cloned() else {
                return Err(format!("unknown compile-time variable `{}`", cx.text(name)));
            };
            let next = if let Some(bin) = assign_to_binary(*op) {
                eval_binary(bin, slot, rhs, cx.interner)?
            } else {
                rhs
            };
            env.insert(name, (next, slot_ty));
            Ok(Flow::Next)
        }
        ast::StmtKind::If(if_stmt) | ast::StmtKind::ComptimeIf(if_stmt) => {
            let ast::Condition::Expr(cond_e) = if_stmt.condition else {
                return Err("pattern conditions are not supported inside `comptime` blocks".to_string());
            };
            let cond = eval_ast_const_with_env(cx, Some(f), None, module, cond_e, env)?;
            let ConstVal::Bool(take) = cond else {
                return Err("`if` condition in `comptime` block must be Bool".to_string());
            };
            if take {
                exec_comptime_subblock(cx, f, &if_stmt.then, env, steps)
            } else if let Some(otherwise) = &if_stmt.otherwise {
                match otherwise {
                    ast::Else::If(s) => exec_comptime_stmt(cx, f, *s, env, steps),
                    ast::Else::Block(b) => exec_comptime_subblock(cx, f, b, env, steps),
                }
            } else {
                Ok(Flow::Next)
            }
        }
        ast::StmtKind::While { condition, body } => {
            let ast::Condition::Expr(cond_e) = condition else {
                return Err("pattern conditions are not supported inside `comptime` loops".to_string());
            };
            loop {
                *steps += 1;
                if *steps > MAX_COMPTIME_STEPS {
                    return Err("`comptime` loop exceeded step limit".to_string());
                }
                let ConstVal::Bool(keep) = eval_ast_const_with_env(cx, Some(f), None, module, *cond_e, env)? else {
                    return Err("`while` condition in `comptime` block must be Bool".to_string());
                };
                if !keep {
                    break;
                }
                match exec_comptime_subblock(cx, f, body, env, steps)? {
                    Flow::Next | Flow::Continue => {}
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                }
            }
            Ok(Flow::Next)
        }
        ast::StmtKind::For { pattern, iterable, body } | ast::StmtKind::ComptimeFor { pattern, iterable, body } => {
            let items = eval_comptime_iterable_with_env(cx, f, *iterable, env)?;
            for (val, ty) in items {
                *steps += 1;
                if *steps > MAX_COMPTIME_STEPS {
                    return Err("`comptime` loop exceeded step limit".to_string());
                }
                let mut iter_env = env.clone();
                bind_ast_pattern(cx, module, *pattern, val, ty, &mut iter_env)?;
                match exec_comptime_subblock(cx, f, body, &mut iter_env, steps)? {
                    Flow::Next | Flow::Continue => {}
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                }
            }
            Ok(Flow::Next)
        }
        ast::StmtKind::ComptimeBlock(b) => exec_comptime_subblock(cx, f, b, env, steps),
        ast::StmtKind::Return(opt) => {
            let val = match opt {
                Some(e) => {
                    let v = eval_ast_const_with_env(cx, Some(f), None, module, *e, env)?;
                    let ty = infer_const_ty(&mut cx.program.types, &v);
                    Some((v, ty))
                }
                None => None,
            };
            Ok(Flow::Return(val))
        }
        ast::StmtKind::Break(_) => Ok(Flow::Break),
        ast::StmtKind::Continue => Ok(Flow::Continue),
        ast::StmtKind::Expr(e) => {
            let _ = eval_ast_const_with_env(cx, Some(f), None, module, *e, env)?;
            Ok(Flow::Next)
        }
        _ => Err("statement is not supported inside a `comptime` block".to_string()),
    }
}

fn exec_comptime_subblock(
    cx: &mut Cx,
    f: &mut FnCx,
    block: &ast::Block,
    env: &mut HashMap<Symbol, (ConstVal, TyId)>,
    steps: &mut usize,
) -> Result<Flow, String> {
    for &s in &block.stmts {
        match exec_comptime_stmt(cx, f, s, env, steps)? {
            Flow::Next => {}
            other => return Ok(other),
        }
    }
    Ok(Flow::Next)
}

fn bind_ast_pattern(
    cx: &mut Cx,
    module: ModuleId,
    pattern: Id<ast::Pattern>,
    val: ConstVal,
    ty: TyId,
    env: &mut HashMap<Symbol, (ConstVal, TyId)>,
) -> Result<(), String> {
    let p = cx.ast(module)[pattern].clone();
    match &p.kind {
        ast::PatternKind::Wildcard => Ok(()),
        ast::PatternKind::Binding(ident) => {
            env.insert(ident.name, (val, ty));
            Ok(())
        }
        ast::PatternKind::Tuple(sub) => {
            let ConstVal::Tuple(items) = val else {
                return Err("cannot destructure non-tuple at compile time".to_string());
            };
            if sub.len() != items.len() {
                return Err("tuple pattern length mismatch".to_string());
            }
            for (sp, item) in sub.iter().zip(items) {
                let it_ty = infer_const_ty(&mut cx.program.types, &item);
                bind_ast_pattern(cx, module, *sp, item, it_ty, env)?;
            }
            Ok(())
        }
        _ => Err("pattern kind not supported in compile-time binding".to_string()),
    }
}

/// Evaluates an iterable expression for `comptime for x in iterable`.
pub(crate) fn eval_comptime_iterable(cx: &mut Cx, f: &mut FnCx, iterable: Id<ast::Expr>) -> Result<Vec<(ConstVal, TyId)>, String> {
    eval_comptime_iterable_with_env(cx, f, iterable, &HashMap::new())
}

fn eval_comptime_iterable_with_env(
    cx: &mut Cx,
    f: &mut FnCx,
    iterable: Id<ast::Expr>,
    env: &HashMap<Symbol, (ConstVal, TyId)>,
) -> Result<Vec<(ConstVal, TyId)>, String> {
    let module = f.module;
    let node = cx.ast(module)[iterable].clone();
    match &node.kind {
        ast::ExprKind::Paren(inner) => eval_comptime_iterable_with_env(cx, f, *inner, env),
        ast::ExprKind::Range { start, end, inclusive } => {
            let s = eval_ast_const_with_env(cx, Some(f), None, module, *start, env)?;
            let e = eval_ast_const_with_env(cx, Some(f), None, module, *end, env)?;
            let (ConstVal::Int(a, int_ty), ConstVal::Int(b, _)) = (s, e) else {
                return Err("`comptime for` range bounds must be integers".to_string());
            };
            let ty = cx.program.types.int(int_ty);
            let mut out = Vec::new();
            let mut cur = a;
            while if *inclusive { cur <= b } else { cur < b } {
                if out.len() >= MAX_COMPTIME_STEPS {
                    return Err("`comptime for` range exceeded maximum iteration limit".to_string());
                }
                out.push((ConstVal::Int(cur, int_ty), ty));
                cur += 1;
            }
            Ok(out)
        }
        ast::ExprKind::Builtin { name, generic_args, args } => {
            let raw = cx.text(name.name);
            if raw == "fields" || raw == "variants" || raw == "__variant_fields" {
                let (target_ty, extra_idx) = if let [ast::GenericArg::Type(t)] = generic_args.as_slice() {
                    (cx.lower_type(&f.scope, *t), 0)
                } else if !args.is_empty() {
                    let arg_ast = cx.ast(module)[args[0].value].clone();
                    let ty = match &arg_ast.kind {
                        ast::ExprKind::Ident(sym) => {
                            let hit = env.get(sym).cloned().or_else(|| f.lookup_const(*sym));
                            if let Some((ConstVal::Type(t), _)) = hit {
                                t
                            } else if cx.text(*sym) == "Self"
                                && let Some(st) = f.scope.self_ty
                            {
                                st
                            } else {
                                let ident = ast::Ident { name: *sym, span: arg_ast.span };
                                let path = ast::Path { segments: vec![ident], span: ident.span };
                                cx.lower_path(&f.scope, &path, None, ident.span)
                            }
                        }
                        ast::ExprKind::TypeApply { path, args: ga } => {
                            let ga = ga.iter().map(|g| cx.lower_generic_arg(&f.scope, g)).collect();
                            cx.lower_path(&f.scope, path, Some(ga), arg_ast.span)
                        }
                        _ => eval_ast_type_only(cx, Some(f), module, args[0].value, env)?,
                    };
                    (ty, 1)
                } else {
                    return Err(format!("`@{raw}` expects a type argument"));
                };
                if cx.program.types.has_param(target_ty) {
                    return Err(format!("`@{raw}` requires a concrete type"));
                }
                if raw == "__variant_fields" {
                    let Some(v_arg) = args.get(extra_idx) else {
                        return Err("`@__variant_fields` expects `(T, variant_name: String)`".to_string());
                    };
                    let val = eval_ast_const_with_env(cx, Some(f), None, module, v_arg.value, env)?;
                    let ConstVal::String(v_name) = val else {
                        return Err("`@__variant_fields` expects a String variant name".to_string());
                    };
                    return Ok(variant_fields_vals(cx.program, cx.interner, target_ty, v_name));
                } else if raw == "fields" {
                    return Ok(fields_vals(cx.program, cx.interner, target_ty));
                } else {
                    return Ok(variants_vals(cx.program, cx.interner, target_ty));
                }
            }
            let val = eval_ast_const_with_env(cx, Some(f), None, module, iterable, env)?;
            iterable_from_val(cx, val)
        }
        _ => {
            let val = eval_ast_const_with_env(cx, Some(f), None, module, iterable, env)?;
            iterable_from_val(cx, val)
        }
    }
}

fn iterable_from_val(cx: &mut Cx, val: ConstVal) -> Result<Vec<(ConstVal, TyId)>, String> {
    match val {
        ConstVal::Array(items) | ConstVal::Tuple(items) => Ok(items
            .into_iter()
            .map(|item| {
                let ty = infer_const_ty(&mut cx.program.types, &item);
                (item, ty)
            })
            .collect()),
        _ => Err("`comptime for` expects a constant array, tuple, range, `@fields(...)`, or `@variants(...)`".to_string()),
    }
}

pub(crate) fn infer_const_ty(types: &mut crate::types::Types, val: &ConstVal) -> TyId {
    match val {
        ConstVal::Int(_, int) => types.int(*int),
        ConstVal::Bool(_) => TyId::BOOL,
        ConstVal::String(_) => TyId::STRING,
        ConstVal::Null => TyId::UNIT,
        ConstVal::Type(_) => TyId::STRING,
        ConstVal::Struct { ty, .. } | ConstVal::Enum { ty, .. } => *ty,
        ConstVal::Tuple(items) => {
            let elems = items.iter().map(|v| infer_const_ty(types, v)).collect();
            types.tuple(elems)
        }
        ConstVal::Array(items) => match items.first() {
            Some(first) => {
                let elem = infer_const_ty(types, first);
                types.array(elem)
            }
            None => TyId::ERROR,
        },
    }
}

fn eval_unary(op: ast::UnaryOp, v: ConstVal) -> Result<ConstVal, String> {
    let t_op = match op {
        ast::UnaryOp::Neg => UnaryOp::Neg,
        ast::UnaryOp::Not => UnaryOp::Not,
        ast::UnaryOp::BitNot => UnaryOp::BitNot,
    };
    eval_tree_unary(t_op, v)
}

fn eval_tree_unary(op: UnaryOp, v: ConstVal) -> Result<ConstVal, String> {
    match (op, v) {
        (UnaryOp::Neg, ConstVal::Int(n, ty)) => n
            .checked_neg()
            .map(|r| ConstVal::Int(r, ty))
            .ok_or_else(|| "integer overflow in compile-time negation".to_string()),
        (UnaryOp::BitNot, ConstVal::Int(n, ty)) => Ok(ConstVal::Int(!n, ty)),
        (UnaryOp::Not, ConstVal::Bool(b)) => Ok(ConstVal::Bool(!b)),
        _ => Err("unsupported operand in compile-time unary operation".to_string()),
    }
}

fn eval_binary(op: ast::BinaryOp, l: ConstVal, r: ConstVal, interner: &Interner) -> Result<ConstVal, String> {
    let t_op = match op {
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
        ast::BinaryOp::Coalesce => return Err("`??` is not supported in constant evaluation".to_string()),
    };
    eval_tree_binary(t_op, l, r, interner)
}

fn assign_to_binary(op: ast::AssignOp) -> Option<ast::BinaryOp> {
    match op {
        ast::AssignOp::Assign => None,
        ast::AssignOp::Add => Some(ast::BinaryOp::Add),
        ast::AssignOp::Sub => Some(ast::BinaryOp::Sub),
        ast::AssignOp::Mul => Some(ast::BinaryOp::Mul),
        ast::AssignOp::Div => Some(ast::BinaryOp::Div),
        ast::AssignOp::Rem => Some(ast::BinaryOp::Rem),
        ast::AssignOp::BitAnd => Some(ast::BinaryOp::BitAnd),
        ast::AssignOp::BitOr => Some(ast::BinaryOp::BitOr),
        ast::AssignOp::BitXor => Some(ast::BinaryOp::BitXor),
        ast::AssignOp::Shl => Some(ast::BinaryOp::Shl),
        ast::AssignOp::Shr => Some(ast::BinaryOp::Shr),
        ast::AssignOp::Coalesce => Some(ast::BinaryOp::Coalesce),
    }
}

fn eval_tree_binary(op: BinaryOp, l: ConstVal, r: ConstVal, interner: &Interner) -> Result<ConstVal, String> {
    match (op, l, r) {
        (BinaryOp::Add, ConstVal::Int(a, t), ConstVal::Int(b, _)) => a.checked_add(b).map(|n| ConstVal::Int(n, t)).ok_or_else(|| "integer overflow".to_string()),
        (BinaryOp::Sub, ConstVal::Int(a, t), ConstVal::Int(b, _)) => a.checked_sub(b).map(|n| ConstVal::Int(n, t)).ok_or_else(|| "integer overflow".to_string()),
        (BinaryOp::Mul, ConstVal::Int(a, t), ConstVal::Int(b, _)) => a.checked_mul(b).map(|n| ConstVal::Int(n, t)).ok_or_else(|| "integer overflow".to_string()),
        (BinaryOp::Div, ConstVal::Int(a, t), ConstVal::Int(b, _)) => {
            if b == 0 {
                Err("division by zero in constant expression".to_string())
            } else {
                a.checked_div(b).map(|n| ConstVal::Int(n, t)).ok_or_else(|| "integer overflow".to_string())
            }
        }
        (BinaryOp::Rem, ConstVal::Int(a, t), ConstVal::Int(b, _)) => {
            if b == 0 {
                Err("remainder by zero in constant expression".to_string())
            } else {
                a.checked_rem(b).map(|n| ConstVal::Int(n, t)).ok_or_else(|| "integer overflow".to_string())
            }
        }
        (BinaryOp::BitAnd, ConstVal::Int(a, t), ConstVal::Int(b, _)) => Ok(ConstVal::Int(a & b, t)),
        (BinaryOp::BitOr, ConstVal::Int(a, t), ConstVal::Int(b, _)) => Ok(ConstVal::Int(a | b, t)),
        (BinaryOp::BitXor, ConstVal::Int(a, t), ConstVal::Int(b, _)) => Ok(ConstVal::Int(a ^ b, t)),
        (BinaryOp::Shl, ConstVal::Int(a, t), ConstVal::Int(b, _)) => {
            if !(0..128).contains(&b) {
                Err("shift count out of range".to_string())
            } else {
                Ok(ConstVal::Int(a.wrapping_shl(b as u32), t))
            }
        }
        (BinaryOp::Shr, ConstVal::Int(a, t), ConstVal::Int(b, _)) => {
            if !(0..128).contains(&b) {
                Err("shift count out of range".to_string())
            } else {
                Ok(ConstVal::Int(a.wrapping_shr(b as u32), t))
            }
        }
        (BinaryOp::Lt, ConstVal::Int(a, _), ConstVal::Int(b, _)) => Ok(ConstVal::Bool(a < b)),
        (BinaryOp::LtEq, ConstVal::Int(a, _), ConstVal::Int(b, _)) => Ok(ConstVal::Bool(a <= b)),
        (BinaryOp::Gt, ConstVal::Int(a, _), ConstVal::Int(b, _)) => Ok(ConstVal::Bool(a > b)),
        (BinaryOp::GtEq, ConstVal::Int(a, _), ConstVal::Int(b, _)) => Ok(ConstVal::Bool(a >= b)),
        (BinaryOp::Eq, ConstVal::Int(a, _), ConstVal::Int(b, _)) => Ok(ConstVal::Bool(a == b)),
        (BinaryOp::NotEq, ConstVal::Int(a, _), ConstVal::Int(b, _)) => Ok(ConstVal::Bool(a != b)),
        (BinaryOp::Eq, ConstVal::Bool(a), ConstVal::Bool(b)) => Ok(ConstVal::Bool(a == b)),
        (BinaryOp::NotEq, ConstVal::Bool(a), ConstVal::Bool(b)) => Ok(ConstVal::Bool(a != b)),
        (BinaryOp::And, ConstVal::Bool(a), ConstVal::Bool(b)) => Ok(ConstVal::Bool(a && b)),
        (BinaryOp::Or, ConstVal::Bool(a), ConstVal::Bool(b)) => Ok(ConstVal::Bool(a || b)),
        (BinaryOp::Eq, ConstVal::String(a), ConstVal::String(b)) => Ok(ConstVal::Bool(interner.resolve(a) == interner.resolve(b))),
        (BinaryOp::NotEq, ConstVal::String(a), ConstVal::String(b)) => Ok(ConstVal::Bool(interner.resolve(a) != interner.resolve(b))),
        (BinaryOp::Eq, ConstVal::Type(a), ConstVal::Type(b)) => Ok(ConstVal::Bool(a == b)),
        (BinaryOp::NotEq, ConstVal::Type(a), ConstVal::Type(b)) => Ok(ConstVal::Bool(a != b)),
        (BinaryOp::Eq, ConstVal::Enum { ty: ta, variant: va }, ConstVal::Enum { ty: tb, variant: vb }) => {
            Ok(ConstVal::Bool(ta == tb && va == vb))
        }
        (BinaryOp::NotEq, ConstVal::Enum { ty: ta, variant: va }, ConstVal::Enum { ty: tb, variant: vb }) => {
            Ok(ConstVal::Bool(ta != tb || va != vb))
        }
        _ => Err("unsupported operands in compile-time binary operation".to_string()),
    }
}

/// Builds a `std.reflect.TypeInfo` constant value for `ty`.
pub fn type_info_val(program: &mut Program, interner: &mut Interner, ty: TyId) -> ConstVal {
    let type_info_ty = program.std.type_info.map_or(TyId::ERROR, |def| program.types.adt(def, Vec::new()));
    let kind_def = program.std.reflect_kind;
    let kind_ty = kind_def.map_or(TyId::ERROR, |def| program.types.adt(def, Vec::new()));
    let kind_variant = kind_variant_name(program, ty);
    let kind_idx = kind_def
        .and_then(|d| program.defs[d].enum_def())
        .and_then(|en| en.variants.iter().position(|v| interner.resolve(v.name) == kind_variant))
        .unwrap_or(0) as u32;

    let name_text = program.render_source(ty, interner);
    let name_sym = interner.intern(&name_text);
    let size = i128::from(layout_size(program, ty));
    let align = i128::from(layout_align(program, ty));
    let (is_pub, field_count, variant_count, doc_text) = match program.types.kind(ty) {
        TyKind::Adt { def, .. } => {
            let d = &program.defs[*def];
            let doc = program.modules[d.module].doc_text(d.doc);
            match &d.kind {
                DefKind::Record(r) => (d.is_pub, r.fields.len() as i128, 0, doc),
                DefKind::Enum(e) => (d.is_pub, 0, e.variants.len() as i128, doc),
                _ => (d.is_pub, 0, 0, doc),
            }
        }
        TyKind::Tuple(els) => (true, els.len() as i128, 0, String::new()),
        TyKind::Union(ms) => (true, 0, ms.len() as i128, String::new()),
        _ => (true, 0, 0, String::new()),
    };
    let doc_sym = interner.intern(&doc_text);

    ConstVal::Struct {
        ty: type_info_ty,
        fields: vec![
            (interner.intern("name"), ConstVal::String(name_sym)),
            (interner.intern("kind"), ConstVal::Enum { ty: kind_ty, variant: kind_idx }),
            (interner.intern("is_pub"), ConstVal::Bool(is_pub)),
            (interner.intern("size"), ConstVal::Int(size, IntTy::I64)),
            (interner.intern("align"), ConstVal::Int(align, IntTy::I64)),
            (interner.intern("field_count"), ConstVal::Int(field_count, IntTy::I64)),
            (interner.intern("variant_count"), ConstVal::Int(variant_count, IntTy::I64)),
            (interner.intern("doc"), ConstVal::String(doc_sym)),
        ],
    }
}

fn kind_variant_name(program: &Program, ty: TyId) -> &'static str {
    match program.types.kind(ty) {
        TyKind::Unit | TyKind::Error | TyKind::Param { .. } | TyKind::Infer(_) => "Null",
        TyKind::Bool => "Bool",
        TyKind::Int(_) => "Int",
        TyKind::Float(_) => "Float",
        TyKind::String => "String",
        TyKind::Tuple(_) => "Tuple",
        TyKind::Array(_) => "Array",
        TyKind::FixedArray(_, _) => "FixedArray",
        TyKind::Optional(_) => "Optional",
        TyKind::Union(_) => "Union",
        TyKind::Any => "Any",
        TyKind::Function(_) => "Function",
        TyKind::Builtin { kind, .. } => match kind {
            BuiltinTy::List => "List",
            _ => "Handle",
        },
        TyKind::Adt { def, .. } => {
            if program.std.map == Some(*def) {
                "Map"
            } else if program.std.set == Some(*def) {
                "Set"
            } else {
                match &program.defs[*def].kind {
                    DefKind::Record(r) => {
                        if r.is_class {
                            "Class"
                        } else {
                            "Struct"
                        }
                    }
                    DefKind::Enum(_) => "Enum",
                    _ => "Struct",
                }
            }
        }
    }
}

struct FieldInfoEntry {
    name: Symbol,
    ty: TyId,
    index: usize,
    is_var: bool,
    is_pub: bool,
    has_default: bool,
    doc: Symbol,
}

fn build_field_info_vals(program: &mut Program, interner: &mut Interner, entries: Vec<FieldInfoEntry>) -> Vec<(ConstVal, TyId)> {
    let field_info_ty = program.std.field_info.map_or(TyId::ERROR, |def| program.types.adt(def, Vec::new()));
    let k_name = interner.intern("name");
    let k_type_name = interner.intern("type_name");
    let k_index = interner.intern("index");
    let k_is_var = interner.intern("is_var");
    let k_is_pub = interner.intern("is_pub");
    let k_has_default = interner.intern("has_default");
    let k_doc = interner.intern("doc");

    entries
        .into_iter()
        .map(|entry| {
            let fty_text = program.render_source(entry.ty, interner);
            let fty_sym = interner.intern(&fty_text);
            let val = ConstVal::Struct {
                ty: field_info_ty,
                fields: vec![
                    (k_name, ConstVal::String(entry.name)),
                    (k_type_name, ConstVal::String(fty_sym)),
                    (k_index, ConstVal::Int(entry.index as i128, IntTy::I64)),
                    (k_is_var, ConstVal::Bool(entry.is_var)),
                    (k_is_pub, ConstVal::Bool(entry.is_pub)),
                    (k_has_default, ConstVal::Bool(entry.has_default)),
                    (k_doc, ConstVal::String(entry.doc)),
                ],
            };
            (val, field_info_ty)
        })
        .collect()
}

/// Builds a list of `(ConstVal, FieldInfo_ty)` for every field of struct/class/tuple `ty`.
pub fn fields_vals(program: &mut Program, interner: &mut Interner, ty: TyId) -> Vec<(ConstVal, TyId)> {
    let empty_doc = interner.intern("");
    if let TyKind::Tuple(members) = program.types.kind(ty).clone() {
        let entries = members
            .into_iter()
            .enumerate()
            .map(|(index, mty)| FieldInfoEntry {
                name: interner.intern(&index.to_string()),
                ty: mty,
                index,
                is_var: false,
                is_pub: true,
                has_default: false,
                doc: empty_doc,
            })
            .collect();
        return build_field_info_vals(program, interner, entries);
    }

    let TyKind::Adt { def, args } = program.types.kind(ty).clone() else {
        return Vec::new();
    };
    let module = program.defs[def].module;
    let Some(record) = program.defs[def].record() else {
        return Vec::new();
    };
    let entries = record
        .fields
        .clone()
        .into_iter()
        .enumerate()
        .map(|(index, field)| {
            let doc_text = program.modules[module].doc_text(field.doc);
            let doc = interner.intern(&doc_text);
            FieldInfoEntry {
                name: field.name,
                ty: program.types.subst_generic(field.ty, &args),
                index,
                is_var: field.is_var,
                is_pub: field.is_pub,
                has_default: field.default.is_some(),
                doc,
            }
        })
        .collect();
    build_field_info_vals(program, interner, entries)
}

/// Builds a list of `(ConstVal, FieldInfo_ty)` for every payload field of enum variant `variant_name` on `ty`.
pub fn variant_fields_vals(program: &mut Program, interner: &mut Interner, ty: TyId, variant_name: Symbol) -> Vec<(ConstVal, TyId)> {
    let empty_doc = interner.intern("");
    let TyKind::Adt { def, args } = program.types.kind(ty).clone() else {
        return Vec::new();
    };
    let module = program.defs[def].module;
    let Some(en) = program.defs[def].enum_def() else {
        return Vec::new();
    };
    let Some(variant) = en.variants.iter().find(|v| v.name == variant_name).cloned() else {
        return Vec::new();
    };

    let entries = match variant.payload {
        Payload::Unit => Vec::new(),
        Payload::Tuple(types) => types
            .into_iter()
            .enumerate()
            .map(|(index, raw_ty)| FieldInfoEntry {
                name: interner.intern(&index.to_string()),
                ty: program.types.subst_generic(raw_ty, &args),
                index,
                is_var: false,
                is_pub: true,
                has_default: false,
                doc: empty_doc,
            })
            .collect(),
        Payload::Struct(fields) => fields
            .into_iter()
            .enumerate()
            .map(|(index, field)| {
                let doc_text = program.modules[module].doc_text(field.doc);
                let doc = interner.intern(&doc_text);
                FieldInfoEntry {
                    name: field.name,
                    ty: program.types.subst_generic(field.ty, &args),
                    index,
                    is_var: field.is_var,
                    is_pub: field.is_pub,
                    has_default: field.default.is_some(),
                    doc,
                }
            })
            .collect(),
    };
    build_field_info_vals(program, interner, entries)
}

/// Builds a list of `(ConstVal, VariantInfo_ty)` for every variant of enum `ty`.
pub fn variants_vals(program: &mut Program, interner: &mut Interner, ty: TyId) -> Vec<(ConstVal, TyId)> {
    let variant_info_ty = program.std.variant_info.map_or(TyId::ERROR, |def| program.types.adt(def, Vec::new()));
    let shape_def = program.std.reflect_shape;
    let shape_ty = shape_def.map_or(TyId::ERROR, |def| program.types.adt(def, Vec::new()));
    let TyKind::Adt { def, .. } = program.types.kind(ty).clone() else {
        return Vec::new();
    };
    let Some(en) = program.defs[def].enum_def() else {
        return Vec::new();
    };
    let variants = en.variants.clone();
    let k_name = interner.intern("name");
    let k_index = interner.intern("index");
    let k_shape = interner.intern("shape");
    let k_field_count = interner.intern("field_count");

    variants
        .into_iter()
        .enumerate()
        .map(|(index, v)| {
            let (shape_name, field_count) = match &v.payload {
                Payload::Unit => ("Unit", 0i128),
                Payload::Tuple(ts) => ("Tuple", ts.len() as i128),
                Payload::Struct(fs) => ("Named", fs.len() as i128),
            };
            let shape_idx = shape_def
                .and_then(|d| program.defs[d].enum_def())
                .and_then(|e| e.variants.iter().position(|sv| interner.resolve(sv.name) == shape_name))
                .unwrap_or(0) as u32;
            let val = ConstVal::Struct {
                ty: variant_info_ty,
                fields: vec![
                    (k_name, ConstVal::String(v.name)),
                    (k_index, ConstVal::Int(index as i128, IntTy::I64)),
                    (k_shape, ConstVal::Enum { ty: shape_ty, variant: shape_idx }),
                    (k_field_count, ConstVal::Int(field_count, IntTy::I64)),
                ],
            };
            (val, variant_info_ty)
        })
        .collect()
}

/// Returns the length of a fixed-size array type `[T; N]`, if statically known.
pub fn fixed_array_len(program: &Program, ty: TyId) -> Option<i128> {
    if let TyKind::FixedArray(_, ConstArg::Val(ConstVal::Int(n, _))) = program.types.kind(ty) {
        Some(*n)
    } else {
        None
    }
}

/// Builds a type-safe zero/empty default `tree::Expr` for `ty`.
pub fn default_value_expr(
    program: &mut Program,
    interner: &mut Interner,
    body: &mut Body,
    ty: TyId,
    span: Span,
    seen: &mut Vec<TyId>,
) -> Result<Id<Expr>, String> {
    match program.types.kind(ty).clone() {
        TyKind::Unit => Ok(body.expr(ExprKind::Null, TyId::UNIT, span)),
        TyKind::Bool => Ok(body.expr(ExprKind::Bool(false), TyId::BOOL, span)),
        TyKind::Int(_) => Ok(body.expr(ExprKind::Int(0), ty, span)),
        TyKind::Float(_) => Ok(body.expr(ExprKind::Float(0.0), ty, span)),
        TyKind::String => Ok(body.expr(ExprKind::String(interner.intern("")), TyId::STRING, span)),
        TyKind::Optional(_) => Ok(body.expr(ExprKind::Null, ty, span)),
        TyKind::Array(_) => Ok(body.expr(ExprKind::Array(Vec::new()), ty, span)),
        TyKind::Builtin { kind: BuiltinTy::List, .. } => Ok(body.expr(ExprKind::List(Vec::new()), ty, span)),
        TyKind::FixedArray(elem, ConstArg::Val(ConstVal::Int(len, _))) => {
            let elem_e = default_value_expr(program, interner, body, elem, span, seen)?;
            let len_e = body.expr(ExprKind::Int(len), TyId::INT, span);
            Ok(body.expr(ExprKind::ArrayRepeat { value: elem_e, len: len_e }, ty, span))
        }
        TyKind::Tuple(members) => {
            let mut elems = Vec::with_capacity(members.len());
            for m in members {
                elems.push(default_value_expr(program, interner, body, m, span, seen)?);
            }
            Ok(body.expr(ExprKind::Tuple(elems), ty, span))
        }
        TyKind::Union(members) => {
            let preferred = members.iter().copied().find(|m| *m == TyId::UNIT).or_else(|| members.first().copied());
            let Some(m0) = preferred else {
                return Err("cannot construct empty union".to_string());
            };
            let inner = default_value_expr(program, interner, body, m0, span, seen)?;
            Ok(body.expr(ExprKind::Convert { conversion: Conversion::ToUnion, expr: inner }, ty, span))
        }
        TyKind::Any => {
            let inner = body.expr(ExprKind::Null, TyId::UNIT, span);
            Ok(body.expr(ExprKind::Convert { conversion: Conversion::ToUnion, expr: inner }, ty, span))
        }
        TyKind::Adt { def, args } => {
            if seen.contains(&ty) {
                return Err(format!("cannot default-construct cyclic type `{}`", program.render_source(ty, interner)));
            }
            if program.std.map == Some(def) {
                return Ok(body.expr(ExprKind::Map(Vec::new()), ty, span));
            }
            seen.push(ty);
            let result = if let Some(record) = program.defs[def].record() {
                let field_tys: Vec<TyId> = record.fields.iter().map(|fl| program.types.subst_generic(fl.ty, &args)).collect();
                let mut fields = Vec::with_capacity(field_tys.len());
                for (index, fty) in field_tys.into_iter().enumerate() {
                    let val = default_value_expr(program, interner, body, fty, span, seen)?;
                    fields.push(FieldValue { index: index as u32, value: val });
                }
                Ok(body.expr(ExprKind::Record { fields, base: None }, ty, span))
            } else if let Some(en) = program.defs[def].enum_def() {
                if en.variants.is_empty() {
                    Err("cannot default-construct empty enum".to_string())
                } else {
                    default_variant_expr(program, interner, body, ty, def, &args, 0, span, seen)
                }
            } else {
                Err(format!("cannot default-construct `{}`", program.render_source(ty, interner)))
            };
            seen.pop();
            result
        }
        _ => Err(format!("cannot default-construct `{}`", program.render_source(ty, interner))),
    }
}

/// Builds a default-initialized `ExprKind::Variant` for variant `variant_idx` of `enum_ty`.
#[allow(clippy::too_many_arguments)]
pub fn default_variant_expr(
    program: &mut Program,
    interner: &mut Interner,
    body: &mut Body,
    enum_ty: TyId,
    def: crate::defs::DefId,
    args: &[GenericArg],
    variant_idx: usize,
    span: Span,
    seen: &mut Vec<TyId>,
) -> Result<Id<Expr>, String> {
    let payload = program.defs[def]
        .enum_def()
        .and_then(|en| en.variants.get(variant_idx))
        .map(|v| v.payload.clone())
        .ok_or_else(|| "missing variant".to_string())?;
    let payload_tys: Vec<TyId> = match payload {
        Payload::Unit => Vec::new(),
        Payload::Tuple(ts) => ts.into_iter().map(|t| program.types.subst_generic(t, args)).collect(),
        Payload::Struct(fs) => fs.into_iter().map(|fl| program.types.subst_generic(fl.ty, args)).collect(),
    };
    let mut fields = Vec::with_capacity(payload_tys.len());
    for (index, fty) in payload_tys.into_iter().enumerate() {
        let val = default_value_expr(program, interner, body, fty, span, seen)?;
        fields.push(FieldValue { index: index as u32, value: val });
    }
    Ok(body.expr(ExprKind::Variant { variant: variant_idx as u32, fields }, enum_ty, span))
}

/// Converts a constant value into a typed `tree::Expr` in `body`.
pub fn const_val_to_expr(program: &mut Program, body: &mut Body, val: &ConstVal, ty: TyId, span: Span) -> Id<Expr> {
    match val {
        ConstVal::Int(n, _) => body.expr(ExprKind::Int(*n), ty, span),
        ConstVal::Bool(b) => body.expr(ExprKind::Bool(*b), TyId::BOOL, span),
        ConstVal::String(s) => body.expr(ExprKind::String(*s), TyId::STRING, span),
        ConstVal::Null => body.expr(ExprKind::Null, ty, span),
        ConstVal::Type(t) => body.expr(ExprKind::ConstArg(ConstArg::Val(ConstVal::Type(*t))), TyId::STRING, span),
        ConstVal::Tuple(items) => {
            let elem_tys: Vec<TyId> = match program.types.kind(ty) {
                TyKind::Tuple(m) if m.len() == items.len() => m.clone(),
                _ => items.iter().map(|v| infer_const_ty(&mut program.types, v)).collect(),
            };
            let exprs = items
                .iter()
                .zip(elem_tys)
                .map(|(v, it_ty)| const_val_to_expr(program, body, v, it_ty, span))
                .collect();
            body.expr(ExprKind::Tuple(exprs), ty, span)
        }
        ConstVal::Array(items) => {
            let elem_ty = match program.types.kind(ty) {
                TyKind::Array(e) | TyKind::FixedArray(e, _) => *e,
                _ => items.first().map_or(TyId::ERROR, |v| infer_const_ty(&mut program.types, v)),
            };
            let exprs = items.iter().map(|v| const_val_to_expr(program, body, v, elem_ty, span)).collect();
            body.expr(ExprKind::Array(exprs), ty, span)
        }
        ConstVal::Struct { ty: s_ty, fields } => {
            let target_ty = if *s_ty != TyId::ERROR { *s_ty } else { ty };
            let mut field_values = Vec::new();
            type RecordFields = (Vec<(crate::defs::Field, usize)>, Vec<GenericArg>);
            let resolved: Option<RecordFields> = match program.types.kind(target_ty) {
                TyKind::Adt { def, args } => {
                    let args = args.clone();
                    program.defs[*def]
                        .record()
                        .map(|record| (record.fields.iter().cloned().enumerate().map(|(i, f)| (f, i)).collect(), args))
                }
                _ => None,
            };
            if let Some((rec_fields, args)) = resolved {
                for (f, index) in rec_fields {
                    if let Some((_, val)) = fields.iter().find(|(n, _)| *n == f.name) {
                        let f_ty = program.types.subst_generic(f.ty, &args);
                        let value = const_val_to_expr(program, body, val, f_ty, span);
                        field_values.push(FieldValue { index: index as u32, value });
                    }
                }
            }
            body.expr(ExprKind::Record { fields: field_values, base: None }, target_ty, span)
        }
        ConstVal::Enum { ty: e_ty, variant } => {
            let target_ty = if *e_ty != TyId::ERROR { *e_ty } else { ty };
            body.expr(
                ExprKind::Variant {
                    variant: *variant,
                    fields: Vec::new(),
                },
                target_ty,
                span,
            )
        }
    }
}

/// Resolves any remaining concrete `ConstArg::Expr` inside a `ConstArg` using `Cx::asts`.
pub(crate) fn normalize_const_arg(cx: &mut Cx, arg: &ConstArg) -> ConstArg {
    match arg {
        ConstArg::Expr(ce) if !ce.args.iter().any(|g| cx.program.types.generic_has_param(g) || cx.program.types.generic_has_infer(g)) => {
            let mut env = HashMap::new();
            let mut resolved_args = Vec::with_capacity(ce.args.len());
            for (idx, g) in ce.args.iter().enumerate() {
                match g {
                    GenericArg::Type(t) => {
                        let t = normalize_ty_consts(cx, *t);
                        if let Some(&sym) = ce.params.get(idx) {
                            env.insert(sym, (ConstVal::Type(t), TyId::UNIT));
                        }
                        resolved_args.push(GenericArg::Type(t));
                    }
                    GenericArg::Const(c) => {
                        let c = normalize_const_arg(cx, c);
                        if let (Some(&sym), ConstArg::Val(v)) = (ce.params.get(idx), &c) {
                            let v_ty = infer_const_ty(&mut cx.program.types, v);
                            env.insert(sym, (v.clone(), v_ty));
                        }
                        resolved_args.push(GenericArg::Const(c));
                    }
                }
            }
            match eval_ast_const_with_env(cx, None, Some(&resolved_args), ce.module, ce.expr, &env) {
                Ok(val) => ConstArg::Val(val),
                Err(_) => arg.clone(),
            }
        }
        _ => arg.clone(),
    }
}

/// Replaces any concrete `ConstArg::Expr` inside `ty` with evaluated `ConstArg::Val` constants.
pub(crate) fn normalize_ty_consts(cx: &mut Cx, ty: TyId) -> TyId {
    normalize_ty_consts_memo(cx, ty, &mut HashMap::new())
}

fn normalize_ty_consts_memo(cx: &mut Cx, ty: TyId, done: &mut HashMap<TyId, TyId>) -> TyId {
    if let Some(&cached) = done.get(&ty) {
        return cached;
    }
    let out = match cx.program.types.kind(ty).clone() {
        TyKind::Builtin { kind, args } => {
            let new_args: Vec<TyId> = args.iter().map(|&t| normalize_ty_consts_memo(cx, t, done)).collect();
            if new_args == args {
                ty
            } else {
                cx.program.types.builtin(kind, new_args)
            }
        }
        TyKind::Adt { def, args } => {
            let new_args: Vec<GenericArg> = args
                .iter()
                .map(|g| match g {
                    GenericArg::Type(t) => GenericArg::Type(normalize_ty_consts_memo(cx, *t, done)),
                    GenericArg::Const(c) => GenericArg::Const(normalize_const_arg(cx, c)),
                })
                .collect();
            if new_args == args {
                ty
            } else {
                cx.program.types.adt(def, new_args)
            }
        }
        TyKind::Tuple(els) => {
            let new_els: Vec<TyId> = els.iter().map(|&t| normalize_ty_consts_memo(cx, t, done)).collect();
            if new_els == els {
                ty
            } else {
                cx.program.types.tuple(new_els)
            }
        }
        TyKind::Array(el) => {
            let new_el = normalize_ty_consts_memo(cx, el, done);
            if new_el == el { ty } else { cx.program.types.array(new_el) }
        }
        TyKind::FixedArray(el, len) => {
            let new_el = normalize_ty_consts_memo(cx, el, done);
            let new_len = normalize_const_arg(cx, &len);
            if new_el == el && new_len == len {
                ty
            } else {
                cx.program.types.fixed_array_arg(new_el, new_len)
            }
        }
        TyKind::Optional(inner) => {
            let new_inner = normalize_ty_consts_memo(cx, inner, done);
            if new_inner == inner { ty } else { cx.program.types.optional(new_inner) }
        }
        TyKind::Union(members) => {
            let new_members: Vec<TyId> = members.iter().map(|&t| normalize_ty_consts_memo(cx, t, done)).collect();
            if new_members == members {
                ty
            } else {
                cx.program.types.union(&new_members)
            }
        }
        TyKind::Function(f) => {
            let params: Vec<crate::types::FnParam> = f
                .params
                .iter()
                .map(|p| crate::types::FnParam {
                    ty: normalize_ty_consts_memo(cx, p.ty, done),
                    is_var: p.is_var,
                })
                .collect();
            let ret = normalize_ty_consts_memo(cx, f.ret, done);
            if params == f.params && ret == f.ret {
                ty
            } else {
                cx.program.types.function(params, ret, f.send)
            }
        }
        _ => ty,
    };
    done.insert(ty, out);
    out
}

/// Evaluates all concrete `ConstArg::Expr` values across an instantiated `Body`.
pub(crate) fn normalize_body_consts(cx: &mut Cx, body: &mut Body) {
    let mut done = HashMap::new();
    let locals: Vec<_> = body.locals.iter().map(|(id, _)| id).collect();
    for id in locals {
        body.locals[id].ty = normalize_ty_consts_memo(cx, body.locals[id].ty, &mut done);
    }
    let exprs: Vec<_> = body.exprs.iter().map(|(id, _)| id).collect();
    for id in exprs {
        body.exprs[id].ty = normalize_ty_consts_memo(cx, body.exprs[id].ty, &mut done);
        match &mut body.exprs[id].kind {
            ExprKind::Is { ty, .. } => *ty = normalize_ty_consts_memo(cx, *ty, &mut done),
            ExprKind::ConstArg(arg) => *arg = normalize_const_arg(cx, arg),
            ExprKind::Function(function) | ExprKind::Call { callee: Callee::Function(function), .. } => {
                function.args = function
                    .args
                    .iter()
                    .map(|g| match g {
                        GenericArg::Type(t) => GenericArg::Type(normalize_ty_consts_memo(cx, *t, &mut done)),
                        GenericArg::Const(c) => GenericArg::Const(normalize_const_arg(cx, c)),
                    })
                    .collect();
            }
            ExprKind::Call { callee: Callee::TraitMethod { self_ty, args, .. }, .. } => {
                *self_ty = normalize_ty_consts_memo(cx, *self_ty, &mut done);
                *args = args
                    .iter()
                    .map(|g| match g {
                        GenericArg::Type(t) => GenericArg::Type(normalize_ty_consts_memo(cx, *t, &mut done)),
                        GenericArg::Const(c) => GenericArg::Const(normalize_const_arg(cx, c)),
                    })
                    .collect();
            }
            ExprKind::Call { callee: Callee::Builtin { generic_args, .. }, .. } => {
                *generic_args = generic_args
                    .iter()
                    .map(|g| match g {
                        GenericArg::Type(t) => GenericArg::Type(normalize_ty_consts_memo(cx, *t, &mut done)),
                        GenericArg::Const(c) => GenericArg::Const(normalize_const_arg(cx, c)),
                    })
                    .collect();
            }
            _ => {}
        }
    }
    let patterns: Vec<_> = body.patterns.iter().map(|(id, _)| id).collect();
    for id in patterns {
        body.patterns[id].ty = normalize_ty_consts_memo(cx, body.patterns[id].ty, &mut done);
        if let PatternKind::Type { ty, .. } = &mut body.patterns[id].kind {
            *ty = normalize_ty_consts_memo(cx, *ty, &mut done);
        }
    }
}

/// Evaluates any deferred `comptime` nodes (`Const`, `ConstArg`, `@type_info`, `@fields`, `@variants`,
/// `@size_of`, `@align_of`, `@type_name`, `@has_field`, `@compile_error`, `@compile_assert`) in a concrete `Body`.
pub fn lower_body(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, body: &mut Body) {
    let stmts = body.block.stmts.clone();
    body.block.stmts = lower_stmts(program, interner, sink, body, &stmts);
}

fn lower_block(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, body: &mut Body, block: &Block) -> Block {
    Block {
        stmts: lower_stmts(program, interner, sink, body, &block.stmts),
        span: block.span,
    }
}

fn lower_stmts(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, body: &mut Body, stmts: &[Id<Stmt>]) -> Vec<Id<Stmt>> {
    let mut out = Vec::with_capacity(stmts.len());
    for &stmt_id in stmts {
        let span = body.stmts[stmt_id].span;
        let kind = body.stmts[stmt_id].kind.clone();
        match kind {
            StmtKind::Const { .. } | StmtKind::ComptimeBlock(_) => {
                // Already evaluated during `check`; drop from runtime IR.
                continue;
            }
            StmtKind::ComptimeIf(if_stmt) => {
                lower_expr(program, interner, sink, body, match if_stmt.condition {
                    Condition::Expr(e) => e,
                    Condition::Let { value, .. } => value,
                });
                if let Condition::Expr(cond_e) = if_stmt.condition
                    && let Ok(ConstVal::Bool(take)) = eval_const_tree_expr(program, interner, None, body, cond_e)
                {
                    if take {
                        out.extend(lower_stmts(program, interner, sink, body, &if_stmt.then.stmts));
                    } else if let Some(otherwise) = if_stmt.otherwise {
                        match otherwise {
                            Else::If(s) => out.extend(lower_stmts(program, interner, sink, body, &[s])),
                            Else::Block(b) => out.extend(lower_stmts(program, interner, sink, body, &b.stmts)),
                        }
                    }
                    continue;
                }
            }
            StmtKind::Let { pattern, init } => {
                lower_expr(program, interner, sink, body, init);
                body.stmts[stmt_id].kind = StmtKind::Let { pattern, init };
            }
            StmtKind::Assign { target, op, value } => {
                lower_expr(program, interner, sink, body, target);
                lower_expr(program, interner, sink, body, value);
                body.stmts[stmt_id].kind = StmtKind::Assign { target, op, value };
            }
            StmtKind::If(mut if_stmt) => {
                lower_condition(program, interner, sink, body, &mut if_stmt.condition);
                if_stmt.then = lower_block(program, interner, sink, body, &if_stmt.then);
                if_stmt.otherwise = if_stmt.otherwise.map(|e| match e {
                    Else::If(s) => {
                        let lowered = lower_stmts(program, interner, sink, body, &[s]);
                        Else::If(lowered.into_iter().next().unwrap_or(s))
                    }
                    Else::Block(b) => Else::Block(lower_block(program, interner, sink, body, &b)),
                });
                body.stmts[stmt_id].kind = StmtKind::If(if_stmt);
            }
            StmtKind::While { mut condition, body: loop_body } => {
                lower_condition(program, interner, sink, body, &mut condition);
                let loop_body = lower_block(program, interner, sink, body, &loop_body);
                body.stmts[stmt_id].kind = StmtKind::While { condition, body: loop_body };
            }
            StmtKind::For(mut fl) => {
                lower_expr(program, interner, sink, body, fl.iterable);
                fl.body = lower_block(program, interner, sink, body, &fl.body);
                body.stmts[stmt_id].kind = StmtKind::For(fl);
            }
            StmtKind::Match(mut m) => {
                lower_match(program, interner, sink, body, &mut m);
                body.stmts[stmt_id].kind = StmtKind::Match(m);
            }
            StmtKind::Scope(b) => {
                let b = lower_block(program, interner, sink, body, &b);
                body.stmts[stmt_id].kind = StmtKind::Scope(b);
            }
            StmtKind::With { local, init, body: wbody, close } => {
                lower_expr(program, interner, sink, body, init);
                let wbody = lower_block(program, interner, sink, body, &wbody);
                body.stmts[stmt_id].kind = StmtKind::With { local, init, body: wbody, close };
            }
            StmtKind::Return(opt) => {
                if let Some(e) = opt {
                    lower_expr(program, interner, sink, body, e);
                }
            }
            StmtKind::Yield(e) | StmtKind::Expr(e) => lower_expr(program, interner, sink, body, e),
            StmtKind::ComptimeFor { .. } | StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
        }
        let _ = span;
        out.push(stmt_id);
    }
    out
}

fn lower_condition(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, body: &mut Body, cond: &mut Condition) {
    match cond {
        Condition::Expr(e) | Condition::Let { value: e, .. } => lower_expr(program, interner, sink, body, *e),
    }
}

fn lower_match(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, body: &mut Body, m: &mut Match) {
    lower_expr(program, interner, sink, body, m.subject);
    for arm in &mut m.arms {
        if let Some(g) = arm.guard {
            lower_expr(program, interner, sink, body, g);
        }
        match &mut arm.body {
            ArmBody::Value(e) => lower_expr(program, interner, sink, body, *e),
            ArmBody::Block(b) => *b = lower_block(program, interner, sink, body, b),
        }
    }
}

fn lower_expr(program: &mut Program, interner: &mut Interner, sink: &mut DiagnosticSink, body: &mut Body, expr_id: Id<Expr>) {
    let span = body.exprs[expr_id].span;
    let ty = body.exprs[expr_id].ty;
    let kind = body.exprs[expr_id].kind.clone();
    match kind {
        ExprKind::Const(def) => {
            if let Some(val) = program.const_vals.get(&def).cloned() {
                let replacement = const_val_to_expr(program, body, &val, ty, span);
                body.exprs[expr_id] = body.exprs[replacement].clone();
            }
        }
        ExprKind::ConstArg(ConstArg::Val(val)) => {
            let replacement = const_val_to_expr(program, body, &val, ty, span);
            body.exprs[expr_id] = body.exprs[replacement].clone();
        }
        ExprKind::ComptimeIf { condition, then, otherwise } => {
            lower_expr(program, interner, sink, body, condition);
            if let Ok(ConstVal::Bool(take)) = eval_const_tree_expr(program, interner, None, body, condition) {
                let chosen = if take { then } else { otherwise };
                lower_expr(program, interner, sink, body, chosen);
                body.exprs[expr_id] = body.exprs[chosen].clone();
            }
        }
        ExprKind::Call { callee: Callee::Builtin { name, generic_args }, args } => {
            for &a in &args {
                lower_expr(program, interner, sink, body, a);
            }
            let raw = interner.resolve(name).to_string();
            match raw.as_str() {
                "size_of" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        body.exprs[expr_id] = Expr { kind: ExprKind::Int(i128::from(layout_size(program, *t))), ty: TyId::INT, span };
                    }
                }
                "align_of" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        body.exprs[expr_id] = Expr { kind: ExprKind::Int(i128::from(layout_align(program, *t))), ty: TyId::INT, span };
                    }
                }
                "is_flat" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        body.exprs[expr_id] = Expr { kind: ExprKind::Bool(is_flat(program, *t)), ty: TyId::BOOL, span };
                    }
                }
                "type_name" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        let rendered = program.render_source(*t, interner);
                        let s = interner.intern(&rendered);
                        body.exprs[expr_id] = Expr { kind: ExprKind::String(s), ty: TyId::STRING, span };
                    }
                }
                "array_len" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first()
                        && let Some(n) = fixed_array_len(program, *t)
                    {
                        body.exprs[expr_id] = Expr { kind: ExprKind::Int(n), ty: TyId::INT, span };
                    }
                }
                "__default" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        let mut seen = Vec::new();
                        match default_value_expr(program, interner, body, *t, span, &mut seen) {
                            Ok(rep) => body.exprs[expr_id] = body.exprs[rep].clone(),
                            Err(msg) => sink.emit(Diagnostic::error(msg, span)),
                        }
                    }
                }
                "has_field" => {
                    if let (Some(GenericArg::Type(t)), Some(&arg0)) = (generic_args.first(), args.first())
                        && let Ok(ConstVal::String(fname)) = eval_const_tree_expr(program, interner, None, body, arg0)
                    {
                        body.exprs[expr_id] = Expr { kind: ExprKind::Bool(type_has_field(program, *t, fname)), ty: TyId::BOOL, span };
                    }
                }
                "type_info" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        let val = type_info_val(program, interner, *t);
                        let rep = const_val_to_expr(program, body, &val, ty, span);
                        body.exprs[expr_id] = body.exprs[rep].clone();
                    }
                }
                "fields" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        let vals: Vec<ConstVal> = fields_vals(program, interner, *t).into_iter().map(|(v, _)| v).collect();
                        let rep = const_val_to_expr(program, body, &ConstVal::Array(vals), ty, span);
                        body.exprs[expr_id] = body.exprs[rep].clone();
                    }
                }
                "variants" => {
                    if let Some(GenericArg::Type(t)) = generic_args.first() {
                        let vals: Vec<ConstVal> = variants_vals(program, interner, *t).into_iter().map(|(v, _)| v).collect();
                        let rep = const_val_to_expr(program, body, &ConstVal::Array(vals), ty, span);
                        body.exprs[expr_id] = body.exprs[rep].clone();
                    }
                }
                "compile_error" => {
                    let msg = args
                        .first()
                        .and_then(|&a| eval_const_tree_expr(program, interner, None, body, a).ok())
                        .and_then(|v| match v {
                            ConstVal::String(s) => Some(interner.resolve(s).to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "compile-time error".to_string());
                    sink.emit(Diagnostic::error(msg, span));
                    body.exprs[expr_id] = Expr { kind: ExprKind::Null, ty: TyId::UNIT, span };
                }
                "compile_assert" => {
                    if let Some(&cond_e) = args.first()
                        && let Ok(ConstVal::Bool(ok)) = eval_const_tree_expr(program, interner, None, body, cond_e)
                        && !ok
                    {
                        let msg = args
                            .get(1)
                            .and_then(|&a| eval_const_tree_expr(program, interner, None, body, a).ok())
                            .and_then(|v| match v {
                                ConstVal::String(s) => Some(interner.resolve(s).to_string()),
                                _ => None,
                            })
                            .unwrap_or_else(|| "compile-time assertion failed".to_string());
                        sink.emit(Diagnostic::error(msg, span));
                    }
                    body.exprs[expr_id] = Expr { kind: ExprKind::Null, ty: TyId::UNIT, span };
                }
                _ => {}
            }
        }
        ExprKind::Call { callee: Callee::Value(v), args } => {
            lower_expr(program, interner, sink, body, v);
            for a in args {
                lower_expr(program, interner, sink, body, a);
            }
        }
        ExprKind::Call { args, .. } => {
            for a in args {
                lower_expr(program, interner, sink, body, a);
            }
        }
        ExprKind::Concat(items) | ExprKind::Tuple(items) | ExprKind::List(items) | ExprKind::Array(items) => {
            for i in items {
                lower_expr(program, interner, sink, body, i);
            }
        }
        ExprKind::Show(inner) | ExprKind::Cast(inner) | ExprKind::Unwrap(inner) | ExprKind::Propagate(inner) => {
            lower_expr(program, interner, sink, body, inner);
        }
        ExprKind::ArrayRepeat { value, len } => {
            lower_expr(program, interner, sink, body, value);
            lower_expr(program, interner, sink, body, len);
        }
        ExprKind::Map(entries) => {
            for (k, v) in entries {
                lower_expr(program, interner, sink, body, k);
                lower_expr(program, interner, sink, body, v);
            }
        }
        ExprKind::Record { fields, base } => {
            for fv in fields {
                lower_expr(program, interner, sink, body, fv.value);
            }
            if let Some(b) = base {
                lower_expr(program, interner, sink, body, b);
            }
        }
        ExprKind::Variant { fields, .. } => {
            for fv in fields {
                lower_expr(program, interner, sink, body, fv.value);
            }
        }
        ExprKind::Closure(mut c) => {
            c.body = lower_block(program, interner, sink, body, &c.body);
            body.exprs[expr_id].kind = ExprKind::Closure(c);
        }
        ExprKind::Spawn { mut task, scoped } => {
            task.body = lower_block(program, interner, sink, body, &task.body);
            body.exprs[expr_id].kind = ExprKind::Spawn { task, scoped };
        }
        ExprKind::Field { object, .. } | ExprKind::TupleField { object, .. } => lower_expr(program, interner, sink, body, object),
        ExprKind::SetField { object, value, .. } => {
            lower_expr(program, interner, sink, body, object);
            lower_expr(program, interner, sink, body, value);
        }
        ExprKind::Index { object, index } => {
            lower_expr(program, interner, sink, body, object);
            lower_expr(program, interner, sink, body, index);
        }
        ExprKind::Slice { object, start, end, step, .. } => {
            lower_expr(program, interner, sink, body, object);
            if let Some(s) = start {
                lower_expr(program, interner, sink, body, s);
            }
            if let Some(e) = end {
                lower_expr(program, interner, sink, body, e);
            }
            if let Some(st) = step {
                lower_expr(program, interner, sink, body, st);
            }
        }
        ExprKind::Unary { operand, .. } | ExprKind::Is { expr: operand, .. } | ExprKind::Convert { expr: operand, .. } => {
            lower_expr(program, interner, sink, body, operand);
        }
        ExprKind::Binary { left, right, .. } | ExprKind::Range { start: left, end: right, .. } => {
            lower_expr(program, interner, sink, body, left);
            lower_expr(program, interner, sink, body, right);
        }
        ExprKind::If { mut condition, then, otherwise } => {
            lower_condition(program, interner, sink, body, &mut condition);
            lower_expr(program, interner, sink, body, then);
            lower_expr(program, interner, sink, body, otherwise);
            body.exprs[expr_id].kind = ExprKind::If { condition, then, otherwise };
        }
        ExprKind::Match(mut m) => {
            lower_match(program, interner, sink, body, &mut m);
            body.exprs[expr_id].kind = ExprKind::Match(m);
        }
        _ => {}
    }
}

/// Ensures every `Local` exists cleanly without lint warnings.
#[allow(dead_code)]
fn _keep_local_kind(_: LocalKind) {}
