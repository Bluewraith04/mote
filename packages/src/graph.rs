//! Parses the files of the packages of a build and resolves each import to a module.

use std::collections::HashMap;
use std::ops::Range;

use mote_base::{Diagnostic, DiagnosticSink, Interner, SourceMap};
use mote_semantics::{Input, ModuleInput};
use mote_syntax::{lex, parse, Import, ItemKind, ModulePath, PathPrefix};

use crate::package::{Packages, SourceFile};

/// The modules of every package as a resolver input, with what parsed them.
pub struct Graph {
    pub input: Input,
    pub interner: Interner,
    pub sink: DiagnosticSink,
    pub map: SourceMap,
    /// One per package of [`Packages::list`].
    pub packages: Vec<PackageModules>,
}

/// Where one package's modules are in [`Input::modules`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageModules {
    pub name: String,
    /// The files of `src/`, then the files of `tests/` when the graph holds them.
    pub modules: Range<usize>,
    /// The files of `tests/`: the end of `modules`.
    pub tests: Range<usize>,
    /// `src/lib.mote`, the module a dependent imports by the package name.
    pub lib: Option<usize>,
    /// The file the manifest's `entry` names.
    pub entry: Option<usize>,
}

/// Where a module sits in its package.
struct Place {
    package: usize,
    /// The directories below `src/`, or below `tests/`.
    dir: Vec<String>,
    /// Whether the file is under `tests/`.
    tests: bool,
}

/// A module by its package, whether it is under `tests/`, and its path below `src/` or `tests/`.
type Keys = HashMap<(usize, bool, Vec<String>), usize>;

fn written(path: &ModulePath, interner: &Interner) -> String {
    let mut text = String::new();
    for prefix in &path.prefix {
        text.push_str(match prefix {
            PathPrefix::Dot => ".",
            PathPrefix::DotDot => "..",
            PathPrefix::Super => "super.",
        });
    }
    let names: Vec<&str> = path.segments.iter().map(|segment| interner.resolve(segment.name)).collect();
    text + &names.join(".")
}

/// What parsing the files builds up.
#[derive(Default)]
struct Parts {
    map: SourceMap,
    interner: Interner,
    sink: DiagnosticSink,
    modules: Vec<ModuleInput>,
    places: Vec<Place>,
    keys: Keys,
}

impl Parts {
    /// Parses the file into a module of the package at `index` and returns its position; `tests` says it is under `tests/`.
    fn add(&mut self, packages: &Packages, index: usize, tests: bool, file: &SourceFile) -> usize {
        let package = &packages.list[index];
        let id = self.map.add_file(file.name.clone(), file.text.clone());
        let lexed = lex(self.map.file(id), &mut self.interner, &mut self.sink);
        let ast = parse(self.map.file(id), &lexed, &mut self.interner, &mut self.sink);
        let names: Vec<String> = file.rel.strip_suffix(".mote").unwrap_or(&file.rel).split('/').map(str::to_string).collect();
        let top = names.len() == 1;
        let path: Vec<String> = if tests {
            [package.manifest.name.clone(), "tests".to_string()].into_iter().chain(names.iter().cloned()).collect()
        } else if top && names[0] == "lib" {
            vec![package.manifest.name.clone()]
        } else if index == 0 && top && names[0] == "prelude" {
            vec!["prelude".to_string()]
        } else {
            std::iter::once(package.manifest.name.clone()).chain(names.iter().cloned()).collect()
        };
        let at = self.modules.len();
        self.keys.insert((index, tests, names.clone()), at);
        self.places.push(Place { package: index, dir: names[..names.len() - 1].to_vec(), tests });
        let path = path.iter().map(|part| self.interner.intern(part)).collect();
        self.modules.push(ModuleInput { path, ast, is_std: index == 0, imports: HashMap::new(), source: file.text.clone(), source_start: self.map.file(id).start });
        at
    }
}

