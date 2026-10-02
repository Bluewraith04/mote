//! Format and codec natives over the `toml`, `base64`, `uuid` and `serde_yaml_ng` crates.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use serde_json::Value as Json;

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

fn toml_error(text: &str, e: &toml::de::Error) -> String {
    let Some(span) = e.span() else { return e.message().to_string() };
    let before = &text[..span.start.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    format!("line {line}, column {column}: {}", e.message())
}

fn toml_to_json(v: toml::Value) -> Json {
    match v {
        toml::Value::String(s) => Json::String(s),
        toml::Value::Integer(n) => Json::from(n),
        toml::Value::Float(f) => serde_json::Number::from_f64(f).map_or_else(
            || Json::String(if f.is_nan() { "nan".to_string() } else if f > 0.0 { "inf".to_string() } else { "-inf".to_string() }),
            Json::Number,
        ),
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

pub(super) fn toml_parse(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let text = text_arg(ctx, 0);
    let table: toml::Table = toml::from_str(&text).map_err(|e| invalid(toml_error(&text, &e)))?;
    let json = toml_to_json(toml::Value::Table(table)).to_string();
    Ok(ctx.heap.alloc_string(json.as_bytes())?)
}

pub(super) fn toml_render(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let text = text_arg(ctx, 0);
    let json: Json = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    if !json.is_object() {
        return Err(invalid("a TOML document is a table, not a value"));
    }
    let toml::Value::Table(table) = json_to_toml(&json, "").map_err(invalid)? else { unreachable!() };
    let out = toml::to_string(&table).map_err(|e| invalid(e.to_string()))?;
    Ok(ctx.heap.alloc_string(out.as_bytes())?)
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

pub(super) fn form_encode(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let flat = string_list_arg(ctx, 0, "form_encode")?;
    let mut out = form_urlencoded::Serializer::new(String::new());
    for pair in flat.chunks(2).filter(|p| p.len() == 2) {
        out.append_pair(&pair[0], &pair[1]);
    }
    ctx.heap.alloc_string(out.finish().as_bytes())
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

pub(super) fn yaml_parse(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    use serde::Deserialize as _;
    let text = text_arg(ctx, 0);
    let all = ctx.arg(1).and_then(|v| v.as_bool()).unwrap_or(false);
    let mut docs = Vec::new();
    let blank = text.trim().is_empty();
    for de in serde_yaml_ng::Deserializer::from_str(&text).filter(|_| !blank) {
        let mut value = serde_yaml_ng::Value::deserialize(de).map_err(|e| invalid(e.to_string()))?;
        value.apply_merge().map_err(|e| invalid(e.to_string()))?;
        docs.push(yaml_to_json(value).map_err(invalid)?);
    }
    let json = if all {
        Json::Array(docs)
    } else {
        match docs.len() {
            0 => Json::Null,
            1 => docs.remove(0),
            n => return Err(invalid(format!("found {n} documents, expected one"))),
        }
    };
    Ok(ctx.heap.alloc_string(json.to_string().as_bytes())?)
}

pub(super) fn yaml_render(ctx: &mut NativeCallContext<'_>) -> Result<Value, NativeError> {
    let json: Json = serde_json::from_str(&text_arg(ctx, 0)).map_err(|e| invalid(e.to_string()))?;
    let out = serde_yaml_ng::to_string(&json).map_err(|e| invalid(e.to_string()))?;
    Ok(ctx.heap.alloc_string(out.as_bytes())?)
}

pub(super) fn uuid_version(ctx: &mut NativeCallContext<'_>) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(&text_arg(ctx, 0)).map_err(|e| e.to_string())?;
    Ok(Value::int(id.get_version_num() as i64))
}
