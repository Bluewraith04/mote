//! Single source of truth for `@name` and `@__name` compiler builtins and typechecking.

use mote_base::{Id, Span, Symbol};
use mote_syntax as ast;
use mote_syntax::{Arg, ExprKind as AstExprKind};

use crate::check::FnCx;
use crate::comptime;
use crate::cx::Cx;
use crate::resolve::Scope;
use crate::tree::{Callee, Expr, ExprKind, FieldValue, FnRef};
use crate::types::{BuiltinTy, ConstArg, ConstVal, GenericArg, TyId, TyKind};

/// All `@__...` builtin names (without `@`) that may park the calling task.
pub const PARKING_BUILTINS: &[&str] = &[
    "__stream_next",
    "__task_await",
    "__task_await_timeout",
    "__sleep",
    "__sender_send",
    "__receiver_recv",
    "__receiver_recv_timeout",
    "__mutex_take",
    "__once_begin",
];

/// Every runtime intrinsic name corresponding to a `@name` or `@__name` builtin
/// (with the leading `__` stripped for internal builtins).
pub const RUNTIME_INTRINSIC_NAMES: &[&str] = &[
    // public
    "print",
    "println",
    "eprint",
    "eprintln",
    "fault",
    "index_out_of_range",
    // the runtime name of a value's type, `std.io.typeof`
    "type_tag",
    // system / port
    "sys_read_line",
    "sys_args",
    "sys_exit",
    "sys_now",
    "port_open",
    // numbers & floats & math
    "float_is_nan",
    "float_is_infinite",
    "float_to_int",
    "num_to_string",
    "num_wrapping_add",
    "num_wrapping_sub",
    "num_wrapping_mul",
    "num_wrapping_neg",
    "num_wrapping_shl",
    "num_wrapping_shr",
    "num_checked_add",
    "num_checked_sub",
    "num_checked_mul",
    "num_checked_neg",
    "num_checked_shl",
    "num_checked_shr",
    "num_saturating_add",
    "num_saturating_sub",
    "num_saturating_mul",
    "num_saturating_neg",
    "num_saturating_shl",
    "num_saturating_shr",
    "num_overflowing_add",
    "num_overflowing_sub",
    "num_overflowing_mul",
    "num_overflowing_neg",
    "num_overflowing_shl",
    "num_overflowing_shr",
    "math_sqrt",
    "math_floor",
    "math_ceil",
    "math_round",
    "math_pow",
    "math_exp",
    "math_ln",
    "math_log10",
    "math_sin",
    "math_cos",
    "math_tan",
    "math_atan2",
    // text / parse
    "parse_int",
    "parse_float",
    "string_len",
    "string_concat",
    "string_replace",
    "string_slice",
    "string_byte_at",
    "string_is_boundary",
    "string_to_upper",
    "string_to_lower",
    "string_trim",
    "string_repeat",
    "string_find",
    "string_rfind",
    "string_split",
    "string_bytes",
    "string_from_bytes",
    // lists & runtime helpers
    "list_new",
    "list_len",
    "list_push",
    "list_pop",
    "list_get",
    "list_set",
    "list_clear",
    "list_contains",
    "list_slice",
    "list_set_slice",
    "list_clone",
    "deep_clone",
    "hash_of",
    "key_eq",
    "equal_of",
    // arrays & masks
    "array_len",
    "array_get",
    "array_set",
    "array_contains",
    "array_slice",
    "array_clone",
    "mask_all",
    "mask_any",
    "mask_none",
    "array_all_by",
    "array_any_by",
    "array_none_by",
    "array_count",
    "array_count_by",
    "array_sum",
    "array_product",
    "array_mean",
    "array_min",
    "array_max",
    "array_argmin",
    "array_argmax",
    "array_dot",
    "array_norm_sq",
    "array_fold",
    "array_reduce",
    "array_scan",
    "array_find",
    "array_find_index",
    "array_index_of",
    "array_select",
    "array_map",
    "array_zip_with",
    "array_reverse",
    "array_clamp",
    "array_filter",
    "array_zip",
    "array_enumerate",
    "array_concat",
    "array_take",
    "array_drop",
    "array_flatten",
    "array_sort",
    "array_sort_by",
    // streams & tasks
    "stream_next",
    "task_await",
    "task_await_timeout",
    "task_cancel",
    "task_is_ready",
    "spawn_call",
    "task_view",
    "task_value",
    "sleep",
    // channels
    "channel_new",
    "sender_send",
    "sender_try_send",
    "sender_close",
    "sender_clone",
    "receiver_recv",
    "receiver_recv_timeout",
    "receiver_try_recv",
    "receiver_is_closed",
    "receiver_close",
    "receiver_clone",
    // mutex / once / atomics
    "mutex_new",
    "mutex_take",
    "mutex_put",
    "once_new",
    "once_begin",
    "once_finish",
    "once_value",
    "atomic_int_new",
    "atomic_int_load",
    "atomic_int_store",
    "atomic_int_swap",
    "atomic_int_compare_exchange",
    "atomic_int_fetch_add",
    "atomic_int_fetch_sub",
    "atomic_bool_new",
    "atomic_bool_load",
    "atomic_bool_store",
    "atomic_bool_swap",
    "atomic_bool_compare_exchange",
    // pointers / off-heap memory
    "ptr_null",
    "ptr_from_addr",
    "ptr_alloc",
    "ptr_realloc",
    "ptr_free",
    "ptr_offset",
    "ptr_load",
    "ptr_store",
    "ptr_copy",
    "ptr_zero",
    "ptr_cast",
];

/// Maps a Mote `@name` / `@__name` builtin to its IR runtime intrinsic name, if it lowers to one.
pub fn runtime_name(builtin: &str) -> Option<&'static str> {
    if matches!(builtin, "print" | "println" | "eprint" | "eprintln" | "fault") {
        return RUNTIME_INTRINSIC_NAMES.iter().copied().find(|&r| r == builtin);
    }
    let stripped = builtin.strip_prefix("__")?;
    RUNTIME_INTRINSIC_NAMES.iter().copied().find(|&r| r == stripped)
}

/// Whether `name` is a compile-time reflection/control builtin rather than a runtime intrinsic.
pub fn is_comptime_builtin(name: &str) -> bool {
    matches!(
        name,
        "type_info"
            | "typeof"
            | "type_name"
            | "size_of"
            | "align_of"
            | "is_flat"
            | "array_len"
            | "fields"
            | "variants"
            | "has_field"
            | "field"
            | "set_field"
            | "construct"
            | "compile_error"
            | "compile_assert"
            | "__default"
            | "__construct_variant"
            | "__with_variant_field"
            | "__with_field"
            | "__duplicate"
            | "__variant_is"
            | "__variant_fields"
            | "__variant_field"
    )
}

/// Whether `name` (raw symbol text after `@`) is a known compiler builtin.
pub fn is_known(name: &str) -> bool {
    is_comptime_builtin(name) || runtime_name(name).is_some()
}

/// Whether `name` expects its first value argument to be a mutable `var` place.
pub fn first_arg_writes(builtin: &str) -> bool {
    matches!(
        builtin,
        "set_field"
            | "__list_push"
            | "__list_pop"
            | "__list_set"
            | "__list_clear"
            | "__list_set_slice"
            | "__array_set"
            | "__stream_next"
    )
}

