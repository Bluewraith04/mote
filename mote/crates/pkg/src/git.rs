//! Git as the package source: any repository at a tag, branch or commit, read with the `git` command.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::semver::Version;

const DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;
const SCHEMES: [&str; 5] = ["https://", "http://", "ssh://", "git://", "file://"];

/// A repository, written as a URL, `user@host:path` or `github:user/repo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    spec: String,
}

impl Repo {
    /// Parses a repository as written in `mote.toml` or `mote add`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let bad = || format!("'{spec}' is not a git repository; write a URL such as https://host/user/repo or github:user/repo");
        if spec.is_empty() || spec.starts_with('-') || spec.contains("::") || spec.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '\\')) {
            return Err(bad());
        }
        if let Some(rest) = spec.strip_prefix("github:") {
            let (user, name) = rest.split_once('/').ok_or_else(bad)?;
            for part in [user, name] {
                let ok = !part.is_empty() && part != "." && part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
                if !ok {
                    return Err(bad());
                }
            }
            return Ok(Self { spec: spec.to_string() });
        }
        let url = SCHEMES.iter().any(|s| spec.len() > s.len() && spec.starts_with(s));
        let scp = spec.split_once(':').is_some_and(|(host, path)| !host.is_empty() && !path.is_empty() && !host.contains('/') && !path.starts_with('/'));
        if url || (scp && !spec.contains("://")) {
            Ok(Self { spec: spec.to_string() })
        } else {
            Err(bad())
        }
    }

    /// The repository as written.
    pub fn spec(&self) -> String {
        self.spec.clone()
    }

    fn url(&self) -> String {
        match self.spec.strip_prefix("github:") {
            Some(rest) => format!("https://github.com/{rest}"),
            None => self.spec.clone(),
        }
    }
}

/// What a git dependency pins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitRef {
    Tag(String),
    Branch(String),
    Rev(String),
}

impl GitRef {
    /// The one of `tag`, `branch` and `rev` that is set.
    pub fn from_fields(tag: Option<&str>, branch: Option<&str>, rev: Option<&str>) -> Result<Self, String> {
        let r = match (tag, branch, rev) {
            (Some(t), None, None) => Self::Tag(t.to_string()),
            (None, Some(b), None) => Self::Branch(b.to_string()),
            (None, None, Some(r)) => Self::Rev(r.to_string()),
            (None, None, None) => return Err("a git dependency needs one of tag, branch or rev".to_string()),
            _ => return Err("a git dependency takes only one of tag, branch and rev".to_string()),
        };
        r.check()?;
        Ok(r)
    }

    /// `tag=v1.2.0`, as written in a lock source.
    pub fn label(&self) -> String {
        match self {
            Self::Tag(t) => format!("tag={t}"),
            Self::Branch(b) => format!("branch={b}"),
            Self::Rev(r) => format!("rev={r}"),
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Tag(n) | Self::Branch(n) | Self::Rev(n) => n,
        }
    }

    fn check(&self) -> Result<(), String> {
        let n = self.name();
        let ok = !n.is_empty()
            && !n.starts_with(['-', '/'])
            && !n.ends_with('/')
            && !n.contains("..")
            && !n.contains("//")
            && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+'));
        if !ok {
            return Err(format!("'{n}' is not a valid tag, branch or commit name"));
        }
        if matches!(self, Self::Rev(_)) && !is_commit(n) {
            return Err(format!("rev '{n}' must be a full 40-digit commit hash"));
        }
        Ok(())
    }
}

