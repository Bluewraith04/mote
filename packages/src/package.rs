//! A package on disk or in memory, and the set of packages one build uses.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::Cache;
use crate::manifest::{is_name, Manifest, PackageError, Source};

/// Names no package may take: `std` is the standard library and `prelude` is its prelude module.
pub const RESERVED: [&str; 2] = ["std", "prelude"];

/// Whether no package may take the name: a reserved name, or the name of a core library in `mote_std`.
pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name) || crate::libraries::core_names().iter().any(|found| found == name)
}

/// How deep `src/` may nest.
const MAX_DEPTH: usize = 32;

/// One `.mote` file of a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    /// The path below `src/` (or `tests/`) with `/` between names, such as `sub/worker.mote`.
    pub rel: String,
    /// What a diagnostic calls the file.
    pub name: String,
    pub text: String,
}

/// A manifest and the `.mote` files under its `src/` and `tests/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub manifest: Manifest,
    pub dir: PathBuf,
    /// What an error calls the manifest.
    pub manifest_file: String,
    /// Under `src/`, in path order.
    pub files: Vec<SourceFile>,
    /// Under `tests/`, in path order.
    pub tests: Vec<SourceFile>,
}

/// The path as an error or a diagnostic names it: without a leading `./`, using `/`.
pub fn shown(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    text.strip_prefix("./").map(str::to_string).unwrap_or(text)
}

fn io(path: &Path, error: std::io::Error) -> PackageError {
    PackageError { file: shown(path), line: 0, key: String::new(), message: format!("cannot read: {error}") }
}

fn bad_file(path: &Path, message: String) -> PackageError {
    PackageError { file: shown(path), line: 0, key: String::new(), message }
}

impl Package {
    /// A package from parts that are not on disk; `dir` names it in errors.
    pub fn new(manifest: Manifest, dir: impl Into<PathBuf>, mut files: Vec<SourceFile>) -> Package {
        let dir = dir.into();
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        let manifest_file = shown(&dir.join("mote.toml"));
        Package { manifest, dir, manifest_file, files, tests: Vec::new() }
    }

    /// The package with these files as its `tests/`.
    pub fn with_tests(mut self, mut tests: Vec<SourceFile>) -> Package {
        tests.sort_by(|a, b| a.rel.cmp(&b.rel));
        self.tests = tests;
        self
    }

    /// Reads `mote.toml` and every `.mote` file under `src/` and `tests/` of the directory.
    pub fn load(dir: &Path) -> Result<Package, PackageError> {
        let manifest = Manifest::read(&dir.join("mote.toml"))?;
        let (mut files, mut tests) = (Vec::new(), Vec::new());
        for (folder, found) in [("src", &mut files), ("tests", &mut tests)] {
            let path = dir.join(folder);
            if path.is_dir() {
                walk(&path, "", found, 0)?;
            }
        }
        Ok(Package::new(manifest, dir, files).with_tests(tests))
    }
}

fn walk(dir: &Path, rel: &str, files: &mut Vec<SourceFile>, depth: usize) -> Result<(), PackageError> {
    if depth > MAX_DEPTH {
        return Err(bad_file(dir, format!("nested deeper than {MAX_DEPTH} directories")));
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir).map_err(|error| io(dir, error))?.map(|entry| entry.map(|entry| entry.path()).map_err(|error| io(dir, error))).collect::<Result<_, _>>()?;
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| bad_file(&path, "not a UTF-8 name".into()))?;
        if path.is_dir() {
            if !is_name(name) {
                return Err(bad_file(&path, format!("`{name}` is not a module name: letters, digits and `_`, not starting with a digit")));
            }
            walk(&path, &format!("{rel}{name}/"), files, depth + 1)?;
        } else if let Some(stem) = name.strip_suffix(".mote") {
            if !is_name(stem) {
                return Err(bad_file(&path, format!("`{name}` is not a module name: letters, digits and `_`, not starting with a digit")));
            }
            let text = fs::read_to_string(&path).map_err(|error| io(&path, error))?;
            files.push(SourceFile { rel: format!("{rel}{name}"), name: shown(&path), text });
        }
    }
    Ok(())
}

