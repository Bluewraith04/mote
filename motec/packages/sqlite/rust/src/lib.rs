//! SQLite databases behind the bytes-in, bytes-out C convention of `std.dev.libtools`.
//!
//! A database is an id the program holds; its connection lives here. `mote_sqlite_run` keeps its rows under a ticket until `mote_sqlite_take` fetches them, so a statement is never run twice.
//! Cells cross as lines: `n`, `i<int>`, `f<float>`, then `s<len>` and `b<len>` each followed by that many bytes.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::slice;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, ErrorCode};

const BUSY_WAIT: Duration = Duration::from_secs(30);

const FILE_LOCK_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct Failure {
    kind: &'static str,
    message: String,
}

fn failure(kind: &'static str, message: impl Into<String>) -> Failure {
    Failure { kind, message: message.into() }
}

fn invalid(message: &str) -> Failure {
    failure("InvalidData", message)
}

fn sql_error(e: rusqlite::Error) -> Failure {
    let kind = match &e {
        rusqlite::Error::SqliteFailure(f, _) => match (f.code, f.extended_code) {
            (ErrorCode::ConstraintViolation, 1555 | 2067) => "AlreadyExists",
            (ErrorCode::ConstraintViolation, _) => "InvalidData",
            (ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked, _) => "TimedOut",
            (ErrorCode::CannotOpen | ErrorCode::NotFound, _) => "NotFound",
            (ErrorCode::PermissionDenied | ErrorCode::ReadOnly | ErrorCode::AuthorizationForStatementDenied, _) => "PermissionDenied",
            (ErrorCode::Unknown | ErrorCode::TypeMismatch | ErrorCode::ApiMisuse | ErrorCode::NotADatabase, _) => "InvalidData",
            _ => "Other",
        },
        rusqlite::Error::SqlInputError { .. }
        | rusqlite::Error::InvalidParameterCount(..)
        | rusqlite::Error::InvalidParameterName(_)
        | rusqlite::Error::MultipleStatement => "InvalidData",
        _ => "Other",
    };
    failure(kind, e.to_string())
}

struct State {
    conn: Connection,
    tx: i64,
}

struct Db {
    state: Mutex<State>,
    gate: Condvar,
}

#[derive(Default)]
struct Registry {
    dbs: HashMap<i64, Arc<Db>>,
    txs: HashMap<i64, i64>,
    pending: HashMap<i64, Vec<u8>>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

static NEXT: AtomicI64 = AtomicI64::new(1);

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn line(&mut self) -> Result<&'a [u8], Failure> {
        let rest = &self.bytes[self.at..];
        let end = rest.iter().position(|b| *b == b'\n').ok_or_else(|| invalid("the request is cut short"))?;
        self.at += end + 1;
        Ok(&rest[..end])
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Failure> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len()).ok_or_else(|| invalid("the request is cut short"))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }
}

fn number<T: std::str::FromStr>(text: &[u8]) -> Result<T, Failure> {
    std::str::from_utf8(text).ok().and_then(|t| t.trim().parse().ok()).ok_or_else(|| invalid("a number in the request is malformed"))
}

fn read_cell(r: &mut Reader) -> Result<Value, Failure> {
    let line = r.line()?;
    let (tag, rest) = line.split_first().ok_or_else(|| invalid("a cell is empty"))?;
    Ok(match tag {
        b'n' => Value::Null,
        b'i' => Value::Integer(number(rest)?),
        b'f' => Value::Real(number(rest)?),
        b's' => Value::Text(String::from_utf8(r.take(number(rest)?)?.to_vec()).map_err(|_| invalid("a text cell is not UTF-8"))?),
        b'b' => Value::Blob(r.take(number(rest)?)?.to_vec()),
        _ => return Err(invalid("a cell has an unknown type")),
    })
}