fn is_commit(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The `git` command, run without prompts and only over the transports a package may use.
#[derive(Default)]
pub struct Git;

impl Git {
    /// The commit a tag, branch or commit names.
    pub fn resolve(&self, repo: &Repo, r: &GitRef) -> Result<String, String> {
        r.check()?;
        match r {
            GitRef::Rev(rev) => Ok(rev.to_ascii_lowercase()),
            GitRef::Tag(name) => self.tag(repo, name)?.ok_or_else(|| format!("{} has no tag '{name}'", repo.spec())),
            GitRef::Branch(name) => self.branch(repo, name)?.ok_or_else(|| format!("{} has no branch '{name}'", repo.spec())),
        }
    }

    /// What `name` is in `repo`: a commit, a tag or a branch.
    pub fn classify(&self, repo: &Repo, name: &str) -> Result<GitRef, String> {
        if is_commit(name) {
            return GitRef::from_fields(None, None, Some(name));
        }
        GitRef::from_fields(Some(name), None, None)?;
        if self.tag(repo, name)?.is_some() {
            return Ok(GitRef::Tag(name.to_string()));
        }
        if self.branch(repo, name)?.is_some() {
            return Ok(GitRef::Branch(name.to_string()));
        }
        Err(format!("{} has no tag or branch '{name}'", repo.spec()))
    }

    /// The highest release tag of `repo`.
    pub fn latest_tag(&self, repo: &Repo) -> Result<String, String> {
        let lines = self.remote(repo, &["--tags"], &[])?;
        lines
            .iter()
            .filter_map(|(_, name)| name.strip_prefix("refs/tags/"))
            .filter(|name| !name.ends_with("^{}"))
            .filter_map(|name| Some((name.strip_prefix('v').unwrap_or(name).parse::<Version>().ok()?, name)))
            .max_by_key(|(v, _)| (!v.is_prerelease(), v.clone()))
            .map(|(_, name)| name.to_string())
            .ok_or_else(|| format!("{} has no version tags; name a branch or commit with {}@<ref>", repo.spec(), repo.spec()))
    }

    /// `mote.toml`, `src/**` and `native/**` of `repo` at `commit`.
    pub fn files(&self, repo: &Repo, commit: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
        let scratch = Scratch::new()?;
        let dir = scratch.path();
        run(&["init", "-q"], Some(dir))?;
        let url = repo.url();
        let shallow = run(&["fetch", "-q", "--depth", "1", &url, commit], Some(dir));
        if shallow.is_err() {
            run(&["fetch", "-q", &url, "+refs/heads/*:refs/remotes/origin/*", "+refs/tags/*:refs/tags/*"], Some(dir))
                .map_err(|e| format!("{} cannot be read: {e}", repo.spec()))?;
        }
        let object = format!("{commit}^{{commit}}");
        run(&["cat-file", "-e", &object], Some(dir)).map_err(|_| format!("{} has no commit {commit}", repo.spec()))?;
        let listing = run(&["ls-tree", "--name-only", &object], Some(dir))?;
        let top: Vec<&str> = std::str::from_utf8(&listing).unwrap_or("").lines().collect();
        if !top.contains(&"mote.toml") {
            return Err(format!("{} has no mote.toml", repo.spec()));
        }
        let mut paths = vec!["mote.toml"];
        for dir in ["src", "native"] {
            if top.contains(&dir) {
                paths.push(dir);
            }
        }
        let mut args = vec!["archive", "--format=tar", "--prefix=pkg/", object.as_str(), "--"];
        args.extend(paths);
        let tar = run(&args, Some(dir))?;
        unpack(tar.as_slice()).map_err(|e| format!("{}: {e}", repo.spec()))
    }

    fn tag(&self, repo: &Repo, name: &str) -> Result<Option<String>, String> {
        let plain = format!("refs/tags/{name}");
        let peeled = format!("refs/tags/{name}^{{}}");
        let lines = self.remote(repo, &[], &[&plain, &peeled])?;
        let find = |want: &str| lines.iter().find(|(_, r)| r == want).map(|(sha, _)| sha.clone());
        Ok(find(&peeled).or_else(|| find(&plain)))
    }

    fn branch(&self, repo: &Repo, name: &str) -> Result<Option<String>, String> {
        let full = format!("refs/heads/{name}");
        Ok(self.remote(repo, &[], &[&full])?.into_iter().find(|(_, r)| *r == full).map(|(sha, _)| sha))
    }

    /// `(commit, ref)` pairs of `git ls-remote`.
    fn remote(&self, repo: &Repo, options: &[&str], patterns: &[&str]) -> Result<Vec<(String, String)>, String> {
        let url = repo.url();
        let mut args = vec!["ls-remote"];
        args.extend(options);
        args.push(url.as_str());
        args.extend(patterns);
        let out = run(&args, None).map_err(|e| format!("{} cannot be read: {e}", repo.spec()))?;
        let text = String::from_utf8_lossy(&out);
        Ok(text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .filter(|(sha, _)| is_commit(sha))
            .map(|(sha, name)| (sha.to_ascii_lowercase(), name.to_string()))
            .collect())
    }
}

fn run(args: &[&str], dir: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https:http:ssh:git:file")
        .stdin(Stdio::null());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let out = command.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "git is not installed; mote reads git dependencies with it".to_string()
        } else {
            format!("failed to run git: {e}")
        }
    })?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("git failed");
    Err(line.trim_start_matches("fatal: ").to_string())
}