struct Resolver<'a> {
    packages: &'a Packages,
    keys: &'a Keys,
    interner: &'a Interner,
}

impl Resolver<'_> {
    /// The module the import names from the module at `from`, or why there is none.
    fn target(&self, from: &Place, path: &ModulePath) -> Result<usize, String> {
        let names: Vec<String> = path.segments.iter().map(|segment| self.interner.resolve(segment.name).to_string()).collect();
        if path.prefix.is_empty() {
            let first = &names[0];
            let package = if first == "std" {
                0
            } else if from.tests && *first == self.packages.list[from.package].manifest.name {
                from.package
            } else {
                match self.packages.links[from.package].get(first) {
                    Some(&package) => package,
                    None => return Err(format!("no package `{first}`")),
                }
            };
            if package != from.package && names[1..].iter().any(|name| name.starts_with('_')) {
                return Err(format!("`{}` is private to `{first}`", names.join(".")));
            }
            return self.keys.get(&(package, false, names[1..].to_vec())).copied().ok_or_else(|| if names.len() == 1 { format!("`{first}` has no src/lib.mote") } else { format!("no module `{}`", names.join(".")) });
        }
        let ups = path.prefix.iter().filter(|prefix| !matches!(prefix, PathPrefix::Dot)).count();
        if ups > from.dir.len() {
            return Err(format!("`{}` is above the package", written(path, self.interner)));
        }
        let mut key = from.dir[..from.dir.len() - ups].to_vec();
        key.extend(names);
        self.keys.get(&(from.package, from.tests, key)).copied().ok_or_else(|| format!("no module `{}`", written(path, self.interner)))
    }
}

/// Parses every file of every package, and the files of the root's `tests/` when `with_tests` is set; an import with no module is an error at the import.
pub fn build(packages: &Packages, with_tests: bool) -> Graph {
    let mut parts = Parts::default();
    let mut prelude = None;
    let mut ranges = Vec::new();
    let root = packages.list.len() - 1;
    for (index, package) in packages.list.iter().enumerate() {
        let start = parts.modules.len();
        let (mut lib, mut entry) = (None, None);
        for file in &package.files {
            let at = parts.add(packages, index, false, file);
            let names: Vec<&str> = file.rel.strip_suffix(".mote").unwrap_or(&file.rel).split('/').collect();
            if names == ["lib"] {
                lib = Some(at);
                parts.keys.insert((index, false, Vec::new()), at);
            }
            if index == 0 && names == ["prelude"] {
                prelude = Some(at);
            }
            if format!("src/{}", file.rel) == package.manifest.entry {
                entry = Some(at);
            }
        }
        let tests_start = parts.modules.len();
        if with_tests && index == root {
            for file in &package.tests {
                parts.add(packages, index, true, file);
            }
        }
        ranges.push(PackageModules { name: package.manifest.name.clone(), modules: start..parts.modules.len(), tests: tests_start..parts.modules.len(), lib, entry });
    }
    let Parts { map, interner, mut sink, mut modules, places, keys } = parts;
    let resolver = Resolver { packages, keys: &keys, interner: &interner };
    let mut found = Vec::new();
    for (at, module) in modules.iter().enumerate() {
        let mut imports = HashMap::new();
        for &item in &module.ast.root {
            let path = match &module.ast[item].kind {
                ItemKind::Import(Import::Module { path, .. }) => path,
                ItemKind::Import(Import::Names { from, .. } | Import::Glob { from }) => from,
                _ => continue,
            };
            match resolver.target(&places[at], path) {
                Ok(target) => {
                    imports.insert(item, target);
                }
                Err(message) => sink.emit(Diagnostic::error(message, path.span)),
            }
        }
        found.push(imports);
    }
    for (module, imports) in modules.iter_mut().zip(found) {
        module.imports = imports;
    }
    Graph { input: Input { modules, prelude }, interner, sink, map, packages: ranges }
}

