//! The `mote` command line: one table of commands drives the parser and the usage text.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::pipeline::{toolchain, Options, Pipeline, Target};
use crate::run::{run_main_with, run_tests_with, Finished, Settings};

/// The exit code of a command that did what it was asked.
pub const OK: i32 = 0;
/// The exit code of a command that ran and found errors, or could not find what it needs.
pub const FAILED: i32 = 1;
/// The exit code of a command line that does not follow the usage.
pub const USAGE: i32 = 2;

/// A positional argument.
pub struct Arg {
    pub name: &'static str,
    pub required: bool,
}

/// A `--name` flag; one with a `value` takes it as `--name value` or `--name=value`.
pub struct Flag {
    pub name: &'static str,
    pub value: Option<&'static str>,
    pub help: &'static str,
}

pub struct Command {
    pub name: &'static str,
    pub help: &'static str,
    pub args: &'static [Arg],
    pub flags: &'static [Flag],
    /// Runs the command in a directory.
    pub run: fn(&Parsed, &Path) -> Outcome,
}

const STD: Flag = Flag { name: "std", value: Some("dir"), help: "Use this directory as the std instead of mote_std/std" };
const CORE: Flag = Flag { name: "core", value: Some("dir"), help: "Use the libraries in this directory instead of mote_std" };

const LIB: Flag = Flag { name: "lib", value: None, help: "Make a library with src/lib.mote in place of a program with src/main.mote" };
const FILTER: Flag = Flag { name: "filter", value: Some("text"), help: "Run only the test blocks whose name contains the text" };
const WORKERS: Flag = Flag { name: "workers", value: Some("n"), help: "Run tasks on this many threads" };
const SEED: Flag = Flag { name: "seed", value: Some("n"), help: "Run on one thread with a virtual clock, in the order the seed gives" };
const MAX_HEAP: Flag = Flag { name: "max-heap", value: Some("size"), help: "Limit the heap to this many bytes; K, M and G suffixes count 1024 each" };
const MEM_STATS: Flag = Flag { name: "mem-stats", value: None, help: "Print what the heap did to standard error when the program ends" };

/// Every command, in the order the usage lists them.
pub const COMMANDS: &[Command] = &[
    Command { name: "add", help: "Add a dependency: a zip file or a package directory", args: &[Arg { name: "package", required: true }], flags: &[], run: add_package },
    Command { name: "build", help: "Link the main of a package into target/<name>", args: &[Arg { name: "dir", required: false }], flags: &[STD, CORE], run: build_package },
    Command { name: "check", help: "Check a package without running it", args: &[Arg { name: "dir", required: false }], flags: &[STD, CORE], run: check },
    Command { name: "compile", help: "Compile the main of a package to the object file target/<name>.o", args: &[Arg { name: "dir", required: false }], flags: &[STD, CORE], run: compile_package },
    Command { name: "help", help: "Show the commands, or the usage of one", args: &[Arg { name: "command", required: false }], flags: &[], run: help },
    Command { name: "init", help: "Make a directory a package: a new one, or `.`", args: &[Arg { name: "dir", required: false }], flags: &[LIB], run: init_package },
    Command { name: "install", help: "Link the main of a package or zip file into <home>/bin/<name>", args: &[Arg { name: "package", required: false }], flags: &[STD, CORE], run: install_package },
    Command { name: "package", help: "Pack a package into target/<name>-<version>.zip", args: &[Arg { name: "dir", required: false }], flags: &[], run: pack_package },
    Command { name: "remove", help: "Remove a dependency", args: &[Arg { name: "name", required: true }], flags: &[], run: remove_package },
    Command { name: "run", help: "Run the main of a package, or of one of its files", args: &[Arg { name: "file", required: false }], flags: &[STD, CORE, WORKERS, SEED, MAX_HEAP, MEM_STATS], run: run_package },
    Command { name: "test", help: "Run the test blocks of a package", args: &[Arg { name: "dir", required: false }], flags: &[STD, CORE, FILTER, WORKERS, SEED, MAX_HEAP], run: test_package },
    Command { name: "version", help: "Print the version", args: &[], flags: &[], run: version },
];

fn command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.name == name)
}

/// A command line that does not follow the usage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageError {
    NoCommand,
    UnknownCommand(String),
    UnknownFlag { command: &'static str, flag: String },
    MissingValue { command: &'static str, flag: &'static str },
    UnwantedValue { command: &'static str, flag: &'static str },
    RepeatedFlag { command: &'static str, flag: &'static str },
    MissingArgument { command: &'static str, name: &'static str },
    UnexpectedArgument { command: &'static str, arg: String },
}

impl fmt::Display for UsageError {
    fn fmt(&self, out: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UsageError::NoCommand => write!(out, "no command given"),
            UsageError::UnknownCommand(name) => write!(out, "unknown command `{name}`"),
            UsageError::UnknownFlag { command, flag } => write!(out, "`mote {command}` has no flag `{flag}`"),
            UsageError::MissingValue { command, flag } => write!(out, "`mote {command} --{flag}` needs a value"),
            UsageError::UnwantedValue { command, flag } => write!(out, "`mote {command} --{flag}` takes no value"),
            UsageError::RepeatedFlag { command, flag } => write!(out, "`mote {command} --{flag}` is given twice"),
            UsageError::MissingArgument { command, name } => write!(out, "`mote {command}` needs <{name}>"),
            UsageError::UnexpectedArgument { command, arg } => write!(out, "`mote {command}` takes no argument `{arg}`"),
        }
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, out: &mut fmt::Formatter) -> fmt::Result {
        write!(out, "Command({})", self.name)
    }
}

/// A command line read against the table.
#[derive(Debug)]
pub struct Parsed {
    pub command: &'static Command,
    pub args: Vec<String>,
    /// Each flag given with its value.
    pub flags: Vec<(&'static str, Option<String>)>,
    /// The words after `--` of `run`, which the program reads with `std.io.args`.
    pub program: Vec<String>,
    /// Whether a program writes to standard output as it prints; `execute_live` sets it.
    pub live: bool,
    /// The mote home; `$MOTE_HOME` or `~/.mote` when unset.
    pub home: Option<PathBuf>,
}

impl Parsed {
    pub fn arg(&self, at: usize) -> Option<&str> {
        self.args.get(at).map(String::as_str)
    }

