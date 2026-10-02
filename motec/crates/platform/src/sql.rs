//! The real machine's SQL databases: SQLite connections by id, one at a time per database.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use contracts::{PlatformError, PlatformErrorKind, PlatformRequest, PlatformResponse, SqlMode, SqlValue};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, ErrorCode};

const BUSY_WAIT: Duration = Duration::from_secs(30);

const FILE_LOCK_WAIT: Duration = Duration::from_secs(5);

struct State {
    conn: Connection,
    tx: i64,
}

struct Db {
    state: Mutex<State>,
    gate: Condvar,
}

pub(crate) struct SystemSql {
    dbs: Mutex<HashMap<i64, Arc<Db>>>,
    txs: Mutex<HashMap<i64, i64>>,
    next_id: AtomicI64,
    next_tx: AtomicI64,
}

fn failure(kind: PlatformErrorKind, message: impl Into<String>) -> PlatformError {
    PlatformError { kind, message: message.into() }
}

fn sql_error(e: rusqlite::Error) -> PlatformError {
    let kind = match &e {
        rusqlite::Error::SqliteFailure(f, _) => match (f.code, f.extended_code) {
            (ErrorCode::ConstraintViolation, 1555 | 2067) => PlatformErrorKind::AlreadyExists,
            (ErrorCode::ConstraintViolation, _) => PlatformErrorKind::InvalidData,
            (ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked, _) => PlatformErrorKind::TimedOut,
            (ErrorCode::CannotOpen | ErrorCode::NotFound, _) => PlatformErrorKind::NotFound,
            (ErrorCode::PermissionDenied | ErrorCode::ReadOnly | ErrorCode::AuthorizationForStatementDenied, _) => {
                PlatformErrorKind::PermissionDenied
            }
            (ErrorCode::Unknown | ErrorCode::TypeMismatch | ErrorCode::ApiMisuse | ErrorCode::NotADatabase, _) => PlatformErrorKind::InvalidData,
            _ => PlatformErrorKind::Other,
        },
        rusqlite::Error::SqlInputError { .. }
        | rusqlite::Error::InvalidParameterCount(..)
        | rusqlite::Error::InvalidParameterName(_)
        | rusqlite::Error::MultipleStatement => PlatformErrorKind::InvalidData,
        _ => PlatformErrorKind::Other,
    };
    failure(kind, e.to_string())
}

fn to_value(v: SqlValue) -> Value {
    match v {
        SqlValue::Null => Value::Null,
        SqlValue::Int(n) => Value::Integer(n),
        SqlValue::Float(bits) => Value::Real(f64::from_bits(bits)),
        SqlValue::Text(s) => Value::Text(s),
        SqlValue::Blob(b) => Value::Blob(b),
    }
}

fn from_ref(v: ValueRef<'_>) -> SqlValue {
    match v {
        ValueRef::Null => SqlValue::Null,
        ValueRef::Integer(n) => SqlValue::Int(n),
        ValueRef::Real(f) => SqlValue::Float(f.to_bits()),
        ValueRef::Text(t) => SqlValue::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
    }
}

impl SystemSql {
    pub(crate) fn new() -> Self {
        SystemSql { dbs: Mutex::new(HashMap::new()), txs: Mutex::new(HashMap::new()), next_id: AtomicI64::new(1), next_tx: AtomicI64::new(1) }
    }

    fn db(&self, id: i64) -> Result<Arc<Db>, PlatformError> {
        self.dbs.lock().unwrap().get(&id).cloned().ok_or_else(|| failure(PlatformErrorKind::Other, "database is closed"))
    }

    fn open(&self, path: &str) -> Result<i64, PlatformError> {
        let conn = if path == ":memory:" { Connection::open_in_memory() } else { Connection::open(path) }.map_err(sql_error)?;
        conn.busy_timeout(FILE_LOCK_WAIT).map_err(sql_error)?;
        conn.execute_batch("PRAGMA foreign_keys = ON").map_err(sql_error)?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.dbs.lock().unwrap().insert(id, Arc::new(Db { state: Mutex::new(State { conn, tx: 0 }), gate: Condvar::new() }));
        Ok(id)
    }