#[cfg(test)]
mod tests {
    use mote_base::Severity;
    use mote_semantics::resolve;

    use super::*;
    use crate::package::Package;
    use crate::testing::{pkg, std_package};

    fn std_pkg() -> Package {
        pkg("std", &[], &[("lib.mote", ""), ("prelude.mote", ""), ("string.mote", "pub fn len() {}"), ("sub/deep.mote", "")])
    }

    fn build_of(root: Package, others: Vec<Package>) -> Graph {
        build(&Packages::new(std_pkg(), others, root).unwrap(), false)
    }

    fn texts(graph: &Graph) -> Vec<String> {
        graph.sink.iter().filter(|diagnostic| diagnostic.severity == Severity::Error).map(|diagnostic| diagnostic.message.clone()).collect()
    }

    fn path_of(graph: &Graph, index: usize) -> String {
        graph.input.modules[index].path.iter().map(|&part| graph.interner.resolve(part)).collect::<Vec<_>>().join(".")
    }

    /// The module each root import item of `module` leads to, as dotted paths in source order.
    fn leads_to(graph: &Graph, module: usize) -> Vec<String> {
        let input = &graph.input.modules[module];
        input.ast.root.iter().filter_map(|item| input.imports.get(item)).map(|&target| path_of(graph, target)).collect()
    }

    #[test]
    fn graph_names_modules_from_their_paths() {
        let graph = build_of(pkg("app", &[], &[("lib.mote", ""), ("main.mote", ""), ("util.mote", ""), ("sub/worker.mote", "")]), Vec::new());
        let names: Vec<String> = (0..graph.input.modules.len()).map(|index| path_of(&graph, index)).collect();
        assert_eq!(names, ["std", "prelude", "std.string", "std.sub.deep", "app", "app.main", "app.sub.worker", "app.util"]);
        assert_eq!(graph.input.prelude, Some(1));
        let std_flags: Vec<bool> = graph.input.modules.iter().map(|module| module.is_std).collect();
        assert_eq!(std_flags, [true, true, true, true, false, false, false, false]);
    }

    #[test]
    fn graph_records_each_package_range_lib_and_entry() {
        let graph = build_of(pkg("app", &[], &[("lib.mote", ""), ("main.mote", ""), ("util.mote", "")]), Vec::new());
        assert_eq!(graph.packages[0], PackageModules { name: "std".into(), modules: 0..4, tests: 4..4, lib: Some(0), entry: None });
        assert_eq!(graph.packages[1], PackageModules { name: "app".into(), modules: 4..7, tests: 7..7, lib: Some(4), entry: Some(5) });
        let no_entry = build_of(pkg("app", &[], &[("util.mote", "")]), Vec::new());
        assert_eq!((no_entry.packages[1].lib, no_entry.packages[1].entry), (None, None));
    }

    #[test]
    fn graph_resolves_a_dot_import_to_the_sibling_file() {
        let graph = build_of(pkg("app", &[], &[("main.mote", "import .util\nimport { f } from .util\nimport { * } from .sub.worker\n"), ("util.mote", ""), ("sub/worker.mote", "")]), Vec::new());
        assert_eq!(texts(&graph), Vec::<String>::new());
        assert_eq!(leads_to(&graph, graph.packages[1].entry.unwrap()), ["app.util", "app.util", "app.sub.worker"]);
    }

    #[test]
    fn graph_resolves_dot_dot_and_super_to_the_directory_above() {
        let worker = "import ..util\nimport super.util\nimport .peer\n";
        let graph = build_of(pkg("app", &[], &[("util.mote", ""), ("sub/worker.mote", worker), ("sub/peer.mote", "")]), Vec::new());
        assert_eq!(texts(&graph), Vec::<String>::new());
        let at = (0..graph.input.modules.len()).find(|&index| path_of(&graph, index) == "app.sub.worker").unwrap();
        assert_eq!(leads_to(&graph, at), ["app.util", "app.util", "app.sub.peer"]);
    }

