//! The mote home: a cache of fetched git packages by commit, and the directories programs are installed into.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use sha2::{Digest, Sha256};

use crate::git::{Git, Repo};
use crate::installer::tree_checksum;

const SUM_FILE: &str = ".sum";

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

/// Where `mote install` puts programs.
pub fn bin_dir() -> Result<PathBuf, String> {
    Ok(home()?.join("bin"))
}

/// Where `mote install` checks a package out to build it.
pub fn build_dir() -> Result<PathBuf, String> {
    Ok(home()?.join("build"))
}

/// The kept files of fetched packages, one directory per repository and commit.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store in the mote home, or none when there is no home.
    pub fn open() -> Option<Store> {
        home().ok().map(|h| Store::at(h.join("git")))
    }

    /// A store in `root`.
    pub fn at(root: PathBuf) -> Store {
        Store { root }
    }

    /// The files of `repo` at `commit`: the kept copy when it is intact, else what `fetch` answers, which is then kept.
    pub fn files(&self, repo: &Repo, commit: &str, fetch: impl FnOnce() -> Result<Vec<(String, Vec<u8>)>, String>) -> Result<Vec<(String, Vec<u8>)>, String> {
        if commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return fetch();
        }
        let dir = self.root.join(slug(repo)).join(commit.to_ascii_lowercase());
        if let Some(kept) = read_entry(&dir) {
            return Ok(kept);
        }
        let fetched = fetch()?;
        let _ = write_entry(&dir, &fetched);
        Ok(fetched)
    }
}

/// The files of `repo` at `commit` through the store in the mote home, fetched with `git` when they are not kept.
pub fn fetch_files(git: &Git, repo: &Repo, commit: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    match Store::open() {
        Some(store) => store.files(repo, commit, || git.files(repo, commit)),
        None => git.files(repo, commit),
    }
}

/// A directory name for a repository: its text with unsafe characters as `_`, then a short hash of the exact text.
fn slug(repo: &Repo) -> String {
    let spec = repo.spec();
    let readable: String = spec.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') { c } else { '_' }).take(48).collect();
    let digest = Sha256::digest(spec.as_bytes());
    let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{readable}-{hash}")
}

/// The files kept in `dir`, when they match the checksum written with them.
fn read_entry(dir: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    let want = fs::read_to_string(dir.join(SUM_FILE)).ok()?;
    let mut names = Vec::new();
    collect(dir, "", &mut names).ok()?;
    let mut files = Vec::new();
    for rel in names.into_iter().filter(|n| n != SUM_FILE) {
        let bytes = fs::read(dir.join(&rel)).ok()?;
        files.push((rel, bytes));
    }
    let sum = tree_checksum(files.iter().map(|(p, d)| (p.clone(), d.as_slice())).collect());
    if sum == want.trim() {
        Some(files)
    } else {
        fs::remove_dir_all(dir).ok();
        None
    }
}

/// Keeps `files` in `dir`, through a temporary directory renamed into place, so no reader sees half of them.
fn write_entry(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let parent = dir.parent().ok_or("a kept package has a parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("Failed to create '{}': {e}", parent.display()))?;
    let tmp = parent.join(format!(".tmp-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::SeqCst)));
    fs::remove_dir_all(&tmp).ok();
    let written = (|| {
        for (path, data) in files {
            let target = tmp.join(path);
            if let Some(p) = target.parent() {
                fs::create_dir_all(p).map_err(|e| format!("Failed to create '{}': {e}", p.display()))?;
            }
            fs::write(&target, data).map_err(|e| format!("Failed to write '{}': {e}", target.display()))?;
        }
        let sum = tree_checksum(files.iter().map(|(p, d)| (p.clone(), d.as_slice())).collect());
        fs::write(tmp.join(SUM_FILE), sum).map_err(|e| format!("Failed to write the checksum: {e}"))
    })();
    let renamed = written.and_then(|()| {
        fs::remove_dir_all(dir).ok();
        fs::rename(&tmp, dir).map_err(|e| format!("Failed to keep '{}': {e}", dir.display()))
    });
    if renamed.is_err() {
        fs::remove_dir_all(&tmp).ok();
    }
    renamed
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if entry.path().is_dir() {
            collect(&entry.path(), &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mote_store_{name}_{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    fn files() -> Vec<(String, Vec<u8>)> {
        vec![("mote.toml".to_string(), b"[package]".to_vec()), ("src/lib.mote".to_string(), b"pub fn a() {}".to_vec())]
    }

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_package_is_fetched_once_and_then_read_from_the_store() {
        let root = scratch("once");
        let store = Store::at(root.clone());
        let repo = Repo::parse("github:user/lib").unwrap();
        let fetches = Cell::new(0);
        let fetch = || {
            fetches.set(fetches.get() + 1);
            Ok(files())
        };
        assert_eq!(store.files(&repo, COMMIT, fetch).unwrap(), files());
        let second = store.files(&repo, COMMIT, || Err("must not fetch again".to_string())).unwrap();
        assert_eq!(second.len(), 2);
        assert!(second.contains(&("src/lib.mote".to_string(), b"pub fn a() {}".to_vec())));
        assert_eq!(fetches.get(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_edited_entry_is_fetched_again() {
        let root = scratch("edited");
        let store = Store::at(root.clone());
        let repo = Repo::parse("github:user/lib").unwrap();
        store.files(&repo, COMMIT, || Ok(files())).unwrap();
        let dir = root.join(slug(&repo)).join(COMMIT);
        fs::write(dir.join("src/lib.mote"), b"edited").unwrap();
        let again = store.files(&repo, COMMIT, || Ok(files())).unwrap();
        assert!(again.contains(&("src/lib.mote".to_string(), b"pub fn a() {}".to_vec())));
        assert_eq!(fs::read(dir.join("src/lib.mote")).unwrap(), b"pub fn a() {}", "the entry is healed");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn repositories_and_commits_are_kept_apart() {
        let root = scratch("apart");
        let store = Store::at(root.clone());
        let a = Repo::parse("github:user/a").unwrap();
        let b = Repo::parse("github:user/b").unwrap();
        assert_ne!(slug(&a), slug(&b));
        store.files(&a, COMMIT, || Ok(files())).unwrap();
        let other = store.files(&b, COMMIT, || Ok(vec![("mote.toml".to_string(), b"b".to_vec())])).unwrap();
        assert_eq!(other.len(), 1);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_name_that_is_not_a_commit_is_never_kept() {
        let root = scratch("name");
        let store = Store::at(root.clone());
        let repo = Repo::parse("github:user/lib").unwrap();
        store.files(&repo, "../escape", || Ok(files())).unwrap();
        assert!(!root.exists());
    }
}
