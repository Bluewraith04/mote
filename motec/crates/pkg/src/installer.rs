use std::fs;
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};
use crate::manifest::PackageManifest;
use crate::mpk::MpkArchive;
use crate::registry::{archive_checksum, hex};
use crate::resolver::ResolvedPackage;
use crate::semver::Version;

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
        }

        Ok(())
    }

    /// Whether `.mote_packages/<name>` holds the archive with `checksum`.
    pub(crate) fn has_archive(&self, name: &str, checksum: &str) -> bool {
        fs::read_to_string(self.packages_dir.join(name).join(".checksum")).is_ok_and(|s| s.trim() == checksum)
    }

    /// Verifies `bytes` against `checksum`, then unpacks `mote.toml` and `src/` into `.mote_packages/<name>/`.
    pub(crate) fn install_archive(&self, name: &str, version: &Version, checksum: &str, bytes: &[u8]) -> Result<(), String> {
        let actual = archive_checksum(bytes);
        if actual != checksum {
            return Err(format!("{name} {version}: downloaded archive has checksum {actual}, expected {checksum}"));
        }
        let entries = MpkArchive::decode(bytes).map_err(|e| format!("{name} {version}: {e}"))?;
        if let Some((path, _)) = entries
            .iter()
            .find(|(p, _)| p.starts_with('/') || p.contains('\\') || p.split('/').any(|s| s.is_empty() || s == "." || s == ".."))
        {
            return Err(format!("{name} {version}: archive entry '{path}' is not a safe relative path"));
        }
        let toml = entries
            .iter()
            .find(|(p, _)| p == "mote.toml")
            .ok_or_else(|| format!("{name} {version}: archive has no mote.toml"))?;
        let manifest = PackageManifest::from_toml_str(&String::from_utf8_lossy(&toml.1))
            .map_err(|e| format!("{name} {version}: {e}"))?;
        if manifest.package.name != name || manifest.package.version != *version {
            return Err(format!(
                "{name} {version}: archive's mote.toml is {} {}",
                manifest.package.name, manifest.package.version
            ));
        }

        let keep: Vec<_> = entries.into_iter().filter(|(p, _)| p == "mote.toml" || p.starts_with("src/")).collect();

        let dst = self.packages_dir.join(name);
        if dst.exists() {
            fs::remove_dir_all(&dst).map_err(|e| format!("Failed to clear '{}': {}", dst.display(), e))?;
        }
        for (path, data) in &keep {
            let target = dst.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
            }
            fs::write(&target, data).map_err(|e| format!("Failed to write '{}': {}", target.display(), e))?;
        }
        fs::write(dst.join(".checksum"), checksum).map_err(|e| format!("Failed to write checksum marker: {e}"))
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
    let src = dir.join("src");
    let mut files = Vec::new();
    collect_files(&src, "", &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for rel in &files {
        let bytes = fs::read(src.join(rel)).map_err(|e| format!("Failed to read '{}': {}", rel, e))?;
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("sha256:{}", hex(&hasher.finalize())))
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