    #[test]
    fn graph_resolves_std_imports_in_every_package() {
        let app = pkg("app", &["pane"], &[("main.mote", "import std\nimport std.string as s\nimport { len } from std.string\nimport std.sub.deep\n")]);
        let pane = pkg("pane", &[], &[("lib.mote", "import std.string\n")]);
        let graph = build_of(app, vec![pane]);
        assert_eq!(texts(&graph), Vec::<String>::new());
        let main = graph.packages[2].entry.unwrap();
        assert_eq!(leads_to(&graph, main), ["std", "std.string", "std.string", "std.sub.deep"]);
        assert_eq!(leads_to(&graph, graph.packages[1].lib.unwrap()), ["std.string"]);
    }

    #[test]
    fn graph_resolves_a_dependency_name_to_its_lib_and_its_files() {
        let app = pkg("app", &["pane"], &[("main.mote", "import pane\nimport pane.nav\nimport { go } from pane.nav\n")]);
        let pane = pkg("pane", &[], &[("lib.mote", ""), ("nav.mote", "")]);
        let graph = build_of(app, vec![pane]);
        assert_eq!(texts(&graph), Vec::<String>::new());
        assert_eq!(leads_to(&graph, graph.packages[2].entry.unwrap()), ["pane", "pane.nav", "pane.nav"]);
    }

    #[test]
    fn graph_a_missing_module_is_an_error_at_the_import() {
        let graph = build_of(pkg("app", &[], &[("main.mote", "import .util\n\nimport .nope\nimport std.nope\nimport ..up\n"), ("util.mote", "")]), Vec::new());
        assert_eq!(texts(&graph), ["no module `.nope`", "no module `std.nope`", "`..up` is above the package"]);
        let first = graph.sink.iter().next().unwrap();
        let at = graph.map.lookup(first.span.start).unwrap();
        assert_eq!((graph.map.file(at.file).name.as_str(), at.line), ("app/src/main.mote", 3));
        assert_eq!(graph.map.snippet(first.span), Some(".nope"));
    }

    #[test]
    fn graph_a_missing_package_or_lib_is_an_error_at_the_import() {
        let app = pkg("app", &["pane"], &[("main.mote", "import ghost\nimport ghost.x\nimport pane\nimport pane.gone\n")]);
        let pane = pkg("pane", &[], &[("other.mote", "")]);
        let graph = build_of(app, vec![pane]);
        assert_eq!(texts(&graph), ["no package `ghost`", "no package `ghost`", "`pane` has no src/lib.mote", "no module `pane.gone`"]);
    }

    #[test]
    fn graph_a_folder_module_is_the_file_of_its_name_beside_it() {
        let app = pkg("app", &[], &[("main.mote", "import .sub\nimport .sub.worker\n"), ("sub.mote", ""), ("sub/worker.mote", "")]);
        let graph = build_of(app, Vec::new());
        assert_eq!(texts(&graph), Vec::<String>::new());
        assert_eq!(leads_to(&graph, graph.packages[1].entry.unwrap()), ["app.sub", "app.sub.worker"]);
    }

    #[test]
    fn graph_a_file_or_folder_that_starts_with_an_underscore_is_private_to_its_package() {
        let app = pkg("app", &["pane"], &[("main.mote", "import pane._native\nimport pane._impl.deep\nimport pane.nav\nimport pane\n")]);
        let pane = pkg("pane", &[], &[("lib.mote", "import ._native\nimport ._impl.deep\n"), ("_native.mote", ""), ("_impl/deep.mote", ""), ("nav.mote", "")]);
        let graph = build_of(app, vec![pane]);
        assert_eq!(texts(&graph), ["`pane._native` is private to `pane`", "`pane._impl.deep` is private to `pane`"]);
        assert_eq!(leads_to(&graph, graph.packages[1].lib.unwrap()), ["pane._native", "pane._impl.deep"]);
    }

