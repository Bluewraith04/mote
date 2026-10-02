//! `std.date` through `mote run`.

use std::process::Command;

fn run(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_date_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!("import std.date as date\nimport {{ Date }} from std.date\n\n{body}");
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn epoch_days_weekdays_and_the_calendar_edges() {
    let body = r#"
fn main() {
    let e: Date = Date.new(1970, 1, 1).unwrap()
    println(e.days_since_epoch())
    println(e.weekday())
    let y2k: Date = Date.new(2000, 3, 1).unwrap()
    println(y2k.days_since_epoch())
    let first: Date = Date.new(1, 1, 1).unwrap()
    println(first.days_since_epoch())
    println(Date.from_days(-1).to_iso())
    println(Date.from_days(19782).to_iso())
    println(date.is_leap_year(1900))
    println(date.is_leap_year(2000))
    println(date.is_leap_year(2024))
    println(date.is_leap_year(-4))
    println(date.days_in_month(2023, 2))
    println(date.days_in_month(2024, 2))
}
"#;
    assert_eq!(run("edges", body), "0\n3\n11017\n-719162\n1969-12-31\n2024-02-29\nfalse\ntrue\ntrue\ntrue\n28\n29");
}

#[test]
fn arithmetic_clamps_months_and_crosses_years() {
    let body = r#"
fn main() {
    let jan: Date = Date.new(2024, 1, 31).unwrap()
    println(jan.add_months(1).to_iso())
    println(jan.add_months(13).to_iso())
    println(jan.add_months(-2).to_iso())
    let leap: Date = Date.new(2024, 2, 29).unwrap()
    println(leap.add_years(1).to_iso())
    println(leap.add_years(-4).to_iso())
    println(jan.add_days(30).to_iso())
    println(jan.add_days(-31).to_iso())
    println(jan.day_of_year())
    let dec: Date = Date.new(2023, 12, 31).unwrap()
    println(dec.day_of_year())
    println(jan.days_until(dec))
    println(jan.compare(dec))
    println(dec.compare(jan))
    println(jan.compare(jan))
}
"#;
    assert_eq!(
        run("arith", body),
        "2024-02-29\n2025-02-28\n2023-11-30\n2025-02-28\n2020-02-29\n2024-03-01\n2023-12-31\n31\n365\n-31\n1\n-1\n0"
    );
}

#[test]
fn every_thousandth_day_round_trips_and_weekdays_cycle() {
    let body = r#"
fn main() {
    var bad = 0
    var n = -800000
    while n < 800000 {
        let d: Date = Date.from_days(n)
        if d.days_since_epoch() != n { bad = bad + 1 }
        let again: Date = Date.new(d.year, d.month, d.day).unwrap()
        if again.days_since_epoch() != n { bad = bad + 1 }
        if d.add_days(7).weekday() != d.weekday() { bad = bad + 1 }
        n = n + 997
    }
    println(bad)
}
"#;
    assert_eq!(run("round_trip", body), "0");
}

#[test]
fn invalid_dates_and_text_are_errors_not_wrapped_values() {
    let body = r#"
fn main() {
    println(Date.new(2023, 2, 29).is_err())
    println(Date.new(2024, 13, 1).is_err())
    println(Date.new(2024, 0, 1).is_err())
    println(Date.new(2024, 4, 31).is_err())
    let d: Date = date.parse_date("2024-03-09").unwrap()
    println(d.to_iso())
    println(d.weekday())
    println(date.weekday_name(d.weekday()))
    println(date.month_name(d.month))
    println(date.parse_date("2024-3-09").is_err())
    println(date.parse_date("2024/03/09").is_err())
    println(date.parse_date("20x4-03-09").is_err())
    println(date.parse_date("2024-02-30").is_err())
    println(date.parse_date("").is_err())
}
"#;
    assert_eq!(run("invalid", body), "true\ntrue\ntrue\ntrue\n2024-03-09\n5\nSaturday\nMarch\ntrue\ntrue\ntrue\ntrue\ntrue");
}

fn run_dt(name: &str, body: &str) -> String {
    let header = "import std.date as date\nimport { DateTime } from std.date\nimport { Duration } from std.time\n\n";
    run(name, &format!("{header}{body}"))
}

#[test]
fn unix_time_shows_at_any_offset_and_round_trips() {
    let body = r#"
fn main() {
    let t: DateTime = DateTime.from_unix_millis(1700000000000, 0)
    println(t.to_iso())
    println(t.unix_millis())
    let east: DateTime = t.with_offset(7200)
    println(east.to_iso())
    println(east.unix_millis())
    println(t.with_offset(-16200).to_iso())
    println(DateTime.from_unix_millis(-1, 0).to_iso())
    println(DateTime.from_unix_millis(1700000000123, 0).to_iso())
    println(t.compare(east))
}
"#;
    assert_eq!(
        run_dt("unix", body),
        "2023-11-14T22:13:20Z\n1700000000000\n2023-11-15T00:13:20+02:00\n1700000000000\n2023-11-14T17:43:20-04:30\n1969-12-31T23:59:59.999Z\n2023-11-14T22:13:20.123Z\n0"
    );
}

#[test]
fn durations_move_a_moment_and_measure_between_two() {
    let body = r#"
fn main() {
    let t: DateTime = DateTime.from_unix_millis(1700000000000, 3600)
    println(t.plus(Duration.from_secs(3600)).to_iso())
    println(t.plus(Duration.from_millis(1500)).to_iso())
    println(t.plus(Duration.from_secs(0 - 86400)).to_iso())
    let later: DateTime = t.plus(Duration.from_secs(90061))
    println(later.since(t).as_secs())
    println(t.since(later).as_secs())
    println(later.compare(t))
    println(t.compare(later))
    let a: DateTime = DateTime.from_unix(10, 900000000, 0)
    println(a.plus(Duration.from_millis(200)).to_iso())
}
"#;
    assert_eq!(
        run_dt("plus", body),
        "2023-11-15T00:13:20+01:00\n2023-11-14T23:13:21.5+01:00\n2023-11-13T23:13:20+01:00\n90061\n-90061\n1\n-1\n1970-01-01T00:00:11.1Z"
    );
}

#[test]
fn format_fills_the_patterns_and_keeps_unknown_ones() {
    let body = r#"
fn main() {
    let t: DateTime = DateTime.from_unix_millis(1700000000123, 19800)
    println(t.format("%Y-%m-%d %H:%M:%S"))
    println(t.format("%A %a, %e %B %b %Y"))
    println(t.format("day %j at %z, %f ns, 100%%"))
    println(t.format("%q and trailing %"))
    println(DateTime.from_unix_millis(0, 0).format("%e|%z"))
    println(DateTime.from_unix_millis(0, -3600).format("%z"))
}
"#;
    assert_eq!(
        run_dt("format", body),
        "2023-11-15 03:43:20\nWednesday Wed, 15 November Nov 2023\nday 319 at +0530, 123000000 ns, 100%\n%q and trailing %\n 1|+0000\n-0100"
    );
}

#[test]
fn iso_text_parses_back_and_bad_text_is_an_error() {
    let body = r#"
fn main() {
    let a: DateTime = date.parse_iso("2023-11-14T22:13:20Z").unwrap()
    println(a.unix_millis())
    let b: DateTime = date.parse_iso("2023-11-15T03:43:20.25+05:30").unwrap()
    println(b.unix_millis())
    println(b.nano)
    println(b.to_iso())
    let c: DateTime = date.parse_iso("2023-11-14T22:13").unwrap()
    println(c.to_iso())
    let d: DateTime = date.parse_iso("2024-02-29").unwrap()
    println(d.to_iso())
    let e: DateTime = date.parse_iso("2023-11-14T22:13:20.123456789123-01:00").unwrap()
    println(e.to_iso())
    println(date.parse_iso("2023-11-14 22:13:20Z").is_err())
    println(date.parse_iso("2023-11-14T25:00:00Z").is_err())
    println(date.parse_iso("2023-11-14T22:13:20+0530").is_err())
    println(date.parse_iso("2023-11-14T22:13:20.Z").is_err())
    println(date.parse_iso("2023-02-30T00:00:00Z").is_err())
    println(date.parse_iso("2023-11-14T22:13:20Zjunk").is_err())
    println(DateTime.new(2024, 1, 1, 0, 0, 60, 0, 0).is_err())
    println(DateTime.new(2024, 1, 1, 0, 0, 0, 0, 86400).is_err())
    println(date.now_utc().year >= 2025)
}
"#;
    assert_eq!(
        run_dt("iso", body),
        "1700000000000\n1700000000250\n250000000\n2023-11-15T03:43:20.25+05:30\n2023-11-14T22:13:00Z\n2024-02-29T00:00:00Z\n2023-11-14T22:13:20.123456789-01:00\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue"
    );
}

#[test]
fn the_machine_offset_comes_from_the_platform_zone() {
    let dir = std::env::temp_dir().join(format!("mote_date_local_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let body = r#"
import std.date as date
import { DateTime } from std.date

fn main() {
    println(date.local_offset(1700000000).unwrap())
    let utc: DateTime = DateTime.from_unix_millis(1700000000000, 0)
    let local: DateTime = utc.to_local().unwrap()
    println(local.to_iso())
    println(local.unix_millis() == utc.unix_millis())
    let now: DateTime = date.now_local().unwrap()
    println(now.offset)
}
"#;
    std::fs::write(dir.join("main.mote"), body).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).env("TZ", "XXX-5:30").output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "19800\n2023-11-15T03:43:20+05:30\ntrue\n19800");
}

#[test]
fn the_fake_platform_scripts_the_zone() {
    use std::sync::Arc;

    use isa::value::TypeRegistry;
    use modules::MultiFileCompiler;
    use platform::FakePlatform;

    let dir = std::env::temp_dir().join(format!("mote_date_fake_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let main_file = dir.join("main.mote");
    let body = r#"
import std.date as date
import { DateTime } from std.date

fn main() {
    let now: DateTime = date.now_local().unwrap()
    println(now.to_iso())
    println(date.now_utc().to_iso())
}
"#;
    std::fs::write(&main_file, body).unwrap();
    let compiled = MultiFileCompiler::new(dir.clone()).compile_program(&main_file).unwrap();
    let mut registry = TypeRegistry::new();
    for t in &compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let fake = Arc::new(FakePlatform::new(1).with_local_offset(-18000));
    let mut rt = runtime::Runtime::with_type_registry(compiled.code_objects, &registry);
    ffi::builtins::install(&mut rt);
    rt.set_native_table(&compiled.native_table).unwrap();
    rt.set_platform(fake.clone());
    rt.run_entry_on(1).unwrap();
    std::fs::remove_dir_all(dir).ok();
    assert_eq!(fake.stdout().trim(), "2023-11-14T17:13:20-05:00\n2023-11-14T22:13:20Z");
}
