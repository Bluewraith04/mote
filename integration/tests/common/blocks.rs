//! The `mote` blocks of `docs/reference`: found, built and judged for the tests that run them.

use std::path::Path;
use std::sync::OnceLock;

use mote::{Options, Pipeline, Target};
use mote_packages::testing::Scratch;

use super::fnv;

/// One `mote` block: where it is, its text, and the `output` block after it.
pub struct Block {
    pub name: String,
    pub page: String,
    pub text: String,
    pub output: Option<String>,
}

impl Block {
    /// The page and a hash of the text, which names the block in the lists and stays when blocks are added before it.
    pub fn key(&self) -> String {
        format!("{}@{}", self.page, &fnv(&self.text)[..8])
    }
}

/// What a block turned out to be.
pub enum Verdict {
    /// Source that builds: the block alone, or after the declarations before it on its page.
    Runs(String),
    /// A block marked `// error` that the compiler rejects, with the message it gave when the mark names one.
    Rejected,
    /// A block that does not build and is not marked: the first line of the error.
    Fragment(String),
    /// A block whose marks or errors do not agree.
    Wrong(String),
}

/// The text after the closing fence at `start`, and the line after that fence.
fn fenced(lines: &[&str], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut at = start;
    while at < lines.len() && lines[at] != "```" {
        text.push_str(lines[at]);
        text.push('\n');
        at += 1;
    }
    (text, at + 1)
}

/// The `mote` blocks of a page, each with the `output` block right after it, and the `output` blocks that follow no `mote` block.
pub fn parse_page(page: &str, text: &str) -> (Vec<Block>, Vec<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let (mut found, mut orphans) = (Vec::new(), Vec::new());
    let (mut at, mut index) = (0, 0);
    while at < lines.len() {
        if lines[at] == "```mote" {
            index += 1;
            let (code, next) = fenced(&lines, at + 1);
            let mut after = next;
            while after < lines.len() && lines[after].is_empty() {
                after += 1;
            }
            let output = (after < lines.len() && lines[after] == "```output").then(|| fenced(&lines, after + 1));
            at = output.as_ref().map_or(next, |(_, end)| *end);
            found.push(Block { name: format!("{page}#{index}"), page: page.to_string(), text: code, output: output.map(|(text, _)| text) });
        } else if lines[at] == "```output" {
            orphans.push(format!("{page}:{}: an `output` block after no `mote` block", at + 1));
            at = fenced(&lines, at + 1).1;
        } else {
            at += 1;
        }
    }
    (found, orphans)
}

/// Every block of every page in name order, and the `output` blocks that follow no `mote` block.
pub fn read() -> (Vec<Block>, Vec<String>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/reference");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).expect("the reference directory").map(|entry| entry.expect("a directory entry").path()).collect();
    paths.sort();
    let (mut found, mut orphans) = (Vec::new(), Vec::new());
    for path in paths.into_iter().filter(|path| path.extension().is_some_and(|extension| extension == "md")) {
        let page = path.file_name().and_then(|name| name.to_str()).expect("a file name").to_string();
        let (blocks, loose) = parse_page(&page, &std::fs::read_to_string(&path).expect("a readable file"));
        found.extend(blocks);
        orphans.extend(loose);
    }
    (found, orphans)
}

/// Whether every line at the left margin starts an item, so the block only declares.
fn declares_only(text: &str) -> bool {
    const ITEMS: [&str; 10] = ["struct ", "class ", "enum ", "trait ", "fn ", "pub ", "import ", "type ", "@", "}"];
    text.lines().filter(|line| !line.is_empty() && !line.starts_with(' ') && !line.starts_with("//")).all(|line| ITEMS.iter().any(|item| line.starts_with(item)))
}

fn declares_main(text: &str) -> bool {
    text.lines().any(|line| line.starts_with("fn main(") || line.starts_with("pub fn main("))
}

pub fn has_tests(text: &str) -> bool {
    text.lines().any(|line| line.starts_with("test "))
}

/// The package `docs` with the source as its `src/main.mote`, built for the target.
pub fn build(source: &str, target: Target) -> Result<Pipeline, Vec<String>> {
    build_with(source, target, false)
}

/// Like `build`, with the optimizer on or off.
pub fn build_with(source: &str, target: Target, optimize: bool) -> Result<Pipeline, Vec<String>> {
    let scratch = Scratch::new();
    let dir = scratch.package("docs", "docs", &[], &[("main.mote", source)]);
    Pipeline::build(&dir, target, &Options { optimize, ..Options::default() }).map_err(|failure| failure.messages.iter().map(|message| headline(message)).collect())
}

pub fn target_of(source: &str) -> Target {
    if declares_main(source) { Target::Run } else { Target::Check }
}

/// The first line of a printed message, without the `error: ` before it.
fn headline(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    line.strip_prefix("error: ").unwrap_or(line).to_string()
}

/// The text after `// error:` on its line, when the block is marked and names a message.
fn error_mark(text: &str) -> Option<Option<String>> {
    let rest = &text[text.find("// error")? + "// error".len()..];
    let line = rest.lines().next().unwrap_or_default();
    Some(line.strip_prefix(':').map(|message| message.trim().to_string()).filter(|message| !message.is_empty()))
}

pub fn classify(all: &[Block], at: usize) -> Verdict {
    let block = &all[at];
    let built = build(&block.text, target_of(&block.text));
    let mark = error_mark(&block.text);
    let Err(errors) = built else {
        return if mark.is_some() { Verdict::Wrong(format!("{} is marked `// error` and builds", block.name)) } else { Verdict::Runs(block.text.clone()) };
    };
    if let Some(message) = mark {
        return match message {
            Some(message) if !errors.iter().any(|error| error.contains(&message)) => Verdict::Wrong(format!("{} is marked `// error: {message}` and the error is {}", block.name, errors[0])),
            _ => Verdict::Rejected,
        };
    }
    let mut context = String::new();
    for before in all[..at].iter().filter(|before| before.page == block.page && declares_only(&before.text)) {
        context.push_str(&before.text);
        context.push('\n');
    }
    let joined = format!("{context}{}", block.text);
    if !context.is_empty() && build(&joined, target_of(&joined)).is_ok() {
        return Verdict::Runs(joined);
    }
    Verdict::Fragment(errors[0].clone())
}

/// Every block with what it turned out to be, worked out once for every test.
pub fn judged() -> &'static (Vec<Block>, Vec<Verdict>) {
    static JUDGED: OnceLock<(Vec<Block>, Vec<Verdict>)> = OnceLock::new();
    JUDGED.get_or_init(|| {
        let (all, _) = read();
        let verdicts = in_parallel(&all, |at, _| classify(&all, at));
        (all, verdicts)
    })
}

/// `work` on each item, spread over the cores, in the order of the items.
pub fn in_parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(usize, &T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(2, |count| count.get());
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let work = &work;
        let handles: Vec<_> = (0..threads).map(|lane| scope.spawn(move || (lane..items.len()).step_by(threads).map(|at| (at, work(at, &items[at]))).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|handle| handle.join().expect("a worker finished")).collect()
    });
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, result)| result).collect()
}
