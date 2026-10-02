# std.data.sql

SQLite, over the `rusqlite` crate with SQLite built in. `import std.data.sql as sql`. A call runs on a helper thread while its task waits; one connection serves every task, one call at a time.

| Function | Answers |
|---|---|
| `open(path)` | `Result<Database, Error>`; creates the file if missing |
| `memory()` | `Result<Database, Error>`; gone when it closes |

| Database call | Answers |
|---|---|
| `db.execute(sql, params)` | `Result<Changes, Error>`; `Changes` has `changed` and `last_id` |
| `db.query(sql, params)` | `Result<List<Row>, Error>` |
| `db.prepare(sql)` | `Result<Statement, Error>`, checked now and run many times |
| `db.begin()` | `Result<Transaction, Error>` |
| `db.close()` | closes now; an unreferenced database closes at the next collection |

`params` is a list for the `?` marks, and may be left out. A value is an `Int`, `Float`, `String`, `Bytes`, or `None` for NULL. Foreign keys are enforced.

```mote
import std.data.sql as sql

fn main() {
    let db = sql.memory()!
    let made = db.execute("CREATE TABLE pets (id INTEGER PRIMARY KEY, name TEXT NOT NULL, weight REAL)")!
    let add = db.prepare("INSERT INTO pets (name, weight) VALUES (?, ?)")!
    let a = add.execute(["Mote", 4.5])!
    let b = add.execute(["Dust", None])!
    println(b.last_id)
    for row in db.query("SELECT name, weight FROM pets ORDER BY id")! {
        let name = row.text("name")!
        let missing = row.is_null("weight")!
        println("${name} ${missing}")
    }
}
```

```output
2
Mote false
Dust true
```

| Row member | Answers |
|---|---|
| `columns()` | the column names |
| `get(name)`, `at(i)` | `Result<Cell?, Error>`; `None` is NULL |
| `int`, `text`, `bytes`, `float` (name) | the cell as that type; `Err` for NULL or another type; `float` converts an `Int` |
| `is_null(name)` | `Result<Bool, Error>` |

SQLite types values, not columns, so any column that is not `NOT NULL` can hold NULL.

## Transactions

`t.execute`, `t.query` and `t.prepare` work as on the database; `t.commit()` keeps the changes, `t.rollback()` undoes them, and `t.close()` rolls back if still open. An open transaction holds the database: other calls wait up to 30 seconds, then fail with a timeout. `with t = db.begin()! { … t.commit() }` rolls back when the block leaves without a commit.

## Errors

| Cause | Kind |
|---|---|
| UNIQUE or PRIMARY KEY conflict | `AlreadyExists` |
| other constraint, bad SQL, wrong parameter count | `InvalidData` |
| a file lock held over 5 seconds, a transaction open past 30 | `TimedOut` |
| a file that cannot be opened | `NotFound` |
| a closed database, a finished transaction | `Other` |

Not included: streaming rows, named parameters, several statements in one call, extensions, a pool, migrations.
