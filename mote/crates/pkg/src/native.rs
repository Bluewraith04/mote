//! Native libraries a package carries under `native/<triple>/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::installer::{dir_checksum, tree_checksum};
use crate::manifest::PackageManifest;

/// The target triple of the running `mote`: the name of its directory under `native/`.
pub fn host_triple() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "linux" => format!("{arch}-unknown-linux-{}", if cfg!(target_env = "musl") { "musl" } else { "gnu" }),
        "macos" => format!("{arch}-apple-darwin"),
        "windows" => format!("{arch}-pc-windows-{}", if cfg!(target_env = "gnu") { "gnu" } else { "msvc" }),
        os => format!("{arch}-unknown-{os}"),
    }
}

/// The checksum of each `native/<triple>/` tree among `entries` (package-relative paths and contents), by triple.
pub(crate) fn archive_checksums(entries: &[(String, Vec<u8>)]) -> BTreeMap<String, String> {
    let mut trees: BTreeMap<&str, Vec<(String, &[u8])>> = BTreeMap::new();
    for (path, data) in entries {
        let Some((triple, rest)) = path.strip_prefix("native/").and_then(|p| p.split_once('/')) else { continue };
        trees.entry(triple).or_default().push((rest.to_string(), data.as_slice()));
    }
    trees.into_iter().map(|(triple, files)| (triple.to_string(), tree_checksum(files))).collect()
}

/// The checksum of each `native/<triple>/` directory of the package at `dir`, by triple.
pub(crate) fn dir_checksums(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let native = dir.join("native");
    let mut sums = BTreeMap::new();
    if !native.is_dir() {
        return Ok(sums);
    }
    for entry in fs::read_dir(&native).map_err(|e| format!("Failed to read '{}': {e}", native.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            let triple = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            sums.insert(triple, dir_checksum(&path)?);
        }
    }
    Ok(sums)
}

/// Fails unless a package with native code `sums` is granted it and carries code for this machine.
pub(crate) fn require(name: &str, sums: &BTreeMap<String, String>, granted: bool) -> Result<(), String> {
    if sums.is_empty() {
        return Ok(());
    }
    if !granted {
        return Err(format!("{name} carries native code; list it in mote.toml with native = true"));
    }
    let host = host_triple();
    if !sums.contains_key(&host) {
        return Err(format!("{name} has no native code for {host}"));
    }
    Ok(())
}

/// The packages `mote.toml` in `root` grants native access, each with where its libraries are installed.
pub fn grants(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(manifest) = PackageManifest::from_file(&root.join("mote.toml")) else { return Vec::new() };
    let host = host_triple();
    manifest
        .dependencies
        .iter()
        .filter(|(_, spec)| spec.native())
        .map(|(name, _)| (name.clone(), root.join(".mote_packages").join(name).join("native").join(&host)))
        .collect()
}

/// The directory beside a built program that holds its native libraries, `<name>.lib`.
pub fn lib_dir(exe: &Path) -> PathBuf {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    exe.with_file_name(format!("{stem}.lib"))
}

/// The packages a built program carries libraries for: the subdirectories of its `.lib` directory.
pub fn built_grants(exe: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(lib_dir(exe)) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect()
}

/// Copies the host's libraries of every granted package into `<exe>.lib/<package>/`; answers how many packages.
pub fn copy_libraries(root: &Path, exe: &Path) -> Result<usize, String> {
    let dir = lib_dir(exe);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("Failed to clear '{}': {e}", dir.display()))?;
    }
    let mut copied = 0;
    for (name, from) in grants(root) {
        if !from.is_dir() {
            continue;
        }
        let to = dir.join(&name);
        fs::create_dir_all(&to).map_err(|e| format!("Failed to create '{}': {e}", to.display()))?;
        for entry in fs::read_dir(&from).map_err(|e| format!("Failed to read '{}': {e}", from.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_file() {
                let target = to.join(path.file_name().unwrap_or_default());
                fs::copy(&path, &target).map_err(|e| format!("Failed to copy '{}': {e}", path.display()))?;
            }
        }
        copied += 1;
    }
    Ok(copied)
}

/// How many library files [`install_libraries`] wrote and how many it found already in place.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Installed {
    pub copied: usize,
    pub kept: usize,
}