    fn enter<'a>(&self, db: &'a Db, tx: i64) -> Result<std::sync::MutexGuard<'a, State>, PlatformError> {
        let mut st = db.state.lock().unwrap();
        if tx != 0 {
            if st.tx != tx {
                return Err(failure(PlatformErrorKind::Other, "the transaction is finished"));
            }
            return Ok(st);
        }
        let start = Instant::now();
        while st.tx != 0 {
            let left = BUSY_WAIT.checked_sub(start.elapsed()).filter(|d| !d.is_zero());
            let Some(left) = left else {
                return Err(failure(PlatformErrorKind::TimedOut, "database is busy: a transaction is open"));
            };
            st = db.gate.wait_timeout(st, left).unwrap().0;
        }
        Ok(st)
    }

    fn run(&self, id: i64, tx: i64, sql: &str, params: Vec<SqlValue>, mode: SqlMode) -> Result<PlatformResponse, PlatformError> {
        let db = self.db(id)?;
        let st = self.enter(&db, tx)?;
        let mut stmt = st.conn.prepare_cached(sql).map_err(sql_error)?;
        let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_string).collect();
        match mode {
            SqlMode::Check => {
                return Ok(PlatformResponse::Sql { changed: stmt.parameter_count() as i64, last_id: 0, columns, rows: Vec::new() });
            }
            SqlMode::Execute => {
                let params = rusqlite::params_from_iter(params.into_iter().map(to_value));
                let changed = stmt.execute(params).map_err(sql_error)? as i64;
                return Ok(PlatformResponse::Sql { changed, last_id: st.conn.last_insert_rowid(), columns: Vec::new(), rows: Vec::new() });
            }
            SqlMode::Query => {}
        }
        let params = rusqlite::params_from_iter(params.into_iter().map(to_value));
        let mut out = Vec::new();
        let mut cursor = stmt.query(params).map_err(sql_error)?;
        while let Some(row) = cursor.next().map_err(sql_error)? {
            out.push((0..columns.len()).map(|i| row.get_ref(i).map(from_ref)).collect::<Result<Vec<_>, _>>().map_err(sql_error)?);
        }
        Ok(PlatformResponse::Sql { changed: 0, last_id: st.conn.last_insert_rowid(), columns, rows: out })
    }

    fn begin(&self, id: i64) -> Result<i64, PlatformError> {
        let db = self.db(id)?;
        let mut st = self.enter(&db, 0)?;
        st.conn.execute_batch("BEGIN").map_err(sql_error)?;
        let token = self.next_tx.fetch_add(1, Ordering::SeqCst);
        st.tx = token;
        self.txs.lock().unwrap().insert(token, id);
        Ok(token)
    }

    fn end(&self, tx: i64, commit: bool) -> Result<(), PlatformError> {
        let Some(id) = self.txs.lock().unwrap().remove(&tx) else {
            return if commit { Err(failure(PlatformErrorKind::Other, "the transaction is finished")) } else { Ok(()) };
        };
        let Ok(db) = self.db(id) else { return Ok(()) };
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

    pub(crate) fn request(&self, request: PlatformRequest) -> Result<PlatformResponse, PlatformError> {
        Ok(match request {
            PlatformRequest::SqlOpen { path } => PlatformResponse::Int(self.open(&path)?),
            PlatformRequest::SqlRun { id, tx, sql, params, mode } => return self.run(id, tx, &sql, params, mode),
            PlatformRequest::SqlBegin { id } => PlatformResponse::Int(self.begin(id)?),
            PlatformRequest::SqlEnd { tx, commit } => {
                self.end(tx, commit)?;
                PlatformResponse::Unit
            }
            PlatformRequest::SqlClose { id } => {
                self.dbs.lock().unwrap().remove(&id).ok_or_else(|| failure(PlatformErrorKind::Other, "database is closed"))?;
                PlatformResponse::Unit
            }
            _ => unreachable!("not a SQL request"),
        })
    }
}

