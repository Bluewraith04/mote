//! The native of `sqlite`: connections, parameter lists and result sets by handle over the bundled SQLite.
#![allow(clippy::missing_safety_doc)]
use rusqlite::ffi;
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};
use std::sync::Mutex;

unsafe fn bytes<'a>(pointer: *const u8, length: i64) -> &'a [u8] {
    if length <= 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

unsafe fn give(value: &[u8], out: *mut u8, capacity: i64) {
    if capacity > 0 {
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, value.len().min(capacity as usize)) };
    }
}

unsafe fn fail(message: &str, error: *mut u8, capacity: i64) -> i64 {
    unsafe { give(message.as_bytes(), error, capacity) };
    -(message.len() as i64) - 1
}

unsafe fn text<'a>(pointer: *const u8, length: i64) -> Result<&'a str, String> {
    std::str::from_utf8(unsafe { bytes(pointer, length) }).map_err(|_| "text is not UTF-8".to_string())
}

struct Rows {
    names: Vec<String>,
    rows: Vec<Vec<Value>>,
}

struct PreparedStatement {
    conn_handle: i64,
    stmt: *mut ffi::sqlite3_stmt,
}

struct DbConn {
    conn: Mutex<Connection>,
    raw: *mut ffi::sqlite3,
}

unsafe fn statement<'a>(handle: i64) -> &'a PreparedStatement {
    unsafe { &*(handle as *const PreparedStatement) }
}

unsafe fn db_conn<'a>(handle: i64) -> &'a DbConn {
    unsafe { &*(handle as *const DbConn) }
}

unsafe fn connection<'a>(handle: i64) -> &'a Mutex<Connection> {
    &unsafe { db_conn(handle) }.conn
}

unsafe fn params<'a>(handle: i64) -> &'a mut Vec<Value> {
    unsafe { &mut *(handle as *mut Vec<Value>) }
}

unsafe fn cell<'a>(handle: i64, row: i64, column: i64) -> Option<&'a Value> {
    unsafe { &*(handle as *const Rows) }.rows.get(row as usize)?.get(column as usize)
}

unsafe fn bind_cell(stmt: *mut ffi::sqlite3_stmt, col: i32, value: &Value) {
    match value {
        Value::Null => {
            unsafe { ffi::sqlite3_bind_null(stmt, col) };
        }
        Value::Integer(i) => {
            unsafe { ffi::sqlite3_bind_int64(stmt, col, *i) };
        }
        Value::Real(r) => {
            unsafe { ffi::sqlite3_bind_double(stmt, col, *r) };
        }
        Value::Text(s) => {
            unsafe {
                ffi::sqlite3_bind_text(
                    stmt,
                    col,
                    s.as_ptr() as *const std::ffi::c_char,
                    s.len() as std::ffi::c_int,
                    ffi::SQLITE_TRANSIENT(),
                )
            };
        }
        Value::Blob(b) => {
            unsafe {
                ffi::sqlite3_bind_blob(
                    stmt,
                    col,
                    b.as_ptr() as *const std::ffi::c_void,
                    b.len() as std::ffi::c_int,
                    ffi::SQLITE_TRANSIENT(),
                )
            };
        }
    }
}

unsafe fn step_rows(raw: *mut ffi::sqlite3_stmt) -> Rows {
    let col_count = unsafe { ffi::sqlite3_column_count(raw) };
    let mut names = Vec::with_capacity(col_count as usize);
    for col in 0..col_count {
        let name_ptr = unsafe { ffi::sqlite3_column_name(raw, col) };
        let name = if name_ptr.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(name_ptr) }.to_string_lossy().into_owned()
        };
        names.push(name);
    }
    let mut rows = Vec::new();
    while unsafe { ffi::sqlite3_step(raw) } == ffi::SQLITE_ROW {
        let mut cells = Vec::with_capacity(col_count as usize);
        for col in 0..col_count {
            let val = match unsafe { ffi::sqlite3_column_type(raw, col) } {
                ffi::SQLITE_INTEGER => Value::Integer(unsafe { ffi::sqlite3_column_int64(raw, col) }),
                ffi::SQLITE_FLOAT => Value::Real(unsafe { ffi::sqlite3_column_double(raw, col) }),
                ffi::SQLITE_TEXT => {
                    let text_ptr = unsafe { ffi::sqlite3_column_text(raw, col) };
                    let bytes_len = unsafe { ffi::sqlite3_column_bytes(raw, col) } as usize;
                    let text = if text_ptr.is_null() || bytes_len == 0 {
                        String::new()
                    } else {
                        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text_ptr, bytes_len) }).into_owned()
                    };
                    Value::Text(text)
                }
                ffi::SQLITE_BLOB => {
                    let blob_ptr = unsafe { ffi::sqlite3_column_blob(raw, col) };
                    let bytes_len = unsafe { ffi::sqlite3_column_bytes(raw, col) } as usize;
                    let blob = if blob_ptr.is_null() || bytes_len == 0 {
                        Vec::new()
                    } else {
                        unsafe { std::slice::from_raw_parts(blob_ptr as *const u8, bytes_len) }.to_vec()
                    };
                    Value::Blob(blob)
                }
                _ => Value::Null,
            };
            cells.push(val);
        }
        rows.push(cells);
    }
    unsafe { ffi::sqlite3_reset(raw) };
    unsafe { ffi::sqlite3_clear_bindings(raw) };
    Rows { names, rows }
}

