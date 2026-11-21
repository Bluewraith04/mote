//! The core libraries: the copy embedded at build time, or the packages in a directory; a package imports one by its name with no manifest entry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mote_base::{DiagnosticSink, Interner, SourceMap};
use mote_syntax::{lex, parse, Import, ItemKind};

use crate::manifest::{Manifest, PackageError};
use crate::package::{Package, Packages, SourceFile};
use crate::standard::embedded;

/// The core libraries this build carries, with their natives expected in `<home>/core/<name>/native/`.
pub fn embedded_core(home: Option<&Path>) -> Result<Vec<Package>, PackageError> {
    let base = home.map_or_else(|| PathBuf::from("core"), |home| home.join("core"));
    embedded::CORE
        .iter()
        .map(|(name, manifest, files)| {
            let manifest = Manifest::parse(&format!("core/{name}/mote.toml"), manifest)?;
            let files = files.iter().map(|(rel, text)| SourceFile { rel: rel.to_string(), name: format!("core/{name}/src/{rel}"), text: text.to_string() }).collect();
            Ok(Package::new(manifest, base.join(name), files))
        })
        .collect()
}

/// Every package in the folders of `dir`; a folder is named as its package.
pub fn packages_in(dir: &Path) -> Result<Vec<Package>, PackageError> {
    let entries = std::fs::read_dir(dir).map_err(|error| PackageError { file: crate::package::shown(dir), line: 0, key: String::new(), message: format!("cannot read: {error}") })?;
    let mut folders: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| path.join("mote.toml").is_file()).collect();
    folders.sort();
    let mut found = Vec::new();
    for folder in folders {
        let package = Package::load(&folder)?;
        let folder_name = folder.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        if package.manifest.name != folder_name {
            return Err(PackageError { file: package.manifest_file, line: package.manifest.name_line, key: "package.name".into(), message: format!("the folder is named `{folder_name}`, not `{}`", package.manifest.name) });
        }
        found.push(package);
    }
    Ok(found)
}

/// The core libraries in `dir`, or the embedded ones when there is no directory.
pub fn core_packages(dir: Option<&Path>, home: Option<&Path>) -> Result<Vec<Package>, PackageError> {
    match dir {
        Some(dir) => packages_in(dir),
        None => embedded_core(home),
    }
}

/// The names of the embedded core libraries.
pub fn core_names() -> Vec<&'static str> {
    embedded::CORE.iter().map(|(name, _, _)| *name).collect()
}

