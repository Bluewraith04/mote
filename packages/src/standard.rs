//! The std package: loaded from `mote_std/std` on disk (via `--std`, `MOTE_STD`, `<home>/mote_std`, workspace, or `~/.mote/mote_std`).

use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, PackageError};
use crate::package::Package;

fn has_mote_std(dir: &Path) -> bool {
    dir.join("std/mote.toml").is_file()
}

/// Locates `mote_std` on disk:
/// 1. `MOTE_STD` if set
/// 2. Explicit `<home>/mote_std` when it contains `std/mote.toml`
/// 3. Ancestors of the running executable (workspace builds)
/// 4. `<default_home>/mote_std` (`$MOTE_HOME/mote_std` or `~/.mote/mote_std`)
/// 5. Ancestors of the current working directory
pub fn default_mote_std(home: Option<&Path>) -> Option<PathBuf> {
    if let Some(env_dir) = std::env::var_os("MOTE_STD").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(env_dir));
    }
    if let Some(home) = home {
        let candidate = home.join("mote_std");
        if has_mote_std(&candidate) {
            return Some(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1) {
            let candidate = ancestor.join("mote_std");
            if has_mote_std(&candidate) {
                return Some(candidate);
            }
        }
    }
    if let Some(default) = crate::cache::default_home() {
        let candidate = default.join("mote_std");
        if has_mote_std(&candidate) {
            return Some(candidate);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            let candidate = ancestor.join("mote_std");
            if has_mote_std(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Loads the std from `dir`, or from `mote_std/std`.
pub fn standard(dir: Option<&Path>, home: Option<&Path>) -> Result<Package, PackageError> {
    let resolved;
    let std_dir = match dir {
        Some(dir) => dir,
        None => {
            let root = default_mote_std(home).ok_or_else(|| PackageError {
                file: "mote_std/std/mote.toml".into(),
                line: 0,
                key: String::new(),
                message: "cannot find `mote_std` (run install.sh, set MOTE_STD, or pass --std)".into(),
            })?;
            resolved = root.join("std");
            &resolved
        }
    };
    let package = Package::load(std_dir)?;
    if package.manifest.name != "std" {
        return Err(PackageError { file: package.manifest_file, line: package.manifest.name_line, key: "package.name".into(), message: format!("the std is named `std`, not `{}`", package.manifest.name) });
    }
    let _ = Manifest::read(&std_dir.join("mote.toml"))?;
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build;
    use crate::package::Packages;
    use crate::testing::{pkg, std_package, Scratch};

    #[test]
    fn std_loads_the_manifest_and_every_file_of_mote_std_std() {
        let on_disk = std_package();
        let loaded = standard(None, None).unwrap();
        assert_eq!(loaded.manifest, on_disk.manifest);
        let rels: Vec<(&str, &str)> = loaded.files.iter().map(|file| (file.rel.as_str(), file.text.as_str())).collect();
        let disk: Vec<(&str, &str)> = on_disk.files.iter().map(|file| (file.rel.as_str(), file.text.as_str())).collect();
        assert!(rels.len() > 20, "{} files", rels.len());
        assert_eq!(rels, disk);
    }

    #[test]
    fn std_marks_every_std_module_and_no_other() {
        let root = pkg("app", &[], &[("main.mote", "import std.string\n")]);
        let graph = build(&Packages::new(standard(None, None).unwrap(), Vec::new(), root).unwrap(), false);
        assert!(graph.sink.is_empty());
        let std_modules = graph.packages[0].modules.clone();
        for (index, module) in graph.input.modules.iter().enumerate() {
            assert_eq!(module.is_std, std_modules.contains(&index), "module {index}");
        }
        assert!(graph.input.modules.len() > std_modules.len());
    }

    #[test]
    fn std_a_directory_replaces_the_default_std() {
        let scratch = Scratch::new();
        let dir = scratch.package("small", "std", &[], &[("lib.mote", ""), ("prelude.mote", ""), ("tiny.mote", "pub fn one() {}")]);
        let small = standard(Some(&dir), None).unwrap();
        assert_eq!(small.files.iter().map(|file| file.rel.as_str()).collect::<Vec<_>>(), ["lib.mote", "prelude.mote", "tiny.mote"]);
        let root = pkg("app", &[], &[("main.mote", "import std.tiny\nimport std.string\n")]);
        let graph = build(&Packages::new(small, Vec::new(), root).unwrap(), false);
        let errors: Vec<&str> = graph.sink.iter().map(|diagnostic| diagnostic.message.as_str()).collect();
        assert_eq!(errors, ["no module `std.string`"]);
    }

    #[test]
    fn std_a_custom_home_with_mote_std_overrides_workspace_default() {
        let scratch = Scratch::new();
        let home = scratch.path("home");
        scratch.package("home/mote_std/std", "std", &[], &[("lib.mote", ""), ("prelude.mote", ""), ("custom.mote", "pub fn ok() {}")]);
        let custom = standard(None, Some(&home)).unwrap();
        assert_eq!(custom.files.iter().map(|file| file.rel.as_str()).collect::<Vec<_>>(), ["custom.mote", "lib.mote", "prelude.mote"]);
    }

    #[test]
    fn std_a_directory_that_is_not_the_std_is_an_error() {
        let scratch = Scratch::new();
        let dir = scratch.package("other", "other", &[], &[]);
        let found = standard(Some(&dir), None).unwrap_err();
        assert_eq!(found.to_string(), format!("{}:2: `package.name`: the std is named `std`, not `other`", crate::package::shown(&dir.join("mote.toml"))));
    }

    #[test]
    fn std_a_directory_with_no_manifest_is_an_error() {
        let scratch = Scratch::new();
        let found = standard(Some(&scratch.path("none")), None).unwrap_err();
        assert!(found.to_string().contains("none/mote.toml: cannot read"), "{found}");
    }
}