pub(crate) fn is_sql_request(request: &PlatformRequest) -> bool {
    matches!(
        request,
        PlatformRequest::SqlOpen { .. }
            | PlatformRequest::SqlRun { .. }
            | PlatformRequest::SqlBegin { .. }
            | PlatformRequest::SqlEnd { .. }
            | PlatformRequest::SqlClose { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(sql: &SystemSql) -> i64 {
        let PlatformResponse::Int(id) = sql.request(PlatformRequest::SqlOpen { path: ":memory:".into() }).unwrap() else { panic!() };
        id
    }

    fn run(sql: &SystemSql, id: i64, tx: i64, text: &str, params: Vec<SqlValue>, mode: SqlMode) -> Result<PlatformResponse, PlatformError> {
        sql.request(PlatformRequest::SqlRun { id, tx, sql: text.into(), params, mode })
    }

    #[test]
    fn values_round_trip_through_a_table() {
        let sql = SystemSql::new();
        let id = open(&sql);
        run(&sql, id, 0, "CREATE TABLE t (a INTEGER, b REAL, c TEXT, d BLOB, e)", vec![], SqlMode::Execute).unwrap();
        let params = vec![SqlValue::Int(7), SqlValue::Float(2.5f64.to_bits()), SqlValue::Text("hi".into()), SqlValue::Blob(vec![1, 2]), SqlValue::Null];
        let PlatformResponse::Sql { changed, last_id, .. } = run(&sql, id, 0, "INSERT INTO t VALUES (?, ?, ?, ?, ?)", params.clone(), SqlMode::Execute).unwrap() else { panic!() };
        assert_eq!((changed, last_id), (1, 1));
        let PlatformResponse::Sql { columns, rows, .. } = run(&sql, id, 0, "SELECT * FROM t", vec![], SqlMode::Query).unwrap() else { panic!() };
        assert_eq!(columns, ["a", "b", "c", "d", "e"]);
        assert_eq!(rows, vec![params]);
    }

    #[test]
    fn a_transaction_holds_the_database_until_it_ends() {
        let sql = Arc::new(SystemSql::new());
        let id = open(&sql);
        run(&sql, id, 0, "CREATE TABLE t (a INTEGER)", vec![], SqlMode::Execute).unwrap();
        let PlatformResponse::Int(tx) = sql.request(PlatformRequest::SqlBegin { id }).unwrap() else { panic!() };
        run(&sql, id, tx, "INSERT INTO t VALUES (1)", vec![], SqlMode::Execute).unwrap();
        let other = sql.clone();
        let waiting = std::thread::spawn(move || run(&other, id, 0, "SELECT COUNT(*) FROM t", vec![], SqlMode::Query));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!waiting.is_finished());
        sql.request(PlatformRequest::SqlEnd { tx, commit: false }).unwrap();
        let PlatformResponse::Sql { rows, .. } = waiting.join().unwrap().unwrap() else { panic!() };
        assert_eq!(rows, vec![vec![SqlValue::Int(0)]]);
        assert!(run(&sql, id, tx, "SELECT 1", vec![], SqlMode::Query).is_err());
    }

    #[test]
    fn errors_carry_a_kind() {
        let sql = SystemSql::new();
        let id = open(&sql);
        run(&sql, id, 0, "CREATE TABLE t (a INTEGER PRIMARY KEY)", vec![], SqlMode::Execute).unwrap();
        run(&sql, id, 0, "INSERT INTO t VALUES (1)", vec![], SqlMode::Execute).unwrap();
        assert_eq!(run(&sql, id, 0, "INSERT INTO t VALUES (1)", vec![], SqlMode::Execute).unwrap_err().kind, PlatformErrorKind::AlreadyExists);
        assert_eq!(run(&sql, id, 0, "SELEKT 1", vec![], SqlMode::Query).unwrap_err().kind, PlatformErrorKind::InvalidData);
        assert_eq!(run(&sql, id, 0, "SELECT ?", vec![], SqlMode::Query).unwrap_err().kind, PlatformErrorKind::InvalidData);
    }
}
