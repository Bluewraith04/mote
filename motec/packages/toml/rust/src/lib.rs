//! TOML to JSON and back, behind the bytes-in, bytes-out C convention of `std.dev.libtools`.

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

fn toml_error(text: &str, e: &toml::de::Error) -> String {
    let Some(span) = e.span() else { return e.message().to_string() };
    let before = &text[..span.start.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!("line {line}, column {column}: {}", e.message())
}

fn non_finite(f: f64) -> Json {
    Json::String(if f.is_nan() { "nan" } else if f > 0.0 { "inf" } else { "-inf" }.to_string())
}

fn toml_to_json(v: toml::Value) -> Json {
    match v {
        toml::Value::String(s) => Json::String(s),
        toml::Value::Integer(n) => Json::from(n),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map_or_else(|| non_finite(f), Json::Number),
        toml::Value::Boolean(b) => Json::Bool(b),
        toml::Value::Datetime(d) => Json::String(d.to_string()),
        toml::Value::Array(items) => Json::Array(items.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => Json::Object(t.into_iter().map(|(k, v)| (k, toml_to_json(v))).collect()),
    }
}

fn json_to_toml(v: &Json, path: &str) -> Result<toml::Value, String> {
    Ok(match v {
        Json::Null => return Err(format!("TOML has no null (at {path})")),
        Json::Bool(b) => toml::Value::Boolean(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => toml::Value::Integer(i),
            (None, Some(f)) if !n.is_u64() => toml::Value::Float(f),
            _ => return Err(format!("integer out of range (at {path})")),
        },
        Json::String(s) => toml::Value::String(s.clone()),
        Json::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                out.push(json_to_toml(item, &format!("{path}[{i}]"))?);
            }
            toml::Value::Array(out)
        }
        Json::Object(map) => {
            let mut table = toml::Table::new();
            for (k, item) in map {
                let at = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                table.insert(k.clone(), json_to_toml(item, &at)?);
            }
            toml::Value::Table(table)
        }
    })
}

fn parse(input: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(input).map_err(|e| e.to_string())?;
    let table: toml::Table = toml::from_str(text).map_err(|e| toml_error(text, &e))?;
    Ok(toml_to_json(toml::Value::Table(table)).to_string().into_bytes())
}

fn render(input: &[u8]) -> Result<Vec<u8>, String> {
    let json: Json = serde_json::from_slice(input).map_err(|e| e.to_string())?;
    if !json.is_object() {
        return Err("a TOML document is a table, not a value".to_string());
    }
    let toml::Value::Table(table) = json_to_toml(&json, "")? else { unreachable!() };
    toml::to_string(&table).map(String::into_bytes).map_err(|e| e.to_string())
}

/// TOML text in, the document as JSON text out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_toml_parse(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, parse)
}

/// A JSON object in, the TOML document out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_toml_render(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, render)
}
