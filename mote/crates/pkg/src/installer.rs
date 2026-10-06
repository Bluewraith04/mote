use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};
use crate::manifest::PackageManifest;
use crate::native::{archive_checksums, dir_checksums, host_triple};
use crate::resolver::ResolvedPackage;

/// Copies dependencies into `.mote_packages`.
pub struct PackageInstaller {
    project_root: PathBuf,
    packages_dir: PathBuf,
}

impl PackageInstaller {
    pub fn new(project_root: &Path) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            packages_dir: project_root.join(".mote_packages"),
        }
    }

    /// The directory of a `local+<path>` source, relative to the project root.
    pub(crate) fn local_source_dir(&self, source: &str) -> Option<PathBuf> {
        source.strip_prefix("local+").map(|p| self.project_root.join(p))
    }

    /// Installs resolved packages into the project's `.mote_packages/` directory.
    pub fn install(&self, packages: &[ResolvedPackage]) -> Result<(), String> {
        fs::create_dir_all(&self.packages_dir)
            .map_err(|e| format!("Failed to create .mote_packages directory: {}", e))?;

        for pkg in packages {
            let Some(src) = self.local_source_dir(&pkg.source).map(|d| d.join("src")) else {
                return Err(format!("cannot install '{}': '{}' is not a path dependency", pkg.name, pkg.source));
            };
            if !src.is_dir() {
                return Err(format!("cannot install '{}': {} is not a directory", pkg.name, src.display()));
            }
            let dst = self.packages_dir.join(&pkg.name).join("src");
            if dst.exists() {
                fs::remove_dir_all(&dst).map_err(|e| format!("Failed to clear '{}': {}", dst.display(), e))?;
            }
            copy_tree(&src, &dst)?;

            let native = self.packages_dir.join(&pkg.name).join("native");
            if native.exists() {
                fs::remove_dir_all(&native).map_err(|e| format!("Failed to clear '{}': {}", native.display(), e))?;
            }
            let host = host_triple();
            let from = self.local_source_dir(&pkg.source).map(|d| d.join("native").join(&host));
            if let Some(from) = from.filter(|d| d.is_dir()) {
                copy_tree(&from, &native.join(&host))?;
            }
        }

        Ok(())
    }

    /// The installed `mote.toml` of `name` when `.mote_packages/<name>` holds `source` with `checksum` and, for a package with native code, this machine's `native` checksum.
    pub(crate) fn installed_git(&self, name: &str, source: &str, checksum: Option<&str>, native: &BTreeMap<String, String>) -> Option<PackageManifest> {
        let dir = self.packages_dir.join(name);
        let marked = fs::read_to_string(dir.join(".source")).is_ok_and(|s| s.trim() == source);
        let intact = package_checksum(&dir).ok().is_some_and(|sum| checksum.is_none_or(|c| c == sum));
        let installed = dir_checksums(&dir).unwrap_or_default();
        let natives_intact = match native.get(&host_triple()) {
            Some(sum) => installed.get(&host_triple()) == Some(sum) && installed.len() == 1,
            None => native.is_empty() && installed.is_empty(),
        };
        if !(marked && intact && natives_intact) {
            return None;
        }
        PackageManifest::from_file(&dir.join("mote.toml")).ok()
    }

    /// The package in a git archive's `entries`, checked to be `name`, with the checksum of its `src/` tree and of each `native/<triple>/` tree.
    pub(crate) fn check_git(&self, name: &str, entries: &[(String, Vec<u8>)]) -> Result<(PackageManifest, String, BTreeMap<String, String>), String> {
        let toml = entries.iter().find(|(p, _)| p == "mote.toml").ok_or_else(|| format!("{name}: the repository has no mote.toml"))?;
        let manifest = PackageManifest::from_toml_str(&String::from_utf8_lossy(&toml.1)).map_err(|e| format!("{name}: {e}"))?;
        if manifest.package.name != name {
            return Err(format!("{name}: the repository's mote.toml names the package {}", manifest.package.name));
        }
        let files = entries.iter().filter_map(|(p, d)| Some((p.strip_prefix("src/")?.to_string(), d.as_slice()))).collect();
        Ok((manifest, tree_checksum(files), archive_checksums(entries)))
    }

    /// Writes `entries` into `.mote_packages/<name>/` and marks them as `source`; only this machine's `native/<triple>/` is written.
    pub(crate) fn install_git(&self, name: &str, source: &str, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
        let dst = self.packages_dir.join(name);
        if dst.exists() {
            fs::remove_dir_all(&dst).map_err(|e| format!("Failed to clear '{}': {}", dst.display(), e))?;
        }
        let host_native = format!("native/{}/", host_triple());
        for (path, data) in entries.iter().filter(|(p, _)| !p.starts_with("native/") || p.starts_with(&host_native)) {
            let target = dst.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
            }
            fs::write(&target, data).map_err(|e| format!("Failed to write '{}': {}", target.display(), e))?;
        }
        fs::write(dst.join(".source"), source).map_err(|e| format!("Failed to write source marker: {e}"))
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("Failed to create '{}': {}", to.display(), e))?;
    for entry in fs::read_dir(from).map_err(|e| format!("Failed to read '{}': {}", from.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .map_err(|e| format!("Failed to copy '{}': {}", entry.path().display(), e))?;
        }
    }
    Ok(())
}

/// `sha256:<hex>` over every file under `dir/src`, by sorted relative path, name and contents.
pub(crate) fn package_checksum(dir: &Path) -> Result<String, String> {
    dir_checksum(&dir.join("src"))
}

/// `sha256:<hex>` over every file under `root`, by sorted relative path, name and contents.
pub(crate) fn dir_checksum(root: &Path) -> Result<String, String> {
    let mut names = Vec::new();
    collect_files(root, "", &mut names)?;
    let mut contents = Vec::new();
    for rel in &names {
        contents.push(fs::read(root.join(rel)).map_err(|e| format!("Failed to read '{}': {}", rel, e))?);
    }
    Ok(tree_checksum(names.into_iter().zip(contents.iter().map(Vec::as_slice)).collect()))
}

/// `sha256:<hex>` over `files` (path under the tree, contents), sorted by path.
pub(crate) fn tree_checksum(mut files: Vec<(String, &[u8])>) -> String {
    files.sort();
    let mut hasher = Sha256::new();
    for (rel, bytes) in &files {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    format!("sha256:{}", hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("Failed to read '{}': {}", dir.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if entry.path().is_dir() {
            collect_files(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}