/// The packages of one build: the std first, then the dependencies with each before the packages that use it, then the root.
#[derive(Clone, Debug)]
pub struct Packages {
    pub list: Vec<Package>,
    /// Per package, the index of each package it imports by name; the std is not in it.
    pub links: Vec<HashMap<String, usize>>,
}

impl Packages {
    /// The set the root reaches through `dependencies`, matched to the packages by name, which are all different; a package of `others` the root does not reach is left out.
    pub fn new(std: Package, others: Vec<Package>, root: Package) -> Result<Packages, PackageError> {
        let mut by_name: HashMap<String, usize> = HashMap::new();
        let mut all = vec![root];
        all.extend(others);
        for (index, package) in all.iter().enumerate() {
            let name = &package.manifest.name;
            if RESERVED.contains(&name.as_str()) {
                return Err(PackageError { file: package.manifest_file.clone(), line: package.manifest.name_line, key: "package.name".into(), message: format!("`{name}` is reserved") });
            }
            if let Some(&first) = by_name.get(name) {
                let other = shown(&all[first].dir);
                return Err(PackageError { file: package.manifest_file.clone(), line: package.manifest.name_line, key: "package.name".into(), message: format!("also the name of the package in {other}") });
            }
            by_name.insert(name.clone(), index);
        }
        let mut order = Vec::new();
        let mut state = vec![Visit::New; all.len()];
        let mut path = Vec::new();
        visit(0, &all, &by_name, &mut state, &mut path, &mut order)?;
        let mut position = vec![usize::MAX; all.len()];
        let mut slots: Vec<Option<Package>> = all.into_iter().map(Some).collect();
        let mut list = vec![std];
        for &old in &order {
            position[old] = list.len();
            list.push(slots[old].take().expect("each package is placed once"));
        }
        let mut links = vec![HashMap::new()];
        for package in &list[1..] {
            let linked = package.manifest.dependencies.iter().map(|dep| (dep.name.clone(), position[by_name[&dep.name]])).collect();
            links.push(linked);
        }
        Ok(Packages { list, links })
    }

    /// The root package: the last.
    pub fn root(&self) -> &Package {
        self.list.last().expect("a root")
    }

    /// Loads the root in `dir` and the packages its dependencies lead to, each once; a zip is unpacked into the cache of `home`. The dependencies of a package that came from a zip are the root's entries of those names.
    pub fn load(dir: &Path, std: Package, home: Option<&Path>) -> Result<Packages, PackageError> {
        let mut all = vec![Package::load(dir)?];
        let mut zipped = vec![false];
        let mut seen: HashSet<PathBuf> = HashSet::new();
        seen.insert(canonical(dir, &all[0].manifest_file, 0, "")?);
        let cache = home.map(Cache::at);
        let mut at = 0;
        while at < all.len() {
            for wanted in all[at].manifest.dependencies.clone() {
                let (file, base, dep) = if zipped[at] {
                    let Some(entry) = all[0].manifest.dependencies.iter().find(|entry| entry.name == wanted.name) else {
                        let message = format!("`{}` needs `{}`; add it", all[at].manifest.name, wanted.name);
                        return Err(PackageError { file: all[0].manifest_file.clone(), line: 0, key: "dependencies".into(), message });
                    };
                    (all[0].manifest_file.clone(), all[0].dir.clone(), entry.clone())
                } else {
                    (all[at].manifest_file.clone(), all[at].dir.clone(), wanted)
                };
                let key = format!("dependencies.{}", dep.name);
                let there = base.join(&dep.path);
                let real = match dep.source {
                    Source::Path => canonical(&there, &file, dep.line, &key)?,
                    Source::Zip => unpacked(&there, cache.as_ref(), &file, dep.line, &key)?,
                };
                if !seen.insert(real.clone()) {
                    continue;
                }
                let loaded = Package::load(&real)?;
                if loaded.manifest.name != dep.name {
                    return Err(PackageError { file, line: dep.line, key, message: format!("the package there is named `{}`", loaded.manifest.name) });
                }
                all.push(loaded);
                zipped.push(dep.source == Source::Zip);
            }
            at += 1;
        }
        let root = all.remove(0);
        Packages::new(std, all, root)
    }
}

