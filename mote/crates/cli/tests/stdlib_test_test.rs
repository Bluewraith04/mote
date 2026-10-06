
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_TEST_ID: AtomicU32 = AtomicU32::new(0);

fn run_mote(script: &str) -> (String, Option<i32>) {
    let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("mote_stdtest_{}_{}.mote", std::process::id(), id));
    std::fs::write(&path, script).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(&path).output().unwrap();
    std::fs::remove_file(&path).ok();
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status.code())
}

#[test]
fn test_std_test_passing_suite_exits_zero() {
    let (out, code) = run_mote(
        "\
import { Suite } from std.test

fn main() {
    var t = Suite.new()
    t.test(\"addition\", |var s: Suite| { s.assert_eq(1 + 1, 2) })
    t.test(\"strings\", |var s: Suite| { s.assert_eq(\"a\" + \"b\", \"ab\") })
    t.test(\"ne\", |var s: Suite| { s.assert_ne(1, 2) })
    t.finish()
}
",
    );
    assert!(out.contains("3 passed, 0 failed"), "{out}");
    assert_eq!(code, Some(0));
}

#[test]
fn test_std_test_failing_suite_reports_and_exits_nonzero() {
    let (out, code) = run_mote(
        "\
import { Suite } from std.test

fn main() {
    var t = Suite.new()
    t.test(\"good\", |var s: Suite| { s.assert(true, \"never shown\") })
    t.test(\"bad\", |var s: Suite| {
        s.assert_eq(1, 2)
        s.assert(false, \"boom\")
    })
    t.finish()
}
",
    );
    assert!(out.contains("ok    good"), "{out}");
    assert!(out.contains("FAIL  bad: expected 1 == 2"), "{out}");
    assert!(out.contains("FAIL  bad: boom"), "{out}");
    assert!(out.contains("1 passed, 1 failed"), "{out}");
    assert_eq!(code, Some(1));
}
