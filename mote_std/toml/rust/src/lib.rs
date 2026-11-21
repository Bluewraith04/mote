//! The Rust backend of `toml`: TOML text to and from a tagged JSON tree.
//! Every value is `[tag, payload]`: `t` a table (an object), `a` an array, `s` a string, `i` an integer and `f` a float (as text), `b` a boolean, `d` a date-time (as text).
#![allow(clippy::missing_safety_doc)]
use serde_json::{Map, Value as Json};
use toml::{Table, Value};

fn tag(kind: &str, payload: Json) -> Json {
    Json::Array(vec![Json::String(kind.to_string()), payload])
}

fn float_text(value: f64) -> String {
    if value.is_nan() {
        "nan".to_string()
    } else if value.is_infinite() {
        if value < 0.0 { "-inf" } else { "inf" }.to_string()
    } else {
        format!("{value:?}")
    }
}

fn to_json(value: &Value) -> Json {
    match value {
        Value::String(text) => tag("s", Json::String(text.clone())),
        Value::Integer(number) => tag("i", Json::String(number.to_string())),
        Value::Float(number) => tag("f", Json::String(float_text(*number))),
        Value::Boolean(flag) => tag("b", Json::Bool(*flag)),
        Value::Datetime(time) => tag("d", Json::String(time.to_string())),
        Value::Array(items) => tag("a", Json::Array(items.iter().map(to_json).collect())),
        Value::Table(table) => {
            let mut members = Map::new();
            for (key, inner) in table {
                members.insert(key.clone(), to_json(inner));
            }
            tag("t", Json::Object(members))
        }
    }
}

fn from_json(json: &Json) -> Result<Value, String> {
    let pair = json.as_array().filter(|pair| pair.len() == 2).ok_or("a value is not a tag and a payload")?;
    let kind = pair[0].as_str().ok_or("a tag is not text")?;
    let payload = &pair[1];
    let text = || payload.as_str().ok_or_else(|| format!("the payload of {kind} is not text"));
    match kind {
        "s" => Ok(Value::String(text()?.to_string())),
        "i" => text()?.parse().map(Value::Integer).map_err(|_| "not an integer".to_string()),
        "f" => match text()? {
            "nan" => Ok(Value::Float(f64::NAN)),
            "inf" => Ok(Value::Float(f64::INFINITY)),
            "-inf" => Ok(Value::Float(f64::NEG_INFINITY)),
            other => other.parse().map(Value::Float).map_err(|_| "not a float".to_string()),
        },
        "b" => payload.as_bool().map(Value::Boolean).ok_or_else(|| "not a boolean".to_string()),
        "d" => text()?.parse().map(Value::Datetime).map_err(|_| "not a date-time".to_string()),
        "a" => {
            let items = payload.as_array().ok_or("an array has no items")?;
            items.iter().map(from_json).collect::<Result<Vec<_>, _>>().map(Value::Array)
        }
        "t" => {
            let members = payload.as_object().ok_or("a table has no members")?;
            let mut table = Table::new();
            for (key, inner) in members {
                table.insert(key.clone(), from_json(inner)?);
            }
            Ok(Value::Table(table))
        }
        other => Err(format!("unknown tag {other}")),
    }
}

/// `line L, column C: message` for a parse error at a place of the text.
fn place(text: &str, error: &toml::de::Error) -> String {
    let at = error.span().map_or(0, |span| span.start).min(text.len());
    let before = &text.as_bytes()[..at];
    let line = before.iter().filter(|&&byte| byte == b'\n').count() + 1;
    let column = at - before.iter().rposition(|&byte| byte == b'\n').map_or(0, |index| index + 1) + 1;
    format!("line {line}, column {column}: {}", error.message())
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

/// Parses TOML text into the tagged JSON of its table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toml_parse(source: *const u8, length: i64, out: *mut u8, capacity: i64) -> i64 {
    let result = std::str::from_utf8(unsafe { text(source, length) })
        .map_err(|_| "not UTF-8".to_string())
        .and_then(|source| source.parse::<Table>().map_err(|error| place(source, &error)))
        .and_then(|table| serde_json::to_string(&to_json(&Value::Table(table))).map_err(|error| error.to_string()));
    unsafe { give(result, out, capacity) }
}

/// Writes the tagged JSON of a table as TOML text, pretty (arrays one item to a line) when `pretty` is not 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn toml_print(json: *const u8, length: i64, pretty: i64, out: *mut u8, capacity: i64) -> i64 {
    let result = serde_json::from_slice::<Json>(unsafe { text(json, length) })
        .map_err(|error| error.to_string())
        .and_then(|json| from_json(&json))
        .and_then(|value| match value {
            Value::Table(table) => {
                let written = if pretty != 0 { toml::to_string_pretty(&table) } else { toml::to_string(&table) };
                written.map_err(|error| error.to_string())
            }
            _ => Err("the root is not a table".to_string()),
        });
    unsafe { give(result, out, capacity) }
}