    /// Whether the flag was given, with or without a value.
    pub fn flag_given(&self, name: &str) -> bool {
        self.flags.iter().any(|(flag, _)| *flag == name)
    }

    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(flag, _)| *flag == name).and_then(|(_, value)| value.as_deref())
    }
}

/// Reads the arguments after `mote`; `--version` and `-V` are `version`, and `--help` and `-h` are `help`.
pub fn parse(words: &[String]) -> Result<Parsed, UsageError> {
    let Some(first) = words.first() else { return Err(UsageError::NoCommand) };
    let (name, rest) = match first.as_str() {
        "--version" | "-V" => ("version", &words[1..]),
        "--help" | "-h" => ("help", &words[1..]),
        other => (other, &words[1..]),
    };
    let command = command(name).ok_or_else(|| UsageError::UnknownCommand(name.to_string()))?;
    let mut args = Vec::new();
    let mut flags: Vec<(&'static str, Option<String>)> = Vec::new();
    let mut at = 0;
    let mut only_arguments = false;
    let mut program = Vec::new();
    while at < rest.len() {
        let word = &rest[at];
        at += 1;
        if only_arguments || !word.starts_with('-') || word == "-" {
            args.push(word.clone());
        } else if word == "--" && command.name == "run" {
            program = rest[at..].to_vec();
            break;
        } else if word == "--" {
            only_arguments = true;
        } else if word == "--help" || word == "-h" {
            let help = self::command("help").expect("the help command");
            return Ok(Parsed { command: help, args: vec![command.name.to_string()], flags: Vec::new(), program: Vec::new(), live: false, home: None });
        } else {
            let text = word.strip_prefix("--").ok_or_else(|| UsageError::UnknownFlag { command: command.name, flag: word.clone() })?;
            let (flag_name, inline) = match text.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (text, None),
            };
            let flag = command.flags.iter().find(|flag| flag.name == flag_name).ok_or_else(|| UsageError::UnknownFlag { command: command.name, flag: word.clone() })?;
            if flags.iter().any(|(given, _)| *given == flag.name) {
                return Err(UsageError::RepeatedFlag { command: command.name, flag: flag.name });
            }
            let value = match (flag.value, inline) {
                (None, None) => None,
                (None, Some(_)) => return Err(UsageError::UnwantedValue { command: command.name, flag: flag.name }),
                (Some(_), Some(value)) => Some(value),
                (Some(_), None) => {
                    let value = rest.get(at).ok_or(UsageError::MissingValue { command: command.name, flag: flag.name })?;
                    at += 1;
                    Some(value.clone())
                }
            };
            flags.push((flag.name, value));
        }
    }
    if let Some(missing) = command.args.iter().enumerate().find(|(index, arg)| arg.required && *index >= args.len()).map(|(_, arg)| arg) {
        return Err(UsageError::MissingArgument { command: command.name, name: missing.name });
    }
    if let Some(extra) = args.get(command.args.len()) {
        return Err(UsageError::UnexpectedArgument { command: command.name, arg: extra.clone() });
    }
    Ok(Parsed { command, args, flags, program, live: false, home: None })
}

fn args_text(command: &Command) -> String {
    command.args.iter().map(|arg| if arg.required { format!(" <{}>", arg.name) } else { format!(" [{}]", arg.name) }).collect()
}

fn flags_text(command: &Command) -> String {
    command
        .flags
        .iter()
        .map(|flag| match flag.value {
            Some(value) => format!(" [--{} <{value}>]", flag.name),
            None => format!(" [--{}]", flag.name),
        })
        .collect()
}

/// The commands with their arguments and what each does.
pub fn usage() -> String {
    let mut out = format!("mote {}\n\nUsage: mote <command> [arguments]\n\nCommands:\n", toolchain());
    let lines: Vec<(String, &str)> = COMMANDS.iter().map(|command| (format!("{}{}", command.name, args_text(command)), command.help)).collect();
    let width = lines.iter().map(|(left, _)| left.len()).max().unwrap_or(0);
    for (left, help) in lines {
        out.push_str(&format!("  {left:<width$}  {help}\n"));
    }
    out.push_str("\nRun `mote help <command>` for the flags of a command.\n");
    out
}

/// The usage of one command with its flags.
pub fn command_usage(command: &Command) -> String {
    let mut out = format!("Usage: mote {}{}{}\n\n{}\n", command.name, args_text(command), flags_text(command), command.help);
    if !command.flags.is_empty() {
        out.push_str("\nFlags:\n");
        let lines: Vec<(String, &str)> = command.flags.iter().map(|flag| (flag.value.map_or(format!("--{}", flag.name), |value| format!("--{} <{value}>", flag.name)), flag.help)).collect();
        let width = lines.iter().map(|(left, _)| left.len()).max().unwrap_or(0);
        for (left, help) in lines {
            out.push_str(&format!("  {left:<width$}  {help}\n"));
        }
    }
    out
}

/// What a command printed and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl Outcome {
    pub fn ok(stdout: impl Into<String>) -> Outcome {
        Outcome { stdout: stdout.into(), stderr: String::new(), code: OK }
    }

    pub fn failed(stderr: impl Into<String>) -> Outcome {
        Outcome { stdout: String::new(), stderr: stderr.into(), code: FAILED }
    }

    fn usage_error(error: &UsageError) -> Outcome {
        let hint = match error {
            UsageError::NoCommand => usage(),
            _ => "Run `mote help` for the usage.\n".to_string(),
        };
        Outcome { stdout: String::new(), stderr: format!("error: {error}\n\n{hint}"), code: USAGE }
    }
}

/// The directory of the package a command works on: `dir` when given, else the nearest directory from `cwd` upward that holds a `mote.toml`.
pub fn find_package(dir: Option<&str>, cwd: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = dir {
        let found = cwd.join(dir);
        return if found.join("mote.toml").is_file() { Ok(found) } else { Err(format!("no mote.toml in {}", mote_packages::shown(&found))) };
    }
    let mut at = cwd.to_path_buf();
    loop {
        if at.join("mote.toml").is_file() {
            return Ok(at);
        }
        let real = std::fs::canonicalize(&at).map_err(|error| format!("cannot read {}: {error}", mote_packages::shown(&at)))?;
        if real.parent().is_none() {
            return Err(format!("no mote.toml in {} or any parent directory", mote_packages::shown(&cwd.join(""))));
        }
        at = at.join("..");
    }
}