/// A scratch repository directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("mote_git_{}_{}", std::process::id(), COUNT.fetch_add(1, Ordering::SeqCst)));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

/// `mote.toml`, `src/**` and `native/<triple>/**` of a tar stream, with its top directory removed.
pub(crate) fn unpack(bytes: impl Read) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = tar::Archive::new(bytes);
    let mut out = Vec::new();
    let mut total = 0u64;
    for entry in archive.entries().map_err(|e| format!("not a tar archive: {e}"))? {
        let entry = entry.map_err(|e| format!("bad archive entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let raw = entry.path().map_err(|e| format!("bad archive entry: {e}"))?.to_string_lossy().into_owned();
        let Some((_, rel)) = raw.split_once('/') else { continue };
        if rel != "mote.toml" && !rel.starts_with("src/") && !rel.starts_with("native/") {
            continue;
        }
        if raw.starts_with('/') || rel.contains('\\') || rel.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
            return Err(format!("archive entry '{raw}' is not a safe relative path"));
        }
        let mut data = Vec::new();
        entry.take(DOWNLOAD_LIMIT.saturating_sub(total) + 1).read_to_end(&mut data).map_err(|e| format!("bad archive entry '{raw}': {e}"))?;
        total += data.len() as u64;
        if total > DOWNLOAD_LIMIT {
            return Err("the package is larger than 64 MiB".to_string());
        }
        out.push((rel.to_string(), data));
    }
    Ok(out)
}

/// Package names are `[a-z][a-z0-9_]*`, at most 64 bytes.
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    let ok = name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("'{name}' is not a valid package name (lowercase letters, digits and _, starting with a letter)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_are_urls_scp_paths_or_the_github_shorthand() {
        for ok in ["github:ann/util", "https://gitlab.com/ann/util.git", "ssh://git@host/ann/util", "file:///tmp/util", "git@codeberg.org:ann/util.git"] {
            assert!(Repo::parse(ok).is_ok(), "{ok}");
        }
        for bad in ["", "util", "ann/util", "-oops", "ext::sh -c x", "https://h/a b", "https://h/a?x=1", "github:ann", "github:ann/..", "/tmp/util"] {
            assert!(Repo::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Repo::parse("github:ann/util").unwrap().url(), "https://github.com/ann/util");
    }

    #[test]
    fn unsafe_tar_paths_are_rejected() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        let name = b"pkg/src/../../x.mote";
        header.as_old_mut().name[..name.len()].copy_from_slice(name);
        header.set_size(1);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
        let tar = builder.into_inner().unwrap();
        assert!(unpack(tar.as_slice()).unwrap_err().contains("not a safe relative path"));
    }
}