/// The first name of each import that is not relative, in the files of `src/` and, when asked, of `tests/`.
fn imported(package: &Package, with_tests: bool) -> Vec<String> {
    let (mut map, mut interner, mut sink) = (SourceMap::default(), Interner::default(), DiagnosticSink::default());
    let tests = if with_tests { &package.tests[..] } else { &[] };
    let mut names = Vec::new();
    for file in package.files.iter().chain(tests) {
        let id = map.add_file(file.name.clone(), file.text.clone());
        let lexed = lex(map.file(id), &mut interner, &mut sink);
        let ast = parse(map.file(id), &lexed, &mut interner, &mut sink);
        for &item in &ast.root {
            let path = match &ast[item].kind {
                ItemKind::Import(Import::Module { path, .. }) => path,
                ItemKind::Import(Import::Names { from, .. } | Import::Glob { from }) => from,
                _ => continue,
            };
            if let (true, Some(first)) = (path.prefix.is_empty(), path.segments.first()) {
                let name = interner.resolve(first.name).to_string();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

impl Packages {
    /// Adds the core libraries the packages import by name, each once, after the std and before the packages that use it; a package named as a core library is an error, unless it is the root in the directory of that library.
    pub fn with_core(self, core: Vec<Package>, with_tests: bool) -> Result<Packages, PackageError> {
        let here = std::fs::canonicalize(&self.root().dir).ok();
        let core: Vec<Package> = core.into_iter().filter(|package| here.is_none() || std::fs::canonicalize(&package.dir).ok() != here).collect();
        if core.is_empty() {
            return Ok(self);
        }
        let names: Vec<String> = core.iter().map(|package| package.manifest.name.clone()).collect();
        let by_name: HashMap<&str, usize> = names.iter().enumerate().map(|(index, name)| (name.as_str(), index)).collect();
        for package in &self.list[1..] {
            let name = &package.manifest.name;
            if by_name.contains_key(name.as_str()) {
                return Err(PackageError { file: package.manifest_file.clone(), line: package.manifest.name_line, key: "package.name".into(), message: format!("`{name}` is reserved") });
            }
        }
        let root = self.list.len() - 1;
        let wanted = |package: &Package, tests: bool| -> Vec<usize> { imported(package, tests).iter().filter(|name| **name != package.manifest.name).filter_map(|name| by_name.get(name.as_str()).copied()).collect() };
        let from_users: Vec<Vec<usize>> = self.list.iter().enumerate().skip(1).map(|(index, package)| wanted(package, with_tests && index == root)).collect();
        let mut needs: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut pending: Vec<usize> = from_users.iter().flatten().copied().collect();
        while let Some(index) = pending.pop() {
            if needs.contains_key(&index) {
                continue;
            }
            let uses = wanted(&core[index], false);
            pending.extend(uses.iter().copied());
            needs.insert(index, uses);
        }
        let mut order = Vec::new();
        let mut state: HashMap<usize, bool> = HashMap::new();
        let mut roots: Vec<usize> = needs.keys().copied().collect();
        roots.sort();
        for index in roots {
            place(index, &core, &needs, &mut state, &mut Vec::new(), &mut order)?;
        }
        let shift = order.len();
        let at = |index: usize| 1 + order.iter().position(|&placed| placed == index).expect("a needed core library is placed");
        let mut slots: Vec<Option<Package>> = core.into_iter().map(Some).collect();
        let mut links: Vec<HashMap<String, usize>> = vec![HashMap::new()];
        let mut list = vec![self.list[0].clone()];
        for &index in &order {
            let package = slots[index].take().expect("a core library is placed once");
            links.push(needs[&index].iter().map(|&used| (names[used].clone(), at(used))).collect());
            list.push(package);
        }
        for (old, package) in self.list.iter().enumerate().skip(1) {
            let mut linked: HashMap<String, usize> = self.links[old].iter().map(|(name, &position)| (name.clone(), position + shift)).collect();
            for &used in &from_users[old - 1] {
                linked.insert(names[used].clone(), at(used));
            }
            links.push(linked);
            list.push(package.clone());
        }
        Ok(Packages { list, links })
    }
}

/// Places a core library after the ones it imports; a cycle is an error at the manifest of the library that closes it.
fn place(index: usize, core: &[Package], needs: &HashMap<usize, Vec<usize>>, state: &mut HashMap<usize, bool>, path: &mut Vec<usize>, order: &mut Vec<usize>) -> Result<(), PackageError> {
    match state.get(&index) {
        Some(true) => return Ok(()),
        Some(false) => {
            let start = path.iter().position(|&member| member == index).expect("an open library is on the path");
            let mut names: Vec<&str> = path[start..].iter().map(|&member| core[member].manifest.name.as_str()).collect();
            names.push(core[index].manifest.name.as_str());
            let package = &core[*path.last().expect("a cycle has a path")];
            return Err(PackageError { file: package.manifest_file.clone(), line: 0, key: "dependencies".into(), message: format!("dependency cycle {}", names.join(" -> ")) });
        }
        None => {}
    }
    state.insert(index, false);
    path.push(index);
    for &used in &needs[&index] {
        place(used, core, needs, state, path, order)?;
    }
    path.pop();
    state.insert(index, true);
    order.push(index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build;
    use crate::testing::{pkg, std_package, Scratch};

    fn names(packages: &Packages) -> Vec<&str> {
        packages.list.iter().map(|package| package.manifest.name.as_str()).collect()
    }

    fn lib(name: &str, text: &str) -> Package {
        pkg(name, &[], &[("lib.mote", text)])
    }

    fn app(text: &str) -> Packages {
        Packages::new(std_package(), Vec::new(), pkg("app", &[], &[("main.mote", text)])).unwrap()
    }

    #[test]
    fn core_embed_an_import_of_a_core_name_adds_the_library_with_no_manifest_entry() {
        let packages = app("import { f } from path\n").with_core(vec![lib("path", "pub fn f() {}\n"), lib("date", "")], false).unwrap();
        assert_eq!(names(&packages), ["std", "path", "app"]);
        assert_eq!(packages.links[2]["path"], 1);
        let graph = build(&packages, false);
        assert!(graph.sink.is_empty(), "{:?}", graph.sink.iter().map(|found| found.message.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn core_embed_a_library_nothing_imports_is_left_out() {
        let packages = app("fn main() {}\n").with_core(vec![lib("path", ""), lib("date", "")], false).unwrap();
        assert_eq!(names(&packages), ["std", "app"]);
    }

    #[test]
    fn core_embed_a_library_that_imports_another_brings_it_first() {
        let core = vec![lib("fs", "import path\n"), lib("path", ""), lib("log", "")];
        let packages = app("import fs\nimport std.string\n").with_core(core, false).unwrap();
        assert_eq!(names(&packages), ["std", "path", "fs", "app"]);
        assert_eq!(packages.links[2]["path"], 1);
        assert_eq!(packages.links[3]["fs"], 2);
        assert!(!packages.links[3].contains_key("path"));
    }

    #[test]
    fn core_embed_a_dependency_package_imports_a_core_library_too() {
        let root = pkg("app", &["pane"], &[("main.mote", "import pane\n")]);
        let pane = pkg("pane", &[], &[("lib.mote", "import path\n")]);
        let packages = Packages::new(std_package(), vec![pane], root).unwrap().with_core(vec![lib("path", "")], false).unwrap();
        assert_eq!(names(&packages), ["std", "path", "pane", "app"]);
        assert_eq!(packages.links[2]["path"], 1);
        assert_eq!(packages.links[3]["pane"], 2);
        assert!(!packages.links[3].contains_key("path"));
    }

    #[test]
    fn core_embed_the_tests_of_the_root_import_a_library_only_when_they_are_built() {
        let root = pkg("app", &[], &[("main.mote", "")]).with_tests(vec![SourceFile { rel: "t.mote".into(), name: "app/tests/t.mote".into(), text: "import path\n".into() }]);
        let packages = Packages::new(std_package(), Vec::new(), root).unwrap();
        assert_eq!(names(&packages.clone().with_core(vec![lib("path", "")], false).unwrap()), ["std", "app"]);
        assert_eq!(names(&packages.with_core(vec![lib("path", "")], true).unwrap()), ["std", "path", "app"]);
    }

    #[test]
    fn core_embed_a_package_named_as_a_core_library_is_reserved() {
        let found = app("").with_core(vec![lib("app", "")], false).unwrap_err();
        assert_eq!(found.to_string(), "/work/app/mote.toml:2: `package.name`: `app` is reserved");
    }

    #[test]
    fn core_embed_the_root_in_the_directory_of_a_library_stands_in_for_it() {
        let scratch = Scratch::new();
        scratch.package("core/path", "path", &[], &[("lib.mote", "pub fn f() {}\n")]);
        scratch.package("core/fs", "fs", &[], &[("lib.mote", "import path\n")]);
        let core = core_packages(Some(&scratch.path("core")), None).unwrap();
        let root = Package::load(&scratch.path("core/fs")).unwrap();
        let packages = Packages::new(std_package(), Vec::new(), root).unwrap().with_core(core, true).unwrap();
        assert_eq!(names(&packages), ["std", "path", "fs"]);
        let elsewhere = scratch.package("elsewhere/fs", "fs", &[], &[("lib.mote", "")]);
        let core = core_packages(Some(&scratch.path("core")), None).unwrap();
        let found = Packages::new(std_package(), Vec::new(), Package::load(&elsewhere).unwrap()).unwrap().with_core(core, true).unwrap_err();
        assert!(found.to_string().ends_with("`fs` is reserved"), "{found}");
    }

    #[test]
    fn core_embed_a_library_does_not_import_itself() {
        let packages = app("import path\n").with_core(vec![lib("path", "import path\n")], false).unwrap();
        assert_eq!(names(&packages), ["std", "path", "app"]);
    }

    #[test]
    fn core_embed_a_cycle_between_libraries_is_an_error() {
        let found = app("import a\n").with_core(vec![lib("a", "import b\n"), lib("b", "import a\n")], false).unwrap_err();
        assert!(found.to_string().ends_with("dependency cycle a -> b -> a"), "{found}");
    }

    #[test]
    fn core_embed_an_unknown_name_is_still_an_unknown_package() {
        let packages = app("import nothing\n").with_core(vec![lib("path", "")], false).unwrap();
        let graph = build(&packages, false);
        assert_eq!(graph.sink.iter().map(|found| found.message.as_str()).collect::<Vec<_>>(), ["no package `nothing`"]);
    }

    #[test]
    fn core_embed_a_directory_holds_one_folder_for_each_library() {
        let scratch = Scratch::new();
        scratch.package("core/path", "path", &[], &[("lib.mote", "pub fn f() {}\n")]);
        scratch.package("core/date", "date", &[], &[("lib.mote", "")]);
        scratch.write("core/notes.txt", "not a library");
        let found = core_packages(Some(&scratch.path("core")), None).unwrap();
        assert_eq!(found.iter().map(|package| package.manifest.name.as_str()).collect::<Vec<_>>(), ["date", "path"]);
        assert_eq!(found[1].dir, scratch.path("core/path"));
    }

    #[test]
    fn core_embed_a_folder_named_otherwise_than_its_package_is_an_error() {
        let scratch = Scratch::new();
        scratch.package("core/path", "paths", &[], &[]);
        let found = core_packages(Some(&scratch.path("core")), None).unwrap_err();
        assert!(found.to_string().ends_with("the folder is named `path`, not `paths`"), "{found}");
    }

    #[test]
    fn core_embed_a_missing_directory_is_an_error() {
        let found = core_packages(Some(Path::new("/nonexistent/core")), None).unwrap_err();
        assert!(found.to_string().starts_with("/nonexistent/core: cannot read"), "{found}");
    }

    #[test]
    fn core_embed_carries_every_package_of_the_core_folder() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core");
        let on_disk = core_in_folder(&folder);
        let embedded = embedded_core(None).unwrap();
        assert_eq!(embedded.iter().map(|package| package.manifest.name.clone()).collect::<Vec<_>>(), on_disk.iter().map(|package| package.manifest.name.clone()).collect::<Vec<_>>());
        for (found, wanted) in embedded.iter().zip(&on_disk) {
            assert_eq!(found.manifest, wanted.manifest);
            assert_eq!(found.files.iter().map(|file| (&file.rel, &file.text)).collect::<Vec<_>>(), wanted.files.iter().map(|file| (&file.rel, &file.text)).collect::<Vec<_>>());
        }
        assert_eq!(core_names(), embedded.iter().map(|package| package.manifest.name.as_str()).collect::<Vec<_>>());
    }

    fn core_in_folder(folder: &Path) -> Vec<Package> {
        packages_in(folder).unwrap()
    }

    #[test]
    fn core_embed_the_natives_of_an_embedded_library_are_looked_up_under_the_home() {
        let home = Path::new("/srv/someone/.mote");
        for package in embedded_core(Some(home)).unwrap() {
            assert_eq!(package.dir, home.join("core").join(&package.manifest.name));
        }
        for package in embedded_core(None).unwrap() {
            assert_eq!(package.dir, Path::new("core").join(&package.manifest.name));
        }
    }
}
