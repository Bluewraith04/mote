//! Takes a package from its directory to a checked, lowered and verified program.

use std::fmt;
use std::path::{Path, PathBuf};

use mote_base::{render, Diagnostic, DiagnosticSink, Interner, SourceMap};
use mote_ir::{lower, optimize, verify, Module};
use mote_packages::{build, core_packages, default_home, host_triple, linked_libraries, standard, LinkedLibraries, PackageModules, Packages, Version};
use mote_semantics::{analyze, check, instantiate, DefId, DefKind, GenericArg, Instance, ModuleId, Program, Resolved, TyKind};

/// What the program is built for, which decides the entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Every plain function of the package and every `test` block.
    Check,
    /// The `main` of the entry file.
    Run,
    /// The `test` blocks of every file of the package.
    Test,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// A directory that replaces `mote_std/std`.
    pub std: Option<PathBuf>,
    /// A directory of libraries that replaces `mote_std`.
    pub core: Option<PathBuf>,
    pub optimize: bool,
    /// For `Run`, the file below the package that holds the `main`, such as `src/other.mote`, instead of the manifest's `entry`.
    pub entry: Option<String>,
    /// For `Test`, the text a `test` block's name must contain.
    pub filter: Option<String>,
    /// The mote home that holds the cache of unpacked zips; `$MOTE_HOME` or `~/.mote` when unset.
    pub home: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options { std: None, core: None, optimize: true, entry: None, filter: None, home: None }
    }
}

/// The messages of a build that stopped, each as it is printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub messages: Vec<String>,
}

