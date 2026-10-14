//! A package registry over HTTP: the sparse index, archive downloads and publishing.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::git::validate_name;
use crate::semver::Version;
use crate::store;

const INDEX_LIMIT: u64 = 16 * 1024 * 1024;
const ARCHIVE_LIMIT: u64 = 64 * 1024 * 1024;

/// One published version in the index.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct IndexEntry {
    pub version: Version,
    /// `sha256:<hex>` of the archive.
    pub checksum: String,
    #[serde(default)]
    pub yanked: bool,
    /// The registry packages this version needs, with their requirements.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    /// The oldest `mote` that builds it.
    #[serde(default)]
    pub mote: Option<Version>,
}

/// A registry: where it is and the token that publishes to it.
#[derive(Clone, Debug)]
pub struct Registry {
    name: String,
    url: String,
    token: Option<String>,
}

enum Answer {
    Reply(u16, Vec<u8>),
    Down(String),
}

impl Registry {
    pub fn new(name: &str, url: &str, token: Option<String>) -> Self {
        Registry { name: name.to_string(), url: url.trim_end_matches('/').to_string(), token }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every version of `package`, from the registry or the kept copy.
    pub fn index(&self, package: &str) -> Result<Vec<IndexEntry>, String> {
        validate_name(package)?;
        let path = format!("index/{}/{}/{package}", &package[..1], &package[1..2]);
        let cache = self.cache_dir().map(|d| d.join("index").join(package));
        let text = match self.get(&path, INDEX_LIMIT) {
            Answer::Reply(200, body) => {
                if let Some(cache) = &cache {
                    keep(cache, &body);
                }
                body
            }
            Answer::Reply(404, _) => return Err(format!("registry `{}` has no package `{package}`", self.name)),
            Answer::Reply(status, body) => return Err(self.failure(status, &body)),
            Answer::Down(why) => match cache.and_then(|c| fs::read(c).ok()) {
                Some(kept) => kept,
                None => return Err(format!("registry `{}` ({}) is not reachable: {why}", self.name, self.url)),
            },
        };
        let text = String::from_utf8(text).map_err(|_| format!("registry `{}` sent an index that is not text", self.name))?;
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(|e| format!("registry `{}` sent a bad index line for `{package}`: {e}", self.name)))
            .collect()
    }

    /// The archive of `package` at `version`, checked against `checksum`.
    pub fn archive(&self, package: &str, version: &Version, checksum: &str) -> Result<Vec<u8>, String> {
        validate_name(package)?;
        let cache = self.cache_dir().map(|d| d.join(package).join(format!("{version}.mpk")));
        if let Some(kept) = cache.as_ref().and_then(|c| fs::read(c).ok()).filter(|b| archive_checksum(b) == checksum) {
            return Ok(kept);
        }
        let bytes = match self.get(&format!("dl/{package}/{version}.mpk"), ARCHIVE_LIMIT) {
            Answer::Reply(200, body) => body,
            Answer::Reply(404, _) => return Err(format!("registry `{}` has no archive for {package} {version}", self.name)),
            Answer::Reply(status, body) => return Err(self.failure(status, &body)),
            Answer::Down(why) => return Err(format!("registry `{}` ({}) is not reachable: {why}", self.name, self.url)),
        };
        let sum = archive_checksum(&bytes);
        if sum != checksum {
            return Err(format!("{package} {version}: the archive is {sum}, but the registry's index says {checksum}"));
        }
        if let Some(cache) = &cache {
            keep(cache, &bytes);
        }
        Ok(bytes)
    }

    /// The token that publishes here, or how to give one.
    pub fn require_token(&self) -> Result<&str, String> {
        self.token.as_deref().ok_or_else(|| {
            format!(
                "publishing to `{}` needs a token; set MOTE_REGISTRY_TOKEN, or add token = \"...\" under [registries.{}] in ~/.mote/mote_config.toml",
                self.name, self.name
            )
        })
    }

    /// Uploads `archive`; answers the checksum the registry computed.
    pub fn publish(&self, archive: &[u8]) -> Result<String, String> {
        let token = self.require_token()?;
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(300)))
            .user_agent(user_agent())
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let url = format!("{}/publish", self.url);
        let mut response = agent
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/octet-stream")
            .send(archive)
            .map_err(|e| format!("registry `{}` ({}) is not reachable: {e}", self.name, self.url))?;
        let status = response.status().as_u16();
        let body = response.body_mut().with_config().limit(1 << 20).read_to_vec().unwrap_or_default();
        if status != 200 && status != 201 {
            return Err(self.failure(status, &body));
        }
        let reply: serde_json::Value = serde_json::from_slice(&body).map_err(|_| format!("registry `{}` answered something that is not JSON", self.name))?;
        reply["checksum"].as_str().map(str::to_string).ok_or_else(|| format!("registry `{}` did not answer a checksum", self.name))
    }

    fn get(&self, path: &str, limit: u64) -> Answer {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(5)
            .timeout_global(Some(Duration::from_secs(120)))
            .user_agent(user_agent())
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let mut response = match agent.get(format!("{}/{path}", self.url)).call() {
            Ok(r) => r,
            Err(e) => return Answer::Down(e.to_string()),
        };
        let status = response.status().as_u16();
        match response.body_mut().with_config().limit(limit).read_to_vec() {
            Ok(body) => Answer::Reply(status, body),
            Err(e) => Answer::Down(e.to_string()),
        }
    }

    fn failure(&self, status: u16, body: &[u8]) -> String {
        let text = String::from_utf8_lossy(body);
        let text = text.trim();
        let text: String = text.chars().take(300).collect();
        if text.is_empty() {
            format!("registry `{}` answered {status}", self.name)
        } else {
            format!("registry `{}` answered {status}: {text}", self.name)
        }
    }

    /// Where this registry's index and archives are kept.
    fn cache_dir(&self) -> Option<PathBuf> {
        store::home().ok().map(|h| h.join("registry").join(store::slug_of(&self.url)))
    }
}

fn user_agent() -> String {
    format!("mote/{}", env!("CARGO_PKG_VERSION"))
}

/// Names a registry refuses: the toolchain's own and the core libraries'.
const RESERVED: [&str; 11] = ["std", "mote", "core", "pane", "wire", "tile", "sqlite", "compress", "archive", "log", "path"];

/// Fails unless `name` is 2 to 40 characters, `[a-z][a-z0-9_]*`, and not reserved.
pub fn check_package_name(name: &str) -> Result<(), String> {
    validate_name(name)?;
    if !(2..=40).contains(&name.len()) {
        return Err(format!("'{name}' is not a registry package name (2 to 40 characters)"));
    }
    if RESERVED.contains(&name) {
        return Err(format!("'{name}' is a reserved name"));
    }
    Ok(())
}

/// `sha256:<hex>` of an archive's bytes.
pub fn archive_checksum(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// Keeps `bytes` at `path`.
fn keep(path: &std::path::Path, bytes: &[u8]) {
    let Some(parent) = path.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    if fs::write(&tmp, bytes).is_err() || fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}
