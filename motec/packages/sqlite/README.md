# sqlite

SQLite databases, over the `rusqlite` crate with SQLite built in. The package carries the crate as a library in `native/<triple>/`, so add it with native access granted, then `import sqlite as sql`.

```toml
[dependencies]
sqlite = { git = "github:user/sqlite", tag = "v0.1.0", native = true }
```

Libraries are built for `x86_64-unknown-linux-gnu` here. To build one for another machine: `cargo build --release --manifest-path rust/Cargo.toml`, then copy `rust/target/release/libmote_sqlite.so` (`.dylib`, `mote_sqlite.dll`) into `native/<triple>/`. The functions are in the bytes-in, bytes-out shape of `std.dev.libtools`; a statement's rows stay in the library under a ticket until `mote_sqlite_take` fetches them, so a statement never runs twice.

A call runs on a helper thread while its task waits, so other tasks keep running. One connection serves every task, one call at a time. A `Database` holds only an id; the connection lives in the library.

## Opening

| Function | Answers |
|---|---|
| `open(path)` | `Result<Database, Error>`; the file is created if it is missing |
| `memory()` | `Result<Database, Error>`; a new database in memory, gone when it closes |

Foreign keys are enforced. Nothing closes a database for you: call `db.close()`, or open it with `with db = sql.open(path)! { ... }`, which closes it when the block ends. One that is never closed stays open until the program exits.

## Running statements

| Call | Answers |
|---|---|
| `db.execute(sql, params)` | `Result<Changes, Error>` for a statement that returns no rows; `Changes` has `changed` (rows) and `last_id` (the row id of the last insert) |
| `db.query(sql, params)` | `Result<List<Row>, Error>`, every row the statement returns |
| `db.prepare(sql)` | `Result<Statement, Error>`: checked now, run many times |

`params` is a list for the `?` marks in order and can be left out. A value is a `Cell` (an `Int`, `Float`, `String` or `Bytes`) or `None` for SQL NULL.

```mote,skip
import sqlite as sql

fn main() {
    let db = sql.memory().unwrap()
    let made = db.execute("CREATE TABLE pets (id INTEGER PRIMARY KEY, name TEXT NOT NULL, weight REAL)").unwrap()
    let add = db.prepare("INSERT INTO pets (name, weight) VALUES (?, ?)").unwrap()
    let a = add.execute(["Mote", 4.5]).unwrap()
    let b = add.execute(["Dust", None]).unwrap()
    println(b.last_id)
    for row in db.query("SELECT name, weight FROM pets ORDER BY id").unwrap() {
        let name = row.text("name").unwrap()
        let missing = row.is_null("weight").unwrap()
        println("${name} ${missing}")
    }
}
```

```output
2
Mote false
Dust true
```

## Rows

| Member | Answers |
|---|---|
| `columns()` | the column names |
| `get(name)`, `at(i)` | `Result<Cell?, Error>`; `None` is NULL |
| `int(name)`, `text(name)`, `bytes(name)` | the cell as that type; an `Err` for NULL or another type |
| `float(name)` | a `Float`; an `Int` converts |
| `is_null(name)` | `Result<Bool, Error>` |

SQLite keeps the type of each value, not of each column, so a `REAL` column can hold an `Int` and a column can hold NULL anywhere it is not `NOT NULL`. A column that does not exist is an `Err`.

## Statements

`prepare` checks the SQL at once, so a mistake shows there. `param_count()` is the number of `?` marks and `columns()` the names of the columns it returns. `execute(params)` and `query(params)` run it; the connection keeps the prepared form, and `db.execute` and `db.query` reuse it the same way.

## Transactions

| Call | Does |
|---|---|
| `db.begin()` | `Result<Transaction, Error>`; waits while another task's transaction is open |
| `t.execute`, `t.query`, `t.prepare` | as on the database, inside the transaction |
| `t.commit()` | makes the changes permanent |
| `t.rollback()` | undoes them |
| `t.close()` | rolls back if the transaction is still open |

A transaction left open holds the database: every other call waits until it ends, up to 30 seconds, then fails with a timeout. Nothing rolls one back for you, so close it on every path; `with t = db.begin()! { ... t.commit() }` rolls back when the block leaves without a commit. Use the transaction's own methods inside it; a call on the database from the same task waits for the transaction it is inside.

## Errors

| Cause | Kind |
|---|---|
| a UNIQUE or PRIMARY KEY conflict | `AlreadyExists` |
| another constraint (NOT NULL, CHECK, FOREIGN KEY), bad SQL, a wrong parameter count | `InvalidData` |
| a file lock another process holds for over 5 seconds, an open transaction past 30 seconds | `TimedOut` |
| a file that cannot be opened | `NotFound` |
| a closed database, a finished transaction | `Other` |

The message is SQLite's own. Not included: streaming rows (a query answers all of them), named parameters, several statements in one call, blobs opened as streams, extensions, a connection pool, and migrations (a list of numbered statements run in a transaction does the job).
