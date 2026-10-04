//! YAML to JSON and back, behind the bytes-in, bytes-out C convention of `std.dev.libtools`.

use serde_json::Value as Json;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

/// Runs `work` on the input and writes its answer to `out`: the answer's length (nothing is written when it exceeds `cap`), or the negated length of an error message written to `out`.
fn call(input: *const u8, len: i64, out: *mut u8, cap: i64, work: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>) -> i64 {
    let bytes: &[u8] = if len <= 0 || input.is_null() { &[] } else { unsafe { slice::from_raw_parts(input, len as usize) } };
    let answer = catch_unwind(AssertUnwindSafe(|| work(bytes))).unwrap_or_else(|_| Err("the library failed".to_string()));
    let (data, sign) = match answer {
        Ok(data) => (data, 1),
        Err(message) => (if message.is_empty() { b"failed".to_vec() } else { message.into_bytes() }, -1),
    };
    let fits = data.len() as i64 <= cap;
    if (fits || sign < 0) && cap > 0 {
        let n = data.len().min(cap as usize);
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, n) };
    }
    sign * data.len() as i64
}

fn yaml_key(k: serde_yaml_ng::Value) -> Result<String, String> {
    use serde_yaml_ng::Value as Y;
    match k {
        Y::String(s) => Ok(s),
        Y::Number(n) => Ok(n.to_string()),
        Y::Bool(b) => Ok(b.to_string()),
        Y::Null => Ok("null".to_string()),
        Y::Tagged(t) => yaml_key(t.value),
        _ => Err("a mapping key must be a scalar".to_string()),
    }
}

fn yaml_to_json(v: serde_yaml_ng::Value) -> Result<Json, String> {
    use serde_yaml_ng::Value as Y;
    Ok(match v {
        Y::Null => Json::Null,
        Y::Bool(b) => Json::Bool(b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                Json::from(i)
            } else if n.as_u64().is_some() {
                return Err("integer out of range".to_string());
            } else {
                let f = n.as_f64().unwrap_or(f64::NAN);
                serde_json::Number::from_f64(f).map_or_else(
                    || Json::String(if f.is_nan() { "nan".to_string() } else if f > 0.0 { "inf".to_string() } else { "-inf".to_string() }),
                    Json::Number,
                )
            }
        }
        Y::String(s) => Json::String(s),
        Y::Sequence(items) => Json::Array(items.into_iter().map(yaml_to_json).collect::<Result<_, _>>()?),
        Y::Mapping(map) => {
            let mut out = serde_json::Map::new();
            for (k, item) in map {
                out.insert(yaml_key(k)?, yaml_to_json(item)?);
            }
            Json::Object(out)
        }
        Y::Tagged(t) => yaml_to_json(t.value)?,
    })
}

fn parse(input: &[u8], all: bool) -> Result<Vec<u8>, String> {
    use serde::Deserialize as _;
    let text = std::str::from_utf8(input).map_err(|e| e.to_string())?;
    let mut docs = Vec::new();
    let blank = text.trim().is_empty();
    for de in serde_yaml_ng::Deserializer::from_str(text).filter(|_| !blank) {
        let mut value = serde_yaml_ng::Value::deserialize(de).map_err(|e| e.to_string())?;
        value.apply_merge().map_err(|e| e.to_string())?;
        docs.push(yaml_to_json(value)?);
    }
    let json = if all {
        Json::Array(docs)
    } else {
        match docs.len() {
            0 => Json::Null,
            1 => docs.remove(0),
            n => return Err(format!("found {n} documents, expected one")),
        }
    };
    Ok(json.to_string().into_bytes())
}

fn render(input: &[u8]) -> Result<Vec<u8>, String> {
    let json: Json = serde_json::from_slice(input).map_err(|e| e.to_string())?;
    serde_yaml_ng::to_string(&json).map(String::into_bytes).map_err(|e| e.to_string())
}

/// YAML text in, its document (or, with `all` non-zero, a list of every document) as JSON text out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_yaml_parse(input: *const u8, len: i64, out: *mut u8, cap: i64, all: i64) -> i64 {
    call(input, len, out, cap, |bytes| parse(bytes, all != 0))
}

/// JSON text in, the YAML document out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_yaml_render(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, render)
}
