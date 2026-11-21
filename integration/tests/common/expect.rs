//! The recorded results of programs and `test` blocks, and the check of a run against them.

use std::path::Path;

use mote::{Ended, MainRun, TestRun, Verdict};

/// Whether `MOTE_BLESS=1` asks for the files to be rewritten.
pub fn recording() -> bool {
    std::env::var("MOTE_BLESS").is_ok_and(|value| value == "1")
}

/// The output as lines that start with `|`; a missing last newline is marked.
pub fn output_lines(output: &str) -> String {
    let mut text = String::new();
    for line in output.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        text.push_str(&if line.is_empty() { "|".to_string() } else { format!("| {line}") });
        text.push('\n');
    }
    if !output.is_empty() && !output.ends_with('\n') {
        text.push_str("\\ no newline at end\n");
    }
    text
}

/// The output that `output_lines` wrote.
pub fn output_of(lines: &str) -> String {
    let mut output = String::new();
    for line in lines.lines() {
        if let Some(content) = line.strip_prefix('|') {
            output.push_str(content.strip_prefix(' ').unwrap_or(content));
            output.push('\n');
        } else if line.starts_with('\\') {
            output.pop();
        }
    }
    output
}

/// How a `main` run ended, then its output: the body of a program's file.
pub fn main_text(run: &MainRun) -> String {
    let ended = match &run.ended {
        Ended::Returned => "Returned".to_string(),
        Ended::Exit(code) => format!("Exit {code}"),
        Ended::Err(text) => format!("Err {text:?}"),
        Ended::Fault { kind, .. } => format!("Fault {kind}"),
    };
    format!("ended {ended}\n{}", output_lines(&run.output))
}

/// A block's verdict, then its output.
pub fn block_body(verdict: &Verdict, output: &str) -> String {
    let verdict = match verdict {
        Verdict::Passed => "passed".to_string(),
        Verdict::Skipped => "skipped".to_string(),
        Verdict::Failed(message) => format!("failed {message:?}"),
    };
    format!("{verdict}\n{}", output_lines(output))
}

/// The start-up output, then each block under its name: the body of a `std/tests` file.
pub fn tests_text(run: &TestRun) -> String {
    let mut text = format!("start\n{}", output_lines(&run.start_output));
    for line in &run.tests {
        text.push_str(&format!("test {:?}\n{}", line.name, block_body(&line.verdict, &line.output)));
    }
    text
}

/// The sections of a tests file as `(header line, the lines after it)`.
pub fn sections(text: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line == "start" || line.starts_with("test ") {
            found.push((line.to_string(), String::new()));
        } else if let Some((_, body)) = found.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    found
}

/// The lines that differ between the two texts, `-` for the recorded and `+` for the run.
fn difference(recorded: &str, run: &str) -> String {
    let (old, new): (Vec<&str>, Vec<&str>) = (recorded.lines().collect(), run.lines().collect());
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..].iter().rev().zip(new[head..].iter().rev()).take_while(|(a, b)| a == b).count();
    let shown = |lines: &[&str], mark: char| lines.iter().take(12).map(|line| format!("{mark} {line}")).collect::<Vec<_>>();
    let mut lines = shown(&old[head..old.len() - tail], '-');
    lines.extend(shown(&new[head..new.len() - tail], '+'));
    lines.join("\n")
}

/// The file's problem when it does not hold the text; with `record` set and recording on, the file is rewritten and the difference printed instead.
pub fn check(path: &Path, text: &str, record: bool) -> Option<String> {
    let recorded = std::fs::read_to_string(path).unwrap_or_default();
    if recorded == text {
        return None;
    }
    let name = super::relative(path);
    let shown = difference(&recorded, text);
    if record && recording() {
        std::fs::write(path, text).unwrap_or_else(|error| panic!("{name}: {error}"));
        println!("recorded {name}\n{shown}");
        return None;
    }
    Some(format!("{name} differs from the run (MOTE_BLESS=1 records it):\n{shown}"))
}