/// Opens a database file, making it when it is missing; `:memory:` is a private one in memory.
/// Returns a handle, or `-length - 1` with the message in `error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_open(path: *const u8, length: i64, error: *mut u8, capacity: i64) -> i64 {
    let path = match unsafe { text(path, length) } {
        Ok(path) => path,
        Err(message) => return unsafe { fail(&message, error, capacity) },
    };
    let opened = if path == ":memory:" { Connection::open_in_memory() } else { Connection::open(path) };
    match opened {
        Ok(connection) => {
            let raw = unsafe { connection.handle() };
            Box::into_raw(Box::new(DbConn { conn: Mutex::new(connection), raw })) as i64
        }
        Err(message) => unsafe { fail(&message.to_string(), error, capacity) },
    }
}

/// Closes a connection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_close(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut DbConn) });
    }
    0
}

/// Interrupts any ongoing query on this connection from any thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_interrupt(handle: i64) {
    if handle != 0 {
        let conn = unsafe { db_conn(handle) };
        unsafe { ffi::sqlite3_interrupt(conn.raw) };
    }
}

/// Compiles an SQL statement and returns a PreparedStatement handle, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_prepare(handle: i64, sql: *const u8, length: i64, error: *mut u8, capacity: i64) -> i64 {
    let sql = match unsafe { text(sql, length) } {
        Ok(sql) => sql,
        Err(message) => return unsafe { fail(&message, error, capacity) },
    };
    let connection = unsafe { connection(handle) }.lock().unwrap_or_else(|e| e.into_inner());
    let mut raw_stmt = std::ptr::null_mut();
    let rc = unsafe {
        ffi::sqlite3_prepare_v2(
            connection.handle(),
            sql.as_ptr() as *const std::ffi::c_char,
            sql.len() as std::ffi::c_int,
            &mut raw_stmt,
            std::ptr::null_mut(),
        )
    };
    if rc != ffi::SQLITE_OK {
        let err_msg = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(connection.handle())) }.to_string_lossy();
        return unsafe { fail(&err_msg, error, capacity) };
    }
    Box::into_raw(Box::new(PreparedStatement { conn_handle: handle, stmt: raw_stmt })) as i64
}

/// Executes a prepared statement with bound parameters. Returns rows changed, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_execute(stmt_handle: i64, bound: i64, error: *mut u8, capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let connection = unsafe { connection(stmt.conn_handle) }.lock().unwrap_or_else(|e| e.into_inner());
    let raw = stmt.stmt;
    unsafe { ffi::sqlite3_reset(raw) };
    unsafe { ffi::sqlite3_clear_bindings(raw) };
    let params = unsafe { params(bound) };
    for (idx, val) in params.iter().enumerate() {
        unsafe { bind_cell(raw, (idx + 1) as i32, val) };
    }
    let step_rc = unsafe { ffi::sqlite3_step(raw) };
    if step_rc != ffi::SQLITE_DONE && step_rc != ffi::SQLITE_ROW {
        let err_msg = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(connection.handle())) }.to_string_lossy();
        unsafe { ffi::sqlite3_reset(raw) };
        unsafe { ffi::sqlite3_clear_bindings(raw) };
        return unsafe { fail(&err_msg, error, capacity) };
    }
    let changed = unsafe { ffi::sqlite3_changes(connection.handle()) as i64 };
    unsafe { ffi::sqlite3_reset(raw) };
    unsafe { ffi::sqlite3_clear_bindings(raw) };
    changed
}

