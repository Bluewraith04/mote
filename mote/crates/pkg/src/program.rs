//! Finding the package `mote install` builds: the current one, a directory, or a repository checked out under the mote home.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::split_ref;
use crate::git::{validate_name, Git, GitRef, Repo};
use crate::manifest::PackageManifest;
use crate::native::host_triple;
use crate::store::{build_dir, fetch_files};

/// A package to build, and the checkout to remove afterwards when it was fetched.
pub struct Source {
    pub root: PathBuf,
    pub manifest: PackageManifest,
    scratch: Option<PathBuf>,
}

impl Drop for Source {
    fn drop(&mut self) {
        if let Some(dir) = &self.scratch {
            fs::remove_dir_all(dir).ok();
        }
    }
}

/// The package `target` names: a directory, `<repository>[@ref]`, or with no target the package around `cwd`.
pub fn locate(target: Option<&str>, cwd: &Path) -> Result<Source, String> {
    let from_disk = |dir: &Path| PackageManifest::discover(dir).map(|(root, manifest)| Source { root, manifest, scratch: None });
    match target {
        None => from_disk(cwd),
        Some(t) if Path::new(t).is_dir() => from_disk(Path::new(t)),
        Some(t) => fetch(t),
    }
}

/// Whether the package's entry file declares `fn main`.
pub fn has_main(root: &Path, manifest: &PackageManifest) -> bool {
    fs::read_to_string(root.join(&manifest.package.entry))
        .is_ok_and(|text| text.lines().map(str::trim_start).any(|l| l.starts_with("fn main(") || l.starts_with("pub fn main(")))
}

/// Checks out `<repository>[@ref]` under the mote home's build directory.
fn fetch(spec: &str) -> Result<Source, String> {
    let (repo_spec, at) = split_ref(spec);
    let repo = Repo::parse(repo_spec).map_err(|e| format!("{e}; `mote install` takes a directory or <repository>[@ref]"))?;
    let git = Git;
    let git_ref = match at {
        Some(at) => git.classify(&repo, at)?,
        None => GitRef::Tag(git.latest_tag(&repo)?),
    };
    let commit = git.resolve(&repo, &git_ref)?;
    let entries = fetch_files(&git, &repo, &commit)?;
    let toml = entries.iter().find(|(p, _)| p == "mote.toml").ok_or_else(|| format!("{} has no mote.toml", repo.spec()))?;
    let manifest = PackageManifest::from_toml_str(&String::from_utf8_lossy(&toml.1)).map_err(|e| format!("{}: {e}", repo.spec()))?;
    validate_name(&manifest.package.name)?;

    let root = build_dir()?.join(format!("{}-{}", manifest.package.name, &commit[..12]));
    fs::remove_dir_all(&root).ok();
    let host_native = format!("native/{}/", host_triple());
    for (path, data) in entries.iter().filter(|(p, _)| p == "mote.toml" || p.starts_with("src/") || p.starts_with(&host_native)) {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create '{}': {e}", parent.display()))?;
        }
        fs::write(&target, data).map_err(|e| format!("Failed to write '{}': {e}", target.display()))?;
    }
    Ok(Source { root: root.clone(), manifest, scratch: Some(root) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, entry: &str, source: &str) -> (PathBuf, PackageManifest) {
        let root = std::env::temp_dir().join(format!("mote_program_{name}_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("mote.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"{entry}\"\n")).unwrap();
        fs::write(root.join(entry), source).unwrap();
        let manifest = PackageManifest::from_file(&root.join("mote.toml")).unwrap();
        (root, manifest)
    }

    #[test]
    fn a_program_is_an_entry_with_main() {
        let (root, manifest) = package("prog", "src/main.mote", "import std.sys.env as env\n\nfn main() {\n}\n");
        assert!(has_main(&root, &manifest));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_library_has_no_main() {
        let (root, manifest) = package("lib", "src/lib.mote", "pub fn hello() -> String { return \"x\" }\n// fn main() in a comment\n");
        assert!(!has_main(&root, &manifest));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_directory_is_found_by_its_manifest() {
        let (root, _) = package("found", "src/main.mote", "fn main() {}\n");
        let source = locate(Some(root.to_str().unwrap()), Path::new(".")).unwrap();
        assert_eq!(source.manifest.package.name, "found");
        assert!(source.scratch.is_none());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_target_that_is_neither_a_directory_nor_a_repository_says_what_it_takes() {
        let err = locate(Some("nowhere"), Path::new(".")).err().unwrap();
        assert!(err.contains("a directory or <repository>[@ref]"), "{err}");
    }
}
