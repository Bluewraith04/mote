//! HTTP/1 request parsing natives over the `httparse` and `http` crates.

use super::formats::{bytes_arg, invalid};
use super::*;

const MAX_HEADERS: usize = 100;

pub(super) fn http_parse_request(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let buf = bytes_arg(ctx, 0)?;
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut request = httparse::Request::new(&mut headers);
    let consumed = match request.parse(&buf).map_err(|e| invalid(e.to_string()))? {
        httparse::Status::Partial => return Ok(alloc_list(ctx, &[Value::int(0)])?),
        httparse::Status::Complete(n) => n,
    };
    let mut flat = vec![
        Value::int(consumed as i64),
        ctx.heap.alloc_string(request.method.unwrap_or_default().as_bytes())?,
        ctx.heap.alloc_string(request.path.unwrap_or_default().as_bytes())?,
        Value::int(i64::from(request.version.unwrap_or(1))),
    ];
    for header in request.headers.iter() {
        flat.push(ctx.heap.alloc_string(header.name.to_ascii_lowercase().as_bytes())?);
        flat.push(ctx.heap.alloc_string(String::from_utf8_lossy(header.value).as_bytes())?);
    }
    Ok(alloc_list(ctx, &flat)?)
}

enum Chunked {
    Incomplete,
    TooLarge,
    Done { consumed: usize, body: Vec<u8> },
}

fn decode_chunked(buf: &[u8], max: u64) -> Result<Chunked, String> {
    let mut pos = 0;
    let mut body: Vec<u8> = Vec::new();
    loop {
        let (used, size) = match httparse::parse_chunk_size(&buf[pos..]).map_err(|_| "bad chunk size".to_string())? {
            httparse::Status::Partial => return Ok(Chunked::Incomplete),
            httparse::Status::Complete(found) => found,
        };
        pos += used;
        if size == 0 {
            loop {
                let Some(end) = buf[pos..].windows(2).position(|w| w == b"\r\n") else { return Ok(Chunked::Incomplete) };
                pos += end + 2;
                if end == 0 {
                    return Ok(Chunked::Done { consumed: pos, body });
                }
            }
        }
        if body.len() as u64 + size > max {
            return Ok(Chunked::TooLarge);
        }
        let size = size as usize;
        if buf.len() < pos + size + 2 {
            return Ok(Chunked::Incomplete);
        }
        body.extend_from_slice(&buf[pos..pos + size]);
        if &buf[pos + size..pos + size + 2] != b"\r\n" {
            return Err("bad chunk end".to_string());
        }
        pos += size + 2;
    }
}

pub(super) fn http_decode_chunked(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let buf = bytes_arg(ctx, 0)?;
    let max = arg_int(ctx, 1).max(0) as u64;
    let flat = match decode_chunked(&buf, max).map_err(invalid)? {
        Chunked::Incomplete => vec![Value::int(0)],
        Chunked::TooLarge => vec![Value::int(-1)],
        Chunked::Done { consumed, body } => vec![Value::int(consumed as i64), alloc_bytes(ctx, &body)?],
    };
    Ok(alloc_list(ctx, &flat)?)
}

pub(super) fn http_date(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    ctx.heap.alloc_string(httpdate::fmt_http_date(std::time::SystemTime::now()).as_bytes())
}

pub(super) fn http_reason(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let reason = u16::try_from(arg_int(ctx, 0))
        .ok()
        .and_then(|code| http::StatusCode::from_u16(code).ok())
        .and_then(|status| status.canonical_reason())
        .unwrap_or_default();
    ctx.heap.alloc_string(reason.as_bytes())
}