fn write_text(out: &mut Vec<u8>, tag: char, bytes: &[u8]) {
    out.extend_from_slice(format!("{tag}{}\n", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
}

fn write_cell(out: &mut Vec<u8>, v: ValueRef<'_>) {
    match v {
        ValueRef::Null => out.extend_from_slice(b"n\n"),
        ValueRef::Integer(n) => out.extend_from_slice(format!("i{n}\n").as_bytes()),
        ValueRef::Real(f) => out.extend_from_slice(format!("f{f:?}\n").as_bytes()),
        ValueRef::Text(t) => write_text(out, 's', String::from_utf8_lossy(t).as_bytes()),
        ValueRef::Blob(b) => write_text(out, 'b', b),
    }
}

fn db(id: i64) -> Result<Arc<Db>, Failure> {
    REGISTRY.lock().unwrap().dbs.get(&id).cloned().ok_or_else(|| failure("Other", "database is closed"))
}

fn open(path: &str) -> Result<i64, Failure> {
    let conn = if path == ":memory:" { Connection::open_in_memory() } else { Connection::open(path) }.map_err(sql_error)?;
    conn.busy_timeout(FILE_LOCK_WAIT).map_err(sql_error)?;
    conn.execute_batch("PRAGMA foreign_keys = ON").map_err(sql_error)?;
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    REGISTRY.lock().unwrap().dbs.insert(id, Arc::new(Db { state: Mutex::new(State { conn, tx: 0 }), gate: Condvar::new() }));
    Ok(id)
}

fn enter(db: &Db, tx: i64) -> Result<std::sync::MutexGuard<'_, State>, Failure> {
    let mut st = db.state.lock().unwrap();
    if tx != 0 {
        if st.tx != tx {
            return Err(failure("Other", "the transaction is finished"));
        }
        return Ok(st);
    }
    let start = Instant::now();
    while st.tx != 0 {
        let Some(left) = BUSY_WAIT.checked_sub(start.elapsed()).filter(|d| !d.is_zero()) else {
            return Err(failure("TimedOut", "database is busy: a transaction is open"));
        };
        st = db.gate.wait_timeout(st, left).unwrap().0;
    }
    Ok(st)
}

/// Runs the statement and answers `<changed> <last id> <columns>\n`, the column names, then the rows.
fn run(input: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut r = Reader { bytes: input, at: 0 };
    let head: Vec<i64> = std::str::from_utf8(r.line()?).map_err(|_| invalid("the request is malformed"))?.split(' ').map(|n| number(n.as_bytes())).collect::<Result<_, _>>()?;
    let [id, tx, mode] = head[..] else { return Err(invalid("the request is malformed")) };
    let sql_len = number(r.line()?)?;
    let sql = String::from_utf8(r.take(sql_len)?.to_vec()).map_err(|_| invalid("the statement is not UTF-8"))?;
    let mut params = Vec::new();
    while !r.done() {
        params.push(read_cell(&mut r)?);
    }
    let db = db(id)?;
    let st = enter(&db, tx)?;
    let mut stmt = st.conn.prepare_cached(&sql).map_err(sql_error)?;
    let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
    let mut out = Vec::new();
    let (changed, last_id) = match mode {
        2 => (stmt.parameter_count() as i64, 0),
        0 => {
            let changed = stmt.execute(rusqlite::params_from_iter(params)).map_err(sql_error)? as i64;
            (changed, st.conn.last_insert_rowid())
        }
        _ => {
            let mut cursor = stmt.query(rusqlite::params_from_iter(params)).map_err(sql_error)?;
            while let Some(row) = cursor.next().map_err(sql_error)? {
                for i in 0..columns.len() {
                    write_cell(&mut out, row.get_ref(i).map_err(sql_error)?);
                }
            }
            (0, st.conn.last_insert_rowid())
        }
    };
    let shown = if mode == 0 { 0 } else { columns.len() };
    let mut answer = format!("{changed} {last_id} {shown}\n").into_bytes();
    if mode != 0 {
        for name in &columns {
            write_text(&mut answer, 's', name.as_bytes());
        }
    }
    answer.extend_from_slice(&out);
    let ticket = NEXT.fetch_add(1, Ordering::SeqCst);
    let size = answer.len();
    REGISTRY.lock().unwrap().pending.insert(ticket, answer);
    Ok(format!("{ticket} {size}\n").into_bytes())
}

fn begin(id: i64) -> Result<i64, Failure> {
    let db = db(id)?;
    let mut st = enter(&db, 0)?;
    st.conn.execute_batch("BEGIN").map_err(sql_error)?;
    let token = NEXT.fetch_add(1, Ordering::SeqCst);
    st.tx = token;
    REGISTRY.lock().unwrap().txs.insert(token, id);
    Ok(token)
}

fn end(tx: i64, commit: bool) -> Result<(), Failure> {
    let Some(id) = REGISTRY.lock().unwrap().txs.remove(&tx) else {
        return if commit { Err(failure("Other", "the transaction is finished")) } else { Ok(()) };
    };
    let Ok(db) = db(id) else { return Ok(()) };
    let mut st = db.state.lock().unwrap();
    if st.tx != tx {
        return Ok(());
    }
    let result = if commit { st.conn.execute_batch("COMMIT") } else { st.conn.execute_batch("ROLLBACK") };
    if result.is_err() {
        let _ = st.conn.execute_batch("ROLLBACK");
    }
    st.tx = 0;
    db.gate.notify_all();
    result.map_err(sql_error)
}

fn close(id: i64) -> Result<(), Failure> {
    let mut registry = REGISTRY.lock().unwrap();
    registry.dbs.remove(&id).ok_or_else(|| failure("Other", "database is closed"))?;
    registry.txs.retain(|_, db| *db != id);
    Ok(())
}

/// Copies `data` to `out` when it fits and answers its length; a failure answers the negated length of `<kind> <message>`, written to `out` (cut to `cap`).
fn finish(answer: Result<Vec<u8>, Failure>, out: *mut u8, cap: i64) -> i64 {
    let (data, sign) = match answer {
        Ok(data) => (data, 1),
        Err(f) => (format!("{} {}", f.kind, f.message).into_bytes(), -1),
    };
    if (sign > 0 && data.len() as i64 <= cap || sign < 0) && cap > 0 && !out.is_null() {
        let n = data.len().min(cap as usize);
        // SAFETY: the caller promises `out` is writable for `cap` bytes and `n <= cap`.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, n) };
    }
    sign * data.len() as i64
}

fn call(input: *const u8, len: i64, out: *mut u8, cap: i64, work: impl FnOnce(&[u8]) -> Result<Vec<u8>, Failure>) -> i64 {
    // SAFETY: the caller promises `input` is readable for `len` bytes.
    let bytes: &[u8] = if len <= 0 || input.is_null() { &[] } else { unsafe { slice::from_raw_parts(input, len as usize) } };
    let answer = catch_unwind(AssertUnwindSafe(|| work(bytes))).unwrap_or_else(|_| Err(failure("Other", "the library failed")));
    finish(answer, out, cap)
}

fn ints(bytes: &[u8]) -> Result<Vec<i64>, Failure> {
    std::str::from_utf8(bytes).map_err(|_| invalid("the request is malformed"))?.split_whitespace().map(|n| number(n.as_bytes())).collect()
}

fn one_int(bytes: &[u8]) -> Result<i64, Failure> {
    match ints(bytes)?[..] {
        [n] => Ok(n),
        _ => Err(invalid("the request is malformed")),
    }
}

/// The path in, the new database's id out.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_sqlite_open(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, |b| {
        let path = std::str::from_utf8(b).map_err(|_| invalid("the path is not UTF-8"))?;
        Ok(open(path)?.to_string().into_bytes())
    })
}