/// Puts the host's libraries of every granted package in `<exe>.lib/<package>/`: a file already there with the same bytes stays, a changed one is replaced, and what the program no longer carries is removed.
pub fn install_libraries(root: &Path, exe: &Path) -> Result<Installed, String> {
    let dir = lib_dir(exe);
    let grants: Vec<(String, PathBuf)> = grants(root).into_iter().filter(|(_, from)| from.is_dir()).collect();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !grants.iter().any(|(n, _)| *n == name) {
                let path = entry.path();
                let gone = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
                gone.map_err(|e| format!("Failed to remove '{}': {e}", path.display()))?;
            }
        }
    }
    let mut done = Installed::default();
    for (name, from) in &grants {
        let to = dir.join(name);
        fs::create_dir_all(&to).map_err(|e| format!("Failed to create '{}': {e}", to.display()))?;
        let mut wanted = Vec::new();
        for entry in fs::read_dir(from).map_err(|e| format!("Failed to read '{}': {e}", from.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if !path.is_file() {
                continue;
            }
            let file = path.file_name().unwrap_or_default().to_os_string();
            let target = to.join(&file);
            let bytes = fs::read(&path).map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
            if fs::read(&target).is_ok_and(|have| have == bytes) {
                done.kept += 1;
            } else {
                // Written beside the target and renamed over it, so a running program that has the old library mapped keeps it.
                let tmp = to.join(format!("{}.mote-tmp", file.to_string_lossy()));
                fs::write(&tmp, &bytes).map_err(|e| format!("Failed to write '{}': {e}", tmp.display()))?;
                fs::rename(&tmp, &target).map_err(|e| format!("Failed to write '{}': {e}", target.display()))?;
                done.copied += 1;
            }
            wanted.push(file);
        }
        for entry in fs::read_dir(&to).map_err(|e| e.to_string())?.filter_map(Result::ok) {
            if !wanted.contains(&entry.file_name()) && entry.path().is_file() {
                fs::remove_file(entry.path()).map_err(|e| format!("Failed to remove '{}': {e}", entry.path().display()))?;
            }
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_with_libraries(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("mote_install_libs_{name}_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let from = root.join(".mote_packages/dep/native").join(host_triple());
        fs::create_dir_all(&from).unwrap();
        fs::write(root.join("mote.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ndep = { path = \"../dep\", native = true }\n").unwrap();
        for (file, text) in files {
            fs::write(from.join(file), text).unwrap();
        }
        root
    }

    #[test]
    fn a_library_already_in_place_is_kept_and_a_changed_one_is_replaced() {
        let root = package_with_libraries("keep", &[("libx.so", "one"), ("liby.so", "two")]);
        let exe = root.join("out").join("app");
        assert_eq!(install_libraries(&root, &exe).unwrap(), Installed { copied: 2, kept: 0 });
        assert_eq!(install_libraries(&root, &exe).unwrap(), Installed { copied: 0, kept: 2 });
        let from = root.join(".mote_packages/dep/native").join(host_triple());
        fs::write(from.join("libx.so"), "changed").unwrap();
        assert_eq!(install_libraries(&root, &exe).unwrap(), Installed { copied: 1, kept: 1 });
        assert_eq!(fs::read_to_string(lib_dir(&exe).join("dep/libx.so")).unwrap(), "changed");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn what_the_program_no_longer_carries_is_removed() {
        let root = package_with_libraries("stale", &[("libx.so", "one"), ("liby.so", "two")]);
        let exe = root.join("out").join("app");
        install_libraries(&root, &exe).unwrap();
        fs::create_dir_all(lib_dir(&exe).join("old")).unwrap();
        fs::write(lib_dir(&exe).join("old/liba.so"), "old").unwrap();
        fs::remove_file(root.join(".mote_packages/dep/native").join(host_triple()).join("liby.so")).unwrap();
        install_libraries(&root, &exe).unwrap();
        assert!(!lib_dir(&exe).join("dep/liby.so").exists());
        assert!(!lib_dir(&exe).join("old").exists());
        assert!(lib_dir(&exe).join("dep/libx.so").is_file());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn checksums_are_per_triple() {
        let entries = vec![
            ("native/a-b-c/lib.so".to_string(), b"one".to_vec()),
            ("native/d-e-f/lib.so".to_string(), b"two".to_vec()),
            ("src/lib.mote".to_string(), b"x".to_vec()),
        ];
        let sums = archive_checksums(&entries);
        assert_eq!(sums.keys().collect::<Vec<_>>(), ["a-b-c", "d-e-f"]);
        assert_ne!(sums["a-b-c"], sums["d-e-f"]);
    }

    #[test]
    fn a_library_directory_is_named_after_the_program() {
        assert_eq!(lib_dir(Path::new("dist/hello")), Path::new("dist/hello.lib"));
        assert_eq!(lib_dir(Path::new("dist/hello.exe")), Path::new("dist/hello.lib"));
    }
}
