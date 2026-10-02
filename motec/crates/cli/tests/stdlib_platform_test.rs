
use std::process::Command;

const SCRIPT: &str = "\
import std.sys.io as io
import std.sys.env as env
import { Rng } from std.random
import { Clock, Duration } from std.time

// 'ddd' -> 123 without a string->int builtin: walk the bytes.
fn parse_int(s: String) -> Int {
    let b = s.bytes()
    var n = 0
    var i = 0
    while i < b.len() {
        n = n * 10 + (b.get(i) - 48)
        i = i + 1
    }
    return n
}

fn fail(msg: String) {
    io.stderr().write_line(msg)
    env.exit(1)
}

fn main() {
    let args = env.args()
    let input = args.get(1)
    let output = args.get(2)

    let read = io.read_file(input)
    match read {
        Err(e) => { fail(\"cannot read input\") }
        Ok(text) => {
            var clock = Clock.fake(1000)
            let start = clock.now()

            var names: List<String> = []
            var total = 0
            for line in text.split(\"\\n\") {
                if line.trim() == \"\" { continue }
                let parts = line.split(\" \")
                names.push(parts.get(0))
                total = total + parse_int(parts.get(1))
            }
            clock.advance(Duration.from_millis(250))
            let elapsed = clock.since(start).as_millis()

            var a = Rng.seeded(42)
            var b = Rng.seeded(42)
            let pick = a.choice(names)
            let again = b.choice(names)
            var picked = \"\"
            match pick {
                Some(v) => { picked = v }
                None => { fail(\"no records\") }
            }
            match again {
                Some(v) => { if v != picked { fail(\"rng not deterministic\") } }
                None => { fail(\"no records\") }
            }

            let result = \"count=${names.len()} total=${total} elapsed_ms=${elapsed} picked=${picked}\\n\"
            let w = io.write_file(output, result)
            match w {
                Err(e) => { fail(\"cannot write output\") }
                Ok(v) => { println(\"wrote result\") }
            }
        }
    }
}
";

fn run(script_name: &str, input: &str, output: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("mote_c8_{}_{}", std::process::id(), script_name));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("prog.mote");
    std::fs::write(&script, SCRIPT).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote"))
        .arg("run")
        .arg(&script)
        .arg(input)
        .arg(output)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code(),
    )
}

#[test]
fn test_phase_c_done_bar_success_and_deterministic() {
    let dir = std::env::temp_dir().join(format!("mote_c8_io_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("in.txt");
    let output = dir.join("out.txt");
    std::fs::write(&input, "alice 5\nbob 7\n\ncarol 3\n").unwrap();

    let (stdout, stderr, code) = run("ok1", input.to_str().unwrap(), output.to_str().unwrap());
    assert_eq!(code, Some(0), "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("wrote result"), "{stdout}");
    let first = std::fs::read_to_string(&output).unwrap();
    assert!(first.starts_with("count=3 total=15 elapsed_ms=250 picked="), "{first}");

    std::fs::remove_file(&output).unwrap();
    let (_, _, code2) = run("ok2", input.to_str().unwrap(), output.to_str().unwrap());
    assert_eq!(code2, Some(0));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), first);

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn test_phase_c_done_bar_err_path_exits_nonzero() {
    let missing = std::env::temp_dir().join("mote_c8_definitely_missing.txt");
    let out = std::env::temp_dir().join("mote_c8_never_written.txt");
    std::fs::remove_file(&out).ok();
    let (_, stderr, code) = run("err", missing.to_str().unwrap(), out.to_str().unwrap());
    assert_eq!(code, Some(1));
    assert!(stderr.contains("cannot read input"), "{stderr}");
    assert!(!out.exists());
}
