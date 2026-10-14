//! `Args` in a trait list and attributes with values.

use std::process::Command;

fn run_args(source: &str, args: &[&str], tag: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("mote_args_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("import std.args\nimport {{ Args }} from std.args\n{source}")).unwrap();
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
    std::fs::write(dir.join("src/main.mote"), "import std.env as env\n\nfn main() {\n    println(env.args())\n}\n").unwrap();
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
    std::fs::write(dir.join("main.mote"), format!("import std.args\nimport {{ Args }} from std.args\n{source}\nfn main() {{}}\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("check").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success() && text.contains(message), "expected `{message}`, got: {text}");
}

const TOOL: &str = include_str!("programs_mote/args_tool.mote");

const OPTIONS: &str = r#"
@arg(v = verbose, o = out, i = include)
class Options: (Args) {
    verbose: Bool
    out: String? = null
    count: Int = 0
    level: Int = 2
    ratio: Float? = null
    include: List<String> = []
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
            &[],
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
            "error: missing argument <FILE>",
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
import std.env

enum Mode: (Args) {
    Fast
    SlowDown
}

class Opts: (Args) {
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
    rejected("struct P {\n    @arg(help = \"x\")\n    a: Int\n}\n", "no_derive_field", "`@arg` cannot go on a field: write help as a `///` comment");
    rejected("@arg(x = a)\nstruct P {\n    a: Int\n}\n", "no_derive_type", "`@arg` on a type is not read by any trait `P` lists");
    rejected("struct P {\n    @stable\n    a: Int\n}\n", "stable_field", "`@stable` cannot go on a field");
    rejected("struct P {\n    @color(x)\n    a: Int\n}\n", "unknown_field_attr", "`@color` cannot go on a field");
    rejected("enum C {\n    A { @arg(x = a) a: Int }\n}\n", "variant_field_attr", "`@arg` cannot go on a field");
    rejected("struct P {\n    a: Int\n    @arg(x = a)\n    fn f(self) {}\n}\n", "method_attr", "`@arg` cannot go on a method");
    rejected("@arg(v = 3)\nstruct P: (Args) {\n    a: Int\n}\n", "non_string", "an attribute value is a name or a plain string");
}

#[test]
fn a_bad_arg_key_or_value_is_an_error() {
    let head = |attr: &str| format!("{attr}\nstruct P: (Args) {{\n    a: Int = 1\n    b: Bool\n}}\n");
    rejected(&head("@arg(color = a)"), "key", "`color` is not a short: a short is one letter");
    rejected(&head("@arg(help = a)"), "gone_help", "`help` is gone: write the help as a `///` comment");
    rejected(&head("@arg(positional)"), "gone_positional", "`positional` is gone: a field with no default is a positional");
    rejected(&head("@arg(short = \"a\")"), "gone_short", "`short` is gone: write `@arg(x = field)`");
    rejected(&head("@arg(default = \"1\")"), "gone_default", "`default` is gone: write `= value`");
    rejected(&head("@arg(x)"), "bare", "`@arg` takes `x = field` pairs, not `x`");
    rejected(&head("@arg(ab = a)"), "short_two", "`ab` is not a short");
    rejected(&head("@arg(x = nope)"), "no_field", "no field named nope");
    rejected(&head("@arg(h = b)"), "short_h", "short -h is reserved");
    rejected(&head("@arg(x = a, x = b)"), "short_twice", "short -x is used twice");
    rejected(&head("@arg(x = a, y = a)"), "two_shorts", "`a` has two shorts, -x and -y");
    rejected("@arg(x = a)\nstruct P: (Args) {\n    a: String\n}\n", "positional_short", "a positional has no short");
    rejected("@arg(x = a)\nenum C: (Args) {\n    A\n}\n", "enum_short", "an enum has no options of its own");
    rejected("struct P: (Args) {\n    a: Bool = true\n}\n", "flag_true", "a flag cannot default to true");
    rejected("struct P: (Args) {\n    a: Int? = 3\n}\n", "optional_default", "is optional, so its default is `null`");
    rejected("class P: (Args) {\n    a: List<String> = [\"x\"]\n}\n", "list_default", "is a list, so its default is `[]`");
    rejected("struct P: (Args) {\n    a: Int = \"x\"\n}\n", "mistyped_default", "the field is 'Int' but its default is 'String'");
}

#[test]
fn a_field_that_a_command_line_cannot_hold_is_an_error() {
    let ty = |t: &str| format!("class P: (Args) {{\n    a: {t}\n}}\n");
    rejected(&ty("Map<String, Int>"), "map", "which a command line cannot hold");
    rejected(&ty("List<Bool>"), "list_bool", "which a command line cannot hold");
    rejected("struct P: (Args) {\n    help: Bool\n}\n", "help_field", "`help` is reserved for `--help`");
    rejected("@arg(a = x, a = y)\nstruct P: (Args) {\n    x: Bool\n    y: Bool\n}\n", "dup_short", "short -a is used twice");
    rejected("class P: (Args) {\n    xs: List<String>\n    y: String\n}\n", "list_not_last", "it must be the last positional");
    rejected("struct P<T>: (Args) {\n    a: Int\n}\n", "generic", "a generic type is not supported");
    rejected("struct P: (Args) {\n    a: Int\n    fn parse(x: Int) -> Int { return x }\n}\n", "has_parse", "`P` does not implement `Args`");
}

#[test]
fn a_bad_subcommand_declaration_is_an_error() {
    let cmd = "enum C: (Args) {\n    A\n}\n";
    rejected(&format!("{cmd}struct P: (Args) {{\n    c: C\n    d: C\n}}\n"), "two_commands", "a command line has one subcommand field");
    rejected(&format!("{cmd}struct P: (Args) {{\n    c: C\n    f: String\n}}\n"), "cmd_and_positional", "a type with a subcommand has no positional arguments");
    rejected("enum C: (Args) {\n    A(Int)\n}\n", "tuple_variant", "variant `A` has unnamed fields");
    rejected(&format!("{cmd}enum D: (Args) {{\n    X {{ c: C }}\n}}\n"), "nested", "a subcommand cannot hold a subcommand");
    rejected("enum C: (Args) {\n    RunTests\n    Run_Tests\n}\n", "dup_command", "two variants are the command `run-tests`");
}