/// Executes a prepared statement query and returns a result handle, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_query(stmt_handle: i64, bound: i64, _error: *mut u8, _capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let _connection = unsafe { connection(stmt.conn_handle) }.lock().unwrap_or_else(|e| e.into_inner());
    let raw = stmt.stmt;
    unsafe { ffi::sqlite3_reset(raw) };
    unsafe { ffi::sqlite3_clear_bindings(raw) };
    let params = unsafe { params(bound) };
    for (idx, val) in params.iter().enumerate() {
        unsafe { bind_cell(raw, (idx + 1) as i32, val) };
    }
    let rows = unsafe { step_rows(raw) };
    Box::into_raw(Box::new(rows)) as i64
}

/// Binds parameters and resets statement for stepping. Returns 0 on success, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_bind(stmt_handle: i64, bound: i64, _error: *mut u8, _capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let raw = stmt.stmt;
    unsafe { ffi::sqlite3_reset(raw) };
    unsafe { ffi::sqlite3_clear_bindings(raw) };
    let params = unsafe { params(bound) };
    for (idx, val) in params.iter().enumerate() {
        unsafe { bind_cell(raw, (idx + 1) as i32, val) };
    }
    0
}

/// Steps the statement. Returns 1 for SQLITE_ROW, 0 for SQLITE_DONE, or `-length - 1` on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_step(stmt_handle: i64, error: *mut u8, capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let connection = unsafe { connection(stmt.conn_handle) }.lock().unwrap_or_else(|e| e.into_inner());
    let raw = stmt.stmt;
    let rc = unsafe { ffi::sqlite3_step(raw) };
    if rc == ffi::SQLITE_ROW {
        1
    } else if rc == ffi::SQLITE_DONE {
        0
    } else {
        let err_msg = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(connection.handle())) }.to_string_lossy();
        unsafe { fail(&err_msg, error, capacity) }
    }
}

/// Resets and clears bindings on a statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_reset(stmt_handle: i64) {
    if stmt_handle != 0 {
        let stmt = unsafe { statement(stmt_handle) };
        let raw = stmt.stmt;
        unsafe { ffi::sqlite3_reset(raw) };
        unsafe { ffi::sqlite3_clear_bindings(raw) };
    }
}

/// Number of columns in the statement result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_columns(stmt_handle: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    unsafe { ffi::sqlite3_column_count(stmt.stmt) as i64 }
}

/// Column name length, writing into `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_col_name(stmt_handle: i64, col: i64, out: *mut u8, capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let name_ptr = unsafe { ffi::sqlite3_column_name(stmt.stmt, col as std::ffi::c_int) };
    if name_ptr.is_null() {
        0
    } else {
        let bytes = unsafe { std::ffi::CStr::from_ptr(name_ptr) }.to_bytes();
        unsafe { give(bytes, out, capacity) };
        bytes.len() as i64
    }
}

