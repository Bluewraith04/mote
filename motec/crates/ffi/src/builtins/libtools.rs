//! `std.dev.libtools` natives: open a library, bind a symbol, call it.

use super::*;
use crate::local_refs::LocalReferenceScope;
use contracts::{CArg, NativeError, NativeFn, NativeOutcome, PlatformError};

fn id_answer(result: Result<PlatformResponse, PlatformError>) -> Result<Value, NativeError> {
    match result {
        Ok(PlatformResponse::Int(id)) => Ok(Value::int(id)),
        Ok(other) => Err(NativeError::fault(format!("libtools: platform returned {other:?}"))),
        Err(e) => Err(NativeError::failure(e.kind as i32, e.message)),
    }
}

fn text_arg(ctx: &NativeCallContext<'_>, idx: usize, who: &str) -> Result<String, String> {
    ctx.arg(idx).and_then(|v| v.as_heap_string()).ok_or_else(|| format!("{who}: argument {} must be a String", idx + 1))
}

pub(super) fn libtools_open(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let path = text_arg(ctx, 0, "libtools open")?;
    id_answer(ctx.heap.platform()?.execute(PlatformRequest::LibOpen { path }))
}

pub(super) fn libtools_symbol(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let lib = arg_int(ctx, 0);
    let name = text_arg(ctx, 1, "libtools bind")?;
    let signature = text_arg(ctx, 2, "libtools bind")?;
    id_answer(ctx.heap.platform()?.execute(PlatformRequest::LibSymbol { lib, name, signature }))
}

pub(super) fn libtools_close(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let lib = arg_int(ctx, 0);
    ask(ctx, PlatformRequest::LibClose { lib })?;
    Ok(Value::null())
}

fn c_arg(ctx: &NativeCallContext<'_>, v: Value) -> Result<CArg, String> {
    if let Some(s) = v.as_heap_string() {
        return Ok(CArg::Text(s));
    }
    if is_bytes(v) {
        let (backing, _) = bytes_state(ctx, v)?;
        return Ok(CArg::Word(ctx.heap.bytes_ptr(backing)? as usize as u64));
    }
    if let Some(f) = v.as_float() {
        return Ok(CArg::Word(f.to_bits()));
    }
    if let Some(b) = v.as_bool() {
        return Ok(CArg::Word(b as u64));
    }
    v.as_int().map(|i| CArg::Word(i as u64)).ok_or_else(|| "libtools call: an argument has no C type".to_string())
}

fn call_request(ctx: &mut NativeCallContext<'_>) -> Result<(PlatformRequest, Option<char>), String> {
    let symbol = arg_int(ctx, 0);
    let signature = text_arg(ctx, 1, "libtools call")?;
    let list = ctx.arg(2).ok_or("libtools call: missing arguments")?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    let backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    let mut args = Vec::with_capacity(len);
    for i in 0..len {
        let v = ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)?;
        args.push(c_arg(ctx, v)?);
    }
    let blocking = ctx.arg(3).and_then(|v| v.as_bool()).unwrap_or(true);
    Ok((PlatformRequest::LibCall { symbol, args, blocking }, signature.chars().last()))
}

fn call_result(ctx: &mut dyn NativeCtx, kind: Option<char>, answer: PlatformResult) -> Result<Value, NativeError> {
    match (kind, answer.map_err(|e| e.message)?) {
        (Some('i'), PlatformResponse::Int(n)) => Ok(Value::int(n)),
        (Some('f'), PlatformResponse::Int(bits)) => Ok(Value::float(f64::from_bits(bits as u64))),
        (Some('b'), PlatformResponse::Int(n)) => Ok(vbool(n != 0)),
        (Some('s'), PlatformResponse::Text(t)) => Ok(ctx.alloc_string(t.as_bytes())?),
        (Some('v'), PlatformResponse::Unit) => Ok(Value::null()),
        (_, other) => Err(format!("libtools call: platform returned {other:?}").into()),
    }
}

pub(super) struct LibtoolsCall;

impl NativeFn for LibtoolsCall {
    fn call(&self, cx: &mut dyn NativeCtx, args: &[Value]) -> NativeOutcome {
        let mut scope = LocalReferenceScope::new();
        let mut ctx = NativeCallContext::new(args, &mut scope, cx);
        match call_request(&mut ctx) {
            Ok((request, kind)) => NativeOutcome::Platform(request, Box::new(move |cx, answer| call_result(cx, kind, answer))),
            Err(message) => NativeOutcome::Fail(NativeError::fault(message)),
        }
    }
}

