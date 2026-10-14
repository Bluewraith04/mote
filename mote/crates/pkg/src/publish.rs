//! `mote publish`: packs the package, checks the archive builds on its own and uploads it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::cli::PackageManager;
use crate::config::Registries;
use crate::git::validate_name;
use crate::installer::copy_tree;
use crate::manifest::PackageManifest;
use crate::mpk::MpkArchive;
use crate::regdeps::check_paths;
use crate::registry::{archive_checksum, check_package_name};
use crate::semver::Version;

/// What a registry accepted.
#[derive(Debug)]
pub struct Published {
    pub name: String,
    pub version: Version,
    pub registry: String,
    pub checksum: String,
}

/// The largest archive a registry takes.
const LIMIT: usize = 32 * 1024 * 1024;

/// Publishes the package in `project_root` to `registry`, or the default one.
pub fn publish(project_root: &Path, registry: Option<&str>) -> Result<Published, String> {
    let manifest = PackageManifest::from_file(&project_root.join("mote.toml"))?;
    let meta = &manifest.package;
    check_package_name(&meta.name)?;
    for (key, value) in [("description", &meta.description), ("license", &meta.license)] {
        if value.as_deref().is_none_or(|v| v.trim().is_empty()) {
            return Err(format!("publishing needs `{key}` in [package] of mote.toml"));
        }
    }
    for (dep, spec) in &manifest.dependencies {
        if !spec.is_registry() {
            return Err(format!("a published package can only depend on registry packages; {dep} is a path or git dependency"));
        }
        validate_name(dep)?;
    }
    let target = Registries::for_manifest(&manifest)?.get(registry)?;
    target.require_token()?;
    if target.index(&meta.name).is_ok_and(|versions| versions.iter().any(|e| e.version == meta.version)) {
        return Err(format!("{} {} is already published in registry `{}`; a version is never replaced", meta.name, meta.version, target.name()));
    }
    if !project_root.join("README.md").is_file() {
        eprintln!("warning: no README.md; the registry's page for {} will have no readme", meta.name);
    }

    let archive = PackageManager::package(project_root)?;
    let bytes = fs::read(&archive).map_err(|e| format!("Failed to read '{}': {e}", archive.display()))?;
    if bytes.len() > LIMIT {
        return Err(format!("the archive is {} MB; a registry takes at most 32", bytes.len().div_ceil(1024 * 1024)));
    }
    check_unpacked(project_root, &bytes)?;

    let sum = target.publish(&bytes)?;
    let local = archive_checksum(&bytes);
    if sum != local {
        return Err(format!("registry `{}` stored {sum}, but the archive is {local}", target.name()));
    }
    Ok(Published { name: meta.name.clone(), version: meta.version.clone(), registry: target.name().to_string(), checksum: sum })
}

/// Unpacks `archive` on its own, next to the project's installed dependencies, and builds it.
fn check_unpacked(project_root: &Path, archive: &[u8]) -> Result<(), String> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let entries = MpkArchive::decode(archive)?;
    check_paths(&entries)?;
    let dir: PathBuf = std::env::temp_dir().join(format!("mote_publish_{}_{}", std::process::id(), COUNT.fetch_add(1, Ordering::SeqCst)));
    fs::remove_dir_all(&dir).ok();
    let built = (|| {
        for (path, data) in &entries {
            let target = dir.join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Failed to create '{}': {e}", parent.display()))?;
            }
            fs::write(&target, data).map_err(|e| format!("Failed to write '{}': {e}", target.display()))?;
        }
        if project_root.join(".mote_packages").is_dir() {
            copy_tree(&project_root.join(".mote_packages"), &dir.join(".mote_packages"))?;
        }
        PackageManager::compile_program(&dir).map(|_| ()).map_err(|e| format!("the archive does not build on its own: {e}"))
    })();
    fs::remove_dir_all(&dir).ok();
    built
}
