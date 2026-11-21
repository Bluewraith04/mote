# sqlite: reference

SQLite databases: open, statements with bound parameters, rows, transactions. Native: a Rust crate (bundled SQLite via `rusqlite`).

`import { Database, PreparedStatement, Row, SqlValue, Transaction, TransactionMode, Value, query_as, query_row_as, row_as, stmt_query_as, stmt_query_row_as, stmt_stream_as, stream_as } from sqlite`

## Database

`Database` is `Closeable`; using it after `close()` is a fault.

| Method | Notes |
|---|---|
| `Database.open(path) -> Result<Database, Error>` | makes the file when missing; `":memory:"` is a private in-memory database |
| `Database.memory()` | the same as `open(":memory:")` |
| `.execute_batch(sql)` | statements separated by `;`, no parameters (schemas) |
| `.execute(sql, params = []) -> Result<Int, Error>` | one statement, `?` bound in order; rows changed |
| `.prepare(sql) -> Result<PreparedStatement, Error>` | compiles a statement for repeated or batch execution; `Closeable` |
| `.query(sql, params = []) -> Result<List<Row>, Error>` | all rows |
| `.query_row(sql, params = []) -> Result<Row?, Error>` | the first row, null for none |
| `.stream(sql, params = []) -> Result<Stream<Row>, Error>` | lazily yields rows one by one without full-result buffering |
| `.last_insert_id() -> Int` | rowid of the last insert through this database |
| `.interrupt()` | interrupts any executing query from any task or thread |
| `.begin()` `.commit()` `.rollback()` | |
| `.begin_transaction(mode = TransactionMode.Deferred) -> Result<Transaction, Error>` | returns a `Closeable` `Transaction` that commits explicitly and rolls back automatically on uncommitted `close()` |
| `.transaction(body: (Database) -> Result<T, Error>) -> Result<T, Error>` | commits on `Ok`, rolls back on `Err` |
| `.close()` | |

## PreparedStatement

`PreparedStatement` is `Closeable`; closing it frees the compiled statement.

| Method | Notes |
|---|---|
| `.execute(params = []) -> Result<Int, Error>` | runs with parameters; rows changed |
| `.query(params = []) -> Result<List<Row>, Error>` | all rows |
| `.query_row(params = []) -> Result<Row?, Error>` | first row, null for none |
| `.stream(params = []) -> Stream<Row>` | lazily yields rows one by one without full-result buffering |
| `.execute_batch_params(param_rows) -> Result<Int, Error>` | runs multiple parameter rows in a single native pass |
| `.close()` | |

## Typed mapping (Reflection)

Top-level generic functions decode SQLite rows directly into Mote `struct` or `class` types using `std.meta`:

| Function | Notes |
|---|---|
| `query_as<T>(db_or_tx, sql, params = []) -> Result<List<T>, Error>` | queries database or transaction, decodes all rows into `List<T>` |
| `query_row_as<T>(db_or_tx, sql, params = []) -> Result<T?, Error>` | queries database or transaction, decodes the first row into `T?` |
| `stream_as<T>(db_or_tx, sql, params = []) -> Result<Stream<T>, Error>` | streams database or transaction query, decodes each row into struct `T` lazily |
| `stmt_query_as<T>(stmt, params = []) -> Result<List<T>, Error>` | queries prepared statement, decodes all rows into `List<T>` |
| `stmt_query_row_as<T>(stmt, params = []) -> Result<T?, Error>` | queries prepared statement, decodes first row into `T?` |
| `stmt_stream_as<T>(stmt, params = []) -> Stream<T>` | streams prepared statement, decodes each row into struct `T` lazily |
| `row_as<T>(row) -> Result<T, Error>` | maps a single `Row` to struct `T` by field name |

## Values and rows

| Type | |
|---|---|
| `SqlValue` | `Value | Int | Float | String | [UInt8] | Bool | Null`; parameters accept primitive literals directly |
| `Value` | `Null`, `Integer(Int)`, `Real(Float)`, `Text(String)`, `Blob([UInt8])`; `.is_null()`, `.as_int()`, `.as_float()` (an integer too), `.as_text()`, `.as_blob()` each give an optional |
| `Row` | `.len()`, `.columns() -> List<String>`, `.get(index) -> Value`, `.named(name) -> Value?` (first column with that name) |

A parameter is an `SqlValue`: primitive literals (`"ada"`, `36`, `9.5`, `true`, `photo: [UInt8]`, `null`) or an explicit `Value`. SQL errors are an `Err` with SQLite's message. Statements with a trailing `;` and more text in `execute` use only the first; use `execute_batch` for scripts.

```mote
import { Database } from sqlite

fn main() -> Result<Null, Error> {
    let db = Database.memory()?
    db.execute_batch("CREATE TABLE people (name TEXT, age INTEGER)")?
    db.execute("INSERT INTO people VALUES (?, ?)", ["ada", 36])?
    for row in db.query("SELECT name, age FROM people")? {
        println("${row.get(0).as_text()!} ${row.named("age")!.as_int()!}")
    }
    db.close()?
    return Ok(null)
}
```

Tests: `tools/test-core.sh sqlite`.
