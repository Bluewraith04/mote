//! Finds the file of each shared library a program links, and says why an external call cannot be linked.

use std::collections::HashMap;
use std::path::PathBuf;

use mote_abi::LinkTable;
use mote_base::{Diagnostic, Interner};
use mote_semantics::{DefId, NativeError, NativeReason, Program};

use crate::graph::PackageModules;
use crate::package::Packages;

/// The directory under a package's `lib/` that holds the libraries for this machine.
pub fn host_triple() -> &'static str {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    return "x86_64-linux";
    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    return "x86_64-windows";
    #[cfg(not(all(target_arch = "x86_64", any(target_os = "linux", target_os = "windows"))))]
    compile_error!("Mote runs on x86_64 Linux or Windows");
}

/// The file name of the library a logical name stands for.
pub fn library_file(name: &str) -> String {
    #[cfg(target_os = "windows")]
    return format!("{name}.dll");
    #[cfg(not(target_os = "windows"))]
    return format!("lib{name}.so");
}

/// The file name of the static archive a logical name stands for.
pub fn archive_file(name: &str) -> String {
    #[cfg(target_os = "windows")]
    return format!("{name}.lib");
    #[cfg(not(target_os = "windows"))]
    return format!("lib{name}.a");
}

/// Where a library is found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    /// A file of a package.
    File(PathBuf),
    /// Nothing of a package has it; the system loader looks for `lib<name>.so`.
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub name: String,
    pub location: Location,
}

impl Library {
    /// The archive that lies beside the shared library of a package, when it has one.
    pub fn archive(&self) -> Option<PathBuf> {
        let Location::File(file) = &self.location else { return None };
        let archive = file.with_file_name(archive_file(&self.name));
        archive.is_file().then_some(archive)
    }
}

/// What a program links.
pub struct LinkedLibraries {
    pub table: LinkTable,
    /// One per entry of `table.libraries`, in the same order.
    pub libraries: Vec<Library>,
    /// An extern fn with no C signature, a library name that is not one, and a library that two packages find in different files.
    pub errors: Vec<Diagnostic>,
}

fn name_error(name: &str) -> Option<String> {
    let valid = !name.is_empty() && name.chars().all(|found| found.is_ascii_alphanumeric() || matches!(found, '_' | '-' | '.' | '+'));
    (!valid).then(|| format!("`{name}` is not a library name: letters, digits and `_-.+`"))
}

/// The error text for an extern fn whose signature does not cross to C.
fn failure_text(def: DefId, error: &NativeError, program: &Program, interner: &Interner) -> String {
    let function = program.defs[def].function().expect("an extern declaration is a function");
    let param = |index: Option<usize>| index.map_or(String::new(), |index| interner.resolve(function.params[index].name).to_string());
    let ty = program.render(error.ty, interner);
    match (error.reason, error.param) {
        (NativeReason::MissingLink, _) => "an extern fn needs a library name".into(),
        (NativeReason::Generic, _) => "an extern fn takes no generic parameters".into(),
        (NativeReason::Receiver, _) => "an extern fn is not a method".into(),
        (NativeReason::Variadic, index) => format!("parameter `{}` is variadic", param(index)),
        (NativeReason::VarParameter, index) => format!("`var` parameter `{}` of type `{ty}` has no pointer form", param(index)),
        (NativeReason::VoidParameter, index) => format!("parameter `{}` cannot be `Null`", param(index)),
        (NativeReason::ArrayResult, _) => format!("the result cannot be `{ty}`"),
        (NativeReason::PortResult, _) => "the result cannot be a `PortEnd`".into(),
        (NativeReason::Unsupported, None) => format!("the result of type `{ty}` has no C type"),
        (NativeReason::Unsupported, index) => format!("parameter `{}` of type `{ty}` has no C type", param(index)),
    }
}

/// The location of the library `name` for a declaration in the package at `declaring`: that package's `lib/<triple>/` first, then the others from the root back, then the system.
fn locate(name: &str, declaring: usize, packages: &Packages, triple: &str) -> Location {
    let others = (1..packages.list.len()).rev().filter(|&index| index != declaring);
    for index in std::iter::once(declaring).filter(|&index| index != 0).chain(others) {
        let file = packages.list[index].dir.join("lib").join(triple).join(library_file(name));
        if file.is_file() {
            return Location::File(file);
        }
    }
    Location::System
}

/// The package that holds the module at `index` of the resolver input.
fn package_of(module: usize, ranges: &[PackageModules]) -> usize {
    ranges.iter().position(|range| range.modules.contains(&module)).expect("every module belongs to a package")
}

