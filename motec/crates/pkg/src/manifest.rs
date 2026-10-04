use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use crate::git::{GitRef, Repo};
use crate::semver::Version;

#[derive(Clone, Debug, Serialize, Deserialize)]
/// The `[package]` table of `mote.toml`.
pub struct PackageMeta {
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default = "default_edition")]
    pub edition: String,
    #[serde(default = "default_entry")]
    pub entry: String,
}

fn default_edition() -> String {
    "2026".to_string()
}

fn default_entry() -> String {
    "src/main.mote".to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
/// How a dependency is specified in `mote.toml`.
pub enum DependencySpec {
    Simple(String),
    Detailed {
        version: Option<String>,
        path: Option<String>,
        git: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
        /// Grants the package's native libraries to the run.
        native: Option<bool>,
    },
}

impl DependencySpec {
    pub(crate) fn version_req_str(&self) -> &str {
        match self {
            DependencySpec::Simple(v) => v.as_str(),
            DependencySpec::Detailed { version, .. } => version.as_deref().unwrap_or("*"),
        }
    }

    pub fn path(&self) -> Option<&str> {
        match self {
            DependencySpec::Simple(_) => None,
            DependencySpec::Detailed { path, .. } => path.as_deref(),
        }
    }

    /// Whether `native = true` grants the package its native libraries.
    pub fn native(&self) -> bool {
        matches!(self, DependencySpec::Detailed { native: Some(true), .. })
    }

    /// The git repository and ref of a `git` dependency; `None` for any other.
    pub fn git(&self) -> Result<Option<(Repo, GitRef)>, String> {
        let DependencySpec::Detailed { git: Some(git), tag, branch, rev, .. } = self else { return Ok(None) };
        let repo = Repo::parse(git)?;
        let r = GitRef::from_fields(tag.as_deref(), branch.as_deref(), rev.as_deref())?;
        Ok(Some((repo, r)))
    }

    /// A `git` dependency pinned to a tag, branch or commit.
    pub fn from_git(repo: &Repo, r: &GitRef) -> Self {
        let (mut tag, mut branch, mut rev) = (None, None, None);
        match r {
            GitRef::Tag(t) => tag = Some(t.clone()),
            GitRef::Branch(b) => branch = Some(b.clone()),
            GitRef::Rev(c) => rev = Some(c.clone()),
        }
        DependencySpec::Detailed { version: None, path: None, git: Some(repo.spec()), tag, branch, rev, native: None }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// The contents of `mote.toml`.
pub struct PackageManifest {
    pub package: PackageMeta,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunConfig>,
}

/// `[run]`: limits `mote run` applies to the package's program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfig {
    /// The heap limit, such as `2GiB` or `unlimited`.
    #[serde(default, rename = "max-heap", skip_serializing_if = "Option::is_none")]
    pub max_heap: Option<String>,
}

impl PackageManifest {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("Failed to read '{:?}': {}", path, e))?;
        Self::from_toml_str(&text)
    }

    /// Walks up from `start` looking for a `mote.toml`; returns the directory it was found in and the parsed manifest.
    pub fn discover(start: &Path) -> Result<(std::path::PathBuf, Self), String> {
        let mut dir = if start.is_dir() {
            start.to_path_buf()
        } else {
            start.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        if let Ok(abs) = dir.canonicalize() {
            dir = abs;
        }
        loop {
            let candidate = dir.join("mote.toml");
            if candidate.is_file() {
                return Ok((dir.clone(), Self::from_file(&candidate)?));
            }
            if !dir.pop() {
                return Err("no mote.toml found in this directory or any parent".to_string());
            }
        }
    }

    /// The entry source file, resolved against `project_root`.
    pub fn entry_path(&self, project_root: &Path) -> std::path::PathBuf {
        project_root.join(&self.package.entry)
    }

    pub fn from_toml_str(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| format!("Failed to parse mote.toml: {}", e))
    }

    pub(crate) fn to_toml_string(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize mote.toml: {}", e))
    }

    pub(crate) fn save_to_file(&self, path: &Path) -> Result<(), String> {
        let toml_str = self.to_toml_string()?;
        fs::write(path, toml_str).map_err(|e| format!("Failed to write '{:?}': {}", path, e))
    }
}