impl fmt::Display for Failure {
    fn fmt(&self, out: &mut fmt::Formatter) -> fmt::Result {
        for message in &self.messages {
            write!(out, "{message}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Failure {}

fn line_error(text: impl fmt::Display) -> Failure {
    Failure { messages: vec![format!("error: {text}\n")] }
}

/// A program that went through every pass.
pub struct Pipeline {
    pub packages: Packages,
    pub map: SourceMap,
    pub interner: Interner,
    pub program: Program,
    pub module: Module,
    pub libraries: LinkedLibraries,
    /// Where each package's modules are in the program.
    pub ranges: Vec<PackageModules>,
    /// `std.reflect.debug_text` for the error type of a `main` that returns a `Result` whose error is not an `Error`.
    pub error_text: Option<Instance>,
    /// The warnings of the passes, as they are printed.
    pub warnings: Vec<String>,
}

/// The version of this toolchain.
pub fn toolchain() -> Version {
    env!("CARGO_PKG_VERSION").parse().expect("the crate version is a version")
}

fn printed(sink: &DiagnosticSink, map: &SourceMap) -> Vec<String> {
    sink.iter().map(|diagnostic| render(map, diagnostic)).collect()
}

/// Stops the build with the sink's diagnostics when it holds an error.
fn stop_on_errors(sink: &DiagnosticSink, map: &SourceMap) -> Result<(), Failure> {
    if sink.has_errors() { Err(Failure { messages: printed(sink, map) }) } else { Ok(()) }
}

impl Pipeline {
    /// Builds the package in `dir` for the target.
    pub fn build(dir: &Path, target: Target, options: &Options) -> Result<Pipeline, Failure> {
        let home = options.home.clone().or_else(default_home);
        let packages = Packages::load(dir, standard(options.std.as_deref(), options.home.as_deref()).map_err(line_error)?, home.as_deref()).map_err(line_error)?;
        let core = core_packages(options.core.as_deref(), options.home.as_deref()).map_err(line_error)?;
        let packages = packages.with_core(core, target != Target::Run).map_err(line_error)?;
        let toolchain = toolchain();
        for package in &packages.list {
            package.manifest.check_toolchain(&package.manifest_file, &toolchain).map_err(line_error)?;
        }
        let graph = build(&packages, target != Target::Run);
        let mote_packages::Graph { input, mut interner, mut sink, map, packages: ranges } = graph;
        stop_on_errors(&sink, &map)?;
        let mut program = mote_semantics::resolve(input, &mut interner, &mut sink);
        stop_on_errors(&sink, &map)?;
        check(&mut program, &mut interner, &mut sink);
        stop_on_errors(&sink, &map)?;
        let Entries { mut roots, error_text } = entries(&program, &interner, &ranges, &packages, target, options).map_err(line_error)?;
        roots.extend(error_text.clone());
        instantiate(&mut program, &mut interner, &mut sink, &roots);
        stop_on_errors(&sink, &map)?;
        analyze(&mut program, &interner, &mut sink);
        stop_on_errors(&sink, &map)?;
        let linked = linked_libraries(&program, &interner, &ranges, &packages, host_triple());
        for error in &linked.errors {
            sink.emit(error.clone());
        }
        stop_on_errors(&sink, &map)?;
        let lowered = lower(&mut program, &mut interner);
        for problem in &lowered.unsupported {
            sink.emit(Diagnostic::error(format!("internal: {}", problem.what), problem.span));
        }
        stop_on_errors(&sink, &map)?;
        let mut module = lowered.module;
        internal(&module, &program, &interner)?;
        if options.optimize {
            optimize(&mut module, &mut program, &mut interner);
            internal(&module, &program, &interner)?;
        }
        let warnings = printed(&sink, &map);
        Ok(Pipeline { packages, map, interner, program, module, libraries: linked, ranges, error_text, warnings })
    }
}

/// A malformed program is a fault of the compiler, not of the source.
fn internal(module: &Module, program: &Program, interner: &Interner) -> Result<(), Failure> {
    let problems = verify(module, program, interner);
    if problems.is_empty() {
        return Ok(());
    }
    Err(Failure { messages: problems.iter().map(|problem| format!("error: internal: {}\n", problem.message)).collect() })
}

/// What the passes start from.
struct Entries {
    roots: Vec<Instance>,
    error_text: Option<Instance>,
}

/// The `debug_text` instance for the error type of `main`, when its result is a `Result` of another error than `Error`.
fn error_text(program: &Program, interner: &Interner, main: DefId) -> Option<Instance> {
    let function = program.defs[main].function()?;
    let TyKind::Adt { def, args } = program.types.kind(function.ret) else { return None };
    if program.std.result != Some(*def) || args.len() != 2 {
        return None;
    }
    let error = args[1].as_type()?;
    if matches!(program.types.kind(error), TyKind::Adt { def, .. } if program.std.error == Some(*def)) {
        return None;
    }
    let reflect = program.modules.iter().find(|(_, module)| module.path.iter().map(|&part| interner.resolve(part)).eq(["std", "reflect"])).map(|(id, _)| id)?;
    match program.modules[reflect].scope.get(&interner.get("debug_text")?) {
        Some(Resolved::Def(def)) => Some(Instance { def: *def, args: vec![GenericArg::Type(error)] }),
        _ => None,
    }
}

fn entries(program: &Program, interner: &Interner, ranges: &[PackageModules], packages: &Packages, target: Target, options: &Options) -> Result<Entries, String> {
    let ids: Vec<ModuleId> = program.modules.iter().map(|(id, _)| id).collect();
    let root = ranges.last().expect("the root package");
    let manifest = &packages.root().manifest;
    let in_root = |def: DefId| ids[root.modules.clone()].contains(&program.defs[def].module);
    let plain = |def: DefId| {
        let item = &program.defs[def];
        item.owner.is_none() && item.function().is_some_and(|function| function.has_body && function.generics.is_empty())
    };
    let wanted = |def: DefId| target == Target::Check || options.filter.as_ref().is_none_or(|text| interner.resolve(program.defs[def].name).contains(text.as_str()));
    let mut found: Vec<DefId> = Vec::new();
    if matches!(target, Target::Check | Target::Test) {
        found.extend(program.defs.iter().map(|(id, _)| id).filter(|&def| in_root(def) && matches!(program.defs[def].kind, DefKind::Test) && wanted(def)));
    }
    if target == Target::Check {
        found.extend(program.defs.iter().map(|(id, _)| id).filter(|&def| in_root(def) && plain(def)));
    }
    found.sort_by_key(|&def| (program.defs[def].module.index(), program.defs[def].span.start));
    let mut error_text_of_main = None;
    if target == Target::Run {
        let (entry, file) = match &options.entry {
            None => (root.entry, manifest.entry.clone()),
            Some(rel) => (packages.root().files.iter().position(|file| format!("src/{}", file.rel) == *rel).map(|at| root.modules.start + at), rel.clone()),
        };
        let Some(entry) = entry else {
            return Err(if options.entry.is_some() { format!("{file} is not a file of the package") } else { format!("{} has no {file}", manifest.name) });
        };
        let main = interner.get("main").and_then(|name| program.modules[ids[entry]].scope.get(&name).copied());
        match main {
            Some(Resolved::Def(def)) if plain(def) => {
                found.push(def);
                error_text_of_main = error_text(program, interner, def);
            }
            _ => return Err(format!("{file} has no `main`")),
        }
    }
    Ok(Entries { roots: found.into_iter().map(|def| Instance { def, args: Vec::new() }).collect(), error_text: error_text_of_main })
}

#[cfg(test)]
mod tests {
    use mote_ir::EntryKind;
    use mote_packages::testing::Scratch;

    use super::*;

    const HELLO: &str = "fn main() {\n    println(\"hi\")\n}\n";

    /// A package `app` with the files below its `src/`.
    fn app(files: &[(&str, &str)]) -> (Scratch, PathBuf) {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], files);
        (scratch, dir)
    }

    fn built(files: &[(&str, &str)], target: Target) -> Result<Pipeline, Failure> {
        let (_scratch, dir) = app(files);
        Pipeline::build(&dir, target, &Options::default())
    }

    fn failure(files: &[(&str, &str)], target: Target) -> String {
        match built(files, target) {
            Ok(_) => panic!("the build should stop"),
            Err(failure) => failure.to_string(),
        }
    }

    fn output_of(pipeline: &mut Pipeline) -> String {
        crate::run::run_main(pipeline).unwrap_or_else(|failure| panic!("{failure}")).output
    }

    #[test]
    fn pipeline_builds_a_package_that_runs() {
        let mut pipeline = built(&[("main.mote", HELLO)], Target::Run).unwrap();
        assert_eq!(output_of(&mut pipeline), "hi\n");
        assert_eq!(pipeline.warnings, Vec::<String>::new());
        assert_eq!(pipeline.packages.root().manifest.name, "app");
        assert_eq!(pipeline.ranges.len(), 2);
    }

    #[test]
    fn pipeline_output_is_the_same_with_and_without_optimizing() {
        let (_scratch, dir) = app(&[("main.mote", "fn main() {\n    var total = 0\n    for n in 1..5 {\n        total += n\n    }\n    println(total)\n}\n")]);
        let mut plain = Pipeline::build(&dir, Target::Run, &Options { optimize: false, ..Options::default() }).unwrap();
        let mut tuned = Pipeline::build(&dir, Target::Run, &Options::default()).unwrap();
        assert_eq!(output_of(&mut plain), output_of(&mut tuned));
        assert_eq!(output_of(&mut plain), "10\n");
    }

    #[test]
    fn pipeline_runs_the_main_of_the_entry_file_of_the_manifest() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("start.mote", HELLO), ("main.mote", "fn main() {\n    println(\"wrong\")\n}\n")]);
        scratch.write("app/mote.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/start.mote\"\n");
        let mut pipeline = Pipeline::build(&dir, Target::Run, &Options::default()).unwrap();
        assert_eq!(output_of(&mut pipeline), "hi\n");
    }

    #[test]
    fn pipeline_test_target_holds_the_test_blocks_of_every_file_and_no_main() {
        let mut pipeline = built(&[("main.mote", "import assert from std.test\n\nfn main() {\n    println(\"hi\")\n}\n\ntest \"one\" {\n    assert(1 == 1)\n}\n"), ("util.mote", "import assert from std.test\n\ntest \"two\" {\n    assert(2 == 2)\n}\n")], Target::Test).unwrap();
        let kinds: Vec<EntryKind> = pipeline.module.entries.iter().map(|entry| entry.kind).collect();
        assert!(kinds.iter().all(|kind| matches!(kind, EntryKind::Test(_))), "{kinds:?}");
        let run = crate::run::run_tests(&mut pipeline).unwrap();
        let outcomes: Vec<(&str, bool)> = run.tests.iter().map(|test| (test.name.as_str(), test.verdict == crate::run::Verdict::Passed)).collect();
        assert_eq!(outcomes, [("one", true), ("two", true)]);
    }

    #[test]
    fn pipeline_check_target_builds_every_plain_function_though_nothing_calls_it() {
        let pipeline = built(&[("main.mote", HELLO), ("util.mote", "fn unused() -> Int {\n    return 1\n}\n\nfn generic<T>(x: T) -> T {\n    return x\n}\n")], Target::Check).unwrap();
        let names: Vec<&str> = pipeline.program.order.iter().map(|instance| pipeline.interner.resolve(pipeline.program.defs[instance.def].name)).collect();
        assert!(names.contains(&"unused"), "{names:?}");
        assert!(!names.contains(&"generic"), "{names:?}");
    }

    #[test]
    fn pipeline_run_target_builds_only_what_main_reaches() {
        let pipeline = built(&[("main.mote", HELLO), ("util.mote", "fn unused() -> Int {\n    return 1\n}\n")], Target::Run).unwrap();
        let names: Vec<&str> = pipeline.program.order.iter().map(|instance| pipeline.interner.resolve(pipeline.program.defs[instance.def].name)).collect();
        assert!(!names.contains(&"unused"), "{names:?}");
    }

    #[test]
    fn pipeline_prints_a_diagnostic_of_the_checker_with_its_source_line() {
        let text = failure(&[("main.mote", "fn main() {\n    let x: Int = \"s\"\n}\n")], Target::Run);
        assert!(text.starts_with("error: "), "{text}");
        assert!(text.contains("src/main.mote:2:"), "{text}");
        assert!(text.contains("2 |     let x: Int = \"s\"\n"), "{text}");
    }

    #[test]
    fn pipeline_prints_every_error_not_only_the_first() {
        let text = failure(&[("main.mote", "fn main() {\n    let x: Int = \"s\"\n    let y: Bool = 1\n}\n")], Target::Run);
        assert_eq!(text.matches("error: ").count(), 2, "{text}");
        assert!(text.contains("3 |     let y: Bool = 1\n"), "{text}");
    }

    #[test]
    fn pipeline_a_syntax_error_stops_before_the_checker() {
        let text = failure(&[("main.mote", "fn main( {\n    let x: Int = \"s\"\n}\n")], Target::Run);
        assert!(text.contains("src/main.mote:1:"), "{text}");
        assert!(!text.contains("let x: Int"), "{text}");
    }

    #[test]
    fn pipeline_a_missing_module_is_printed_at_the_import() {
        let text = failure(&[("main.mote", "import .nope\n\nfn main() {\n}\n")], Target::Run);
        assert!(text.contains("error: no module `.nope`"), "{text}");
        assert!(text.contains("1 | import .nope\n"), "{text}");
    }

    #[test]
    fn pipeline_a_circular_import_is_printed_at_the_import() {
        let text = failure(&[("main.mote", "import .a\n\nfn main() {\n}\n"), ("a.mote", "import .b\n"), ("b.mote", "import .a\n")], Target::Run);
        assert!(text.contains("error: circular import app.a -> app.b -> app.a"), "{text}");
        assert!(text.contains("1 | import .a\n"), "{text}");
    }

    #[test]
    fn pipeline_a_native_that_cannot_cross_to_c_is_printed_at_the_native_fn() {
        let text = failure(&[("main.mote", "extern \"c\" {\n    fn f(items: [String]) -> Int\n}\n\nfn main() {\n    println(f([\"a\"]))\n}\n")], Target::Run);
        assert!(text.contains("error: parameter `items` of type `[String]` has no C type"), "{text}");
        assert!(text.contains("2 |     fn f(items: [String]) -> Int\n"), "{text}");
    }

    #[test]
    fn pipeline_prints_a_warning_of_a_build_that_succeeds() {
        let pipeline = built(&[("main.mote", "fn main() {\n}\n\n/// stray\n")], Target::Run).unwrap();
        assert_eq!(pipeline.warnings.len(), 1);
        assert!(pipeline.warnings[0].starts_with("warning: this doc comment documents nothing\n"), "{}", pipeline.warnings[0]);
    }

    #[test]
    fn pipeline_prints_a_manifest_error_with_its_file_and_line() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", HELLO)]);
        scratch.write("app/mote.toml", "[package]\nname = \"app\"\nversion = 3\n");
        let text = Pipeline::build(&dir, Target::Run, &Options::default()).err().unwrap().to_string();
        assert_eq!(text, format!("error: {}:3: `package.version`: expected a string, found an integer\n", mote_packages::shown(&dir.join("mote.toml"))));
    }

    #[test]
    fn pipeline_a_package_that_needs_a_newer_mote_is_an_error() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", HELLO)]);
        scratch.write("app/mote.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nmote = \"99.0.0\"\n");
        let text = Pipeline::build(&dir, Target::Run, &Options::default()).err().unwrap().to_string();
        assert_eq!(text, format!("error: {}:4: `package.mote`: needs mote 99.0.0, this is {}\n", mote_packages::shown(&dir.join("mote.toml")), toolchain()));
    }

    #[test]
    fn pipeline_a_dependency_is_built_with_the_program() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[("gadget", "../gadget")], &[("main.mote", "import gadget\n\nfn main() {\n    println(gadget.title())\n}\n")]);
        scratch.package("gadget", "gadget", &[], &[("lib.mote", "pub fn title() -> String {\n    return \"gadget\"\n}\n")]);
        let mut pipeline = Pipeline::build(&dir, Target::Run, &Options::default()).unwrap();
        assert_eq!(output_of(&mut pipeline), "gadget\n");
        assert_eq!(pipeline.ranges.len(), 3);
    }

    #[test]
    fn pipeline_a_type_error_in_a_dependency_is_printed_with_the_dependency_file() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[("gadget", "../gadget")], &[("main.mote", "import gadget\n\nfn main() {\n    println(gadget.title())\n}\n")]);
        scratch.package("gadget", "gadget", &[], &[("lib.mote", "pub fn title() -> String {\n    return 1\n}\n")]);
        let text = Pipeline::build(&dir, Target::Run, &Options::default()).err().unwrap().to_string();
        assert!(text.contains("gadget/src/lib.mote:2:"), "{text}");
    }

    #[test]
    fn pipeline_run_needs_an_entry_file_with_a_main() {
        assert_eq!(failure(&[("util.mote", "fn f() {\n}\n")], Target::Run), "error: app has no src/main.mote\n");
        assert_eq!(failure(&[("main.mote", "fn other() {\n}\n")], Target::Run), "error: src/main.mote has no `main`\n");
    }

    #[test]
    fn pipeline_the_entry_option_runs_the_main_of_another_file() {
        let (_scratch, dir) = app(&[("main.mote", HELLO), ("tool.mote", "fn main() {\n    println(\"tool\")\n}\n")]);
        let options = Options { entry: Some("src/tool.mote".into()), ..Options::default() };
        let mut pipeline = Pipeline::build(&dir, Target::Run, &options).unwrap();
        assert_eq!(output_of(&mut pipeline), "tool\n");
    }

    #[test]
    fn pipeline_the_entry_option_names_a_file_that_is_not_in_the_package() {
        let (_scratch, dir) = app(&[("main.mote", HELLO)]);
        let options = Options { entry: Some("src/none.mote".into()), ..Options::default() };
        let text = Pipeline::build(&dir, Target::Run, &options).err().unwrap().to_string();
        assert_eq!(text, "error: src/none.mote is not a file of the package\n");
        let bare = Options { entry: Some("src/util.mote".into()), ..Options::default() };
        let (_other, again) = app(&[("main.mote", HELLO), ("util.mote", "fn f() {\n}\n")]);
        assert_eq!(Pipeline::build(&again, Target::Run, &bare).err().unwrap().to_string(), "error: src/util.mote has no `main`\n");
    }

    #[test]
    fn pipeline_the_filter_option_keeps_the_test_blocks_whose_name_contains_the_text() {
        let source = "test \"parse numbers\" {\n}\n\ntest \"parse words\" {\n}\n\ntest \"print\" {\n}\n";
        let (_scratch, dir) = app(&[("main.mote", source)]);
        let count = |filter: Option<&str>| {
            let options = Options { filter: filter.map(str::to_string), ..Options::default() };
            Pipeline::build(&dir, Target::Test, &options).unwrap().module.entries.len()
        };
        assert_eq!((count(None), count(Some("parse")), count(Some("numbers")), count(Some("zzz"))), (3, 2, 1, 0));
    }

    const PASSING: &str = "import assert from std.test\n\ntest \"in tests\" {\n    assert(1 == 1)\n}\n";

    /// The names of the test blocks the pipeline holds.
    fn test_names(pipeline: &mut Pipeline) -> Vec<String> {
        crate::run::run_tests(pipeline).unwrap().tests.iter().map(|test| test.name.clone()).collect()
    }

    #[test]
    fn pipeline_test_and_check_targets_include_the_test_blocks_of_tests() {
        let (scratch, dir) = app(&[("main.mote", HELLO)]);
        scratch.write("app/tests/a.mote", PASSING);
        scratch.write("app/tests/sub/b.mote", "import assert from std.test\nimport ..a\n\ntest \"deeper\" {\n    assert(true)\n}\n");
        for target in [Target::Test, Target::Check] {
            let mut pipeline = Pipeline::build(&dir, target, &Options::default()).unwrap();
            assert_eq!(test_names(&mut pipeline), ["in tests", "deeper"], "{target:?}");
            assert_eq!(pipeline.ranges[1].tests.len(), 2);
        }
    }

    #[test]
    fn pipeline_run_target_leaves_tests_out_even_when_they_do_not_compile() {
        let (scratch, dir) = app(&[("main.mote", HELLO)]);
        scratch.write("app/tests/broken.mote", "fn (\n");
        let mut pipeline = Pipeline::build(&dir, Target::Run, &Options::default()).unwrap();
        assert_eq!(output_of(&mut pipeline), "hi\n");
        assert_eq!(pipeline.ranges[1].tests.len(), 0);
        let text = Pipeline::build(&dir, Target::Test, &Options::default()).err().unwrap().to_string();
        assert!(text.contains("tests/broken.mote"), "{text}");
    }

    #[test]
    fn pipeline_a_test_file_imports_the_package_it_tests() {
        let (scratch, dir) = app(&[("lib.mote", "pub fn two() -> Int {\n    return 2\n}\n"), ("main.mote", HELLO)]);
        scratch.write("app/tests/uses.mote", "import assert_eq from std.test\nimport two from app\n\ntest \"two\" {\n    assert_eq(two(), 2)\n}\n");
        let mut pipeline = Pipeline::build(&dir, Target::Test, &Options::default()).unwrap();
        assert_eq!(test_names(&mut pipeline), ["two"]);
    }

    #[test]
    fn pipeline_a_std_directory_that_is_not_there_is_an_error() {
        let (_scratch, dir) = app(&[("main.mote", HELLO)]);
        let options = Options { std: Some(PathBuf::from("/nonexistent/std")), ..Options::default() };
        let text = Pipeline::build(&dir, Target::Run, &options).err().unwrap().to_string();
        assert!(text.starts_with("error: /nonexistent/std/mote.toml: cannot read"), "{text}");
    }

    #[test]
    fn pipeline_reflect_show_on_multi_field_struct_unrolls_statically() {
        let source = "import show from std.reflect\n\nstruct Point {\n    x: Int\n    y: Int\n    label: String\n}\n\nfn main() {\n    println(show(Point { x: 1, y: 2, label: \"hi\" }))\n}\n";
        let pipeline = built(&[("main.mote", source)], Target::Run).unwrap();
        let show_ir: String = pipeline
            .module
            .functions
            .iter()
            .filter_map(|(_, func)| match &func.kind {
                mote_ir::FunctionKind::Instance(inst)
                    if pipeline.interner.resolve(pipeline.program.defs[inst.def].name) == "show_nested" =>
                {
                    Some(mote_ir::print_function(func, &pipeline.program, &pipeline.interner))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(show_ir.contains("field r0.0") && show_ir.contains("field r0.1") && show_ir.contains("field r0.2"), "{show_ir}");
        assert!(show_ir.contains("\"x\"") && show_ir.contains("\"y\"") && show_ir.contains("\"label\""), "{show_ir}");
        assert!(!show_ir.contains("meta_ints") && !show_ir.contains("meta_texts"), "{show_ir}");
    }

    #[test]
    fn pipeline_dependent_const_generic_array_length_evaluates_and_runs() {
        let source = "fn padded<T, const N: Int>(fill: T) -> [T; N + 1] {\n    return [fill; N + 1]\n}\n\nfn main() {\n    let arr = padded<Int, 2>(9)\n    println(arr[0])\n    println(arr[2])\n}\n";
        let mut pipeline = built(&[("main.mote", source)], Target::Run).unwrap();
        assert_eq!(output_of(&mut pipeline), "9\n9\n");
    }
}
