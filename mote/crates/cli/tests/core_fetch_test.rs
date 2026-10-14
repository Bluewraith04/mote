//! The core component is fetched as one archive, installed in the mote home and found by name.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

fn archive_name() -> String {
    format!("mote-core-{}-{}.tar.gz", modules::core::VERSION, pkg::native::host_triple())
}

/// A directory serving an archive with every core library, `path` holding `marker()`.
fn serve(tag: &str, sha: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_corefetch_{tag}_{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    let mut add = |path: String, data: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar.append_data(&mut header, path, data).unwrap();
    };
    for name in modules::core::LIBRARIES {
        let body = if name == "path" { "pub fn marker() -> Int {\n    return 7\n}\n" } else { "pub fn present() -> Int {\n    return 1\n}\n" };
        add(format!("core/{name}/src/lib.mote"), body.as_bytes());
    }
    add(format!("core/sqlite/native/{}/libmote_sqlite.so", pkg::native::host_triple()), b"native");
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar.into_inner().unwrap()).unwrap();
    let bytes = gz.finish().unwrap();
    let sum: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    fs::write(dir.join(archive_name()), &bytes).unwrap();
    fs::write(dir.join(format!("{}.sha256", archive_name())), format!("{}  {}\n", sha.unwrap_or(&sum), archive_name())).unwrap();
    dir
}

fn mote(args: &[&str], home: &Path, url: &Path, extra: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mote"));
    cmd.args(args).env("MOTE_HOME", home).env("MOTE_CORE_URL", url).env("MOTE_CORE_PATH", "").env_remove("MOTE_OFFLINE");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn program(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_coreprog_{tag}_{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.mote"), "import path\n\nfn main() {\n    println(path.marker())\n}\n").unwrap();
    dir.join("main.mote")
}

fn store(home: &Path) -> PathBuf {
    home.join("core").join(modules::core::VERSION)
}

#[test]
fn fetch_installs_every_library_and_list_shows_them() {
    let served = serve("fetch", None);
    let home = std::env::temp_dir().join(format!("mote_corehome_fetch_{}", std::process::id()));
    fs::remove_dir_all(&home).ok();
    let (ok, text) = mote(&["core", "fetch"], &home, &served, &[]);
    assert!(ok, "got: {text}");
    assert!(store(&home).join("pane/src/lib.mote").is_file());
    assert!(store(&home).join("sqlite/native").join(pkg::native::host_triple()).join("libmote_sqlite.so").is_file());
    let (ok, listed) = mote(&["core", "list"], &home, &served, &[]);
    assert!(ok && listed.contains("pane") && listed.contains("(native)"), "got: {listed}");
    fs::remove_dir_all(&served).ok();
    fs::remove_dir_all(&home).ok();
}

#[test]
fn a_program_that_imports_a_missing_core_library_fetches_it_once() {
    let served = serve("auto", None);
    let home = std::env::temp_dir().join(format!("mote_corehome_auto_{}", std::process::id()));
    fs::remove_dir_all(&home).ok();
    let main = program("auto");
    let (ok, first) = mote(&["run", main.to_str().unwrap()], &home, &served, &[]);
    assert!(ok && first.contains("fetching the core libraries") && first.starts_with("7\n"), "got: {first}");
    let (ok, second) = mote(&["run", main.to_str().unwrap()], &home, &served, &[]);
    assert!(ok && !second.contains("fetching") && second.trim() == "7", "got: {second}");
    fs::remove_dir_all(&served).ok();
    fs::remove_dir_all(&home).ok();
    fs::remove_dir_all(main.parent().unwrap()).ok();
}

#[test]
fn offline_names_the_command_that_fetches() {
    let served = serve("offline", None);
    let home = std::env::temp_dir().join(format!("mote_corehome_offline_{}", std::process::id()));
    fs::remove_dir_all(&home).ok();
    let main = program("offline");
    let (ok, text) = mote(&["run", main.to_str().unwrap()], &home, &served, &[("MOTE_OFFLINE", "1")]);
    assert!(!ok && text.contains("mote core fetch"), "got: {text}");
    assert!(!store(&home).exists());
    fs::remove_dir_all(&served).ok();
    fs::remove_dir_all(main.parent().unwrap()).ok();
}

#[test]
fn an_archive_that_does_not_match_its_checksum_is_refused() {
    let served = serve("sum", Some("0000"));
    let home = std::env::temp_dir().join(format!("mote_corehome_sum_{}", std::process::id()));
    fs::remove_dir_all(&home).ok();
    let (ok, text) = mote(&["core", "fetch"], &home, &served, &[]);
    assert!(!ok && text.contains(".sha256"), "got: {text}");
    assert!(!store(&home).exists());
    fs::remove_dir_all(&served).ok();
}

#[test]
fn a_dependency_or_program_module_named_like_a_core_library_is_not_fetched() {
    let served = serve("own", None);
    let home = std::env::temp_dir().join(format!("mote_corehome_own_{}", std::process::id()));
    fs::remove_dir_all(&home).ok();
    let main = program("own");
    fs::write(main.parent().unwrap().join("path.mote"), "pub fn marker() -> Int {\n    return 3\n}\n").unwrap();
    let (ok, text) = mote(&["run", main.to_str().unwrap()], &home, &served, &[]);
    assert!(ok && text.trim() == "3", "got: {text}");
    assert!(!store(&home).exists());
    fs::remove_dir_all(&served).ok();
    fs::remove_dir_all(main.parent().unwrap()).ok();
}
