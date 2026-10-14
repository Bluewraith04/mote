//! The core library component: one archive per release and host.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::native::host_triple;

const DEFAULT_BASE: &str = "https://github.com/Bluewraith04/mote/releases/download";
const LIMIT: u64 = 256 << 20;

/// Where the archives are: `MOTE_CORE_URL`, else the public releases.
pub fn base_url() -> String {
    match std::env::var("MOTE_CORE_URL") {
        Ok(url) if !url.is_empty() => url,
        _ => format!("{DEFAULT_BASE}/v{}", modules::core::VERSION),
    }
}

/// The archive of this version for this machine.
pub fn archive_name() -> String {
    format!("mote-core-{}-{}.tar.gz", modules::core::VERSION, host_triple())
}

/// Whether `MOTE_OFFLINE` forbids fetching.
pub fn offline() -> bool {
    std::env::var("MOTE_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0")
}

fn download(base: &str, name: &str) -> Result<Vec<u8>, String> {
    if base.starts_with("http://") || base.starts_with("https://") {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(600)))
            .user_agent(format!("mote/{}", modules::core::VERSION))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let url = format!("{}/{name}", base.trim_end_matches('/'));
        let mut response = agent.get(&url).call().map_err(|e| format!("{url} is not reachable: {e}"))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(format!("{url} answered {status}"));
        }
        return response.body_mut().with_config().limit(LIMIT).read_to_vec().map_err(|e| format!("reading {url}: {e}"));
    }
    let dir = base.strip_prefix("file://").unwrap_or(base);
    let path = Path::new(dir).join(name);
    fs::read(&path).map_err(|e| format!("cannot read '{}': {e}", path.display()))
}

fn checksum(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether every core library is in `dir`.
fn complete(dir: &Path) -> bool {
    modules::core::LIBRARIES.iter().all(|name| modules::core::has_library(dir, name))
}

/// The files of a core archive, `core/` removed from their paths.
fn unpack(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut out = Vec::new();
    let mut total = 0u64;
    for entry in archive.entries().map_err(|e| format!("not a core archive: {e}"))? {
        let entry = entry.map_err(|e| format!("bad archive entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let raw = entry.path().map_err(|e| format!("bad archive entry: {e}"))?.to_string_lossy().into_owned();
        let Some(rel) = raw.strip_prefix("core/") else { continue };
        if rel.contains('\\') || rel.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
            return Err(format!("archive entry '{raw}' is not a safe relative path"));
        }
        let mut data = Vec::new();
        entry.take(LIMIT.saturating_sub(total) + 1).read_to_end(&mut data).map_err(|e| format!("bad archive entry '{raw}': {e}"))?;
        total += data.len() as u64;
        if total > LIMIT {
            return Err("the core archive is larger than 256 MiB".to_string());
        }
        out.push((rel.to_string(), data));
    }
    Ok(out)
}

/// Downloads the core archive for this version and machine and installs it in the store.
pub fn fetch(force: bool) -> Result<PathBuf, String> {
    let dest = modules::core::store_dir()?;
    if !force && complete(&dest) {
        return Ok(dest);
    }
    let base = base_url();
    let name = archive_name();
    let bytes = download(&base, &name)?;
    let sidecar = download(&base, &format!("{name}.sha256"))?;
    let expected = String::from_utf8_lossy(&sidecar).split_whitespace().next().unwrap_or("").to_string();
    let actual = checksum(&bytes);
    if expected != actual {
        return Err(format!("{name} is {actual}, but its .sha256 says {expected}"));
    }
    let files = unpack(&bytes)?;
    let parent = dest.parent().ok_or("no core directory")?.to_path_buf();
    fs::create_dir_all(&parent).map_err(|e| format!("Failed to create '{}': {e}", parent.display()))?;
    let scratch = parent.join(format!(".fetch-{}", std::process::id()));
    fs::remove_dir_all(&scratch).ok();
    for (rel, data) in &files {
        let path = scratch.join(rel);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Failed to create '{}': {e}", dir.display()))?;
        }
        fs::write(&path, data).map_err(|e| format!("Failed to write '{}': {e}", path.display()))?;
    }
    if !complete(&scratch) {
        fs::remove_dir_all(&scratch).ok();
        return Err(format!("{name} does not hold every core library"));
    }
    fs::remove_dir_all(&dest).ok();
    fs::rename(&scratch, &dest).map_err(|e| format!("Failed to install '{}': {e}", dest.display()))?;
    Ok(dest)
}

/// Fetches the core component when the compiler does not find a core library.
fn fetch_on_demand() -> Result<(), String> {
    if offline() {
        return Err(format!("the core libraries of mote {} are not installed; MOTE_OFFLINE is set, run `mote core fetch` without it", modules::core::VERSION));
    }
    eprintln!("fetching the core libraries of mote {} for {} from {}", modules::core::VERSION, host_triple(), base_url());
    let dest = fetch(false).map_err(|e| format!("cannot fetch the core libraries: {e}"))?;
    eprintln!("installed in {}", dest.display());
    Ok(())
}

/// Lets the compiler fetch the core component when a core library is not installed.
pub fn install_fetcher() {
    modules::core::set_fetcher(fetch_on_demand);
}

/// One line per core library, with where it is.
pub fn list() -> Vec<String> {
    let dirs = modules::core::core_dirs();
    let host = host_triple();
    modules::core::LIBRARIES
        .iter()
        .map(|name| match dirs.iter().find(|d| modules::core::has_library(d, name)) {
            Some(dir) => {
                let native = dir.join(name).join("native");
                let note = if !native.is_dir() {
                    ""
                } else if native.join(&host).is_dir() {
                    " (native)"
                } else {
                    " (native: none for this machine)"
                };
                format!("{name}  {}{note}", dir.join(name).display())
            }
            None => format!("{name}  not installed"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_path_outside_core_is_skipped() {
        let mut tar = tar::Builder::new(Vec::new());
        let mut add = |path: &str, data: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            tar.append_data(&mut header, path, data).unwrap();
        };
        add("core/path/src/lib.mote", b"x");
        add("other/file", b"y");
        let gz = {
            use std::io::Write;
            let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            w.write_all(&tar.into_inner().unwrap()).unwrap();
            w.finish().unwrap()
        };
        assert_eq!(unpack(&gz).unwrap(), vec![("path/src/lib.mote".to_string(), b"x".to_vec())]);
    }
}
