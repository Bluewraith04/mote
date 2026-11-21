//! The Rust backend of `yaml`: YAML text to and from JSON text.
//! A tag is dropped, an alias and a merge key are expanded, a scalar key is written as text, and a float that is not finite is the text `.inf`, `-.inf` or `.nan`.
#![allow(clippy::missing_safety_doc)]
use serde::Deserialize;
use serde_json::{Map, Number, Value as Json};
use serde_yaml_ng::{Deserializer, Value};

fn key_text(key: &Value) -> Result<String, String> {
    match key {
        Value::Null => Ok("null".to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        Value::String(text) => Ok(text.clone()),
        Value::Tagged(tagged) => key_text(&tagged.value),
        _ => Err("a key that is not a scalar".to_string()),
    }
}

fn to_json(value: &Value) -> Result<Json, String> {
    Ok(match value {
        Value::Null => Json::Null,
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Number(number) => {
            if let Some(whole) = number.as_i64() {
                Json::Number(whole.into())
            } else if let Some(whole) = number.as_u64() {
                Json::Number(whole.into())
            } else {
                let real = number.as_f64().unwrap_or(f64::NAN);
                match Number::from_f64(real) {
                    Some(real) => Json::Number(real),
                    None if real.is_nan() => Json::String(".nan".to_string()),
                    None => Json::String(if real < 0.0 { "-.inf" } else { ".inf" }.to_string()),
                }
            }
        }
        Value::String(text) => Json::String(text.clone()),
        Value::Sequence(items) => Json::Array(items.iter().map(to_json).collect::<Result<_, _>>()?),
        Value::Mapping(members) => {
            let mut out = Map::new();
            for (key, inner) in members {
                out.insert(key_text(key)?, to_json(inner)?);
            }
            Json::Object(out)
        }
        Value::Tagged(tagged) => to_json(&tagged.value)?,
    })
}

fn documents(text: &str) -> Result<Vec<Json>, String> {
    let mut out = Vec::new();
    if text.lines().all(|line| line.trim().is_empty() || line.trim_start().starts_with('#')) {
        return Ok(out);
    }
    for document in Deserializer::from_str(text) {
        let mut value = Value::deserialize(document).map_err(|error| error.to_string())?;
        value.apply_merge().map_err(|error| error.to_string())?;
        out.push(to_json(&value)?);
    }
    Ok(out)
}

/// Copies up to `capacity` bytes of a result to `out`; gives `length` for a value and `-length - 1` for an error message.
unsafe fn give(result: Result<String, String>, out: *mut u8, capacity: i64) -> i64 {
    let (bytes, failed) = match &result {
        Ok(text) => (text.as_bytes(), false),
        Err(message) => (message.as_bytes(), true),
    };
    if capacity > 0 {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len().min(capacity as usize)) };
    }
    if failed { -(bytes.len() as i64) - 1 } else { bytes.len() as i64 }
}

unsafe fn text<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

/// Parses a stream of YAML documents into the JSON text of an array of them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yaml_parse(source: *const u8, length: i64, out: *mut u8, capacity: i64) -> i64 {
    let result = std::str::from_utf8(unsafe { text(source, length) })
        .map_err(|_| "not UTF-8".to_string())
        .and_then(documents)
        .and_then(|documents| serde_json::to_string(&documents).map_err(|error| error.to_string()));
    unsafe { give(result, out, capacity) }
}

/// Writes JSON text as the YAML text of one document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yaml_print(json: *const u8, length: i64, out: *mut u8, capacity: i64) -> i64 {
    let result = serde_json::from_slice::<Json>(unsafe { text(json, length) })
        .map_err(|error| error.to_string())
        .and_then(|json| serde_yaml_ng::to_string(&json).map_err(|error| error.to_string()));
    unsafe { give(result, out, capacity) }
}
