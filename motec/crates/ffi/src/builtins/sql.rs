//! SQL natives: requests to the platform's SQLite databases.

use contracts::{SqlMode, SqlValue};

use super::*;

fn answering_sql(label: &'static str, build: impl FnOnce(&mut NativeCallContext<'_>, PlatformResponse) -> Result<Value, String> + Send + 'static) -> ContinuationFn {
    Box::new(move |ctx, result| match result {
        Ok(response) => build(ctx, response).map_err(NativeError::from),
        Err(e) => Err(NativeError::failure(e.kind as i32, format!("{label}: {}", e.message))),
    })
}

fn cells_arg(ctx: &NativeCallContext<'_>, idx: usize) -> Result<Vec<SqlValue>, String> {
    let list = ctx.arg(idx).ok_or("sql: missing parameter list")?;
    let len = slot_count(ctx.heap, list, HEADER_LEN_SLOT)?;
    let backing = ctx.heap.get_slot(list, HEADER_BACKING_SLOT)?;
    let mut cells = Vec::with_capacity(len);
    for i in 0..len {
        let v = ctx.heap.get_slot(backing, BACKING_DATA_BASE + i)?;
        cells.push(if v.is_null() {
            SqlValue::Null
        } else if let Some(n) = v.as_int() {
            SqlValue::Int(n)
        } else if let Some(f) = v.as_float() {
            SqlValue::Float(f.to_bits())
        } else if let Some(b) = v.as_bool() {
            SqlValue::Int(i64::from(b))
        } else if let Some(s) = v.as_heap_string() {
            SqlValue::Text(s)
        } else if is_bytes(v) {
            let (backing, len) = bytes_state(ctx, v)?;
            SqlValue::Blob(ctx.heap.read_bytes(backing, 0, len)?)
        } else {
            return Err(format!("sql: parameter {} must be null, an Int, a Float, a String or Bytes", i + 1));
        });
    }
    Ok(cells)
}

fn cell_value(ctx: &mut NativeCallContext<'_>, cell: &SqlValue) -> Result<Value, String> {
    Ok(match cell {
        SqlValue::Null => Value::null(),
        SqlValue::Int(n) => Value::int(*n),
        SqlValue::Float(bits) => Value::float(f64::from_bits(*bits)),
        SqlValue::Text(s) => ctx.heap.alloc_string(s.as_bytes())?,
        SqlValue::Blob(b) => alloc_bytes(ctx, b)?,
    })
}

pub(super) fn sql_open(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let path = ctx.arg(0).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let label = "sql open";
    let done = answering_sql(label, move |_, r| match r {
        PlatformResponse::Int(id) => Ok(Value::int(id)),
        other => Err(format!("{label}: platform returned {other:?}")),
    });
    Ok((PlatformRequest::SqlOpen { path }, done))
}

pub(super) fn sql_run(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let id = arg_int(ctx, 0);
    let tx = arg_int(ctx, 1);
    let sql = ctx.arg(2).and_then(|v| v.as_heap_string()).unwrap_or_default();
    let params = cells_arg(ctx, 3)?;
    let mode = match arg_int(ctx, 4) {
        0 => SqlMode::Execute,
        1 => SqlMode::Query,
        _ => SqlMode::Check,
    };
    let done = answering_sql("sql", |c, r| match r {
        PlatformResponse::Sql { changed, last_id, columns, rows } => {
            let mut flat = Vec::with_capacity(3 + columns.len() + rows.len() * columns.len());
            flat.extend([Value::int(changed), Value::int(last_id), Value::int(columns.len() as i64)]);
            for name in &columns {
                flat.push(c.heap.alloc_string(name.as_bytes())?);
            }
            for row in &rows {
                for cell in row {
                    flat.push(cell_value(c, cell)?);
                }
            }
            alloc_list(c, &flat)
        }
        other => Err(format!("sql: platform returned {other:?}")),
    });
    Ok((PlatformRequest::SqlRun { id, tx, sql, params, mode }, done))
}

pub(super) fn sql_begin(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let done = answering_sql("sql begin", |_, r| match r {
        PlatformResponse::Int(token) => Ok(Value::int(token)),
        other => Err(format!("sql begin: platform returned {other:?}")),
    });
    Ok((PlatformRequest::SqlBegin { id: arg_int(ctx, 0) }, done))
}

pub(super) fn sql_end(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let commit = arg_int(ctx, 1) != 0;
    let done = answering_sql(if commit { "sql commit" } else { "sql rollback" }, |_, _| Ok(Value::null()));
    Ok((PlatformRequest::SqlEnd { tx: arg_int(ctx, 0), commit }, done))
}

pub(super) fn sql_close(ctx: &mut NativeCallContext<'_>) -> PlatformNative {
    let done = answering_sql("sql close", |_, _| Ok(Value::null()));
    Ok((PlatformRequest::SqlClose { id: arg_int(ctx, 0) }, done))
}
