//! String and math natives.

use super::*;

pub(super) fn str_receiver(ctx: &NativeCallContext<'_>, method: &str) -> Result<String, String> {
    ctx.arg(0)
        .and_then(|v| v.as_heap_string())
        .ok_or_else(|| format!("`.{method}()` is only defined on String"))
}

pub(super) fn str_arg(ctx: &NativeCallContext<'_>, i: usize, method: &str) -> Result<String, String> {
    ctx.arg(i)
        .and_then(|v| v.as_heap_string())
        .ok_or_else(|| format!("`.{method}()` expects a String argument"))
}

pub(super) fn build_bytes(ctx: &mut NativeCallContext<'_>, bytes: &[u8]) -> Result<Value, String> {
    let b = ctx.heap.alloc_header(BYTES_TYPE_ID, 2)?;
    let backing = ctx.heap.alloc_bytes_backing(bytes.len().max(1))?;
    if !bytes.is_empty() {
        ctx.heap.write_bytes(backing, 0, bytes)?;
    }
    ctx.heap
        .set_slot(b, HEADER_LEN_SLOT, Value::uint(bytes.len() as u64))?;
    ctx.heap.set_slot(b, HEADER_BACKING_SLOT, backing)?;
    Ok(b)
}

pub(super) fn str_find(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "find")?;
    let sub = str_arg(ctx, 1, "find")?;
    Ok(Value::int(s.find(&sub).map(|i| i as i64).unwrap_or(-1)))
}

pub(super) fn str_rfind(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "rfind")?;
    let sub = str_arg(ctx, 1, "rfind")?;
    Ok(Value::int(s.rfind(&sub).map(|i| i as i64).unwrap_or(-1)))
}

pub(super) fn str_replace(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "replace")?;
    let from = str_arg(ctx, 1, "replace")?;
    let to = str_arg(ctx, 2, "replace")?;
    if from.is_empty() {
        return Err("string.replace: the pattern must be non-empty".to_string());
    }
    ctx.heap.alloc_string(s.replace(&from, &to).as_bytes())
}

pub(super) fn str_to_upper(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "to_upper")?;
    ctx.heap.alloc_string(s.to_ascii_uppercase().as_bytes())
}

pub(super) fn str_to_lower(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "to_lower")?;
    ctx.heap.alloc_string(s.to_ascii_lowercase().as_bytes())
}

pub(super) fn str_trim(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "trim")?;
    ctx.heap.alloc_string(s.trim().as_bytes())
}

pub(super) fn str_split(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "split")?;
    let sep = str_arg(ctx, 1, "split")?;
    if sep.is_empty() {
        return Err("string.split: the separator must be non-empty".to_string());
    }
    let pieces: Vec<Value> = s
        .split(&sep)
        .map(|p| ctx.heap.alloc_string(p.as_bytes()))
        .collect::<Result<_, _>>()?;
    build_list(ctx, &pieces)
}

pub(super) fn str_bytes(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "bytes")?;
    build_bytes(ctx, s.as_bytes())
}

pub(super) fn str_repeat(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "repeat")?;
    let n = arg_int(ctx, 1);
    if n < 0 {
        return Err("string.repeat: the count must be non-negative".to_string());
    }
    ctx.heap.alloc_string(s.repeat(n as usize).as_bytes())
}

pub(super) fn str_starts_with(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "starts_with")?;
    let p = str_arg(ctx, 1, "starts_with")?;
    Ok(vbool(s.starts_with(&p)))
}

pub(super) fn str_ends_with(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let s = str_receiver(ctx, "ends_with")?;
    let p = str_arg(ctx, 1, "ends_with")?;
    Ok(vbool(s.ends_with(&p)))
}

pub(super) fn str_from(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let v = ctx.arg(0).unwrap_or(Value::null());
    let rendered = render(&v);
    ctx.heap.alloc_string(rendered.as_bytes())
}

pub(super) fn debug_raw(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    str_from(ctx)
}

pub(super) fn math_op(ctx: &NativeCallContext<'_>) -> Result<String, String> {
    ctx.arg(0)
        .and_then(|v| v.as_heap_string())
        .ok_or_else(|| "math: the op name must be a String".to_string())
}

pub(super) fn math_f(ctx: &NativeCallContext<'_>, i: usize) -> f64 {
    ctx.arg(i)
        .and_then(|v| v.as_float().or_else(|| v.as_int().map(|n| n as f64)))
        .unwrap_or(0.0)
}

pub(super) fn math_i(ctx: &NativeCallContext<'_>, i: usize) -> i64 {
    ctx.arg(i).and_then(|v| v.as_int()).unwrap_or(0)
}