/// A byte count such as `4096`, `64K`, `512M` or `2G`.
fn size(text: &str) -> Option<usize> {
    let (digits, unit) = match text.char_indices().last()? {
        (at, 'K' | 'k') => (&text[..at], 1 << 10),
        (at, 'M' | 'm') => (&text[..at], 1 << 20),
        (at, 'G' | 'g') => (&text[..at], 1 << 30),
        _ => (text, 1),
    };
    digits.parse::<usize>().ok()?.checked_mul(unit)
}

/// The run settings the flags name.
fn how(parsed: &Parsed) -> Result<Settings, String> {
    let number = |name: &str| parsed.flag(name).map(|text| text.parse::<u64>().map_err(|_| format!("`--{name}` is a number, not {text}"))).transpose();
    let workers = number("workers")?;
    if workers == Some(0) {
        return Err("`--workers` is at least 1".to_string());
    }
    let seed = number("seed")?;
    if workers.is_some() && seed.is_some() {
        return Err("`--seed` runs on one thread, so it takes no `--workers`".to_string());
    }
    let max_heap = parsed.flag("max-heap").map(|text| size(text).ok_or_else(|| format!("`--max-heap` is a size such as 64M, not {text}"))).transpose()?;
    Ok(Settings { workers: workers.map(|count| count as usize), seed, max_heap, mem_stats: parsed.flag_given("mem-stats"), live: parsed.live, args: parsed.program.clone() })
}

/// `MOTE_OPT=none` turns the optimizer off.
fn options(parsed: &Parsed, cwd: &Path) -> Options {
    let optimize = std::env::var("MOTE_OPT").map_or(true, |value| value != "none");
    Options { std: parsed.flag("std").map(|dir| cwd.join(dir)), core: parsed.flag("core").map(|dir| cwd.join(dir)), filter: parsed.flag("filter").map(str::to_string), optimize, home: parsed.home.clone(), ..Options::default() }
}

fn pack_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    match find_package(parsed.arg(0), cwd).and_then(|dir| crate::deps::package(&dir)) {
        Ok(said) => Outcome::ok(said),
        Err(message) => Outcome::failed(format!("error: {message}\n")),
    }
}

fn add_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    let what = parsed.arg(0).expect("add needs a package");
    match find_package(None, cwd).and_then(|project| crate::deps::add(&project, cwd, what, parsed.home.as_deref())) {
        Ok(said) => Outcome::ok(said),
        Err(message) => Outcome::failed(format!("error: {message}\n")),
    }
}

fn remove_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    let name = parsed.arg(0).expect("remove needs a name");
    match find_package(None, cwd).and_then(|project| crate::deps::remove(&project, name)) {
        Ok(said) => Outcome::ok(said),
        Err(message) => Outcome::failed(format!("error: {message}\n")),
    }
}

/// The directory `install` builds: the zip's cache directory, a package directory, or the package around `cwd`.
fn install_source(parsed: &Parsed, cwd: &Path) -> Result<PathBuf, String> {
    match parsed.arg(0) {
        Some(what) if cwd.join(what).extension().is_some_and(|extension| extension == "zip") && cwd.join(what).is_file() => Ok(crate::deps::unzipped(&cwd.join(what), parsed.home.as_deref())?.0),
        other => find_package(other, cwd),
    }
}

fn install_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    let home = match parsed.home.clone().or_else(mote_packages::default_home) {
        Some(home) => home,
        None => return Outcome::failed("error: no mote home: set MOTE_HOME or HOME\n"),
    };
    let dir = match install_source(parsed, cwd) {
        Ok(dir) => dir,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    let mut pipeline = match Pipeline::build(&dir, Target::Run, &options(parsed, cwd)) {
        Ok(pipeline) => pipeline,
        Err(failure) => return Outcome::failed(failure.to_string()),
    };
    let bin = home.join("bin");
    if let Err(error) = std::fs::create_dir_all(&bin) {
        return Outcome::failed(format!("error: cannot make {}: {error}\n", mote_packages::shown(&bin)));
    }
    let out = executable_path(&bin.join(&pipeline.packages.root().manifest.name));
    match crate::executable::link(&mut pipeline, &out) {
        Ok(()) => Outcome { stdout: format!("installed {}\n", mote_packages::shown(&out)), stderr: pipeline.warnings.concat(), code: OK },
        Err(failure) => Outcome::failed(failure.to_string()),
    }
}

/// Appends `.exe` on Windows when missing.
fn executable_path(base: &Path) -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    if suffix.is_empty() || base.to_string_lossy().to_ascii_lowercase().ends_with(suffix) {
        base.to_path_buf()
    } else {
        PathBuf::from(format!("{}{}", base.display(), suffix))
    }
}

/// `file` as a path below the package at `dir`, such as `src/tool.mote`.
fn below_package(file: &str, cwd: &Path, dir: &Path) -> Result<String, String> {
    let given = std::fs::canonicalize(cwd.join(file)).map_err(|_| format!("no such file {file}"))?;
    let root = std::fs::canonicalize(dir).map_err(|error| format!("cannot read {}: {error}", mote_packages::shown(dir)))?;
    let below = given.strip_prefix(&root).map_err(|_| format!("{file} is not in the package"))?;
    Ok(below.to_string_lossy().replace('\\', "/"))
}

fn init_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    match crate::init::init(cwd, parsed.arg(0), parsed.flag_given("lib")) {
        Ok(said) => Outcome::ok(said),
        Err(message) => Outcome::failed(format!("error: {message}\n")),
    }
}