impl Cx<'_> {
    /// Checks a `@name[<...>](...)` expression and lowers it into typed `Body` nodes.
    pub(crate) fn check_builtin(
        &mut self,
        f: &mut FnCx,
        name: ast::Ident,
        written_generics: &[ast::GenericArg],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Id<Expr> {
        let raw = self.text(name.name);
        if is_comptime_builtin(&raw) {
            return self.check_comptime_builtin(f, name, &raw, written_generics, args, expected, span);
        }
        let Some(rt_name) = runtime_name(&raw) else {
            self.error(name.span, format!("unknown compiler builtin `@{raw}`"));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let scope = f.scope.clone();
        let written_tys: Vec<TyId> = written_generics
            .iter()
            .filter_map(|g| match g {
                ast::GenericArg::Type(ty) => Some(self.lower_type_with(&scope, Some(f), *ty)),
                ast::GenericArg::Expr(e) => {
                    self.error(self.ast(f.module)[*e].span, format!("`@{raw}` does not take a const generic argument"));
                    None
                }
            })
            .collect();

        if let Some(id) = self.check_array_or_mask_builtin(f, name.name, rt_name, &written_tys, args, expected, span) {
            return id;
        }
        self.check_runtime_builtin(f, name.name, rt_name, &written_tys, args, expected, span)
    }

    /// Evaluates `@typeof`, `@TypeArg`, `@FieldType`, and `@VariantFieldType` in type position.
    pub(crate) fn check_type_builtin(
        &mut self,
        scope: &Scope,
        f: Option<&mut FnCx>,
        name: ast::Ident,
        generic_args: &[ast::GenericArg],
        args: &[Arg],
        span: Span,
    ) -> TyId {
        let mut temp;
        let fcx = match f {
            Some(existing) => existing,
            None => {
                temp = FnCx::new(scope.module, scope.clone(), TyId::UNIT);
                &mut temp
            }
        };
        let raw = self.text(name.name);
        match raw.as_str() {
            "typeof" => {
                let expr_id = match (generic_args, args) {
                    ([], [Arg { name: None, value }]) => *value,
                    ([ast::GenericArg::Expr(value)], []) => *value,
                    _ => {
                        self.error(span, "`@typeof` takes one expression argument");
                        return TyId::ERROR;
                    }
                };
                let checked = self.expr(fcx, expr_id, None);
                self.solved(fcx, fcx.body.exprs[checked].ty)
            }
            "TypeArg" => {
                let (target_ty, rest) = self.resolve_builtin_type_and_extra_args(fcx, &raw, generic_args, args, 1, span);
                if target_ty == TyId::ERROR || rest.len() != 1 {
                    return TyId::ERROR;
                }
                let idx_expr = self.expr(fcx, rest[0].value, Some(TyId::INT));
                let idx_val = match comptime::eval_const_expr(self, fcx, idx_expr) {
                    Ok(ConstVal::Int(n, _)) if n >= 0 => n as usize,
                    _ => {
                        self.error(span, "`@TypeArg` index must be a non-negative constant integer");
                        return TyId::ERROR;
                    }
                };
                let out = match self.program.types.kind(target_ty).clone() {
                    TyKind::Optional(inner) | TyKind::Array(inner) | TyKind::FixedArray(inner, _) if idx_val == 0 => Some(inner),
                    TyKind::Tuple(els) | TyKind::Union(els) => els.get(idx_val).copied(),
                    TyKind::Builtin { args: type_args, .. } => type_args.get(idx_val).copied(),
                    TyKind::Adt { args: type_args, .. } => type_args
                        .into_iter()
                        .filter_map(|g| match g {
                            GenericArg::Type(t) => Some(t),
                            GenericArg::Const(_) => None,
                        })
                        .nth(idx_val),
                    _ => None,
                };
                if let Some(t) = out {
                    t
                } else {
                    self.error(span, format!("type {} has no type argument at index {idx_val}", self.show_ty(target_ty)));
                    TyId::ERROR
                }
            }
            "FieldType" => {
                let (target_ty, rest) = self.resolve_builtin_type_and_extra_args(fcx, &raw, generic_args, args, 1, span);
                if target_ty == TyId::ERROR || rest.len() != 1 {
                    return TyId::ERROR;
                }
                let key_expr = self.expr(fcx, rest[0].value, None);
                let Ok(key_val) = comptime::eval_const_expr(self, fcx, key_expr) else {
                    self.error(span, "`@FieldType` requires a constant field name or index");
                    return TyId::ERROR;
                };
                match self.resolve_record_or_tuple_field(target_ty, &key_val) {
                    Some((_, fty)) => fty,
                    None => {
                        self.error(span, format!("type {} has no matching field", self.show_ty(target_ty)));
                        TyId::ERROR
                    }
                }
            }
            "VariantFieldType" => {
                let (target_ty, rest) = self.resolve_builtin_type_and_extra_args(fcx, &raw, generic_args, args, 2, span);
                if target_ty == TyId::ERROR || rest.len() != 2 {
                    return TyId::ERROR;
                }
                let v_expr = self.expr(fcx, rest[0].value, Some(TyId::STRING));
                let Ok(ConstVal::String(v_name)) = comptime::eval_const_expr(self, fcx, v_expr) else {
                    self.error(span, "`@VariantFieldType` requires a constant variant name");
                    return TyId::ERROR;
                };
                let f_expr = self.expr(fcx, rest[1].value, None);
                let Ok(f_val) = comptime::eval_const_expr(self, fcx, f_expr) else {
                    self.error(span, "`@VariantFieldType` requires a constant field name or index");
                    return TyId::ERROR;
                };
                match self.resolve_variant_field(target_ty, v_name, &f_val) {
                    Some((_, _, fty)) => fty,
                    None => {
                        let vn = self.text(v_name);
                        self.error(span, format!("variant `{vn}` of {} has no matching field", self.show_ty(target_ty)));
                        TyId::ERROR
                    }
                }
            }
            _ => {
                self.error(name.span, format!("unknown type builtin `@{raw}`"));
                TyId::ERROR
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_comptime_builtin(
        &mut self,
        f: &mut FnCx,
        name: ast::Ident,
        raw: &str,
        written_generics: &[ast::GenericArg],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Id<Expr> {
        let sym = name.name;
        let emit_builtin = |f: &mut FnCx, generic_args: Vec<GenericArg>, arg_exprs: Vec<Id<Expr>>, ty: TyId| {
            f.body.expr(
                ExprKind::Call {
                    callee: Callee::Builtin { name: sym, generic_args },
                    args: arg_exprs,
                },
                ty,
                span,
            )
        };
        match raw {
            "compile_error" => {
                if !written_generics.is_empty() || args.len() != 1 || args[0].name.is_some() {
                    self.error(span, "`@compile_error` takes 1 String argument");
                    return f.body.expr(ExprKind::Error, TyId::UNIT, span);
                }
                let msg_expr = self.expr(f, args[0].value, Some(TyId::STRING));
                let msg_expr = self.coerce(f, msg_expr, TyId::STRING, self.ast(f.module)[args[0].value].span);
                match comptime::eval_const_expr(self, f, msg_expr) {
                    Ok(ConstVal::String(s)) => {
                        let text = self.text(s);
                        self.error(span, text);
                    }
                    Ok(_) => self.error(span, "`@compile_error` expects a String message"),
                    Err(msg) => {
                        // Allow deferral if expression depends on a deferred generic const/type parameter.
                        if !self.program.types.has_param(f.body.exprs[msg_expr].ty) && !f.body.exprs[msg_expr].ty.eq(&TyId::ERROR) {
                            self.error(span, msg);
                        }
                    }
                }
                f.body.expr(ExprKind::Null, TyId::UNIT, span)
            }
            "compile_assert" => {
                if !written_generics.is_empty() || args.len() != 2 || args[0].name.is_some() || args[1].name.is_some() {
                    self.error(span, "`@compile_assert` takes `(cond: Bool, message: String)`");
                    return f.body.expr(ExprKind::Error, TyId::UNIT, span);
                }
                let cond_expr = self.expr(f, args[0].value, Some(TyId::BOOL));
                let cond_expr = self.coerce(f, cond_expr, TyId::BOOL, self.ast(f.module)[args[0].value].span);
                let msg_expr = self.expr(f, args[1].value, Some(TyId::STRING));
                let msg_expr = self.coerce(f, msg_expr, TyId::STRING, self.ast(f.module)[args[1].value].span);
                if let Ok(ConstVal::Bool(ok)) = comptime::eval_const_expr(self, f, cond_expr) {
                    if !ok {
                        let msg = match comptime::eval_const_expr(self, f, msg_expr) {
                            Ok(ConstVal::String(s)) => self.text(s),
                            _ => "compile-time assertion failed".to_string(),
                        };
                        self.error(span, msg);
                    }
                    return f.body.expr(ExprKind::Null, TyId::UNIT, span);
                }
                emit_builtin(f, Vec::new(), vec![cond_expr, msg_expr], TyId::UNIT)
            }
            "typeof" => {
                if !written_generics.is_empty() || args.len() != 1 || args[0].name.is_some() {
                    self.error(span, "`@typeof` takes 1 expression argument");
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
                // `@typeof(x)` is a type, not a value: reflection builtins and type positions take it through `expr_as_type_arg`.
                self.error(span, "`@typeof(x)` is a type, not a value: pass it to `@type_name`, `@type_info`, `@fields` and the like, bind it with `const`, or write it as a type (`std.io.typeof(x)` gives the runtime name as a String)");
                f.body.expr(ExprKind::Error, TyId::ERROR, span)
            }
            "type_name" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                if target_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::STRING, span);
                }
                if !self.program.types.has_param(target_ty) {
                    let rendered = self.program.render_source(target_ty, self.interner);
                    let sym_text = self.interner.intern(&rendered);
                    return f.body.expr(ExprKind::String(sym_text), TyId::STRING, span);
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), TyId::STRING)
            }
            "size_of" | "align_of" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                if target_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::INT, span);
                }
                if !self.program.types.has_param(target_ty) {
                    let value = if raw == "size_of" {
                        comptime::layout_size(self.program, target_ty)
                    } else {
                        comptime::layout_align(self.program, target_ty)
                    };
                    return f.body.expr(ExprKind::Int(value as i128), TyId::INT, span);
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), TyId::INT)
            }
            "is_flat" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                if target_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::BOOL, span);
                }
                if !self.program.types.has_param(target_ty) {
                    return f.body.expr(ExprKind::Bool(comptime::is_flat(self.program, target_ty)), TyId::BOOL, span);
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), TyId::BOOL)
            }
            "has_field" => {
                let (target_ty, name_arg) = self.resolve_builtin_type_and_extra_args(f, raw, written_generics, args, 1, span);
                if target_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::BOOL, span);
                }
                let name_expr = self.expr(f, name_arg[0].value, Some(TyId::STRING));
                let name_expr = self.coerce(f, name_expr, TyId::STRING, self.ast(f.module)[name_arg[0].value].span);
                if !self.program.types.has_param(target_ty)
                    && let Ok(ConstVal::String(field_name)) = comptime::eval_const_expr(self, f, name_expr)
                {
                    let has = comptime::type_has_field(self.program, target_ty, field_name);
                    return f.body.expr(ExprKind::Bool(has), TyId::BOOL, span);
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], vec![name_expr], TyId::BOOL)
            }
            "type_info" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                let ret_ty = self.program.std.type_info.map_or(TyId::ERROR, |def| self.program.types.adt_types(def, Vec::new()));
                if ret_ty == TyId::ERROR && target_ty != TyId::ERROR {
                    self.error(span, "`@type_info` requires `std.reflect.TypeInfo` to be available");
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), ret_ty)
            }
            "fields" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                let ret_ty = self
                    .program
                    .std
                    .field_info
                    .map_or(TyId::ERROR, |def| {
                        let elem = self.program.types.adt_types(def, Vec::new());
                        self.program.types.array(elem)
                    });
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), ret_ty)
            }
            "variants" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                let ret_ty = self
                    .program
                    .std
                    .variant_info
                    .map_or(TyId::ERROR, |def| {
                        let elem = self.program.types.adt_types(def, Vec::new());
                        self.program.types.array(elem)
                    });
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), ret_ty)
            }
            "field" => {
                if !written_generics.is_empty() || args.len() != 2 || args[0].name.is_some() || args[1].name.is_some() {
                    self.error(span, "`@field` takes `(value, field_name: String)`");
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
                let obj = self.expr(f, args[0].value, None);
                let obj_ty = self.solved(f, f.body.exprs[obj].ty);
                let name_expr = self.expr(f, args[1].value, Some(TyId::STRING));
                let name_expr = self.coerce(f, name_expr, TyId::STRING, self.ast(f.module)[args[1].value].span);
                let Ok(ConstVal::String(field_name)) = comptime::eval_const_expr(self, f, name_expr) else {
                    self.error(self.ast(f.module)[args[1].value].span, "`@field` requires a compile-time constant field name");
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                };
                if obj_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
                if let TyKind::Tuple(members) = self.program.types.kind(obj_ty).clone() {
                    let field_str = self.text(field_name);
                    if let Ok(index) = field_str.parse::<usize>()
                        && let Some(&field_ty) = members.get(index)
                    {
                        return f.body.expr(ExprKind::TupleField { object: obj, index: index as u32 }, field_ty, span);
                    }
                    self.error(span, format!("type {} has no field `{field_str}`", self.show_ty(obj_ty)));
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
                match comptime::lookup_record_field(self.program, obj_ty, field_name) {
                    Some((index, field_ty)) => f.body.expr(ExprKind::Field { object: obj, index }, field_ty, span),
                    None => {
                        let fn_str = self.text(field_name);
                        self.error(span, format!("type {} has no field `{fn_str}`", self.show_ty(obj_ty)));
                        f.body.expr(ExprKind::Error, TyId::ERROR, span)
                    }
                }
            }
            "__variant_fields" => {
                let (target_ty, rest) = self.resolve_builtin_type_and_extra_args(f, raw, written_generics, args, 1, span);
                let ret_ty = self
                    .program
                    .std
                    .field_info
                    .map_or(TyId::ERROR, |def| {
                        let elem = self.program.types.adt_types(def, Vec::new());
                        self.program.types.array(elem)
                    });
                if target_ty == TyId::ERROR || rest.len() != 1 {
                    return f.body.expr(ExprKind::Error, ret_ty, span);
                }
                let v_expr = self.expr(f, rest[0].value, Some(TyId::STRING));
                let v_expr = self.coerce(f, v_expr, TyId::STRING, self.ast(f.module)[rest[0].value].span);
                let Ok(ConstVal::String(v_name)) = comptime::eval_const_expr(self, f, v_expr) else {
                    self.error(span, "`@__variant_fields` requires a constant variant name");
                    return f.body.expr(ExprKind::Error, ret_ty, span);
                };
                let vals: Vec<ConstVal> = comptime::variant_fields_vals(self.program, self.interner, target_ty, v_name)
                    .into_iter()
                    .map(|(v, _)| v)
                    .collect();
                comptime::const_val_to_expr(self.program, &mut f.body, &ConstVal::Array(vals), ret_ty, span)
            }
            "__variant_is" => self.check_variant_is_builtin(f, written_generics, args, span),
            "__variant_field" => self.check_variant_field_builtin(f, written_generics, args, span),
            "__duplicate" => self.check_duplicate_builtin(f, written_generics, args, span),
            "set_field" => {
                if !written_generics.is_empty() || args.len() != 3 || args.iter().any(|a| a.name.is_some()) {
                    self.error(span, "`@set_field` takes `(var value, field_name: String, new_value)`");
                    return f.body.expr(ExprKind::Error, TyId::UNIT, span);
                }
                let obj = self.expr(f, args[0].value, None);
                let obj_ty = self.solved(f, f.body.exprs[obj].ty);
                let name_expr = self.expr(f, args[1].value, Some(TyId::STRING));
                let name_expr = self.coerce(f, name_expr, TyId::STRING, self.ast(f.module)[args[1].value].span);
                let Ok(ConstVal::String(field_name)) = comptime::eval_const_expr(self, f, name_expr) else {
                    self.error(self.ast(f.module)[args[1].value].span, "`@set_field` requires a compile-time constant field name");
                    return f.body.expr(ExprKind::Error, TyId::UNIT, span);
                };
                let (index, field_ty) = match comptime::lookup_record_field(self.program, obj_ty, field_name) {
                    Some(found) => found,
                    None => {
                        let fn_str = self.text(field_name);
                        self.error(span, format!("type {} has no field `{fn_str}`", self.show_ty(obj_ty)));
                        let _ = self.expr(f, args[2].value, None);
                        return f.body.expr(ExprKind::Error, TyId::UNIT, span);
                    }
                };
                if Self::is_place(f, obj) && !self.writable(f, obj) {
                    self.error(span, "this place has no `var` path to write through");
                }
                let val_expr = self.expr(f, args[2].value, Some(field_ty));
                let val_expr = self.coerce(f, val_expr, field_ty, self.ast(f.module)[args[2].value].span);
                f.body.expr(ExprKind::SetField { object: obj, index, value: val_expr }, TyId::UNIT, span)
            }
            "array_len" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                if target_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::INT, span);
                }
                if !self.program.types.has_param(target_ty) {
                    if let Some(n) = comptime::fixed_array_len(self.program, target_ty) {
                        return f.body.expr(ExprKind::Int(n), TyId::INT, span);
                    }
                    self.error(span, format!("`@array_len` expects a fixed-size array `[T; N]`, found {}", self.show_ty(target_ty)));
                    return f.body.expr(ExprKind::Error, TyId::INT, span);
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), TyId::INT)
            }
            "__default" => {
                let target_ty = self.resolve_builtin_type_arg(f, raw, written_generics, args, 0, span);
                if target_ty == TyId::ERROR {
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
                if !self.program.types.has_param(target_ty) {
                    let mut seen = Vec::new();
                    return match comptime::default_value_expr(self.program, self.interner, &mut f.body, target_ty, span, &mut seen) {
                        Ok(expr_id) => expr_id,
                        Err(msg) => {
                            self.error(span, msg);
                            f.body.expr(ExprKind::Error, target_ty, span)
                        }
                    };
                }
                emit_builtin(f, vec![GenericArg::Type(target_ty)], Vec::new(), target_ty)
            }
            "__construct_variant" => {
                let (target_ty, rest) = self.resolve_builtin_type_and_extra_args(f, raw, written_generics, args, 1, span);
                if target_ty == TyId::ERROR || rest.len() != 1 {
                    return f.body.expr(ExprKind::Error, target_ty, span);
                }
                let v_expr = self.expr(f, rest[0].value, Some(TyId::STRING));
                let v_expr = self.coerce(f, v_expr, TyId::STRING, self.ast(f.module)[rest[0].value].span);
                let Ok(ConstVal::String(v_name)) = comptime::eval_const_expr(self, f, v_expr) else {
                    self.error(span, "`@__construct_variant` requires a constant variant name");
                    return f.body.expr(ExprKind::Error, target_ty, span);
                };
                let TyKind::Adt { def, args: type_args } = self.program.types.kind(target_ty).clone() else {
                    self.error(span, format!("expected an enum, found {}", self.show_ty(target_ty)));
                    return f.body.expr(ExprKind::Error, target_ty, span);
                };
                let Some(v_idx) = self.program.defs[def]
                    .enum_def()
                    .and_then(|en| en.variants.iter().position(|v| v.name == v_name))
                else {
                    let vn = self.text(v_name);
                    self.error(span, format!("{} has no variant `{vn}`", self.show_ty(target_ty)));
                    return f.body.expr(ExprKind::Error, target_ty, span);
                };
                let mut seen = Vec::new();
                match comptime::default_variant_expr(self.program, self.interner, &mut f.body, target_ty, def, &type_args, v_idx, span, &mut seen) {
                    Ok(expr_id) => expr_id,
                    Err(msg) => {
                        self.error(span, msg);
                        f.body.expr(ExprKind::Error, target_ty, span)
                    }
                }
            }
            "__with_variant_field" => self.check_with_variant_field_builtin(f, written_generics, args, span),
            "__with_field" => self.check_with_field_builtin(f, written_generics, args, span),
            "construct" => self.check_construct_builtin(f, written_generics, args, expected, span),
            _ => {
                self.error(name.span, format!("unknown compiler builtin `@{raw}`"));
                f.body.expr(ExprKind::Error, TyId::ERROR, span)
            }
        }
    }

    fn check_construct_builtin(
        &mut self,
        f: &mut FnCx,
        written_generics: &[ast::GenericArg],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Id<Expr> {
        let scope = f.scope.clone();
        let target_ty = match written_generics {
            [ast::GenericArg::Type(t)] => self.lower_type_with(&scope, Some(f), *t),
            [] => match expected {
                Some(ty) => self.solved(f, ty),
                None => {
                    self.error(span, "`@construct` needs a target type `@construct<T>(...)`");
                    return f.body.expr(ExprKind::Error, TyId::ERROR, span);
                }
            },
            _ => {
                self.error(span, "`@construct` takes 1 type argument `<T>`");
                return f.body.expr(ExprKind::Error, TyId::ERROR, span);
            }
        };
        let target_ty = self.solved(f, target_ty);
        if target_ty == TyId::ERROR {
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        }
        let TyKind::Adt { def, args: type_args } = self.program.types.kind(target_ty).clone() else {
            self.error(span, format!("`@construct` expects a struct or class, found {}", self.show_ty(target_ty)));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let Some(record) = self.program.defs[def].record().map(|r| r.fields.clone()) else {
            self.error(span, format!("`@construct` expects a struct or class, found {}", self.show_ty(target_ty)));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let subst_fields: Vec<(Symbol, TyId, bool)> = record
            .iter()
            .map(|field| (field.name, self.program.types.subst_generic(field.ty, &type_args), field.default.is_some()))
            .collect();

        if args.len() != subst_fields.len() {
            self.error(
                span,
                format!("`@construct` for {} expects {} argument(s), found {}", self.show_ty(target_ty), subst_fields.len(), args.len()),
            );
            for arg in args {
                let _ = self.expr(f, arg.value, None);
            }
            return f.body.expr(ExprKind::Error, target_ty, span);
        }

        let mut field_values = Vec::with_capacity(subst_fields.len());
        let all_named = args.iter().all(|a| a.name.is_some());
        if all_named {
            let mut seen = vec![false; subst_fields.len()];
            for arg in args {
                let Some(arg_name) = arg.name else { unreachable!() };
                if let Some((idx, (_, field_ty, _))) = subst_fields.iter().enumerate().find(|(_, (n, _, _))| *n == arg_name.name) {
                    if seen[idx] {
                        self.error(arg_name.span, format!("duplicate field `{}`", self.text(arg_name.name)));
                    }
                    seen[idx] = true;
                    let val = self.expr(f, arg.value, Some(*field_ty));
                    let val = self.coerce(f, val, *field_ty, self.ast(f.module)[arg.value].span);
                    field_values.push(FieldValue { index: idx as u32, value: val });
                } else {
                    self.error(arg_name.span, format!("{} has no field `{}`", self.show_ty(target_ty), self.text(arg_name.name)));
                    let _ = self.expr(f, arg.value, None);
                }
            }
        } else {
            for (idx, (arg, (_, field_ty, _))) in args.iter().zip(subst_fields.iter()).enumerate() {
                if arg.name.is_some() {
                    self.error(span, "cannot mix named and positional arguments in `@construct`");
                }
                let val = self.expr(f, arg.value, Some(*field_ty));
                let val = self.coerce(f, val, *field_ty, self.ast(f.module)[arg.value].span);
                field_values.push(FieldValue { index: idx as u32, value: val });
            }
        }
        field_values.sort_by_key(|fv| fv.index);
        f.body.expr(ExprKind::Record { fields: field_values, base: None }, target_ty, span)
    }

    fn enum_variant_payload(&mut self, enum_ty: TyId, variant_idx: usize) -> Vec<(Option<Symbol>, TyId)> {
        let TyKind::Adt { def, args } = self.program.types.kind(enum_ty).clone() else {
            return Vec::new();
        };
        let Some(payload) = self.program.defs[def].enum_def().and_then(|en| en.variants.get(variant_idx)).map(|v| v.payload.clone()) else {
            return Vec::new();
        };
        match payload {
            crate::defs::Payload::Unit => Vec::new(),
            crate::defs::Payload::Tuple(ts) => ts
                .into_iter()
                .map(|t| (None, self.program.types.subst_generic(t, &args)))
                .collect(),
            crate::defs::Payload::Struct(fs) => fs
                .into_iter()
                .map(|fl| (Some(fl.name), self.program.types.subst_generic(fl.ty, &args)))
                .collect(),
        }
    }

    fn check_variant_is_builtin(&mut self, f: &mut FnCx, written_generics: &[ast::GenericArg], args: &[Arg], span: Span) -> Id<Expr> {
        if !written_generics.is_empty() || args.len() != 2 || args.iter().any(|a| a.name.is_some()) {
            self.error(span, "`@__variant_is` takes `(value, variant_name: String)`");
            return f.body.expr(ExprKind::Error, TyId::BOOL, span);
        }
        let obj = self.expr(f, args[0].value, None);
        let obj_ty = self.solved(f, f.body.exprs[obj].ty);
        let name_expr = self.expr(f, args[1].value, Some(TyId::STRING));
        let name_expr = self.coerce(f, name_expr, TyId::STRING, self.ast(f.module)[args[1].value].span);
        let Ok(ConstVal::String(v_name)) = comptime::eval_const_expr(self, f, name_expr) else {
            self.error(span, "`@__variant_is` requires a constant variant name");
            return f.body.expr(ExprKind::Error, TyId::BOOL, span);
        };
        let TyKind::Adt { def, .. } = self.program.types.kind(obj_ty).clone() else {
            self.error(span, format!("expected an enum, found {}", self.show_ty(obj_ty)));
            return f.body.expr(ExprKind::Error, TyId::BOOL, span);
        };
        let Some(en) = self.program.defs[def].enum_def().cloned() else {
            self.error(span, format!("expected an enum, found {}", self.show_ty(obj_ty)));
            return f.body.expr(ExprKind::Error, TyId::BOOL, span);
        };
        let Some((target_idx, _)) = en.variants.iter().enumerate().find(|(_, v)| v.name == v_name) else {
            let name_s = self.text(v_name);
            self.error(span, format!("{} has no variant `{name_s}`", self.show_ty(obj_ty)));
            return f.body.expr(ExprKind::Error, TyId::BOOL, span);
        };
        let field_pat = self
            .enum_variant_payload(obj_ty, target_idx)
            .into_iter()
            .enumerate()
            .map(|(i, (_, pty))| crate::tree::FieldPattern {
                index: i as u32,
                pattern: f.body.pattern(crate::tree::PatternKind::Wildcard, pty, span),
            })
            .collect();
        let match_pat = f.body.pattern(crate::tree::PatternKind::Variant { variant: target_idx as u32, fields: field_pat }, obj_ty, span);
        let t_expr = f.body.expr(ExprKind::Bool(true), TyId::BOOL, span);
        let f_expr = f.body.expr(ExprKind::Bool(false), TyId::BOOL, span);
        let wild_pat = f.body.pattern(crate::tree::PatternKind::Wildcard, obj_ty, span);
        f.body.expr(
            ExprKind::Match(crate::tree::Match {
                subject: obj,
                arms: vec![
                    crate::tree::Arm { pattern: match_pat, guard: None, body: crate::tree::ArmBody::Value(t_expr), span },
                    crate::tree::Arm { pattern: wild_pat, guard: None, body: crate::tree::ArmBody::Value(f_expr), span },
                ],
            }),
            TyId::BOOL,
            span,
        )
    }

    fn check_variant_field_builtin(&mut self, f: &mut FnCx, written_generics: &[ast::GenericArg], args: &[Arg], span: Span) -> Id<Expr> {
        if !written_generics.is_empty() || args.len() != 3 || args.iter().any(|a| a.name.is_some()) {
            self.error(span, "`@__variant_field` takes `(value, variant_name: String, field_name: String)`");
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        }
        let obj = self.expr(f, args[0].value, None);
        let obj_ty = self.solved(f, f.body.exprs[obj].ty);
        let v_expr = self.expr(f, args[1].value, Some(TyId::STRING));
        let v_expr = self.coerce(f, v_expr, TyId::STRING, self.ast(f.module)[args[1].value].span);
        let f_expr = self.expr(f, args[2].value, Some(TyId::STRING));
        let f_expr = self.coerce(f, f_expr, TyId::STRING, self.ast(f.module)[args[2].value].span);
        let (Ok(ConstVal::String(v_name)), Ok(ConstVal::String(f_name))) = (
            comptime::eval_const_expr(self, f, v_expr),
            comptime::eval_const_expr(self, f, f_expr),
        ) else {
            self.error(span, "`@__variant_field` requires constant variant and field names");
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let TyKind::Adt { def, .. } = self.program.types.kind(obj_ty).clone() else {
            self.error(span, format!("expected an enum, found {}", self.show_ty(obj_ty)));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let Some(en) = self.program.defs[def].enum_def().cloned() else {
            self.error(span, format!("expected an enum, found {}", self.show_ty(obj_ty)));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let Some((v_idx, _)) = en.variants.iter().enumerate().find(|(_, v)| v.name == v_name) else {
            let vn = self.text(v_name);
            self.error(span, format!("{} has no variant `{vn}`", self.show_ty(obj_ty)));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let payload = self.enum_variant_payload(obj_ty, v_idx);
        let f_str = self.text(f_name);
        let target_field = payload.iter().enumerate().find(|(i, (nm, _))| match nm {
            Some(sym) => *sym == f_name,
            None => f_str == i.to_string(),
        });
        let Some((target_fidx, &(_, field_ty))) = target_field else {
            self.error(span, format!("variant has no field `{f_str}`"));
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        };
        let tmp_sym = self.interner.intern("_vf");
        let mut field_pats = Vec::with_capacity(payload.len());
        let mut bound_local = None;
        for (i, (_, pty)) in payload.iter().enumerate() {
            let pat = if i == target_fidx {
                let loc = f.body.local(crate::tree::Local {
                    name: tmp_sym,
                    ty: *pty,
                    is_var: false,
                    kind: crate::tree::LocalKind::Pattern,
                    span,
                });
                bound_local = Some(loc);
                f.body.pattern(crate::tree::PatternKind::Bind(loc), *pty, span)
            } else {
                f.body.pattern(crate::tree::PatternKind::Wildcard, *pty, span)
            };
            field_pats.push(crate::tree::FieldPattern { index: i as u32, pattern: pat });
        }
        let match_pat = f.body.pattern(crate::tree::PatternKind::Variant { variant: v_idx as u32, fields: field_pats }, obj_ty, span);
        let hit_expr = f.body.expr(ExprKind::Local(bound_local.expect("field index exists")), field_ty, span);
        let wild_pat = f.body.pattern(crate::tree::PatternKind::Wildcard, obj_ty, span);
        let fault_sym = self.interner.intern("fault");
        let fault_msg = self.interner.intern("variant field mismatch");
        let msg_expr = f.body.expr(ExprKind::String(fault_msg), TyId::STRING, span);
        let miss_expr = f.body.expr(
            ExprKind::Call {
                callee: Callee::Builtin { name: fault_sym, generic_args: vec![GenericArg::Type(field_ty)] },
                args: vec![msg_expr],
            },
            field_ty,
            span,
        );
        f.body.expr(
            ExprKind::Match(crate::tree::Match {
                subject: obj,
                arms: vec![
                    crate::tree::Arm { pattern: match_pat, guard: None, body: crate::tree::ArmBody::Value(hit_expr), span },
                    crate::tree::Arm { pattern: wild_pat, guard: None, body: crate::tree::ArmBody::Value(miss_expr), span },
                ],
            }),
            field_ty,
            span,
        )
    }

    fn check_duplicate_builtin(&mut self, f: &mut FnCx, written_generics: &[ast::GenericArg], args: &[Arg], span: Span) -> Id<Expr> {
        if !written_generics.is_empty() || args.len() != 1 || args[0].name.is_some() {
            self.error(span, "`@__duplicate` takes 1 argument");
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        }
        let obj = self.expr(f, args[0].value, None);
        let ty = self.solved(f, f.body.exprs[obj].ty);
        if ty == TyId::ERROR {
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        }
        match self.program.types.kind(ty).clone() {
            TyKind::Tuple(members) => {
                let tmp_sym = self.interner.intern("_dup");
                let tmp = f.body.local(crate::tree::Local { name: tmp_sym, ty, is_var: false, kind: crate::tree::LocalKind::Pattern, span });
                let bind_pat = f.body.pattern(crate::tree::PatternKind::Bind(tmp), ty, span);
                let elems = members
                    .iter()
                    .enumerate()
                    .map(|(i, &mty)| {
                        let ref_e = f.body.expr(ExprKind::Local(tmp), ty, span);
                        f.body.expr(ExprKind::TupleField { object: ref_e, index: i as u32 }, mty, span)
                    })
                    .collect();
                let tup = f.body.expr(ExprKind::Tuple(elems), ty, span);
                f.body.expr(
                    ExprKind::Match(crate::tree::Match {
                        subject: obj,
                        arms: vec![crate::tree::Arm { pattern: bind_pat, guard: None, body: crate::tree::ArmBody::Value(tup), span }],
                    }),
                    ty,
                    span,
                )
            }
            TyKind::Adt { def, args: type_args } => {
                if let Some(record) = self.program.defs[def].record().map(|r| r.fields.clone()) {
                    let tmp_sym = self.interner.intern("_dup");
                    let tmp = f.body.local(crate::tree::Local { name: tmp_sym, ty, is_var: false, kind: crate::tree::LocalKind::Pattern, span });
                    let bind_pat = f.body.pattern(crate::tree::PatternKind::Bind(tmp), ty, span);
                    let fields = record
                        .iter()
                        .enumerate()
                        .map(|(i, field)| {
                            let fty = self.program.types.subst_generic(field.ty, &type_args);
                            let ref_e = f.body.expr(ExprKind::Local(tmp), ty, span);
                            let val = f.body.expr(ExprKind::Field { object: ref_e, index: i as u32 }, fty, span);
                            FieldValue { index: i as u32, value: val }
                        })
                        .collect();
                    let rec = f.body.expr(ExprKind::Record { fields, base: None }, ty, span);
                    return f.body.expr(
                        ExprKind::Match(crate::tree::Match {
                            subject: obj,
                            arms: vec![crate::tree::Arm { pattern: bind_pat, guard: None, body: crate::tree::ArmBody::Value(rec), span }],
                        }),
                        ty,
                        span,
                    );
                }
                if let Some(en) = self.program.defs[def].enum_def().cloned() {
                    let mut arms = Vec::with_capacity(en.variants.len());
                    for (v_idx, _) in en.variants.iter().enumerate() {
                        let payload = self.enum_variant_payload(ty, v_idx);
                        let mut field_pats = Vec::with_capacity(payload.len());
                        let mut field_vals = Vec::with_capacity(payload.len());
                        for (i, (_, pty)) in payload.into_iter().enumerate() {
                            let nm = self.interner.intern(&format!("_v{i}"));
                            let loc = f.body.local(crate::tree::Local { name: nm, ty: pty, is_var: false, kind: crate::tree::LocalKind::Pattern, span });
                            let pat = f.body.pattern(crate::tree::PatternKind::Bind(loc), pty, span);
                            let val = f.body.expr(ExprKind::Local(loc), pty, span);
                            field_pats.push(crate::tree::FieldPattern { index: i as u32, pattern: pat });
                            field_vals.push(FieldValue { index: i as u32, value: val });
                        }
                        let arm_pat = f.body.pattern(crate::tree::PatternKind::Variant { variant: v_idx as u32, fields: field_pats }, ty, span);
                        let arm_val = f.body.expr(ExprKind::Variant { variant: v_idx as u32, fields: field_vals }, ty, span);
                        arms.push(crate::tree::Arm { pattern: arm_pat, guard: None, body: crate::tree::ArmBody::Value(arm_val), span });
                    }
                    return f.body.expr(ExprKind::Match(crate::tree::Match { subject: obj, arms }), ty, span);
                }
                obj
            }
            _ => obj,
        }
    }

    fn resolve_record_or_tuple_field(&mut self, ty: TyId, key: &ConstVal) -> Option<(u32, TyId)> {
        if let TyKind::Tuple(members) = self.program.types.kind(ty).clone() {
            let idx = match key {
                ConstVal::Int(n, _) if *n >= 0 => *n as usize,
                ConstVal::String(s) => self.text(*s).parse::<usize>().ok()?,
                _ => return None,
            };
            let &fty = members.get(idx)?;
            return Some((idx as u32, fty));
        }
        let TyKind::Adt { def, args } = self.program.types.kind(ty).clone() else { return None };
        let fields = self.program.defs[def].record()?.fields.clone();
        let idx = match key {
            ConstVal::String(sym) => fields.iter().position(|fl| fl.name == *sym)?,
            ConstVal::Int(n, _) if *n >= 0 => (*n as usize).min(fields.len()),
            _ => return None,
        };
        let field = fields.get(idx)?;
        Some((idx as u32, self.program.types.subst_generic(field.ty, &args)))
    }

    fn resolve_variant_field(&mut self, enum_ty: TyId, v_name: Symbol, key: &ConstVal) -> Option<(usize, usize, TyId)> {
        let TyKind::Adt { def, .. } = self.program.types.kind(enum_ty).clone() else { return None };
        let v_idx = self.program.defs[def].enum_def()?.variants.iter().position(|v| v.name == v_name)?;
        let payload = self.enum_variant_payload(enum_ty, v_idx);
        let f_idx = match key {
            ConstVal::Int(n, _) if *n >= 0 => *n as usize,
            ConstVal::String(sym) => {
                let text = self.text(*sym);
                payload.iter().enumerate().position(|(i, (nm, _))| match nm {
                    Some(s) => *s == *sym,
                    None => text == i.to_string(),
                })
                .or_else(|| text.parse::<usize>().ok())?
            }
            _ => return None,
        };
        let &(_, fty) = payload.get(f_idx)?;
        Some((v_idx, f_idx, fty))
    }

    fn check_with_variant_field_builtin(&mut self, f: &mut FnCx, written_generics: &[ast::GenericArg], args: &[Arg], span: Span) -> Id<Expr> {
        if !written_generics.is_empty() || args.len() != 4 || args.iter().any(|a| a.name.is_some()) {
            self.error(span, "`@__with_variant_field` takes `(value, variant_name, field_name, new_value)`");
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        }
        let obj = self.expr(f, args[0].value, None);
        let obj_ty = self.solved(f, f.body.exprs[obj].ty);
        let v_expr = self.expr(f, args[1].value, Some(TyId::STRING));
        let v_expr = self.coerce(f, v_expr, TyId::STRING, self.ast(f.module)[args[1].value].span);
        let f_expr = self.expr(f, args[2].value, None);
        let (Ok(ConstVal::String(v_name)), Ok(f_val)) = (
            comptime::eval_const_expr(self, f, v_expr),
            comptime::eval_const_expr(self, f, f_expr),
        ) else {
            self.error(span, "`@__with_variant_field` requires constant variant and field names");
            let _ = self.expr(f, args[3].value, None);
            return f.body.expr(ExprKind::Error, obj_ty, span);
        };
        let Some((v_idx, target_fidx, field_ty)) = self.resolve_variant_field(obj_ty, v_name, &f_val) else {
            let vn = self.text(v_name);
            self.error(span, format!("{} has no matching variant field on `{vn}`", self.show_ty(obj_ty)));
            let _ = self.expr(f, args[3].value, None);
            return f.body.expr(ExprKind::Error, obj_ty, span);
        };
        let new_val = self.expr(f, args[3].value, Some(field_ty));
        let new_val = self.coerce(f, new_val, field_ty, self.ast(f.module)[args[3].value].span);
        let payload = self.enum_variant_payload(obj_ty, v_idx);
        let mut field_pats = Vec::with_capacity(payload.len());
        let mut field_vals = Vec::with_capacity(payload.len());
        for (i, (_, pty)) in payload.into_iter().enumerate() {
            if i == target_fidx {
                field_pats.push(crate::tree::FieldPattern {
                    index: i as u32,
                    pattern: f.body.pattern(crate::tree::PatternKind::Wildcard, pty, span),
                });
                field_vals.push(FieldValue { index: i as u32, value: new_val });
            } else {
                let sym = self.interner.intern(&format!("_vf{i}"));
                let loc = f.body.local(crate::tree::Local {
                    name: sym,
                    ty: pty,
                    is_var: false,
                    kind: crate::tree::LocalKind::Pattern,
                    span,
                });
                field_pats.push(crate::tree::FieldPattern {
                    index: i as u32,
                    pattern: f.body.pattern(crate::tree::PatternKind::Bind(loc), pty, span),
                });
                let ref_e = f.body.expr(ExprKind::Local(loc), pty, span);
                field_vals.push(FieldValue { index: i as u32, value: ref_e });
            }
        }
        let match_pat = f.body.pattern(crate::tree::PatternKind::Variant { variant: v_idx as u32, fields: field_pats }, obj_ty, span);
        let hit_expr = f.body.expr(ExprKind::Variant { variant: v_idx as u32, fields: field_vals }, obj_ty, span);
        let wild_pat = f.body.pattern(crate::tree::PatternKind::Wildcard, obj_ty, span);
        let fault_sym = self.interner.intern("fault");
        let fault_msg = self.interner.intern("variant field mismatch");
        let msg_expr = f.body.expr(ExprKind::String(fault_msg), TyId::STRING, span);
        let miss_expr = f.body.expr(
            ExprKind::Call {
                callee: Callee::Builtin { name: fault_sym, generic_args: vec![GenericArg::Type(obj_ty)] },
                args: vec![msg_expr],
            },
            obj_ty,
            span,
        );
        f.body.expr(
            ExprKind::Match(crate::tree::Match {
                subject: obj,
                arms: vec![
                    crate::tree::Arm { pattern: match_pat, guard: None, body: crate::tree::ArmBody::Value(hit_expr), span },
                    crate::tree::Arm { pattern: wild_pat, guard: None, body: crate::tree::ArmBody::Value(miss_expr), span },
                ],
            }),
            obj_ty,
            span,
        )
    }

    fn check_with_field_builtin(&mut self, f: &mut FnCx, written_generics: &[ast::GenericArg], args: &[Arg], span: Span) -> Id<Expr> {
        if !written_generics.is_empty() || args.len() != 3 || args.iter().any(|a| a.name.is_some()) {
            self.error(span, "`@__with_field` takes `(value, field_name_or_index, new_value)`");
            return f.body.expr(ExprKind::Error, TyId::ERROR, span);
        }
        let obj = self.expr(f, args[0].value, None);
        let obj_ty = self.solved(f, f.body.exprs[obj].ty);
        let f_expr = self.expr(f, args[1].value, None);
        let Ok(f_val) = comptime::eval_const_expr(self, f, f_expr) else {
            self.error(span, "`@__with_field` requires a constant field name or index");
            let _ = self.expr(f, args[2].value, None);
            return f.body.expr(ExprKind::Error, obj_ty, span);
        };
        let Some((target_idx, field_ty)) = self.resolve_record_or_tuple_field(obj_ty, &f_val) else {
            self.error(span, format!("type {} has no matching field", self.show_ty(obj_ty)));
            let _ = self.expr(f, args[2].value, None);
            return f.body.expr(ExprKind::Error, obj_ty, span);
        };
        let new_val = self.expr(f, args[2].value, Some(field_ty));
        let new_val = self.coerce(f, new_val, field_ty, self.ast(f.module)[args[2].value].span);
        let tmp_sym = self.interner.intern("_wf");
        let tmp = f.body.local(crate::tree::Local {
            name: tmp_sym,
            ty: obj_ty,
            is_var: false,
            kind: crate::tree::LocalKind::Pattern,
            span,
        });
        let bind_pat = f.body.pattern(crate::tree::PatternKind::Bind(tmp), obj_ty, span);
        let rebuilt = match self.program.types.kind(obj_ty).clone() {
            TyKind::Tuple(members) => {
                let elems = members
                    .into_iter()
                    .enumerate()
                    .map(|(i, mty)| {
                        if i as u32 == target_idx {
                            new_val
                        } else {
                            let ref_e = f.body.expr(ExprKind::Local(tmp), obj_ty, span);
                            f.body.expr(ExprKind::TupleField { object: ref_e, index: i as u32 }, mty, span)
                        }
                    })
                    .collect();
                f.body.expr(ExprKind::Tuple(elems), obj_ty, span)
            }
            TyKind::Adt { def, args: type_args } => {
                let count = self.program.defs[def].record().map_or(0, |r| r.fields.len());
                let mut field_vals = Vec::with_capacity(count);
                for i in 0..count {
                    let val = if i as u32 == target_idx {
                        new_val
                    } else {
                        let fty = self.program.defs[def].record().map(|r| self.program.types.subst_generic(r.fields[i].ty, &type_args)).unwrap_or(TyId::ERROR);
                        let ref_e = f.body.expr(ExprKind::Local(tmp), obj_ty, span);
                        f.body.expr(ExprKind::Field { object: ref_e, index: i as u32 }, fty, span)
                    };
                    field_vals.push(FieldValue { index: i as u32, value: val });
                }
                f.body.expr(ExprKind::Record { fields: field_vals, base: None }, obj_ty, span)
            }
            _ => return obj,
        };
        f.body.expr(
            ExprKind::Match(crate::tree::Match {
                subject: obj,
                arms: vec![crate::tree::Arm { pattern: bind_pat, guard: None, body: crate::tree::ArmBody::Value(rebuilt), span }],
            }),
            obj_ty,
            span,
        )
    }

    /// Resolves `T` either from `@builtin<T>(...)` or from `@builtin(T, ...)` when `T` is written as a value argument.
    fn resolve_builtin_type_arg(
        &mut self,
        f: &mut FnCx,
        builtin: &str,
        written_generics: &[ast::GenericArg],
        args: &[Arg],
        extra_count: usize,
        span: Span,
    ) -> TyId {
        let (ty, rest) = self.resolve_builtin_type_and_extra_args(f, builtin, written_generics, args, extra_count, span);
        if !rest.is_empty() {
            self.error(span, format!("`@{builtin}` expects {extra_count} value argument(s), found {}", rest.len() + extra_count));
        }
        ty
    }

    fn resolve_builtin_type_and_extra_args<'a>(
        &mut self,
        f: &mut FnCx,
        builtin: &str,
        written_generics: &[ast::GenericArg],
        args: &'a [Arg],
        extra_count: usize,
        span: Span,
    ) -> (TyId, &'a [Arg]) {
        if !written_generics.is_empty() {
            if written_generics.len() != 1 {
                self.error(span, format!("`@{builtin}` takes 1 type argument"));
                return (TyId::ERROR, &[]);
            }
            let scope = f.scope.clone();
            let ty = match &written_generics[0] {
                ast::GenericArg::Type(t) => self.lower_type_with(&scope, Some(f), *t),
                ast::GenericArg::Expr(e) => {
                    self.error(self.ast(f.module)[*e].span, format!("`@{builtin}` expects a type, not an expression"));
                    TyId::ERROR
                }
            };
            if args.len() != extra_count {
                self.error(span, format!("`@{builtin}` expects {extra_count} argument(s), found {}", args.len()));
            }
            return (ty, args);
        }
        if args.len() != 1 + extra_count || args[0].name.is_some() {
            self.error(span, format!("`@{builtin}` expects a type and {extra_count} argument(s)"));
            return (TyId::ERROR, &[]);
        }
        let ty = self.expr_as_type_arg(f, args[0].value);
        (ty, &args[1..])
    }

    /// Interprets an expression passed as a type argument (e.g. `@fields(User)`, `@fields(@typeof(x))`, `@size_of(List<Int>)`).
    pub(crate) fn expr_as_type_arg(&mut self, f: &mut FnCx, expr_id: Id<ast::Expr>) -> TyId {
        let node = self.ast(f.module)[expr_id].clone();
        match &node.kind {
            AstExprKind::Paren(inner) => self.expr_as_type_arg(f, *inner),
            AstExprKind::Ident(sym) => {
                if self.text(*sym) == "Self"
                    && let Some(st) = f.scope.self_ty
                {
                    return st;
                }
                if let Some((ConstVal::Type(ty), _)) = f.lookup_const(*sym) {
                    return ty;
                }
                if let Some(def) = self.program.const_vals.get(&match self.lookup(f.module, *sym) {
                    Some(crate::defs::Resolved::Def(def)) => def,
                    _ => return self.lower_named_type(&f.scope, *sym, &[], node.span),
                }) && let ConstVal::Type(ty) = def
                {
                    return *ty;
                }
                self.lower_named_type(&f.scope, *sym, &[], node.span)
            }
            AstExprKind::Member { object, name, args } => {
                // Module-qualified path or `T<...>` path
                if let Some(path) = self.expr_to_path(f.module, *object, *name) {
                    let scope = f.scope.clone();
                    let g_args = args.iter().map(|g| self.lower_generic_arg_with(&scope, Some(f), g)).collect();
                    return self.lower_path(&scope, &path, Some(g_args), node.span);
                }
                self.error(node.span, "expected a type argument");
                TyId::ERROR
            }
            AstExprKind::TypeApply { path, args } => {
                let scope = f.scope.clone();
                let g_args = args.iter().map(|g| self.lower_generic_arg_with(&scope, Some(f), g)).collect();
                self.lower_path(&scope, path, Some(g_args), node.span)
            }
            AstExprKind::Tuple(els) => {
                let members = els.iter().map(|&e| self.expr_as_type_arg(f, e)).collect();
                self.program.types.tuple(members)
            }
            AstExprKind::List(els) if els.len() == 1 => {
                let elem = self.expr_as_type_arg(f, els[0]);
                self.program.types.array(elem)
            }
            AstExprKind::ArrayRepeat { value, len } => {
                let elem = self.expr_as_type_arg(f, *value);
                let len_arg = self.lower_const_arg_with(&f.scope.clone(), Some(f), *len);
                self.program.types.fixed_array_arg(elem, len_arg)
            }
            AstExprKind::Builtin { name, generic_args, args }
                if matches!(self.text(name.name).as_str(), "typeof" | "TypeArg" | "FieldType" | "VariantFieldType") =>
            {
                let bname = self.text(name.name);
                if bname == "typeof" {
                    if !generic_args.is_empty() || args.len() != 1 || args[0].name.is_some() {
                        self.error(node.span, "`@typeof` takes 1 expression argument");
                        return TyId::ERROR;
                    }
                    let inner = self.expr(f, args[0].value, None);
                    return self.solved(f, f.body.exprs[inner].ty);
                }
                let scope = f.scope.clone();
                self.check_type_builtin(&scope, Some(f), *name, generic_args, args, node.span)
            }
            AstExprKind::Propagate(inner) => {
                let inner_ty = self.expr_as_type_arg(f, *inner);
                self.program.types.optional(inner_ty)
            }
            AstExprKind::Binary { op: ast::BinaryOp::BitOr, left, right } => {
                let lhs = self.expr_as_type_arg(f, *left);
                let rhs = self.expr_as_type_arg(f, *right);
                self.program.types.union(&[lhs, rhs])
            }
            AstExprKind::Null => TyId::UNIT,
            _ => {
                let checked = self.expr(f, expr_id, None);
                if let Ok(ConstVal::Type(ty)) = comptime::eval_const_expr(self, f, checked) {
                    return ty;
                }
                self.error(node.span, "expected a type argument");
                TyId::ERROR
            }
        }
    }

    fn lower_named_type(&mut self, scope: &Scope, name: Symbol, args: &[GenericArg], span: Span) -> TyId {
        let ident = ast::Ident { name, span };
        let path = ast::Path { segments: vec![ident], span };
        let opt_args = if args.is_empty() { None } else { Some(args.to_vec()) };
        self.lower_path(scope, &path, opt_args, span)
    }

    fn expr_to_path(&self, module: crate::defs::ModuleId, object: Id<ast::Expr>, tail: ast::Ident) -> Option<ast::Path> {
        let ast = self.ast(module);
        let mut segments = Vec::new();
        let mut cur = object;
        loop {
            match &ast[cur].kind {
                AstExprKind::Ident(sym) => {
                    segments.push(ast::Ident { name: *sym, span: ast[cur].span });
                    break;
                }
                AstExprKind::Member { object: inner, name, args } if args.is_empty() => {
                    segments.push(*name);
                    cur = *inner;
                }
                _ => return None,
            }
        }
        segments.reverse();
        segments.push(tail);
        Some(ast::Path { segments, span: tail.span })
    }

    #[allow(clippy::too_many_arguments)]
    fn check_runtime_builtin(
        &mut self,
        f: &mut FnCx,
        sym: Symbol,
        rt_name: &str,
        written: &[TyId],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Id<Expr> {
        // Determine generic parameter count, param expectations, writable mask, and return type.
        let t0 = self.fresh(f);
        let t1 = self.fresh(f);

        let error_ty = || self.program.std.error.map_or(TyId::ERROR, |d| self.program.types.adt_types(d, Vec::new()));
        let result_ty = |cx: &mut Cx<'_>, ok: TyId| -> TyId {
            match (cx.program.std.result, cx.program.std.error) {
                (Some(r), Some(e)) => {
                    let err = cx.program.types.adt_types(e, Vec::new());
                    cx.program.types.adt_types(r, vec![ok, err])
                }
                _ => TyId::ERROR,
            }
        };

        let (gen_count, param_specs, ret_ty, mutates_first): (usize, Vec<(TyId, bool)>, TyId, bool) = match rt_name {
            "print" | "println" | "eprint" | "eprintln" => (1, vec![(t0, false)], TyId::UNIT, false),
            "type_tag" => (1, vec![(t0, false)], TyId::STRING, false),
            "fault" => (1, vec![(TyId::STRING, false)], t0, false),
            "index_out_of_range" => (0, Vec::new(), TyId::UNIT, false),
            "sys_read_line" => {
                let opt = self.program.types.optional(TyId::STRING);
                (0, vec![], opt, false)
            }
            "sys_args" => {
                let list_str = self.program.types.builtin(BuiltinTy::List, vec![TyId::STRING]);
                (0, vec![], list_str, false)
            }
            "sys_exit" => (0, vec![(TyId::INT, false)], TyId::UNIT, false),
            "sys_now" => (0, vec![], TyId::INT, false),
            "port_open" => {
                let arr_u8 = self.program.types.array(TyId::UINT8);
                let port_end = self.program.types.builtin(BuiltinTy::PortEnd, Vec::new());
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![arr_u8]);
                let tup = self.program.types.tuple(vec![port_end, rx]);
                (0, vec![(t0, false)], tup, false)
            }
            "float_is_nan" | "float_is_infinite" => (1, vec![(t0, false)], TyId::BOOL, false),
            "float_to_int" => (0, vec![(TyId::FLOAT, false)], TyId::INT, false),
            "num_to_string" => (1, vec![(t0, false)], TyId::STRING, false),
            "num_wrapping_add" | "num_wrapping_sub" | "num_wrapping_mul" | "num_saturating_add" | "num_saturating_sub" | "num_saturating_mul" => {
                (1, vec![(t0, false), (t0, false)], t0, false)
            }
            "num_wrapping_neg" | "num_saturating_neg" => (1, vec![(t0, false)], t0, false),
            "num_wrapping_shl" | "num_wrapping_shr" | "num_saturating_shl" | "num_saturating_shr" => {
                (1, vec![(t0, false), (TyId::INT, false)], t0, false)
            }
            "num_checked_add" | "num_checked_sub" | "num_checked_mul" => {
                let opt = self.program.types.optional(t0);
                (1, vec![(t0, false), (t0, false)], opt, false)
            }
            "num_checked_neg" => {
                let opt = self.program.types.optional(t0);
                (1, vec![(t0, false)], opt, false)
            }
            "num_checked_shl" | "num_checked_shr" => {
                let opt = self.program.types.optional(t0);
                (1, vec![(t0, false), (TyId::INT, false)], opt, false)
            }
            "num_overflowing_add" | "num_overflowing_sub" | "num_overflowing_mul" => {
                let tup = self.program.types.tuple(vec![t0, TyId::BOOL]);
                (1, vec![(t0, false), (t0, false)], tup, false)
            }
            "num_overflowing_neg" => {
                let tup = self.program.types.tuple(vec![t0, TyId::BOOL]);
                (1, vec![(t0, false)], tup, false)
            }
            "num_overflowing_shl" | "num_overflowing_shr" => {
                let tup = self.program.types.tuple(vec![t0, TyId::BOOL]);
                (1, vec![(t0, false), (TyId::INT, false)], tup, false)
            }
            "math_sqrt" | "math_floor" | "math_ceil" | "math_round" | "math_exp" | "math_ln" | "math_log10" | "math_sin" | "math_cos" | "math_tan" => {
                (0, vec![(TyId::FLOAT, false)], TyId::FLOAT, false)
            }
            "math_pow" | "math_atan2" => (0, vec![(TyId::FLOAT, false), (TyId::FLOAT, false)], TyId::FLOAT, false),
            "parse_int" => {
                let opt = self.program.types.optional(TyId::INT);
                (0, vec![(TyId::STRING, false)], opt, false)
            }
            "parse_float" => {
                let opt = self.program.types.optional(TyId::FLOAT);
                (0, vec![(TyId::STRING, false)], opt, false)
            }
            "string_len" => (0, vec![(TyId::STRING, false)], TyId::INT, false),
            "string_concat" | "string_split" => {
                let ret = if rt_name == "string_concat" {
                    TyId::STRING
                } else {
                    self.program.types.builtin(BuiltinTy::List, vec![TyId::STRING])
                };
                (0, vec![(TyId::STRING, false), (TyId::STRING, false)], ret, false)
            }
            "string_replace" => (0, vec![(TyId::STRING, false), (TyId::STRING, false), (TyId::STRING, false)], TyId::STRING, false),
            "string_slice" => (0, vec![(TyId::STRING, false), (TyId::INT, false), (TyId::INT, false)], TyId::STRING, false),
            "string_byte_at" => (0, vec![(TyId::STRING, false), (TyId::INT, false)], TyId::UINT8, false),
            "string_is_boundary" => (0, vec![(TyId::STRING, false), (TyId::INT, false)], TyId::BOOL, false),
            "string_to_upper" | "string_to_lower" | "string_trim" => (0, vec![(TyId::STRING, false)], TyId::STRING, false),
            "string_repeat" => (0, vec![(TyId::STRING, false), (TyId::INT, false)], TyId::STRING, false),
            "string_find" | "string_rfind" => {
                let opt = self.program.types.optional(TyId::INT);
                (0, vec![(TyId::STRING, false), (TyId::STRING, false)], opt, false)
            }
            "string_bytes" => {
                let arr = self.program.types.array(TyId::UINT8);
                (0, vec![(TyId::STRING, false)], arr, false)
            }
            "string_from_bytes" => {
                let arr = self.program.types.array(TyId::UINT8);
                let opt = self.program.types.optional(TyId::STRING);
                (0, vec![(arr, false)], opt, false)
            }
            "list_new" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![], list, false)
            }
            "list_len" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, false)], TyId::INT, false)
            }
            "list_push" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, true), (t0, false)], TyId::UNIT, true)
            }
            "list_pop" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                let opt = self.program.types.optional(t0);
                (1, vec![(list, true)], opt, true)
            }
            "list_get" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, false), (TyId::INT, false)], t0, false)
            }
            "list_set" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, true), (TyId::INT, false), (t0, false)], TyId::UNIT, true)
            }
            "list_clear" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, true)], TyId::UNIT, true)
            }
            "list_contains" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, false), (t0, false)], TyId::BOOL, false)
            }
            "list_slice" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, false), (TyId::INT, false), (TyId::INT, false)], list, false)
            }
            "list_set_slice" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, true), (TyId::INT, false), (TyId::INT, false), (list, false)], TyId::UNIT, true)
            }
            "list_clone" => {
                let list = self.program.types.builtin(BuiltinTy::List, vec![t0]);
                (1, vec![(list, false)], list, false)
            }
            "deep_clone" => (1, vec![(t0, false)], t0, false),
            "hash_of" => (1, vec![(t0, false)], TyId::INT, false),
            "key_eq" | "equal_of" => (1, vec![(t0, false), (t0, false)], TyId::BOOL, false),
            "stream_next" => {
                let stream = self.program.types.builtin(BuiltinTy::Stream, vec![t0]);
                let opt = self.program.types.optional(t0);
                (1, vec![(stream, true)], opt, true)
            }
            "task_await" => {
                let task = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                let res = result_ty(self, t0);
                (1, vec![(task, false)], res, false)
            }
            "task_await_timeout" => {
                let task = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                let res = result_ty(self, t0);
                let opt = self.program.types.optional(res);
                (1, vec![(task, false), (TyId::INT, false)], opt, false)
            }
            "task_cancel" => {
                let task = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                (1, vec![(task, false)], TyId::UNIT, false)
            }
            "task_is_ready" => {
                let task = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                (1, vec![(task, false)], TyId::BOOL, false)
            }
            "spawn_call" => {
                let work = self.program.types.function(vec![], t0, true);
                let task = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                (1, vec![(work, false)], task, false)
            }
            "task_view" => {
                let task_a = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                let conv = self.program.types.function(vec![], t1, false);
                let task_b = self.program.types.builtin(BuiltinTy::Task, vec![t1]);
                (2, vec![(task_a, false), (conv, false)], task_b, false)
            }
            "task_value" => {
                let task = self.program.types.builtin(BuiltinTy::Task, vec![t0]);
                (1, vec![(task, false)], t0, false)
            }
            "sleep" => (0, vec![(TyId::INT, false)], TyId::UNIT, false),
            "channel_new" => {
                let tx = self.program.types.builtin(BuiltinTy::Sender, vec![t0]);
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                let tup = self.program.types.tuple(vec![tx, rx]);
                (1, vec![(TyId::INT, false)], tup, false)
            }
            "sender_send" => {
                let tx = self.program.types.builtin(BuiltinTy::Sender, vec![t0]);
                (1, vec![(tx, false), (t0, false)], TyId::UNIT, false)
            }
            "sender_try_send" => {
                let tx = self.program.types.builtin(BuiltinTy::Sender, vec![t0]);
                (1, vec![(tx, false), (t0, false)], TyId::BOOL, false)
            }
            "sender_close" => {
                let tx = self.program.types.builtin(BuiltinTy::Sender, vec![t0]);
                (1, vec![(tx, false)], TyId::UNIT, false)
            }
            "sender_clone" => {
                let tx = self.program.types.builtin(BuiltinTy::Sender, vec![t0]);
                (1, vec![(tx, false)], tx, false)
            }
            "receiver_recv" => {
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                let opt = self.program.types.optional(t0);
                (1, vec![(rx, false)], opt, false)
            }
            "receiver_recv_timeout" => {
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                let opt = self.program.types.optional(t0);
                (1, vec![(rx, false), (TyId::INT, false)], opt, false)
            }
            "receiver_try_recv" => {
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                let opt = self.program.types.optional(t0);
                (1, vec![(rx, false)], opt, false)
            }
            "receiver_is_closed" => {
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                (1, vec![(rx, false)], TyId::BOOL, false)
            }
            "receiver_close" => {
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                (1, vec![(rx, false)], TyId::UNIT, false)
            }
            "receiver_clone" => {
                let rx = self.program.types.builtin(BuiltinTy::Receiver, vec![t0]);
                (1, vec![(rx, false)], rx, false)
            }
            "mutex_new" => {
                let mtx = self.program.types.builtin(BuiltinTy::Mutex, vec![t0]);
                (1, vec![(t0, false)], mtx, false)
            }
            "mutex_take" => {
                let mtx = self.program.types.builtin(BuiltinTy::Mutex, vec![t0]);
                (1, vec![(mtx, false)], t0, false)
            }
            "mutex_put" => {
                let mtx = self.program.types.builtin(BuiltinTy::Mutex, vec![t0]);
                (1, vec![(mtx, false), (t0, false)], TyId::UNIT, false)
            }
            "once_new" => {
                let once = self.program.types.builtin(BuiltinTy::Once, vec![t0]);
                (1, vec![], once, false)
            }
            "once_begin" => {
                let once = self.program.types.builtin(BuiltinTy::Once, vec![t0]);
                (1, vec![(once, false)], TyId::BOOL, false)
            }
            "once_finish" => {
                let once = self.program.types.builtin(BuiltinTy::Once, vec![t0]);
                (1, vec![(once, false), (t0, false)], TyId::UNIT, false)
            }
            "once_value" => {
                let once = self.program.types.builtin(BuiltinTy::Once, vec![t0]);
                (1, vec![(once, false)], t0, false)
            }
            "atomic_int_new" => {
                let ai = self.program.types.builtin(BuiltinTy::AtomicInt, Vec::new());
                (0, vec![(TyId::INT, false)], ai, false)
            }
            "atomic_int_load" => {
                let ai = self.program.types.builtin(BuiltinTy::AtomicInt, Vec::new());
                (0, vec![(ai, false)], TyId::INT, false)
            }
            "atomic_int_store" => {
                let ai = self.program.types.builtin(BuiltinTy::AtomicInt, Vec::new());
                (0, vec![(ai, false), (TyId::INT, false)], TyId::UNIT, false)
            }
            "atomic_int_swap" | "atomic_int_fetch_add" | "atomic_int_fetch_sub" => {
                let ai = self.program.types.builtin(BuiltinTy::AtomicInt, Vec::new());
                (0, vec![(ai, false), (TyId::INT, false)], TyId::INT, false)
            }
            "atomic_int_compare_exchange" => {
                let ai = self.program.types.builtin(BuiltinTy::AtomicInt, Vec::new());
                (0, vec![(ai, false), (TyId::INT, false), (TyId::INT, false)], TyId::BOOL, false)
            }
            "atomic_bool_new" => {
                let ab = self.program.types.builtin(BuiltinTy::AtomicBool, Vec::new());
                (0, vec![(TyId::BOOL, false)], ab, false)
            }
            "atomic_bool_load" => {
                let ab = self.program.types.builtin(BuiltinTy::AtomicBool, Vec::new());
                (0, vec![(ab, false)], TyId::BOOL, false)
            }
            "atomic_bool_store" => {
                let ab = self.program.types.builtin(BuiltinTy::AtomicBool, Vec::new());
                (0, vec![(ab, false), (TyId::BOOL, false)], TyId::UNIT, false)
            }
            "atomic_bool_swap" => {
                let ab = self.program.types.builtin(BuiltinTy::AtomicBool, Vec::new());
                (0, vec![(ab, false), (TyId::BOOL, false)], TyId::BOOL, false)
            }
            "atomic_bool_compare_exchange" => {
                let ab = self.program.types.builtin(BuiltinTy::AtomicBool, Vec::new());
                (0, vec![(ab, false), (TyId::BOOL, false), (TyId::BOOL, false)], TyId::BOOL, false)
            }
            "ptr_null" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![], ptr, false)
            }
            "ptr_from_addr" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(TyId::UINT64, false)], ptr, false)
            }
            "ptr_alloc" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(TyId::INT, false)], ptr, false)
            }
            "ptr_realloc" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false), (TyId::INT, false)], ptr, false)
            }
            "ptr_free" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false)], TyId::UNIT, false)
            }
            "ptr_offset" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false), (TyId::INT, false)], ptr, false)
            }
            "ptr_load" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false), (TyId::INT, false)], t0, false)
            }
            "ptr_store" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false), (TyId::INT, false), (t0, false)], TyId::UNIT, false)
            }
            "ptr_copy" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false), (ptr, false), (TyId::INT, false)], TyId::UNIT, false)
            }
            "ptr_zero" => {
                let ptr = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                (1, vec![(ptr, false), (TyId::INT, false)], TyId::UNIT, false)
            }
            "ptr_cast" => {
                let src = self.program.types.builtin(BuiltinTy::Pointer, vec![t0]);
                let dst = self.program.types.builtin(BuiltinTy::Pointer, vec![t1]);
                (2, vec![(src, false)], dst, false)
            }
            _ => {
                let _ = error_ty;
                self.error(span, format!("unknown runtime builtin `@{rt_name}`"));
                return f.body.expr(ExprKind::Error, TyId::ERROR, span);
            }
        };

        if !written.is_empty() {
            if written.len() != gen_count {
                self.error(span, format!("`@{}` takes {gen_count} type argument(s)", self.text(sym)));
            } else {
                let vars = [t0, t1];
                for (&w, &v) in written.iter().zip(vars.iter()) {
                    self.unify(f, v, w);
                }
            }
        }
        if args.len() != param_specs.len() {
            self.error(span, format!("`@{}` expects {} argument(s), found {}", self.text(sym), param_specs.len(), args.len()));
            for arg in args {
                let _ = self.expr(f, arg.value, None);
            }
            return f.body.expr(ExprKind::Error, self.solved(f, ret_ty), span);
        }

        if let Some(exp) = expected {
            let cur_ret = self.solved(f, ret_ty);
            if self.program.types.has_infer(cur_ret) {
                self.unify(f, cur_ret, exp);
            }
        }

        let mut checked_args = Vec::with_capacity(args.len());
        for (idx, (arg, (param_ty, is_var))) in args.iter().zip(param_specs.iter()).enumerate() {
            if arg.name.is_some() {
                self.error(span, format!("`@{}` does not take named arguments", self.text(sym)));
            }
            let arg_span = self.ast(f.module)[arg.value].span;
            let known = self.solved(f, *param_ty);
            let lambda = matches!(self.ast(f.module)[arg.value].kind, AstExprKind::Lambda { .. });
            let node = self.expr(f, arg.value, (lambda || !self.program.types.has_infer(known)).then_some(known));
            if (*is_var || (idx == 0 && mutates_first)) && Self::is_place(f, node) && !self.writable(f, node) {
                self.error(arg_span, "this place has no `var` path to write through");
            }
            if self.program.types.has_infer(known) {
                let found = f.body.exprs[node].ty;
                self.unify(f, known, found);
            }
            let known_after = self.solved(f, *param_ty);
            let coerced = if self.program.types.has_infer(known_after) {
                node
            } else {
                self.coerce(f, node, known_after, arg_span)
            };
            checked_args.push(coerced);
        }

        // Domain validation for numeric / key / sendable builtins
        let t0_solved = self.solved(f, t0);
        match rt_name {
            "num_wrapping_add" | "num_wrapping_sub" | "num_wrapping_mul" | "num_wrapping_neg" | "num_wrapping_shl" | "num_wrapping_shr"
            | "num_checked_add" | "num_checked_sub" | "num_checked_mul" | "num_checked_neg" | "num_checked_shl" | "num_checked_shr"
            | "num_saturating_add" | "num_saturating_sub" | "num_saturating_mul" | "num_saturating_neg" | "num_saturating_shl" | "num_saturating_shr"
            | "num_overflowing_add" | "num_overflowing_sub" | "num_overflowing_mul" | "num_overflowing_neg" | "num_overflowing_shl" | "num_overflowing_shr" => {
                if !matches!(self.program.types.kind(t0_solved), TyKind::Error | TyKind::Int(_) | TyKind::Param { .. }) {
                    self.error(span, format!("`@{}` expects an integer type, found {}", self.text(sym), self.show_ty(t0_solved)));
                }
            }
            "float_is_nan" | "float_is_infinite" => {
                if !matches!(self.program.types.kind(t0_solved), TyKind::Error | TyKind::Float(_) | TyKind::Param { .. }) {
                    self.error(span, format!("`@{}` expects a Float or Float32, found {}", self.text(sym), self.show_ty(t0_solved)));
                }
            }
            "num_to_string" => {
                if !self.is_number(t0_solved) && !matches!(self.program.types.kind(t0_solved), TyKind::Error | TyKind::Param { .. }) {
                    self.error(span, format!("`@{}` expects a number, found {}", self.text(sym), self.show_ty(t0_solved)));
                }
            }
            "hash_of" | "key_eq" => {
                self.check_key_type(f, t0_solved, span);
            }
            "spawn_call" | "channel_new" | "mutex_new" | "once_new" => {
                if let Some(reason) = self.not_sendable(t0_solved) {
                    self.error(span, format!("`{}` cannot cross tasks: {reason}", self.show_ty(t0_solved)));
                }
            }
            _ => {}
        }

        let mut final_ret = self.solved(f, ret_ty);
        if self.program.types.has_infer(final_ret) {
            if rt_name == "fault" {
                final_ret = expected.unwrap_or(TyId::UNIT);
            } else {
                self.error(span, format!("cannot infer type for `@{}`", self.text(sym)));
                final_ret = TyId::ERROR;
            }
        }
        let gen_args = [t0, t1]
            .into_iter()
            .take(gen_count)
            .map(|v| GenericArg::Type(self.solved(f, v)))
            .collect();
        f.body.expr(
            ExprKind::Call {
                callee: Callee::Builtin { name: sym, generic_args: gen_args },
                args: checked_args,
            },
            final_ret,
            span,
        )
    }

    /// Checks a direct call to a `std.array` wrapper function using dependent array shape inference.
    pub(crate) fn check_std_array_call(
        &mut self,
        f: &mut FnCx,
        def: crate::defs::DefId,
        written: &[GenericArg],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Option<Id<Expr>> {
        let name_str = self.text(self.program.defs[def].name);
        let rt_name = if matches!(name_str.as_str(), "all" | "any" | "none") {
            format!("mask_{name_str}")
        } else {
            format!("array_{name_str}")
        };
        let written_tys: Vec<TyId> = written.iter().filter_map(GenericArg::as_type).collect();
        self.check_array_or_mask_inner(f, Some(def), None, &name_str, &rt_name, &written_tys, args, expected, span)
    }

    #[allow(clippy::too_many_arguments)]
    fn check_array_or_mask_builtin(
        &mut self,
        f: &mut FnCx,
        sym: Symbol,
        rt_name: &str,
        written: &[TyId],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Option<Id<Expr>> {
        let label = format!("@{}", self.text(sym));
        self.check_array_or_mask_inner(f, None, Some(sym), &label, rt_name, written, args, expected, span)
    }

    #[allow(clippy::too_many_arguments)]
    fn check_array_or_mask_inner(
        &mut self,
        f: &mut FnCx,
        std_def: Option<crate::defs::DefId>,
        builtin_sym: Option<Symbol>,
        label: &str,
        rt_name: &str,
        written: &[TyId],
        args: &[Arg],
        expected: Option<TyId>,
        span: Span,
    ) -> Option<Id<Expr>> {
        if !rt_name.starts_with("array_") && !rt_name.starts_with("mask_") {
            return None;
        }
        let a = self.fresh(f);
        let e = self.fresh(f);
        let b = self.fresh(f);
        let e2 = self.fresh(f);
        let u = self.fresh(f);
        let out = self.fresh(f);

        let pred = self.program.types.function(vec![crate::types::FnParam { ty: e, is_var: false }], TyId::BOOL, false);
        let op2 = self.program.types.function(
            vec![crate::types::FnParam { ty: e, is_var: false }, crate::types::FnParam { ty: e, is_var: false }],
            e,
            false,
        );
        let cmp2 = self.program.types.function(
            vec![crate::types::FnParam { ty: e, is_var: false }, crate::types::FnParam { ty: e, is_var: false }],
            TyId::INT,
            false,
        );
        let map_fn = self.program.types.function(vec![crate::types::FnParam { ty: e, is_var: false }], u, false);
        let fold_fn = self.program.types.function(
            vec![crate::types::FnParam { ty: u, is_var: false }, crate::types::FnParam { ty: e, is_var: false }],
            u,
            false,
        );
        let zip_fn = self.program.types.function(
            vec![crate::types::FnParam { ty: e, is_var: false }, crate::types::FnParam { ty: e2, is_var: false }],
            u,
            false,
        );

        let (gen_count, param_specs, ret_ty): (usize, Vec<(TyId, bool)>, TyId) = match rt_name {
            "array_len" => (1, vec![(a, false)], TyId::INT),
            "array_get" => (2, vec![(a, false), (TyId::INT, false)], e),
            "array_set" => (2, vec![(a, true), (TyId::INT, false), (e, false)], TyId::UNIT),
            "array_contains" => (2, vec![(a, false), (e, false)], TyId::BOOL),
            "array_slice" => (2, vec![(a, false), (TyId::INT, false), (TyId::INT, false)], self.program.types.array(e)),
            "array_clone" => (2, vec![(a, false)], self.program.types.array(e)),
            "mask_all" | "mask_any" | "mask_none" => (1, vec![(a, false)], TyId::BOOL),
            "array_all_by" | "array_any_by" | "array_none_by" => (2, vec![(a, false), (pred, false)], TyId::BOOL),
            "array_count" => (2, vec![(a, false), (e, false)], TyId::INT),
            "array_count_by" => (2, vec![(a, false), (pred, false)], TyId::INT),
            "array_sum" | "array_product" | "array_norm_sq" => (2, vec![(a, false)], e),
            "array_mean" => (2, vec![(a, false)], TyId::FLOAT),
            "array_min" | "array_max" => (2, vec![(a, false)], self.program.types.optional(e)),
            "array_argmin" | "array_argmax" => (2, vec![(a, false)], self.program.types.optional(TyId::INT)),
            "array_dot" => (2, vec![(a, false), (a, false)], e),
            "array_fold" => (3, vec![(a, false), (u, false), (fold_fn, false)], u),
            "array_reduce" => (2, vec![(a, false), (op2, false)], self.program.types.optional(e)),
            "array_scan" => (4, vec![(a, false), (u, false), (fold_fn, false)], out),
            "array_find" => (2, vec![(a, false), (pred, false)], self.program.types.optional(e)),
            "array_find_index" => (2, vec![(a, false), (pred, false)], self.program.types.optional(TyId::INT)),
            "array_index_of" => (2, vec![(a, false), (e, false)], self.program.types.optional(TyId::INT)),
            "array_select" => (2, vec![(b, false), (a, false), (a, false)], a),
            "array_map" => (4, vec![(a, false), (map_fn, false)], out),
            "array_zip_with" => (6, vec![(a, false), (b, false), (zip_fn, false)], out),
            "array_reverse" | "array_sort" => (1, vec![(a, false)], a),
            "array_clamp" => (2, vec![(a, false), (e, false), (e, false)], a),
            "array_filter" => (2, vec![(a, false), (pred, false)], self.program.types.array(e)),
            "array_zip" => (5, vec![(a, false), (b, false)], out),
            "array_enumerate" => (3, vec![(a, false)], out),
            "array_concat" => (3, vec![(a, false), (b, false)], self.program.types.array(e)),
            "array_take" | "array_drop" => (2, vec![(a, false), (TyId::INT, false)], self.program.types.array(e)),
            "array_flatten" => (2, vec![(a, false)], self.program.types.array(e)),
            "array_sort_by" => (2, vec![(a, false), (cmp2, false)], a),
            _ => return None,
        };

        let all_vars = match rt_name {
            "array_len" | "mask_all" | "mask_any" | "mask_none" | "array_reverse" | "array_sort" => vec![a],
            "array_get" | "array_set" | "array_contains" | "array_slice" | "array_clone"
            | "array_all_by" | "array_any_by" | "array_none_by" | "array_count" | "array_count_by"
            | "array_sum" | "array_product" | "array_mean" | "array_min" | "array_max"
            | "array_argmin" | "array_argmax" | "array_dot" | "array_norm_sq" | "array_reduce"
            | "array_find" | "array_find_index" | "array_index_of" | "array_clamp"
            | "array_filter" | "array_take" | "array_drop" | "array_flatten" | "array_sort_by" => vec![a, e],
            "array_select" => vec![b, a],
            "array_fold" => vec![a, e, u],
            "array_scan" | "array_map" => vec![a, e, u, out],
            "array_enumerate" => vec![a, e, out],
            "array_concat" => vec![a, b, e],
            "array_zip" => vec![a, e, b, e2, out],
            "array_zip_with" => vec![a, e, b, e2, u, out],
            _ => unreachable!(),
        };

        if !written.is_empty() {
            if written.len() == gen_count {
                for (&w, &v) in written.iter().zip(all_vars.iter()) {
                    self.unify(f, v, w);
                }
            } else {
                self.error(span, format!("`{label}` takes {gen_count} type argument(s)"));
            }
        }
        if args.len() != param_specs.len() {
            self.error(span, format!("`{label}` expects {} argument(s), found {}", param_specs.len(), args.len()));
            for arg in args {
                let _ = self.expr(f, arg.value, None);
            }
            return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
        }

        let mut checked_args = Vec::with_capacity(args.len());
        for (idx, (arg, (param_ty, is_var))) in args.iter().zip(param_specs.iter()).enumerate() {
            let arg_span = self.ast(f.module)[arg.value].span;
            let known = self.solved(f, *param_ty);
            let lambda = matches!(self.ast(f.module)[arg.value].kind, AstExprKind::Lambda { .. });
            let node = self.expr(f, arg.value, (lambda || !self.program.types.has_infer(known)).then_some(known));
            if *is_var && Self::is_place(f, node) && !self.writable(f, node) {
                self.error(arg_span, "this place has no `var` path to write through");
            }
            if self.program.types.has_infer(known) {
                let found = f.body.exprs[node].ty;
                self.unify(f, known, found);
            }
            let known_after = self.solved(f, *param_ty);
            let coerced = if self.program.types.has_infer(known_after) {
                node
            } else {
                self.coerce(f, node, known_after, arg_span)
            };
            checked_args.push(coerced);
            // Propagate element/shape constraints from arg 0 (& arg 1) right away so trailing lambda params get typed.
            if idx == 0 {
                let a0 = self.solved(f, a);
                if let Some((elem, _)) = self.array_shape(a0) {
                    if rt_name == "array_flatten" {
                        if let Some((inner_e, _)) = self.array_shape(elem) {
                            self.unify(f, e, inner_e);
                        }
                    } else if rt_name != "array_select" {
                        self.unify(f, e, elem);
                    }
                }
            } else if idx == 1 && matches!(rt_name, "array_zip" | "array_zip_with") {
                let b0 = self.solved(f, b);
                if let Some((elem2, _)) = self.array_shape(b0) {
                    self.unify(f, e2, elem2);
                }
            } else if idx == 1 && rt_name == "array_fold" {
                let u_solved = self.solved(f, u);
                self.unify(f, u, u_solved);
            }
        }

        // Now validate array shapes and domain constraints!
        let mut had_error = false;
        let a_ty = self.solved(f, a);
        let a_is_param = matches!(self.program.types.kind(a_ty), TyKind::Param { .. });
        if rt_name != "array_select" && a_ty != TyId::ERROR && !a_is_param {
            let Some((elem, len)) = self.array_shape(a_ty) else {
                self.error(span, format!("`{label}` expects an array `[T]` or `[T; N]`, found {}", self.show_ty(a_ty)));
                return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
            };
            let elem_is_param = matches!(self.program.types.kind(elem), TyKind::Param { .. });
            match rt_name {
                "mask_all" | "mask_any" | "mask_none" => {
                    if elem != TyId::BOOL && elem != TyId::ERROR && !elem_is_param {
                        self.error(span, format!("`{label}` expects a `[Bool]` or `[Bool; N]` mask, found {}", self.show_ty(a_ty)));
                        had_error = true;
                    }
                }
                "array_sum" | "array_product" | "array_mean" | "array_dot" | "array_norm_sq" | "array_clamp" => {
                    if elem != TyId::ERROR && !elem_is_param && !self.is_number(elem) {
                        self.error(span, format!("`{label}` expects an array of numbers, found {}", self.show_ty(a_ty)));
                        had_error = true;
                    }
                    self.unify(f, e, elem);
                }
                "array_min" | "array_max" | "array_argmin" | "array_argmax" | "array_sort" => {
                    if elem != TyId::ERROR && !elem_is_param && !self.is_number(elem) && elem != TyId::STRING {
                        self.error(span, format!("`{label}` expects an array of numbers or strings, found {}", self.show_ty(a_ty)));
                        had_error = true;
                    }
                    self.unify(f, e, elem);
                }
                "array_count" | "array_index_of" => {
                    if matches!(self.program.types.kind(elem), TyKind::Function(_)) {
                        self.error(span, format!("`{label}` cannot compare elements of {}", self.show_ty(a_ty)));
                        had_error = true;
                    }
                    self.unify(f, e, elem);
                }
                "array_scan" | "array_map" => {
                    self.unify(f, e, elem);
                    let out_elem = self.solved(f, u);
                    if !self.program.types.has_infer(out_elem) {
                        let out_ty = self.shape_with_element(len, out_elem);
                        self.unify(f, out, out_ty);
                    }
                }
                "array_enumerate" => {
                    self.unify(f, e, elem);
                    let pair = self.program.types.tuple(vec![TyId::INT, elem]);
                    let out_ty = self.shape_with_element(len, pair);
                    self.unify(f, out, out_ty);
                }
                "array_zip" | "array_zip_with" => {
                    self.unify(f, e, elem);
                    let b_ty = self.solved(f, b);
                    if b_ty != TyId::ERROR {
                        let Some((elem2, len2)) = self.array_shape(b_ty) else {
                            self.error(span, format!("`{label}` expects an array `[T]` or `[T; N]`, found {}", self.show_ty(b_ty)));
                            return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
                        };
                        if len != len2 {
                            self.error(span, format!("`{label}` needs matching shapes for {} and {}", self.show_ty(a_ty), self.show_ty(b_ty)));
                            had_error = true;
                        }
                        self.unify(f, e2, elem2);
                        if rt_name == "array_zip" {
                            let pair = self.program.types.tuple(vec![elem, elem2]);
                            let out_ty = self.shape_with_element(len, pair);
                            self.unify(f, out, out_ty);
                        } else {
                            let out_elem = self.solved(f, u);
                            if !self.program.types.has_infer(out_elem) {
                                let out_ty = self.shape_with_element(len, out_elem);
                                self.unify(f, out, out_ty);
                            }
                        }
                    }
                }
                "array_concat" => {
                    let b_ty = self.solved(f, b);
                    if b_ty != TyId::ERROR {
                        let Some((elem2, _)) = self.array_shape(b_ty) else {
                            self.error(span, format!("`{label}` expects an array `[T]` or `[T; N]`, found {}", self.show_ty(b_ty)));
                            return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
                        };
                        if !self.unify(f, elem, elem2) {
                            self.error(span, format!("`{label}` cannot concatenate {} and {}", self.show_ty(a_ty), self.show_ty(b_ty)));
                            had_error = true;
                        }
                        self.unify(f, e, elem);
                    }
                }
                "array_flatten" => {
                    let Some((inner_elem, _)) = self.array_shape(elem) else {
                        self.error(span, format!("`{label}` expects a nested array, found {}", self.show_ty(a_ty)));
                        return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
                    };
                    self.unify(f, e, inner_elem);
                }
                _ => {
                    self.unify(f, e, elem);
                }
            }
        } else if rt_name == "array_select" {
            let mask_ty = self.solved(f, b);
            let val_ty = self.solved(f, a);
            let mask_is_param = matches!(self.program.types.kind(mask_ty), TyKind::Param { .. });
            if mask_ty != TyId::ERROR && !mask_is_param {
                let Some((m_elem, m_len)) = self.array_shape(mask_ty) else {
                    self.error(span, format!("`{label}` expects a `[Bool]` or `[Bool; N]` mask, found {}", self.show_ty(mask_ty)));
                    return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
                };
                if m_elem != TyId::BOOL && m_elem != TyId::ERROR {
                    self.error(span, format!("`{label}` expects a `[Bool]` or `[Bool; N]` mask, found {}", self.show_ty(mask_ty)));
                    had_error = true;
                }
                let val_is_param = matches!(self.program.types.kind(val_ty), TyKind::Param { .. });
                if val_ty != TyId::ERROR && !val_is_param {
                    let Some((_, v_len)) = self.array_shape(val_ty) else {
                        self.error(span, format!("`{label}` expects arrays `[T]` or `[T; N]`, found {}", self.show_ty(val_ty)));
                        return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
                    };
                    if m_len != v_len {
                        self.error(span, format!("`{label}` needs matching shapes for {} and {}", self.show_ty(mask_ty), self.show_ty(val_ty)));
                        had_error = true;
                    }
                }
            }
        }

        if let Some(exp) = expected {
            let cur_ret = self.solved(f, ret_ty);
            if self.program.types.has_infer(cur_ret) {
                self.unify(f, cur_ret, exp);
            }
        }
        let mut final_ret = self.solved(f, ret_ty);
        if self.program.types.has_infer(final_ret) {
            self.error(span, format!("cannot infer type for `{label}`"));
            final_ret = TyId::ERROR;
        }
        if std_def.is_some() && had_error {
            return Some(f.body.expr(ExprKind::Error, TyId::ERROR, span));
        }
        let gen_args = all_vars.into_iter().map(|v| GenericArg::Type(self.solved(f, v))).collect();
        let callee = if let Some(def) = std_def {
            Callee::Function(FnRef { def, args: gen_args })
        } else {
            let sym = builtin_sym.expect("either std_def or builtin_sym must be Some");
            Callee::Builtin { name: sym, generic_args: gen_args }
        };
        Some(f.body.expr(
            ExprKind::Call {
                callee,
                args: checked_args,
            },
            final_ret,
            span,
        ))
    }

    pub(crate) fn array_shape(&self, ty: TyId) -> Option<(TyId, Option<ConstArg>)> {
        match self.program.types.kind(ty) {
            TyKind::Array(element) => Some((*element, None)),
            TyKind::FixedArray(element, len) => Some((*element, Some(len.clone()))),
            _ => None,
        }
    }

    pub(crate) fn shape_with_element(&mut self, len: Option<ConstArg>, element: TyId) -> TyId {
        match len {
            Some(n) => self.program.types.fixed_array_arg(element, n),
            None => self.program.types.array(element),
        }
    }
}
