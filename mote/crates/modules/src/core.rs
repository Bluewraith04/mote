//! Where the core libraries (`pane`, `wire`, `tile`, …) live and how a missing one is fetched.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The core libraries, by import name.
pub const LIBRARIES: [&str; 8] = ["archive", "compress", "log", "pane", "path", "sqlite", "tile", "wire"];

/// The version of `mote`, which names the core component it uses.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether `name` is a core library.
pub fn is_library(name: &str) -> bool {
    LIBRARIES.contains(&name)
}

/// The mote home: `$MOTE_HOME`, else `.mote` in the user's home directory.
pub fn home() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("MOTE_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d).join(".mote"))
        .ok_or_else(|| format!("no home directory; set MOTE_HOME or {var}"))
}

/// Where the fetched core libraries of this version are kept: `<home>/core/<version>/`.
pub fn store_dir() -> Result<PathBuf, String> {
    Ok(home()?.join("core").join(VERSION))
}

/// The directories searched for core libraries.
pub fn core_dirs() -> Vec<PathBuf> {
    let set = std::env::var_os("MOTE_CORE_PATH");
    if let Some(paths) = set.as_ref().filter(|p| !p.is_empty()) {
        return std::env::split_paths(paths).filter(|p| p.is_dir()).collect();
    }
    let mut dirs = Vec::new();
    if set.is_none() {
        let built_from = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../core"));
        dirs.extend(built_from.canonicalize().ok().filter(|p| p.is_dir()));
    }
    dirs.extend(store_dir().ok().filter(|p| p.is_dir()));
    dirs
}

/// Whether library `name` is in `dir`.
pub fn has_library(dir: &Path, name: &str) -> bool {
    dir.join(name).join("src").join("lib.mote").is_file()
}

/// Whether library `name` is in any of the core directories.
pub fn found(name: &str) -> bool {
    core_dirs().iter().any(|d| has_library(d, name))
}

type Fetcher = fn() -> Result<(), String>;

static FETCHER: OnceLock<Fetcher> = OnceLock::new();
static FETCHED: OnceLock<Result<(), String>> = OnceLock::new();

/// Registers what fetches the core component.
pub fn set_fetcher(f: Fetcher) {
    let _ = FETCHER.set(f);
}

/// Fetches the core component once per process.
pub fn fetch_missing() -> Result<bool, String> {
    let Some(fetch) = FETCHER.get() else { return Ok(false) };
    FETCHED.get_or_init(fetch).clone().map(|()| true)
}
