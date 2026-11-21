//! A C program links the static library and calls the entries.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use mote_abi::{Entry, FaultKind, FaultTable, Kind, Scalar, Span, TypeImage, TypeTable};

/// The directory of the profile this test was built in, such as `target/debug`.
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test executable");
    exe.parent().and_then(Path::parent).expect("deps is inside the profile directory").to_path_buf()
}

/// Builds the static library with cargo, once, and gives its path.
fn library() -> &'static Path {
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let dir = profile_dir();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
        let mut build = Command::new(env!("CARGO"));
        build.args(["build", "--locked", "--offline", "-p", "mote-runtime", "--manifest-path"]).arg(&manifest);
        if dir.file_name().is_some_and(|name| name == "release") {
            build.arg("--release");
        }
        let status = build.status().expect("cargo runs");
        assert!(status.success(), "cargo build failed");
        let library = dir.join("libmote_runtime.a");
        assert!(library.exists(), "{} was not built", library.display());
        library
    })
}

/// The names of the types the C program allocates.
fn tables() -> String {
    let mut table = TypeTable::new();
    let int = table.add("Int", Kind::Scalar(Scalar::I64));
    let point = table.add("Point", Kind::Class(vec![int, int]));
    let text = table.add("String", Kind::String);
    let mut faults = FaultTable::new();
    faults.add(FaultKind::DivideByZero, Span::new(10, 15));
    faults.add(FaultKind::IndexOutOfRange, Span::new(20, 30));
    let array = |name: &str, bytes: &[u8]| {
        let mut text = format!("static const unsigned char {name}[] = {{");
        for byte in bytes {
            write!(text, "{byte},").unwrap();
        }
        text + "};\n"
    };
    let image = TypeImage::new(table).to_bytes().expect("the table is written");
    format!("#define POINT {}\n#define TEXT {}\n{}{}", point.raw(), text.raw(), array("types", &image), array("faults", &faults.to_bytes()))
}

/// Compiles the C program against the library and runs the scenario.
fn run_c(scenario: &str) -> Output {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("c_link").join(scenario);
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    std::fs::write(dir.join("tables.h"), tables()).expect("the tables are written");
    let program = dir.join("program");
    let compile = Command::new("cc")
        .args(["-O0", "-g", "-fno-omit-frame-pointer", "-Wall", "-Werror", "-I"])
        .arg(&dir)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_link/main.c"))
        .arg(library())
        .args(["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc", "-o"])
        .arg(&program)
        .output()
        .expect("cc runs");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    Command::new(&program).arg(scenario).output().expect("the program runs")
}

fn lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).collect()
}

#[test]
fn c_link_a_c_program_allocates_through_the_library() {
    let output = run_c("alloc");
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(lines(&output), ["point type 2 words 0 0", "text type 3 length 5", "region moved 1"]);
}

#[test]
fn c_link_a_fault_reaches_the_c_handler_with_the_frames_of_the_c_callers() {
    let output = run_c("fault");
    assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(lines(&output), ["kind 3", "site 1", "span 20 30", "message index out of range", "fault_site 2", "frames 1 1 1"]);
}

#[test]
fn c_link_a_message_from_the_program_is_a_raised_fault_without_a_site() {
    let output = run_c("message");
    assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    let seen = lines(&output);
    assert_eq!(&seen[..5], ["kind 6", "site -1", "span -1 -1", "message too deep", "fault_site 0"]);
}

#[test]
fn c_link_without_a_handler_the_report_goes_to_standard_error_and_the_process_exits_with_1() {
    let output = run_c("default");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(error.starts_with("fault: division by zero\n  at 0x"), "{error}");
    assert!(output.stdout.is_empty());
}

#[test]
fn c_link_the_library_defines_a_symbol_for_every_entry_of_the_abi() {
    let listing = Command::new("nm").arg("-g").arg(library()).output().expect("nm runs");
    let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
    for entry in Entry::ALL {
        assert!(listing.lines().any(|line| line.ends_with(&format!(" T {}", entry.name()))), "{} is not defined", entry.name());
    }
}
