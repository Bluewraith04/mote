//! Every `mote` example in the language book runs, and the output it claims is what it prints.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Run,
    Check,
    Error,
    Skip,
}

struct Example {
    page: String,
    line: usize,
    mode: Mode,
    source: String,
    expected: Option<String>,
}

fn book_dirs() -> Vec<PathBuf> {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs");
    let candidates = [docs.join("book/src"), docs.join("public")];
    let found: Vec<PathBuf> = candidates.iter().filter(|d| d.is_dir()).cloned().collect();
    if found.is_empty() { vec![docs] } else { found }
}

fn pages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            pages(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

fn examples() -> Vec<Example> {
    let mut files = Vec::new();
    for root in book_dirs() {
        let mut pages_here = Vec::new();
        pages(&root, &mut pages_here);
        files.extend(pages_here.into_iter().map(|file| (root.clone(), file)));
    }
    let mut found: Vec<Example> = Vec::new();
    for (root, file) in files {
        let page = file.strip_prefix(&root).unwrap().display().to_string();
        let text = std::fs::read_to_string(&file).unwrap();
        let mut open: Option<(String, usize, String)> = None;
        let mut after_example = false;
        for (i, line) in text.lines().enumerate() {
            match (&mut open, line.strip_prefix("```")) {
                (None, Some(info)) => open = Some((info.to_string(), i + 1, String::new())),
                (Some((info, start, body)), Some("")) => {
                    let mut parts = info.split(',');
                    match parts.next() {
                        Some("mote") => {
                            let mode = match parts.next() {
                                None => Mode::Run,
                                Some("check") => Mode::Check,
                                Some("error") => Mode::Error,
                                Some("skip") => Mode::Skip,
                                Some(other) => panic!("{page}:{start}: unknown example attribute `{other}`"),
                            };
                            found.push(Example { page: page.clone(), line: *start, mode, source: std::mem::take(body), expected: None });
                            after_example = true;
                        }
                        Some("output") => {
                            assert!(after_example, "{page}:{start}: an output block must follow a mote example");
                            found.last_mut().unwrap().expected = Some(std::mem::take(body));
                            after_example = false;
                        }
                        _ => after_example = false,
                    }
                    open = None;
                }
                (Some((_, _, body)), _) => {
                    body.push_str(line);
                    body.push('\n');
                }
                (None, None) => {
                    if !line.trim().is_empty() {
                        after_example = false;
                    }
                }
            }
        }
        assert!(open.is_none(), "{page}: a code block is never closed");
    }
    found
}

fn verify(example: &Example, dir: &Path) -> Option<String> {
    let file = dir.join("main.mote");
    std::fs::write(&file, &example.source).unwrap();
    let verb = if matches!(example.mode, Mode::Check | Mode::Error) { "check" } else { "run" };
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg(verb).arg(&file).current_dir(dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if example.mode == Mode::Error {
        return out.status.success().then(|| "the compiler accepts it".to_string());
    }
    if !out.status.success() {
        return Some(format!("exited with {}\n{stdout}{stderr}", out.status));
    }
    if example.mode == Mode::Run {
        match &example.expected {
            Some(want) if stdout.trim_end() != want.trim_end() => {
                return Some(format!("it prints\n{stdout}\nbut the book says\n{want}"));
            }
            None if !stdout.trim().is_empty() => {
                return Some(format!("it prints, but no `output` block follows it\n{stdout}"));
            }
            _ => {}
        }
    }
    None
}

#[test]
fn every_book_example_holds() {
    let all = examples();
    assert!(all.len() > 50, "found only {} examples", all.len());
    let dir = std::env::temp_dir().join(format!("mote_book_{}", std::process::id()));
    let mut failures = Vec::new();
    for example in all.iter().filter(|e| e.mode != Mode::Skip) {
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(why) = verify(example, &dir) {
            failures.push(format!("{}:{}: {why}", example.page, example.line));
        }
        std::fs::remove_dir_all(&dir).ok();
    }
    assert!(failures.is_empty(), "{} of {} book examples fail:\n\n{}", failures.len(), all.len(), failures.join("\n"));
}