/// A database id in; the database is closed, and a transaction still open on it is dropped.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_sqlite_close(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, |b| close(one_int(b)?).map(|()| Vec::new()))
}

/// A statement in, `<ticket> <size>\n` out: `mote_sqlite_take` fetches the answer.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_sqlite_run(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, run)
}

/// A database id in, the new transaction's token out; waits while another transaction is open.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_sqlite_begin(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, |b| Ok(begin(one_int(b)?)?.to_string().into_bytes()))
}

/// `<token> <1 to commit, 0 to roll back>` in.
///
/// # Safety
/// `input` must be readable for `len` bytes and `out` writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_sqlite_end(input: *const u8, len: i64, out: *mut u8, cap: i64) -> i64 {
    call(input, len, out, cap, |b| match ints(b)?[..] {
        [tx, commit] => end(tx, commit != 0).map(|()| Vec::new()),
        _ => Err(invalid("the request is malformed")),
    })
}

/// Writes the answer kept under `ticket` to `out`, which must hold the `<size>` that `mote_sqlite_run` gave.
///
/// # Safety
/// `out` must be writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mote_sqlite_take(ticket: i64, out: *mut u8, cap: i64) -> i64 {
    let kept = REGISTRY.lock().unwrap().pending.remove(&ticket);
    finish(kept.ok_or_else(|| failure("Other", "the answer was already taken")), out, cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn request(db: i64, tx: i64, mode: i64, sql: &str, params: &[u8]) -> Vec<u8> {
        let mut out = format!("{db} {tx} {mode}\n{}\n{sql}", sql.len()).into_bytes();
        out.extend_from_slice(params);
        out
    }

    fn answer(db: i64, tx: i64, mode: i64, sql: &str, params: &[u8]) -> Result<Vec<u8>, Failure> {
        let head = String::from_utf8(run(&request(db, tx, mode, sql, params))?).unwrap();
        let ticket: i64 = head.split(' ').next().unwrap().parse().unwrap();
        Ok(REGISTRY.lock().unwrap().pending.remove(&ticket).unwrap())
    }

    #[test]
    fn every_kind_of_cell_round_trips() {
        let id = open(":memory:").unwrap();
        answer(id, 0, 0, "CREATE TABLE t (a INTEGER, b REAL, c TEXT, d BLOB, e)", b"").ok().unwrap();
        let params = b"i7\nf2.5\ns2\nhib2\n\x01\xffn\n";
        let put = answer(id, 0, 0, "INSERT INTO t VALUES (?, ?, ?, ?, ?)", params).ok().unwrap();
        assert_eq!(put, b"1 1 0\n");
        let got = answer(id, 0, 1, "SELECT * FROM t", b"").ok().unwrap();
        let mut expect = b"0 1 5\ns1\nas1\nbs1\ncs1\nds1\ne".to_vec();
        expect.extend_from_slice(b"i7\nf2.5\ns2\nhib2\n\x01\xffn\n");
        assert_eq!(got, expect);
    }

    #[test]
    fn non_finite_floats_cross_as_text() {
        let id = open(":memory:").unwrap();
        let got = answer(id, 0, 1, "SELECT ?", b"finf\n").ok().unwrap();
        assert!(got.ends_with(b"finf\n"));
    }

    #[test]
    fn a_transaction_holds_the_database_until_it_ends() {
        let id = open(":memory:").unwrap();
        answer(id, 0, 0, "CREATE TABLE t (a INTEGER)", b"").ok().unwrap();
        let tx = begin(id).unwrap();
        answer(id, tx, 0, "INSERT INTO t VALUES (1)", b"").ok().unwrap();
        let waiting = std::thread::spawn(move || answer(id, 0, 1, "SELECT COUNT(*) FROM t", b"").ok().unwrap());
        std::thread::sleep(Duration::from_millis(100));
        assert!(!waiting.is_finished());
        end(tx, false).ok().unwrap();
        assert!(waiting.join().unwrap().ends_with(b"i0\n"));
        assert!(answer(id, tx, 1, "SELECT 1", b"").is_err());
    }

    #[test]
    fn errors_carry_a_kind() {
        let id = open(":memory:").unwrap();
        answer(id, 0, 0, "CREATE TABLE t (a INTEGER PRIMARY KEY)", b"").ok().unwrap();
        answer(id, 0, 0, "INSERT INTO t VALUES (1)", b"").ok().unwrap();
        assert_eq!(answer(id, 0, 0, "INSERT INTO t VALUES (1)", b"").err().unwrap().kind, "AlreadyExists");
        assert_eq!(answer(id, 0, 1, "SELEKT 1", b"").err().unwrap().kind, "InvalidData");
        assert_eq!(answer(id, 0, 1, "SELECT ?", b"").err().unwrap().kind, "InvalidData");
    }
}
