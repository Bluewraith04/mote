//! `@derive(Args)` and attributes with values.

use std::process::Command;

fn run_args(source: &str, args: &[&str], tag: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("mote_args_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).args(args).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn run_dash_dash_passes_the_arguments_to_the_package_program() {
    let dir = std::env::temp_dir().join(format!("mote_args_package_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("mote.toml"), "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nentry = \"src/main.mote\"\n").unwrap();
    std::fs::write(dir.join("src/main.mote"), "import std.sys.env as env\n\nfn main() {\n    println(env.args())\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).current_dir(&dir).args(["run", "--", "a.md", "--theme", "dark"]).output().unwrap();
    let plain = Command::new(env!("CARGO_BIN_EXE_mote")).current_dir(&dir).arg("run").output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), r#"["tool", "a.md", "--theme", "dark"]"#);
    assert!(plain.status.success());
}

fn rejected(source: &str, tag: &str, message: &str) {
    let dir = std::env::temp_dir().join(format!("mote_args_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("{source}\nfn main() {{}}\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success() && text.contains(message), "expected `{message}`, got: {text}");
}

const TOOL: &str = include_str!("programs_mote/args_tool.mote");

const OPTIONS: &str = r#"
@derive(Args)
class Options {
    @arg(short = "v")
    verbose: Bool
    @arg(short = "o")
    out: String?
    count: Int
    @arg(default = "2")
    level: Int
    ratio: Float?
    @arg(short = "i")
    include: List<String>
    @arg(positional)
    file: String
}

fn show(argv: List<String>) {
    var full = ["p"]
    for a in argv { full.push(a) }
    match Options.parse(full) {
        Ok(o) => { println("verbose=${o.verbose} out=${o.out} count=${o.count} level=${o.level} ratio=${o.ratio} include=${o.include} file=${o.file}") }
        Err(e) => { println("error: ${e.message}") }
    }
}
"#;

fn parse_each(cases: &[&[&str]], tag: &str) -> Vec<String> {
    let calls: Vec<String> = cases
        .iter()
        .map(|c| {
            let items: Vec<String> = c.iter().map(|a| format!("\"{a}\"")).collect();
            format!("    show([{}])\n", items.join(", "))
        })
        .collect();
    let src = format!("{OPTIONS}\nfn main() {{\n{}}}\n", calls.concat());
    let (code, out, err) = run_args(&src, &[], tag);
    assert_eq!(code, 0, "{out}{err}");
    out.lines().map(String::from).collect()
}

#[test]
fn done_bar_a_tool_run_as_a_process_with_arguments() {
    let run = |args: &[&str], tag: &str| run_args(TOOL, args, tag);
    let (code, out, _) = run(&["sum", "1", "2", "3"], "sum");
    assert_eq!((code, out.as_str()), (0, "sum of 3 numbers: 6\n"));
    let (code, out, _) = run(&["-q", "sum", "4", "5"], "quiet");
    assert_eq!((code, out.as_str()), (0, "9\n"));
    let (code, out, _) = run(&["say", "hi", "-n", "2", "--end=bye"], "say");
    assert_eq!((code, out.as_str()), (0, "hi\nhi\nbye\n"));
    let (code, out, err) = run(&["sum", "1", "x"], "bad");
    assert_eq!(code, 2);
    assert!(out.is_empty() && err.starts_with("error: invalid value for <NUMBERS>: expected an int, found \"x\"\n"), "{err}");
    assert!(err.contains("Usage: main.mote [OPTIONS] <COMMAND>"), "{err}");
}

#[test]
fn help_prints_the_whole_tree_and_exits_zero() {
    let (code, out, err) = run_args(TOOL, &["--help"], "help");
    assert_eq!(code, 0, "{err}");
    let expected = "Usage: main.mote [OPTIONS] <COMMAND>

Small tools

Options:
  -q, --quiet  print only the result
  -h, --help   print this help

Commands:
  sum                      add up the numbers
      [NUMBERS]...         the numbers
  say                      repeat a word
      <WORD>               the word
      -n, --times <TIMES>  how many times [default: 1]
      --end <END>          end with this
";
    assert_eq!(out, expected);
}

#[test]
fn a_command_line_reads_into_the_fields() {
    let got = parse_each(
        &[
            &["--count", "3", "f"],
            &["-v", "-o", "x", "--count=4", "--level", "5", "f"],
            &["-ox", "--count", "1", "f"],
            &["-o=x", "--count", "1", "f"],
            &["--count", "-3", "f"],
            &["--count", "1", "--", "-v"],
            &["f", "--count", "1", "--ratio", "0.5", "-i", "a", "--include=b"],
        ],
        "reads",
    );
    assert_eq!(
        got,
        [
            "verbose=false out=null count=3 level=2 ratio=null include=[] file=f",
            "verbose=true out=x count=4 level=5 ratio=null include=[] file=f",
            "verbose=false out=x count=1 level=2 ratio=null include=[] file=f",
            "verbose=false out=x count=1 level=2 ratio=null include=[] file=f",
            "verbose=false out=null count=-3 level=2 ratio=null include=[] file=f",
            "verbose=false out=null count=1 level=2 ratio=null include=[] file=-v",
            "verbose=false out=null count=1 level=2 ratio=0.5 include=[\"a\", \"b\"] file=f",
        ]
    );
}

#[test]
fn a_bad_command_line_is_an_error_naming_the_problem() {
    let got = parse_each(
        &[
            &["--count"],
            &["f"],
            &["--count", "1"],
            &["--count", "x", "f"],
            &["--count", "1", "--ratio", "y", "f"],
            &["--bogus", "f"],
            &["--count", "1", "f", "g"],
            &["-vo"],
            &["--verbose=1", "--count", "1", "f"],
            &["--help"],
        ],
        "errors",
    );
    assert_eq!(
        got,
        [
            "error: option --count needs a value",
            "error: missing option --count",
            "error: missing argument <FILE>",
            "error: invalid value for --count: expected an int, found \"x\"",
            "error: invalid value for --ratio: expected a float, found \"y\"",
            "error: unknown option --bogus",
            "error: unexpected argument \"g\"",
            "error: unknown option -vo",
            "error: option --verbose takes no value",
            "error: help requested",
        ]
    );
}

#[test]
fn subcommands_read_their_own_options_and_report_the_command() {
    let (code, out, _) = run_args(TOOL, &["say", "hey", "--times", "1"], "own");
    assert_eq!((code, out.as_str()), (0, "hey\n"));
    let (code, _, err) = run_args(TOOL, &[], "none");
    assert_eq!(code, 2);
    assert!(err.starts_with("error: missing command (expected sum, say)\n"), "{err}");
    let (code, _, err) = run_args(TOOL, &["deploy"], "unknown");
    assert_eq!(code, 2);
    assert!(err.starts_with("error: unknown command \"deploy\" (expected sum, say)\n"), "{err}");
    let (_, _, err) = run_args(TOOL, &["say"], "missing_word");
    assert!(err.starts_with("error: missing argument <WORD>\n"), "{err}");
    let (_, _, err) = run_args(TOOL, &["--loud", "sum"], "global");
    assert!(err.starts_with("error: unknown option --loud\n"), "{err}");
}

#[test]
fn an_optional_subcommand_is_null_when_absent() {
    let src = r#"
import std.sys.env

@derive(Args)
enum Mode {
    Fast
    SlowDown
}

@derive(Args)
class Opts {
    mode: Mode?
}

fn main() {
    let o = Opts.parse_or_exit(env.args())
    println(o.mode)
}
"#;
    assert_eq!(run_args(src, &[], "opt_none").1, "null\n");
    assert_eq!(run_args(src, &["slow-down"], "opt_some").1, "SlowDown\n");
}

#[test]
fn usage_returns_the_text_for_a_program_name() {
    let src = format!("{OPTIONS}\nfn main() {{\n    println(Options.usage(\"cp\"))\n}}\n");
    let (code, out, err) = run_args(&src, &[], "usage");
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("Usage: cp [OPTIONS] <FILE>\n\n"), "{out}");
    assert!(out.contains("  -v, --verbose"), "{out}");
    assert!(out.contains("--level <LEVEL>") && out.contains("[default: 2]"), "{out}");
}

#[test]
fn a_misplaced_or_unknown_attribute_is_an_error() {
    rejected("struct P {\n    @arg(help = \"x\")\n    a: Int\n}\n", "no_derive_field", "`@arg` on a field needs `@derive(Args)` on its type");
    rejected("@arg(help = \"x\")\nstruct P {\n    a: Int\n}\n", "no_derive_type", "`@arg` needs `@derive(Args)` on the type");
    rejected("struct P {\n    @stable\n    a: Int\n}\n", "stable_field", "`@stable` cannot go on a field");
    rejected("struct P {\n    @color(x)\n    a: Int\n}\n", "unknown_field_attr", "`@color` is not a known attribute");
    rejected("struct P {\n    a: Int\n    @arg(help = \"x\")\n    fn f(self) {}\n}\n", "method_attr", "`@arg` cannot go on a method");
    rejected("@derive(Args)\nstruct P {\n    @arg(help = 3)\n    a: Int\n}\n", "non_string", "an attribute value is a plain string");
}

#[test]
fn a_bad_arg_key_or_value_is_an_error() {
    let field = |attr: &str| format!("@derive(Args)\nstruct P {{\n    {attr}\n    a: Int\n}}\n");
    rejected(&field("@arg(color = \"x\")"), "key", "unknown `@arg` key `color`");
    rejected(&field("@arg(help)"), "bare_help", "`help` needs a value");
    rejected(&field("@arg(positional = \"x\")"), "valued_positional", "`positional` takes no value");
    rejected(&field("@arg(short = \"ab\")"), "short_two", "`short` is one letter");
    rejected(&field("@arg(short = \"h\")"), "short_h", "`-h` is reserved for `--help`");
    rejected(&field("@arg(default = \"x\")"), "bad_default", "`default` is `x`, which `a` cannot read");
    rejected("@derive(Args)\nstruct P {\n    @arg(default = \"1\")\n    a: Bool\n}\n", "default_bool", "`default` goes on");
    rejected("@derive(Args)\nstruct P {\n    @arg(default = \"1\")\n    a: Int?\n}\n", "default_optional", "`default` goes on");
    rejected("@derive(Args)\n@arg(short = \"x\")\nstruct P {\n    a: Int\n}\n", "item_short", "unknown `@arg` key `short`");
}

#[test]
fn a_field_that_a_command_line_cannot_hold_is_an_error() {
    let ty = |t: &str| format!("@derive(Args)\nclass P {{\n    a: {t}\n}}\n");
    rejected(&ty("Map<String, Int>"), "map", "which a command line cannot hold");
    rejected(&ty("List<Bool>"), "list_bool", "which a command line cannot hold");
    rejected("@derive(Args)\nstruct P {\n    help: Bool\n}\n", "help_field", "`help` is reserved for `--help`");
    rejected("@derive(Args)\nstruct P {\n    @arg(positional)\n    a: Bool\n}\n", "bool_positional", "so it cannot be positional");
    rejected("@derive(Args)\nstruct P {\n    @arg(short = \"a\")\n    x: Bool\n    @arg(short = \"a\")\n    y: Bool\n}\n", "dup_short", "two fields use `-a`");
    rejected(
        "@derive(Args)\nclass P {\n    @arg(positional)\n    xs: List<String>\n    @arg(positional)\n    y: String\n}\n",
        "list_not_last",
        "it must be the last positional",
    );
    rejected("@derive(Args)\nstruct P<T> {\n    a: Int\n}\n", "generic", "`@derive(Args)` does not support a generic type");
    rejected("@derive(Args)\nstruct P {\n    a: Int\n    fn parse(x: Int) -> Int { return x }\n}\n", "has_parse", "already has a method named `parse`");
}

#[test]
fn a_bad_subcommand_declaration_is_an_error() {
    let cmd = "@derive(Args)\nenum C {\n    A\n}\n";
    rejected(&format!("{cmd}@derive(Args)\nstruct P {{\n    c: C\n    d: C\n}}\n"), "two_commands", "a command line has one subcommand field");
    rejected(&format!("{cmd}@derive(Args)\nstruct P {{\n    c: C\n    @arg(positional)\n    f: String\n}}\n"), "cmd_and_positional", "a type with a subcommand has no positional arguments");
    rejected("@derive(Args)\nenum C {\n    A(Int)\n}\n", "tuple_variant", "variant `A` has unnamed fields");
    rejected(&format!("{cmd}@derive(Args)\nenum D {{\n    X {{ c: C }}\n}}\n"), "nested", "a subcommand cannot hold a subcommand");
    rejected("@derive(Args)\nenum C {\n    RunTests\n    Run_Tests\n}\n", "dup_command", "two variants are the command `run-tests`");
}