/// The link table of the program, the file of each library in it, and the errors of the declarations that cannot be linked.
pub fn linked_libraries(program: &Program, interner: &Interner, ranges: &[PackageModules], packages: &Packages, triple: &str) -> LinkedLibraries {
    let (table, failures) = program.link_table(interner);
    let mut errors: Vec<Diagnostic> = failures.iter().map(|failure| Diagnostic::error(failure_text(failure.def, &failure.error, program, interner), program.defs[failure.def].span)).collect();
    let failed: Vec<DefId> = failures.iter().map(|failure| failure.def).collect();
    let mut found: HashMap<String, Location> = HashMap::new();
    for def in program.linked_libraries() {
        if failed.contains(&def) {
            continue;
        }
        let item = &program.defs[def];
        let link = item.function().and_then(|function| function.link).expect("a linked function has a link");
        let name = interner.resolve(link.library).to_string();
        if let Some(message) = name_error(&name) {
            errors.push(Diagnostic::error(message, item.span));
            continue;
        }
        let location = locate(&name, package_of(item.module.index(), ranges), packages, triple);
        match found.get(&name) {
            Some(first) if *first != location => errors.push(Diagnostic::error(format!("library `{name}` is a different file here"), item.span)),
            Some(_) => {}
            None => {
                found.insert(name, location);
            }
        }
    }
    let libraries = table
        .libraries
        .iter()
        .map(|name| Library { name: name.clone(), location: found.get(name).cloned().unwrap_or(Location::System) })
        .collect();
    LinkedLibraries { table, libraries, errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::Packages;
    use crate::standard::standard;
    use crate::testing::{analyzed, Scratch};

    const DEMO: &str = "extern \"demo\" {\n    fn add(a: Int, b: Int) -> Int\n}\n\nfn main() {\n    println(add(1, 2))\n}\n";
    const PANE_LIB: &str = "extern \"pane\" {\n    fn pane_start() -> Int\n}\n\npub fn open() -> Int {\n    return pane_start()\n}\n";
    const USE_PANE: &str = "import pane\n\nfn main() {\n    println(pane.open())\n}\n";

    /// Loads the package at `dir` and takes it through the passes.
    fn link(dir: &std::path::Path) -> (Packages, LinkedLibraries, Vec<String>, Program, Interner) {
        let packages = Packages::load(dir, standard(None, None).unwrap(), None).unwrap();
        let built = analyzed(&packages);
        let found = linked_libraries(&built.program, &built.interner, &built.ranges, &packages, host_triple());
        (packages, found, built.errors, built.program, built.interner)
    }

    fn lib_in(packages: &Packages, package: usize, name: &str) -> PathBuf {
        packages.list[package].dir.join("lib").join(host_triple()).join(library_file(name))
    }

    fn texts(found: &LinkedLibraries) -> Vec<&str> {
        found.errors.iter().map(|diagnostic| diagnostic.message.as_str()).collect()
    }

    #[test]
    fn lib_triple_names_the_architecture_of_this_machine() {
        #[cfg(target_os = "linux")]
        {
            assert_eq!(host_triple(), "x86_64-linux");
            assert_eq!(library_file("sqlite3"), "libsqlite3.so");
        }
        #[cfg(target_os = "windows")]
        {
            assert_eq!(host_triple(), "x86_64-windows");
            assert_eq!(library_file("sqlite3"), "sqlite3.dll");
        }
    }

    #[test]
    fn lib_finds_the_library_in_the_lib_directory_of_the_package_that_declares_it() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[], &[("main.mote", DEMO)]);
        scratch.write(&format!("app/lib/{}/{}", host_triple(), library_file("demo")), "");
        let (packages, found, errors, _, _) = link(&app);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(texts(&found), Vec::<&str>::new());
        assert_eq!(found.libraries, [Library { name: "demo".into(), location: Location::File(lib_in(&packages, 1, "demo")) }]);
        assert_eq!(found.table.libraries, ["demo"]);
    }

    #[test]
    fn lib_an_archive_beside_the_shared_library_is_the_libraries_archive() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[], &[("main.mote", DEMO)]);
        scratch.write(&format!("app/lib/{}/{}", host_triple(), library_file("demo")), "");
        let (packages, found, _, _, _) = link(&app);
        assert_eq!(found.libraries[0].archive(), None);
        let archive = archive_file("demo");
        scratch.write(&format!("app/lib/{}/{archive}", host_triple()), "");
        assert_eq!(found.libraries[0].archive(), Some(lib_in(&packages, 1, "demo").with_file_name(&archive)));
        assert_eq!(Library { name: "c".into(), location: Location::System }.archive(), None);
    }

    #[test]
    fn lib_finds_the_library_of_a_dependency_in_the_dependency() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[("pane", "../pane")], &[("main.mote", USE_PANE)]);
        scratch.package("pane", "pane", &[], &[("lib.mote", PANE_LIB)]);
        scratch.write(&format!("pane/lib/{}/{}", host_triple(), library_file("pane")), "");
        let (packages, found, errors, _, _) = link(&app);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(texts(&found), Vec::<&str>::new());
        assert_eq!(found.libraries[0].location, Location::File(lib_in(&packages, 1, "pane")));
    }

    #[test]
    fn lib_a_library_the_declaring_package_lacks_is_found_in_another_package() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[("pane", "../pane")], &[("main.mote", USE_PANE)]);
        scratch.package("pane", "pane", &[], &[("lib.mote", PANE_LIB)]);
        scratch.write(&format!("app/lib/{}/{}", host_triple(), library_file("pane")), "");
        let (packages, found, _, _, _) = link(&app);
        assert_eq!(found.libraries[0].location, Location::File(lib_in(&packages, 2, "pane")));
    }

    #[test]
    fn lib_a_library_no_package_has_is_left_to_the_system() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[], &[("main.mote", "extern \"m\" {\n    fn pow(x: Float, y: Float) -> Float\n}\n\nfn main() {\n    println(pow(2.0, 3.0))\n}\n")]);
        let (_, found, errors, _, _) = link(&app);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(found.libraries, [Library { name: "m".into(), location: Location::System }]);
        assert_eq!(texts(&found), Vec::<&str>::new());
    }

    #[test]
    fn lib_a_program_with_no_external_calls_links_nothing() {
        let scratch = Scratch::new();
        let app = scratch.package("app", "app", &[], &[("main.mote", "fn main() {\n    println(1)\n}\n")]);
        let (_, found, _, _, _) = link(&app);
        assert!(found.table.functions.is_empty());
        assert_eq!(found.libraries, Vec::new());
        assert_eq!(texts(&found), Vec::<&str>::new());
    }

    #[test]
    fn lib_one_library_found_in_two_files_is_an_error_at_the_second_declaration() {
        let scratch = Scratch::new();
        let own = "extern \"pane\" {\n    fn other() -> Int\n}\n\nfn main() {\n    println(other())\n    println(pane.open())\n}\n";
        let app = scratch.package("app", "app", &[("pane", "../pane")], &[("main.mote", &format!("import pane\n\n{own}"))]);
        scratch.package("pane", "pane", &[], &[("lib.mote", PANE_LIB)]);
        scratch.write(&format!("app/lib/{}/{}", host_triple(), library_file("pane")), "");
        scratch.write(&format!("pane/lib/{}/{}", host_triple(), library_file("pane")), "");
        let (_, found, errors, _, _) = link(&app);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(texts(&found), ["library `pane` is a different file here"]);
    }

    #[test]
    fn lib_a_library_name_with_a_path_is_an_error_at_the_extern_fn() {
        let scratch = Scratch::new();
        let source = "extern \"../evil\" {\n    fn f() -> Int\n}\n\nfn main() {\n    println(f())\n}\n";
        let app = scratch.package("app", "app", &[], &[("main.mote", source)]);
        let (_, found, _, program, _) = link(&app);
        assert_eq!(texts(&found), ["`../evil` is not a library name: letters, digits and `_-.+`"]);
        let span = found.errors[0].span;
        let module = program.modules.iter().map(|(_, module)| module).find(|module| module.source_start <= span.start && ((span.end - module.source_start) as usize) <= module.source.len() && !module.is_std).expect("a module holds the span");
        assert_eq!(&module.source[(span.start - module.source_start) as usize..(span.end - module.source_start) as usize], "f");
    }

    /// The error texts of a program whose only external declaration is `declaration`, called from main as `call`.
    fn failure(declaration: &str, call: &str) -> Vec<String> {
        let scratch = Scratch::new();
        let source = format!("extern \"c\" {{\n    {declaration}\n}}\n\nfn main() {{\n    {call}\n}}\n");
        let app = scratch.package("app", "app", &[], &[("main.mote", &source)]);
        let (_, found, errors, _, _) = link(&app);
        assert_eq!(errors, Vec::<String>::new(), "{source}");
        found.errors.iter().map(|diagnostic| diagnostic.message.clone()).collect()
    }

    #[test]
    fn lib_a_signature_that_does_not_cross_to_c_is_an_error_naming_the_parameter_and_the_type() {
        assert_eq!(failure("fn f(items: [String]) -> Int", "println(f([\"a\"]))"), ["parameter `items` of type `[String]` has no C type"]);
        assert_eq!(failure("fn f(var text: String)", "var s = \"a\"\n    f(s)"), ["`var` parameter `text` of type `String` has no pointer form"]);
        assert_eq!(failure("fn f() -> [Int]", "println(f().len())"), ["the result cannot be `[Int]`"]);
        assert_eq!(failure("fn f() -> (Int, Int)", "println(f())"), ["the result of type `(Int, Int)` has no C type"]);
    }

    #[test]
    fn lib_an_extern_that_is_never_called_is_not_linked_and_not_an_error() {
        assert_eq!(failure("fn f(items: [String]) -> Int", "println(1)"), Vec::<String>::new());
    }
}