/// The directory the zip at `path` unpacks to in the cache.
fn unpacked(path: &Path, cache: Option<&Cache>, file: &str, line: u32, key: &str) -> Result<PathBuf, PackageError> {
    let fail = |message: String| PackageError { file: file.to_string(), line, key: key.to_string(), message };
    let bytes = fs::read(path).map_err(|_| fail(format!("no zip at {}", shown(path))))?;
    let cache = cache.ok_or_else(|| fail("no mote home: set MOTE_HOME or HOME".to_string()))?;
    let dir = cache.extract(&bytes).map_err(|message| fail(format!("{}: {message}", shown(path))))?;
    canonical(&dir, file, line, key)
}

fn canonical(path: &Path, file: &str, line: u32, key: &str) -> Result<PathBuf, PackageError> {
    fs::canonicalize(path).map_err(|_| PackageError { file: file.to_string(), line, key: key.to_string(), message: format!("no package at {}", shown(path)) })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Open,
    Done,
}

/// Places the package after everything it depends on; a dependency cycle is an error at the dependency that closes it.
fn visit(index: usize, all: &[Package], by_name: &HashMap<String, usize>, state: &mut [Visit], path: &mut Vec<usize>, order: &mut Vec<usize>) -> Result<(), PackageError> {
    state[index] = Visit::Open;
    path.push(index);
    for dep in &all[index].manifest.dependencies {
        let key = format!("dependencies.{}", dep.name);
        let Some(&target) = by_name.get(&dep.name) else {
            return Err(PackageError { file: all[index].manifest_file.clone(), line: dep.line, key, message: "no such package".into() });
        };
        match state[target] {
            Visit::New => visit(target, all, by_name, state, path, order)?,
            Visit::Open => {
                let start = path.iter().position(|&member| member == target).expect("an open package is on the path");
                let mut names: Vec<&str> = path[start..].iter().map(|&member| all[member].manifest.name.as_str()).collect();
                names.push(all[target].manifest.name.as_str());
                return Err(PackageError { file: all[index].manifest_file.clone(), line: dep.line, key, message: format!("dependency cycle {}", names.join(" -> ")) });
            }
            Visit::Done => {}
        }
    }
    path.pop();
    state[index] = Visit::Done;
    order.push(index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{pkg, std_package, Scratch};

    fn names(packages: &Packages) -> Vec<&str> {
        packages.list.iter().map(|package| package.manifest.name.as_str()).collect()
    }

    fn error_of(result: Result<Packages, PackageError>) -> String {
        result.expect_err("an error").to_string()
    }

    #[test]
    fn graph_packages_are_ordered_std_first_each_dependency_before_its_dependents_and_the_root_last() {
        let (app, pane, wire, tile) = (pkg("app", &["pane", "tile"], &[]), pkg("pane", &["wire"], &[]), pkg("wire", &[], &[]), pkg("tile", &["wire"], &[]));
        let packages = Packages::new(pkg("std", &[], &[]), vec![tile, pane, wire], app).unwrap();
        assert_eq!(names(&packages), ["std", "wire", "pane", "tile", "app"]);
        assert_eq!(packages.root().manifest.name, "app");
    }

    #[test]
    fn graph_links_give_each_package_the_index_of_each_dependency_it_names() {
        let packages = Packages::new(pkg("std", &[], &[]), vec![pkg("wire", &[], &[]), pkg("pane", &["wire"], &[])], pkg("app", &["pane"], &[])).unwrap();
        assert_eq!(names(&packages), ["std", "wire", "pane", "app"]);
        assert!(packages.links[0].is_empty());
        assert!(packages.links[1].is_empty());
        assert_eq!(packages.links[2], HashMap::from([("wire".to_string(), 1)]));
        assert_eq!(packages.links[3], HashMap::from([("pane".to_string(), 2)]));
    }

    #[test]
    fn graph_a_package_the_root_does_not_reach_is_left_out() {
        let packages = Packages::new(pkg("std", &[], &[]), vec![pkg("unused", &[], &[])], pkg("app", &[], &[])).unwrap();
        assert_eq!(names(&packages), ["std", "app"]);
    }

    #[test]
    fn graph_two_packages_with_one_name_are_an_error_at_the_second() {
        let other = pkg("app", &[], &[]);
        let result = Packages::new(pkg("std", &[], &[]), vec![other], pkg("app", &[], &[]));
        assert_eq!(error_of(result), "/work/app/mote.toml:2: `package.name`: also the name of the package in /work/app");
    }

    #[test]
    fn graph_the_names_std_and_prelude_are_reserved() {
        let std_named = Packages::new(pkg("std", &[], &[]), Vec::new(), pkg("std", &[], &[]));
        assert_eq!(error_of(std_named), "/work/std/mote.toml:2: `package.name`: `std` is reserved");
        let prelude_dep = Packages::new(pkg("std", &[], &[]), vec![pkg("prelude", &[], &[])], pkg("app", &["prelude"], &[]));
        assert_eq!(error_of(prelude_dep), "/work/prelude/mote.toml:2: `package.name`: `prelude` is reserved");
    }

    #[test]
    fn graph_a_dependency_that_is_not_given_is_an_error_at_its_line() {
        let result = Packages::new(pkg("std", &[], &[]), Vec::new(), pkg("app", &["pane"], &[]));
        assert_eq!(error_of(result), "/work/app/mote.toml:5: `dependencies.pane`: no such package");
    }

    #[test]
    fn graph_a_dependency_cycle_is_an_error_at_the_dependency_that_closes_it() {
        let result = Packages::new(pkg("std", &[], &[]), vec![pkg("a", &["b"], &[]), pkg("b", &["a"], &[])], pkg("app", &["a"], &[]));
        assert_eq!(error_of(result), "/work/b/mote.toml:5: `dependencies.a`: dependency cycle a -> b -> a");
        let itself = Packages::new(pkg("std", &[], &[]), Vec::new(), pkg("app", &["app"], &[]));
        assert_eq!(error_of(itself), "/work/app/mote.toml:5: `dependencies.app`: dependency cycle app -> app");
    }

    #[test]
    fn graph_load_reads_the_manifest_and_every_mote_file_under_src_in_path_order() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", "m"), ("b/z.mote", "z"), ("b/a.mote", "a"), ("a.mote", "a")]);
        scratch.write("app/src/notes.txt", "not a module");
        scratch.write("app/tests/t.mote", "under tests");
        scratch.write("app/tests/sub/u.mote", "deeper");
        scratch.write("app/tests/notes.txt", "not a module");
        let package = Package::load(&dir).unwrap();
        let rels: Vec<&str> = package.files.iter().map(|file| file.rel.as_str()).collect();
        assert_eq!(rels, ["a.mote", "b/a.mote", "b/z.mote", "main.mote"]);
        let tests: Vec<(&str, &str)> = package.tests.iter().map(|file| (file.rel.as_str(), file.text.as_str())).collect();
        assert_eq!(tests, [("sub/u.mote", "deeper"), ("t.mote", "under tests")]);
        assert_eq!(package.tests[1].name, shown(&dir.join("tests/t.mote")));
        assert_eq!(package.files[3].text, "m");
        assert_eq!(package.files[3].name, shown(&dir.join("src/main.mote")));
        assert_eq!(package.manifest.name, "app");
    }

    #[test]
    fn graph_names_in_errors_and_diagnostics_have_no_leading_dot_slash() {
        assert_eq!(shown(Path::new("./src/main.mote")), "src/main.mote");
        assert_eq!(shown(Path::new("./")), "");
        assert_eq!(shown(Path::new("a/./b")), "a/./b");
        assert_eq!(shown(Path::new("../x")), "../x");
        assert_eq!(shown(Path::new("/abs/x")), "/abs/x");
    }

    #[test]
    fn graph_load_a_package_with_no_src_has_no_files() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[]);
        assert_eq!(Package::load(&dir).unwrap().files, Vec::new());
    }

    #[test]
    fn graph_load_a_source_file_that_is_not_utf8_is_an_error_naming_the_file() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("main.mote", "")]);
        std::fs::write(dir.join("src/main.mote"), [0xff, 0xfe, 0x41]).unwrap();
        let found = Package::load(&dir).unwrap_err();
        assert_eq!(found.file, shown(&dir.join("src/main.mote")));
        assert!(found.message.starts_with("cannot read: ") && found.message.contains("UTF-8"), "{found}");
    }

    #[test]
    fn graph_load_a_directory_without_a_manifest_is_an_error_naming_the_file() {
        let scratch = Scratch::new();
        let found = Package::load(&scratch.path("empty")).unwrap_err();
        assert_eq!(found.file, shown(&scratch.path("empty/mote.toml")));
        assert!(found.message.starts_with("cannot read: "), "{found}");
    }

    #[test]
    fn graph_load_a_file_or_directory_that_is_not_a_module_name_is_an_error() {
        let scratch = Scratch::new();
        let dir = scratch.package("app", "app", &[], &[("my-file.mote", "")]);
        let found = Package::load(&dir).unwrap_err().to_string();
        assert_eq!(found, format!("{}: `my-file.mote` is not a module name: letters, digits and `_`, not starting with a digit", shown(&dir.join("src/my-file.mote"))));
        let nested = scratch.package("nested", "nested", &[], &[("a-b/x.mote", "")]);
        assert!(Package::load(&nested).unwrap_err().to_string().contains("`a-b` is not a module name"));
    }

    #[test]
    fn graph_load_follows_path_dependencies_and_loads_each_package_once() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[("pane", "../pane"), ("tile", "../tile")], &[("main.mote", "")]);
        scratch.package("pane", "pane", &[("wire", "../wire")], &[("lib.mote", "")]);
        scratch.package("tile", "tile", &[("wire", "../wire/../wire")], &[("lib.mote", "")]);
        scratch.package("wire", "wire", &[], &[("lib.mote", "")]);
        let packages = Packages::load(&app, std_package(), None).unwrap();
        assert_eq!(names(&packages), ["std", "wire", "pane", "tile", "app"]);
        assert_eq!(packages.root().dir, app);
    }

    #[test]
    fn graph_load_a_dependency_with_another_name_is_an_error_at_its_line() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[("pane", "../other")], &[]);
        scratch.package("other", "other", &[], &[]);
        let found = Packages::load(&app, std_package(), None).unwrap_err();
        assert_eq!(found.to_string(), format!("{}:5: `dependencies.pane`: the package there is named `other`", shown(&app.join("mote.toml"))));
    }

    #[test]
    fn graph_load_a_dependency_directory_that_is_missing_is_an_error_at_its_line() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[("pane", "../gone")], &[]);
        let found = Packages::load(&app, std_package(), None).unwrap_err();
        assert_eq!((found.line, found.key.as_str()), (5, "dependencies.pane"));
        assert!(found.message.starts_with("no package at "), "{found}");
    }

    #[test]
    fn graph_load_a_dependency_cycle_on_disk_is_an_error() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[("a", "../a")], &[]);
        scratch.package("a", "a", &[("b", "../b")], &[]);
        scratch.package("b", "b", &[("a", "../a")], &[]);
        let found = Packages::load(&app, std_package(), None).unwrap_err();
        let real = fs::canonicalize(scratch.path("b")).unwrap();
        assert_eq!(found.to_string(), format!("{}:5: `dependencies.a`: dependency cycle a -> b -> a", shown(&real.join("mote.toml"))));
    }

    /// An app whose manifest lists `entries` (name, source key, path) and holds the zip of each package in `deps/`.
    fn app_with_zips(scratch: &Scratch, entries: &[(&str, &str, &str)], zips: &[(&str, &PathBuf)]) -> PathBuf {
        let mut text = String::from("[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\n");
        for (name, key, path) in entries {
            text.push_str(&format!("{name} = {{ {key} = \"{path}\" }}\n"));
        }
        scratch.write("app/mote.toml", &text);
        scratch.write("app/src/main.mote", "");
        for (file, dir) in zips {
            let packed = crate::archive::pack(dir).unwrap();
            let path = scratch.path("app").join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, packed.bytes).unwrap();
        }
        scratch.path("app")
    }

    #[test]
    fn zip_a_dependency_is_loaded_from_the_cache() {
        let scratch = Scratch::new();
        let pane = scratch.package("pane", "pane", &[], &[("lib.mote", "pub fn go() {}")]);
        let app = app_with_zips(&scratch, &[("pane", "zip", "deps/pane.zip")], &[("deps/pane.zip", &pane)]);
        let home = scratch.path("home");
        let packages = Packages::load(&app, std_package(), Some(&home)).unwrap();
        assert_eq!(names(&packages), ["std", "pane", "app"]);
        assert!(packages.list[1].dir.starts_with(fs::canonicalize(home.join("cache")).unwrap()));
        assert_eq!(packages.list[1].files[0].text, "pub fn go() {}");
        assert_eq!(packages.links[2], HashMap::from([("pane".to_string(), 1)]));
    }

    #[test]
    fn zip_the_dependencies_of_a_zip_package_are_the_roots_entries_of_those_names() {
        let scratch = Scratch::new();
        let wire = scratch.package("wire", "wire", &[], &[("lib.mote", "")]);
        let pane = scratch.package("pane", "pane", &[("wire", "../wire")], &[("lib.mote", "import wire\n")]);
        let app = app_with_zips(&scratch, &[("pane", "zip", "deps/pane.zip"), ("wire", "zip", "deps/wire.zip")], &[("deps/pane.zip", &pane), ("deps/wire.zip", &wire)]);
        let packages = Packages::load(&app, std_package(), Some(&scratch.path("home"))).unwrap();
        assert_eq!(names(&packages), ["std", "wire", "pane", "app"]);
        assert_eq!(packages.links[2], HashMap::from([("wire".to_string(), 1)]));
    }

    #[test]
    fn zip_a_package_that_needs_a_name_the_root_lacks_is_an_error() {
        let scratch = Scratch::new();
        let pane = scratch.package("pane", "pane", &[("wire", "../wire")], &[("lib.mote", "")]);
        let app = app_with_zips(&scratch, &[("pane", "zip", "deps/pane.zip")], &[("deps/pane.zip", &pane)]);
        let found = Packages::load(&app, std_package(), Some(&scratch.path("home"))).unwrap_err();
        assert_eq!(found.to_string(), format!("{}: `dependencies`: `pane` needs `wire`; add it", shown(&app.join("mote.toml"))));
    }

    #[test]
    fn zip_the_same_zip_twice_is_one_package_and_a_changed_zip_is_another_directory() {
        let scratch = Scratch::new();
        let pane = scratch.package("pane", "pane", &[], &[("lib.mote", "a")]);
        let app = app_with_zips(&scratch, &[("pane", "zip", "deps/pane.zip")], &[("deps/pane.zip", &pane)]);
        let home = scratch.path("home");
        let first = Packages::load(&app, std_package(), Some(&home)).unwrap().list[1].dir.clone();
        assert_eq!(Packages::load(&app, std_package(), Some(&home)).unwrap().list[1].dir, first);
        fs::write(pane.join("src/lib.mote"), "b").unwrap();
        app_with_zips(&scratch, &[("pane", "zip", "deps/pane.zip")], &[("deps/pane.zip", &pane)]);
        let second = Packages::load(&app, std_package(), Some(&home)).unwrap().list[1].clone();
        assert_ne!(second.dir, first);
        assert_eq!(second.files[0].text, "b");
    }

    #[test]
    fn zip_a_missing_file_a_missing_home_a_bad_zip_and_another_name_are_errors_at_the_entry() {
        let scratch = Scratch::new();
        let pane = scratch.package("pane", "pane", &[], &[("lib.mote", "")]);
        let app = app_with_zips(&scratch, &[("pane", "zip", "deps/pane.zip"), ("gone", "zip", "deps/gone.zip")], &[("deps/pane.zip", &pane)]);
        let home = scratch.path("home");
        let manifest = shown(&app.join("mote.toml"));
        let no_home = Packages::load(&app, std_package(), None).unwrap_err();
        assert_eq!(no_home.to_string(), format!("{manifest}:5: `dependencies.pane`: no mote home: set MOTE_HOME or HOME"));
        let missing = Packages::load(&app, std_package(), Some(&home)).unwrap_err();
        assert_eq!(missing.to_string(), format!("{manifest}:6: `dependencies.gone`: no zip at {}", shown(&app.join("deps/gone.zip"))));
        fs::write(app.join("deps/gone.zip"), "not a zip").unwrap();
        let bad = Packages::load(&app, std_package(), Some(&home)).unwrap_err();
        assert!(bad.to_string().starts_with(&format!("{manifest}:6: `dependencies.gone`: {}: not a zip file: ", shown(&app.join("deps/gone.zip")))), "{bad}");
        let renamed = app_with_zips(&scratch, &[("other", "zip", "deps/pane.zip")], &[("deps/pane.zip", &pane)]);
        let found = Packages::load(&renamed, std_package(), Some(&home)).unwrap_err();
        assert_eq!(found.to_string(), format!("{manifest}:5: `dependencies.other`: the package there is named `pane`"));
    }
}