pub(super) fn math_f1(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let op = math_op(ctx)?;
    let x = math_f(ctx, 1);
    let r = match op.as_str() {
        "sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "exp" => x.exp(),
        "ln" => x.ln(),
        "log2" => x.log2(),
        "log10" => x.log10(),
        "sin" => x.sin(),
        "cos" => x.cos(),
        "tan" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => x.round(),
        "trunc" => x.trunc(),
        "abs" => x.abs(),
        _ => return Err(format!("math: unknown unary op `{op}`")),
    };
    Ok(Value::float(r))
}

pub(super) fn math_f2(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let op = math_op(ctx)?;
    let x = math_f(ctx, 1);
    let y = math_f(ctx, 2);
    let r = match op.as_str() {
        "pow" => x.powf(y),
        "log" => x.log(y),
        "atan2" => x.atan2(y),
        "hypot" => x.hypot(y),
        "min" => x.min(y),
        "max" => x.max(y),
        "copysign" => x.copysign(y),
        _ => return Err(format!("math: unknown binary op `{op}`")),
    };
    Ok(Value::float(r))
}

pub(super) fn math_pred(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let op = math_op(ctx)?;
    let x = math_f(ctx, 1);
    let r = match op.as_str() {
        "is_nan" => x.is_nan(),
        "is_infinite" => x.is_infinite(),
        "is_finite" => x.is_finite(),
        _ => return Err(format!("math: unknown predicate `{op}`")),
    };
    Ok(vbool(r))
}

pub(super) fn math_iwrap(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let op = math_op(ctx)?;
    let a = math_i(ctx, 1);
    let b = math_i(ctx, 2);
    let r = match op.as_str() {
        "wrapping_add" => a.wrapping_add(b),
        "wrapping_sub" => a.wrapping_sub(b),
        "wrapping_mul" => a.wrapping_mul(b),
        "saturating_add" => a.saturating_add(b),
        "saturating_sub" => a.saturating_sub(b),
        "saturating_mul" => a.saturating_mul(b),
        _ => return Err(format!("math: unknown wrapping op `{op}`")),
    };
    Ok(Value::int(r))
}

pub(super) fn num_to_float(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    Ok(Value::float(math_i(ctx, 0) as f64))
}

pub(super) fn num_to_int(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let x = math_f(ctx, 0);
    if !x.is_finite() {
        return Err(format!("to_int: {x} is not finite"));
    }
    Ok(Value::int(x.trunc() as i64))
}

pub(super) fn int_from(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let v = ctx.arg(0).ok_or("Int.from: missing argument")?;
    if let Some(c) = v.as_char() {
        return Ok(Value::int(c as i64));
    }
    if let Some(b) = v.as_bool() {
        return Ok(Value::int(b as i64));
    }
    if v.as_float().is_some() {
        return num_to_int(ctx);
    }
    Err("Int.from takes a Float, Bool or Char".into())
}

fn binary_fits(sym: &str, l: &str, r: &str) -> bool {
    match sym {
        "+" => l == r && matches!(l, "Int" | "Float" | "String"),
        "-" | "*" | "/" | "%" => l == r && matches!(l, "Int" | "Float"),
        "<" | "<=" | ">" | ">=" => l == r && matches!(l, "Int" | "Float" | "String"),
        _ => l == "Int" && r == "Int",
    }
}

pub(super) fn operand_check(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let sym = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let (l, r) = (ctx.arg(1).unwrap_or_else(Value::null), ctx.arg(2).unwrap_or_else(Value::null));
    let verbose = ctx.arg(3).and_then(|v| v.as_bool()).unwrap_or(true);
    let (lt, rt) = (describe_type_name(&l), describe_type_name(&r));
    if binary_fits(&sym, &lt, &rt) {
        return Ok(Value::null());
    }
    Err(if verbose { format!("type mismatch: cannot apply `{sym}` to `{lt}` and `{rt}`") } else { "type mismatch".to_string() })
}

pub(super) fn unary_check(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let sym = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let v = ctx.arg(1).unwrap_or_else(Value::null);
    let verbose = ctx.arg(2).and_then(|v| v.as_bool()).unwrap_or(true);
    let ty = describe_type_name(&v);
    let fits = match sym.as_str() {
        "-" => matches!(ty.as_str(), "Int" | "Float"),
        "!" => ty == "Bool",
        _ => ty == "Int",
    };
    if fits {
        return Ok(Value::null());
    }
    Err(if verbose { format!("type mismatch: cannot apply `{sym}` to `{ty}`") } else { "type mismatch".to_string() })
}