/// Column type: 1 Integer, 2 Real, 3 Text, 4 Blob, 5 Null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_col_type(stmt_handle: i64, col: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    match unsafe { ffi::sqlite3_column_type(stmt.stmt, col as std::ffi::c_int) } {
        ffi::SQLITE_INTEGER => 1,
        ffi::SQLITE_FLOAT => 2,
        ffi::SQLITE_TEXT => 3,
        ffi::SQLITE_BLOB => 4,
        _ => 5,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_col_int(stmt_handle: i64, col: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    unsafe { ffi::sqlite3_column_int64(stmt.stmt, col as std::ffi::c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_col_real(stmt_handle: i64, col: i64) -> f64 {
    let stmt = unsafe { statement(stmt_handle) };
    unsafe { ffi::sqlite3_column_double(stmt.stmt, col as std::ffi::c_int) }
}

/// Column text or blob length, writing into `out`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_col_bytes(stmt_handle: i64, col: i64, out: *mut u8, capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let raw = stmt.stmt;
    let col_i32 = col as std::ffi::c_int;
    let len = unsafe { ffi::sqlite3_column_bytes(raw, col_i32) } as usize;
    if len == 0 {
        return 0;
    }
    let ptr = match unsafe { ffi::sqlite3_column_type(raw, col_i32) } {
        ffi::SQLITE_BLOB => unsafe { ffi::sqlite3_column_blob(raw, col_i32) as *const u8 },
        _ => unsafe { ffi::sqlite3_column_text(raw, col_i32) as *const u8 },
    };
    if !ptr.is_null() {
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        unsafe { give(slice, out, capacity) };
    }
    len as i64
}

/// Executes a prepared statement with multiple parameter sets in a single native pass.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_execute_batch(stmt_handle: i64, batch_handle: i64, error: *mut u8, capacity: i64) -> i64 {
    let stmt = unsafe { statement(stmt_handle) };
    let connection = unsafe { connection(stmt.conn_handle) }.lock().unwrap_or_else(|e| e.into_inner());
    let raw = stmt.stmt;
    let batch = unsafe { &*(batch_handle as *const Vec<Vec<Value>>) };
    let mut total_changed = 0i64;
    for row_params in batch {
        unsafe { ffi::sqlite3_reset(raw) };
        unsafe { ffi::sqlite3_clear_bindings(raw) };
        for (idx, val) in row_params.iter().enumerate() {
            unsafe { bind_cell(raw, (idx + 1) as i32, val) };
        }
        let step_rc = unsafe { ffi::sqlite3_step(raw) };
        if step_rc != ffi::SQLITE_DONE && step_rc != ffi::SQLITE_ROW {
            let err_msg = unsafe { std::ffi::CStr::from_ptr(ffi::sqlite3_errmsg(connection.handle())) }.to_string_lossy();
            unsafe { ffi::sqlite3_reset(raw) };
            unsafe { ffi::sqlite3_clear_bindings(raw) };
            return unsafe { fail(&err_msg, error, capacity) };
        }
        total_changed += unsafe { ffi::sqlite3_changes(connection.handle()) as i64 };
    }
    unsafe { ffi::sqlite3_reset(raw) };
    unsafe { ffi::sqlite3_clear_bindings(raw) };
    total_changed
}

/// Closes and frees a prepared statement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_stmt_close(stmt_handle: i64) -> i64 {
    if stmt_handle != 0 {
        let stmt = unsafe { Box::from_raw(stmt_handle as *mut PreparedStatement) };
        let _connection = unsafe { connection(stmt.conn_handle) }.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { ffi::sqlite3_finalize(stmt.stmt) };
    }
    0
}

/// Runs statements separated by `;` that take no parameters. Returns 0, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_batch(handle: i64, sql: *const u8, length: i64, error: *mut u8, capacity: i64) -> i64 {
    let sql = match unsafe { text(sql, length) } {
        Ok(sql) => sql,
        Err(message) => return unsafe { fail(&message, error, capacity) },
    };
    let connection = unsafe { connection(handle) }.lock().unwrap_or_else(|e| e.into_inner());
    match connection.execute_batch(sql) {
        Ok(()) => 0,
        Err(message) => unsafe { fail(&message.to_string(), error, capacity) },
    }
}

/// Runs one statement with parameters. Returns the rows it changed, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_execute(handle: i64, sql: *const u8, length: i64, bound: i64, error: *mut u8, capacity: i64) -> i64 {
    let sql = match unsafe { text(sql, length) } {
        Ok(sql) => sql,
        Err(message) => return unsafe { fail(&message, error, capacity) },
    };
    let connection = unsafe { connection(handle) }.lock().unwrap_or_else(|e| e.into_inner());
    let run = || -> rusqlite::Result<usize> { connection.prepare(sql)?.execute(params_from_iter(unsafe { params(bound) }.iter())) };
    match run() {
        Ok(changed) => changed as i64,
        Err(message) => unsafe { fail(&message.to_string(), error, capacity) },
    }
}

fn collect(connection: &Connection, sql: &str, bound: &[Value]) -> rusqlite::Result<Rows> {
    let mut statement = connection.prepare(sql)?;
    let names: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
    let mut rows = Vec::new();
    let mut cursor = statement.query(params_from_iter(bound.iter()))?;
    while let Some(row) = cursor.next()? {
        let mut cells = Vec::with_capacity(names.len());
        for column in 0..names.len() {
            cells.push(row.get::<_, Value>(column)?);
        }
        rows.push(cells);
    }
    Ok(Rows { names, rows })
}

/// Runs a query with parameters. Returns a result handle, or `-length - 1`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_query(handle: i64, sql: *const u8, length: i64, bound: i64, error: *mut u8, capacity: i64) -> i64 {
    let sql = match unsafe { text(sql, length) } {
        Ok(sql) => sql,
        Err(message) => return unsafe { fail(&message, error, capacity) },
    };
    let connection = unsafe { connection(handle) }.lock().unwrap_or_else(|e| e.into_inner());
    match collect(&connection, sql, unsafe { params(bound) }) {
        Ok(rows) => Box::into_raw(Box::new(rows)) as i64,
        Err(message) => unsafe { fail(&message.to_string(), error, capacity) },
    }
}

