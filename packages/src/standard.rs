//! The std package: the copy embedded at build time, or the one in a directory.

use std::path::Path;

use crate::manifest::{Manifest, PackageError};
use crate::package::{Package, SourceFile};

pub(crate) mod embedded {
    include!(concat!(env!("OUT_DIR"), "/std_files.rs"));
}

/// What an error calls the embedded std.
const EMBEDDED_DIR: &str = "std";

/// The std this build carries.
pub fn embedded() -> Result<Package, PackageError> {
    let manifest = Manifest::parse(&format!("{EMBEDDED_DIR}/mote.toml"), embedded::STD_MANIFEST)?;
    let files = embedded::STD_FILES.iter().map(|(rel, text)| SourceFile { rel: rel.to_string(), name: format!("{EMBEDDED_DIR}/src/{rel}"), text: text.to_string() }).collect();
    Ok(Package::new(manifest, EMBEDDED_DIR, files))
}

/// The std in `dir`, or the embedded one when there is no directory.
pub fn standard(dir: Option<&Path>) -> Result<Package, PackageError> {
    let Some(dir) = dir else { return embedded() };
    let package = Package::load(dir)?;
    if package.manifest.name != "std" {
        return Err(PackageError { file: package.manifest_file, line: package.manifest.name_line, key: "package.name".into(), message: format!("the std is named `std`, not `{}`", package.manifest.name) });
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build;
    use crate::package::Packages;
    use crate::testing::{pkg, std_package, Scratch};

    #[test]
    fn std_embed_carries_the_manifest_and_every_file_of_the_std_directory() {
        let on_disk = std_package();
        let embedded = embedded().unwrap();
        assert_eq!(embedded.manifest, on_disk.manifest);
        let rels: Vec<(&str, &str)> = embedded.files.iter().map(|file| (file.rel.as_str(), file.text.as_str())).collect();
        let disk: Vec<(&str, &str)> = on_disk.files.iter().map(|file| (file.rel.as_str(), file.text.as_str())).collect();
        assert!(rels.len() > 20, "{} files", rels.len());
        assert_eq!(rels, disk);
    }

    #[test]
    fn std_embed_names_its_files_without_a_path_of_the_build_machine() {
        let embedded = embedded().unwrap();
        assert!(embedded.files.iter().all(|file| file.name == format!("std/src/{}", file.rel)));
        assert_eq!(embedded.manifest_file, "std/mote.toml");
    }

    #[test]
    fn std_embed_marks_every_std_module_and_no_other() {
        let root = pkg("app", &[], &[("main.mote", "import std.string\n")]);
        let graph = build(&Packages::new(standard(None).unwrap(), Vec::new(), root).unwrap(), false);
        assert!(graph.sink.is_empty());
        let std_modules = graph.packages[0].modules.clone();
        for (index, module) in graph.input.modules.iter().enumerate() {
            assert_eq!(module.is_std, std_modules.contains(&index), "module {index}");
        }
        assert!(graph.input.modules.len() > std_modules.len());
    }

    #[test]
    fn std_embed_a_directory_replaces_the_embedded_std() {
        let scratch = Scratch::new();
        let dir = scratch.package("small", "std", &[], &[("lib.mote", ""), ("prelude.mote", ""), ("tiny.mote", "pub fn one() {}")]);
        let small = standard(Some(&dir)).unwrap();
        assert_eq!(small.files.iter().map(|file| file.rel.as_str()).collect::<Vec<_>>(), ["lib.mote", "prelude.mote", "tiny.mote"]);
        let root = pkg("app", &[], &[("main.mote", "import std.tiny\nimport std.string\n")]);
        let graph = build(&Packages::new(small, Vec::new(), root).unwrap(), false);
        let errors: Vec<&str> = graph.sink.iter().map(|diagnostic| diagnostic.message.as_str()).collect();
        assert_eq!(errors, ["no module `std.string`"]);
    }

    #[test]
    fn std_embed_a_directory_that_is_not_the_std_is_an_error() {
        let scratch = Scratch::new();
        let dir = scratch.package("other", "other", &[], &[]);
        let found = standard(Some(&dir)).unwrap_err();
        assert_eq!(found.to_string(), format!("{}:2: `package.name`: the std is named `std`, not `other`", dir.join("mote.toml").display()));
    }

    #[test]
    fn std_embed_a_directory_with_no_manifest_is_an_error() {
        let scratch = Scratch::new();
        let found = standard(Some(&scratch.path("none"))).unwrap_err();
        assert!(found.to_string().contains("none/mote.toml: cannot read"), "{found}");
    }
}
