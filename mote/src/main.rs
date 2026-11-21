use std::io::Write;
use std::path::Path;

fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let outcome = mote::execute_live(&words, Path::new("."));
    let _ = std::io::stdout().write_all(outcome.stdout.as_bytes());
    let _ = std::io::stderr().write_all(outcome.stderr.as_bytes());
    std::process::exit(outcome.code);
}