/// The rowid of the last row inserted on the connection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_last_insert_id(handle: i64) -> i64 {
    unsafe { connection(handle) }.lock().unwrap_or_else(|e| e.into_inner()).last_insert_rowid()
}

/// Starts a parameter list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_new() -> i64 {
    Box::into_raw(Box::new(Vec::<Value>::new())) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_null(handle: i64) -> i64 {
    unsafe { params(handle) }.push(Value::Null);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_int(handle: i64, value: i64) -> i64 {
    unsafe { params(handle) }.push(Value::Integer(value));
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_real(handle: i64, value: f64) -> i64 {
    unsafe { params(handle) }.push(Value::Real(value));
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_text(handle: i64, value: *const u8, length: i64) -> i64 {
    let value = String::from_utf8_lossy(unsafe { bytes(value, length) }).into_owned();
    unsafe { params(handle) }.push(Value::Text(value));
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_blob(handle: i64, value: *const u8, length: i64) -> i64 {
    unsafe { params(handle) }.push(Value::Blob(unsafe { bytes(value, length) }.to_vec()));
    0
}

/// Frees a parameter list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_params_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Vec<Value>) });
    }
    0
}

/// How many rows a result holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_count(handle: i64) -> i64 {
    unsafe { &*(handle as *const Rows) }.rows.len() as i64
}

/// How many columns a result has.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_columns(handle: i64) -> i64 {
    unsafe { &*(handle as *const Rows) }.names.len() as i64
}

/// Copies the name of a column to `out` and returns its length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_name(handle: i64, column: i64, out: *mut u8, capacity: i64) -> i64 {
    let Some(name) = unsafe { &*(handle as *const Rows) }.names.get(column as usize) else { return -1 };
    unsafe { give(name.as_bytes(), out, capacity) };
    name.len() as i64
}

/// The type of a cell: 0 null, 1 integer, 2 real, 3 text, 4 blob; -1 outside the result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_type(handle: i64, row: i64, column: i64) -> i64 {
    match unsafe { cell(handle, row, column) } {
        Some(Value::Null) => 0,
        Some(Value::Integer(_)) => 1,
        Some(Value::Real(_)) => 2,
        Some(Value::Text(_)) => 3,
        Some(Value::Blob(_)) => 4,
        None => -1,
    }
}

/// An integer cell; 0 for another type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_int(handle: i64, row: i64, column: i64) -> i64 {
    match unsafe { cell(handle, row, column) } {
        Some(Value::Integer(value)) => *value,
        _ => 0,
    }
}

/// A real cell; 0 for another type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_real(handle: i64, row: i64, column: i64) -> f64 {
    match unsafe { cell(handle, row, column) } {
        Some(Value::Real(value)) => *value,
        _ => 0.0,
    }
}

/// Copies a text or blob cell to `out` and returns its length; 0 for another type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_bytes(handle: i64, row: i64, column: i64, out: *mut u8, capacity: i64) -> i64 {
    let value: &[u8] = match unsafe { cell(handle, row, column) } {
        Some(Value::Text(value)) => value.as_bytes(),
        Some(Value::Blob(value)) => value,
        _ => return 0,
    };
    unsafe { give(value, out, capacity) };
    value.len() as i64
}

/// Frees a result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_rows_free(handle: i64) -> i64 {
    if handle != 0 {
        drop(unsafe { Box::from_raw(handle as *mut Rows) });
    }
    0
}

/// Starts a batch parameter list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_batch_params_new() -> i64 {
    Box::into_raw(Box::new(Vec::<Vec<Value>>::new())) as i64
}

/// Pushes a parameter list into a batch, taking ownership of the parameter list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_batch_params_push(batch_handle: i64, params_handle: i64) -> i64 {
    let batch = unsafe { &mut *(batch_handle as *mut Vec<Vec<Value>>) };
    let params = unsafe { Box::from_raw(params_handle as *mut Vec<Value>) };
    batch.push(*params);
    0
}

/// Frees a batch parameter list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sqlite_batch_params_free(batch_handle: i64) -> i64 {
    if batch_handle != 0 {
        drop(unsafe { Box::from_raw(batch_handle as *mut Vec<Vec<Value>>) });
    }
    0
}
