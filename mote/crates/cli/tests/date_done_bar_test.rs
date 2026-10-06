//! Reads timestamps in mixed offsets from a file, orders them by moment, shows them at one offset and measures the gaps.

use std::process::Command;

#[test]
fn a_program_orders_and_converts_timestamps_from_a_file() {
    let dir = std::env::temp_dir().join(format!("mote_date_done_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("events.txt");
    std::fs::write(
        &log,
        "2024-03-10T09:30:00+02:00\n2024-03-09T23:59:59Z\n2024-03-10T00:00:00.5-05:00\n\nnot a time\n2024-02-29\n",
    )
    .unwrap();
    let source = format!(
        r#"import std.sys.io as io
import std.iter as iter
import std.date as date
import std.string as string
import {{ DateTime }} from std.date
import {{ Duration }} from std.time

fn main() {{
    let text: String = io.read_file("{}").unwrap()
    var moments: List<Int> = []
    var bad = 0
    for line in string.lines(text) {{
        if line.is_empty() {{ continue }}
        match date.parse_iso(line) {{
            Ok(t) => {{
                let parsed: DateTime = t
                moments.push(parsed.unix_millis())
            }}
            Err(e) => {{ bad = bad + 1 }}
        }}
    }}
    println("parsed " + moments.len().to_string() + ", rejected " + bad.to_string())
    let ordered = iter.sort(moments)
    var previous: DateTime = DateTime.from_unix_millis(ordered.get(0), 19800)
    for ms in ordered {{
        let t: DateTime = DateTime.from_unix_millis(ms, 19800)
        let gap: Duration = t.since(previous)
        println(t.format("%a %d %b %H:%M:%S%z") + "  +" + (gap.as_secs() / 3600).to_string() + "h")
        previous = t
    }}
    let first: DateTime = DateTime.from_unix_millis(ordered.get(0), 0)
    let last: DateTime = DateTime.from_unix_millis(ordered.get(ordered.len() - 1), 0)
    println(first.date().days_until(last.date()))
}}
"#,
        log.display()
    );
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let expected = "parsed 4, rejected 1\n\
Thu 29 Feb 05:30:00+0530  +0h\n\
Sun 10 Mar 05:29:59+0530  +239h\n\
Sun 10 Mar 10:30:00+0530  +5h\n\
Sun 10 Mar 13:00:00+0530  +2h\n\
10\n";
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