fn run_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    let dir = match find_package(None, cwd) {
        Ok(dir) => dir,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    let settings = match how(parsed) {
        Ok(how) => how,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    let mut options = options(parsed, cwd);
    if let Some(file) = parsed.arg(0) {
        match below_package(file, cwd, &dir) {
            Ok(below) => options.entry = Some(below),
            Err(message) => return Outcome::failed(format!("error: {message}\n")),
        }
    }
    match Pipeline::build(&dir, Target::Run, &options) {
        Ok(mut pipeline) => {
            let Finished { run: ran, mem_stats } = match run_main_with(&mut pipeline, &settings) {
                Ok(finished) => finished,
                Err(failure) => return Outcome::failed(failure.to_string()),
            };
            Outcome { stderr: pipeline.warnings.concat() + &ran.failure().unwrap_or_default() + &mem_stats.unwrap_or_default(), code: ran.code(), stdout: ran.output }
        }
        Err(failure) => Outcome::failed(failure.to_string()),
    }
}

fn test_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    let dir = match find_package(parsed.arg(0), cwd) {
        Ok(dir) => dir,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    let settings = match how(parsed) {
        Ok(how) => how,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    match Pipeline::build(&dir, Target::Test, &options(parsed, cwd)) {
        Ok(mut pipeline) => {
            let ran = match run_tests_with(&mut pipeline, &settings) {
                Ok(ran) => ran,
                Err(failure) => return Outcome::failed(failure.to_string()),
            };
            Outcome { stdout: ran.report(), stderr: pipeline.warnings.concat(), code: ran.code() }
        }
        Err(failure) => Outcome::failed(failure.to_string()),
    }
}

/// Builds the package for `main` and gives `produce` the pipeline and `target/<name>` of the package.
fn emit(parsed: &Parsed, cwd: &Path, produce: impl FnOnce(&mut Pipeline, &Path) -> Result<PathBuf, crate::Failure>) -> Outcome {
    let dir = match find_package(parsed.arg(0), cwd) {
        Ok(dir) => dir,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    let mut pipeline = match Pipeline::build(&dir, Target::Run, &options(parsed, cwd)) {
        Ok(pipeline) => pipeline,
        Err(failure) => return Outcome::failed(failure.to_string()),
    };
    let target = dir.join("target");
    if let Err(error) = std::fs::create_dir_all(&target) {
        return Outcome::failed(format!("error: cannot make {}: {error}\n", mote_packages::shown(&target)));
    }
    let base = target.join(&pipeline.packages.root().manifest.name);
    match produce(&mut pipeline, &base) {
        Ok(written) => Outcome { stdout: format!("wrote {}\n", mote_packages::shown(&written)), stderr: pipeline.warnings.concat(), code: OK },
        Err(failure) => Outcome::failed(failure.to_string()),
    }
}

fn compile_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    emit(parsed, cwd, |pipeline, base| {
        let file = PathBuf::from(format!("{}.o", base.display()));
        let bytes = crate::executable::object(pipeline)?;
        std::fs::write(&file, bytes).map_err(|error| crate::Failure { messages: vec![format!("error: cannot write {}: {error}\n", mote_packages::shown(&file))] })?;
        Ok(file)
    })
}

fn build_package(parsed: &Parsed, cwd: &Path) -> Outcome {
    emit(parsed, cwd, |pipeline, base| {
        let out = executable_path(base);
        crate::executable::link(pipeline, &out)?;
        Ok(out)
    })
}

fn check(parsed: &Parsed, cwd: &Path) -> Outcome {
    let dir = match find_package(parsed.arg(0), cwd) {
        Ok(dir) => dir,
        Err(message) => return Outcome::failed(format!("error: {message}\n")),
    };
    match Pipeline::build(&dir, Target::Check, &options(parsed, cwd)) {
        Ok(pipeline) => {
            let manifest = &pipeline.packages.root().manifest;
            Outcome { stdout: format!("checked {} {}\n", manifest.name, manifest.version), stderr: pipeline.warnings.concat(), code: OK }
        }
        Err(failure) => Outcome::failed(failure.to_string()),
    }
}

fn version(_: &Parsed, _: &Path) -> Outcome {
    Outcome::ok(format!("mote {}\n", toolchain()))
}

fn help(parsed: &Parsed, _: &Path) -> Outcome {
    match parsed.arg(0) {
        None => Outcome::ok(usage()),
        Some(name) => match command(name) {
            Some(command) => Outcome::ok(command_usage(command)),
            None => Outcome::usage_error(&UsageError::UnknownCommand(name.to_string())),
        },
    }
}

/// Runs the command line `words` (the arguments after `mote`) in `cwd`; what a program prints is in the outcome.
pub fn execute(words: &[String], cwd: &Path) -> Outcome {
    run_line(words, cwd, false, None)
}

/// Like `execute`, with the mote home in `home` instead of `$MOTE_HOME` or `~/.mote`.
pub fn execute_at(words: &[String], cwd: &Path, home: &Path) -> Outcome {
    run_line(words, cwd, false, Some(home.to_path_buf()))
}

/// Like `execute`, with a program's output written to standard output as it prints, so the outcome does not hold it.
pub fn execute_live(words: &[String], cwd: &Path) -> Outcome {
    run_line(words, cwd, true, None)
}

fn run_line(words: &[String], cwd: &Path, live: bool, home: Option<PathBuf>) -> Outcome {
    let mut parsed = match parse(words) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::usage_error(&error),
    };
    parsed.live = live;
    parsed.home = home;
    (parsed.command.run)(&parsed, cwd)
}

#[cfg(test)]
mod tests {
    use mote_packages::testing::Scratch;

    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn run(line: &str) -> Outcome {
        execute(&words(line), Path::new("."))
    }

    fn parsed(line: &str) -> Parsed {
        parse(&words(line)).unwrap()
    }

    fn usage_error(line: &str) -> String {
        parse(&words(line)).unwrap_err().to_string()
    }

    #[test]
    fn cli_version_prints_the_toolchain_version_in_every_spelling() {
        let expected = Outcome::ok(format!("mote {}\n", toolchain()));
        for line in ["version", "--version", "-V"] {
            assert_eq!(run(line), expected, "{line}");
        }
    }

    #[test]
    fn cli_help_lists_every_command_of_the_table_with_its_help_line() {
        let outcome = run("help");
        assert_eq!(outcome.code, OK);
        assert!(outcome.stdout.starts_with(&format!("mote {}\n\nUsage: mote <command> [arguments]\n\nCommands:\n", toolchain())), "{}", outcome.stdout);
        for command in COMMANDS {
            let line = outcome.stdout.lines().find(|line| line.trim_start().starts_with(command.name)).unwrap_or_else(|| panic!("{} is not listed", command.name));
            assert!(line.ends_with(command.help), "{line}");
        }
        assert_eq!(run("--help"), outcome);
        assert_eq!(run("-h"), outcome);
    }

    #[test]
    fn cli_help_for_a_command_shows_its_arguments_and_flags() {
        let outcome = run("help check");
        assert_eq!(outcome.code, OK);
        assert_eq!(outcome.stdout, "Usage: mote check [dir] [--std <dir>] [--core <dir>]\n\nCheck a package without running it\n\nFlags:\n  --std <dir>   Use this directory as the std instead of mote_std/std\n  --core <dir>  Use the libraries in this directory instead of mote_std\n");
        assert_eq!(run("check --help"), outcome);
        assert_eq!(run("check -h"), outcome);
    }

    #[test]
    fn cli_help_for_a_command_without_flags_has_no_flags_section() {
        assert_eq!(run("help version").stdout, "Usage: mote version\n\nPrint the version\n");
    }

    #[test]
    fn cli_help_for_an_unknown_command_is_a_usage_error() {
        let outcome = run("help frobnicate");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (USAGE, ""));
        assert!(outcome.stderr.starts_with("error: unknown command `frobnicate`\n"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_no_command_prints_the_usage_to_standard_error_and_exits_2() {
        let outcome = run("");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (USAGE, ""));
        assert!(outcome.stderr.starts_with("error: no command given\n\nmote "), "{}", outcome.stderr);
        assert!(outcome.stderr.contains("Commands:\n"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_a_usage_error_names_the_problem_and_points_at_help() {
        let outcome = run("frobnicate");
        assert_eq!(outcome.code, USAGE);
        assert_eq!(outcome.stderr, "error: unknown command `frobnicate`\n\nRun `mote help` for the usage.\n");
    }

    #[test]
    fn cli_the_parser_reads_arguments_and_both_spellings_of_a_flag_with_a_value() {
        let a = parsed("check pkg --std lib");
        assert_eq!((a.arg(0), a.flag("std")), (Some("pkg"), Some("lib")));
        let b = parsed("check --std=lib pkg");
        assert_eq!((b.arg(0), b.flag("std")), (Some("pkg"), Some("lib")));
        let c = parsed("check");
        assert_eq!((c.arg(0), c.flag("std")), (None, None));
        let dash = parsed("check -- -odd");
        assert_eq!(dash.arg(0), Some("-odd"));
        let value = parsed("check --std -odd");
        assert_eq!(value.flag("std"), Some("-odd"));
    }

    #[test]
    fn cli_the_parser_rejects_what_the_table_does_not_allow() {
        assert_eq!(usage_error("check --nope"), "`mote check` has no flag `--nope`");
        assert_eq!(usage_error("check -x"), "`mote check` has no flag `-x`");
        assert_eq!(usage_error("check --std"), "`mote check --std` needs a value");
        assert_eq!(usage_error("check --std a --std b"), "`mote check --std` is given twice");
        assert_eq!(usage_error("check a b"), "`mote check` takes no argument `b`");
        assert_eq!(usage_error("version extra"), "`mote version` takes no argument `extra`");
        assert_eq!(usage_error("version --x=1"), "`mote version` has no flag `--x=1`");
        assert_eq!(usage_error(""), "no command given");
    }

    #[test]
    fn cli_every_row_of_the_table_has_a_unique_name_and_flags_are_unique_in_it() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|command| command.name).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
        for command in COMMANDS {
            let mut flags: Vec<&str> = command.flags.iter().map(|flag| flag.name).collect();
            flags.sort();
            let count = flags.len();
            flags.dedup();
            assert_eq!(flags.len(), count, "{}", command.name);
            let required_first = command.args.windows(2).all(|pair| pair[0].required || !pair[1].required);
            assert!(required_first, "{}", command.name);
        }
    }

    #[test]
    fn cli_the_parser_accepts_every_command_flag_and_argument_the_table_lists() {
        for command in COMMANDS {
            for flag in command.flags {
                let line = match flag.value {
                    Some(_) => format!("{} --{} x", command.name, flag.name),
                    None => format!("{} --{}", command.name, flag.name),
                };
                let args: Vec<String> = command.args.iter().filter(|arg| arg.required).map(|arg| arg.name.to_string()).collect();
                let all = [words(&line), args].concat();
                let found = parse(&all).unwrap_or_else(|error| panic!("{line}: {error}"));
                assert_eq!(found.command.name, command.name);
                assert!(found.flags.iter().any(|(name, _)| *name == flag.name), "{line}");
            }
            let required: Vec<String> = command.args.iter().map(|arg| arg.name.to_string()).collect();
            let found = parse(&[vec![command.name.to_string()], required].concat()).unwrap();
            assert_eq!(found.args.len(), command.args.len());
        }
    }

    #[test]
    fn cli_the_usage_text_mentions_every_command_argument_and_flag_of_the_table() {
        let overview = usage();
        for command in COMMANDS {
            assert!(overview.contains(&format!("{}{}", command.name, args_text(command))), "{}", command.name);
            let detail = command_usage(command);
            for flag in command.flags {
                assert!(detail.contains(&format!("--{}", flag.name)) && detail.contains(flag.help), "{}", command.name);
            }
            for arg in command.args {
                assert!(detail.contains(arg.name), "{}", command.name);
            }
        }
    }

    #[test]
    fn cli_the_exit_codes_are_0_for_success_1_for_failure_and_2_for_usage() {
        assert_eq!((OK, FAILED, USAGE), (0, 1, 2));
    }

    fn package(source: &str) -> (Scratch, PathBuf) {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
        (scratch, dir)
    }

    fn at(dir: &Path, line: &str) -> Outcome {
        execute(&words(line), dir)
    }

    #[test]
    fn cli_check_of_a_package_that_is_sound_exits_0() {
        let (_scratch, dir) = package("fn main() {\n    println(1)\n}\n");
        let outcome = at(&dir, "check");
        assert_eq!(outcome, Outcome { stdout: "checked app 0.1.0\n".into(), stderr: String::new(), code: OK });
    }

    #[test]
    fn cli_check_prints_the_diagnostics_of_a_package_with_errors_and_exits_1() {
        let (_scratch, dir) = package("fn main() {\n    let x: Int = \"s\"\n}\n");
        let outcome = at(&dir, "check");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (FAILED, ""));
        assert!(outcome.stderr.starts_with("error: "), "{}", outcome.stderr);
        assert!(outcome.stderr.contains("2 |     let x: Int = \"s\"\n"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_check_prints_warnings_to_standard_error_and_still_exits_0() {
        let (_scratch, dir) = package("fn main() {\n}\n\n/// stray\n");
        let outcome = at(&dir, "check");
        assert_eq!(outcome.code, OK);
        assert!(outcome.stdout.starts_with("checked app"), "{}", outcome.stdout);
        assert!(outcome.stderr.starts_with("warning: this doc comment documents nothing\n"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_check_checks_functions_nothing_calls() {
        let (_scratch, dir) = package("fn main() {\n}\n\nfn unused() {\n    let x: Int = \"s\"\n}\n");
        assert_eq!(at(&dir, "check").code, FAILED);
    }

    #[test]
    fn cli_check_takes_the_package_directory_as_an_argument() {
        let (scratch, _) = package("fn main() {\n}\n");
        assert_eq!(execute(&words("check app"), &scratch.dir).code, OK);
        let outcome = execute(&words("check nowhere"), &scratch.dir);
        assert_eq!(outcome.code, FAILED);
        assert!(outcome.stderr.starts_with("error: no mote.toml in "), "{}", outcome.stderr);
    }

    #[test]
    fn cli_check_finds_the_package_from_a_directory_inside_it() {
        let (scratch, dir) = package("fn main() {\n}\n");
        scratch.write("app/src/sub/x.mote", "");
        let inside = dir.join("src/sub");
        assert_eq!(at(&inside, "check").code, OK);
    }

    #[test]
    fn cli_check_without_a_package_says_where_it_looked() {
        let scratch = Scratch::new();
        let outcome = execute(&words("check"), &scratch.dir);
        assert_eq!(outcome.code, FAILED);
        assert_eq!(outcome.stderr, format!("error: no mote.toml in {}/ or any parent directory\n", mote_packages::shown(&scratch.dir)));
    }

    #[test]
    fn cli_check_reports_a_bad_manifest_with_its_file_and_line() {
        let (scratch, dir) = package("fn main() {\n}\n");
        scratch.write("app/mote.toml", "[package]\nname = \"app\"\n");
        let outcome = at(&dir, "check");
        assert_eq!(outcome.code, FAILED);
        assert_eq!(outcome.stderr, format!("error: {}/mote.toml:1: `package.version`: missing\n", mote_packages::shown(&dir)));
    }

    #[test]
    fn cli_run_prints_the_output_of_main_and_exits_0() {
        let (_scratch, dir) = package("fn main() {\n    println(\"hi\")\n}\n");
        assert_eq!(at(&dir, "run"), Outcome { stdout: "hi\n".into(), stderr: String::new(), code: OK });
    }

    #[test]
    fn cli_compile_writes_the_object_file_below_target() {
        let (_scratch, dir) = package("fn main() {\n    println(\"hi\")\n}\n");
        let outcome = at(&dir, "compile");
        assert_eq!((outcome.code, outcome.stderr.as_str()), (OK, ""));
        assert_eq!(outcome.stdout, format!("wrote {}\n", mote_packages::shown(&dir.join("target/app.o"))));
        let bytes = std::fs::read(dir.join("target/app.o")).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(&bytes[..4], b"\x7fELF");
        #[cfg(target_os = "windows")]
        assert_eq!(&bytes[..2], &[0x64, 0x86]);
    }

    #[test]
    fn cli_build_writes_an_executable_that_runs_main() {
        let (_scratch, dir) = package("fn main() {\n    println(\"hi\")\n}\n");
        let outcome = at(&dir, "build");
        assert_eq!((outcome.code, outcome.stderr.as_str()), (OK, ""));
        let exe = dir.join(format!("target/app{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(outcome.stdout, format!("wrote {}\n", mote_packages::shown(&exe)));
        let ran = std::process::Command::new(&exe).output().unwrap();
        assert_eq!((String::from_utf8_lossy(&ran.stdout).as_ref(), ran.status.code()), ("hi\n", Some(0)));
    }

    #[test]
    fn cli_build_of_a_package_with_errors_exits_1_and_writes_nothing() {
        let (_scratch, dir) = package("fn main() {\n    let x: Int = \"s\"\n}\n");
        let outcome = at(&dir, "build");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (FAILED, ""));
        assert!(outcome.stderr.starts_with("error: "), "{}", outcome.stderr);
        assert!(!dir.join("target").exists());
    }

    #[test]
    fn cli_jit_runs_main_with_its_output_and_exit_code() {
        let (_scratch, dir) = package("fn main() {\n    println(\"hi\")\n}\n");
        assert_eq!(at(&dir, "run"), Outcome { stdout: "hi\n".into(), stderr: String::new(), code: OK });
    }

    #[test]
    fn cli_no_command_takes_a_backend() {
        let (_scratch, dir) = package("fn main() {\n}\n");
        for command in ["run --backend interp", "run --backend jit", "test --backend interp"] {
            let outcome = at(&dir, command);
            assert_eq!(outcome.code, USAGE, "{command}");
            assert!(outcome.stderr.contains("backend"), "{}", outcome.stderr);
        }
        assert!(!at(&dir, "help run").stdout.contains("interp"));
    }

    #[test]
    fn cli_jit_prints_the_error_main_returns_and_the_line_of_a_fault() {
        let (_scratch, dir) = package("fn main() -> Result<(), Error> {\n    println(\"before\")\n    return Err(Error.other(\"boom\"))\n}\n");
        assert_eq!(at(&dir, "run"), Outcome { stdout: "before\n".into(), stderr: "error: boom\n".into(), code: FAILED });
        let (_scratch, dir) = package("fn main() {\n    let xs: List<Int> = []\n    println(xs[0])\n}\n");
        let outcome = at(&dir, "run");
        assert_eq!(outcome.code, FAILED);
        assert!(outcome.stderr.starts_with("fault: index out of range\n  at "), "{}", outcome.stderr);
    }

    #[test]
    fn cli_jit_runs_the_test_blocks_and_reports_each_one() {
        let (_scratch, dir) = package("import assert from std.test\n\nprintln(\"setup\")\n\ntest \"adds\" {\n    println(\"quiet\")\n    assert(1 + 1 == 2)\n}\n\ntest \"breaks\" {\n    println(\"loud\")\n    assert(1 + 1 == 3)\n}\n\n@ignore\ntest \"later\" {\n    assert(false)\n}\n");
        let compiled = at(&dir, "test");
        assert_eq!(compiled.code, FAILED);
        assert!(compiled.stdout.starts_with("setup\ntest adds (src/main.mote:5) ... ok\n"), "{}", compiled.stdout);
        assert!(compiled.stdout.contains("test later (src/main.mote:15) ... skipped\n"), "{}", compiled.stdout);
    }

    #[test]
    fn cli_jit_a_fault_while_starting_up_runs_no_test_block() {
        let (_scratch, dir) = package("let xs: List<Int> = []\nlet first = xs[0]\n\ntest \"never\" {\n    println(first)\n}\n");
        let outcome = at(&dir, "test");
        assert_eq!(outcome.code, FAILED);
        assert!(outcome.stdout.contains("fault: index out of range"), "{}", outcome.stdout);
        assert!(!outcome.stdout.contains("never"), "{}", outcome.stdout);
    }

    #[test]
    fn cli_jit_workers_and_seed_run_the_program_and_a_seed_repeats_its_order() {
        let (_scratch, dir) = package("fn main() {\n    println(\"hi\")\n}\n");
        assert_eq!(at(&dir, "run --workers 2").stdout, "hi\n");
        assert_eq!(at(&dir, "run --seed 7"), at(&dir, "run --seed=7"));
        assert_eq!(at(&dir, "test --workers 1").code, OK);
        assert_eq!(at(&dir, "test --seed 3").code, OK);
    }

    #[test]
    fn cli_jit_workers_seed_and_max_heap_take_a_number_or_a_size() {
        let (_scratch, dir) = package("fn main() {\n}\n");
        assert_eq!(at(&dir, "run --workers 0"), Outcome::failed("error: `--workers` is at least 1\n"));
        assert_eq!(at(&dir, "run --workers many"), Outcome::failed("error: `--workers` is a number, not many\n"));
        assert_eq!(at(&dir, "run --seed -1"), Outcome::failed("error: `--seed` is a number, not -1\n"));
        assert_eq!(at(&dir, "run --seed x"), Outcome::failed("error: `--seed` is a number, not x\n"));
        assert_eq!(at(&dir, "run --seed 1 --workers 2"), Outcome::failed("error: `--seed` runs on one thread, so it takes no `--workers`\n"));
        assert_eq!(at(&dir, "run --max-heap lots"), Outcome::failed("error: `--max-heap` is a size such as 64M, not lots\n"));
        assert_eq!(at(&dir, "run --max-heap 64M").code, OK);
        assert_eq!(at(&dir, "run --max-heap 65536").code, OK);
    }

    #[test]
    fn cli_jit_sizes_count_bytes_with_binary_suffixes() {
        assert_eq!([size("4096"), size("64K"), size("2m"), size("1G")], [Some(4096), Some(65536), Some(2 << 20), Some(1 << 30)]);
        assert_eq!([size(""), size("K"), size("1T"), size("-5"), size("99999999999G")], [None; 5]);
    }

    #[test]
    fn cli_jit_max_heap_stops_a_program_that_outgrows_it() {
        let (_scratch, dir) = package("fn main() {\n    var items: List<String> = []\n    var i = 0\n    while i < 400000 {\n        items.push(\"item {i}\")\n        i += 1\n    }\n    println(items.len())\n}\n");
        assert_eq!(at(&dir, "run").stdout, "400000\n");
        let outcome = at(&dir, "run --max-heap 1M");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (FAILED, ""));
        assert!(outcome.stderr.starts_with("fault: "), "{}", outcome.stderr);
    }

    #[test]
    fn cli_jit_mem_stats_prints_the_heap_report_to_standard_error() {
        let (_scratch, dir) = package("fn main() {\n    println(\"hi\")\n}\n");
        let outcome = at(&dir, "run --mem-stats");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (OK, "hi\n"));
        assert!(outcome.stderr.contains("collections") && outcome.stderr.contains("peak used bytes"), "{}", outcome.stderr);
        assert_eq!(at(&dir, "run").stderr, "");
    }

    #[test]
    fn cli_jit_live_output_goes_to_standard_output_while_the_program_runs() {
        let (_scratch, dir) = package("fn main() {\n    println(\"live\")\n    var zero = 0\n    zero += 0\n    println(1 / zero)\n}\n");
        let outcome = execute_live(&words("run"), &dir);
        assert_eq!(outcome.stdout, "");
        assert_eq!(outcome.code, FAILED);
        assert!(outcome.stderr.starts_with("fault: division by zero\n"), "{}", outcome.stderr);
        assert_eq!(at(&dir, "run").stdout, "live\n");
    }

    #[test]
    fn cli_run_with_a_file_runs_the_main_of_that_file() {
        let (scratch, dir) = package("fn main() {\n    println(\"main\")\n}\n");
        scratch.write("app/src/tool.mote", "fn main() {\n    println(\"tool\")\n}\n");
        assert_eq!(at(&dir, "run src/tool.mote").stdout, "tool\n");
        assert_eq!(at(&dir.join("src"), "run tool.mote").stdout, "tool\n");
    }

    #[test]
    fn cli_run_with_a_file_that_is_missing_or_outside_the_package_is_an_error() {
        let (scratch, dir) = package("fn main() {\n}\n");
        scratch.write("other.mote", "fn main() {\n}\n");
        scratch.write("app/tests/t.mote", "fn main() {\n}\n");
        let missing = at(&dir, "run src/none.mote");
        assert_eq!((missing.code, missing.stderr.as_str()), (FAILED, "error: no such file src/none.mote\n"));
        let outside = at(&dir, "run ../other.mote");
        assert_eq!((outside.code, outside.stderr.as_str()), (FAILED, "error: ../other.mote is not in the package\n"));
        let beside = at(&dir, "run tests/t.mote");
        assert_eq!((beside.code, beside.stderr.as_str()), (FAILED, "error: tests/t.mote is not a file of the package\n"));
    }

    #[test]
    fn cli_run_test_includes_the_tests_directory_after_src_and_run_ignores_it() {
        let (scratch, dir) = package("import assert from std.test\n\nfn main() {\n    println(\"hi\")\n}\n\ntest \"in src\" {\n    assert(true)\n}\n");
        scratch.write("app/tests/more.mote", "import assert_eq from std.test\n\ntest \"in tests\" {\n    assert_eq(1, 2)\n}\n");
        let outcome = at(&dir, "test");
        assert_eq!((outcome.code, outcome.stderr.as_str()), (FAILED, ""));
        assert!(outcome.stdout.starts_with("test in src (src/main.mote:7) ... ok\ntest in tests (tests/more.mote:3) ... FAILED\n"), "{}", outcome.stdout);
        assert!(outcome.stdout.ends_with("test result: FAILED. 1 passed; 1 failed; 0 skipped\n"), "{}", outcome.stdout);
        assert_eq!(at(&dir, "run"), Outcome { stdout: "hi\n".into(), stderr: String::new(), code: OK });
    }

    #[test]
    fn cli_run_prints_the_error_main_returns_and_exits_1_after_the_output() {
        let (_scratch, dir) = package("fn main() -> Result<(), Error> {\n    println(\"before\")\n    return Err(Error.other(\"boom\"))\n}\n");
        assert_eq!(at(&dir, "run"), Outcome { stdout: "before\n".into(), stderr: "error: boom\n".into(), code: FAILED });
    }

    #[test]
    fn cli_run_prints_a_fault_with_its_place_and_exits_1() {
        let (_scratch, dir) = package("fn main() {\n    let xs: List<Int> = []\n    println(xs[0])\n}\n");
        let outcome = at(&dir, "run");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (FAILED, ""));
        assert!(outcome.stderr.starts_with("fault: index out of range\n  at "), "{}", outcome.stderr);
        assert!(outcome.stderr.contains("src/main.mote:3:"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_run_does_not_run_a_package_with_errors() {
        let (_scratch, dir) = package("fn main() {\n    println(1)\n    let x: Int = \"s\"\n}\n");
        let outcome = at(&dir, "run");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (FAILED, ""));
        assert!(outcome.stderr.contains("3 |     let x: Int = \"s\"\n"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_run_without_a_main_is_an_error() {
        let (_scratch, dir) = package("fn other() {\n}\n");
        assert_eq!(at(&dir, "run"), Outcome::failed("error: src/main.mote has no `main`\n"));
    }

    #[test]
    fn cli_run_finds_the_package_from_a_directory_inside_it() {
        let (scratch, dir) = package("fn main() {\n    println(\"up\")\n}\n");
        scratch.write("app/src/sub/x.mote", "");
        assert_eq!(at(&dir.join("src/sub"), "run").stdout, "up\n");
    }

    #[test]
    fn cli_run_takes_at_most_one_file() {
        let outcome = run("run a b");
        assert_eq!((outcome.code, outcome.stderr.as_str()), (USAGE, "error: `mote run` takes no argument `b`\n\nRun `mote help` for the usage.\n"));
    }

    #[test]
    fn cli_run_test_lists_one_line_per_block_and_exits_1_when_one_fails() {
        let source = "import assert from std.test\n\ntest \"adds\" {\n    assert(1 + 1 == 2)\n}\n\ntest \"breaks\" {\n    assert(1 + 1 == 3)\n}\n\n@ignore\ntest \"later\" {\n}\n";
        let (_scratch, dir) = package(source);
        let outcome = at(&dir, "test");
        assert_eq!((outcome.code, outcome.stderr.as_str()), (FAILED, ""));
        let lines: Vec<&str> = outcome.stdout.lines().collect();
        assert_eq!(lines[..3], ["test adds (src/main.mote:3) ... ok", "test breaks (src/main.mote:7) ... FAILED", "test later (src/main.mote:11) ... skipped"]);
        assert_eq!(lines.last(), Some(&"test result: FAILED. 1 passed; 1 failed; 1 skipped"));
    }

    #[test]
    fn cli_run_test_exits_0_when_every_block_passes() {
        let (_scratch, dir) = package("import assert from std.test\n\ntest \"adds\" {\n    assert(1 + 1 == 2)\n}\n");
        let outcome = at(&dir, "test");
        assert_eq!(outcome, Outcome { stdout: "test adds (src/main.mote:3) ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 skipped\n".into(), stderr: String::new(), code: OK });
    }

    #[test]
    fn cli_run_test_filter_runs_only_the_blocks_whose_name_contains_the_text() {
        let (_scratch, dir) = package("test \"parse numbers\" {\n}\n\ntest \"parse words\" {\n}\n\ntest \"print\" {\n}\n");
        let outcome = at(&dir, "test --filter parse");
        assert_eq!(outcome.code, OK);
        assert_eq!(outcome.stdout.lines().filter(|line| line.starts_with("test ")).count(), 2 + 1, "{}", outcome.stdout);
        assert!(!outcome.stdout.contains("print"), "{}", outcome.stdout);
        assert!(outcome.stdout.ends_with("test result: ok. 2 passed; 0 failed; 0 skipped\n"), "{}", outcome.stdout);
        let none = at(&dir, "test --filter zzz");
        assert_eq!((none.code, none.stdout.as_str()), (OK, "\ntest result: ok. 0 passed; 0 failed; 0 skipped\n"));
    }

    #[test]
    fn cli_run_test_takes_the_package_directory_as_an_argument() {
        let (scratch, _) = package("test \"one\" {\n}\n");
        let outcome = execute(&words("test app"), &scratch.dir);
        assert_eq!(outcome.code, OK);
        assert!(outcome.stdout.contains("test one (src/main.mote:1) ... ok"), "{}", outcome.stdout);
        assert_eq!(execute(&words("test nowhere"), &scratch.dir).code, FAILED);
    }

    #[test]
    fn cli_run_test_does_not_run_a_package_with_errors() {
        let (_scratch, dir) = package("test \"one\" {\n    let x: Int = \"s\"\n}\n");
        let outcome = at(&dir, "test");
        assert_eq!((outcome.code, outcome.stdout.as_str()), (FAILED, ""));
        assert!(outcome.stderr.contains("2 |     let x: Int = \"s\"\n"), "{}", outcome.stderr);
    }

    #[test]
    fn cli_check_uses_the_std_flag_in_place_of_the_embedded_std() {
        let (_scratch, dir) = package("fn main() {\n}\n");
        let outcome = at(&dir, "check --std missing");
        assert_eq!(outcome.code, FAILED);
        assert!(outcome.stderr.contains("missing/mote.toml: cannot read"), "{}", outcome.stderr);
    }
}
