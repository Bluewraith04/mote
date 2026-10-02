//! Registry client: a static tree of `index/<name>.toml` and `packages/<name>/<version>.mpk`.

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::PackageManifest;
use crate::semver::Version;

/// Environment variable overriding `[registry] url`.
pub(crate) const REGISTRY_ENV: &str = "MOTE_REGISTRY";

const DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

/// One published version in a package's index file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub version: Version,
    pub checksum: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub yanked: bool,
}

/// `index/<name>.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFile {
    #[serde(default, rename = "version")]
    pub versions: Vec<IndexEntry>,
}

#[derive(Debug)]
/// A package registry on disk or over HTTP.
pub struct Registry {
    url: String,
}

impl Registry {
    /// A registry at an `https://` or `file://` URL.
    pub fn new(url: &str) -> Result<Self, String> {
        let url = url.trim().trim_end_matches('/');
        if !url.starts_with("https://") && !url.starts_with("file://") {
            return Err(format!("registry URL '{url}' must start with https:// or file://"));
        }
        Ok(Self { url: url.to_string() })
    }

    /// The registry from `MOTE_REGISTRY`, then `[registry] url`; `None` when neither is set.
    pub fn configured(manifest: &PackageManifest) -> Result<Option<Self>, String> {
        match std::env::var(REGISTRY_ENV).ok().filter(|v| !v.trim().is_empty()) {
            Some(url) => Self::new(&url).map(Some),
            None => manifest.registry.as_ref().map(|r| Self::new(&r.url)).transpose(),
        }
    }

    /// Like [`configured`](Self::configured), but an error naming both settings when neither is set.
    pub fn required(manifest: &PackageManifest, dependency: &str) -> Result<Self, String> {
        Self::configured(manifest)?.ok_or_else(|| {
            format!("'{dependency}' is a registry dependency, but no registry is configured; set [registry] url in mote.toml or {REGISTRY_ENV}")
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// The `mote.lock` source of packages from this registry.
    pub fn source(&self) -> String {
        format!("registry+{}", self.url)
    }

    /// Every version listed in `index/<name>.toml`.
    pub fn index(&self, name: &str) -> Result<Vec<IndexEntry>, String> {
        validate_name(name)?;
        let bytes = self
            .get(&format!("index/{name}.toml"))?
            .ok_or_else(|| format!("package '{name}' is not in the registry at {}", self.url))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("index for '{name}' is not UTF-8"))?;
        let file: IndexFile = toml::from_str(&text).map_err(|e| format!("index for '{name}' is malformed: {e}"))?;
        Ok(file.versions)
    }

    /// The bytes of `packages/<name>/<version>.mpk`.
    pub fn archive(&self, name: &str, version: &Version) -> Result<Vec<u8>, String> {
        validate_name(name)?;
        self.get(&format!("packages/{name}/{version}.mpk"))?
            .ok_or_else(|| format!("{name} {version} is in the index but its archive is missing from {}", self.url))
    }

    fn get(&self, rel: &str) -> Result<Option<Vec<u8>>, String> {
        let full = format!("{}/{rel}", self.url);
        if let Some(path) = self.url.strip_prefix("file://") {
            let path = format!("{path}/{rel}");
            return match fs::read(&path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("failed to read {full}: {e}")),
            };
        }
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into();
        match agent.get(&full).call() {
            Ok(mut resp) => resp
                .body_mut()
                .with_config()
                .limit(DOWNLOAD_LIMIT)
                .read_to_vec()
                .map(Some)
                .map_err(|e| format!("failed to download {full}: {e}")),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(format!("failed to fetch {full}: {e}")),
        }
    }
}

/// Registry names are `[a-z][a-z0-9_]*`, at most 64 bytes.
pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    let ok = name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("'{name}' is not a valid registry package name (lowercase letters, digits and _, starting with a letter)"))
    }
}

/// `sha256:<hex>` of `bytes`.
pub fn archive_checksum(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
