//! The package `sqlite` over SQLite, run as real programs.

mod common;

use std::process::Command;

fn run(source: &str, tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("mote_sqlite_{}_{}", tag, std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    common::install_native_packages(&dir, &["sqlite"]);
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).current_dir(&dir).env("SQL_DIR", &dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().map(String::from).collect()
}

#[test]
fn every_kind_of_cell_round_trips() {
    let got = run(
        r#"import sqlite as sql

fn main() {
    let db = sql.memory().unwrap()
    let made = db.execute("CREATE TABLE t (a INTEGER, b REAL, c TEXT, d BLOB, e TEXT)").unwrap()
    println(made.changed)
    let blob = Bytes(3)
    blob.set(0, 1)
    blob.set(1, 2)
    blob.set(2, 255)
    let put = db.execute("INSERT INTO t VALUES (?, ?, ?, ?, ?)", [-7, 2.5, "héllo", blob, None]).unwrap()
    println("${put.changed} ${put.last_id}")
    let rows = db.query("SELECT * FROM t").unwrap()
    println(rows.len())
    let r = rows.get(0)
    println(r.columns().len())
    println(r.int("a").unwrap())
    println(r.float("b").unwrap())
    println(r.text("c").unwrap())
    println(r.bytes("d").unwrap().len())
    println(r.bytes("d").unwrap().get(2))
    println(r.is_null("e").unwrap())
    println(r.is_null("a").unwrap())
    println(r.float("a").unwrap())
}
"#,
        "cells",
    );
    assert_eq!(got, ["0", "1 1", "1", "5", "-7", "2.5", "héllo", "3", "255", "true", "false", "-7.0"]);
}

#[test]
fn a_prepared_statement_is_checked_once_and_run_many_times() {
    let got = run(
        r#"import sqlite as sql

fn main() {
    let db = sql.memory().unwrap()
    let made = db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").unwrap()
    let add = db.prepare("INSERT INTO t (name) VALUES (?)").unwrap()
    println(add.param_count())
    for n in ["a", "b", "c"] {
        let done = add.execute([n]).unwrap()
        println(done.last_id)
    }
    let find = db.prepare("SELECT id, name FROM t WHERE id >= ? ORDER BY id").unwrap()
    println(find.columns().len())
    println(find.query([2]).unwrap().len())
    println(find.query([4]).unwrap().len())
    match add.execute([]) {
        Ok(d) => { println("ran with no parameter") }
        Err(e) => { println("too few: ${e.message}") }
    }
    match db.prepare("SELEKT 1") {
        Ok(s) => { println("prepared nonsense") }
        Err(e) => { println("bad sql") }
    }
}
"#,
        "prepared",
    );
    assert_eq!(got[..6], ["1", "1", "2", "3", "2", "2"]);
    assert_eq!(got[6], "0");
    assert!(got[7].starts_with("too few: "), "{got:?}");
    assert_eq!(got[8], "bad sql");
}

#[test]
fn a_transaction_commits_or_rolls_back() {
    let got = run(
        r#"import sqlite as sql

fn count(db: sql.Database) -> Int {
    let rows = db.query("SELECT COUNT(*) AS n FROM t").unwrap()
    return rows.get(0).int("n").unwrap()
}

fn main() {
    let db = sql.memory().unwrap()
    let made = db.execute("CREATE TABLE t (a INTEGER)").unwrap()
    let t = db.begin().unwrap()
    let one = t.execute("INSERT INTO t VALUES (1)").unwrap()
    let seen = t.query("SELECT a FROM t").unwrap()
    println(seen.len())
    let ok = t.commit()
    println(count(db))
    let u = db.begin().unwrap()
    let two = u.execute("INSERT INTO t VALUES (2)").unwrap()
    let undone = u.rollback()
    println(count(db))
    with w = db.begin().unwrap() {
        let three = w.execute("INSERT INTO t VALUES (3)").unwrap()
    }
    println(count(db))
    with x = db.begin().unwrap() {
        let four = x.execute("INSERT INTO t VALUES (4)").unwrap()
        let kept = x.commit()
    }
    println(count(db))
    match t.execute("INSERT INTO t VALUES (5)") {
        Ok(d) => { println("ran after commit") }
        Err(e) => { println("finished: ${e.message}") }
    }
}
"#,
        "transactions",
    );
    assert_eq!(got, ["1", "1", "1", "1", "2", "finished: sql: the transaction is finished"]);
}

#[test]
fn failures_say_what_went_wrong() {
    let got = run(
        r#"import sqlite as sql

fn say(label: String, r: Result<sql.Changes, Error>) {
    match r {
        Ok(d) => { println("${label}: ok") }
        Err(e) => { println("${label}: ${e.message}") }
    }
}

fn main() {
    let db = sql.memory().unwrap()
    let a = db.execute("CREATE TABLE p (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap()
    let b = db.execute("CREATE TABLE c (pid INTEGER REFERENCES p(id))").unwrap()
    say("insert", db.execute("INSERT INTO p VALUES (1, ?)", ["x"]))
    say("duplicate", db.execute("INSERT INTO p VALUES (1, ?)", ["y"]))
    say("null", db.execute("INSERT INTO p VALUES (2, ?)", [None]))
    say("foreign", db.execute("INSERT INTO c VALUES (99)"))
    say("syntax", db.execute("SELEKT"))
    say("table", db.execute("DROP TABLE nope"))
    say("count", db.execute("INSERT INTO p VALUES (?, ?)", [3]))
    let rows = db.query("SELECT id, name FROM p").unwrap()
    let r = rows.get(0)
    match r.int("name") {
        Ok(n) => { println("name as int") }
        Err(e) => { println(e.message) }
    }
    match r.get("nope") {
        Ok(c) => { println("found") }
        Err(e) => { println(e.message) }
    }
    let closed = db.close()
    say("closed", db.execute("SELECT 1"))
}
"#,
        "failures",
    );
    assert_eq!(got[0], "insert: ok");
    assert!(got[1].starts_with("duplicate: ") && got[1].contains("UNIQUE"), "{got:?}");
    assert!(got[2].starts_with("null: ") && got[2].contains("NOT NULL"), "{got:?}");
    assert!(got[3].starts_with("foreign: ") && got[3].contains("FOREIGN KEY"), "{got:?}");
    assert!(got[4].starts_with("syntax: ") && got[4].contains("syntax error"), "{got:?}");
    assert!(got[5].starts_with("table: ") && got[5].contains("no such table"), "{got:?}");
    assert!(got[6].starts_with("count: "), "{got:?}");
    assert_eq!(got[7], "column name is not an Int");
    assert_eq!(got[8], "no column nope");
    assert!(got[9].starts_with("closed: ") && got[9].contains("closed"), "{got:?}");
}

#[test]
fn tasks_share_one_database() {
    let got = run(
        r#"import sqlite as sql
import std.task as task

fn main() {
    let db = sql.memory().unwrap()
    let made = db.execute("CREATE TABLE t (n INTEGER)").unwrap()
    scope {
        var i = 0
        while i < 50 {
            let n = i
            spawn {
                let t = db.begin().unwrap()
                let one = t.execute("INSERT INTO t VALUES (?)", [n]).unwrap()
                let two = t.execute("INSERT INTO t VALUES (?)", [n + 1000]).unwrap()
                let done = t.commit()
            }
            i = i + 1
        }
    }
    let rows = db.query("SELECT COUNT(*) AS n, SUM(n) AS total FROM t").unwrap()
    println(rows.get(0).int("n").unwrap())
    println(rows.get(0).int("total").unwrap())
}
"#,
        "tasks",
    );
    assert_eq!(got, ["100", "52450"]);
}

#[test]
fn a_file_database_keeps_its_rows() {
    let got = run(
        r#"import sqlite as sql
import std.sys.env as env

fn main() {
    let dir = env.get_var("SQL_DIR").unwrap()
    let path = "${dir}/app.db"
    let db = sql.open(path).unwrap()
    let made = db.execute("CREATE TABLE t (n INTEGER)").unwrap()
    let one = db.execute("INSERT INTO t VALUES (41), (1)").unwrap()
    let closed = db.close()
    let again = sql.open(path).unwrap()
    let rows = again.query("SELECT SUM(n) AS s FROM t").unwrap()
    println(rows.get(0).int("s").unwrap())
    match sql.open("/no/such/dir/x.db") {
        Ok(d) => { println("opened") }
        Err(e) => { println("cannot open") }
    }
}
"#,
        "file",
    );
    assert_eq!(got, ["42", "cannot open"]);
}

#[test]
fn large_answers_and_extreme_values_cross_intact() {
    let got = run(
        r#"import sqlite as sql
import std.math as math

fn main() {
    let db = sql.memory().unwrap()
    let made = db.execute("CREATE TABLE t (id INTEGER, body TEXT)").unwrap()
    let wide = "0123456789".repeat(20)
    let t = db.begin().unwrap()
    var i = 0
    while i < 3000 {
        let done = t.execute("INSERT INTO t VALUES (?, ?)", [i, wide]).unwrap()
        i = i + 1
    }
    let kept = t.commit()
    let rows = db.query("SELECT id, body FROM t ORDER BY id").unwrap()
    println(rows.len())
    println(rows.get(2999).int("id").unwrap())
    println(rows.get(2999).text("body").unwrap() == wide)
    let lo = 0 - 9223372036854775807 - 1
    let edges = db.query("SELECT ? AS lo, ? AS hi, ? AS big, ? AS up, ? AS down, ? AS empty, ? AS none", [lo, 9223372036854775807, 1.0e300, math.inf(), 0.0 - math.inf(), "", Bytes()]).unwrap()
    let e = edges.get(0)
    println(e.int("lo").unwrap() == lo)
    println(e.int("hi").unwrap())
    println(e.float("big").unwrap() == 1.0e300)
    println(math.is_infinite(e.float("up").unwrap()))
    println(e.float("down").unwrap() < 0.0)
    println(e.text("empty").unwrap().len())
}
"#,
        "large",
    );
    assert_eq!(got, ["3000", "2999", "true", "true", "9223372036854775807", "true", "true", "true", "0"]);
}
