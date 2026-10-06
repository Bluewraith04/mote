//! Format and codec natives over the `base64`, `uuid` and `form_urlencoded` crates.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;

use super::*;

pub(super) fn invalid(message: impl Into<String>) -> NativeError {
    NativeError::failure(INVALID_DATA, message)
}

pub(super) fn text_arg(ctx: &NativeCallContext<'_>, i: usize) -> String {
    ctx.arg(i).and_then(|v| v.as_heap_string()).unwrap_or_default()
}

pub(super) fn bytes_arg(ctx: &NativeCallContext<'_>, i: usize) -> Result<Vec<u8>, String> {
    let b = ctx.arg(i).ok_or("missing Bytes argument")?;
    let (backing, len) = bytes_state(ctx, b)?;
    Ok(ctx.heap.read_bytes(backing, 0, len)?.to_vec())
}

pub(super) fn base64_encode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let data = bytes_arg(ctx, 0)?;
    let url = ctx.arg(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let text = if url { URL_SAFE_NO_PAD.encode(data) } else { STANDARD.encode(data) };
    ctx.heap.alloc_string(text.as_bytes())
}

pub(super) fn base64_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let text = text_arg(ctx, 0);
    let url = ctx.arg(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let decoded = if url { URL_SAFE_NO_PAD.decode(text) } else { STANDARD.decode(text) };
    let data = decoded.map_err(|e| invalid(e.to_string()))?;
    Ok(alloc_bytes(ctx, &data)?)
}

pub(super) fn form_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = text_arg(ctx, 0);
    let mut flat = Vec::new();
    for (name, value) in form_urlencoded::parse(text.as_bytes()) {
        flat.push(ctx.heap.alloc_string(name.as_bytes())?);
        flat.push(ctx.heap.alloc_string(value.as_bytes())?);
    }
    alloc_list(ctx, &flat)
}

pub(super) fn percent_decode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let text = text_arg(ctx, 0);
    ctx.heap.alloc_string(percent_encoding::percent_decode_str(&text).decode_utf8_lossy().as_bytes())
}

fn uuid_error(e: uuid::Error) -> NativeError {
    invalid(e.to_string().replace(": ", ", "))
}

fn alloc_uuid(ctx: &mut NativeCallContext<'_>, id: uuid::Uuid) -> Result<Value, String> {
    ctx.heap.alloc_string(id.hyphenated().to_string().as_bytes())
}

pub(super) fn uuid_v4(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    alloc_uuid(ctx, uuid::Uuid::new_v4())
}

pub(super) fn uuid_v7(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    alloc_uuid(ctx, uuid::Uuid::now_v7())
}

pub(super) fn uuid_parse(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(uuid_error)?;
    Ok(alloc_uuid(ctx, id)?)
}

pub(super) fn uuid_from_bytes(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let id = uuid::Uuid::from_slice(&bytes_arg(ctx, 0)?).map_err(uuid_error)?;
    Ok(alloc_uuid(ctx, id)?)
}

pub(super) fn uuid_bytes(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(|e| e.to_string())?;
    alloc_bytes(ctx, id.as_bytes())
}

pub(super) fn uuid_version(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(|e| e.to_string())?;
    Ok(Value::int(id.get_version_num() as i64))
}
