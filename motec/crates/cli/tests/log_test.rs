//! `std.dev.log`.

use std::process::Command;

fn run_env(source: &str, env: &[(&str, &str)], tag: &str) -> (bool, String, String) {
    let dir = std::env::temp_dir().join(format!("mote_log_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mote"));
    cmd.arg("run").arg(dir.join("main.mote")).env_remove("MOTE_LOG");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn lines(source: &str, env: &[(&str, &str)], tag: &str) -> Vec<String> {
    let (ok, out, err) = run_env(source, env, tag);
    assert!(ok, "{out}{err}");
    err.lines().map(|l| l.split_once(' ').map_or(l.to_string(), |(_, rest)| rest.to_string())).collect()
}

fn raw_lines(source: &str, env: &[(&str, &str)], tag: &str) -> Vec<String> {
    let (ok, out, err) = run_env(source, env, tag);
    assert!(ok, "{out}{err}");
    err.lines().map(String::from).collect()
}

const HEAD: &str = "import std.dev.log as log\n";

const LOGGING: &str = include_str!("programs_mote/logging.mote");

#[test]
fn done_bar_tasks_log_json_to_a_file_that_reads_back() {
    let (ok, out, err) = run_env(LOGGING, &[], "done_bar");
    assert!(ok && err.is_empty(), "{out}{err}");
    assert_eq!(out, "debug starting worker\nerror finished worker\ninfo job done worker\nwarn job slow worker\n");
}

#[test]
fn the_default_threshold_is_info_and_the_default_format_is_text() {
    let src = format!("{HEAD}fn main() {{\n    log.debug(\"hidden\")\n    log.info(\"started\")\n    log.warn(\"slow\")\n    log.error(\"boom\")\n}}\n");
    assert_eq!(lines(&src, &[], "default"), ["INFO started", "WARN slow", "ERROR boom"]);
}

#[test]
fn a_record_starts_with_a_utc_millisecond_timestamp() {
    let src = format!("{HEAD}fn main() {{\n    log.info(\"x\")\n}}\n");
    let line = &raw_lines(&src, &[], "time")[0];
    let stamp = line.split(' ').next().unwrap();
    assert!(stamp.len() == 24 && stamp.ends_with('Z') && stamp.as_bytes()[10] == b'T' && stamp.as_bytes()[19] == b'.', "{line}");
}

#[test]
fn set_level_moves_the_threshold() {
    let src = format!("{HEAD}fn main() {{\n    log.set_level(log.Level.Debug)\n    log.debug(\"d\")\n    log.set_level(log.Level.Error)\n    log.warn(\"w\")\n    log.error(\"e\")\n    log.set_level(log.Level.Off)\n    log.error(\"never\")\n}}\n");
    assert_eq!(lines(&src, &[], "levels"), ["DEBUG d", "ERROR e"]);
}

#[test]
fn fields_are_written_in_order_and_strings_are_quoted_when_needed() {
    let src = format!("{HEAD}fn main() {{\n    log.info(\"login\", {{\"user\": \"ann smith\", \"n\": 3, \"ok\": true, \"ratio\": 0.5, \"plain\": \"x\", \"empty\": \"\", \"eq\": \"a=b\"}})\n}}\n");
    assert_eq!(lines(&src, &[], "fields"), ["INFO login user=\"ann smith\" n=3 ok=true ratio=0.5 plain=x empty=\"\" eq=\"a=b\""]);
}

#[test]
fn base_fields_come_first_and_a_call_replaces_one() {
    let src = format!("{HEAD}fn main() {{\n    log.set_fields({{\"service\": \"api\", \"v\": 1}})\n    log.info(\"a\", {{\"v\": 2, \"x\": 3}})\n    log.set_fields({{}})\n    log.info(\"b\")\n}}\n");
    assert_eq!(lines(&src, &[], "base"), ["INFO a service=api v=2 x=3", "INFO b"]);
}

#[test]
fn a_struct_is_logged_through_its_json() {
    let src = format!("{HEAD}@derive(Json)\nstruct User {{\n    name: String\n    id: Int\n}}\nfn main() {{\n    log.info(\"login\", {{\"user\": User {{ name: \"ann\", id: 7 }}.to_json()}})\n}}\n");
    assert_eq!(lines(&src, &[], "struct"), ["INFO login user={\"name\":\"ann\",\"id\":7}"]);
}

#[test]
fn the_json_format_is_one_flat_object_per_line() {
    let src = format!("{HEAD}fn main() {{\n    log.set_format(log.Format.Json)\n    log.set_fields({{\"service\": \"api\"}})\n    log.warn(\"slow\", {{\"ms\": 1200, \"ok\": false, \"time\": \"x\", \"level\": \"y\", \"msg\": \"z\"}})\n}}\n");
    let line = &raw_lines(&src, &[], "json")[0];
    let (head, rest) = line.split_once("\",\"level\"").unwrap();
    assert!(head.starts_with("{\"time\":\"20"), "{line}");
    assert_eq!(rest, ":\"warn\",\"msg\":\"slow\",\"service\":\"api\",\"ms\":1200,\"ok\":false,\"time_\":\"x\",\"level_\":\"y\",\"msg_\":\"z\"}");
}

#[test]
fn a_non_finite_float_is_written_as_text_in_json() {
    let src = format!("{HEAD}fn main() {{\n    log.set_format(log.Format.Json)\n    log.info(\"x\", {{\"f\": 1.0 / 0.0}})\n}}\n");
    let line = &raw_lines(&src, &[], "nan")[0];
    assert!(line.contains("\"f\":\"") && line.ends_with("\"}"), "{line}");
}

#[test]
fn mote_log_sets_the_threshold_and_wins_over_set_level() {
    let src = format!("{HEAD}fn main() {{\n    log.set_level(log.Level.Debug)\n    log.debug(\"d\")\n    log.info(\"i\")\n    log.error(\"e\")\n}}\n");
    assert_eq!(lines(&src, &[("MOTE_LOG", "error")], "env_error"), ["ERROR e"]);
    assert_eq!(lines(&src, &[("MOTE_LOG", " WARN ")], "env_case"), ["ERROR e"]);
    assert_eq!(lines(&src, &[("MOTE_LOG", "debug")], "env_debug"), ["DEBUG d", "INFO i", "ERROR e"]);
    assert!(lines(&src, &[("MOTE_LOG", "off")], "env_off").is_empty());
}

#[test]
fn an_unknown_mote_log_is_ignored() {
    let src = format!("{HEAD}fn main() {{\n    log.debug(\"d\")\n    log.info(\"i\")\n}}\n");
    assert_eq!(lines(&src, &[("MOTE_LOG", "loud")], "env_bogus"), ["INFO i"]);
}

#[test]
fn a_file_output_appends_one_line_per_record() {
    let path = std::env::temp_dir().join(format!("mote_log_file_{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let src = format!(
        "{HEAD}fn main() {{\n    log.set_output(log.Output.File(\"{0}\"))\n    log.set_format(log.Format.Json)\n    scope {{\n        spawn {{ log.info(\"a\", {{\"t\": 1}}) }}\n        spawn {{ log.info(\"b\", {{\"t\": 2}}) }}\n        spawn {{ log.warn(\"c\", {{\"t\": 3}}) }}\n    }}\n    log.set_output(log.Output.Stderr)\n    log.info(\"after\")\n}}\n",
        path.display()
    );
    let (ok, out, err) = run_env(&src, &[], "file");
    assert!(ok && err.lines().count() == 1 && err.contains("\"msg\":\"after\""), "{out}{err}");
    let written = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).ok();
    let mut msgs: Vec<&str> = written.lines().map(|l| l.split("\"msg\":\"").nth(1).unwrap().split('"').next().unwrap()).collect();
    msgs.sort();
    assert_eq!(msgs, ["a", "b", "c"], "{written}");
}

#[test]
fn a_file_that_cannot_be_written_loses_the_record_without_a_fault() {
    let src = format!("{HEAD}fn main() {{\n    log.set_output(log.Output.File(\"/no/such/dir/x.log\"))\n    log.info(\"lost\")\n    println(\"done\")\n}}\n");
    let (ok, out, err) = run_env(&src, &[], "unwritable");
    assert!(ok && out.trim() == "done" && err.is_empty(), "{out}{err}");
}

#[test]
fn a_map_of_another_type_is_not_a_fields_map() {
    let src = format!("{HEAD}fn main() {{\n    let a = {{\"n\": 1}}\n    log.info(\"x\", a)\n}}\n");
    let (ok, out, err) = run_env(&src, &[], "invariant");
    assert!(!ok && format!("{out}{err}").contains("`info` argument 2 expects"), "{out}{err}");
}
