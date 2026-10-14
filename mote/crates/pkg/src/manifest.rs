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
    /// The oldest `mote` that builds this package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mote: Option<Version>,
    /// One line on what the package is; a registry requires it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// An SPDX license id such as `MIT`; a registry requires it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

/// The version of this toolchain.
pub fn toolchain_version() -> Version {
    env!("CARGO_PKG_VERSION").parse().expect("the crate version is a version")
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
        /// The registry a version requirement resolves through.
        registry: Option<String>,
    },
}

impl DependencySpec {
    pub fn version_req_str(&self) -> &str {
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
        DependencySpec::Detailed { version: None, path: None, git: Some(repo.spec()), tag, branch, rev, registry: None }
    }

    /// A version requirement with no path or git source: it resolves through a registry.
    pub fn is_registry(&self) -> bool {
        match self {
            DependencySpec::Simple(_) => true,
            DependencySpec::Detailed { path, git, .. } => path.is_none() && git.is_none(),
        }
    }

    /// The registry a registry dependency names; `None` for the default one.
    pub fn registry(&self) -> Option<&str> {
        match self {
            DependencySpec::Simple(_) => None,
            DependencySpec::Detailed { registry, .. } => registry.as_deref(),
        }
    }

    /// A registry dependency on versions matching `req`, through `registry` or the default one.
    pub fn from_registry(req: &str, registry: Option<&str>) -> Self {
        match registry {
            None => DependencySpec::Simple(req.to_string()),
            Some(name) => DependencySpec::Detailed {
                version: Some(req.to_string()),
                path: None,
                git: None,
                tag: None,
                branch: None,
                rev: None,
                registry: Some(name.to_string()),
            },
        }
    }
}

/// A registry named in `mote.toml`: its URL, and a token that is rejected there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RegistryEntry {
    Url(String),
    Table { url: String, token: Option<String> },
}

impl RegistryEntry {
    pub fn url(&self) -> &str {
        match self {
            RegistryEntry::Url(url) | RegistryEntry::Table { url, .. } => url,
        }
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
    /// Registries by name; tokens live in `~/.mote/mote_config.toml`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub registries: BTreeMap<String, RegistryEntry>,
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

    /// Walks up from `start` to the nearest `mote.toml` and parses it.
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
        let manifest: Self = toml::from_str(toml_str).map_err(|e| format!("Failed to parse mote.toml: {}", e))?;
        for (name, entry) in &manifest.registries {
            if matches!(entry, RegistryEntry::Table { token: Some(_), .. }) {
                return Err(format!("registry `{name}`: a token belongs in ~/.mote/mote_config.toml, not mote.toml"));
            }
        }
        match &manifest.package.mote {
            Some(needed) if *needed > toolchain_version() => Err(format!(
                "package `{}` needs mote {needed} or newer; this is mote {}",
                manifest.package.name,
                toolchain_version()
            )),
            _ => Ok(manifest),
        }
    }

    pub(crate) fn to_toml_string(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize mote.toml: {}", e))
    }

    pub(crate) fn save_to_file(&self, path: &Path) -> Result<(), String> {
        let toml_str = self.to_toml_string()?;
        fs::write(path, toml_str).map_err(|e| format!("Failed to write '{:?}': {}", path, e))
    }
}