    #[test]
    fn graph_a_package_sees_its_own_dependencies_and_not_theirs() {
        let app = pkg("app", &["pane"], &[("main.mote", "import pane\nimport wire\n")]);
        let pane = pkg("pane", &["wire"], &[("lib.mote", "import wire\n")]);
        let wire = pkg("wire", &[], &[("lib.mote", "")]);
        let graph = build_of(app, vec![wire, pane]);
        assert_eq!(texts(&graph), ["no package `wire`"]);
        assert_eq!(leads_to(&graph, graph.packages[2].lib.unwrap()), ["wire"]);
    }

    #[test]
    fn graph_a_package_does_not_import_itself_by_name() {
        let graph = build_of(pkg("app", &[], &[("main.mote", "import app.util\n"), ("util.mote", "")]), Vec::new());
        assert_eq!(texts(&graph), ["no package `app`"]);
    }

    /// The root `app` with `src` files and `tests` files, built with its tests.
    fn build_with_tests(files: &[(&str, &str)], tests: &[(&str, &str)]) -> Graph {
        let tests = tests.iter().map(|(rel, text)| SourceFile { rel: rel.to_string(), name: format!("app/tests/{rel}"), text: text.to_string() }).collect();
        build(&Packages::new(std_pkg(), Vec::new(), pkg("app", &[], files).with_tests(tests)).unwrap(), true)
    }

    #[test]
    fn graph_tests_are_modules_of_the_root_under_tests() {
        let graph = build_with_tests(&[("lib.mote", ""), ("main.mote", "")], &[("a.mote", ""), ("sub/b.mote", "")]);
        let names: Vec<String> = (4..graph.input.modules.len()).map(|index| path_of(&graph, index)).collect();
        assert_eq!(names, ["app", "app.main", "app.tests.a", "app.tests.sub.b"]);
        assert_eq!(graph.packages[1].modules, 4..8);
        assert_eq!(graph.packages[1].tests, 6..8);
        assert_eq!(graph.packages[0].tests, 4..4);
    }

    #[test]
    fn graph_tests_are_left_out_unless_asked_for() {
        let tests = vec![SourceFile { rel: "a.mote".into(), name: "app/tests/a.mote".into(), text: "fn (".into() }];
        let root = pkg("app", &[], &[("main.mote", "")]).with_tests(tests);
        let graph = build(&Packages::new(std_pkg(), Vec::new(), root).unwrap(), false);
        assert_eq!((graph.input.modules.len(), graph.packages[1].tests.clone()), (5, 5..5));
        assert!(texts(&graph).is_empty());
    }

    #[test]
    fn graph_a_dependency_has_no_tests_in_the_graph() {
        let pane = pkg("pane", &[], &[("lib.mote", "")]).with_tests(vec![SourceFile { rel: "t.mote".into(), name: "pane/tests/t.mote".into(), text: String::new() }]);
        let app = pkg("app", &["pane"], &[("main.mote", "import pane\n")]);
        let graph = build(&Packages::new(std_pkg(), vec![pane], app).unwrap(), true);
        assert_eq!(graph.input.modules.len(), 6);
        assert_eq!(graph.packages[1].tests, 5..5);
    }

    #[test]
    fn graph_a_test_file_imports_the_package_by_name_and_its_siblings_by_dot() {
        let main = "pub fn f() {}\n";
        let graph = build_with_tests(&[("lib.mote", ""), ("main.mote", main), ("util.mote", "")], &[("a.mote", "import app\nimport app.util\nimport .b\nimport std.string\n"), ("b.mote", ""), ("sub/c.mote", "import ..a\nimport super.b\n")]);
        assert_eq!(texts(&graph), Vec::<String>::new());
        let at = |name: &str| (0..graph.input.modules.len()).find(|&index| path_of(&graph, index) == name).unwrap();
        assert_eq!(leads_to(&graph, at("app.tests.a")), ["app", "app.util", "app.tests.b", "std.string"]);
        assert_eq!(leads_to(&graph, at("app.tests.sub.c")), ["app.tests.a", "app.tests.b"]);
    }

