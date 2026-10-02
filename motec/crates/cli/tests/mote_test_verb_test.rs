//! The `mote test` verb: test discovery, `@ignore`, `--filter`, reporting and the exit code.

use std::process::Command;

fn run_test(source: &str, tag: &str, extra_args: &[&str]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("mote_test_verb_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.mote");
    std::fs::write(&file, source).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mote"));
    cmd.arg("test").arg(&file);
    for a in extra_args {
        cmd.arg(a);
    }
    let out = cmd.output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn a_passing_test_reports_ok_and_exits_zero() {
    let src = "test \"addition\" {\n    assert_eq(1 + 1, 2)\n}\n";
    let (ok, text) = run_test(src, "pass", &[]);
    assert!(ok, "{text}");
    assert!(text.contains("ok") && text.contains("addition"), "{text}");
    assert!(text.contains("1 passed, 0 failed, 0 ignored"), "{text}");
}

#[test]
fn a_failing_test_is_named_and_does_not_crash_the_run() {
    let src = "test \"broken\" {\n    assert_eq(1 + 1, 3)\n}\ntest \"fine\" {\n    assert_eq(2 + 2, 4)\n}\n";
    let (ok, text) = run_test(src, "fail", &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("FAIL") && text.contains("broken"), "{text}");
    assert!(text.contains("ok") && text.contains("fine"), "{text}");
    assert!(text.contains("1 passed, 1 failed, 0 ignored"), "{text}");
}

#[test]
fn ignore_skips_a_test_without_running_it() {
    let src = "@ignore\ntest \"skipped\" {\n    assert_eq(1, 2)\n}\ntest \"runs\" {\n    assert_eq(1, 1)\n}\n";
    let (ok, text) = run_test(src, "ignore", &[]);
    assert!(ok, "{text}");
    assert!(text.contains("ignored") && text.contains("skipped"), "{text}");
    assert!(text.contains("1 passed, 0 failed, 1 ignored"), "{text}");
}

#[test]
fn at_test_marks_a_plain_function_for_discovery() {
    let src = "@test\nfn plain_check() {\n    assert_eq(3 + 3, 6)\n}\n";
    let (ok, text) = run_test(src, "attr", &[]);
    assert!(ok, "{text}");
    assert!(text.contains("ok") && text.contains("plain_check"), "{text}");
}

#[test]
fn filter_keeps_only_matching_tests() {
    let src = "test \"alpha case\" {\n    assert_eq(1, 1)\n}\ntest \"beta case\" {\n    assert_eq(2, 2)\n}\n";
    let (ok, text) = run_test(src, "filter", &["--filter", "alpha"]);
    assert!(ok, "{text}");
    assert!(text.contains("alpha case"), "{text}");
    assert!(!text.contains("beta case"), "{text}");
    assert!(text.contains("1 passed, 0 failed, 0 ignored"), "{text}");
}

#[test]
fn filter_is_a_regex_and_applies_to_ignored_tests() {
    let src = "test \"parse int\" {\n    assert(true)\n}\ntest \"parse float\" {\n    assert(true)\n}\ntest \"render\" {\n    assert(true)\n}\n@ignore\ntest \"parse slow\" {\n    assert(true)\n}\n@ignore\ntest \"render slow\" {\n    assert(true)\n}\n";
    let (ok, text) = run_test(src, "filter_regex", &["--filter", "^parse (int|slow)$"]);
    assert!(ok, "{text}");
    assert!(text.contains("ok      parse int") && !text.contains("parse float") && !text.contains("render"), "{text}");
    assert!(text.contains("1 passed, 0 failed, 1 ignored"), "{text}");
}

#[test]
fn an_invalid_filter_pattern_exits_two() {
    let src = "test \"a\" {\n    assert(true)\n}\n";
    let (ok, text) = run_test(src, "filter_bad", &["--filter", "("]);
    assert!(!ok && text.contains("error: mote test --filter: unterminated group"), "{text}");
}

#[test]
fn filter_matching_nothing_reports_zero_tests_and_exits_zero() {
    let src = "test \"alpha case\" {\n    assert_eq(1, 1)\n}\n";
    let (ok, text) = run_test(src, "filter_empty", &["--filter", "no_such_test"]);
    assert!(ok, "{text}");
    assert!(text.contains("0 passed, 0 failed, 0 ignored"), "{text}");
}

#[test]
fn overall_exit_code_is_zero_when_every_test_passes() {
    let src = "test \"one\" {\n    assert(true)\n}\ntest \"two\" {\n    assert(true)\n}\n";
    let (ok, text) = run_test(src, "all_pass", &[]);
    assert!(ok, "{text}");
    assert!(text.contains("2 passed, 0 failed, 0 ignored"), "{text}");
}

#[test]
fn a_failing_suite_under_test_fails_only_that_test() {
    let src = "import { Suite } from std.test\n@test\nfn suite_case() {\n    var t = Suite.new()\n    t.test(\"bad\", |var s: Suite| { s.assert_eq(1, 2) })\n    t.finish()\n}\ntest \"after\" {\n    assert_eq(2, 2)\n}\n";
    let (ok, text) = run_test(src, "suite", &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("FAIL") && text.contains("suite_case"), "{text}");
    assert!(text.contains("ok") && text.contains("after"), "{text}");
    assert!(text.contains("1 passed, 1 failed, 0 ignored"), "{text}");
}