    #[test]
    fn graph_a_test_file_does_not_reach_src_by_dot_and_src_does_not_reach_tests() {
        let graph = build_with_tests(&[("main.mote", "import .a\nimport app.tests.a\n"), ("util.mote", "")], &[("a.mote", "import .util\n")]);
        assert_eq!(texts(&graph), ["no module `.a`", "no package `app`", "no module `.util`"]);
    }

    #[test]
    fn graph_a_file_outside_tests_still_cannot_import_its_own_package_by_name() {
        let graph = build_with_tests(&[("main.mote", "import app.util\n"), ("util.mote", "")], &[("a.mote", "import app.util\n")]);
        assert_eq!(texts(&graph), ["no package `app`"]);
    }

    #[test]
    fn graph_a_cycle_between_test_files_is_an_error() {
        let graph = build_with_tests(&[("main.mote", "")], &[("a.mote", "import .b\n"), ("b.mote", "import .a\n")]);
        let Graph { input, mut interner, mut sink, .. } = graph;
        let _program = resolve(input, &mut interner, &mut sink);
        let errors: Vec<String> = sink.iter().map(|diagnostic| diagnostic.message.clone()).collect();
        assert_eq!(errors, ["circular import app.tests.a -> app.tests.b -> app.tests.a"]);
    }

    #[test]
    fn graph_lib_is_also_reachable_as_a_sibling() {
        let graph = build_of(pkg("app", &[], &[("lib.mote", ""), ("main.mote", "import .lib\n")]), Vec::new());
        assert_eq!(texts(&graph), Vec::<String>::new());
        assert_eq!(leads_to(&graph, graph.packages[1].entry.unwrap()), ["app"]);
    }

    #[test]
    fn graph_a_cycle_of_imports_is_an_error_at_the_closing_import() {
        let graph = build_of(pkg("app", &[], &[("a.mote", "import .b\n"), ("b.mote", "import .a\n")]), Vec::new());
        assert_eq!(texts(&graph), Vec::<String>::new());
        let Graph { input, mut interner, mut sink, map, .. } = graph;
        let _program = resolve(input, &mut interner, &mut sink);
        let errors: Vec<String> = sink.iter().map(|diagnostic| diagnostic.message.clone()).collect();
        assert_eq!(errors, ["circular import app.a -> app.b -> app.a"]);
        let span = sink.iter().next().unwrap().span;
        assert_eq!(map.snippet(span), Some("import .a"));
    }

    #[test]
    fn graph_a_syntax_error_in_a_file_is_in_the_sink() {
        let graph = build_of(pkg("app", &[], &[("main.mote", "fn (\n")]), Vec::new());
        assert!(!texts(&graph).is_empty());
    }

    #[test]
    fn graph_names_the_modules_of_the_real_std_as_the_semantics_harness_does() {
        let std = std_package();
        let root = pkg("app", &[], &[("main.mote", "import std.string\nimport std.iter\n")]);
        let graph = build(&Packages::new(std, Vec::new(), root).unwrap(), false);
        assert_eq!(texts(&graph), Vec::<String>::new());
        let mut ours: Vec<String> = (0..graph.packages[0].modules.end).map(|index| path_of(&graph, index)).collect();
        let mut theirs: Vec<String> = mote_semantics::std_sources().into_iter().map(|(path, _)| path).collect();
        ours.sort();
        theirs.sort();
        assert_eq!(ours, theirs);
        let prelude = graph.input.prelude.expect("a prelude");
        assert_eq!(path_of(&graph, prelude), "prelude");
    }
}
